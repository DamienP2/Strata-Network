//! Online switches: live reads of the port states, counters and VLAN setup
//! (read-only SNMP v2c), the differences between the switch and the
//! inventory, and taking the switch's setup into the inventory.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use strata_net::snmp::{self as s, IfStat, System, VlanSetup};
use tokio::task::AbortHandle;

use super::model::{Switch, SwitchPort};
use super::rates::Sample;
use super::snmp::{same_port, session};
use super::Net;
use crate::paths::now_ms;

/// Rates kept per port for the sparklines.
const HISTORY: usize = 60;
/// The identity and the VLAN setup are read again after this long.
const SLOW: Duration = Duration::from_secs(300);

/// One faceplate port as the switch sees it now.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LivePort {
    pub n: u16,
    pub if_index: u32,
    pub up: bool,
    pub admin_up: bool,
    pub speed_mbps: u64,
    pub in_bps: u64,
    pub out_bps: u64,
    pub in_errors: u64,
    pub out_errors: u64,
    /// Packets discarded since the switch started (Counter32 as read).
    pub in_discards: u64,
    pub out_discards: u64,
    /// Discarded packets a second, from the last two readings.
    pub in_discards_pps: f64,
    pub out_discards_pps: f64,
    /// The port description set in the switch.
    pub alias: String,
    /// Rates, oldest first, ≤ 60.
    pub in_history: Vec<u64>,
    pub out_history: Vec<u64>,
}

/// The VLAN setup of one faceplate port in the switch.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortVlans {
    pub n: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pvid: Option<u16>,
    pub untagged: Vec<u16>,
    pub tagged: Vec<u16>,
}

/// A port set up differently in the switch and in the inventory.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    pub n: u16,
    /// "mode", "vlan", "tagged" or "off".
    pub field: &'static str,
    /// Short facts: "access VLAN 10", "trunk 30, 40", "disabled", "VLAN 20", "native VLAN 1".
    pub switch: String,
    pub inventory: String,
}

/// What is known of an online switch (in memory only).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Live {
    /// Last successful read (unix ms); 0 before the first one.
    pub at: i64,
    /// Why the last read failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<System>,
    pub ports: Vec<LivePort>,
    /// Absent until the VLAN setup could be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlans: Option<Vec<PortVlans>>,
    pub diffs: Vec<Diff>,
}

/// An interface's counters at the reading rates are computed from.
#[derive(Clone, Debug)]
pub struct Prev {
    pub in_octets: u64,
    pub out_octets: u64,
    pub in_discards: u64,
    pub out_discards: u64,
    /// The switch's sysUpTime then (centiseconds).
    pub ticks: Option<u32>,
    pub at: Instant,
    /// Readings skipped since because the switch had not refreshed its counters.
    pub stale: u32,
    /// Rates are computed from this reading: the counters had moved since the first one.
    pub primed: bool,
    /// The last moves of the counters: time since the move before (seconds), and whether
    /// stale readings came in between. Their average is the switch's refresh period.
    pub moves: VecDeque<(f64, bool)>,
}

/// The counters and slow reads behind `Live`.
#[derive(Default)]
struct Memory {
    /// ifIndex → the reading the next rates are computed from.
    counters: HashMap<u32, Prev>,
    system: Option<(System, Instant)>,
    system_read: Option<Instant>,
    setup: Option<VlanSetup>,
    vlans_read: Option<Instant>,
    /// Every interface's name by ifIndex, from the last read.
    names: HashMap<u32, String>,
}

#[derive(Default)]
pub struct Runner {
    pub live: Live,
    mem: Memory,
    task: Option<AbortHandle>,
    /// The slow reads (neighbours, optics, health…) and their task.
    pub info: super::info::InfoMem,
    pub(super) info_task: Option<AbortHandle>,
    /// What the alert rules remember.
    pub(super) watch: super::info::Watch,
}

// ── Computations ──

/// Bits per second between two readings of an octet counter; `None` when the
/// counter went back (wrapped, or the switch restarted): that sample is skipped.
pub fn rate(prev: u64, cur: u64, secs: f64) -> Option<u64> {
    (secs > 0.0 && cur >= prev).then(|| ((cur - prev) as f64 * 8.0 / secs).round() as u64)
}

/// The increase of a Counter32 between two readings, past one wrap at 2^32 (from the
/// upper half to the lower half); `None` when it went back otherwise (cleared, restart).
pub fn delta32(prev: u64, cur: u64) -> Option<u64> {
    const WRAP: u64 = 1 << 32;
    if cur >= prev {
        Some(cur - prev)
    } else if (WRAP / 2..WRAP).contains(&prev) && cur < WRAP / 2 {
        Some(WRAP - prev + cur)
    } else {
        None
    }
}

/// Seconds between two readings: by the switch's own clock (sysUpTime, centiseconds,
/// wrapping at 2^32) when both readings have it and it agrees with the server's
/// clock (within half the interval, at least 5 s); else the server's clock (a
/// switch that restarted, an agent without sysUpTime).
pub fn interval(prev: Option<u32>, cur: Option<u32>, server_secs: f64) -> f64 {
    if let (Some(a), Some(b)) = (prev, cur) {
        let d = f64::from(b.wrapping_sub(a)) / 100.0;
        if d > 0.0 && (d - server_secs).abs() <= (server_secs * 0.5).max(5.0) {
            return d;
        }
    }
    server_secs
}

/// Readings in a row whose octet counters may be taken as not refreshed yet:
/// a rate then covers at most 3 read intervals.
const MAX_STALE: u32 = 2;
/// Moves of the counters kept to find the switch's refresh period.
const MOVES: usize = 24;

fn pps(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Faceplate port n → index in `stats`, by interface name.
pub fn match_ports(sw: &Switch, stats: &[IfStat]) -> Vec<(u16, usize)> {
    sw.ports
        .iter()
        .filter(|p| !p.name.is_empty())
        .filter_map(|p| stats.iter().position(|st| !st.name.is_empty() && same_port(&st.name, &p.name)).map(|i| (p.n, i)))
        .collect()
}

fn push(h: &mut Vec<u64>, v: u64) {
    h.push(v);
    if h.len() > HISTORY {
        h.drain(..h.len() - HISTORY);
    }
}

fn prev_of(st: &IfStat, now: Instant) -> Prev {
    Prev {
        in_octets: st.in_octets,
        out_octets: st.out_octets,
        in_discards: st.in_discards,
        out_discards: st.out_discards,
        ticks: st.ticks,
        at: now,
        stale: 0,
        primed: false,
        moves: VecDeque::new(),
    }
}

/// The ports after a new reading: rates from the counter deltas, history kept from `prev`;
/// and the new samples.
///
/// The interval is the switch's sysUpTime delta (see `interval`). Switches that
/// refresh their interface counters only every few seconds (NX-OS) answer the same
/// octets again in between: when both octet counters of a port that is up and had
/// traffic (or no rate yet) did not move, the reading is taken as stale; nothing is emitted, and the
/// next reading that moved gives the rate over the whole time (at most `MAX_STALE`
/// readings are skipped in a row, then the stop is real). Once stale readings came twice
/// among the last 24 moves of the counters of a port with traffic (≥ 10 kb/s), each move is taken as one refresh of the
/// switch and the interval is the refresh period (the average time between those
/// moves) rather than the time between the two reads that saw them, which jitters by up
/// to a read interval as the reads drift across the refreshes. The first rate is computed
/// from the counters' first move, not from the first reading, which can catch the
/// cached counters anywhere in their refresh period.
fn read_ports(sw: &Switch, stats: &[IfStat], prev: &[LivePort], counters: &mut HashMap<u32, Prev>, now: Instant) -> (Vec<LivePort>, Vec<Sample>) {
    let mut out = Vec::new();
    let mut samples = Vec::new();
    for (n, i) in match_ports(sw, stats) {
        let st = &stats[i];
        let old = prev.iter().find(|p| p.n == n && p.if_index == st.if_index);
        let mut p = LivePort {
            n,
            if_index: st.if_index,
            up: st.up,
            admin_up: st.admin_up,
            speed_mbps: st.speed_mbps,
            in_bps: old.map_or(0, |o| o.in_bps),
            out_bps: old.map_or(0, |o| o.out_bps),
            in_errors: st.in_errors,
            out_errors: st.out_errors,
            in_discards: st.in_discards,
            out_discards: st.out_discards,
            in_discards_pps: old.map_or(0.0, |o| o.in_discards_pps),
            out_discards_pps: old.map_or(0.0, |o| o.out_discards_pps),
            alias: st.alias.clone(),
            in_history: old.map(|o| o.in_history.clone()).unwrap_or_default(),
            out_history: old.map(|o| o.out_history.clone()).unwrap_or_default(),
        };
        if let Some(pv) = counters.get_mut(&st.if_index) {
            let same = st.in_octets == pv.in_octets && st.out_octets == pv.out_octets;
            // No rate yet (just online): an unmoved reading may be stale as well.
            let had_traffic = old.is_none_or(|o| o.in_history.is_empty() || o.in_bps > 0 || o.out_bps > 0);
            if st.up && same && had_traffic && pv.stale < MAX_STALE {
                pv.stale += 1;
                out.push(p);
                continue;
            }
            if !pv.primed && !same {
                // The first reading caught the switch's cached counters at an unknown point of its
                // refresh: the rates start from their first move instead.
                *pv = Prev { primed: true, stale: 0, ..prev_of(st, now) };
                out.push(p);
                continue;
            }
            let (old_in_d, old_out_d) = (pv.in_discards, pv.out_discards);
            let mut secs = interval(pv.ticks, st.ticks, now.duration_since(pv.at).as_secs_f64());
            let mut moves = std::mem::take(&mut pv.moves);
            if !same {
                moves.push_back((secs, pv.stale > 0));
                if moves.len() > MOVES {
                    moves.pop_front();
                }
                // Cached counters (stale readings twice among the last moves of a port with
                // traffic): each move is one refresh of the switch, whatever the time between
                // the reads that saw it; the interval is the refresh period, the average time
                // between moves. (A port under 10 kb/s may not move between two reads of live
                // counters: its raw intervals are kept.)
                let busy = old.is_some_and(|o| o.in_bps.max(o.out_bps) >= 10_000);
                if busy && moves.iter().filter(|m| m.1).count() >= 2 {
                    secs = moves.iter().map(|m| m.0).sum::<f64>() / moves.len() as f64;
                }
            }
            let keep = Prev { moves, primed: true, ..prev_of(st, now) };
            let rates = (rate(pv.in_octets, st.in_octets, secs), rate(pv.out_octets, st.out_octets, secs));
            *pv = keep;
            if let (Some(i), Some(o)) = rates {
                let d = |a: u64, b: u64| delta32(a, b).map_or(0.0, |x| pps(x as f64 / secs));
                p.in_bps = i;
                p.out_bps = o;
                p.in_discards_pps = d(old_in_d, st.in_discards);
                p.out_discards_pps = d(old_out_d, st.out_discards);
                push(&mut p.in_history, i);
                push(&mut p.out_history, o);
                samples.push(Sample { n, in_bps: i, out_bps: o, in_discards: p.in_discards_pps, out_discards: p.out_discards_pps });
            } else {
                // The counters went back (restart, cleared): their moves say nothing any more.
                pv.moves.clear();
            }
            out.push(p);
            continue;
        }
        counters.insert(st.if_index, prev_of(st, now));
        out.push(p);
    }
    (out, samples)
}

/// The VLAN setup of each matched port.
pub fn port_vlans(ports: &[LivePort], setup: &VlanSetup) -> Vec<PortVlans> {
    ports
        .iter()
        .map(|p| PortVlans {
            n: p.n,
            pvid: setup.pvid.get(&p.if_index).copied(),
            untagged: setup.untagged.get(&p.if_index).cloned().unwrap_or_default(),
            tagged: setup.tagged.get(&p.if_index).cloned().unwrap_or_default(),
        })
        .collect()
}

/// A port's setup in the inventory's terms.
#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    pub mode: &'static str,
    /// Access VLAN, or native VLAN of a trunk.
    pub vlan: Option<u16>,
    pub tagged: Vec<u16>,
}

/// The switch's setup of a port: off when disabled, trunk when it carries
/// tagged VLANs (native = the PVID when it leaves untagged), else access on the PVID.
pub fn switch_setup(lp: &LivePort, v: &PortVlans) -> Setup {
    if !lp.admin_up {
        Setup { mode: "off", vlan: v.pvid, tagged: v.tagged.clone() }
    } else if !v.tagged.is_empty() {
        Setup { mode: "trunk", vlan: v.pvid.filter(|p| v.untagged.contains(p)), tagged: v.tagged.clone() }
    } else {
        Setup { mode: "access", vlan: v.pvid, tagged: vec![] }
    }
}

fn list(v: &[u16]) -> String {
    v.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")
}

fn vlan_text(v: Option<u16>) -> String {
    v.map_or_else(|| "no VLAN".into(), |v| format!("VLAN {v}"))
}

fn describe(mode: &str, vlan: Option<u16>, tagged: &[u16]) -> String {
    match mode {
        "off" => "disabled".into(),
        "trunk" if tagged.is_empty() => "trunk".into(),
        "trunk" => format!("trunk {}", list(tagged)),
        _ => vlan.map_or_else(|| "access".into(), |v| format!("access VLAN {v}")),
    }
}

/// How the port differs between the switch and the inventory, if it does.
/// A trunk's native VLAN is compared only when the inventory sets one; an
/// access port on VLAN 1 matches an inventory port without VLAN.
pub fn diff_of(p: &SwitchPort, sw: &Setup) -> Option<Diff> {
    let d = |field, switch: String, inventory: String| Some(Diff { n: p.n, field, switch, inventory });
    let inv = || describe(&p.mode, p.vlan, &p.tagged);
    if sw.mode == "off" {
        return if p.mode == "off" { None } else { d("off", "disabled".into(), inv()) };
    }
    if p.mode != sw.mode {
        return d("mode", describe(sw.mode, sw.vlan, &sw.tagged), inv());
    }
    if sw.mode == "access" {
        // VLAN 1 is where a switch leaves the ports nobody set up: same as no VLAN in the inventory.
        let same = p.vlan == sw.vlan || (p.vlan.is_none() && sw.vlan == Some(1));
        return if same { None } else { d("vlan", vlan_text(sw.vlan), vlan_text(p.vlan)) };
    }
    if p.tagged != sw.tagged {
        return d("tagged", describe("trunk", None, &sw.tagged), inv());
    }
    if p.vlan.is_some() && p.vlan != sw.vlan {
        return d("vlan", format!("native {}", vlan_text(sw.vlan)), format!("native {}", vlan_text(p.vlan)));
    }
    None
}

/// The ports set up differently in the switch (needs the VLAN setup).
pub fn diffs(sw: &Switch, ports: &[LivePort], vlans: &[PortVlans]) -> Vec<Diff> {
    sw.ports
        .iter()
        .filter_map(|p| {
            let lp = ports.iter().find(|x| x.n == p.n)?;
            let v = vlans.iter().find(|x| x.n == p.n)?;
            diff_of(p, &switch_setup(lp, v))
        })
        .collect()
}

/// Copies the switch's setup of these ports (`None`: every port with a
/// difference) into the inventory: mode, VLAN, tagged VLANs, and the port
/// description as label where the inventory has none. VLAN 1, the switch's
/// default, becomes no VLAN unless the inventory has a VLAN 1 (`known`).
/// Returns the names of the ports taken.
pub fn take(sw: &mut Switch, live: &Live, which: Option<&[u16]>, known: &[u16]) -> Vec<String> {
    let vlan = |v: Option<u16>| v.filter(|v| *v != 1 || known.contains(&1));
    let Some(vlans) = &live.vlans else { return vec![] };
    let wanted: Vec<u16> = match which {
        Some(w) => w.to_vec(),
        None => live.diffs.iter().map(|d| d.n).collect(),
    };
    let mut names = Vec::new();
    for p in sw.ports.iter_mut().filter(|p| wanted.contains(&p.n)) {
        let (Some(lp), Some(v)) = (live.ports.iter().find(|x| x.n == p.n), vlans.iter().find(|x| x.n == p.n)) else { continue };
        let st = switch_setup(lp, v);
        match st.mode {
            // Disabled: the VLANs of the inventory stay for when it is enabled again.
            "off" => p.mode = "off".into(),
            "trunk" => {
                p.mode = "trunk".into();
                p.vlan = vlan(st.vlan);
                p.tagged = st.tagged;
            }
            _ => {
                p.mode = "access".into();
                p.vlan = vlan(st.vlan);
                p.tagged = vec![];
            }
        }
        if p.label.is_empty() && !lp.alias.is_empty() {
            p.label = lp.alias.clone();
        }
        names.push(p.name.clone());
    }
    names
}

impl Runner {
    /// Puts a new reading in: ports and rates, system, VLANs, differences. Returns the new rates.
    fn update(&mut self, sw: &Switch, stats: &[IfStat], now: Instant) -> Vec<Sample> {
        let (ports, samples) = read_ports(sw, stats, &self.live.ports, &mut self.mem.counters, now);
        self.live.ports = ports;
        self.live.at = now_ms();
        self.live.error = None;
        self.mem.names = stats.iter().filter(|s| !s.name.is_empty()).map(|s| (s.if_index, s.name.clone())).collect();
        self.refresh(sw);
        samples
    }

    /// Every interface's name by ifIndex.
    pub fn names(&self) -> &HashMap<u32, String> {
        &self.mem.names
    }

    /// System uptime brought to now, VLANs and differences from the inventory as it is.
    fn refresh(&mut self, sw: &Switch) {
        self.live.system = self.mem.system.as_ref().map(|(sys, at)| System { uptime_secs: sys.uptime_secs + at.elapsed().as_secs(), ..sys.clone() });
        self.live.vlans = self.mem.setup.as_ref().map(|setup| port_vlans(&self.live.ports, setup));
        self.live.diffs = self.live.vlans.as_deref().map(|v| diffs(sw, &self.live.ports, v)).unwrap_or_default();
    }
}

// ── Reading ──

impl Net {
    /// Starts reading the switch (again, keeping what is known, when the interval changed).
    pub fn start_live(self: &Arc<Self>, id: &str) {
        let mut map = self.live.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let r = map.entry(id.to_string()).or_default();
        let net = self.clone();
        let key = id.to_string();
        let task = tokio::spawn(async move { net.live_loop(key).await }).abort_handle();
        if let Some(old) = r.task.replace(task) {
            old.abort();
        }
        self.start_info(id, r);
    }

    /// Stops reading the switch and forgets what was read.
    pub fn stop_live(&self, id: &str) {
        if let Some(r) = self.live.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(id) {
            for t in [r.task, r.info_task].into_iter().flatten() {
                t.abort();
            }
        }
    }

    pub fn live_of(&self, id: &str) -> Option<Live> {
        self.live.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(id).map(|r| r.live.clone())
    }

    /// The inventory of an online switch changed: differences again, sent to the clients.
    pub fn live_changed(&self, id: &str) {
        let Some(sw) = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).switches.iter().find(|s| s.id == id).cloned() else { return };
        let live = {
            let mut map = self.live.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(r) = map.get_mut(id) else { return };
            r.refresh(&sw);
            r.live.clone()
        };
        self.send_live(id, &live);
    }

    fn send_live(&self, id: &str, live: &Live) {
        self.send(&serde_json::json!({"type": "switchLive", "id": id, "live": live}));
    }

    async fn live_loop(self: Arc<Self>, id: String) {
        loop {
            let secs = {
                let st = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                match st.switches.iter().find(|s| s.id == id) {
                    Some(sw) if sw.online => sw.online_secs,
                    _ => break,
                }
            };
            self.read_live(&id).await;
            tokio::time::sleep(Duration::from_secs(u64::from(secs.max(1)))).await;
        }
        if let Some(t) = self.live.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&id).and_then(|r| r.info_task) {
            t.abort();
        }
    }

    /// One read: states and counters; identity and VLAN setup when due.
    async fn read_live(self: &Arc<Self>, id: &str) {
        let agent = {
            let st = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let sw = st.switches.iter().find(|s| s.id == id);
            sw.and_then(|sw| sw.snmp.as_deref()).and_then(|a| st.snmp.iter().find(|x| x.id == a)).cloned()
        };
        let (need_system, need_vlans) = {
            let map = self.live.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(r) = map.get(id) else { return };
            let due = |t: Option<Instant>| t.is_none_or(|t| t.elapsed() >= SLOW);
            (due(r.mem.system_read), due(r.mem.vlans_read))
        };
        let read = async {
            let agent = agent.ok_or_else(|| "the switch has no snmp agent".to_string())?;
            let session = session(&agent).await?;
            let stats = s::if_stats(&session).await.map_err(|e| e.to_string())?;
            // The server's clock for the counters: right after them, not after the slower reads.
            let read_at = Instant::now();
            let system = if need_system { Some(s::system(&session).await.ok()) } else { None };
            let setup = if need_vlans { Some(s::vlan_setup(&session).await.ok()) } else { None };
            Ok::<_, String>((stats, read_at, system, setup))
        };
        let result = read.await;
        let now = Instant::now();
        let Some(sw) = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).switches.iter().find(|s| s.id == id).cloned() else { return };
        let mut samples = Vec::new();
        let mut raised = Vec::new();
        let live = {
            let mut map = self.live.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(r) = map.get_mut(id) else { return };
            match result {
                Ok((stats, read_at, system, setup)) => {
                    if let Some(sys) = system {
                        r.mem.system_read = Some(now);
                        if let Some(sys) = sys {
                            r.mem.system = Some((sys, now));
                        }
                    }
                    if let Some(setup) = setup {
                        r.mem.vlans_read = Some(now);
                        if setup.is_some() {
                            r.mem.setup = setup;
                        }
                    }
                    samples = r.update(&sw, &stats, read_at);
                    let diffs = r.live.vlans.is_some().then_some(r.live.diffs.as_slice());
                    raised = super::info::check_live(&mut r.watch, &sw.rules, &sw, &r.live.ports, diffs, r.live.at, now);
                }
                Err(e) => {
                    tracing::warn!("live read of {}: {e}", sw.name);
                    r.live.error = Some(e);
                    r.refresh(&sw);
                }
            }
            r.live.clone()
        };
        self.record_rates(id, &samples);
        self.raise_switch(&sw, raised);
        self.send_live(id, &live);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sw() -> Switch {
        let mut s: Switch = serde_json::from_value(serde_json::json!({"name": "sw", "copper": 4, "sfp": 2})).unwrap();
        s.normalize();
        s
    }

    fn stat(i: u32, name: &str, inn: u64, out: u64) -> IfStat {
        IfStat { if_index: i, name: name.into(), admin_up: true, up: true, speed_mbps: 1000, in_octets: inn, out_octets: out, ..IfStat::default() }
    }

    #[test]
    fn rates_skip_wraps() {
        assert_eq!(rate(1_000, 2_000, 2.0), Some(4_000));
        assert_eq!(rate(5_000, 100, 10.0), None);
        assert_eq!(rate(0, 100, 0.0), None);
        let s = sw();
        let mut counters = HashMap::new();
        let t0 = Instant::now() + Duration::from_secs(10);
        // The counters' first move primes the rates.
        read_ports(&s, &[stat(10101, "GigabitEthernet1/0/1", 500, 0)], &[], &mut counters, t0 - Duration::from_secs(10));
        let (p, _) = read_ports(&s, &[stat(10101, "GigabitEthernet1/0/1", 1_000, 0)], &[], &mut counters, t0);
        assert_eq!((p[0].in_bps, p[0].in_history.len()), (0, 0));
        let t1 = t0 + Duration::from_secs(10);
        let (p, _) = read_ports(&s, &[stat(10101, "GigabitEthernet1/0/1", 126_000, 12_500)], &p, &mut counters, t1);
        assert_eq!((p[0].in_bps, p[0].out_bps), (100_000, 10_000));
        assert_eq!(p[0].in_history, vec![100_000]);
        // The counter went back (restart): the sample is skipped, the last rate stays.
        let t2 = t1 + Duration::from_secs(10);
        let (p, _) = read_ports(&s, &[stat(10101, "GigabitEthernet1/0/1", 50, 12_600)], &p, &mut counters, t2);
        assert_eq!((p[0].in_bps, p[0].in_history.len()), (100_000, 1));
        let t3 = t2 + Duration::from_secs(10);
        let (p, _) = read_ports(&s, &[stat(10101, "GigabitEthernet1/0/1", 1_300, 12_600)], &p, &mut counters, t3);
        assert_eq!((p[0].in_bps, p[0].in_history.len()), (1_000, 2));
    }

    fn ticked(i: u32, inn: u64, out: u64, ticks: Option<u32>) -> IfStat {
        IfStat { ticks, ..stat(i, "GigabitEthernet1/0/1", inn, out) }
    }

    #[test]
    fn intervals_by_the_switch_clock() {
        // sysUpTime delta (centiseconds) when it agrees with the server's clock.
        assert_eq!(interval(Some(100_000), Some(101_000), 10.4), 10.0);
        assert_eq!(interval(Some(100_000), Some(100_850), 10.0), 8.5);
        // Missing on either side: the server's clock.
        assert_eq!(interval(None, Some(101_000), 10.4), 10.4);
        assert_eq!(interval(Some(100_000), None, 10.4), 10.4);
        // sysUpTime wraps at 2^32 centiseconds (497 days).
        assert_eq!(interval(Some(u32::MAX - 499), Some(500), 10.0), 10.0);
        // The switch restarted (uptime went back), or a clock far off: the server's clock.
        assert_eq!(interval(Some(9_000_000), Some(300), 10.0), 10.0);
        assert_eq!(interval(Some(100_000), Some(100_000), 10.0), 10.0);
        assert_eq!(interval(Some(100_000), Some(106_000), 10.0), 10.0);

        // A read delayed on the server's side (2 s late) still gives the true rate.
        let s = sw();
        let mut counters = HashMap::new();
        let t0 = Instant::now() + Duration::from_secs(10);
        read_ports(&s, &[ticked(1, 0, 0, Some(0))], &[], &mut counters, t0 - Duration::from_secs(10));
        let (p, _) = read_ports(&s, &[ticked(1, 1, 0, Some(1_000))], &[], &mut counters, t0);
        let (p, smp) = read_ports(&s, &[ticked(1, 12_500_001, 0, Some(2_000))], &p, &mut counters, t0 + Duration::from_secs(12));
        assert_eq!((p[0].in_bps, smp[0].in_bps), (10_000_000, 10_000_000));
        // Restarted between two reads: counters went back, the sample is skipped; then the server's clock until both readings have a sane uptime.
        let (p, smp) = read_ports(&s, &[ticked(1, 1_000, 0, Some(500))], &p, &mut counters, t0 + Duration::from_secs(22));
        assert!(smp.is_empty());
        let (_, smp) = read_ports(&s, &[ticked(1, 1_000 + 1_250_000, 0, Some(1_500))], &p, &mut counters, t0 + Duration::from_secs(32));
        assert_eq!(smp[0].in_bps, 1_000_000);
    }

    #[test]
    fn counter32_wraps() {
        assert_eq!(delta32(10, 25), Some(15));
        assert_eq!(delta32(u64::from(u32::MAX) - 4, 10), Some(15));
        // Back from the lower half: cleared or restarted, not a wrap.
        assert_eq!(delta32(1_000, 10), None);
        let s = sw();
        let mut counters = HashMap::new();
        let t0 = Instant::now() + Duration::from_secs(10);
        let d = |inn: u64, out: u64, din: u64, dout: u64| IfStat { in_discards: din, out_discards: dout, ..ticked(1, inn, out, None) };
        read_ports(&s, &[d(0, 0, 0, 0)], &[], &mut counters, t0 - Duration::from_secs(10));
        let (p, _) = read_ports(&s, &[d(1, 1, u64::from(u32::MAX) - 49, 7)], &[], &mut counters, t0);
        assert_eq!((p[0].in_discards, p[0].out_discards), (u64::from(u32::MAX) - 49, 7));
        let (p, smp) = read_ports(&s, &[d(1_001, 1_001, 50, 7 + 125)], &p, &mut counters, t0 + Duration::from_secs(10));
        assert_eq!((p[0].in_discards_pps, p[0].out_discards_pps), (10.0, 12.5));
        assert_eq!((smp[0].in_discards, smp[0].out_discards), (10.0, 12.5));
        // Discards cleared alone: no rate for them, the octet rates stay.
        let (p, smp) = read_ports(&s, &[d(2_001, 2_001, 0, 0)], &p, &mut counters, t0 + Duration::from_secs(20));
        assert_eq!((smp[0].in_bps, p[0].in_discards_pps, p[0].out_discards_pps), (800, 0.0, 0.0));
    }

    #[test]
    fn stale_counters_are_merged() {
        // The switch refreshes its counters every other read (20 s, read every 10 s at
        // 1 Gb/s): raw deltas alternate 2.5 GB and 0; the rates stay flat. The first move
        // of the counters only primes the rates (it may cover more than the read interval).
        let s = sw();
        let mut counters = HashMap::new();
        let t0 = Instant::now();
        let mut p = Vec::new();
        let mut rates = Vec::new();
        for k in 0..12u32 {
            let refreshed = u64::from(k.div_ceil(2)) * 2 * 1_250_000_000;
            let (np, smp) = read_ports(&s, &[ticked(1, refreshed, refreshed / 2, Some(10_000 + k * 1_000))], &p, &mut counters, t0 + Duration::from_secs(u64::from(k) * 10));
            p = np;
            rates.extend(smp.iter().map(|x| (x.in_bps, x.out_bps)));
        }
        assert_eq!(rates.len(), 5);
        assert!(rates.iter().all(|r| *r == (1_000_000_000, 500_000_000)), "{rates:?}");
        assert_eq!(p[0].in_history, vec![1_000_000_000; 5]);
        assert_eq!(p[0].in_bps, 1_000_000_000);

        // Traffic that really stopped: zero after at most three intervals.
        let mut zero = Vec::new();
        let at = |k: u64| t0 + Duration::from_secs(120 + k * 10);
        let total = 6 * 2 * 1_250_000_000;
        for k in 0..4 {
            let (np, smp) = read_ports(&s, &[ticked(1, total, total / 2, Some(22_000 + k as u32 * 1_000))], &p, &mut counters, at(k));
            p = np;
            zero.extend(smp.iter().map(|x| x.in_bps));
        }
        // Reads 0 and 1 are taken as stale, 2 gives zero over three intervals, 3 zero again.
        assert_eq!(zero, vec![0, 0]);
        assert_eq!(p[0].in_bps, 0);
        // Just online, read every 5 s, refreshed every 10 s: no zero, no double rate; the
        // rates start from the counters' first move.
        let mut c4 = HashMap::new();
        let mut q = Vec::new();
        let mut first = Vec::new();
        for (k, octets) in [(0u64, 0u64), (1, 0), (2, 1_250_000_000), (3, 1_250_000_000), (4, 2_500_000_000)] {
            let (nq, smp) = read_ports(&s, &[ticked(1, octets, 0, None)], &q, &mut c4, t0 + Duration::from_secs(k * 5));
            q = nq;
            first.extend(smp.iter().map(|x| x.in_bps));
        }
        assert_eq!(first, vec![1_000_000_000]);
        // A port without traffic: unmoved readings are a real zero (after at most two skipped).
        let idle = [ticked(1, 5, 5, None)];
        let mut c2 = HashMap::new();
        let mut q = Vec::new();
        let mut zeros = Vec::new();
        for k in 0..5 {
            let (nq, smp) = read_ports(&s, &idle, &q, &mut c2, t0 + Duration::from_secs(k * 10));
            q = nq;
            zeros.extend(smp.iter().map(|x| x.in_bps));
        }
        assert_eq!(zeros, vec![0, 0]);
        // A port that went down is not stale either.
        let down = [IfStat { up: false, ..ticked(1, total, total / 2, None) }];
        let mut c3 = HashMap::new();
        let busy = vec![LivePort { n: 1, if_index: 1, in_bps: 5, ..LivePort::default() }];
        read_ports(&s, &down, &busy, &mut c3, t0);
        let (_, smp) = read_ports(&s, &down, &busy, &mut c3, t0 + Duration::from_secs(10));
        assert_eq!(smp.len(), 1);
    }

    #[test]
    fn reads_drifting_across_the_refresh() {
        // Refreshed every 10 s, read every 5.3 s (5 s + the read): mostly a stale read
        // between two moves, and now and then two moves in a row 5.3 s apart that still
        // carry 10 s of traffic. Without the refresh period these were 1.9 Gb/s spikes.
        let s = sw();
        let mut counters = HashMap::new();
        let t0 = Instant::now();
        let mut p = Vec::new();
        let mut rates = Vec::new();
        for k in 0..200u32 {
            let t = 0.2 + f64::from(k) * 5.3;
            let octets = (t / 10.0).floor() as u64 * 1_250_000_000;
            let at = t0 + Duration::from_secs_f64(t);
            let (np, smp) = read_ports(&s, &[ticked(1, octets, 0, Some((t * 100.0) as u32))], &p, &mut counters, at);
            p = np;
            rates.extend(smp.iter().map(|x| x.in_bps as f64 / 1e9));
        }
        assert!(rates.len() > 90, "{}", rates.len());
        // Never a spike; once the moves show the cached counters, the refresh period
        // (their average spacing) as interval: flat within 3 %.
        assert!(rates.iter().all(|r| (0.9..1.1).contains(r)), "{rates:?}");
        let late = &rates[8..];
        assert!(late.iter().all(|r| (0.97..1.03).contains(r)), "{late:?}");
        // One read late (a slow answer): three reads between two moves, still one refresh.
        let mut c = HashMap::new();
        let mut p = Vec::new();
        let mut rates = Vec::new();
        let mut t = 0.0_f64;
        for k in 0..60u32 {
            t += if k == 30 { 9.0 } else { 5.0 };
            let octets = ((t + 2.5) / 10.0).floor() as u64 * 1_250_000_000;
            let (np, smp) = read_ports(&s, &[ticked(1, octets, 0, Some((t * 100.0) as u32))], &p, &mut c, t0 + Duration::from_secs_f64(t));
            p = np;
            rates.extend(smp.iter().map(|x| x.in_bps as f64 / 1e9));
        }
        assert!(rates[2..].iter().all(|r| (0.95..1.05).contains(r)), "{rates:?}");
    }

    #[test]
    fn ports_match_by_name() {
        let s = sw();
        let stats = [stat(1, "Vlan1", 0, 0), stat(2, "GigabitEthernet1/0/12", 0, 0), stat(3, "GigabitEthernet1/0/2", 0, 0), stat(4, "TenGigabitEthernet1/1/1", 0, 0), stat(5, "", 0, 0)];
        assert_eq!(match_ports(&s, &stats), vec![(2, 2), (5, 3)]);
    }

    fn lp(n: u16, admin_up: bool, alias: &str) -> LivePort {
        LivePort { n, if_index: u32::from(n), admin_up, up: admin_up, alias: alias.into(), ..LivePort::default() }
    }

    fn pv(n: u16, pvid: u16, untagged: &[u16], tagged: &[u16]) -> PortVlans {
        PortVlans { n, pvid: Some(pvid), untagged: untagged.to_vec(), tagged: tagged.to_vec() }
    }

    fn live_of(s: &Switch) -> Live {
        let ports = vec![lp(1, true, ""), lp(2, true, "ptz3"), lp(3, false, ""), lp(4, true, ""), lp(5, true, "uplink")];
        let vlans = vec![pv(1, 10, &[10], &[]), pv(2, 10, &[10], &[]), pv(3, 99, &[99], &[]), pv(4, 1, &[1], &[30, 40]), pv(5, 1, &[1], &[10, 20])];
        let diffs = diffs(s, &ports, &vlans);
        Live { at: 1, ports, vlans: Some(vlans), diffs, ..Live::default() }
    }

    fn inventory() -> Switch {
        let mut s = sw();
        s.ports[0].vlan = Some(10);
        s.ports[1].vlan = Some(20);
        s.ports[1].label = String::new();
        s.ports[2].vlan = Some(99);
        s.ports[3].vlan = Some(30);
        s.ports[4].mode = "trunk".into();
        s.ports[4].tagged = vec![10, 20];
        s
    }

    #[test]
    fn differences() {
        let s = inventory();
        let l = live_of(&s);
        let d: Vec<(u16, &str, &str, &str)> = l.diffs.iter().map(|d| (d.n, d.field, d.switch.as_str(), d.inventory.as_str())).collect();
        assert_eq!(
            d,
            vec![(2, "vlan", "VLAN 10", "VLAN 20"), (3, "off", "disabled", "access VLAN 99"), (4, "mode", "trunk 30, 40", "access VLAN 30")]
        );
        // A trunk's native VLAN counts only when the inventory sets one.
        let mut s2 = s.clone();
        s2.ports[4].vlan = Some(10);
        assert_eq!(diffs(&s2, &l.ports, l.vlans.as_deref().unwrap()).last().map(|d| (d.n, d.field)), Some((5, "vlan")));
        s2.ports[4].tagged = vec![10];
        assert_eq!(diffs(&s2, &l.ports, l.vlans.as_deref().unwrap()).last().map(|d| (d.n, d.field, d.switch.clone())), Some((5, "tagged", "trunk 10, 20".to_string())));
        // VLAN 1 (the switch's default) and no VLAN in the inventory: nothing to say.
        let one = [pv(1, 1, &[1], &[])];
        let mut bare = sw();
        bare.ports[0].vlan = None;
        assert!(diffs(&bare, &l.ports[..1], &one).is_empty());
        bare.ports[0].vlan = Some(10);
        assert_eq!(diffs(&bare, &l.ports[..1], &one)[0].switch, "VLAN 1");
        // Disabled on both sides: nothing to say.
        s2.ports[2].mode = "off".into();
        assert!(diffs(&s2, &l.ports, l.vlans.as_deref().unwrap()).iter().all(|d| d.n != 3));
    }

    #[test]
    fn taking_from_the_switch() {
        let mut s = inventory();
        let l = live_of(&s);
        assert_eq!(take(&mut s, &l, Some(&[2]), &[]), vec!["Gi1/0/2".to_string()]);
        assert_eq!((s.ports[1].vlan, s.ports[1].label.as_str()), (Some(10), "ptz3"));
        assert_eq!(s.ports[3].mode, "access");
        let names = take(&mut s, &l, None, &[]);
        assert_eq!(names, vec!["Gi1/0/2", "Gi1/0/3", "Gi1/0/4"]);
        assert_eq!((s.ports[2].mode.as_str(), s.ports[2].vlan), ("off", Some(99)));
        // VLAN 1 (native here) is the switch's default: no VLAN in an inventory that has no VLAN 1.
        assert_eq!((s.ports[3].mode.as_str(), s.ports[3].vlan, s.ports[3].tagged.clone()), ("trunk", None, vec![30, 40]));
        let mut with_one = inventory();
        take(&mut with_one, &l, Some(&[4]), &[1, 10]);
        assert_eq!(with_one.ports[3].vlan, Some(1));
        // A label set by hand stays.
        s.ports[4].label = "Uplink cœur".into();
        take(&mut s, &l, Some(&[5]), &[]);
        assert_eq!(s.ports[4].label, "Uplink cœur");
        assert!(diffs(&s, &l.ports, l.vlans.as_deref().unwrap()).is_empty());
        // Nothing is taken before the VLAN setup is known.
        let blind = Live { vlans: None, ..l };
        assert!(take(&mut s, &blind, None, &[]).is_empty());
    }
}
