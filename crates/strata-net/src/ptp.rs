//! PTP (IEEE 1588-2008), passive: Announce messages give the clocks that offer
//! themselves as grandmaster (compared as the best master clock algorithm
//! does), Delay_Req messages give the devices that follow a master.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

pub const MSG_SYNC: u8 = 0x0;
pub const MSG_DELAY_REQ: u8 = 0x1;
pub const MSG_PDELAY_REQ: u8 = 0x2;
pub const MSG_ANNOUNCE: u8 = 0xB;

/// An announcer silent for longer is not a candidate any more (3 × a 2 s interval, and some slack).
pub const ANNOUNCE_STALE_MS: u64 = 10_000;
/// Clocks and devices not heard for longer are forgotten.
pub const FORGET_MS: u64 = 3_600_000;

/// The common header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub msg_type: u8,
    pub domain: u8,
    /// Clock identity of the sender's port.
    pub clock: [u8; 8],
    pub port: u16,
}

/// The dataset of an Announce message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announce {
    pub priority1: u8,
    pub clock_class: u8,
    pub clock_accuracy: u8,
    pub variance: u16,
    pub priority2: u8,
    pub gm: [u8; 8],
    pub steps_removed: u16,
    pub time_source: u8,
    pub utc_offset: i16,
}

/// A PTP version 2 message: its header, and its dataset when it is an Announce.
pub fn parse(p: &[u8]) -> Option<(Header, Option<Announce>)> {
    if p.len() < 34 || p[1] & 0x0F != 2 {
        return None;
    }
    let h = Header { msg_type: p[0] & 0x0F, domain: p[4], clock: p[20..28].try_into().ok()?, port: u16::from_be_bytes([p[28], p[29]]) };
    let a = (h.msg_type == MSG_ANNOUNCE && p.len() >= 64).then(|| Announce {
        utc_offset: i16::from_be_bytes([p[44], p[45]]),
        priority1: p[47],
        clock_class: p[48],
        clock_accuracy: p[49],
        variance: u16::from_be_bytes([p[50], p[51]]),
        priority2: p[52],
        gm: p[53..61].try_into().unwrap_or_default(),
        steps_removed: u16::from_be_bytes([p[61], p[62]]),
        time_source: p[63],
    });
    Some((h, a))
}

/// "EC:46:70:FF:FE:00:1A:2B".
pub fn identity(id: &[u8; 8]) -> String {
    id.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// The MAC inside an EUI-64 clock identity built from one (xx:xx:xx:FF:FE:xx:xx:xx).
pub fn identity_mac(id: &str) -> Option<String> {
    let b: Vec<&str> = id.split(':').collect();
    (b.len() == 8 && b[3].eq_ignore_ascii_case("FF") && b[4].eq_ignore_ascii_case("FE")).then(|| [b[0], b[1], b[2], b[5], b[6], b[7]].join(":").to_ascii_uppercase())
}

/// Clock accuracy (IEEE 1588 table 6) in nanoseconds.
pub fn accuracy_ns(a: u8) -> Option<f64> {
    let table = [25.0, 100.0, 250.0, 1e3, 2.5e3, 1e4, 2.5e4, 1e5, 2.5e5, 1e6, 2.5e6, 1e7, 2.5e7, 1e8, 2.5e8, 1e9, 1e10];
    (0x20..=0x31).contains(&a).then(|| table[usize::from(a - 0x20)])
}

/// Time source (IEEE 1588 table 7).
pub fn time_source_name(t: u8) -> &'static str {
    match t {
        0x10 => "atomic",
        0x20 => "gps",
        0x30 => "radio",
        0x40 => "ptp",
        0x50 => "ntp",
        0x60 => "handSet",
        0x90 => "other",
        0xA0 => "internalOscillator",
        _ => "unknown",
    }
}

/// A clock that sends Announce messages.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Announcer {
    pub ip: Ipv4Addr,
    pub domain: u8,
    /// The sender's clock identity.
    pub clock: String,
    /// The grandmaster it announces (itself when it is the grandmaster).
    pub gm: String,
    pub priority1: u8,
    pub clock_class: u8,
    pub clock_accuracy: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accuracy_ns: Option<f64>,
    pub variance: u16,
    pub priority2: u8,
    pub steps_removed: u16,
    pub time_source: String,
    pub utc_offset: i16,
    /// Unix ms.
    pub first_seen: u64,
    pub last_seen: u64,
    pub count: u64,
    /// Announce messages per second (from the interval between the last ones).
    pub rate: f32,
}

impl Announcer {
    /// The order of the best master clock algorithm (smaller is better): the
    /// grandmaster's dataset, then the distance to it, then the sender.
    pub fn bmca_key(&self) -> (u8, u8, u8, u16, u8, &str, u16, &str) {
        (self.priority1, self.clock_class, self.clock_accuracy, self.variance, self.priority2, &self.gm, self.steps_removed, &self.clock)
    }
}

/// BMCA comparison: `Less` when `a` is the better master.
pub fn compare(a: &Announcer, b: &Announcer) -> Ordering {
    a.bmca_key().cmp(&b.bmca_key())
}

/// The best announcer heard in the last [`ANNOUNCE_STALE_MS`] (of one domain, or of all).
pub fn best(list: &[Announcer], domain: Option<u8>, now: u64) -> Option<&Announcer> {
    list.iter().filter(|a| domain.is_none_or(|d| a.domain == d) && now.saturating_sub(a.last_seen) <= ANNOUNCE_STALE_MS).min_by(|a, b| compare(a, b))
}

/// A device that asks a master for the delay (Delay_Req or Pdelay_Req).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Follower {
    pub clock: String,
    pub ip: Ipv4Addr,
    pub domain: u8,
    pub first_seen: u64,
    pub last_seen: u64,
    pub count: u64,
}

/// What was heard.
#[derive(Default, Debug)]
pub struct Tracker {
    announcers: HashMap<(Ipv4Addr, u8, [u8; 8]), Announcer>,
    followers: HashMap<([u8; 8], u8), Follower>,
}

impl Tracker {
    /// A PTP packet from `ip`.
    pub fn record(&mut self, ip: Ipv4Addr, p: &[u8], now: u64) {
        let Some((h, a)) = parse(p) else { return };
        match (h.msg_type, a) {
            (MSG_ANNOUNCE, Some(a)) => {
                let e = self.announcers.entry((ip, h.domain, h.clock)).or_insert_with(|| Announcer {
                    ip,
                    domain: h.domain,
                    clock: identity(&h.clock),
                    gm: String::new(),
                    priority1: 0,
                    clock_class: 0,
                    clock_accuracy: 0,
                    accuracy_ns: None,
                    variance: 0,
                    priority2: 0,
                    steps_removed: 0,
                    time_source: String::new(),
                    utc_offset: 0,
                    first_seen: now,
                    last_seen: now,
                    count: 0,
                    rate: 0.0,
                });
                if e.count > 0 && now > e.last_seen {
                    let r = 1000.0 / (now - e.last_seen) as f32;
                    e.rate = if e.rate == 0.0 { r } else { e.rate * 0.7 + r * 0.3 };
                }
                e.gm = identity(&a.gm);
                e.priority1 = a.priority1;
                e.clock_class = a.clock_class;
                e.clock_accuracy = a.clock_accuracy;
                e.accuracy_ns = accuracy_ns(a.clock_accuracy);
                e.variance = a.variance;
                e.priority2 = a.priority2;
                e.steps_removed = a.steps_removed;
                e.time_source = time_source_name(a.time_source).to_string();
                e.utc_offset = a.utc_offset;
                e.last_seen = now;
                e.count += 1;
            }
            (MSG_DELAY_REQ | MSG_PDELAY_REQ, _) => {
                let e = self.followers.entry((h.clock, h.domain)).or_insert_with(|| Follower { clock: identity(&h.clock), ip, domain: h.domain, first_seen: now, last_seen: now, count: 0 });
                e.ip = ip;
                e.last_seen = now;
                e.count += 1;
            }
            _ => {}
        }
    }

    /// Forgets what was not heard for an hour.
    pub fn prune(&mut self, now: u64) {
        self.announcers.retain(|_, a| now.saturating_sub(a.last_seen) < FORGET_MS);
        self.followers.retain(|_, f| now.saturating_sub(f.last_seen) < FORGET_MS);
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut announcers: Vec<Announcer> = self.announcers.values().cloned().collect();
        announcers.sort_by(compare);
        let mut followers: Vec<Follower> = self.followers.values().cloned().collect();
        followers.sort_by_key(|f| (u32::from(f.ip), f.domain));
        Snapshot { announcers, followers }
    }
}

/// The clocks heard.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// Best first (BMCA order).
    pub announcers: Vec<Announcer>,
    pub followers: Vec<Follower>,
}

/// A PTP message, for tests and tools: header + body.
pub fn build(msg_type: u8, domain: u8, clock: [u8; 8], announce: Option<&Announce>) -> Vec<u8> {
    let len = if announce.is_some() { 64 } else { 44 };
    let mut p = vec![0u8; len];
    p[0] = msg_type;
    p[1] = 2;
    p[2..4].copy_from_slice(&(len as u16).to_be_bytes());
    p[4] = domain;
    p[20..28].copy_from_slice(&clock);
    p[28..30].copy_from_slice(&1u16.to_be_bytes());
    if let Some(a) = announce {
        p[44..46].copy_from_slice(&a.utc_offset.to_be_bytes());
        p[47] = a.priority1;
        p[48] = a.clock_class;
        p[49] = a.clock_accuracy;
        p[50..52].copy_from_slice(&a.variance.to_be_bytes());
        p[52] = a.priority2;
        p[53..61].copy_from_slice(&a.gm);
        p[61..63].copy_from_slice(&a.steps_removed.to_be_bytes());
        p[63] = a.time_source;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    const GM1: [u8; 8] = [0xEC, 0x46, 0x70, 0xFF, 0xFE, 0x00, 0x1A, 0x2B];
    const GM2: [u8; 8] = [0xEC, 0x46, 0x70, 0xFF, 0xFE, 0x00, 0x1A, 0x3C];

    fn ann(p1: u8, class: u8, p2: u8, gm: [u8; 8]) -> Announce {
        Announce { priority1: p1, clock_class: class, clock_accuracy: 0x21, variance: 0x4E5D, priority2: p2, gm, steps_removed: 0, time_source: 0x20, utc_offset: 37 }
    }

    #[test]
    fn parses_announce_and_delay_req() {
        let p = build(MSG_ANNOUNCE, 127, GM1, Some(&ann(128, 6, 128, GM1)));
        let (h, a) = parse(&p).unwrap();
        assert_eq!((h.msg_type, h.domain, h.clock), (MSG_ANNOUNCE, 127, GM1));
        let a = a.unwrap();
        assert_eq!((a.priority1, a.clock_class, a.priority2, a.gm, a.time_source), (128, 6, 128, GM1, 0x20));
        assert_eq!(identity(&GM1), "EC:46:70:FF:FE:00:1A:2B");
        assert_eq!(identity_mac("EC:46:70:FF:FE:00:1A:2B").as_deref(), Some("EC:46:70:00:1A:2B"));
        assert_eq!(identity_mac("00:11:22:33:44:55:66:77"), None);
        assert_eq!(accuracy_ns(0x21), Some(100.0));
        let (h, a) = parse(&build(MSG_DELAY_REQ, 127, [1; 8], None)).unwrap();
        assert_eq!((h.msg_type, a), (MSG_DELAY_REQ, None));
        assert!(parse(&[0u8; 20]).is_none());
    }

    #[test]
    fn bmca_order() {
        let mut t = Tracker::default();
        let ip = |n| Ipv4Addr::new(10, 99, 0, n);
        // GM2 announces priority2 129: GM1 wins on priority2.
        t.record(ip(48), &build(MSG_ANNOUNCE, 127, GM2, Some(&ann(128, 6, 129, GM2))), 1000);
        t.record(ip(47), &build(MSG_ANNOUNCE, 127, GM1, Some(&ann(128, 6, 128, GM1))), 1000);
        // A free-running clock (class 248) in another domain, priority1 lower but class does not matter first…
        t.record(ip(70), &build(MSG_ANNOUNCE, 0, [7; 8], Some(&ann(127, 248, 128, [7; 8]))), 1000);
        let s = t.snapshot();
        // …priority1 comes first: 127 beats 128.
        assert_eq!(s.announcers[0].ip, ip(70));
        assert_eq!(best(&s.announcers, Some(127), 2000).unwrap().ip, ip(47));
        // Same priority1: class decides.
        t.record(ip(71), &build(MSG_ANNOUNCE, 127, [8; 8], Some(&ann(128, 7, 1, [8; 8]))), 1000);
        assert_eq!(best(&t.snapshot().announcers, Some(127), 2000).unwrap().ip, ip(47));
        // Stale announcers are no candidates.
        t.record(ip(48), &build(MSG_ANNOUNCE, 127, GM2, Some(&ann(128, 6, 129, GM2))), 20_000);
        assert_eq!(best(&t.snapshot().announcers, Some(127), 20_500).unwrap().ip, ip(48));
        // Equal datasets: the grandmaster identity decides (smaller wins).
        let a = Announcer { ip: ip(1), domain: 0, clock: "B".into(), gm: "B".into(), priority1: 1, clock_class: 6, clock_accuracy: 0x21, accuracy_ns: None, variance: 0, priority2: 1, steps_removed: 0, time_source: String::new(), utc_offset: 0, first_seen: 0, last_seen: 0, count: 0, rate: 0.0 };
        let b = Announcer { gm: "A".into(), clock: "A".into(), ..a.clone() };
        assert_eq!(compare(&b, &a), Ordering::Less);
        let c = Announcer { steps_removed: 1, ..b.clone() };
        assert_eq!(compare(&b, &c), Ordering::Less);
    }

    #[test]
    fn followers() {
        let mut t = Tracker::default();
        t.record(Ipv4Addr::new(10, 10, 1, 11), &build(MSG_DELAY_REQ, 127, [1; 8], None), 5);
        t.record(Ipv4Addr::new(10, 10, 1, 11), &build(MSG_DELAY_REQ, 127, [1; 8], None), 6);
        t.record(Ipv4Addr::new(10, 10, 1, 70), &build(MSG_DELAY_REQ, 0, [2; 8], None), 6);
        t.record(Ipv4Addr::new(10, 10, 1, 70), &build(MSG_SYNC, 0, [2; 8], None), 6);
        let s = t.snapshot();
        assert_eq!(s.followers.len(), 2);
        assert_eq!((s.followers[0].count, s.followers[0].domain), (2, 127));
        t.prune(FORGET_MS + 7);
        assert!(t.snapshot().followers.is_empty());
    }
}
