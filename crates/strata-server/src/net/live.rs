//! Online switches: live reads of the port states, counters and VLAN setup
//! (read-only SNMP v2c), the differences between the switch and the
//! inventory, and taking the switch's setup into the inventory.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use strata_net::snmp::{self as s, Agent, IfStat, Session, System, VlanSetup};
use tokio::task::AbortHandle;

use super::model::{Switch, SwitchPort};
use super::snmp::same_port;
use super::Net;
use crate::paths::now_ms;

/// Rates kept per port for the sparklines.
const HISTORY: usize = 60;
/// The identity and the VLAN setup are read again after this long.
const SLOW: Duration = Duration::from_secs(300);
const TIMEOUT: Duration = Duration::from_secs(2);

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

/// The counters and slow reads behind `Live`.
#[derive(Default)]
struct Memory {
    /// ifIndex → (in octets, out octets, when).
    counters: HashMap<u32, (u64, u64, Instant)>,
    system: Option<(System, Instant)>,
    system_read: Option<Instant>,
    setup: Option<VlanSetup>,
    vlans_read: Option<Instant>,
}

#[derive(Default)]
pub struct Runner {
    pub live: Live,
    mem: Memory,
    task: Option<AbortHandle>,
}

// ── Computations ──

/// Bits per second between two readings of an octet counter; `None` when the
/// counter went back (wrapped, or the switch restarted): that sample is skipped.
pub fn rate(prev: u64, cur: u64, secs: f64) -> Option<u64> {
    (secs > 0.0 && cur >= prev).then(|| ((cur - prev) as f64 * 8.0 / secs).round() as u64)
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

/// The ports after a new reading: rates from the counter deltas, history kept from `prev`.
fn read_ports(sw: &Switch, stats: &[IfStat], prev: &[LivePort], counters: &mut HashMap<u32, (u64, u64, Instant)>, now: Instant) -> Vec<LivePort> {
    let mut out = Vec::new();
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
            alias: st.alias.clone(),
            in_history: old.map(|o| o.in_history.clone()).unwrap_or_default(),
            out_history: old.map(|o| o.out_history.clone()).unwrap_or_default(),
        };
        if let Some((pin, pout, at)) = counters.get(&st.if_index) {
            let secs = now.duration_since(*at).as_secs_f64();
            if let (Some(i), Some(o)) = (rate(*pin, st.in_octets, secs), rate(*pout, st.out_octets, secs)) {
                p.in_bps = i;
                p.out_bps = o;
                push(&mut p.in_history, i);
                push(&mut p.out_history, o);
            }
        }
        counters.insert(st.if_index, (st.in_octets, st.out_octets, now));
        out.push(p);
    }
    out
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
/// description as label where the inventory has none. Returns the names of the ports taken.
pub fn take(sw: &mut Switch, live: &Live, which: Option<&[u16]>) -> Vec<String> {
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
                p.vlan = st.vlan;
                p.tagged = st.tagged;
            }
            _ => {
                p.mode = "access".into();
                p.vlan = st.vlan;
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
    /// Puts a new reading in: ports and rates, system, VLANs, differences.
    fn update(&mut self, sw: &Switch, stats: &[IfStat], now: Instant) {
        self.live.ports = read_ports(sw, stats, &self.live.ports, &mut self.mem.counters, now);
        self.live.at = now_ms();
        self.live.error = None;
        self.refresh(sw);
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
        let mut map = self.live.lock().unwrap();
        let r = map.entry(id.to_string()).or_default();
        let net = self.clone();
        let key = id.to_string();
        let task = tokio::spawn(async move { net.live_loop(key).await }).abort_handle();
        if let Some(old) = r.task.replace(task) {
            old.abort();
        }
    }

    /// Stops reading the switch and forgets what was read.
    pub fn stop_live(&self, id: &str) {
        if let Some(t) = self.live.lock().unwrap().remove(id).and_then(|r| r.task) {
            t.abort();
        }
    }

    pub fn live_of(&self, id: &str) -> Option<Live> {
        self.live.lock().unwrap().get(id).map(|r| r.live.clone())
    }

    /// The inventory of an online switch changed: differences again, sent to the clients.
    pub fn live_changed(&self, id: &str) {
        let Some(sw) = self.store.lock().unwrap().switches.iter().find(|s| s.id == id).cloned() else { return };
        let live = {
            let mut map = self.live.lock().unwrap();
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
                let st = self.store.lock().unwrap();
                match st.switches.iter().find(|s| s.id == id) {
                    Some(sw) if sw.online => sw.online_secs,
                    _ => break,
                }
            };
            self.read_live(&id).await;
            tokio::time::sleep(Duration::from_secs(u64::from(secs.max(1)))).await;
        }
        self.live.lock().unwrap().remove(&id);
    }

    /// One read: states and counters; identity and VLAN setup when due.
    async fn read_live(self: &Arc<Self>, id: &str) {
        let agent = {
            let st = self.store.lock().unwrap();
            let sw = st.switches.iter().find(|s| s.id == id);
            sw.and_then(|sw| sw.snmp.as_deref()).and_then(|a| st.snmp.iter().find(|x| x.id == a)).map(|a| Agent { ip: a.ip, port: a.port, community: a.community.clone() })
        };
        let (need_system, need_vlans) = {
            let map = self.live.lock().unwrap();
            let Some(r) = map.get(id) else { return };
            let due = |t: Option<Instant>| t.is_none_or(|t| t.elapsed() >= SLOW);
            (due(r.mem.system_read), due(r.mem.vlans_read))
        };
        let read = async {
            let agent = agent.ok_or_else(|| "the switch has no snmp agent".to_string())?;
            let session = Session::open(&agent, TIMEOUT).await.map_err(|e| e.to_string())?;
            let stats = s::if_stats(&session).await.map_err(|e| e.to_string())?;
            let system = if need_system { Some(s::system(&session).await.ok()) } else { None };
            let setup = if need_vlans { Some(s::vlan_setup(&session).await.ok()) } else { None };
            Ok::<_, String>((stats, system, setup))
        };
        let result = read.await;
        let now = Instant::now();
        let Some(sw) = self.store.lock().unwrap().switches.iter().find(|s| s.id == id).cloned() else { return };
        let live = {
            let mut map = self.live.lock().unwrap();
            let Some(r) = map.get_mut(id) else { return };
            match result {
                Ok((stats, system, setup)) => {
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
                    r.update(&sw, &stats, now);
                }
                Err(e) => {
                    tracing::warn!("live read of {}: {e}", sw.name);
                    r.live.error = Some(e);
                    r.refresh(&sw);
                }
            }
            r.live.clone()
        };
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
        let t0 = Instant::now();
        let p = read_ports(&s, &[stat(10101, "GigabitEthernet1/0/1", 1_000, 0)], &[], &mut counters, t0);
        assert_eq!((p[0].in_bps, p[0].in_history.len()), (0, 0));
        let t1 = t0 + Duration::from_secs(10);
        let p = read_ports(&s, &[stat(10101, "GigabitEthernet1/0/1", 126_000, 12_500)], &p, &mut counters, t1);
        assert_eq!((p[0].in_bps, p[0].out_bps), (100_000, 10_000));
        assert_eq!(p[0].in_history, vec![100_000]);
        // The counter went back (restart): the sample is skipped, the last rate stays.
        let t2 = t1 + Duration::from_secs(10);
        let p = read_ports(&s, &[stat(10101, "GigabitEthernet1/0/1", 50, 12_600)], &p, &mut counters, t2);
        assert_eq!((p[0].in_bps, p[0].in_history.len()), (100_000, 1));
        let t3 = t2 + Duration::from_secs(10);
        let p = read_ports(&s, &[stat(10101, "GigabitEthernet1/0/1", 1_300, 12_600)], &p, &mut counters, t3);
        assert_eq!((p[0].in_bps, p[0].in_history.len()), (1_000, 2));
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
        assert_eq!(take(&mut s, &l, Some(&[2])), vec!["Gi1/0/2".to_string()]);
        assert_eq!((s.ports[1].vlan, s.ports[1].label.as_str()), (Some(10), "ptz3"));
        assert_eq!(s.ports[3].mode, "access");
        let names = take(&mut s, &l, None);
        assert_eq!(names, vec!["Gi1/0/2", "Gi1/0/3", "Gi1/0/4"]);
        assert_eq!((s.ports[2].mode.as_str(), s.ports[2].vlan), ("off", Some(99)));
        assert_eq!((s.ports[3].mode.as_str(), s.ports[3].vlan, s.ports[3].tagged.clone()), ("trunk", Some(1), vec![30, 40]));
        // A label set by hand stays.
        s.ports[4].label = "Uplink cœur".into();
        take(&mut s, &l, Some(&[5]));
        assert_eq!(s.ports[4].label, "Uplink cœur");
        assert!(diffs(&s, &l.ports, l.vlans.as_deref().unwrap()).is_empty());
        // Nothing is taken before the VLAN setup is known.
        let blind = Live { vlans: None, ..l };
        assert!(take(&mut s, &blind, None).is_empty());
    }
}
