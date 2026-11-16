//! Online switches, slow reads: CDP / LLDP neighbours, optics, IGMP snooping,
//! health, aggregates, spanning tree and PoE (`strata_net::switchinfo`, read-only
//! SNMP) every minute; the alert rules of the switch; the neighbour switches not
//! yet linked in the inventory.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use strata_net::switchinfo::{self as si, PoeStatus, Proto, StpState, SwitchInfo};

use super::live::{Diff, LivePort};
use super::model::{AlertRules, PortRef, Switch, SwitchPort};
use super::snmp::session;
use super::{File, Net};
use crate::paths::now_ms;

/// Time between two reads.
const EVERY: Duration = Duration::from_secs(60);
/// CPU readings kept (24 h at one a minute).
const CPU_KEEP: i64 = 24 * 3_600_000;
const FLAP_WINDOW: i64 = 600_000;
const SATURATION_FOR: Duration = Duration::from_secs(60);

/// The slow reads of an online switch (in memory only).
#[derive(Default)]
pub struct InfoMem {
    pub data: Option<SwitchInfo>,
    /// Last successful read (unix ms).
    pub at: i64,
    pub error: Option<String>,
    /// (unix ms, CPU %).
    pub cpu: VecDeque<(i64, f32)>,
}

// ── The view sent to the clients ──

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortV {
    /// Faceplate port, when the interface is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u16>,
    /// Interface name.
    pub port: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighbourV {
    #[serde(flatten)]
    pub at: PortV,
    pub name: String,
    pub remote_port: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<Ipv4Addr>,
    pub platform: String,
    pub chassis_id: String,
    pub protocol: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpticV {
    #[serde(flatten)]
    pub at: PortV,
    pub kind: String,
    pub vendor: String,
    pub part: String,
    pub serial: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_dbm: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_dbm: Option<f32>,
    pub rx_lanes_dbm: Vec<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bias_ma: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voltage: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_low_warn_dbm: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_low_alarm_dbm: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupV {
    pub vlan: u16,
    /// The IP group: read, else the one stream seen whose MAC matches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Ipv4Addr>,
    pub mac: String,
    /// Label set by hand, else the SAP name of the stream.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub ports: Vec<PortV>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterPortsV {
    pub vlan: u16,
    pub ports: Vec<PortV>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TempV {
    pub name: String,
    pub c: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitV {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watts: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthV {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu5min_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_used_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_total_bytes: Option<u64>,
    pub temps: Vec<TempV>,
    pub fans: Vec<UnitV>,
    pub psus: Vec<UnitV>,
    /// CPU % read every minute, the last 24 h: [unix ms, %].
    pub cpu: Vec<(i64, f32)>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LagV {
    pub n: u16,
    pub port: String,
    /// The aggregate's interface name (`port-channel1`, `Po1`).
    pub agg: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StpV {
    pub n: u16,
    pub state: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoePortV {
    #[serde(flatten)]
    pub at: PortV,
    pub admin_on: bool,
    pub on: bool,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watts: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoeV {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_w: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_w: Option<f32>,
    pub ports: Vec<PoePortV>,
}

/// A neighbour that is another switch of the inventory, not linked to this port yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkV {
    /// This switch's port.
    pub n: u16,
    pub switch_id: String,
    /// The other switch's port.
    pub port: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoView {
    /// Last successful read (unix ms); 0 before the first one.
    pub at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The parts the switch did not answer, and why.
    pub errors: Vec<String>,
    pub neighbours: Vec<NeighbourV>,
    pub optics: Vec<OpticV>,
    pub mcast: Vec<GroupV>,
    pub router_ports: Vec<RouterPortsV>,
    pub health: HealthV,
    pub lags: Vec<LagV>,
    pub stp: Vec<StpV>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poe: Option<PoeV>,
    pub links: Vec<LinkV>,
}

fn stp_name(s: StpState) -> &'static str {
    match s {
        StpState::Disabled => "disabled",
        StpState::Blocking => "blocking",
        StpState::Listening => "listening",
        StpState::Learning => "learning",
        StpState::Forwarding => "forwarding",
        StpState::Broken => "broken",
    }
}

fn poe_name(s: PoeStatus) -> &'static str {
    match s {
        PoeStatus::Disabled => "disabled",
        PoeStatus::Searching => "searching",
        PoeStatus::Delivering => "delivering",
        PoeStatus::Fault => "fault",
        PoeStatus::Test => "test",
        PoeStatus::OtherFault => "otherFault",
        PoeStatus::Unknown => "unknown",
    }
}

/// How interfaces map to the faceplate: ifIndex → name, ifIndex → port.
#[derive(Default)]
pub struct Ifs {
    pub names: HashMap<u32, String>,
    pub ports: HashMap<u32, u16>,
}

impl Ifs {
    pub fn of(names: &HashMap<u32, String>, ports: &[LivePort]) -> Ifs {
        Ifs { names: names.clone(), ports: ports.iter().map(|p| (p.if_index, p.n)).collect() }
    }

    fn port(&self, i: u32) -> PortV {
        PortV { n: self.ports.get(&i).copied(), port: self.names.get(&i).cloned().unwrap_or_else(|| format!("if{i}")) }
    }
}

/// Another switch of the inventory, as a neighbour could name it.
pub struct Other<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub ip: Option<Ipv4Addr>,
    pub agent_ip: Option<Ipv4Addr>,
    /// Its name in the switch (read when it is online).
    pub sysname: Option<String>,
    pub ports: &'a [SwitchPort],
}

/// The neighbours that are another inventory switch (by name, sysName or IP), on
/// one of its ports (not linked to a third port), and not linked to this port yet.
pub fn link_candidates(sw: &Switch, neighbours: &[NeighbourV], others: &[Other]) -> Vec<LinkV> {
    let same_name = |a: &str, b: &str| !a.is_empty() && si::clean_device_id(a).eq_ignore_ascii_case(&si::clean_device_id(b));
    let mut out: Vec<LinkV> = Vec::new();
    for nb in neighbours {
        let Some(n) = nb.at.n else { continue };
        if out.iter().any(|l| l.n == n) {
            continue;
        }
        let found = others.iter().filter(|o| o.id != sw.id).find_map(|o| {
            let by_name = same_name(&nb.name, o.name) || o.sysname.as_deref().is_some_and(|s| same_name(&nb.name, s));
            let by_ip = nb.ip.is_some() && (nb.ip == o.ip || nb.ip == o.agent_ip);
            if !by_name && !by_ip {
                return None;
            }
            let p = o.ports.iter().find(|p| !p.name.is_empty() && si::same_port(&nb.remote_port, &p.name))?;
            // Its port linked to another port already: left as it is.
            let elsewhere = p.peer.as_ref().is_some_and(|r| r.id != sw.id || r.port != n);
            (!elsewhere).then_some((o.id, p.n))
        });
        let Some((id, m)) = found else { continue };
        let linked = sw.ports.iter().find(|p| p.n == n).and_then(|p| p.peer.as_ref()).is_some_and(|r| r.id == id && r.port == m);
        if !linked {
            out.push(LinkV { n, switch_id: id.to_string(), port: m });
        }
    }
    out
}

/// Builds the view of what was read (ports named, mapped to the faceplate).
pub fn view(mem: &InfoMem, ifs: &Ifs, streams: &[(Ipv4Addr, Option<String>)]) -> InfoView {
    let mut v = InfoView { at: mem.at, error: mem.error.clone(), ..InfoView::default() };
    let Some(d) = &mem.data else { return v };
    v.errors = d.errors.clone();
    v.neighbours = d
        .neighbours
        .iter()
        .map(|n| NeighbourV {
            at: ifs.port(n.if_index),
            name: n.remote_name.clone(),
            remote_port: n.remote_port.clone(),
            ip: n.remote_ip,
            platform: n.platform.clone(),
            chassis_id: n.chassis_id.clone(),
            protocol: if n.protocol == Proto::Cdp { "cdp" } else { "lldp" },
        })
        .collect();
    v.optics = d
        .optics
        .iter()
        .map(|o| OpticV {
            at: ifs.port(o.if_index),
            kind: o.kind.clone(),
            vendor: o.vendor.clone(),
            part: o.part.clone(),
            serial: o.serial.clone(),
            rx_dbm: o.rx_dbm,
            tx_dbm: o.tx_dbm,
            rx_lanes_dbm: o.rx_lanes_dbm.clone(),
            temp_c: o.temp_c,
            bias_ma: o.bias_ma,
            voltage: o.voltage,
            rx_low_warn_dbm: o.rx_low_warn_dbm,
            rx_low_alarm_dbm: o.rx_low_alarm_dbm,
        })
        .collect();
    v.mcast = d
        .mcast_groups
        .iter()
        .map(|g| {
            let group = g.group.or_else(|| {
                let mut seen = streams.iter().map(|s| s.0).filter(|ip| si::group_mac(*ip) == g.mac).collect::<Vec<_>>();
                seen.dedup();
                (seen.len() == 1).then(|| seen[0])
            });
            let name = group.and_then(|ip| streams.iter().find(|s| s.0 == ip && s.1.is_some()).and_then(|s| s.1.clone()));
            GroupV { vlan: g.vlan, group, mac: strata_net::mac::format(&g.mac), name, ports: g.ports.iter().map(|i| ifs.port(*i)).collect() }
        })
        .collect();
    v.router_ports = d.mcast_router_ports.iter().map(|(vlan, ports)| RouterPortsV { vlan: *vlan, ports: ports.iter().map(|i| ifs.port(*i)).collect() }).collect();
    let h = &d.health;
    v.health = HealthV {
        cpu_pct: h.cpu_pct,
        cpu5min_pct: h.cpu_5min_pct,
        mem_used_bytes: h.mem_used_bytes,
        mem_total_bytes: h.mem_total_bytes,
        temps: h.temps.iter().map(|(name, c)| TempV { name: name.clone(), c: *c }).collect(),
        fans: h.fans.iter().map(|f| UnitV { name: f.name.clone(), ok: f.ok, detail: f.detail.clone(), watts: None }).collect(),
        psus: h.psus.iter().map(|p| UnitV { name: p.name.clone(), ok: p.ok, detail: p.detail.clone(), watts: p.watts }).collect(),
        cpu: mem.cpu.iter().copied().collect(),
    };
    let agg_of: HashMap<u32, u32> = d.lags.iter().copied().collect();
    let mut faceplate: Vec<(u32, u16)> = ifs.ports.iter().map(|(i, n)| (*i, *n)).collect();
    faceplate.sort_by_key(|x| x.1);
    v.lags = faceplate
        .iter()
        .filter_map(|(i, n)| agg_of.get(i).map(|a| LagV { n: *n, port: ifs.port(*i).port, agg: ifs.port(*a).port }))
        .collect();
    let stp: HashMap<u32, StpState> = d.stp.iter().copied().collect();
    // A member of an aggregate takes the state of the aggregate's bridge port.
    v.stp = faceplate.iter().filter_map(|(i, n)| stp.get(i).or_else(|| agg_of.get(i).and_then(|a| stp.get(a))).map(|s| StpV { n: *n, state: stp_name(*s) })).collect();
    v.poe = d.poe.as_ref().map(|p| PoeV {
        budget_w: p.budget_w,
        used_w: p.used_w,
        ports: p
            .ports
            .iter()
            .map(|x| PoePortV {
                at: if x.if_index == 0 { PortV { n: None, port: format!("{}/{}", x.group, x.port) } } else { ifs.port(x.if_index) },
                admin_on: x.admin_on,
                on: x.on,
                status: poe_name(x.status),
                class: x.class,
                watts: x.watts,
            })
            .collect(),
    });
    v
}

// ── Alert rules ──

/// An alert to raise.
#[derive(Clone, Debug, PartialEq)]
pub struct Raise {
    pub kind: &'static str,
    pub port: Option<u16>,
    pub detail: String,
    pub value: Option<f64>,
    pub limit: Option<f64>,
}

/// What the rules remember between two reads.
#[derive(Default)]
pub struct Watch {
    /// Conditions alerted and not cleared since: alerted once until they clear.
    active: HashSet<String>,
    up: HashMap<u16, bool>,
    /// Link drops (unix ms) in the last 10 minutes.
    drops: HashMap<u16, VecDeque<i64>>,
    /// In + out errors, and when.
    errors: HashMap<u16, (u64, Instant)>,
    /// Since when each port is above the saturation limit.
    hot: HashMap<u16, Instant>,
    /// Neighbours at the last read (port, remote name, remote port); None before the first.
    neighbours: Option<HashSet<(String, String, String)>>,
    /// The differences were seen once (those already there when the switch went online are not new).
    drift_known: bool,
}

/// One evaluation pass over some condition families.
struct Pass<'a> {
    w: &'a mut Watch,
    seen: HashSet<String>,
    out: Vec<Raise>,
}

impl<'a> Pass<'a> {
    fn new(w: &'a mut Watch) -> Pass<'a> {
        Pass { w, seen: HashSet::new(), out: Vec::new() }
    }

    /// The condition `key` holds: raised the first time.
    fn hit(&mut self, key: String, r: Raise) {
        if self.w.active.insert(key.clone()) {
            self.out.push(r);
        }
        self.seen.insert(key);
    }

    /// The conditions of these families not seen in this pass are cleared.
    fn finish(self, families: &[&str]) -> Vec<Raise> {
        let seen = self.seen;
        self.w.active.retain(|k| !families.iter().any(|f| k.starts_with(f)) || seen.contains(k));
        self.out
    }
}

fn raise(kind: &'static str, port: Option<u16>, detail: impl Into<String>, value: Option<f64>, limit: Option<f64>) -> Raise {
    Raise { kind, port, detail: detail.into(), value, limit }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// The rules on a live read: watched ports down, flapping, errors, saturation, new differences.
pub fn check_live(w: &mut Watch, rules: &AlertRules, sw: &Switch, ports: &[LivePort], diffs: Option<&[Diff]>, now_ms: i64, now: Instant) -> Vec<Raise> {
    let mut pass = Pass::new(w);
    for lp in ports {
        let Some(p) = sw.ports.iter().find(|p| p.n == lp.n) else { continue };
        let n = lp.n;
        // Link drops.
        let was_up = pass.w.up.insert(n, lp.up);
        let drops = pass.w.drops.entry(n).or_default();
        if was_up == Some(true) && !lp.up {
            drops.push_back(now_ms);
        }
        while drops.front().is_some_and(|t| *t < now_ms - FLAP_WINDOW) {
            drops.pop_front();
        }
        let drop_count = drops.len();
        let flapping = rules.flap.on && drop_count as f32 > rules.flap.value;
        if flapping {
            pass.hit(format!("flap:{n}"), raise("portFlap", Some(n), "", Some(drop_count as f64), Some(f64::from(rules.flap.value))));
        }
        if rules.link_down.on && p.watched && lp.admin_up && !lp.up {
            let key = format!("down:{n}");
            if flapping {
                // A flapping port is reported once as unstable, not at every drop.
                pass.w.active.insert(key.clone());
                pass.seen.insert(key);
            } else {
                pass.hit(key, raise("portDown", Some(n), p.label.clone(), None, None));
            }
        }
        // Errors per minute since the last read.
        let errs = lp.in_errors + lp.out_errors;
        if let Some((prev, at)) = pass.w.errors.insert(n, (errs, now)) {
            let secs = now.duration_since(at).as_secs_f64();
            if secs > 0.0 && errs >= prev {
                let per_min = (errs - prev) as f64 * 60.0 / secs;
                if rules.errors.on && per_min > f64::from(rules.errors.value) {
                    pass.hit(format!("errors:{n}"), raise("portErrors", Some(n), "", Some(per_min.round()), Some(f64::from(rules.errors.value))));
                } else if pass.w.active.contains(&format!("errors:{n}")) && per_min > 0.0 && rules.errors.on {
                    // Still erring, under the limit: not cleared until it stops.
                    pass.seen.insert(format!("errors:{n}"));
                }
            }
        }
        // Saturation for a minute.
        let pct = if lp.up && lp.speed_mbps > 0 { lp.in_bps.max(lp.out_bps) as f64 / (lp.speed_mbps as f64 * 1e6) * 100.0 } else { 0.0 };
        if rules.saturation.on && pct > f64::from(rules.saturation.value) {
            let since = *pass.w.hot.entry(n).or_insert(now);
            if now.duration_since(since) >= SATURATION_FOR || pass.w.active.contains(&format!("saturation:{n}")) {
                pass.hit(format!("saturation:{n}"), raise("portSaturated", Some(n), "", Some(round1(pct)), Some(f64::from(rules.saturation.value))));
            }
        } else {
            pass.w.hot.remove(&n);
        }
    }
    let mut families = vec!["down:", "flap:", "errors:", "saturation:"];
    // New differences, once the VLAN setup is known.
    if let Some(diffs) = diffs {
        let first = !pass.w.drift_known;
        pass.w.drift_known = true;
        for d in diffs {
            let key = format!("drift:{}:{}:{}:{}", d.n, d.field, d.switch, d.inventory);
            if first || !rules.drift.on {
                // Already there, or not watched: known, not raised.
                pass.w.active.insert(key.clone());
                pass.seen.insert(key);
            } else {
                pass.hit(key, raise("configDrift", Some(d.n), format!("{} / {}", d.switch, d.inventory), None, None));
            }
        }
        families.push("drift:");
    }
    pass.finish(&families)
}

/// The rules on a slow read: optics, power supplies, fans, temperatures, PoE budget, neighbours.
pub fn check_info(w: &mut Watch, rules: &AlertRules, info: &SwitchInfo, ifs: &Ifs) -> Vec<Raise> {
    let failed = |part: &str| info.errors.iter().any(|e| e.starts_with(part));
    let mut pass = Pass::new(w);
    let mut families = Vec::new();
    if !failed("optics") {
        families.push("optic:");
        if rules.optic_rx.on {
            for o in &info.optics {
                let Some(rx) = o.rx_dbm else { continue };
                if rx < rules.optic_rx.value {
                    let at = ifs.port(o.if_index);
                    pass.hit(format!("optic:{}", at.port), raise("opticLow", at.n, if at.n.is_some() { String::new() } else { at.port }, Some(round1(f64::from(rx))), Some(f64::from(rules.optic_rx.value))));
                }
            }
        }
    }
    if !failed("health") {
        families.extend(["psu:", "fan:", "temp:"]);
        if rules.health.on {
            for p in info.health.psus.iter().filter(|p| !p.ok) {
                pass.hit(format!("psu:{}", p.name), raise("psuFault", None, format!("{} · {}", p.name, p.detail), None, None));
            }
            for f in info.health.fans.iter().filter(|f| !f.ok) {
                pass.hit(format!("fan:{}", f.name), raise("fanFault", None, format!("{} · {}", f.name, f.detail), None, None));
            }
            for (name, c) in info.health.temps.iter().filter(|t| t.1 > rules.health.value) {
                pass.hit(format!("temp:{name}"), raise("tempHigh", None, name.clone(), Some(round1(f64::from(*c))), Some(f64::from(rules.health.value))));
            }
        }
    }
    if !failed("poe") {
        families.push("poe");
        if let Some((budget, used)) = info.poe.as_ref().and_then(|p| Some((p.budget_w?, p.used_w?))) {
            let pct = if budget > 0.0 { f64::from(used) / f64::from(budget) * 100.0 } else { 0.0 };
            if rules.poe.on && pct > f64::from(rules.poe.value) {
                pass.hit("poe".into(), raise("poeBudget", None, format!("{used} / {budget} W"), Some(round1(pct)), Some(f64::from(rules.poe.value))));
            }
        }
    }
    let mut out = pass.finish(&families);
    if !failed("neighbours") {
        let now: HashSet<(String, String, String)> = info.neighbours.iter().map(|n| (ifs.port(n.if_index).port, n.remote_name.clone(), n.remote_port.clone())).collect();
        if let Some(before) = w.neighbours.replace(now.clone()) {
            if rules.neighbours.on {
                let port_of = |name: &str| ifs.names.iter().find(|(_, x)| x.as_str() == name).and_then(|(i, _)| ifs.ports.get(i).copied());
                let mut added: Vec<_> = now.difference(&before).collect();
                let mut gone: Vec<_> = before.difference(&now).collect();
                added.sort();
                gone.sort();
                for (kind, list) in [("neighbourNew", added), ("neighbourGone", gone)] {
                    for (port, name, remote) in list {
                        let n = port_of(port);
                        let detail = if n.is_some() { format!("{name} {remote}") } else { format!("{port} · {name} {remote}") };
                        out.push(raise(kind, n, detail.trim().to_string(), None, None));
                    }
                }
            }
        }
    }
    out
}

// ── Reading ──

impl Net {
    /// Starts the slow reads of an online switch, unless they run.
    pub(super) fn start_info(self: &Arc<Self>, id: &str, r: &mut super::live::Runner) {
        if r.info_task.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        let net = self.clone();
        let key = id.to_string();
        r.info_task = Some(tokio::spawn(async move { net.info_loop(key).await }).abort_handle());
    }

    async fn info_loop(self: Arc<Self>, id: String) {
        loop {
            let online = self.store.lock().unwrap().switches.iter().any(|s| s.id == id && s.online);
            if !online || !self.live.lock().unwrap().contains_key(&id) {
                break;
            }
            self.read_info(&id).await;
            tokio::time::sleep(EVERY).await;
        }
    }

    async fn read_info(self: &Arc<Self>, id: &str) {
        let agent = {
            let st = self.store.lock().unwrap();
            let sw = st.switches.iter().find(|s| s.id == id);
            sw.and_then(|sw| sw.snmp.as_deref()).and_then(|a| st.snmp.iter().find(|x| x.id == a)).cloned()
        };
        let result = async {
            let agent = agent.ok_or_else(|| "the switch has no snmp agent".to_string())?;
            let session = session(&agent).await?;
            let info = si::all(&session).await;
            // Nothing answered: keep what was read before.
            if info.errors.len() >= 8 {
                return Err(info.errors.first().cloned().unwrap_or_default());
            }
            Ok(info)
        }
        .await;
        let Some(sw) = self.store.lock().unwrap().switches.iter().find(|s| s.id == id).cloned() else { return };
        let raised = {
            let mut map = self.live.lock().unwrap();
            let Some(r) = map.get_mut(id) else { return };
            match result {
                Ok(info) => {
                    let at = now_ms();
                    if let Some(cpu) = info.health.cpu_pct {
                        r.info.cpu.push_back((at, cpu));
                        while r.info.cpu.front().is_some_and(|x| x.0 < at - CPU_KEEP) {
                            r.info.cpu.pop_front();
                        }
                    }
                    let ifs = Ifs::of(r.names(), &r.live.ports);
                    let raised = check_info(&mut r.watch, &sw.rules, &info, &ifs);
                    r.info.data = Some(info);
                    r.info.at = at;
                    r.info.error = None;
                    raised
                }
                Err(e) => {
                    tracing::warn!("info read of {}: {e}", sw.name);
                    r.info.error = Some(e);
                    vec![]
                }
            }
        };
        self.raise_switch(&sw, raised);
        self.send_info(id);
    }

    /// What the slow reads found, for the clients (None when the switch is not online).
    pub fn info_of(&self, id: &str) -> Option<InfoView> {
        let (sw, others_data, labels) = {
            let st = self.store.lock().unwrap();
            let sw = st.switches.iter().find(|s| s.id == id)?.clone();
            // The other switches and their agent's IP.
            let others: Vec<(Switch, Option<Ipv4Addr>)> =
                st.switches.iter().filter(|s| s.id != id).map(|s| (s.clone(), s.snmp.as_deref().and_then(|a| st.snmp.iter().find(|x| x.id == a)).map(|a| a.ip))).collect();
            (sw, others, st.multicast.labels.clone())
        };
        let (mut view, sysnames) = {
            let map = self.live.lock().unwrap();
            let r = map.get(id)?;
            let ifs = Ifs::of(r.names(), &r.live.ports);
            let streams = self.stream_names(&labels);
            let sysnames: HashMap<String, String> = map.iter().filter_map(|(k, x)| Some((k.clone(), x.live.system.as_ref()?.name.clone()))).collect();
            (view(&r.info, &ifs, &streams), sysnames)
        };
        let others: Vec<Other> = others_data
            .iter()
            .map(|(o, agent_ip)| Other { id: &o.id, name: &o.name, ip: o.ip, agent_ip: *agent_ip, sysname: sysnames.get(&o.id).cloned(), ports: &o.ports })
            .collect();
        view.links = link_candidates(&sw, &view.neighbours, &others);
        Some(view)
    }

    /// The multicast streams seen: group and name (label set by hand, else SAP name).
    fn stream_names(&self, labels: &std::collections::BTreeMap<String, String>) -> Vec<(Ipv4Addr, Option<String>)> {
        let streams = self.mcast.lock().unwrap().as_ref().map(|e| e.snapshot()).unwrap_or_default();
        streams
            .into_iter()
            .map(|s| {
                let key = format!("{}:{}:{}", s.group, s.port, s.source.map(|x| x.to_string()).unwrap_or_default());
                (s.group, labels.get(&key).cloned().or(s.name))
            })
            .collect()
    }

    pub(super) fn send_info(&self, id: &str) {
        if let Some(info) = self.info_of(id) {
            self.send(&serde_json::json!({"type": "switchInfo", "id": id, "info": info}));
        }
    }

    /// Raises the alerts of a switch.
    pub(super) fn raise_switch(&self, sw: &Switch, list: Vec<Raise>) {
        if list.is_empty() {
            return;
        }
        let mut raised = false;
        {
            let mut st = self.store.lock().unwrap();
            let agent_ip = sw.snmp.as_deref().and_then(|a| st.snmp.iter().find(|x| x.id == a)).map(|a| a.ip);
            let ip = sw.ip.or(agent_ip).unwrap_or(Ipv4Addr::UNSPECIFIED);
            for r in list {
                raised |= self.switch_alert(&mut st, sw, ip, r);
            }
        }
        if raised {
            self.save(&[File::Alerts]);
            self.changed("alerts");
        }
    }

    /// Records the neighbour switches found on these ports as linked (both ends). Returns the links made.
    pub fn link_neighbours(&self, id: &str) -> Option<Vec<(String, String, String)>> {
        let links = self.info_of(id)?.links;
        let mut st = self.store.lock().unwrap();
        let mut made = Vec::new();
        for l in links {
            let Some(other) = st.switches.iter_mut().find(|s| s.id == l.switch_id) else { continue };
            let Some(op) = other.ports.iter_mut().find(|p| p.n == l.port) else { continue };
            op.peer = Some(PortRef { id: id.to_string(), port: l.n });
            op.host = None;
            let (other_name, other_port) = (other.name.clone(), op.name.clone());
            let Some(sw) = st.switches.iter_mut().find(|s| s.id == id) else { continue };
            let Some(p) = sw.ports.iter_mut().find(|p| p.n == l.n) else { continue };
            p.peer = Some(PortRef { id: l.switch_id.clone(), port: l.port });
            p.host = None;
            made.push((p.name.clone(), other_name, other_port));
        }
        Some(made)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_net::switchinfo::{Health, Optic, Poe, PortNeighbour, Psu, SnoopGroup, Unit};

    fn sw() -> Switch {
        let mut s: Switch = serde_json::from_value(serde_json::json!({"id": "a", "name": "SW-A", "groups": [{"kind": "sfp", "count": 4, "naming": "Eth1/{n}"}]})).unwrap();
        s.normalize();
        s
    }

    fn lp(n: u16, up: bool, errors: u64, bps: u64) -> LivePort {
        LivePort { n, if_index: 100 + u32::from(n), up, admin_up: true, speed_mbps: 1000, in_bps: bps, in_errors: errors, ..LivePort::default() }
    }

    fn ifs() -> Ifs {
        let names = (1..=4).map(|n| (100 + n, format!("Ethernet1/{n}"))).chain([(900, "port-channel1".to_string())]).collect();
        Ifs { names, ports: (1..=4).map(|n| (100 + n, n as u16)).collect() }
    }

    #[test]
    fn live_rules() {
        let mut s = sw();
        s.ports[0].watched = true;
        s.ports[0].label = "CAM".into();
        let rules = AlertRules::default();
        let mut w = Watch::default();
        let t0 = Instant::now();
        let kinds = |r: &[Raise]| r.iter().map(|x| (x.kind, x.port)).collect::<Vec<_>>();
        assert!(check_live(&mut w, &rules, &s, &[lp(1, true, 0, 0), lp(2, true, 0, 0)], None, 0, t0).is_empty());
        // Watched port down: once, until it comes back.
        let r = check_live(&mut w, &rules, &s, &[lp(1, false, 0, 0), lp(2, true, 0, 0)], None, 10_000, t0 + Duration::from_secs(10));
        assert_eq!(kinds(&r), vec![("portDown", Some(1))]);
        assert_eq!(r[0].detail, "CAM");
        assert!(check_live(&mut w, &rules, &s, &[lp(1, false, 0, 0)], None, 20_000, t0 + Duration::from_secs(20)).is_empty());
        // Flapping: 4 drops in 10 minutes (> 3).
        let mut raised = vec![];
        for k in 0..8u64 {
            let at = 30_000 + k as i64 * 10_000;
            raised.extend(check_live(&mut w, &rules, &s, &[lp(1, k % 2 == 0, 0, 0)], None, at, t0 + Duration::from_secs(30 + k * 10)));
        }
        // The fourth drop: unstable, and its drops are no longer reported one by one.
        assert_eq!(kinds(&raised), vec![("portDown", Some(1)), ("portDown", Some(1)), ("portFlap", Some(1))]);
        assert_eq!(raised[2].value, Some(4.0));
        // Errors: 300 in 60 s = 300 / min > 100; saturation after a minute at 95 %.
        let mut w = Watch::default();
        let t = |s| t0 + Duration::from_secs(s);
        check_live(&mut w, &rules, &s, &[lp(2, true, 1000, 950_000_000)], None, 0, t(0));
        let r = check_live(&mut w, &rules, &s, &[lp(2, true, 1300, 950_000_000)], None, 30_000, t(60));
        assert_eq!(kinds(&r), vec![("portErrors", Some(2)), ("portSaturated", Some(2))]);
        assert_eq!((r[0].value, r[1].value, r[1].limit), (Some(300.0), Some(95.0), Some(90.0)));
        // Rule off: cleared, nothing raised.
        let mut off = rules.clone();
        off.saturation.on = false;
        off.errors.on = false;
        assert!(check_live(&mut w, &off, &s, &[lp(2, true, 9000, 950_000_000)], None, 60_000, t(120)).is_empty());
        let r = check_live(&mut w, &rules, &s, &[lp(2, true, 9000, 950_000_000)], None, 90_000, t(121));
        assert!(r.is_empty(), "the saturation counts again from now");
    }

    #[test]
    fn drift_is_new_only() {
        let s = sw();
        let rules = AlertRules::default();
        let mut w = Watch::default();
        let d = |n: u16, sw: &str| Diff { n, field: "vlan", switch: sw.into(), inventory: "VLAN 20".into() };
        let now = Instant::now();
        assert!(check_live(&mut w, &rules, &s, &[], Some(&[d(1, "VLAN 10")]), 0, now).is_empty());
        let r = check_live(&mut w, &rules, &s, &[], Some(&[d(1, "VLAN 10"), d(2, "VLAN 30")]), 0, now);
        assert_eq!(r.iter().map(|x| (x.kind, x.port, x.detail.as_str())).collect::<Vec<_>>(), vec![("configDrift", Some(2), "VLAN 30 / VLAN 20")]);
        // Fixed, then back: new again.
        assert!(check_live(&mut w, &rules, &s, &[], Some(&[d(1, "VLAN 10")]), 0, now).is_empty());
        assert_eq!(check_live(&mut w, &rules, &s, &[], Some(&[d(1, "VLAN 10"), d(2, "VLAN 30")]), 0, now).len(), 1);
    }

    fn info() -> SwitchInfo {
        SwitchInfo {
            neighbours: vec![PortNeighbour { if_index: 103, remote_name: "sw-core-01".into(), remote_port: "Ethernet1/1".into(), remote_ip: Some(Ipv4Addr::new(10, 0, 0, 1)), platform: String::new(), chassis_id: String::new(), protocol: Proto::Cdp }],
            optics: vec![Optic { if_index: 102, rx_dbm: Some(-11.84), ..Optic::default() }, Optic { if_index: 101, rx_dbm: Some(-2.0), ..Optic::default() }],
            health: Health {
                temps: vec![("ASIC".into(), 61.0), ("Inlet".into(), 30.0)],
                psus: vec![Psu { name: "PSU-2".into(), ok: false, detail: "off (no input power)".into(), watts: None }],
                fans: vec![Unit { name: "Fan-1".into(), ok: true, detail: "up".into() }],
                ..Health::default()
            },
            poe: Some(Poe { budget_w: Some(740.0), used_w: Some(638.0), ports: vec![] }),
            mcast_groups: vec![SnoopGroup { vlan: 10, group: None, mac: si::group_mac(Ipv4Addr::new(239, 1, 2, 3)), ports: vec![101, 900] }],
            lags: vec![(103, 900), (104, 900)],
            stp: vec![(900, StpState::Blocking), (101, StpState::Forwarding)],
            ..SwitchInfo::default()
        }
    }

    #[test]
    fn info_rules() {
        let mut rules = AlertRules::default();
        let mut w = Watch::default();
        let i = info();
        let r = check_info(&mut w, &rules, &i, &ifs());
        let k: Vec<_> = r.iter().map(|x| (x.kind, x.port, x.value)).collect();
        assert_eq!(k, vec![("opticLow", Some(2), Some(-11.8)), ("psuFault", None, None), ("tempHigh", None, Some(61.0)), ("poeBudget", None, Some(86.2))]);
        assert_eq!(r[1].detail, "PSU-2 · off (no input power)");
        // Once only.
        assert!(check_info(&mut w, &rules, &i, &ifs()).is_empty());
        // A part that failed keeps its conditions.
        let mut failed = i.clone();
        failed.optics.clear();
        failed.errors = vec!["optics: timed out".into()];
        assert!(check_info(&mut w, &rules, &failed, &ifs()).is_empty());
        assert!(check_info(&mut w, &rules, &i, &ifs()).is_empty());
        // Neighbours: compared with the read before, when the rule is on.
        rules.neighbours.on = true;
        let mut gone = i.clone();
        gone.neighbours.clear();
        let r = check_info(&mut w, &rules, &gone, &ifs());
        assert_eq!(r.iter().map(|x| (x.kind, x.port, x.detail.as_str())).collect::<Vec<_>>(), vec![("neighbourGone", Some(3), "sw-core-01 Ethernet1/1")]);
        assert_eq!(check_info(&mut w, &rules, &i, &ifs())[0].kind, "neighbourNew");
    }

    #[test]
    fn view_and_links() {
        let mem = InfoMem { data: Some(info()), at: 5, ..InfoMem::default() };
        let streams = vec![(Ipv4Addr::new(239, 1, 2, 3), Some("CAM-01".to_string()))];
        let v = view(&mem, &ifs(), &streams);
        assert_eq!(v.mcast[0].group, Some(Ipv4Addr::new(239, 1, 2, 3)));
        assert_eq!(v.mcast[0].name.as_deref(), Some("CAM-01"));
        assert_eq!(v.mcast[0].ports, vec![PortV { n: Some(1), port: "Ethernet1/1".into() }, PortV { n: None, port: "port-channel1".into() }]);
        assert_eq!(v.lags.iter().map(|l| (l.n, l.agg.as_str())).collect::<Vec<_>>(), vec![(3, "port-channel1"), (4, "port-channel1")]);
        // Members take the aggregate's STP state.
        assert_eq!(v.stp.iter().map(|s| (s.n, s.state)).collect::<Vec<_>>(), vec![(1, "forwarding"), (3, "blocking"), (4, "blocking")]);
        assert_eq!(v.optics[0].at.n, Some(2));
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["neighbours"][0]["n"], 3);
        assert_eq!(json["neighbours"][0]["port"], "Ethernet1/3");
        assert_eq!(json["neighbours"][0]["remotePort"], "Ethernet1/1");

        let s = sw();
        let mut core: Switch = serde_json::from_value(serde_json::json!({"id": "c", "name": "SW-Core-01", "groups": [{"kind": "qsfp", "count": 2, "naming": "Eth1/{n}"}]})).unwrap();
        core.normalize();
        let other = |name: &'static str, ip: Option<Ipv4Addr>, sysname: Option<&str>| Other { id: "c", name, ip, agent_ip: None, sysname: sysname.map(String::from), ports: &core.ports };
        assert_eq!(link_candidates(&s, &v.neighbours, &[other("SW-Core-01", None, None)]), vec![LinkV { n: 3, switch_id: "c".into(), port: 1 }]);
        assert_eq!(link_candidates(&s, &v.neighbours, &[other("Cœur", None, Some("sw-core-01.example.lan"))]).len(), 1);
        assert_eq!(link_candidates(&s, &v.neighbours, &[other("Cœur", Some(Ipv4Addr::new(10, 0, 0, 1)), None)]).len(), 1);
        assert!(link_candidates(&s, &v.neighbours, &[other("Cœur", None, None)]).is_empty());
        // Already linked.
        let mut linked = s.clone();
        linked.ports[2].peer = Some(PortRef { id: "c".into(), port: 1 });
        assert!(link_candidates(&linked, &v.neighbours, &[other("SW-Core-01", None, None)]).is_empty());
        // The other end linked to another switch: not proposed.
        core.ports[0].peer = Some(PortRef { id: "z".into(), port: 7 });
        let taken = Other { id: "c", name: "SW-Core-01", ip: None, agent_ip: None, sysname: None, ports: &core.ports };
        assert!(link_candidates(&s, &v.neighbours, &[taken]).is_empty());
    }
}
