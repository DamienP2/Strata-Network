//! Read-only SNMP v2c: `get` and table walks (GETBULK), and the tables Strata
//! Network reads from routers and switches without changing anything: ARP,
//! LLDP neighbours, MAC address tables, interface names, multicast routes.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;

use crate::mac::{self, Mac};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Oid(pub Vec<u32>);

impl Oid {
    pub fn parse(s: &str) -> Oid {
        Oid(s.trim_start_matches('.').split('.').filter_map(|p| p.parse().ok()).collect())
    }
    pub fn starts_with(&self, base: &Oid) -> bool {
        self.0.starts_with(&base.0)
    }
    /// The part after `base` (the row index of a table cell).
    pub fn suffix(&self, base: &Oid) -> &[u32] {
        &self.0[base.0.len().min(self.0.len())..]
    }
    pub fn child(&self, n: u32) -> Oid {
        let mut v = self.0.clone();
        v.push(n);
        Oid(v)
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s: Vec<String> = self.0.iter().map(u32::to_string).collect();
        f.write_str(&s.join("."))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Bytes(Vec<u8>),
    Null,
    Oid(Oid),
    Ip(Ipv4Addr),
    /// Counter32.
    Counter(u64),
    Counter64(u64),
    Gauge(u32),
    Ticks(u32),
    /// An unknown type.
    Missing,
    NoSuchObject,
    NoSuchInstance,
    EndOfView,
}

impl Value {
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Int(i) => u64::try_from(*i).ok(),
            Value::Counter(c) | Value::Counter64(c) => Some(*c),
            Value::Gauge(g) | Value::Ticks(g) => Some(u64::from(*g)),
            _ => None,
        }
    }
    /// No value: an unknown type, no such object or instance, end of the MIB view.
    pub fn is_absent(&self) -> bool {
        matches!(self, Value::Missing | Value::NoSuchObject | Value::NoSuchInstance | Value::EndOfView)
    }
    /// The ASN.1 / SMI name of the type ("OctetString", "Counter64"…).
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Int(_) => "Integer",
            Value::Bytes(_) => "OctetString",
            Value::Null => "Null",
            Value::Oid(_) => "OID",
            Value::Ip(_) => "IpAddress",
            Value::Counter(_) => "Counter32",
            Value::Counter64(_) => "Counter64",
            Value::Gauge(_) => "Gauge32",
            Value::Ticks(_) => "TimeTicks",
            Value::Missing => "Unknown",
            Value::NoSuchObject => "NoSuchObject",
            Value::NoSuchInstance => "NoSuchInstance",
            Value::EndOfView => "EndOfMibView",
        }
    }
    pub fn as_text(&self) -> Option<String> {
        match self {
            Value::Bytes(b) => Some(String::from_utf8_lossy(b).trim_matches(char::from(0)).trim().to_string()),
            Value::Int(i) => Some(i.to_string()),
            Value::Ip(ip) => Some(ip.to_string()),
            _ => None,
        }
    }
    pub fn as_mac(&self) -> Option<Mac> {
        match self {
            Value::Bytes(b) if b.len() == 6 => b.as_slice().try_into().ok(),
            Value::Bytes(b) => mac::parse(&String::from_utf8_lossy(b)),
            _ => None,
        }
    }
}

// ── BER ──

fn put_len(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
}

pub(crate) fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    put_len(&mut out, body.len());
    out.extend_from_slice(body);
    out
}

pub(crate) fn int(v: i64) -> Vec<u8> {
    let bytes = v.to_be_bytes();
    let mut start = 0;
    while start < 7 && ((bytes[start] == 0 && bytes[start + 1] & 0x80 == 0) || (bytes[start] == 0xFF && bytes[start + 1] & 0x80 != 0)) {
        start += 1;
    }
    tlv(0x02, &bytes[start..])
}

fn oid(o: &Oid) -> Vec<u8> {
    let mut body = Vec::new();
    let v = &o.0;
    if v.len() >= 2 {
        body.push((v[0] * 40 + v[1]) as u8);
    }
    for &n in v.iter().skip(2) {
        let mut stack = vec![(n & 0x7F) as u8];
        let mut n = n >> 7;
        while n > 0 {
            stack.push(0x80 | (n & 0x7F) as u8);
            n >>= 7;
        }
        body.extend(stack.iter().rev());
    }
    tlv(0x06, &body)
}

/// One PDU: GET (0xA0), GETNEXT (0xA1) or GETBULK (0xA5, with max repetitions).
pub fn request(community: &str, pdu: u8, id: i32, oids: &[Oid], max_rep: i64) -> Vec<u8> {
    tlv(0x30, &[int(1), tlv(0x04, community.as_bytes()), pdu_bytes(pdu, id, oids, max_rep)].concat())
}

/// A PDU (GET 0xA0, GETNEXT 0xA1, GETBULK 0xA5 with max repetitions).
pub(crate) fn pdu_bytes(pdu: u8, id: i32, oids: &[Oid], max_rep: i64) -> Vec<u8> {
    let binds: Vec<u8> = oids.iter().flat_map(|o| tlv(0x30, &[oid(o), vec![0x05, 0]].concat())).collect();
    let (a, b) = if pdu == 0xA5 { (0, max_rep) } else { (0, 0) };
    tlv(pdu, &[int(i64::from(id)), int(a), int(b), tlv(0x30, &binds)].concat())
}

pub(crate) struct Reader<'a> {
    pub(crate) b: &'a [u8],
    pub(crate) pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn next(&mut self) -> Option<(u8, &'a [u8])> {
        let tag = *self.b.get(self.pos)?;
        let mut len = usize::from(*self.b.get(self.pos + 1)?);
        let mut p = self.pos + 2;
        if len & 0x80 != 0 {
            let n = len & 0x7F;
            if n > 4 {
                return None;
            }
            len = 0;
            for i in 0..n {
                len = (len << 8) | usize::from(*self.b.get(p + i)?);
            }
            p += n;
        }
        let body = self.b.get(p..p + len)?;
        self.pos = p + len;
        Some((tag, body))
    }
}

pub(crate) fn read_int(b: &[u8]) -> i64 {
    let mut v: i64 = if b.first().is_some_and(|x| x & 0x80 != 0) { -1 } else { 0 };
    for x in b.iter().take(8) {
        v = (v << 8) | i64::from(*x);
    }
    v
}

fn read_uint(b: &[u8]) -> u64 {
    b.iter().take(9).fold(0u64, |v, x| (v << 8) | u64::from(*x))
}

fn read_oid(b: &[u8]) -> Oid {
    let mut v = Vec::new();
    if let Some(first) = b.first() {
        v.push(u32::from(*first / 40));
        v.push(u32::from(*first % 40));
    }
    let mut n: u32 = 0;
    for x in b.iter().skip(1) {
        n = (n << 7) | u32::from(x & 0x7F);
        if x & 0x80 == 0 {
            v.push(n);
            n = 0;
        }
    }
    Oid(v)
}

fn value(tag: u8, b: &[u8]) -> Value {
    match tag {
        0x02 => Value::Int(read_int(b)),
        0x04 | 0x44 => Value::Bytes(b.to_vec()),
        0x05 => Value::Null,
        0x06 => Value::Oid(read_oid(b)),
        0x40 if b.len() == 4 => Value::Ip(Ipv4Addr::new(b[0], b[1], b[2], b[3])),
        0x41 => Value::Counter(read_uint(b)),
        0x46 => Value::Counter64(read_uint(b)),
        0x42 => Value::Gauge(read_uint(b) as u32),
        0x43 => Value::Ticks(read_uint(b) as u32),
        0x80 => Value::NoSuchObject,
        0x81 => Value::NoSuchInstance,
        0x82 => Value::EndOfView,
        _ => Value::Missing,
    }
}

/// A response: (request id, error status, bindings).
pub type Response = (i32, i64, Vec<(Oid, Value)>);

/// Reads a response PDU.
pub fn parse_response(b: &[u8]) -> Option<Response> {
    let (tag, msg) = Reader { b, pos: 0 }.next()?;
    if tag != 0x30 {
        return None;
    }
    let mut r = Reader { b: msg, pos: 0 };
    r.next()?; // version
    r.next()?; // community
    let (pdu, body) = r.next()?;
    if pdu != 0xA2 {
        return None;
    }
    parse_pdu(body)
}

/// The inside of a response or report PDU: (request id, error status, bindings).
pub(crate) fn parse_pdu(body: &[u8]) -> Option<Response> {
    let mut p = Reader { b: body, pos: 0 };
    let id = read_int(p.next()?.1) as i32;
    let status = read_int(p.next()?.1);
    p.next()?;
    let (_, binds) = p.next()?;
    let mut list = Reader { b: binds, pos: 0 };
    let mut out = Vec::new();
    while let Some((_, bind)) = list.next() {
        let mut v = Reader { b: bind, pos: 0 };
        let (_, o) = v.next()?;
        let (t, val) = v.next()?;
        out.push((read_oid(o), value(t, val)));
    }
    Some((id, status, out))
}

// ── Client ──

/// A read-only SNMP v2c agent (a router, a switch).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub ip: Ipv4Addr,
    #[serde(default = "default_port")]
    pub port: u16,
    pub community: String,
}

fn default_port() -> u16 {
    161
}

pub struct Session {
    sock: UdpSocket,
    agent: SocketAddr,
    community: String,
    timeout: Duration,
    retries: u32,
    v3: Option<crate::snmpv3::V3Session>,
}

fn err(m: &str) -> io::Error {
    io::Error::other(m.to_string())
}

impl Session {
    pub async fn open(agent: &Agent, timeout: Duration) -> io::Result<Session> {
        let sock = UdpSocket::bind("0.0.0.0:0").await?;
        Ok(Session { sock, agent: SocketAddr::from((agent.ip, agent.port)), community: agent.community.clone(), timeout, retries: 2, v3: None })
    }

    /// An SNMP v3 session (user-based security): finds the agent's engine first.
    pub async fn open_v3(ip: Ipv4Addr, port: u16, user: &crate::snmpv3::V3, timeout: Duration) -> io::Result<Session> {
        let sock = UdpSocket::bind("0.0.0.0:0").await?;
        let agent = SocketAddr::from((ip, port));
        let v3 = crate::snmpv3::V3Session::discover(&sock, agent, user, timeout).await?;
        Ok(Session { sock, agent, community: String::new(), timeout, retries: 2, v3: Some(v3) })
    }

    async fn ask(&self, pdu: u8, oids: &[Oid], max_rep: i64) -> io::Result<Vec<(Oid, Value)>> {
        if let Some(v3) = &self.v3 {
            return v3.ask(&self.sock, self.agent, pdu, oids, max_rep, self.timeout, self.retries).await;
        }
        let id = rand::random::<i32>() & 0x7FFF_FFFF;
        let req = request(&self.community, pdu, id, oids, max_rep);
        let mut buf = vec![0u8; 65_535];
        for _ in 0..=self.retries {
            self.sock.send_to(&req, self.agent).await?;
            let deadline = tokio::time::Instant::now() + self.timeout;
            while let Ok(r) = tokio::time::timeout_at(deadline, self.sock.recv_from(&mut buf)).await {
                let (n, from) = r?;
                if from != self.agent {
                    continue;
                }
                if let Some((rid, status, binds)) = parse_response(&buf[..n]) {
                    if rid != id {
                        continue;
                    }
                    if status != 0 {
                        return Err(err(&format!("the agent answered with error {status}")));
                    }
                    return Ok(binds);
                }
            }
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, "no snmp answer (address, community or firewall?)"))
    }

    pub async fn get(&self, oids: &[Oid]) -> io::Result<Vec<(Oid, Value)>> {
        self.ask(0xA0, oids, 0).await
    }

    /// One GETBULK step: up to `reps` cells after `from` (read-only).
    pub async fn bulk(&self, from: &Oid, reps: i64) -> io::Result<Vec<(Oid, Value)>> {
        self.ask(0xA5, std::slice::from_ref(from), reps).await
    }

    /// Every cell under `base` (at most `limit`).
    pub async fn walk(&self, base: &Oid, limit: usize) -> io::Result<Vec<(Oid, Value)>> {
        let mut out = Vec::new();
        let mut cur = base.clone();
        while out.len() < limit {
            let binds = self.ask(0xA5, std::slice::from_ref(&cur), 25).await?;
            if binds.is_empty() {
                break;
            }
            let mut done = false;
            for (o, v) in binds {
                if !o.starts_with(base) || v == Value::EndOfView || o <= cur {
                    done = true;
                    break;
                }
                cur = o.clone();
                out.push((o, v));
            }
            if done {
                break;
            }
        }
        Ok(out)
    }
}

// ── What is read ──

pub mod oids {
    pub const SYS_DESCR: &str = "1.3.6.1.2.1.1.1.0";
    pub const SYS_NAME: &str = "1.3.6.1.2.1.1.5.0";
    pub const IF_NAME: &str = "1.3.6.1.2.1.31.1.1.1.1";
    pub const IF_DESCR: &str = "1.3.6.1.2.1.2.2.1.2";
    pub const IF_TYPE: &str = "1.3.6.1.2.1.2.2.1.3"; // IANAifType: 6 ethernetCsmacd, 117 gigabitEthernet
    pub const ARP: &str = "1.3.6.1.2.1.4.22.1.2"; // ipNetToMediaPhysAddress.ifIndex.a.b.c.d
    pub const BRIDGE_PORT_IF: &str = "1.3.6.1.2.1.17.1.4.1.2"; // dot1dBasePortIfIndex
    pub const FDB_Q: &str = "1.3.6.1.2.1.17.7.1.2.2.1.2"; // dot1qTpFdbPort.vlan.mac
    pub const FDB: &str = "1.3.6.1.2.1.17.4.3.1.2"; // dot1dTpFdbPort.mac
    pub const LLDP_LOC_PORT: &str = "1.0.8802.1.1.2.1.3.7.1.3"; // lldpLocPortId.localPort
    pub const LLDP_REM_PORT_ID: &str = "1.0.8802.1.1.2.1.4.1.1.7"; // .time.localPort.index
    pub const LLDP_REM_PORT_DESC: &str = "1.0.8802.1.1.2.1.4.1.1.8";
    pub const LLDP_REM_SYS_NAME: &str = "1.0.8802.1.1.2.1.4.1.1.9";
    pub const MROUTE_OCTETS: &str = "1.3.6.1.2.1.83.1.1.2.1.10"; // ipMRouteOctets.group.source.mask
    pub const MROUTE_PKTS: &str = "1.3.6.1.2.1.83.1.1.2.1.8";
    // System and chassis (ENTITY-MIB)
    pub const SYS_UPTIME: &str = "1.3.6.1.2.1.1.3.0";
    pub const SYS_CONTACT: &str = "1.3.6.1.2.1.1.4.0";
    pub const SYS_LOCATION: &str = "1.3.6.1.2.1.1.6.0";
    pub const ENT_CLASS: &str = "1.3.6.1.2.1.47.1.1.1.1.5"; // 3 = chassis
    pub const ENT_FIRMWARE: &str = "1.3.6.1.2.1.47.1.1.1.1.9";
    pub const ENT_SOFTWARE: &str = "1.3.6.1.2.1.47.1.1.1.1.10";
    pub const ENT_SERIAL: &str = "1.3.6.1.2.1.47.1.1.1.1.11";
    pub const ENT_MODEL: &str = "1.3.6.1.2.1.47.1.1.1.1.13";
    // Interfaces (IF-MIB)
    pub const IF_ADMIN: &str = "1.3.6.1.2.1.2.2.1.7";
    pub const IF_OPER: &str = "1.3.6.1.2.1.2.2.1.8";
    pub const IF_IN_ERRORS: &str = "1.3.6.1.2.1.2.2.1.14";
    pub const IF_OUT_ERRORS: &str = "1.3.6.1.2.1.2.2.1.20";
    pub const IF_HC_IN: &str = "1.3.6.1.2.1.31.1.1.1.6";
    pub const IF_HC_OUT: &str = "1.3.6.1.2.1.31.1.1.1.10";
    pub const IF_HIGH_SPEED: &str = "1.3.6.1.2.1.31.1.1.1.15"; // Mb/s
    pub const IF_ALIAS: &str = "1.3.6.1.2.1.31.1.1.1.18";
    // VLANs (Q-BRIDGE-MIB)
    pub const Q_PVID: &str = "1.3.6.1.2.1.17.7.1.4.5.1.1"; // dot1qPvid.bridgePort
    pub const Q_VLAN_NAME: &str = "1.3.6.1.2.1.17.7.1.4.3.1.1"; // dot1qVlanStaticName.vlan
    pub const Q_VLAN_EGRESS: &str = "1.3.6.1.2.1.17.7.1.4.3.1.2"; // dot1qVlanStaticEgressPorts.vlan
    pub const Q_VLAN_UNTAGGED: &str = "1.3.6.1.2.1.17.7.1.4.3.1.4"; // dot1qVlanStaticUntaggedPorts.vlan
    // Cisco (IOS, IOS-XE, NX-OS): access VLAN, trunks and VLAN names
    pub const CISCO_VM_VLAN: &str = "1.3.6.1.4.1.9.9.68.1.2.2.1.2"; // vmVlan.ifIndex
    pub const CISCO_TRUNK_ENABLED: &str = "1.3.6.1.4.1.9.9.46.1.6.1.1.4"; // vlanTrunkPortVlansEnabled (0-1023)
    pub const CISCO_TRUNK_NATIVE: &str = "1.3.6.1.4.1.9.9.46.1.6.1.1.5"; // vlanTrunkPortNativeVlan
    pub const CISCO_TRUNK_STATUS: &str = "1.3.6.1.4.1.9.9.46.1.6.1.1.14"; // vlanTrunkPortDynamicStatus: 1 trunking
    pub const CISCO_TRUNK_ENABLED_2K: &str = "1.3.6.1.4.1.9.9.46.1.6.1.1.17"; // 1024-2047
    pub const CISCO_TRUNK_ENABLED_3K: &str = "1.3.6.1.4.1.9.9.46.1.6.1.1.18"; // 2048-3071
    pub const CISCO_TRUNK_ENABLED_4K: &str = "1.3.6.1.4.1.9.9.46.1.6.1.1.19"; // 3072-4095
    pub const CISCO_VTP_VLAN_NAME: &str = "1.3.6.1.4.1.9.9.46.1.3.1.1.4.1"; // vtpVlanName.1.vlan
}

/// The VLANs set in a Cisco VLAN bitmap whose first bit is VLAN `first`.
pub fn vlan_bits(bytes: &[u8], first: u16) -> Vec<u16> {
    let mut out = Vec::new();
    for (i, b) in bytes.iter().enumerate() {
        for bit in 0..8 {
            if b & (0x80 >> bit) != 0 {
                let v = first as usize + i * 8 + bit;
                if (1..=4094).contains(&v) {
                    out.push(v as u16);
                }
            }
        }
    }
    out
}

/// What a switch says about itself.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct System {
    pub name: String,
    pub description: String,
    pub uptime_secs: u64,
    pub contact: String,
    pub location: String,
    pub model: String,
    pub serial: String,
    pub firmware: String,
}

/// Reads the switch's identity: system group, and the chassis entry of ENTITY-MIB when present.
pub async fn system(s: &Session) -> io::Result<System> {
    let v = s
        .get(&[Oid::parse(oids::SYS_NAME), Oid::parse(oids::SYS_DESCR), Oid::parse(oids::SYS_UPTIME), Oid::parse(oids::SYS_CONTACT), Oid::parse(oids::SYS_LOCATION)])
        .await?;
    let text = |i: usize| v.get(i).and_then(|(_, v)| v.as_text()).unwrap_or_default();
    let mut out = System {
        name: text(0),
        description: text(1).lines().next().unwrap_or_default().to_string(),
        uptime_secs: v.get(2).and_then(|(_, v)| v.as_u64()).unwrap_or(0) / 100,
        contact: text(3),
        location: text(4),
        ..System::default()
    };
    // The first chassis (class 3) of the entity table.
    let class = Oid::parse(oids::ENT_CLASS);
    let chassis = s.walk(&class, 2000).await.unwrap_or_default().into_iter().find(|(_, v)| v.as_u64() == Some(3)).and_then(|(o, _)| o.suffix(&class).first().copied());
    if let Some(i) = chassis {
        let at = |base: &str| Oid::parse(base).child(i);
        if let Ok(e) = s.get(&[at(oids::ENT_MODEL), at(oids::ENT_SERIAL), at(oids::ENT_SOFTWARE), at(oids::ENT_FIRMWARE)]).await {
            let t = |k: usize| e.get(k).and_then(|(_, v)| v.as_text()).unwrap_or_default();
            out.model = t(0);
            out.serial = t(1);
            out.firmware = if t(2).is_empty() { t(3) } else { t(2) };
        }
    }
    Ok(out)
}

/// One interface at one moment: state, speed and counters.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IfStat {
    pub if_index: u32,
    pub name: String,
    pub alias: String,
    /// Administratively enabled.
    pub admin_up: bool,
    /// Link up.
    pub up: bool,
    pub speed_mbps: u64,
    pub in_octets: u64,
    pub out_octets: u64,
    pub in_errors: u64,
    pub out_errors: u64,
}

/// Every interface's state and counters (64-bit counters).
pub async fn if_stats(s: &Session) -> io::Result<Vec<IfStat>> {
    let mut by: std::collections::BTreeMap<u32, IfStat> = std::collections::BTreeMap::new();
    for (col, base) in [
        (0, oids::IF_NAME),
        (1, oids::IF_ALIAS),
        (2, oids::IF_ADMIN),
        (3, oids::IF_OPER),
        (4, oids::IF_HIGH_SPEED),
        (5, oids::IF_HC_IN),
        (6, oids::IF_HC_OUT),
        (7, oids::IF_IN_ERRORS),
        (8, oids::IF_OUT_ERRORS),
    ] {
        let base = Oid::parse(base);
        let rows = s.walk(&base, 20_000).await;
        // The name column must answer; the others may be missing on small switches.
        let rows = if col == 0 { rows? } else { rows.unwrap_or_default() };
        for (o, v) in rows {
            let Some(i) = o.suffix(&base).first().copied() else { continue };
            let e = by.entry(i).or_insert_with(|| IfStat { if_index: i, ..IfStat::default() });
            let n = v.as_u64().unwrap_or(0);
            match col {
                0 => e.name = v.as_text().unwrap_or_default(),
                1 => e.alias = v.as_text().unwrap_or_default(),
                2 => e.admin_up = n == 1,
                3 => e.up = n == 1,
                4 => e.speed_mbps = n,
                5 => e.in_octets = n,
                6 => e.out_octets = n,
                7 => e.in_errors = n,
                _ => e.out_errors = n,
            }
        }
    }
    Ok(by.into_values().collect())
}

/// The VLAN setup of a switch, by interface.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VlanSetup {
    /// Native (untagged ingress) VLAN by ifIndex.
    pub pvid: HashMap<u32, u16>,
    /// VLANs carried untagged / tagged by ifIndex.
    pub untagged: HashMap<u32, Vec<u16>>,
    pub tagged: HashMap<u32, Vec<u16>>,
    /// Names of the VLANs declared on the switch.
    pub names: HashMap<u16, String>,
}

/// The bridge ports set in a Q-BRIDGE port list (first byte = ports 1..8, high bit first).
pub fn port_list(bytes: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    for (i, b) in bytes.iter().enumerate() {
        for bit in 0..8 {
            if b & (0x80 >> bit) != 0 {
                out.push((i * 8 + bit + 1) as u32);
            }
        }
    }
    out
}

/// Reads PVIDs and the static VLAN memberships (Q-BRIDGE-MIB).
pub async fn vlan_setup(s: &Session) -> io::Result<VlanSetup> {
    let bp = Oid::parse(oids::BRIDGE_PORT_IF);
    let to_if: HashMap<u32, u32> =
        s.walk(&bp, 20_000).await.unwrap_or_default().into_iter().filter_map(|(o, v)| Some((*o.suffix(&bp).first()?, v.as_u64()? as u32))).collect();
    let ifx = |port: u32| to_if.get(&port).copied().unwrap_or(port);
    let mut out = VlanSetup::default();
    let pv = Oid::parse(oids::Q_PVID);
    for (o, v) in s.walk(&pv, 20_000).await? {
        if let (Some(p), Some(vlan)) = (o.suffix(&pv).first(), v.as_u64()) {
            out.pvid.insert(ifx(*p), vlan as u16);
        }
    }
    let name = Oid::parse(oids::Q_VLAN_NAME);
    for (o, v) in s.walk(&name, 5_000).await.unwrap_or_default() {
        if let Some(vlan) = o.suffix(&name).first() {
            out.names.insert(*vlan as u16, v.as_text().unwrap_or_default());
        }
    }
    let mut untagged_of: HashMap<u16, Vec<u32>> = HashMap::new();
    let un = Oid::parse(oids::Q_VLAN_UNTAGGED);
    for (o, v) in s.walk(&un, 5_000).await.unwrap_or_default() {
        if let (Some(vlan), Value::Bytes(b)) = (o.suffix(&un).first(), &v) {
            untagged_of.insert(*vlan as u16, port_list(b).into_iter().map(ifx).collect());
        }
    }
    let eg = Oid::parse(oids::Q_VLAN_EGRESS);
    for (o, v) in s.walk(&eg, 5_000).await.unwrap_or_default() {
        let (Some(vlan), Value::Bytes(b)) = (o.suffix(&eg).first(), &v) else { continue };
        let vlan = *vlan as u16;
        let untagged = untagged_of.get(&vlan).cloned().unwrap_or_default();
        for i in port_list(b).into_iter().map(ifx) {
            let list = if untagged.contains(&i) { &mut out.untagged } else { &mut out.tagged };
            list.entry(i).or_default().push(vlan);
        }
    }
    cisco_vlans(s, &mut out).await;
    for l in out.untagged.values_mut().chain(out.tagged.values_mut()) {
        l.sort_unstable();
        l.dedup();
    }
    Ok(out)
}

/// Cisco switches (notably Nexus) describe access ports and trunks in their
/// own MIBs, more reliably than Q-BRIDGE: when they answer, they win.
async fn cisco_vlans(s: &Session, out: &mut VlanSetup) {
    let walk = |base: &'static str| async move {
        let b = Oid::parse(base);
        s.walk(&b, 20_000).await.unwrap_or_default().into_iter().filter_map(move |(o, v)| Some((*o.suffix(&b).first()?, v))).collect::<Vec<(u32, Value)>>()
    };
    let names = Oid::parse(oids::CISCO_VTP_VLAN_NAME);
    for (o, v) in s.walk(&names, 5_000).await.unwrap_or_default() {
        if let Some(vlan) = o.suffix(&names).first() {
            out.names.entry(*vlan as u16).or_insert_with(|| v.as_text().unwrap_or_default());
        }
    }
    // Access ports.
    for (i, v) in walk(oids::CISCO_VM_VLAN).await {
        if let Some(vlan) = v.as_u64().map(|x| x as u16).filter(|x| *x > 0) {
            out.pvid.insert(i, vlan);
            out.untagged.insert(i, vec![vlan]);
            out.tagged.remove(&i);
        }
    }
    // Trunks: allowed VLANs (only those that exist on the switch), native VLAN untagged.
    let trunking: Vec<u32> = walk(oids::CISCO_TRUNK_STATUS).await.into_iter().filter(|(_, v)| v.as_u64() == Some(1)).map(|(i, _)| i).collect();
    if trunking.is_empty() {
        return;
    }
    let natives: HashMap<u32, u16> = walk(oids::CISCO_TRUNK_NATIVE).await.into_iter().filter_map(|(i, v)| Some((i, v.as_u64()? as u16))).collect();
    let mut allowed: HashMap<u32, Vec<u16>> = HashMap::new();
    for (base, first) in [(oids::CISCO_TRUNK_ENABLED, 0u16), (oids::CISCO_TRUNK_ENABLED_2K, 1024), (oids::CISCO_TRUNK_ENABLED_3K, 2048), (oids::CISCO_TRUNK_ENABLED_4K, 3072)] {
        for (i, v) in walk(base).await {
            if let Value::Bytes(b) = v {
                allowed.entry(i).or_default().extend(vlan_bits(&b, first));
            }
        }
    }
    let exists = |v: &u16| out.names.is_empty() || out.names.contains_key(v);
    for i in trunking {
        let native = natives.get(&i).copied().unwrap_or(1);
        let tagged: Vec<u16> = allowed.get(&i).map(|l| l.iter().copied().filter(|v| *v != native && exists(v)).collect()).unwrap_or_default();
        out.pvid.insert(i, native);
        out.untagged.insert(i, vec![native]);
        out.tagged.insert(i, tagged);
    }
}

/// Who the agent is.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub name: String,
    pub description: String,
}

pub async fn identity(s: &Session) -> io::Result<Identity> {
    let v = s.get(&[Oid::parse(oids::SYS_NAME), Oid::parse(oids::SYS_DESCR)]).await?;
    let text = |i: usize| v.get(i).and_then(|(_, v)| v.as_text()).unwrap_or_default();
    Ok(Identity { name: text(0), description: text(1).lines().next().unwrap_or_default().to_string() })
}

fn ip_from(idx: &[u32]) -> Option<Ipv4Addr> {
    let n: Vec<u8> = idx.iter().map(|x| u8::try_from(*x).ok()).collect::<Option<_>>()?;
    (n.len() == 4).then(|| Ipv4Addr::new(n[0], n[1], n[2], n[3]))
}

/// The agent's ARP table (routers know every machine of their subnets).
pub async fn arp(s: &Session) -> io::Result<HashMap<Ipv4Addr, Mac>> {
    let base = Oid::parse(oids::ARP);
    let mut out = HashMap::new();
    for (o, v) in s.walk(&base, 100_000).await? {
        let idx = o.suffix(&base);
        if let (Some(ip), Some(m)) = (idx.get(1..).and_then(ip_from), v.as_mac()) {
            out.insert(ip, m);
        }
    }
    Ok(out)
}

/// Interface names by ifIndex (`Gi1/0/12`).
pub async fn interfaces(s: &Session) -> io::Result<HashMap<u32, String>> {
    let mut out = HashMap::new();
    for base in [oids::IF_DESCR, oids::IF_NAME] {
        let base = Oid::parse(base);
        for (o, v) in s.walk(&base, 20_000).await.unwrap_or_default() {
            if let (Some(i), Some(n)) = (o.suffix(&base).first(), v.as_text()) {
                if !n.is_empty() {
                    out.insert(*i, n);
                }
            }
        }
    }
    Ok(out)
}

/// An interface as the switch lists it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IfInfo {
    pub if_index: u32,
    /// ifName (`Gi1/0/12`), else ifDescr.
    pub name: String,
    /// IANA ifType; 0 when the switch does not say.
    pub if_type: u32,
    pub speed_mbps: u64,
}

/// Every interface: name, type and speed (to recognise the faceplate).
pub async fn if_list(s: &Session) -> io::Result<Vec<IfInfo>> {
    let mut by: std::collections::BTreeMap<u32, IfInfo> = std::collections::BTreeMap::new();
    for (col, base) in [(0, oids::IF_DESCR), (1, oids::IF_NAME), (2, oids::IF_TYPE), (3, oids::IF_HIGH_SPEED)] {
        let base = Oid::parse(base);
        let rows = s.walk(&base, 20_000).await;
        // The interface table must answer; the other columns may be missing.
        let rows = if col == 0 { rows? } else { rows.unwrap_or_default() };
        for (o, v) in rows {
            let Some(i) = o.suffix(&base).first().copied() else { continue };
            let e = by.entry(i).or_insert_with(|| IfInfo { if_index: i, ..IfInfo::default() });
            match col {
                0 | 1 => {
                    let n = v.as_text().unwrap_or_default();
                    if !n.is_empty() {
                        e.name = n;
                    }
                }
                2 => e.if_type = v.as_u64().unwrap_or(0) as u32,
                _ => e.speed_mbps = v.as_u64().unwrap_or(0),
            }
        }
    }
    Ok(by.into_values().filter(|i| !i.name.is_empty()).collect())
}

/// A MAC address learnt on a switch port.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Learnt {
    pub mac: String,
    pub vlan: Option<u16>,
    pub if_index: u32,
}

/// The switch's MAC address table (802.1Q, else plain bridge).
pub async fn mac_table(s: &Session) -> io::Result<Vec<Learnt>> {
    let bp = Oid::parse(oids::BRIDGE_PORT_IF);
    let to_if: HashMap<u32, u32> =
        s.walk(&bp, 20_000).await.unwrap_or_default().into_iter().filter_map(|(o, v)| Some((*o.suffix(&bp).first()?, v.as_u64()? as u32))).collect();
    let mut out = Vec::new();
    let q = Oid::parse(oids::FDB_Q);
    let rows = s.walk(&q, 200_000).await.unwrap_or_default();
    let (rows, base, with_vlan) = if rows.is_empty() {
        let b = Oid::parse(oids::FDB);
        (s.walk(&b, 200_000).await?, b, false)
    } else {
        (rows, q, true)
    };
    for (o, v) in rows {
        let idx = o.suffix(&base);
        let (vlan, m) = if with_vlan { (idx.first().map(|v| *v as u16), idx.get(1..)) } else { (None, Some(idx)) };
        let Some(m) = m.filter(|m| m.len() == 6) else { continue };
        let Some(port) = v.as_u64().map(|p| p as u32) else { continue };
        if port == 0 {
            continue;
        }
        let mac: Mac = [m[0] as u8, m[1] as u8, m[2] as u8, m[3] as u8, m[4] as u8, m[5] as u8];
        out.push(Learnt { mac: mac::format(&mac), vlan, if_index: to_if.get(&port).copied().unwrap_or(port) });
    }
    Ok(out)
}

/// A neighbour seen by LLDP on a local port.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Neighbour {
    pub local_port: String,
    pub name: String,
    pub port: String,
}

pub async fn lldp(s: &Session) -> io::Result<Vec<Neighbour>> {
    let loc = Oid::parse(oids::LLDP_LOC_PORT);
    let local: HashMap<u32, String> =
        s.walk(&loc, 10_000).await.unwrap_or_default().into_iter().filter_map(|(o, v)| Some((*o.suffix(&loc).first()?, v.as_text()?))).collect();
    let mut rows: HashMap<(u32, u32), Neighbour> = HashMap::new();
    for (col, base) in [(9, oids::LLDP_REM_SYS_NAME), (7, oids::LLDP_REM_PORT_ID), (8, oids::LLDP_REM_PORT_DESC)] {
        let base = Oid::parse(base);
        for (o, v) in s.walk(&base, 10_000).await.unwrap_or_default() {
            let idx = o.suffix(&base);
            let (Some(lp), Some(i)) = (idx.get(1), idx.get(2)) else { continue };
            let text = v.as_text().unwrap_or_default();
            let n = rows.entry((*lp, *i)).or_insert_with(|| Neighbour { local_port: local.get(lp).cloned().unwrap_or_else(|| lp.to_string()), name: String::new(), port: String::new() });
            match col {
                9 => n.name = text,
                7 if n.port.is_empty() => n.port = text,
                8 if !text.is_empty() => n.port = text,
                _ => {}
            }
        }
    }
    let mut v: Vec<Neighbour> = rows.into_values().collect();
    v.sort_by(|a, b| a.local_port.cmp(&b.local_port));
    Ok(v)
}

/// A route of a multicast router: a stream and its source, counters since it started.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub group: Ipv4Addr,
    pub source: Ipv4Addr,
    pub octets: u64,
    pub packets: u64,
}

pub async fn mroutes(s: &Session) -> io::Result<Vec<Route>> {
    let (ob, pb) = (Oid::parse(oids::MROUTE_OCTETS), Oid::parse(oids::MROUTE_PKTS));
    let pkts: HashMap<Vec<u32>, u64> = s.walk(&pb, 50_000).await.unwrap_or_default().into_iter().filter_map(|(o, v)| Some((o.suffix(&pb).to_vec(), v.as_u64()?))).collect();
    let mut out = Vec::new();
    for (o, v) in s.walk(&ob, 50_000).await? {
        let idx = o.suffix(&ob);
        let (Some(group), Some(source)) = (idx.get(0..4).and_then(ip_from), idx.get(4..8).and_then(ip_from)) else { continue };
        if source.is_unspecified() || !group.is_multicast() {
            continue; // (*, G) entries have no source
        }
        out.push(Route { group, source, octets: v.as_u64().unwrap_or(0), packets: pkts.get(idx).copied().unwrap_or(0) });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_net_snmp() {
        // snmpget -v2c -c public x 1.3.6.1.2.1.1.5.0, request id 1
        let r = request("public", 0xA0, 1, &[Oid::parse(oids::SYS_NAME)], 0);
        let want: Vec<u8> = vec![
            0x30, 0x26, 0x02, 0x01, 0x01, 0x04, 0x06, b'p', b'u', b'b', b'l', b'i', b'c', 0xA0, 0x19, 0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30,
            0x0E, 0x30, 0x0C, 0x06, 0x08, 0x2B, 0x06, 0x01, 0x02, 0x01, 0x01, 0x05, 0x00, 0x05, 0x00,
        ];
        assert_eq!(r, want);
        assert_eq!(read_oid(&oid(&Oid::parse("1.3.6.1.4.1.9.9.300"))[2..]), Oid::parse("1.3.6.1.4.1.9.9.300"));
        assert_eq!(read_int(&int(-129)[2..]), -129);
        assert_eq!(read_int(&int(128)[2..]), 128);
        assert_eq!(int(0), vec![2, 1, 0]);
    }

    #[test]
    fn cisco_vlan_bitmaps() {
        // VLAN 0 is the first bit: 0b0110_0000 = VLANs 1 and 2.
        assert_eq!(vlan_bits(&[0b0110_0000, 0b1000_0000], 0), vec![1, 2, 8]);
        assert_eq!(vlan_bits(&[0b1000_0000], 1024), vec![1024]);
        assert_eq!(vlan_bits(&[0xFF; 128], 3072).last(), Some(&4094));
    }

    #[test]
    fn port_lists() {
        assert_eq!(port_list(&[0b1000_0001, 0b0100_0000]), vec![1, 8, 10]);
        assert!(port_list(&[0, 0]).is_empty());
    }

    #[test]
    fn reads_a_response() {
        let binds = tlv(0x30, &[oid(&Oid::parse(oids::SYS_NAME)), tlv(0x04, b"sw-regie-01")].concat());
        let pdu = tlv(0xA2, &[int(42), int(0), int(0), tlv(0x30, &binds)].concat());
        let msg = tlv(0x30, &[int(1), tlv(0x04, b"public"), pdu].concat());
        let (id, status, v) = parse_response(&msg).unwrap();
        assert_eq!((id, status), (42, 0));
        assert_eq!(v[0].0, Oid::parse(oids::SYS_NAME));
        assert_eq!(v[0].1.as_text().as_deref(), Some("sw-regie-01"));
    }

    /// A tiny agent on loopback answering GETBULK on a two-row table.
    #[tokio::test]
    async fn walks_a_table() {
        let agent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = agent.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            let table = [(Oid::parse("1.3.6.1.2.1.4.22.1.2.3.10.0.0.1"), [0, 0x11, 0x32, 1, 2, 3]), (Oid::parse("1.3.6.1.2.1.4.22.1.2.3.10.0.0.9"), [0, 0x11, 0x32, 1, 2, 9])];
            loop {
                let (n, from) = agent.recv_from(&mut buf).await.unwrap();
                // Request id: the first INTEGER inside the PDU.
                let mut r = Reader { b: &buf[..n], pos: 0 };
                let (_, msg) = r.next().unwrap();
                let mut m = Reader { b: msg, pos: 0 };
                m.next();
                m.next();
                let (_, body) = m.next().unwrap();
                let mut p = Reader { b: body, pos: 0 };
                let id = read_int(p.next().unwrap().1);
                p.next();
                p.next();
                let (_, vb) = p.next().unwrap();
                let (_, first) = Reader { b: vb, pos: 0 }.next().unwrap();
                let asked = read_oid(Reader { b: first, pos: 0 }.next().unwrap().1);
                let mut binds = Vec::new();
                for (o, mac) in table.iter().filter(|(o, _)| *o > asked) {
                    binds.extend(tlv(0x30, &[oid(o), tlv(0x04, mac)].concat()));
                }
                binds.extend(tlv(0x30, &[oid(&Oid::parse("1.3.6.1.2.1.4.23")), vec![0x82, 0]].concat()));
                let pdu = tlv(0xA2, &[int(id), int(0), int(0), tlv(0x30, &binds)].concat());
                let _ = agent.send_to(&tlv(0x30, &[int(1), tlv(0x04, b"public"), pdu].concat()), from).await;
            }
        });
        let s = Session::open(&Agent { ip: Ipv4Addr::LOCALHOST, port, community: "public".into() }, Duration::from_millis(500)).await.unwrap();
        let t = arp(&s).await.unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[&Ipv4Addr::new(10, 0, 0, 9)], [0, 0x11, 0x32, 1, 2, 9]);
    }
}
