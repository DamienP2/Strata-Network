//! Extra switch information read over SNMP: CDP / LLDP neighbours, optics
//! (DOM), IGMP snooping groups, health (CPU, memory, temperatures, fans, power
//! supplies), link aggregates, spanning tree and PoE.
//!
//! Only GET / GETNEXT / GETBULK are sent: switches are never modified. A MIB the
//! switch does not implement gives an empty result (or `None`); only a
//! transport failure (no answer) is an error. Cisco MIBs are read first (Nexus
//! 9000 NX-OS, Catalyst 9300 IOS-XE), then the standard ones. Only the needed
//! columns of a table are walked, each walk bounded. Ports are ifIndexes.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io;
use std::net::Ipv4Addr;

use tokio::sync::OnceCell;

use crate::mac::{self, Mac};
use crate::snmp::{port_list, Oid, Session, Value};

pub mod oids {
    // IF-MIB
    pub const IF_INDEX: &str = "1.3.6.1.2.1.2.2.1.1";
    pub const IF_DESCR: &str = "1.3.6.1.2.1.2.2.1.2";
    pub const IF_TYPE: &str = "1.3.6.1.2.1.2.2.1.3"; // 6 ethernetCsmacd, 117 gigabitEthernet, 161 ieee8023adLag
    pub const IF_NAME: &str = "1.3.6.1.2.1.31.1.1.1.1";
    pub const IF_STACK_STATUS: &str = "1.3.6.1.2.1.31.1.2.1.3"; // ifStackStatus.higher.lower
                                                                // BRIDGE-MIB, Q-BRIDGE-MIB
    pub const BRIDGE_PORT_IF: &str = "1.3.6.1.2.1.17.1.4.1.2"; // dot1dBasePortIfIndex.bridgePort
    pub const STP_PORT_STATE: &str = "1.3.6.1.2.1.17.2.15.1.3"; // dot1dStpPortState.bridgePort
    pub const Q_GROUP_EGRESS: &str = "1.3.6.1.2.1.17.7.1.2.3.1.2"; // dot1qTpGroupEgressPorts.vlan.mac(6)
    pub const Q_FORWARD_ALL: &str = "1.3.6.1.2.1.17.7.1.2.4.1.1"; // dot1qForwardAllPorts.vlan
                                                                  // ENTITY-MIB entPhysicalEntry: .2 descr, .4 containedIn, .5 class, .7 name, .11 serial, .12 mfg, .13 model
    pub const ENT_PHYSICAL: &str = "1.3.6.1.2.1.47.1.1.1.1";
    pub const ENT_ALIAS: &str = "1.3.6.1.2.1.47.1.3.2.1.2"; // entAliasMappingIdentifier.ent.logical -> ifIndex.N
                                                            // CISCO-ENTITY-SENSOR-MIB entSensorValueEntry: .1 type, .2 scale, .3 precision, .4 value, .5 status
    pub const CISCO_SENSOR: &str = "1.3.6.1.4.1.9.9.91.1.1.1.1";
    // entSensorThresholdEntry (.ent.thresholdIndex): .2 severity, .3 relation, .4 value
    pub const CISCO_SENSOR_THRESHOLD: &str = "1.3.6.1.4.1.9.9.91.1.2.1.1";
    // ENTITY-SENSOR-MIB entPhySensorEntry: same columns as the Cisco table (no dBm type)
    pub const ENTITY_SENSOR: &str = "1.3.6.1.2.1.99.1.1.1";
    // CISCO-PROCESS-MIB cpmCPUTotalEntry: .4/.7 1-min %, .5/.8 5-min %, .12/.13 mem used/free KB, .17/.19 HC
    pub const CPM: &str = "1.3.6.1.4.1.9.9.109.1.1.1.1";
    // HOST-RESOURCES-MIB
    pub const HR_PROCESSOR_LOAD: &str = "1.3.6.1.2.1.25.3.3.1.2";
    pub const HR_STORAGE: &str = "1.3.6.1.2.1.25.2.3.1"; // .2 type, .4 allocation units, .5 size, .6 used
    pub const HR_STORAGE_RAM: &str = "1.3.6.1.2.1.25.2.1.2";
    // CISCO-ENTITY-FRU-CONTROL-MIB (indexed by entPhysicalIndex)
    pub const CEFC_FAN_STATUS: &str = "1.3.6.1.4.1.9.9.117.1.4.1.1.1"; // cefcFanTrayOperStatus: 2 up
    pub const CEFC_POWER_STATUS: &str = "1.3.6.1.4.1.9.9.117.1.1.2.1.2"; // cefcFRUPowerOperStatus: 2 on
                                                                         // ENTITY-STATE-MIB
    pub const ENT_STATE_OPER: &str = "1.3.6.1.2.1.131.1.1.1.3"; // entStateOper: 3 enabled
                                                                // CISCO-ENVMON-MIB (older IOS / IOS-XE): .2 descr, .3 value or state, .6 temperature state
    pub const ENVMON_TEMP: &str = "1.3.6.1.4.1.9.9.13.1.3.1";
    pub const ENVMON_FAN: &str = "1.3.6.1.4.1.9.9.13.1.4.1";
    pub const ENVMON_SUPPLY: &str = "1.3.6.1.4.1.9.9.13.1.5.1";
    // CISCO-CDP-MIB cdpCacheEntry (.ifIndex.devIndex): .4 address, .6 device id, .7 device port, .8 platform
    pub const CDP_CACHE: &str = "1.3.6.1.4.1.9.9.23.1.2.1.1";
    // LLDP-MIB
    pub const LLDP_LOC_PORT: &str = "1.0.8802.1.1.2.1.3.7.1"; // .2 id subtype, .3 id, .4 desc
    pub const LLDP_REM: &str = "1.0.8802.1.1.2.1.4.1.1"; // .4/.5 chassis, .6/.7 port id, .8 port desc, .9 sys name, .10 sys desc
    pub const LLDP_REM_MAN_ADDR: &str = "1.0.8802.1.1.2.1.4.2.1.3"; // lldpRemManAddrIfSubtype (read for its index)
                                                                    // IEEE8023-LACP-MIB
    pub const LACP_ATTACHED_AGG: &str = "1.2.840.10006.300.43.1.2.1.1.13"; // dot3adAggPortAttachedAggID.memberIf
                                                                           // POWER-ETHERNET-MIB
    pub const PETH_PORT: &str = "1.3.6.1.2.1.105.1.1.1"; // .group.port: .3 admin, .6 detection, .10 class
    pub const PETH_MAIN: &str = "1.3.6.1.2.1.105.1.3.1.1"; // .group: .2 power W, .4 consumption W
                                                           // CISCO-POWER-ETHERNET-EXT-MIB cpeExtPsePortEntry (augments pethPsePortEntry): .10 mW, .11 entPhysicalIndex
    pub const CPE_EXT_PORT: &str = "1.3.6.1.4.1.9.9.402.1.2.1";
}

/// Bound of every table walk.
const ROWS: usize = 20_000;

// ── Results ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    Lldp,
    Cdp,
}

/// A device seen on a local port by CDP or LLDP.
#[derive(Debug, Clone, PartialEq)]
pub struct PortNeighbour {
    pub if_index: u32,
    /// Host name, without domain or serial (`sw-core-01`).
    pub remote_name: String,
    /// Its port (`Ethernet1/49`).
    pub remote_port: String,
    pub remote_ip: Option<Ipv4Addr>,
    /// CDP platform, or the first line of the LLDP system description.
    pub platform: String,
    /// As sent: CDP device ID, or LLDP chassis ID (a MAC when it is one).
    pub chassis_id: String,
    pub protocol: Proto,
}

/// A transceiver and its digital diagnostics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Optic {
    pub if_index: u32,
    /// entPhysicalDescr (`25GBASE-SR`), else the model.
    pub kind: String,
    pub vendor: String,
    /// entPhysicalModelName (`SFP-25G-SR-S`).
    pub part: String,
    pub serial: String,
    /// Received power; for multi-lane optics (QSFP) the weakest lane.
    pub rx_dbm: Option<f32>,
    /// Transmitted power; weakest lane too.
    pub tx_dbm: Option<f32>,
    /// Every lane's received power, in lane order (one value for SFPs).
    pub rx_lanes_dbm: Vec<f32>,
    pub temp_c: Option<f32>,
    /// Laser bias current (lane 1).
    pub bias_ma: Option<f32>,
    pub voltage: Option<f32>,
    /// Low received power thresholds of the reported Rx sensor (Cisco only).
    pub rx_low_warn_dbm: Option<f32>,
    pub rx_low_alarm_dbm: Option<f32>,
}

/// A multicast group the switch forwards to some ports (IGMP snooping).
#[derive(Debug, Clone, PartialEq)]
pub struct SnoopGroup {
    pub vlan: u16,
    /// The IP group when a MIB tells it; else match `mac` with [`group_mac`].
    pub group: Option<Ipv4Addr>,
    pub mac: Mac,
    pub ports: Vec<u32>,
}

/// A fan or a power supply.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Psu {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    /// Output power when a sensor of the supply tells it.
    pub watts: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Health {
    /// 1-minute CPU load (average of the CPUs / stack members).
    pub cpu_pct: Option<f32>,
    pub cpu_5min_pct: Option<f32>,
    pub mem_used_bytes: Option<u64>,
    pub mem_total_bytes: Option<u64>,
    /// (sensor name, °C), transceivers excluded.
    pub temps: Vec<(String, f32)>,
    pub fans: Vec<Unit>,
    pub psus: Vec<Psu>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StpState {
    Disabled,
    Blocking,
    Listening,
    Learning,
    Forwarding,
    Broken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoeStatus {
    Disabled,
    Searching,
    Delivering,
    Fault,
    Test,
    /// otherFault(6): includes power denied (budget exhausted) on some switches.
    OtherFault,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PoePort {
    /// 0 when the port could not be matched to an interface.
    pub if_index: u32,
    /// POWER-ETHERNET-MIB index: group (stack member) and port.
    pub group: u32,
    pub port: u32,
    pub admin_on: bool,
    /// Delivering power.
    pub on: bool,
    pub status: PoeStatus,
    /// PoE class 0..4, while delivering.
    pub class: Option<u8>,
    pub watts: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Poe {
    pub budget_w: Option<f32>,
    pub used_w: Option<f32>,
    pub ports: Vec<PoePort>,
}

/// Everything at once, each part empty when it failed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SwitchInfo {
    pub neighbours: Vec<PortNeighbour>,
    pub optics: Vec<Optic>,
    pub mcast_groups: Vec<SnoopGroup>,
    pub mcast_router_ports: Vec<(u16, Vec<u32>)>,
    pub health: Health,
    /// (member ifIndex, aggregate ifIndex).
    pub lags: Vec<(u32, u32)>,
    pub stp: Vec<(u32, StpState)>,
    pub poe: Option<Poe>,
    /// The parts that failed, and why.
    pub errors: Vec<String>,
}

// ── Pure helpers ──

/// A sensor reading in its base unit (volts, amperes, watts, °C, dBm):
/// `raw` × 10^scale ÷ 10^precision, `scale` being the SensorDataScale enum
/// (8 milli, 9 units, 10 kilo...) of ENTITY-SENSOR-MIB and CISCO-ENTITY-SENSOR-MIB.
pub fn sensor_value(raw: i64, scale: u32, precision: i32) -> f64 {
    // The MIB lists exa(14) before peta(15).
    let exp: i32 = match scale {
        1 => -24,
        2 => -21,
        3 => -18,
        4 => -15,
        5 => -12,
        6 => -9,
        7 => -6,
        8 => -3,
        10 => 3,
        11 => 6,
        12 => 9,
        13 => 12,
        14 => 18,
        15 => 15,
        16 => 21,
        17 => 24,
        _ => 0,
    };
    let e = exp - precision.clamp(-9, 9);
    // Divide for negative powers: 10^-1 is not exact, 10 is.
    if e >= 0 {
        raw as f64 * 10f64.powi(e)
    } else {
        raw as f64 / 10f64.powi(-e)
    }
}

/// The Ethernet MAC an IPv4 multicast group is sent to (01:00:5E + low 23 bits).
pub fn group_mac(group: Ipv4Addr) -> Mac {
    let o = group.octets();
    [0x01, 0x00, 0x5e, o[1] & 0x7f, o[2], o[3]]
}

/// A CDP device ID or LLDP system name reduced to the host name:
/// `sw-core-01(FDO123)` and `sw-core-01.example.lan` give `sw-core-01`.
pub fn clean_device_id(id: &str) -> String {
    let mut s = id.trim();
    if let Some(i) = s.find('(') {
        if i > 0 && s.ends_with(')') {
            s = s[..i].trim();
        }
    }
    if s.parse::<Ipv4Addr>().is_ok() {
        return s.to_string();
    }
    match s.split_once('.') {
        Some((host, _)) if !host.is_empty() => host.to_string(),
        _ => s.to_string(),
    }
}

/// (lowercase letters, the rest) of an interface label: `Gi1/0/1` -> (`gi`, `1/0/1`).
fn split_port(s: &str) -> (String, &str) {
    let s = s.trim();
    let i = s.find(|c: char| c.is_ascii_digit()).unwrap_or(s.len());
    (s[..i].trim().to_ascii_lowercase(), &s[i..])
}

/// Whether two labels name the same interface, short or long:
/// `Eth1/49` / `Ethernet1/49`, `Te1/1/1` / `TenGigabitEthernet1/1/1`, `Po1` / `port-channel1`.
pub fn same_port(a: &str, b: &str) -> bool {
    let ((pa, na), (pb, nb)) = (split_port(a), split_port(b));
    !na.is_empty() && na == nb && (pa == pb || (!pa.is_empty() && !pb.is_empty() && (pa.starts_with(&pb) || pb.starts_with(&pa))))
}

fn first_token(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or("")
}

/// The lane a transceiver sensor is about (`... Lane 3 ...`), 1 when not said.
fn lane_of(label: &str) -> u32 {
    let l = label.to_ascii_lowercase();
    l.find("lane").and_then(|i| l[i + 4..].trim_start().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()).unwrap_or(1)
}

fn watts_to_dbm(w: f64) -> Option<f64> {
    (w > 0.0).then(|| 10.0 * (w * 1000.0).log10())
}

/// Low thresholds from (severity, relation, value) rows: (warning, alarm).
/// Severity 10 minor warns, 20 major / 30 critical alarm; relation 1/2 is "less".
/// Without severities the higher limit warns, the lower one alarms.
fn low_thresholds(rows: &[(i64, i64, f64)]) -> (Option<f32>, Option<f32>) {
    let mut lows: Vec<(i64, f64)> = rows.iter().filter(|r| r.1 == 1 || r.1 == 2).map(|r| (r.0, r.2)).collect();
    lows.sort_by(|a, b| a.1.total_cmp(&b.1));
    let minor: Vec<f64> = lows.iter().filter(|r| r.0 < 20).map(|r| r.1).collect();
    let major: Vec<f64> = lows.iter().filter(|r| r.0 >= 20).map(|r| r.1).collect();
    let (warn, alarm) = match (minor.last(), major.first()) {
        (Some(w), Some(a)) => (Some(*w), Some(*a)),
        _ if lows.len() >= 2 => (lows.last().map(|r| r.1), lows.first().map(|r| r.1)),
        (w, a) => (w.copied(), a.copied()),
    };
    (warn.map(|v| v as f32), alarm.map(|v| v as f32))
}

/// What a transceiver sensor measures.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reading {
    Rx,
    Tx,
    Temp,
    Bias,
    Volt,
}

/// From the sensor type (4 voltsDC, 5 amperes, 6 watts, 8 celsius, 14 dBm) and its name.
fn reading(kind: u32, label: &str) -> Option<Reading> {
    let l = label.to_ascii_lowercase();
    let has = |w: &str| l.split(|c: char| !c.is_ascii_alphanumeric()).any(|t| t == w);
    match kind {
        14 | 6 if l.contains("receive") || has("rx") => Some(Reading::Rx),
        14 | 6 if l.contains("transmit") || has("tx") => Some(Reading::Tx),
        8 => Some(Reading::Temp),
        5 => Some(Reading::Bias),
        4 => Some(Reading::Volt),
        _ => None,
    }
}

fn num(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) => Some(*i),
        Value::Counter(c) | Value::Counter64(c) => i64::try_from(*c).ok(),
        Value::Gauge(g) | Value::Ticks(g) => Some(i64::from(*g)),
        _ => None,
    }
}

fn text(v: &Value) -> String {
    v.as_text().unwrap_or_default()
}

fn avg(v: impl Iterator<Item = i64>) -> Option<f32> {
    let v: Vec<i64> = v.collect();
    (!v.is_empty()).then(|| v.iter().sum::<i64>() as f32 / v.len() as f32)
}

// ── Reading tables ──

/// "The agent answered with error N": the agent refuses that table, it is not down.
fn refused(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::Other && e.to_string().contains("answered with error")
}

/// Every cell under `base` as (row index, value); empty when the agent has no such table.
async fn column(s: &Session, base: &str, limit: usize) -> io::Result<Vec<(Vec<u32>, Value)>> {
    let b = Oid::parse(base);
    match s.walk(&b, limit).await {
        Ok(rows) => Ok(rows
            .into_iter()
            .filter(|(o, v)| o.0.len() > b.0.len() && !v.is_absent() && *v != Value::Null)
            .map(|(o, v)| (o.suffix(&b).to_vec(), v))
            .collect()),
        Err(e) if refused(&e) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// A column indexed by one number.
async fn column1(s: &Session, base: &str) -> io::Result<BTreeMap<u32, Value>> {
    Ok(column(s, base, ROWS).await?.into_iter().map(|(i, v)| (i[0], v)).collect())
}

/// A column indexed by two numbers (group.port, ifIndex.device...).
async fn column2(s: &Session, base: &str) -> io::Result<BTreeMap<(u32, u32), Value>> {
    Ok(column(s, base, ROWS).await?.into_iter().filter(|(i, _)| i.len() >= 2).map(|(i, v)| ((i[0], i[1]), v)).collect())
}

fn col(entry: &str, n: u32) -> String {
    format!("{entry}.{n}")
}

/// Interface names and types.
#[derive(Default)]
struct Ifs {
    name: BTreeMap<u32, String>,
    descr: BTreeMap<u32, String>,
    kind: HashMap<u32, u32>,
}

impl Ifs {
    async fn load(s: &Session) -> io::Result<Ifs> {
        let mut o = Ifs::default();
        for (i, v) in column1(s, oids::IF_NAME).await? {
            o.name.insert(i, text(&v));
        }
        for (i, v) in column1(s, oids::IF_DESCR).await? {
            o.descr.insert(i, text(&v));
        }
        for (i, v) in column1(s, oids::IF_TYPE).await? {
            o.kind.insert(i, num(&v).unwrap_or(0) as u32);
        }
        for (i, d) in &o.descr {
            let n = o.name.entry(*i).or_default();
            if n.is_empty() {
                n.clone_from(d);
            }
        }
        Ok(o)
    }

    /// The interface a label names (ifName or ifDescr, exactly or short/long form).
    fn find(&self, label: &str) -> Option<u32> {
        let l = label.trim();
        if l.is_empty() {
            return None;
        }
        let all = || self.name.iter().chain(self.descr.iter());
        all().find(|(_, n)| n.eq_ignore_ascii_case(l)).or_else(|| all().find(|(_, n)| same_port(n, l))).map(|(i, _)| *i)
    }

    /// Front-panel Ethernet ports by ifIndex (management ports left out).
    fn ethernet(&self) -> Vec<u32> {
        self.name
            .iter()
            .filter(|(i, n)| {
                let (prefix, rest) = split_port(n);
                matches!(self.kind.get(i), Some(6 | 62 | 69 | 117))
                    && !prefix.starts_with("mgmt")
                    && !prefix.starts_with("management")
                    && rest != "0/0"
                    && rest != "0"
            })
            .map(|(i, _)| *i)
            .collect()
    }
}

#[derive(Default, Clone, Debug)]
struct Ent {
    descr: String,
    parent: u32,
    class: u32,
    name: String,
    serial: String,
    mfg: String,
    model: String,
}

/// The ENTITY-MIB physical table (needed columns only) and its ifIndex aliases.
#[derive(Default)]
struct Entities {
    by: HashMap<u32, Ent>,
    alias: HashMap<u32, u32>,
}

impl Entities {
    async fn load(s: &Session) -> io::Result<Entities> {
        let mut by: HashMap<u32, Ent> = HashMap::new();
        for n in [2u32, 4, 5, 7, 11, 12, 13] {
            for (i, v) in column1(s, &col(oids::ENT_PHYSICAL, n)).await? {
                let e = by.entry(i).or_default();
                match n {
                    2 => e.descr = text(&v),
                    4 => e.parent = num(&v).unwrap_or(0) as u32,
                    5 => e.class = num(&v).unwrap_or(0) as u32,
                    7 => e.name = text(&v),
                    11 => e.serial = text(&v),
                    12 => e.mfg = text(&v),
                    _ => e.model = text(&v),
                }
            }
        }
        let ifx = Oid::parse(oids::IF_INDEX);
        let mut alias = HashMap::new();
        for (i, v) in column(s, oids::ENT_ALIAS, ROWS).await? {
            if let Value::Oid(o) = v {
                if o.starts_with(&ifx) && o.0.len() == ifx.0.len() + 1 {
                    alias.entry(i[0]).or_insert(o.0[ifx.0.len()]);
                }
            }
        }
        Ok(Entities { by, alias })
    }

    fn label(&self, i: u32) -> String {
        match self.by.get(&i) {
            Some(e) if !e.name.is_empty() => e.name.clone(),
            Some(e) if !e.descr.is_empty() => e.descr.clone(),
            _ => format!("entity {i}"),
        }
    }

    /// Name and description, for classifying sensors.
    fn words(&self, i: u32) -> String {
        self.by.get(&i).map(|e| format!("{} {}", e.name, e.descr)).unwrap_or_default()
    }

    fn parent(&self, i: u32) -> u32 {
        self.by.get(&i).map_or(0, |e| e.parent)
    }

    fn class(&self, i: u32) -> Option<u32> {
        self.by.get(&i).map(|e| e.class)
    }

    /// Whether `i` is contained in `anc` (at most `depth` levels up).
    fn under(&self, i: u32, anc: u32, depth: usize) -> bool {
        let mut p = self.parent(i);
        for _ in 0..depth {
            if p == 0 {
                return false;
            }
            if p == anc {
                return true;
            }
            p = self.parent(p);
        }
        false
    }

    /// The interface of a transceiver entity: its own alias; the alias of its
    /// only port child (IOS: transceiver module > port); a container's alias
    /// (NX-OS: port > transceiver); else the interface named by its or its
    /// optical sensors' entPhysicalName (`Ethernet1/49 Transceiver`, `Te1/1/1 Receive Power Sensor`).
    /// Other children are not looked at: a line card contains ports named like interfaces.
    fn if_index(&self, t: u32, ifs: &Ifs, optical: &HashSet<u32>) -> Option<u32> {
        if let Some(i) = self.alias.get(&t) {
            return Some(*i);
        }
        let ports: Vec<u32> = self.by.iter().filter(|(_, e)| e.parent == t && e.class == 10).filter_map(|(i, _)| self.alias.get(i).copied()).collect();
        if ports.len() == 1 {
            return Some(ports[0]);
        }
        let mut p = self.parent(t);
        for _ in 0..2 {
            if p == 0 {
                break;
            }
            if let Some(i) = self.alias.get(&p) {
                return Some(*i);
            }
            p = self.parent(p);
        }
        let own = self.by.get(&t).map(|e| e.name.as_str()).unwrap_or_default();
        ifs.find(first_token(own)).or_else(|| {
            let mut kids: Vec<(&u32, &Ent)> = self.by.iter().filter(|(i, e)| e.parent == t && optical.contains(i)).collect();
            kids.sort_by_key(|(i, _)| **i);
            kids.into_iter().find_map(|(_, e)| ifs.find(first_token(&e.name)))
        })
    }
}

#[derive(Clone, Debug)]
struct Sensor {
    ent: u32,
    kind: u32,
    raw: i64,
    scale: u32,
    precision: i32,
    ok: bool,
}

impl Sensor {
    fn value(&self) -> f64 {
        sensor_value(self.raw, self.scale, self.precision)
    }
}

/// The sensors (Cisco table, else the standard one); true when they are Cisco's.
async fn load_sensors(s: &Session) -> io::Result<(Vec<Sensor>, bool)> {
    for (base, cisco) in [(oids::CISCO_SENSOR, true), (oids::ENTITY_SENSOR, false)] {
        let kinds = column1(s, &col(base, 1)).await?;
        if kinds.is_empty() {
            continue;
        }
        let values = column1(s, &col(base, 4)).await?;
        let scales = column1(s, &col(base, 2)).await?;
        let precs = column1(s, &col(base, 3)).await?;
        let status = column1(s, &col(base, 5)).await?;
        let out = kinds
            .iter()
            .filter_map(|(i, k)| {
                Some(Sensor {
                    ent: *i,
                    kind: num(k)? as u32,
                    raw: num(values.get(i)?)?,
                    scale: scales.get(i).and_then(num).unwrap_or(9) as u32,
                    precision: precs.get(i).and_then(num).unwrap_or(0) as i32,
                    // 1 ok, 2 unavailable, 3 nonoperational
                    ok: status.get(i).and_then(num).is_none_or(|v| v == 1),
                })
            })
            .collect();
        return Ok((out, cisco));
    }
    Ok((Vec::new(), false))
}

/// What the parts share, read once.
struct Ctx<'a> {
    s: &'a Session,
    ifs: OnceCell<Ifs>,
    ents: OnceCell<Entities>,
    sensors: OnceCell<(Vec<Sensor>, bool)>,
    bridge: OnceCell<HashMap<u32, u32>>,
}

impl<'a> Ctx<'a> {
    fn new(s: &'a Session) -> Self {
        Ctx { s, ifs: OnceCell::new(), ents: OnceCell::new(), sensors: OnceCell::new(), bridge: OnceCell::new() }
    }
    async fn ifs(&self) -> io::Result<&Ifs> {
        self.ifs.get_or_try_init(|| Ifs::load(self.s)).await
    }
    async fn ents(&self) -> io::Result<&Entities> {
        self.ents.get_or_try_init(|| Entities::load(self.s)).await
    }
    async fn sensors(&self) -> io::Result<&(Vec<Sensor>, bool)> {
        self.sensors.get_or_try_init(|| load_sensors(self.s)).await
    }
    /// Bridge port -> ifIndex (BRIDGE-MIB dot1dBasePortIfIndex).
    async fn bridge(&self) -> io::Result<&HashMap<u32, u32>> {
        self.bridge
            .get_or_try_init(|| async {
                Ok(column1(self.s, oids::BRIDGE_PORT_IF).await?.into_iter().filter_map(|(p, v)| Some((p, num(&v)? as u32))).collect())
            })
            .await
    }
    async fn ports(&self, list: &[u8]) -> io::Result<Vec<u32>> {
        let b = self.bridge().await?;
        Ok(port_list(list).into_iter().map(|p| b.get(&p).copied().unwrap_or(p)).collect())
    }
}

/// Parents of optical (Rx/Tx power) sensors: the transceivers with diagnostics.
fn optical_parents(ents: &Entities, sensors: &[Sensor]) -> HashSet<u32> {
    sensors
        .iter()
        .filter(|s| matches!(reading(s.kind, &ents.words(s.ent)), Some(Reading::Rx | Reading::Tx)))
        .map(|s| ents.parent(s.ent))
        .filter(|p| *p != 0)
        .collect()
}

// ── Neighbours ──

/// CDP and LLDP neighbours by local port. When both see the same device on a
/// port, the CDP entry is kept (completed with the LLDP address if it had none).
pub async fn neighbours(s: &Session) -> io::Result<Vec<PortNeighbour>> {
    neighbours_in(&Ctx::new(s)).await
}

async fn neighbours_in(c: &Ctx<'_>) -> io::Result<Vec<PortNeighbour>> {
    let s = c.s;
    let mut out: Vec<PortNeighbour> = Vec::new();
    // CDP, indexed by ifIndex.deviceIndex.
    let ids = column2(s, &col(oids::CDP_CACHE, 6)).await?;
    if !ids.is_empty() {
        let ports = column2(s, &col(oids::CDP_CACHE, 7)).await?;
        let addrs = column2(s, &col(oids::CDP_CACHE, 4)).await?;
        let plats = column2(s, &col(oids::CDP_CACHE, 8)).await?;
        for (k, id) in &ids {
            let raw = text(id);
            let remote_ip = match addrs.get(k) {
                Some(Value::Bytes(b)) if b.len() == 4 => Some(Ipv4Addr::new(b[0], b[1], b[2], b[3])),
                Some(Value::Ip(ip)) => Some(*ip),
                _ => None,
            };
            out.push(PortNeighbour {
                if_index: k.0,
                remote_name: clean_device_id(&raw),
                remote_port: ports.get(k).map(text).unwrap_or_default(),
                remote_ip,
                platform: plats.get(k).map(text).unwrap_or_default(),
                chassis_id: raw,
                protocol: Proto::Cdp,
            });
        }
    }
    for n in lldp_neighbours(c).await? {
        let same = out.iter_mut().find(|o| {
            o.if_index == n.if_index && (o.remote_name.eq_ignore_ascii_case(&n.remote_name) || (o.remote_ip.is_some() && o.remote_ip == n.remote_ip))
        });
        match same {
            Some(o) => {
                if o.remote_ip.is_none() {
                    o.remote_ip = n.remote_ip;
                }
            }
            None => out.push(n),
        }
    }
    out.sort_by(|a, b| (a.if_index, &a.remote_name).cmp(&(b.if_index, &b.remote_name)));
    Ok(out)
}

async fn lldp_neighbours(c: &Ctx<'_>) -> io::Result<Vec<PortNeighbour>> {
    let s = c.s;
    // lldpRemTable is indexed by timeMark.localPortNum.remIndex.
    let mut rem: BTreeMap<(u32, u32), BTreeMap<u32, Value>> = BTreeMap::new();
    for n in [7u32, 9, 6, 8, 5, 4, 10] {
        let rows = column(s, &col(oids::LLDP_REM, n), ROWS).await?;
        // The port ID is mandatory: none, no neighbours (or no LLDP-MIB).
        if n == 7 && rows.is_empty() {
            return Ok(Vec::new());
        }
        for (i, v) in rows {
            if i.len() >= 3 {
                rem.entry((i[1], i[2])).or_default().insert(n, v);
            }
        }
    }
    if rem.is_empty() {
        return Ok(Vec::new());
    }
    // Management addresses: index ...remIndex.addrSubtype(1 = IPv4)[.length].a.b.c.d
    let mut ips: HashMap<(u32, u32), Ipv4Addr> = HashMap::new();
    for (i, _) in column(s, oids::LLDP_REM_MAN_ADDR, ROWS).await? {
        let (Some(key), Some(1)) = (i.get(1..3), i.get(3).copied()) else { continue };
        let a = match &i[4..] {
            [4, a @ ..] if a.len() == 4 => a,
            a if a.len() == 4 => a,
            _ => continue,
        };
        if let Ok(o) = a.iter().map(|x| u8::try_from(*x)).collect::<Result<Vec<u8>, _>>() {
            ips.entry((key[0], key[1])).or_insert(Ipv4Addr::new(o[0], o[1], o[2], o[3]));
        }
    }
    // lldpLocPortNum -> ifIndex: by port ID or description against ifName / ifDescr,
    // else the number itself when it is an ifIndex (usual on Cisco).
    let ifs = c.ifs().await?;
    let loc_id = column1(s, &col(oids::LLDP_LOC_PORT, 3)).await?;
    let loc_desc = column1(s, &col(oids::LLDP_LOC_PORT, 4)).await?;
    let local = |lp: u32| {
        loc_id
            .get(&lp)
            .and_then(|v| ifs.find(&text(v)))
            .or_else(|| loc_desc.get(&lp).and_then(|v| ifs.find(&text(v))))
            .or_else(|| ifs.name.contains_key(&lp).then_some(lp))
    };
    let mut out = Vec::new();
    for ((lp, ri), cells) in rem {
        let Some(if_index) = local(lp) else { continue };
        let t = |n: u32| cells.get(&n).map(text).unwrap_or_default();
        let sub = |n: u32| cells.get(&n).and_then(num).unwrap_or(0);
        // Chassis ID subtype 4 = MAC, 5 = network address (family byte + IPv4).
        let chassis_id = match (sub(4), cells.get(&5)) {
            (4, Some(v)) => v.as_mac().map(|m| mac::format(&m)).unwrap_or_default(),
            (5, Some(Value::Bytes(b))) if b.len() == 5 && b[0] == 1 => Ipv4Addr::new(b[1], b[2], b[3], b[4]).to_string(),
            _ => t(5),
        };
        // Port ID subtype 5 = interface name (best), 3 = MAC (description is better), 7 = local.
        let (id, desc) = (t(7), t(8));
        let remote_port = match sub(6) {
            5 if !id.is_empty() => id,
            3 | 7 if !desc.is_empty() => desc,
            3 => cells.get(&7).and_then(Value::as_mac).map(|m| mac::format(&m)).unwrap_or_default(),
            _ if id.is_empty() => desc,
            _ => id,
        };
        let name = t(9);
        out.push(PortNeighbour {
            if_index,
            remote_name: if name.is_empty() { chassis_id.clone() } else { clean_device_id(&name) },
            remote_port,
            remote_ip: ips.get(&(lp, ri)).copied(),
            platform: t(10).lines().next().unwrap_or_default().trim().to_string(),
            chassis_id,
            protocol: Proto::Lldp,
        });
    }
    Ok(out)
}

// ── Optics ──

/// Transceivers and their diagnostics (DOM), by interface.
///
/// A transceiver is the entity containing Rx/Tx power sensors, or one whose
/// name or description says transceiver / SFP / QSFP... and has a serial
/// number (optics without diagnostics, DACs). Its sensors are its direct
/// children, classified by type (dBm 14 or watts 6 with "Receive"/"Transmit"
/// in the name, celsius 8, amperes 5, voltsDC 4). Multi-lane optics: Rx and Tx
/// report the weakest lane (a dead lane shows), bias, temperature and voltage
/// lane 1. Breakout ports (one QSFP, four ifIndexes) are not split per lane.
pub async fn optics(s: &Session) -> io::Result<Vec<Optic>> {
    optics_in(&Ctx::new(s)).await
}

fn looks_like_transceiver(e: &Ent) -> bool {
    let w = format!("{} {} {}", e.name, e.descr, e.model).to_ascii_lowercase();
    e.class != 8 && !e.serial.is_empty() && (w.contains("transceiver") || ["sfp", "qsfp", "xfp", "gbic", "cfp", "osfp"].iter().any(|k| w.contains(k)))
}

async fn optics_in(c: &Ctx<'_>) -> io::Result<Vec<Optic>> {
    let (sensors, cisco) = c.sensors().await?;
    let ents = c.ents().await?;
    let optical: HashSet<u32> =
        sensors.iter().filter(|s| matches!(reading(s.kind, &ents.words(s.ent)), Some(Reading::Rx | Reading::Tx))).map(|s| s.ent).collect();
    let mut trx: BTreeSet<u32> = optical.iter().map(|i| ents.parent(*i)).filter(|p| *p != 0).collect();
    trx.extend(ents.by.iter().filter(|(_, e)| looks_like_transceiver(e)).map(|(i, _)| *i));
    if trx.is_empty() {
        return Ok(Vec::new());
    }
    let ifs = c.ifs().await?;
    let mut by_if: BTreeMap<u32, Optic> = BTreeMap::new();
    for t in trx {
        let Some(if_index) = ents.if_index(t, ifs, &optical) else { continue };
        let e = ents.by.get(&t).cloned().unwrap_or_default();
        let mut o = Optic {
            if_index,
            kind: if e.descr.is_empty() { e.model.clone() } else { e.descr.clone() },
            vendor: e.mfg,
            part: e.model,
            serial: e.serial,
            ..Optic::default()
        };
        let mut rx: Vec<(u32, f64, &Sensor)> = Vec::new();
        let mut tx: Vec<(u32, f64)> = Vec::new();
        let mut other: BTreeMap<(u32, u8), f64> = BTreeMap::new(); // (lane, what) -> value
        for sn in sensors.iter().filter(|x| x.ok && ents.parent(x.ent) == t) {
            let words = ents.words(sn.ent);
            let Some(r) = reading(sn.kind, &words) else { continue };
            let lane = lane_of(&words);
            let v = sn.value();
            let dbm = || if sn.kind == 6 { watts_to_dbm(v) } else { Some(v) };
            match r {
                Reading::Rx => rx.extend(dbm().map(|d| (lane, d, sn))),
                Reading::Tx => tx.extend(dbm().map(|d| (lane, d))),
                Reading::Temp => _ = other.entry((lane, 0)).or_insert(v),
                Reading::Bias => _ = other.entry((lane, 1)).or_insert(v * 1000.0),
                Reading::Volt => _ = other.entry((lane, 2)).or_insert(v),
            }
        }
        rx.sort_by_key(|r| r.0);
        o.rx_lanes_dbm = rx.iter().map(|r| r.1 as f32).collect();
        let weakest = rx.iter().min_by(|a, b| a.1.total_cmp(&b.1));
        o.rx_dbm = weakest.map(|r| r.1 as f32);
        o.tx_dbm = tx.iter().map(|r| r.1).min_by(f64::total_cmp).map(|v| v as f32);
        let first = |what: u8| other.iter().find(|((_, w), _)| *w == what).map(|(_, v)| *v as f32);
        (o.temp_c, o.bias_ma, o.voltage) = (first(0), first(1), first(2));
        if let (true, Some((_, _, sn))) = (*cisco, weakest) {
            (o.rx_low_warn_dbm, o.rx_low_alarm_dbm) = rx_thresholds(c.s, sn).await?;
        }
        // Two entities for one port (port and transceiver): keep the one with readings.
        match by_if.get(&if_index) {
            Some(old) if old.rx_dbm.is_some() || (o.rx_dbm.is_none() && !old.serial.is_empty()) => {}
            _ => {
                by_if.insert(if_index, o);
            }
        }
    }
    Ok(by_if.into_values().collect())
}

/// Low Rx thresholds of a Cisco sensor (entSensorThresholdTable, same scale as the value).
async fn rx_thresholds(s: &Session, sn: &Sensor) -> io::Result<(Option<f32>, Option<f32>)> {
    let mut rows: BTreeMap<u32, [Option<i64>; 3]> = BTreeMap::new();
    for (k, n) in [2u32, 3, 4].into_iter().enumerate() {
        for (i, v) in column(s, &format!("{}.{n}.{}", oids::CISCO_SENSOR_THRESHOLD, sn.ent), 16).await? {
            rows.entry(i[0]).or_default()[k] = num(&v);
        }
    }
    let list: Vec<(i64, i64, f64)> = rows
        .values()
        .filter_map(|r| {
            let v = sensor_value(r[2]?, sn.scale, sn.precision);
            Some((r[0].unwrap_or(1), r[1]?, if sn.kind == 6 { watts_to_dbm(v)? } else { v }))
        })
        .collect();
    Ok(low_thresholds(&list))
}

// ── IGMP snooping ──

/// Multicast groups and the ports they go to (Q-BRIDGE dot1qTpGroupTable).
/// The MIB gives MAC groups only: `group` stays `None`, see [`group_mac`].
pub async fn mcast_groups(s: &Session) -> io::Result<Vec<SnoopGroup>> {
    mcast_in(&Ctx::new(s)).await
}

async fn mcast_in(c: &Ctx<'_>) -> io::Result<Vec<SnoopGroup>> {
    let mut out = Vec::new();
    for (i, v) in column(c.s, oids::Q_GROUP_EGRESS, ROWS).await? {
        let (Some(m), Value::Bytes(list)) = (i.get(1..7), &v) else { continue };
        let Ok(mac) = m.iter().map(|x| u8::try_from(*x)).collect::<Result<Vec<u8>, _>>() else { continue };
        let mac: Mac = mac.try_into().unwrap_or_default();
        out.push(SnoopGroup { vlan: i[0] as u16, group: None, mac, ports: c.ports(list).await? });
    }
    Ok(out)
}

/// Ports receiving all multicast of a VLAN (dot1qForwardAllPorts), which some
/// switches fill with the multicast router ports. Empty lists left out.
pub async fn mcast_router_ports(s: &Session) -> io::Result<Vec<(u16, Vec<u32>)>> {
    router_ports_in(&Ctx::new(s)).await
}

async fn router_ports_in(c: &Ctx<'_>) -> io::Result<Vec<(u16, Vec<u32>)>> {
    let mut out = Vec::new();
    for (vlan, v) in column1(c.s, oids::Q_FORWARD_ALL).await? {
        if let Value::Bytes(list) = v {
            let ports = c.ports(&list).await?;
            if !ports.is_empty() {
                out.push((vlan as u16, ports));
            }
        }
    }
    Ok(out)
}

// ── Health ──

/// CPU, memory, temperatures, fans and power supplies.
pub async fn health(s: &Session) -> io::Result<Health> {
    health_in(&Ctx::new(s)).await
}

fn fan_detail(v: i64) -> &'static str {
    match v {
        2 => "up",
        3 => "down",
        4 => "warning",
        _ => "unknown",
    }
}

fn power_detail(v: i64) -> &'static str {
    match v {
        1 => "off (environment)",
        2 => "on",
        3 => "off (admin)",
        4 => "off (denied)",
        5 => "off (no input power)",
        6 => "off (temperature)",
        7 => "off (fan)",
        8 => "failed",
        9 => "on, fan failed",
        10 => "off (cooling)",
        11 => "off (connector rating)",
        12 => "on, inline power failed",
        _ => "unknown",
    }
}

fn envmon_detail(v: i64) -> &'static str {
    match v {
        1 => "normal",
        2 => "warning",
        3 => "critical",
        4 => "shutdown",
        5 => "not present",
        6 => "not functioning",
        _ => "unknown",
    }
}

async fn health_in(c: &Ctx<'_>) -> io::Result<Health> {
    let s = c.s;
    let mut h = Health::default();
    // CPU and memory: CISCO-PROCESS-MIB, else HOST-RESOURCES-MIB.
    let mut one = column1(s, &col(oids::CPM, 7)).await?;
    if one.is_empty() {
        one = column1(s, &col(oids::CPM, 4)).await?;
    }
    h.cpu_pct = avg(one.values().filter_map(num));
    if !one.is_empty() {
        let mut five = column1(s, &col(oids::CPM, 8)).await?;
        if five.is_empty() {
            five = column1(s, &col(oids::CPM, 5)).await?;
        }
        h.cpu_5min_pct = avg(five.values().filter_map(num));
    }
    let mut used = column1(s, &col(oids::CPM, 17)).await?;
    let mut free = BTreeMap::new();
    if used.is_empty() {
        used = column1(s, &col(oids::CPM, 12)).await?;
        if !used.is_empty() {
            free = column1(s, &col(oids::CPM, 13)).await?;
        }
    } else {
        free = column1(s, &col(oids::CPM, 19)).await?;
    }
    if !used.is_empty() {
        let kb = |m: &BTreeMap<u32, Value>| m.values().filter_map(num).map(|v| v.max(0) as u64).sum::<u64>();
        let (u, f) = (kb(&used), kb(&free));
        h.mem_used_bytes = Some(u * 1024);
        h.mem_total_bytes = (f > 0).then_some((u + f) * 1024);
    }
    if h.cpu_pct.is_none() {
        h.cpu_pct = avg(column1(s, oids::HR_PROCESSOR_LOAD).await?.values().filter_map(num));
    }
    if h.mem_used_bytes.is_none() {
        let ram = Oid::parse(oids::HR_STORAGE_RAM);
        let types = column1(s, &col(oids::HR_STORAGE, 2)).await?;
        if let Some(i) = types.iter().find(|(_, v)| **v == Value::Oid(ram.clone())).map(|(i, _)| *i) {
            let cell = |n: u32| async move { column1(s, &col(oids::HR_STORAGE, n)).await.map(|m| m.get(&i).and_then(num).unwrap_or(0).max(0) as u64) };
            let (units, size, used) = (cell(4).await?, cell(5).await?, cell(6).await?);
            if size > 0 {
                h.mem_used_bytes = Some(used * units);
                h.mem_total_bytes = Some(size * units);
            }
        }
    }

    // Temperatures: entity sensors in °C, transceivers left out.
    let (sensors, _) = c.sensors().await?;
    let ents = c.ents().await?;
    let optical = optical_parents(ents, sensors);
    for sn in sensors.iter().filter(|x| x.kind == 8 && x.ok && !optical.contains(&ents.parent(x.ent))) {
        h.temps.push((ents.label(sn.ent), sn.value() as f32));
    }

    // Fans and power supplies: CISCO-ENTITY-FRU-CONTROL-MIB.
    for (i, v) in column1(s, oids::CEFC_FAN_STATUS).await? {
        let v = num(&v).unwrap_or(0);
        h.fans.push(Unit { name: ents.label(i), ok: v == 2, detail: fan_detail(v).into() });
    }
    // The power table lists every FRU with power control (modules too): supplies only.
    let watts = |p: u32| {
        let mine: Vec<&Sensor> = sensors.iter().filter(|x| x.kind == 6 && x.ok && ents.under(x.ent, p, 3)).collect();
        let out = mine.iter().find(|x| ents.words(x.ent).to_ascii_lowercase().contains("out")).or(mine.first());
        out.map(|x| x.value() as f32)
    };
    for (i, v) in column1(s, oids::CEFC_POWER_STATUS).await? {
        if ents.class(i).is_some_and(|k| k != 6) {
            continue;
        }
        let v = num(&v).unwrap_or(0);
        h.psus.push(Psu { name: ents.label(i), ok: v == 2, detail: power_detail(v).into(), watts: watts(i) });
    }
    // Else ENTITY-STATE-MIB on fan (7) and power supply (6) entities.
    if h.fans.is_empty() || h.psus.is_empty() {
        let (want_fans, want_psus) = (h.fans.is_empty(), h.psus.is_empty());
        for (i, v) in column1(s, oids::ENT_STATE_OPER).await? {
            let v = num(&v).unwrap_or(0);
            let detail = match v {
                2 => "disabled",
                3 => "enabled",
                4 => "testing",
                _ => "unknown",
            };
            match ents.class(i) {
                Some(7) if want_fans => h.fans.push(Unit { name: ents.label(i), ok: v == 3, detail: detail.into() }),
                Some(6) if want_psus => h.psus.push(Psu { name: ents.label(i), ok: v == 3, detail: detail.into(), watts: watts(i) }),
                _ => {}
            }
        }
    }
    // Else CISCO-ENVMON-MIB (older Cisco software).
    let envmon = |base: &'static str, state_col: u32| async move {
        let names = column1(s, &col(base, 2)).await?;
        if names.is_empty() {
            return Ok::<_, io::Error>(Vec::new());
        }
        let values = column1(s, &col(base, 3)).await?;
        let states = if state_col == 3 { values.clone() } else { column1(s, &col(base, state_col)).await? };
        Ok(names
            .iter()
            .map(|(i, n)| (text(n), values.get(i).and_then(num), states.get(i).and_then(num).unwrap_or(0)))
            .filter(|r| r.2 != 5) // not present
            .collect::<Vec<_>>())
    };
    if h.fans.is_empty() {
        for (name, _, st) in envmon(oids::ENVMON_FAN, 3).await? {
            h.fans.push(Unit { name, ok: st == 1, detail: envmon_detail(st).into() });
        }
    }
    if h.psus.is_empty() {
        for (name, _, st) in envmon(oids::ENVMON_SUPPLY, 3).await? {
            h.psus.push(Psu { name, ok: st == 1, detail: envmon_detail(st).into(), watts: None });
        }
    }
    if h.temps.is_empty() {
        for (name, v, _) in envmon(oids::ENVMON_TEMP, 6).await? {
            if let Some(v) = v {
                h.temps.push((name, v as f32));
            }
        }
    }
    Ok(h)
}

// ── Aggregates and spanning tree ──

/// Link aggregation members: (member ifIndex, aggregate ifIndex). LACP members
/// from IEEE8023-LACP-MIB, plus static ones from IF-MIB ifStackTable (higher
/// layer of type ieee8023adLag(161), or named port-channel / PoN).
pub async fn lags(s: &Session) -> io::Result<Vec<(u32, u32)>> {
    lags_in(&Ctx::new(s)).await
}

async fn lags_in(c: &Ctx<'_>) -> io::Result<Vec<(u32, u32)>> {
    let mut out: BTreeSet<(u32, u32)> = BTreeSet::new();
    for (m, v) in column1(c.s, oids::LACP_ATTACHED_AGG).await? {
        // 0 = not attached; some agents give the port itself when it runs alone.
        match num(&v) {
            Some(a) if a > 0 && a as u32 != m => _ = out.insert((m, a as u32)),
            _ => {}
        }
    }
    let stack: Vec<(u32, u32)> = column(c.s, oids::IF_STACK_STATUS, ROWS)
        .await?
        .into_iter()
        .filter(|(i, _)| i.len() == 2 && i[0] != 0 && i[1] != 0)
        .map(|(i, _)| (i[0], i[1]))
        .collect();
    if !stack.is_empty() {
        let ifs = c.ifs().await?;
        let is_lag = |i: u32| {
            let n = ifs.name.get(&i).map(|n| n.to_ascii_lowercase()).unwrap_or_default();
            ifs.kind.get(&i) == Some(&161) || n.starts_with("port-channel") || (n.starts_with("po") && n[2..].starts_with(|ch: char| ch.is_ascii_digit()))
        };
        let known: HashSet<u32> = out.iter().map(|m| m.0).collect();
        for (hi, lo) in stack {
            if is_lag(hi) && !known.contains(&lo) {
                out.insert((lo, hi));
            }
        }
    }
    Ok(out.into_iter().collect())
}

/// Spanning tree state of each port (BRIDGE-MIB dot1dStpPortState).
/// On Cisco PVST+/Rapid-PVST+ this is the state in the default context
/// (VLAN 1); the per-VLAN states need the community@vlan context and are out of scope.
pub async fn stp(s: &Session) -> io::Result<Vec<(u32, StpState)>> {
    stp_in(&Ctx::new(s)).await
}

async fn stp_in(c: &Ctx<'_>) -> io::Result<Vec<(u32, StpState)>> {
    let states = column1(c.s, oids::STP_PORT_STATE).await?;
    if states.is_empty() {
        return Ok(Vec::new());
    }
    let b = c.bridge().await?;
    let mut out: Vec<(u32, StpState)> = states
        .into_iter()
        .filter_map(|(p, v)| {
            let st = match num(&v)? {
                1 => StpState::Disabled,
                2 => StpState::Blocking,
                3 => StpState::Listening,
                4 => StpState::Learning,
                5 => StpState::Forwarding,
                6 => StpState::Broken,
                _ => return None,
            };
            Some((b.get(&p).copied().unwrap_or(p), st))
        })
        .collect();
    out.sort_by_key(|r| r.0);
    Ok(out)
}

// ── PoE ──

/// Power over Ethernet budget and ports; `None` when the switch has no PoE (Nexus).
///
/// Ports (group.port) are matched to interfaces by, in order: Cisco
/// cpeExtPsePortEntPhyIndex -> entAliasMapping; a Cisco name `<type>G/0/P`
/// (Catalyst stack member G, port P); else the n-th PoE port (by group, port)
/// is the n-th front-panel Ethernet interface by ifIndex.
pub async fn poe(s: &Session) -> io::Result<Option<Poe>> {
    poe_in(&Ctx::new(s)).await
}

async fn poe_in(c: &Ctx<'_>) -> io::Result<Option<Poe>> {
    let s = c.s;
    let det = column2(s, &col(oids::PETH_PORT, 6)).await?;
    let budget = column1(s, &col(oids::PETH_MAIN, 2)).await?;
    if det.is_empty() && budget.is_empty() {
        return Ok(None);
    }
    let used = column1(s, &col(oids::PETH_MAIN, 4)).await?;
    let sum = |m: &BTreeMap<u32, Value>| (!m.is_empty()).then(|| m.values().filter_map(num).sum::<i64>() as f32);
    let mut out = Poe { budget_w: sum(&budget), used_w: sum(&used), ports: Vec::new() };
    if det.is_empty() {
        return Ok(Some(out));
    }
    let admin = column2(s, &col(oids::PETH_PORT, 3)).await?;
    let class = column2(s, &col(oids::PETH_PORT, 10)).await?;
    let mw = column2(s, &col(oids::CPE_EXT_PORT, 10)).await?;
    let phy = column2(s, &col(oids::CPE_EXT_PORT, 11)).await?;
    let ents = if phy.is_empty() { None } else { Some(c.ents().await?) };
    let ifs = c.ifs().await?;
    let eth = ifs.ethernet();
    let cisco_name = |g: u32, p: u32| {
        let want = format!("{g}/0/{p}");
        ifs.name.iter().find(|(_, n)| {
            let (prefix, rest) = split_port(n);
            !prefix.is_empty() && rest == want
        })
    };
    let n_ports = det.len();
    for (n, (&(g, p), v)) in det.iter().enumerate() {
        let by_ent = phy.get(&(g, p)).and_then(num).and_then(|e| ents.and_then(|x| x.alias.get(&(e as u32)).copied()));
        let if_index = by_ent.or_else(|| cisco_name(g, p).map(|(i, _)| *i)).or_else(|| (n_ports <= eth.len()).then(|| eth[n])).unwrap_or(0);
        let status = match num(v).unwrap_or(0) {
            1 => PoeStatus::Disabled,
            2 => PoeStatus::Searching,
            3 => PoeStatus::Delivering,
            4 => PoeStatus::Fault,
            5 => PoeStatus::Test,
            6 => PoeStatus::OtherFault,
            _ => PoeStatus::Unknown,
        };
        let on = status == PoeStatus::Delivering;
        out.ports.push(PoePort {
            if_index,
            group: g,
            port: p,
            admin_on: admin.get(&(g, p)).and_then(num).is_none_or(|a| a == 1),
            on,
            status,
            // class0(1) .. class4(5)
            class: class.get(&(g, p)).and_then(num).filter(|c| on && (1..=5).contains(c)).map(|c| (c - 1) as u8),
            watts: mw.get(&(g, p)).and_then(num).map(|m| m as f32 / 1000.0),
        });
    }
    Ok(Some(out))
}

// ── All ──

/// Every part, one after the other (gentle on the switch CPU), the shared
/// tables (interfaces, entities, sensors, bridge ports) read once. A part that
/// fails is left empty and noted in `errors`; after a timeout the remaining
/// parts are skipped (the switch is unreachable or overloaded).
pub async fn all(s: &Session) -> SwitchInfo {
    let c = Ctx::new(s);
    let mut out = SwitchInfo::default();
    let mut dead = false;
    macro_rules! part {
        ($field:ident, $what:literal, $e:expr) => {
            if dead {
                out.errors.push(format!("{}: skipped", $what));
            } else {
                match $e.await {
                    Ok(v) => out.$field = v,
                    Err(e) => {
                        dead |= e.kind() == io::ErrorKind::TimedOut;
                        out.errors.push(format!("{}: {e}", $what));
                    }
                }
            }
        };
    }
    part!(health, "health", health_in(&c));
    part!(neighbours, "neighbours", neighbours_in(&c));
    part!(lags, "lags", lags_in(&c));
    part!(stp, "stp", stp_in(&c));
    part!(mcast_groups, "multicast groups", mcast_in(&c));
    part!(mcast_router_ports, "multicast router ports", router_ports_in(&c));
    part!(poe, "poe", poe_in(&c));
    part!(optics, "optics", optics_in(&c));
    if dead {
        tracing::debug!("switch info: parts skipped after a timeout");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9 * b.abs().max(1.0)
    }

    #[test]
    fn sensor_scaling() {
        // dBm, units, precision 1: -118 -> -11.8
        assert!(close(sensor_value(-118, 9, 1), -11.8));
        // bias in amperes, milli, precision 2: 755 -> 7.55 mA
        assert!(close(sensor_value(755, 8, 2) * 1000.0, 7.55));
        // volts, milli, precision 0
        assert!(close(sensor_value(3301, 8, 0), 3.301));
        // °C, units, precision 0
        assert!(close(sensor_value(42, 9, 0), 42.0));
        // kilo, negative precision
        assert!(close(sensor_value(5, 10, -1), 50_000.0));
        // micro watts
        assert!(close(sensor_value(500, 7, 0), 0.0005));
        // exa / peta keep their MIB order
        assert!(close(sensor_value(1, 14, 0), 1e18));
        assert!(close(sensor_value(1, 15, 0), 1e15));
        // unknown scale = units
        assert!(close(sensor_value(7, 0, 0), 7.0));
        assert!(close(watts_to_dbm(0.001).unwrap(), 0.0));
        assert!(close(watts_to_dbm(0.0005).unwrap(), -3.010299956639812));
        assert_eq!(watts_to_dbm(0.0), None);
    }

    #[test]
    fn multicast_macs() {
        assert_eq!(group_mac(Ipv4Addr::new(239, 1, 2, 3)), [0x01, 0x00, 0x5e, 0x01, 0x02, 0x03]);
        // The 24th bit is dropped: 239.129.2.3 shares the MAC of 239.1.2.3.
        assert_eq!(group_mac(Ipv4Addr::new(239, 129, 2, 3)), group_mac(Ipv4Addr::new(239, 1, 2, 3)));
        assert_eq!(group_mac(Ipv4Addr::new(224, 0, 0, 251)), [0x01, 0x00, 0x5e, 0, 0, 251]);
    }

    #[test]
    fn device_ids() {
        assert_eq!(clean_device_id("sw-core-01(FDO21300ABC)"), "sw-core-01");
        assert_eq!(clean_device_id("sw-core-01.example.lan(FDO21300ABC)"), "sw-core-01");
        assert_eq!(clean_device_id("sw-core-01.example.lan"), "sw-core-01");
        assert_eq!(clean_device_id(" SEP001122334455 "), "SEP001122334455");
        assert_eq!(clean_device_id("10.0.0.1"), "10.0.0.1");
        assert_eq!(clean_device_id("(weird)"), "(weird)");
        assert_eq!(clean_device_id(""), "");
    }

    #[test]
    fn port_names() {
        assert!(same_port("Eth1/49", "Ethernet1/49"));
        assert!(same_port("Te1/1/1", "TenGigabitEthernet1/1/1"));
        assert!(same_port("Po1", "port-channel1"));
        assert!(same_port("gi1/0/1", "GigabitEthernet1/0/1"));
        assert!(!same_port("Eth1/4", "Ethernet1/49"));
        assert!(!same_port("Gi1/0/1", "Te1/0/1"));
        assert!(!same_port("Gi1/0/1", "Gi1/0/1.100"));
        assert!(!same_port("1", "Gi1"));
        assert!(!same_port("Transceiver", "Transceiver"));
        assert_eq!(lane_of("Ethernet1/49 Lane 3 Transceiver Receive Power Sensor"), 3);
        assert_eq!(lane_of("Te1/1/1 Receive Power Sensor"), 1);
        assert_eq!(first_token("Ethernet1/1 Transceiver"), "Ethernet1/1");
        let mut ifs = Ifs::default();
        ifs.name.insert(436207616, "Ethernet1/1".into());
        ifs.name.insert(436207617, "Ethernet1/10".into());
        ifs.descr.insert(10101, "GigabitEthernet1/0/1".into());
        assert_eq!(ifs.find("Eth1/1"), Some(436207616));
        assert_eq!(ifs.find("ethernet1/10"), Some(436207617));
        assert_eq!(ifs.find("Gi1/0/1"), Some(10101));
        assert_eq!(ifs.find(""), None);
    }

    #[test]
    fn sensor_kinds() {
        assert_eq!(reading(14, "Ethernet1/1 Lane 1 Transceiver Receive Power Sensor"), Some(Reading::Rx));
        assert_eq!(reading(14, "Te1/1/1 Transmit Power Sensor"), Some(Reading::Tx));
        assert_eq!(reading(6, "DOM RX Power"), Some(Reading::Rx));
        assert_eq!(reading(6, "PowerSupply-1 Output Power"), None);
        assert_eq!(reading(8, "Te1/1/1 Module Temperature Sensor"), Some(Reading::Temp));
        assert_eq!(reading(5, "Bias Current"), Some(Reading::Bias));
        assert_eq!(reading(4, "Supply Voltage"), Some(Reading::Volt));
        assert_eq!(reading(14, "Something"), None);
    }

    #[test]
    fn thresholds() {
        // Cisco DOM: high alarm/warn (relation 4), low warn minor, low alarm major.
        let rows = [(20, 4, 2.0), (10, 4, -1.0), (10, 2, -10.3), (20, 2, -14.3)];
        assert_eq!(low_thresholds(&rows), (Some(-10.3), Some(-14.3)));
        // Severity "other": the higher low limit warns.
        assert_eq!(low_thresholds(&[(1, 1, -14.0), (1, 1, -10.0)]), (Some(-10.0), Some(-14.0)));
        assert_eq!(low_thresholds(&[(10, 2, -9.0)]), (Some(-9.0), None));
        assert_eq!(low_thresholds(&[(30, 1, -15.0)]), (None, Some(-15.0)));
        assert_eq!(low_thresholds(&[(10, 4, 2.0)]), (None, None));
    }

    #[test]
    fn port_lists() {
        // Bridge ports 1, 2 and 49 (PortList: high bit first).
        let mut b = vec![0u8; 8];
        b[0] = 0b1100_0000;
        b[6] = 0b1000_0000;
        assert_eq!(port_list(&b), vec![1, 2, 49]);
    }
}
