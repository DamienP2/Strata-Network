//! CISCO-PTP-MIB (ciscoPtpMIB, 1.3.6.1.4.1.9.9.760): what a switch's PTP clock
//! says of itself and of each of its PTP ports. Pure parsing of the walked
//! tables; the walks are in [`crate::switchinfo::ptp`].
//!
//! Layout assumed (ciscoPtpMIBClockInfo = 760.1.2; every table's entry is `.1`,
//! a column is `<table>.1.<column>`; clock tables are indexed by domain, clock
//! type, clock instance; port tables add the port number):
//!
//! | table | name | columns read |
//! | --- | --- | --- |
//! | 1 | cPtpClockCurrentDSTable | 4 stepsRemoved, 5 offsetFromMaster, 6 meanPathDelay |
//! | 2 | cPtpClockParentDSTable | 4 parentPortIdentity, 8 GM identity, 9 / 10 GM priority 1 / 2, 11 GM class, 12 GM accuracy |
//! | 3 | cPtpClockDefaultDSTable | 4 twoStepFlag, 5 clockIdentity, 6 / 7 priority 1 / 2, 8 slaveOnly, 9 class, 10 accuracy |
//! | 4 | cPtpClockRunningTable | 4 state (freerun 1, holdover 2, acquiring 3, frequencyLocked 4, phaseAligned 5) |
//! | 5 | cPtpClockTimePropertiesDSTable | 11 time source (IEEE 1588 codes) |
//! | 8 | cPtpClockPortDSTable | 5 name, 6 port identity, 7 logAnnounceInterval, 8 announceReceiptTimeout, 9 logSyncInterval, 10 logMinDelayReqInterval, 12 delay mechanism (e2e 1, p2p 2, disabled 254) |
//! | 9 | cPtpClockPortRunningTable | 5 name, 6 state (IEEE 1588 port states 1..9), 8 ifIndex, 13 packets received, 14 packets sent |
//!
//! Where an agent does not follow it: a port's name is taken from any text
//! column of tables 9, 8 or 7 that looks like an interface name; its state from
//! an integer column of table 9 holding only 1..9; its counters from the
//! Counter columns of table 9, in order (received, then sent). A time interval
//! (ClockTimeInterval, an OCTET STRING whose content the MIB leaves open) is read
//! as text in nanoseconds ("-12", "6900 ns", "6.9 us"), as 8 bytes of IEEE 1588
//! scaledNanoseconds (ns × 2^16), or as an integer in nanoseconds.

use std::collections::{BTreeMap, BTreeSet};

use crate::ptp;
use crate::snmp::Value;

/// ciscoPtpMIBClockInfo.
pub const CLOCK_INFO: &str = "1.3.6.1.4.1.9.9.760.1.2";
/// The tables read (see the module documentation).
pub const TABLES: [u32; 7] = [3, 9, 1, 2, 4, 5, 8];
/// Table 7 (cPtpClockPortTable) is only read for names when 8 and 9 give none.
pub const PORT_TABLE: u32 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockType {
    Ordinary,
    Boundary,
    Transparent,
    BoundaryNode,
    Other(u32),
}

impl ClockType {
    pub fn of(v: u32) -> ClockType {
        match v {
            1 => ClockType::Ordinary,
            2 => ClockType::Boundary,
            3 => ClockType::Transparent,
            4 => ClockType::BoundaryNode,
            x => ClockType::Other(x),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            ClockType::Ordinary => "ordinary",
            ClockType::Boundary => "boundary",
            ClockType::Transparent => "transparent",
            ClockType::BoundaryNode => "boundaryNode",
            ClockType::Other(_) => "other",
        }
    }
}

/// IEEE 1588 port states (ClockPortState).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortState {
    Initializing,
    Faulty,
    Disabled,
    Listening,
    PreMaster,
    Master,
    Passive,
    Uncalibrated,
    Slave,
}

impl PortState {
    pub fn of(v: i64) -> Option<PortState> {
        Some(match v {
            1 => PortState::Initializing,
            2 => PortState::Faulty,
            3 => PortState::Disabled,
            4 => PortState::Listening,
            5 => PortState::PreMaster,
            6 => PortState::Master,
            7 => PortState::Passive,
            8 => PortState::Uncalibrated,
            9 => PortState::Slave,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            PortState::Initializing => "initializing",
            PortState::Faulty => "faulty",
            PortState::Disabled => "disabled",
            PortState::Listening => "listening",
            PortState::PreMaster => "preMaster",
            PortState::Master => "master",
            PortState::Passive => "passive",
            PortState::Uncalibrated => "uncalibrated",
            PortState::Slave => "slave",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DelayMech {
    E2e,
    P2p,
    Disabled,
}

impl DelayMech {
    pub fn name(self) -> &'static str {
        match self {
            DelayMech::E2e => "e2e",
            DelayMech::P2p => "p2p",
            DelayMech::Disabled => "disabled",
        }
    }
}

/// The servo of the clock (ClockStateType).
pub fn servo_name(v: i64) -> Option<&'static str> {
    Some(match v {
        1 => "freerun",
        2 => "holdover",
        3 => "acquiring",
        4 => "frequencyLocked",
        5 => "phaseAligned",
        _ => return None,
    })
}

/// One PTP port of a clock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PtpPortMib {
    /// PTP port number (the table index).
    pub number: u32,
    /// Interface name as the switch gives it (`Ethernet1/48`), '' when none.
    pub name: String,
    /// ifIndex when the switch gives it (0: match `name`).
    pub if_index: u32,
    pub state: Option<PortState>,
    pub identity: Option<String>,
    pub log_announce: Option<i8>,
    pub log_sync: Option<i8>,
    pub log_min_delay_req: Option<i8>,
    pub announce_timeout: Option<u8>,
    pub delay_mech: Option<DelayMech>,
    /// Running counters of PTP packets (all kinds), since the switch started.
    pub rx: Option<u64>,
    pub tx: Option<u64>,
}

/// One PTP clock of the switch (domain, clock type, instance) and its ports.
#[derive(Clone, Debug, PartialEq)]
pub struct PtpClockMib {
    pub domain: u8,
    pub clock_type: ClockType,
    pub instance: u32,
    /// Default DS.
    pub identity: Option<String>,
    pub two_step: Option<bool>,
    pub slave_only: Option<bool>,
    pub priority1: Option<u8>,
    pub priority2: Option<u8>,
    pub class: Option<u8>,
    pub accuracy: Option<u8>,
    /// Current DS.
    pub steps_removed: Option<u16>,
    pub offset_ns: Option<f64>,
    pub mean_path_delay_ns: Option<f64>,
    /// Parent DS: the master port this clock follows (clock identity, port number).
    pub parent: Option<String>,
    pub parent_port: Option<u16>,
    pub gm: Option<String>,
    pub gm_priority1: Option<u8>,
    pub gm_priority2: Option<u8>,
    pub gm_class: Option<u8>,
    pub gm_accuracy: Option<u8>,
    /// Running DS (freerun, holdover, acquiring, frequencyLocked, phaseAligned).
    pub servo: Option<&'static str>,
    /// Time properties DS: IEEE 1588 time source code.
    pub time_source: Option<u8>,
    pub ports: Vec<PtpPortMib>,
}

impl PtpClockMib {
    pub fn new(domain: u8, clock_type: ClockType, instance: u32) -> PtpClockMib {
        PtpClockMib {
            domain,
            clock_type,
            instance,
            identity: None,
            two_step: None,
            slave_only: None,
            priority1: None,
            priority2: None,
            class: None,
            accuracy: None,
            steps_removed: None,
            offset_ns: None,
            mean_path_delay_ns: None,
            parent: None,
            parent_port: None,
            gm: None,
            gm_priority1: None,
            gm_priority2: None,
            gm_class: None,
            gm_accuracy: None,
            servo: None,
            time_source: None,
            ports: Vec::new(),
        }
    }

    /// The port in the slave state, if any.
    pub fn slave(&self) -> Option<&PtpPortMib> {
        self.ports.iter().find(|p| p.state == Some(PortState::Slave))
    }
}

// ── Values ──

fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) => Some(*i),
        Value::Gauge(g) | Value::Ticks(g) => Some(i64::from(*g)),
        Value::Counter(c) | Value::Counter64(c) => i64::try_from(*c).ok(),
        // Some agents answer numbers as text.
        Value::Bytes(b) => std::str::from_utf8(b).ok()?.trim_matches(char::from(0)).trim().parse().ok(),
        _ => None,
    }
}

fn byte(v: &Value) -> Option<u8> {
    int(v).and_then(|i| u8::try_from(i).ok())
}

/// TruthValue: true(1), false(2).
fn truth(v: &Value) -> Option<bool> {
    match int(v)? {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// A log2 interval (Integer32), in the range PTP uses.
fn log_interval(v: &Value) -> Option<i8> {
    int(v).filter(|i| (-16..=16).contains(i)).and_then(|i| i8::try_from(i).ok())
}

fn counter(v: &Value) -> Option<u64> {
    match v {
        Value::Counter(c) | Value::Counter64(c) => Some(*c),
        Value::Gauge(g) => Some(u64::from(*g)),
        Value::Int(i) => u64::try_from(*i).ok(),
        _ => None,
    }
}

/// Text (UTF-8 without control characters, trailing NULs aside).
fn printable(b: &[u8]) -> bool {
    std::str::from_utf8(trim_nul(b)).is_ok_and(|s| !s.is_empty() && !s.chars().any(char::is_control))
}

fn trim_nul(b: &[u8]) -> &[u8] {
    let end = b.iter().rposition(|c| *c != 0).map_or(0, |i| i + 1);
    &b[..end]
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::Bytes(b) if printable(b) => Some(String::from_utf8_lossy(trim_nul(b)).trim().to_string()).filter(|s| !s.is_empty()),
        _ => None,
    }
}

/// A clock identity: 8 bytes (a port identity: 8 + 2 bytes), or written as hex
/// text. With the port number of a 10-byte port identity.
pub fn identity_of(v: &Value) -> Option<(String, Option<u16>)> {
    match v {
        Value::Bytes(b) if b.len() == 8 || b.len() == 10 && !printable(b) => {
            let id: [u8; 8] = b.get(..8)?.try_into().ok()?;
            let port = b.get(8..10).map(|p| u16::from_be_bytes([p[0], p[1]]));
            Some((ptp::identity(&id), port))
        }
        Value::Bytes(_) => {
            let t = text(v)?;
            // "7069.5aff.feee.6f79", "70:69:5A:FF:FE:EE:6F:79", "70695afffeee6f79.31", "…-1"
            if let Some(id) = ptp::normalize_identity(&t) {
                return Some((id, None));
            }
            let hex: String = t.chars().take_while(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '-' | '.' | ' ')).filter(char::is_ascii_hexdigit).collect();
            let id = ptp::normalize_identity(hex.get(..16)?)?;
            let port = t.rsplit(|c: char| !c.is_ascii_digit()).next().and_then(|p| p.parse().ok()).filter(|_| hex.len() > 16 || t.len() > 23);
            Some((id, port))
        }
        _ => None,
    }
}

/// A ClockTimeInterval in nanoseconds (see the module documentation).
pub fn time_interval_ns(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Gauge(g) => Some(f64::from(*g)),
        Value::Counter(c) | Value::Counter64(c) => Some(*c as f64),
        Value::Bytes(b) if printable(b) => {
            let t = String::from_utf8_lossy(trim_nul(b)).trim().to_ascii_lowercase().replace('µ', "u");
            let (num, scale) = [("ns", 1.0), ("us", 1e3), ("ms", 1e6), ("s", 1e9)].iter().find_map(|(u, k)| t.strip_suffix(u).map(|n| (n.trim().to_string(), *k))).unwrap_or((t.clone(), 1.0));
            num.replace(' ', "").parse::<f64>().ok().filter(|x| x.is_finite()).map(|x| x * scale)
        }
        Value::Bytes(b) if b.len() == 8 => {
            let raw = i64::from_be_bytes(b.as_slice().try_into().ok()?);
            Some(raw as f64 / 65536.0)
        }
        _ => None,
    }
}

/// Whether a text names an interface (`Ethernet1/48`, `Eth1/48`, `Po1`, `Gi1/0/1`).
fn looks_like_port(s: &str) -> bool {
    let s = s.trim();
    s.len() <= 64 && s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().any(|c| c.is_ascii_digit()) && !s.contains(char::is_whitespace)
}

// ── Tables ──

/// A walked table: (suffix after `<table>.1`: column then index, value).
pub type Rows = Vec<(Vec<u32>, Value)>;

type ClockKey = (u8, u32, u32);
type PortKey = (u8, u32, u32, u32);

fn clock_key(idx: &[u32]) -> Option<ClockKey> {
    match idx {
        [d, t, i, ..] => Some((u8::try_from(*d).ok()?, *t, *i)),
        _ => None,
    }
}

fn port_key(idx: &[u32]) -> Option<PortKey> {
    match idx {
        [d, t, i, p, ..] => Some((u8::try_from(*d).ok()?, *t, *i, *p)),
        _ => None,
    }
}

/// table → column → index → value.
struct Cells<'a>(BTreeMap<u32, BTreeMap<u32, BTreeMap<&'a [u32], &'a Value>>>);

impl<'a> Cells<'a> {
    fn new(tables: &'a BTreeMap<u32, Rows>) -> Cells<'a> {
        let mut m: BTreeMap<u32, BTreeMap<u32, BTreeMap<&'a [u32], &'a Value>>> = BTreeMap::new();
        for (t, rows) in tables {
            for (suffix, v) in rows {
                if let [col, idx @ ..] = suffix.as_slice() {
                    if !idx.is_empty() && !v.is_absent() && *v != Value::Null {
                        m.entry(*t).or_default().entry(*col).or_default().insert(idx, v);
                    }
                }
            }
        }
        Cells(m)
    }

    fn col(&self, t: u32, c: u32) -> impl Iterator<Item = (&'a [u32], &'a Value)> + '_ {
        self.0.get(&t).and_then(|m| m.get(&c)).into_iter().flat_map(|m| m.iter().map(|(k, v)| (*k, *v)))
    }

    fn columns(&self, t: u32) -> Vec<u32> {
        self.0.get(&t).map(|m| m.keys().copied().collect()).unwrap_or_default()
    }

    /// Whether every value of a column passes `f` (and there is one).
    fn all(&self, t: u32, c: u32, f: impl Fn(&Value) -> bool) -> bool {
        let mut n = 0;
        for (_, v) in self.col(t, c) {
            if !f(v) {
                return false;
            }
            n += 1;
        }
        n > 0
    }
}

/// The clocks and ports of the walked tables (by table number, see [`TABLES`]);
/// empty when the switch has no PTP clock.
pub fn parse(tables: &BTreeMap<u32, Rows>) -> Vec<PtpClockMib> {
    let cells = Cells::new(tables);
    let mut clocks: BTreeMap<ClockKey, PtpClockMib> = BTreeMap::new();
    fn get(m: &mut BTreeMap<ClockKey, PtpClockMib>, k: ClockKey) -> &mut PtpClockMib {
        m.entry(k).or_insert_with(|| PtpClockMib::new(k.0, ClockType::of(k.1), k.2))
    }

    // Default DS.
    for (c, f) in [(4u32, 0u8), (5, 1), (6, 2), (7, 3), (8, 4), (9, 5), (10, 6)] {
        for (idx, v) in cells.col(3, c) {
            let Some(k) = clock_key(idx) else { continue };
            let x = get(&mut clocks, k);
            match f {
                0 => x.two_step = truth(v),
                1 => x.identity = identity_of(v).map(|i| i.0),
                2 => x.priority1 = byte(v),
                3 => x.priority2 = byte(v),
                4 => x.slave_only = truth(v),
                5 => x.class = byte(v),
                _ => x.accuracy = byte(v),
            }
        }
    }
    // Current DS.
    for c in [4u32, 5, 6] {
        for (idx, v) in cells.col(1, c) {
            let Some(k) = clock_key(idx) else { continue };
            let x = get(&mut clocks, k);
            match c {
                4 => x.steps_removed = int(v).and_then(|i| u16::try_from(i).ok()),
                5 => x.offset_ns = time_interval_ns(v),
                _ => x.mean_path_delay_ns = time_interval_ns(v),
            }
        }
    }
    // Parent DS.
    for c in [4u32, 8, 9, 10, 11, 12] {
        for (idx, v) in cells.col(2, c) {
            let Some(k) = clock_key(idx) else { continue };
            let x = get(&mut clocks, k);
            match c {
                4 => {
                    if let Some((id, port)) = identity_of(v) {
                        x.parent = Some(id);
                        x.parent_port = port;
                    }
                }
                8 => x.gm = identity_of(v).map(|i| i.0),
                9 => x.gm_priority1 = byte(v),
                10 => x.gm_priority2 = byte(v),
                11 => x.gm_class = byte(v),
                _ => x.gm_accuracy = byte(v),
            }
        }
    }
    for (idx, v) in cells.col(4, 4) {
        if let Some(k) = clock_key(idx) {
            get(&mut clocks, k).servo = int(v).and_then(servo_name);
        }
    }
    for (idx, v) in cells.col(5, 11) {
        if let Some(k) = clock_key(idx) {
            get(&mut clocks, k).time_source = byte(v);
        }
    }

    // Ports: every port key of tables 7, 8, 9.
    let mut keys: BTreeSet<PortKey> = BTreeSet::new();
    for t in [9u32, 8, PORT_TABLE] {
        for c in cells.columns(t) {
            keys.extend(cells.col(t, c).filter_map(|(idx, _)| port_key(idx)));
        }
    }
    let mut ports: BTreeMap<PortKey, PtpPortMib> = keys.iter().map(|k| (*k, PtpPortMib { number: k.3, ..PtpPortMib::default() })).collect();
    let mut set = |t: u32, c: u32, f: &dyn Fn(&mut PtpPortMib, &Value)| {
        for (idx, v) in cells.col(t, c) {
            if let Some(p) = port_key(idx).and_then(|k| ports.get_mut(&k)) {
                f(p, v);
            }
        }
    };
    // Names: the expected columns, else any column of interface-like texts.
    let name_ok = |v: &Value| text(v).is_some_and(|s| looks_like_port(&s));
    let mut name_cols: Vec<(u32, u32)> = [(9u32, 5u32), (8, 5), (PORT_TABLE, 5)].into_iter().filter(|(t, c)| cells.all(*t, *c, name_ok)).collect();
    if name_cols.is_empty() {
        for t in [9u32, 8, PORT_TABLE] {
            name_cols.extend(cells.columns(t).into_iter().filter(|c| cells.all(t, *c, name_ok)).map(|c| (t, c)));
        }
    }
    for (t, c) in name_cols.into_iter().rev() {
        set(t, c, &|p, v| p.name = text(v).unwrap_or_default());
    }
    // State: column 6, else an integer column of table 9 that holds port states only (master or slave among them).
    let state_ok = |v: &Value| matches!(v, Value::Int(i) if (1..=9).contains(i));
    let state_col = if cells.all(9, 6, state_ok) {
        Some(6)
    } else {
        cells.columns(9).into_iter().filter(|c| *c != 7).find(|c| cells.all(9, *c, state_ok) && cells.col(9, *c).any(|(_, v)| matches!(v, Value::Int(6 | 9))))
    };
    if let Some(c) = state_col {
        set(9, c, &|p, v| p.state = int(v).and_then(PortState::of));
    }
    set(9, 8, &|p, v| {
        if let Some(i) = int(v).filter(|i| *i > 0).and_then(|i| u32::try_from(i).ok()) {
            p.if_index = i;
        }
    });
    // Counters: 13 received, 14 sent; else the Counter columns of table 9 in order.
    let is_counter = |v: &Value| matches!(v, Value::Counter(_) | Value::Counter64(_));
    let (rx_col, tx_col) = if cells.all(9, 13, is_counter) || cells.all(9, 14, is_counter) {
        (Some(13), Some(14))
    } else {
        let cs: Vec<u32> = cells.columns(9).into_iter().filter(|c| cells.all(9, *c, is_counter)).collect();
        (cs.first().copied(), cs.get(1).copied())
    };
    if let Some(c) = rx_col {
        set(9, c, &|p, v| p.rx = counter(v));
    }
    if let Some(c) = tx_col {
        set(9, c, &|p, v| p.tx = counter(v));
    }
    // Port DS.
    set(8, 6, &|p, v| p.identity = identity_of(v).map(|i| i.0));
    set(8, 7, &|p, v| p.log_announce = log_interval(v));
    set(8, 8, &|p, v| p.announce_timeout = byte(v));
    set(8, 9, &|p, v| p.log_sync = log_interval(v));
    set(8, 10, &|p, v| p.log_min_delay_req = log_interval(v));
    set(8, 12, &|p, v| {
        p.delay_mech = match int(v) {
            Some(1) => Some(DelayMech::E2e),
            Some(2) => Some(DelayMech::P2p),
            Some(254) => Some(DelayMech::Disabled),
            _ => None,
        }
    });

    for (k, p) in ports {
        get(&mut clocks, (k.0, k.1, k.2)).ports.push(p);
    }
    clocks
        .into_values()
        .filter(|c| c.identity.is_some() || !c.ports.is_empty())
        .map(|mut c| {
            c.ports.sort_by_key(|p| p.number);
            c
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDX: [u32; 3] = [127, 2, 0];

    fn row(col: u32, idx: &[u32], v: Value) -> (Vec<u32>, Value) {
        let mut s = vec![col];
        s.extend_from_slice(idx);
        (s, v)
    }

    fn port_idx(n: u32) -> Vec<u32> {
        let mut v = IDX.to_vec();
        v.push(n);
        v
    }

    fn nexus_tables() -> BTreeMap<u32, Rows> {
        let id = vec![0x70, 0x69, 0x5A, 0xFF, 0xFE, 0xEE, 0x6F, 0x79];
        let gm = vec![0xEC, 0x46, 0x70, 0xFF, 0xFE, 0x00, 0x1A, 0x2B];
        let mut parent = vec![0x00, 0x3A, 0x9C, 0xFF, 0xFE, 0x11, 0x22, 0x33];
        parent.extend_from_slice(&[0, 1]);
        let mut t: BTreeMap<u32, Rows> = BTreeMap::new();
        t.insert(3, vec![row(4, &IDX, Value::Int(1)), row(5, &IDX, Value::Bytes(id)), row(6, &IDX, Value::Int(128)), row(7, &IDX, Value::Int(128)), row(9, &IDX, Value::Int(248)), row(10, &IDX, Value::Int(0xFE))]);
        // Offset as 8 bytes of scaled nanoseconds (−100 ns), delay as text.
        t.insert(1, vec![row(4, &IDX, Value::Gauge(2)), row(5, &IDX, Value::Bytes((-100i64 * 65536).to_be_bytes().to_vec())), row(6, &IDX, Value::Bytes(b"6900".to_vec()))]);
        t.insert(
            2,
            vec![
                row(4, &IDX, Value::Bytes(parent)),
                row(8, &IDX, Value::Bytes(gm)),
                row(9, &IDX, Value::Int(128)),
                row(10, &IDX, Value::Int(128)),
                row(11, &IDX, Value::Int(6)),
                row(12, &IDX, Value::Int(0x21)),
            ],
        );
        t.insert(4, vec![row(4, &IDX, Value::Int(5))]);
        t.insert(5, vec![row(11, &IDX, Value::Int(0x20))]);
        let mut p8 = Vec::new();
        let mut p9 = Vec::new();
        for (n, name, state, ann, rx, tx) in [(1u32, "Ethernet1/48", 9, -2, 2500u64, 800u64), (2, "Ethernet1/12", 6, 0, 800, 2500), (3, "Ethernet1/20", 3, 0, 0, 0)] {
            let i = port_idx(n);
            p8.push(row(5, &i, Value::Bytes(name.as_bytes().to_vec())));
            p8.push(row(7, &i, Value::Int(ann)));
            p8.push(row(8, &i, Value::Int(3)));
            p8.push(row(9, &i, Value::Int(-3)));
            p8.push(row(10, &i, Value::Int(-3)));
            p8.push(row(12, &i, Value::Int(1)));
            p9.push(row(5, &i, Value::Bytes(name.as_bytes().to_vec())));
            p9.push(row(6, &i, Value::Int(state)));
            p9.push(row(7, &i, Value::Int(if state == 9 { 2 } else { 1 })));
            p9.push(row(8, &i, Value::Int(if n == 1 { 436207616 + 47 * 4096 } else { 0 })));
            p9.push(row(13, &i, Value::Counter64(rx)));
            p9.push(row(14, &i, Value::Counter64(tx)));
        }
        t.insert(8, p8);
        t.insert(9, p9);
        t
    }

    #[test]
    fn parses_a_boundary_clock() {
        let clocks = parse(&nexus_tables());
        assert_eq!(clocks.len(), 1);
        let c = &clocks[0];
        assert_eq!((c.domain, c.clock_type, c.instance), (127, ClockType::Boundary, 0));
        assert_eq!(c.identity.as_deref(), Some("70:69:5A:FF:FE:EE:6F:79"));
        assert_eq!((c.two_step, c.priority1, c.class, c.accuracy), (Some(true), Some(128), Some(248), Some(0xFE)));
        assert_eq!(c.steps_removed, Some(2));
        assert_eq!(c.offset_ns, Some(-100.0));
        assert_eq!(c.mean_path_delay_ns, Some(6900.0));
        assert_eq!((c.parent.as_deref(), c.parent_port), (Some("00:3A:9C:FF:FE:11:22:33"), Some(1)));
        assert_eq!(c.gm.as_deref(), Some("EC:46:70:FF:FE:00:1A:2B"));
        assert_eq!((c.gm_priority1, c.gm_priority2, c.gm_class, c.gm_accuracy), (Some(128), Some(128), Some(6), Some(0x21)));
        assert_eq!((c.servo, c.time_source), (Some("phaseAligned"), Some(0x20)));
        assert_eq!(c.ports.len(), 3);
        let s = c.slave().unwrap();
        assert_eq!((s.number, s.name.as_str(), s.if_index, s.log_announce, s.log_sync, s.log_min_delay_req), (1, "Ethernet1/48", 436207616 + 47 * 4096, Some(-2), Some(-3), Some(-3)));
        assert_eq!((s.announce_timeout, s.delay_mech, s.rx, s.tx), (Some(3), Some(DelayMech::E2e), Some(2500), Some(800)));
        assert_eq!((c.ports[1].state, c.ports[1].if_index, c.ports[1].log_announce), (Some(PortState::Master), 0, Some(0)));
        assert_eq!(c.ports[2].state, Some(PortState::Disabled));
    }

    #[test]
    fn absent_mib_gives_nothing() {
        assert!(parse(&BTreeMap::new()).is_empty());
        // Garbage that is not a clock: no index, short index.
        let mut t: BTreeMap<u32, Rows> = BTreeMap::new();
        t.insert(3, vec![(vec![5], Value::Int(1)), (vec![5, 1], Value::Bytes(vec![1, 2]))]);
        t.insert(9, vec![(vec![6, 1, 2], Value::Int(6))]);
        assert!(parse(&t).is_empty());
    }

    #[test]
    fn discovers_moved_columns() {
        // An agent with the name in column 15 (text), the state in column 11, counters 20/21 (Counter64).
        let mut t: BTreeMap<u32, Rows> = BTreeMap::new();
        let i = port_idx(5);
        t.insert(9, vec![row(11, &i, Value::Int(6)), row(15, &i, Value::Bytes(b"Eth1/10".to_vec())), row(20, &i, Value::Counter64(10)), row(21, &i, Value::Counter64(20)), row(22, &i, Value::Bytes(b"some text".to_vec()))]);
        let c = parse(&t);
        assert_eq!(c.len(), 1);
        let p = &c[0].ports[0];
        assert_eq!((p.number, p.name.as_str(), p.state, p.rx, p.tx), (5, "Eth1/10", Some(PortState::Master), Some(10), Some(20)));
    }

    #[test]
    fn values() {
        assert_eq!(time_interval_ns(&Value::Bytes(b"-12".to_vec())), Some(-12.0));
        assert_eq!(time_interval_ns(&Value::Bytes(b"6.9 us".to_vec())), Some(6900.0));
        assert_eq!(time_interval_ns(&Value::Bytes(b"85 ns\0".to_vec())), Some(85.0));
        assert_eq!(time_interval_ns(&Value::Int(-40)), Some(-40.0));
        assert_eq!(time_interval_ns(&Value::Bytes((6900i64 << 16).to_be_bytes().to_vec())), Some(6900.0));
        assert_eq!(time_interval_ns(&Value::Bytes(vec![1, 2, 3])), None);
        assert_eq!(time_interval_ns(&Value::Bytes(b"n/a".to_vec())), None);
        assert_eq!(identity_of(&Value::Bytes(b"7069.5aff.feee.6f79".to_vec())).map(|x| x.0).as_deref(), Some("70:69:5A:FF:FE:EE:6F:79"));
        assert_eq!(identity_of(&Value::Bytes(b"70:69:5A:FF:FE:EE:6F:79".to_vec())), Some(("70:69:5A:FF:FE:EE:6F:79".to_string(), None)));
        assert_eq!(identity_of(&Value::Int(3)), None);
        assert_eq!(identity_of(&Value::Bytes(vec![1, 2, 3])), None);
        assert_eq!(log_interval(&Value::Int(-3)), Some(-3));
        assert_eq!(log_interval(&Value::Int(127)), None);
        assert_eq!(PortState::of(10), None);
        assert_eq!(ClockType::of(2).name(), "boundary");
        assert_eq!(servo_name(9), None);
    }
}
