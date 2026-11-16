//! Multicast streams.
//!
//! - SAP announcements (239.255.255.255 and 224.2.127.254, port 9875) are
//!   listened to all the time: they give the streams' names and formats (SDP).
//! - PTP (224.0.1.129, ports 319/320) is listened to all the time: domain and
//!   grandmaster, every clock that announces itself (`ptp`), and the devices
//!   that ask for the delay (Delay_Req).
//! - Flooding: when turned on, every 5 min, the interface is sampled for 1 s
//!   for media packets of groups not joined (`flood`; receive only, Linux).
//! - Routers' multicast routing tables, read by SNMP (see `snmp`), give the
//!   other streams with their sources and rates: nothing is joined.
//! - Nothing else is joined on its own: joining a group makes the network bring
//!   the stream here. A single stream can be checked on demand, briefly, and the
//!   check stops at once if the stream is heavier than allowed.

use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;

use crate::mformat::Format;
use crate::ptp;


pub const SAP_GROUPS: [Ipv4Addr; 2] = [Ipv4Addr::new(239, 255, 255, 255), Ipv4Addr::new(224, 2, 127, 254)];
pub const SAP_PORT: u16 = 9875;
pub const PTP_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 1, 129);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Video,
    Audio,
    /// Ancillary data (ST 2110-40).
    Data,
    MpegTs,
    /// RTP whose content is not known.
    Rtp,
    Ptp,
    Sap,
    Other,
}

/// A stream: a group and port, sent by one source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    pub group: Ipv4Addr,
    pub port: u16,
    /// `None` for a stream only announced (SAP) and not received yet.
    pub source: Option<Ipv4Addr>,
    pub kind: Kind,
    /// RTP encoding from the SDP (`raw`, `L24`, `smpte291`, `jxsv`, `MP2T`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtp_type: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ptp_domain: Option<u8>,
    /// Grandmaster clock identity (PTP announce).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ptp_grandmaster: Option<String>,
    /// Bits per second, packets per second (last measure).
    pub bps: f64,
    pub pps: f64,
    pub packets: u64,
    /// Unix milliseconds.
    pub first_seen: u64,
    pub last_seen: u64,
    /// Rates measured, oldest first (at most 60).
    pub history: Vec<f32>,
    /// Announced by SAP.
    pub announced: bool,
    /// Seen in a router's multicast routing table.
    #[serde(default)]
    pub routed: bool,
    /// Format read from the SDP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    /// Bits per second the SDP implies (headers included).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub est_bps: Option<f64>,
    /// Last SAP announcement (unix ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_announced: Option<u64>,
    /// Time between the last two announcements (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sap_interval_ms: Option<u64>,
    /// The other leg of an ST 2022-7 pair named by the SDP (`a=group:DUP`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dup: Option<Leg>,
    /// Last time its packets reached this machine without being joined (unix ms; 0 = never).
    #[serde(default)]
    pub flooded: u64,
    /// The 802.1Q tag of those packets, when they carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flood_vlan: Option<u16>,
}

/// One leg of an ST 2022-7 pair.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Leg {
    pub group: Ipv4Addr,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Ipv4Addr>,
}

impl Stream {
    fn new(group: Ipv4Addr, port: u16, source: Option<Ipv4Addr>, kind: Kind, now: u64) -> Self {
        Stream {
            group,
            port,
            source,
            kind,
            encoding: None,
            name: None,
            rtp_type: None,
            ptp_domain: None,
            ptp_grandmaster: None,
            bps: 0.0,
            pps: 0.0,
            packets: 0,
            first_seen: now,
            last_seen: now,
            history: Vec::new(),
            announced: false,
            routed: false,
            format: None,
            est_bps: None,
            last_announced: None,
            sap_interval_ms: None,
            dup: None,
            flooded: 0,
            flood_vlan: None,
        }
    }

    /// Takes what an announcement says.
    fn take_sdp(&mut self, sdp: &Sdp, now: u64) {
        self.announced = true;
        self.name.clone_from(&sdp.name);
        self.encoding.clone_from(&sdp.encoding);
        if let Some(k) = sdp.kind {
            self.kind = k;
        }
        self.est_bps = sdp.format.as_ref().and_then(Format::est_bps);
        self.format.clone_from(&sdp.format);
        self.dup.clone_from(&sdp.dup);
        if let Some(last) = self.last_announced {
            if now > last + 200 {
                self.sap_interval_ms = Some(now - last);
            }
        }
        self.last_announced = Some(now);
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
struct Key {
    group: Ipv4Addr,
    port: u16,
    source: Option<Ipv4Addr>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// What the SDP of a SAP announcement says about one media.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sdp {
    pub name: Option<String>,
    pub group: Option<Ipv4Addr>,
    pub port: u16,
    pub kind: Option<Kind>,
    pub encoding: Option<String>,
    pub source: Option<Ipv4Addr>,
    pub format: Option<Format>,
    /// `a=mid`.
    pub mid: Option<String>,
    /// The other media of its `a=group:DUP` (ST 2022-7).
    pub dup: Option<Leg>,
}

/// The medias of an SDP description.
pub fn parse_sdp(text: &str) -> Vec<Sdp> {
    let mut session = Sdp::default();
    let mut medias: Vec<Sdp> = Vec::new();
    let mut origin = None;
    // Lines of the session, then of each media, for the formats.
    let mut lines: Vec<Vec<(String, String)>> = vec![Vec::new()];
    let mut dups: Vec<Vec<String>> = Vec::new();
    for line in text.lines().map(str::trim) {
        let Some((k, v)) = line.split_once('=') else { continue };
        if k == "m" {
            lines.push(Vec::new());
        }
        if let Some(l) = lines.last_mut() {
            l.push((k.to_string(), v.to_string()));
        }
        if let Some(g) = v.strip_prefix("group:DUP").filter(|_| k == "a") {
            dups.push(g.split_whitespace().map(str::to_string).collect());
        }
        let cur = medias.last_mut().unwrap_or(&mut session);
        match k {
            "s" => session.name = Some(v.trim().to_string()).filter(|s| !s.is_empty() && s != "-"),
            "o" => origin = v.split_whitespace().nth(5).and_then(|a| a.parse().ok()),
            "c" => cur.group = v.split_whitespace().nth(2).and_then(|a| a.split('/').next()).and_then(|a| a.parse().ok()),
            "m" => {
                let f: Vec<&str> = v.split_whitespace().collect();
                medias.push(Sdp {
                    port: f.get(1).and_then(|p| p.split('/').next()).and_then(|p| p.parse().ok()).unwrap_or(0),
                    kind: match f.first().copied() {
                        Some("video") => Some(Kind::Video),
                        Some("audio") => Some(Kind::Audio),
                        Some("application") | Some("data") => Some(Kind::Data),
                        _ => None,
                    },
                    ..Sdp::default()
                });
            }
            "a" => {
                if let Some(r) = v.strip_prefix("rtpmap:") {
                    let enc = r.split_whitespace().nth(1).and_then(|e| e.split('/').next()).map(str::to_string);
                    if enc.as_deref() == Some("MP2T") {
                        cur.kind = Some(Kind::MpegTs);
                    }
                    if enc.as_deref() == Some("smpte291") {
                        cur.kind = Some(Kind::Data);
                    }
                    cur.encoding = enc;
                } else if let Some(r) = v.strip_prefix("source-filter:") {
                    // incl IN IP4 <group> <source>
                    cur.source = r.split_whitespace().nth(4).and_then(|a| a.parse().ok());
                } else if let Some(r) = v.strip_prefix("mid:") {
                    cur.mid = Some(r.trim().to_string());
                }
            }
            _ => {}
        }
    }
    let session_lines = lines.first().cloned().unwrap_or_default();
    if medias.is_empty() {
        medias.push(session.clone());
        lines.push(Vec::new());
    }
    for (i, m) in medias.iter_mut().enumerate() {
        m.name = m.name.clone().or_else(|| session.name.clone());
        m.group = m.group.or(session.group);
        m.source = m.source.or(session.source).or(origin);
        let own = lines.get(i + 1).map(Vec::as_slice).unwrap_or(&[]);
        m.format = Some(Format::parse(&session_lines, own, m.encoding.as_deref()));
    }
    // ST 2022-7: the medias of one DUP group are the legs of one pair.
    let legs: Vec<(Option<String>, Leg)> = medias.iter().map(|m| (m.mid.clone(), Leg { group: m.group.unwrap_or(Ipv4Addr::UNSPECIFIED), port: m.port, source: m.source })).collect();
    for m in medias.iter_mut() {
        let Some(mid) = m.mid.clone() else { continue };
        let Some(group) = dups.iter().find(|g| g.contains(&mid)) else { continue };
        m.dup = legs.iter().find(|(other, _)| other.as_ref().is_some_and(|o| o != &mid && group.contains(o))).map(|(_, l)| l.clone());
    }
    medias.retain(|m| m.group.is_some_and(|g| g.is_multicast()) && m.port != 0);
    medias
}

/// A SAP packet: `(deletion, sdp text)`.
pub fn parse_sap(p: &[u8]) -> Option<(bool, String)> {
    let b0 = *p.first()?;
    if b0 >> 5 != 1 || b0 & 0x02 != 0 {
        return None; // version 1, not compressed
    }
    let ipv6 = b0 & 0x10 != 0;
    let delete = b0 & 0x04 != 0;
    let auth = usize::from(*p.get(1)?) * 4;
    let mut pos = 4 + if ipv6 { 16 } else { 4 } + auth;
    let rest = p.get(pos..)?;
    if !rest.starts_with(b"v=") {
        pos += rest.iter().position(|b| *b == 0)? + 1;
    }
    Some((delete, String::from_utf8_lossy(p.get(pos..)?).into_owned()))
}

/// RTP header length, payload type, if `p` looks like RTP version 2.
fn rtp(p: &[u8]) -> Option<(usize, u8)> {
    if p.len() < 12 || p[0] >> 6 != 2 {
        return None;
    }
    let mut len = 12 + 4 * usize::from(p[0] & 0x0F);
    if p[0] & 0x10 != 0 {
        let ext = p.get(len + 2..len + 4)?;
        len += 4 + 4 * usize::from(u16::from_be_bytes([ext[0], ext[1]]));
    }
    (len <= p.len()).then_some((len, p[1] & 0x7F))
}

/// What a packet looks like.
pub fn classify(p: &[u8], port: u16) -> (Kind, Option<u8>) {
    if port == 319 || port == 320 {
        return (Kind::Ptp, None);
    }
    if port == SAP_PORT {
        return (Kind::Sap, None);
    }
    if p.len() >= 188 && p.len().is_multiple_of(188) && p[0] == 0x47 {
        return (Kind::MpegTs, None);
    }
    if let Some((h, pt)) = rtp(p) {
        let kind = match pt {
            33 => Kind::MpegTs,
            _ if p.get(h) == Some(&0x47) && (p.len() - h).is_multiple_of(188) => Kind::MpegTs,
            0 | 8 | 10 | 11 | 14 => Kind::Audio,
            _ => Kind::Rtp,
        };
        return (kind, Some(pt));
    }
    (Kind::Other, None)
}

/// PTP: (domain, grandmaster identity if this is an Announce).
pub fn parse_ptp(p: &[u8]) -> Option<(u8, Option<String>)> {
    if p.len() < 34 || p[1] & 0x0F != 2 {
        return None;
    }
    let gm = (p[0] & 0x0F == 0x0B && p.len() >= 61).then(|| p[53..61].iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"));
    Some((p[4], gm))
}

#[derive(Default)]
struct State {
    streams: HashMap<Key, Stream>,
    /// The SDP text of the last announcement, by (group, port).
    sdps: HashMap<(Ipv4Addr, u16), String>,
    ptp: ptp::Tracker,
    /// Counters for the continuous listeners: (bytes, packets) at the last tick.
    ticks: HashMap<Key, (u64, u64)>,
    bytes: HashMap<Key, u64>,
}

impl State {
    fn record(&mut self, group: Ipv4Addr, port: u16, source: Ipv4Addr, p: &[u8]) -> Key {
        let key = Key { group, port, source: Some(source) };
        let now = now_ms();
        // A stream only announced so far now has its source.
        let announced = self.streams.remove(&Key { group, port, source: None });
        let s = self.streams.entry(key).or_insert_with(|| {
            let (kind, rtp_type) = classify(p, port);
            let mut s = Stream::new(group, port, Some(source), kind, now);
            s.rtp_type = rtp_type;
            if let Some(a) = announced {
                let first_seen = a.first_seen;
                s = Stream { source: Some(source), rtp_type, kind: if a.kind != Kind::Rtp { a.kind } else { kind }, first_seen, last_seen: now, ..a };
            }
            s
        });
        if s.kind == Kind::Ptp {
            self.ptp.record(source, p, now);
            if let Some((domain, gm)) = parse_ptp(p) {
                s.ptp_domain = Some(domain);
                if gm.is_some() {
                    s.ptp_grandmaster = gm;
                }
            }
        }
        s.packets += 1;
        s.last_seen = now;
        *self.bytes.entry(key).or_default() += p.len() as u64;
        key
    }

    fn announce(&mut self, sdp: &Sdp, delete: bool) {
        self.announce_at(sdp, delete, now_ms());
    }

    fn announce_at(&mut self, sdp: &Sdp, delete: bool, now: u64) {
        let (Some(group), port) = (sdp.group, sdp.port) else { return };
        if delete {
            self.streams.retain(|k, s| !(k.group == group && k.port == port && s.packets == 0));
            self.sdps.remove(&(group, port));
            return;
        }
        let mut found = false;
        for s in self.streams.values_mut().filter(|s| s.group == group && s.port == port) {
            found = true;
            s.take_sdp(sdp, now);
        }
        if !found {
            let mut s = Stream::new(group, port, sdp.source, sdp.kind.unwrap_or(Kind::Rtp), now);
            s.take_sdp(sdp, now);
            self.streams.insert(Key { group, port, source: None }, s);
        }
    }

    /// A group received without being joined (flood sample of `secs` seconds).
    fn flooded(&mut self, f: &crate::flood::Seen, secs: f64, now: u64) {
        let key = Key { group: f.group, port: f.port, source: Some(f.source) };
        let announced = self.streams.remove(&Key { group: f.group, port: f.port, source: None });
        let s = self.streams.entry(key).or_insert_with(|| {
            let (kind, rtp_type) = classify(&f.head, f.port);
            let kind = if kind == Kind::Other { Kind::Rtp } else { kind };
            let mut s = Stream::new(f.group, f.port, Some(f.source), kind, now);
            s.rtp_type = rtp_type;
            if let Some(a) = announced {
                s = Stream { source: Some(f.source), rtp_type, kind: if a.kind != Kind::Rtp { a.kind } else { kind }, ..a };
            }
            s
        });
        s.packets += f.packets;
        s.last_seen = now;
        s.flooded = now;
        s.flood_vlan = f.vlan;
        let secs = secs.max(0.1);
        self.set_rate(key, f.bytes as f64 * 8.0 / secs, f.packets as f64 / secs);
    }

    fn set_rate(&mut self, key: Key, bps: f64, pps: f64) {
        if let Some(s) = self.streams.get_mut(&key) {
            s.bps = bps;
            s.pps = pps;
            s.history.push(bps as f32);
            if s.history.len() > 60 {
                s.history.remove(0);
            }
            // Without an SDP, guess from the rate.
            if s.kind == Kind::Rtp && s.encoding.is_none() {
                if bps > 100e6 {
                    s.kind = Kind::Video;
                } else if pps >= 900.0 && bps < 40e6 {
                    s.kind = Kind::Audio;
                }
            }
        }
    }
}

/// Opens a socket that receives `group:port` on the interface `iface`.
pub fn join(group: Ipv4Addr, port: u16, iface: Ipv4Addr) -> io::Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    s.set_reuse_port(true)?;
    let _ = s.set_recv_buffer_size(4 << 20);
    // Bound to the group (Unix) so that only this group arrives; Windows wants the any address.
    let bind = if cfg!(windows) { Ipv4Addr::UNSPECIFIED } else { group };
    s.bind(&SocketAddr::V4(SocketAddrV4::new(bind, port)).into())?;
    #[cfg(target_os = "linux")]
    s.set_multicast_all_v4(false)?;
    s.join_multicast_v4(&group, &iface)?;
    s.set_nonblocking(true)?;
    UdpSocket::from_std(s.into())
}

/// The multicast engine of one interface.
#[derive(Clone)]
pub struct Engine {
    iface: Ipv4Addr,
    state: Arc<Mutex<State>>,
    closing: Arc<AtomicBool>,
    checking: Arc<AtomicBool>,
    /// Checks started so far (a flood sample that saw one start is not trusted).
    checks: Arc<std::sync::atomic::AtomicU64>,
    flood: Arc<Mutex<FloodStatus>>,
    /// Flood sampling turned on (off until said).
    flood_on: Arc<AtomicBool>,
}

/// Flood sampling of the interface.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloodStatus {
    /// The interface name sampled.
    pub iface: String,
    /// Sampling turned on.
    pub enabled: bool,
    /// Samples work (false until the first one, and when they cannot run).
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Last sample (unix ms).
    pub at: u64,
    /// Groups found flooded at the last sample.
    pub groups: Vec<Ipv4Addr>,
}

/// Time between two flood samples, and their length.
pub const FLOOD_EVERY: Duration = Duration::from_secs(300);
pub const FLOOD_SAMPLE_MS: u64 = 1000;

/// Limits of an on-demand check of one stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CheckOptions {
    /// Listening time, in milliseconds (at most 5 s).
    pub max_ms: u64,
    /// The check stops at once above this rate (megabits per second).
    pub max_mbps: u64,
}

impl Default for CheckOptions {
    fn default() -> Self {
        CheckOptions { max_ms: 2000, max_mbps: 100 }
    }
}

/// What an on-demand check found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub received: bool,
    /// Stopped early because the stream is heavier than allowed.
    pub cut_short: bool,
    pub bps: f64,
    pub pps: f64,
}

impl Engine {
    /// Starts the SAP and PTP listeners on `iface` (the address of the interface).
    pub fn start(iface: Ipv4Addr) -> Engine {
        let e = Engine::new(iface);
        for g in SAP_GROUPS {
            e.listen(g, SAP_PORT);
        }
        for port in [319, 320] {
            e.listen(PTP_GROUP, port);
        }
        let t = e.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            while !t.closing.load(Ordering::Relaxed) {
                tick.tick().await;
                t.tick();
            }
        });
        let t = e.clone();
        tokio::spawn(async move { t.watch_floods().await });
        e
    }

    fn new(iface: Ipv4Addr) -> Engine {
        Engine { iface, state: Arc::default(), closing: Arc::default(), checking: Arc::default(), checks: Arc::default(), flood: Arc::default(), flood_on: Arc::default() }
    }

    /// Turns flood sampling on or off (off when the engine starts).
    pub fn set_flood_watch(&self, on: bool) {
        self.flood_on.store(on, Ordering::Relaxed);
        self.flood.lock().unwrap_or_else(std::sync::PoisonError::into_inner).enabled = on;
    }

    /// Samples the interface for floods until closed.
    async fn watch_floods(&self) {
        let name = if_addrs::get_if_addrs().unwrap_or_default().into_iter().find(|i| i.ip() == std::net::IpAddr::V4(self.iface)).map(|i| i.name);
        let Some(name) = name else {
            self.flood.lock().unwrap_or_else(std::sync::PoisonError::into_inner).error = Some("no interface with this address".into());
            return;
        };
        self.flood.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iface.clone_from(&name);
        // The first sample 10 s after starting (or after being turned on), then every 5 minutes.
        let mut next = Instant::now() + Duration::from_secs(10);
        while !self.closing.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if !self.flood_on.load(Ordering::Relaxed) {
                next = next.max(Instant::now() + Duration::from_secs(10));
                continue;
            }
            if Instant::now() < next {
                continue;
            }
            next = Instant::now() + FLOOD_EVERY;
            let before = (self.checks.load(Ordering::SeqCst), self.is_checking());
            let n = name.clone();
            let r = tokio::task::spawn_blocking(move || crate::flood::sample(&n, FLOOD_SAMPLE_MS)).await.unwrap_or_else(|e| Err(io::Error::other(e.to_string())));
            let checked = before.1 || self.is_checking() || self.checks.load(Ordering::SeqCst) != before.0;
            let now = now_ms();
            match r {
                Ok(seen) => {
                    let seen: Vec<_> = if checked { Vec::new() } else { seen };
                    {
                        let mut st = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        for f in &seen {
                            st.flooded(f, FLOOD_SAMPLE_MS as f64 / 1000.0, now);
                        }
                    }
                    let mut fs = self.flood.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    fs.available = true;
                    fs.error = None;
                    fs.at = now;
                    if !checked {
                        fs.groups = seen.iter().map(|s| s.group).collect();
                        fs.groups.dedup();
                    }
                }
                Err(e) => {
                    let mut fs = self.flood.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if fs.error.is_none() {
                        tracing::info!("multicast: flood detection on {name} unavailable: {e}");
                    }
                    fs.available = false;
                    fs.error = Some(e.to_string());
                    fs.at = now;
                    if matches!(e.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported) {
                        return;
                    }
                }
            }
        }
    }

    /// The state of flood sampling.
    pub fn flood_status(&self) -> FloodStatus {
        self.flood.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// The clocks heard (PTP).
    pub fn ptp(&self) -> ptp::Snapshot {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).ptp.snapshot()
    }

    /// The SDP text of the last announcement of `group:port`.
    pub fn sdp(&self, group: Ipv4Addr, port: u16) -> Option<String> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).sdps.get(&(group, port)).cloned()
    }

    pub fn iface(&self) -> Ipv4Addr {
        self.iface
    }

    pub fn is_checking(&self) -> bool {
        self.checking.load(Ordering::Relaxed)
    }

    /// Stops everything (the listeners leave their groups).
    pub fn close(&self) {
        self.closing.store(true, Ordering::Relaxed);
    }

    /// All streams seen or announced.
    pub fn snapshot(&self) -> Vec<Stream> {
        let mut v: Vec<Stream> = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).streams.values().cloned().collect();
        v.sort_by_key(|s| (u32::from(s.group), s.port, s.source.map(u32::from)));
        v
    }

    /// Forgets the streams.
    pub fn clear(&self) {
        let mut st = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        st.streams.clear();
        st.bytes.clear();
        st.ticks.clear();
        st.sdps.clear();
        st.ptp = ptp::Tracker::default();
    }

    /// Rates of the continuously listened streams (SAP, PTP).
    fn tick(&self) {
        let mut st = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        st.ptp.prune(now_ms());
        let keys: Vec<Key> = st.streams.iter().filter(|(_, s)| matches!(s.kind, Kind::Ptp | Kind::Sap)).map(|(k, _)| *k).collect();
        for k in keys {
            let bytes = st.bytes.get(&k).copied().unwrap_or(0);
            let packets = st.streams[&k].packets;
            let (b0, p0) = st.ticks.insert(k, (bytes, packets)).unwrap_or((bytes, packets));
            st.set_rate(k, (bytes - b0) as f64 * 8.0, (packets - p0) as f64);
        }
    }

    fn listen(&self, group: Ipv4Addr, port: u16) {
        let sock = match join(group, port, self.iface) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("multicast: cannot listen to {group}:{port}: {e}");
                return;
            }
        };
        let (state, closing) = (self.state.clone(), self.closing.clone());
        tokio::spawn(async move {
            let mut buf = vec![0u8; 9000];
            while !closing.load(Ordering::Relaxed) {
                let Ok(Ok((n, from))) = tokio::time::timeout(Duration::from_secs(1), sock.recv_from(&mut buf)).await else { continue };
                let SocketAddr::V4(from) = from else { continue };
                let p = &buf[..n];
                let mut st = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                st.record(group, port, *from.ip(), p);
                if port == SAP_PORT {
                    if let Some((delete, sdp)) = parse_sap(p) {
                        for m in parse_sdp(&sdp) {
                            st.announce(&m, delete);
                            if let (Some(g), false) = (m.group, delete) {
                                st.sdps.insert((g, m.port), sdp.clone());
                            }
                        }
                    }
                }
            }
        });
    }

    /// A stream seen in a router's multicast routing table (SNMP): no join,
    /// no traffic. The port is unknown (0) unless an announcement gives it.
    pub fn report_routed(&self, group: Ipv4Addr, source: Ipv4Addr, bps: Option<f64>, pps: Option<f64>) {
        let mut st = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = now_ms();
        let keys: Vec<Key> = st.streams.keys().filter(|k| k.group == group && k.source.is_none_or(|s| s == source)).copied().collect();
        let keys = if keys.is_empty() {
            let key = Key { group, port: 0, source: Some(source) };
            st.streams.insert(key, Stream::new(group, 0, Some(source), Kind::Other, now));
            vec![key]
        } else {
            keys
        };
        for k in keys {
            let mut s = st.streams.remove(&k).expect("listed");
            s.source = Some(source);
            s.routed = true;
            s.last_seen = now;
            let key = Key { source: Some(source), ..k };
            st.streams.insert(key, s);
            if let (Some(b), Some(p)) = (bps, pps) {
                st.set_rate(key, b, p);
            }
        }
    }

    /// Joins one group for a moment to see what it carries. Refused while
    /// another check runs; stops as soon as the stream exceeds `max_mbps`.
    pub async fn check(&self, group: Ipv4Addr, port: u16, opts: &CheckOptions) -> io::Result<Check> {
        if !group.is_multicast() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a multicast group"));
        }
        if self.checking.swap(true, Ordering::SeqCst) {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "a check is already running"));
        }
        self.checks.fetch_add(1, Ordering::SeqCst);
        let r = self.check_one(group, port, opts).await;
        self.checking.store(false, Ordering::SeqCst);
        r
    }

    async fn check_one(&self, group: Ipv4Addr, port: u16, opts: &CheckOptions) -> io::Result<Check> {
        let sock = join(group, port, self.iface)?;
        let started = Instant::now();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(opts.max_ms.clamp(200, 5000));
        let limit = opts.max_mbps.max(1) as f64 * 1e6;
        let mut buf = vec![0u8; 9000];
        let mut seen: HashMap<Key, (u64, u64)> = HashMap::new();
        let (mut bytes, mut cut_short) = (0u64, false);
        while let Ok(Ok((n, from))) = tokio::time::timeout_at(deadline, sock.recv_from(&mut buf)).await {
            let SocketAddr::V4(from) = from else { continue };
            let key = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).record(group, port, *from.ip(), &buf[..n]);
            let e = seen.entry(key).or_default();
            e.0 += n as u64;
            e.1 += 1;
            bytes += n as u64;
            let secs = started.elapsed().as_secs_f64();
            if secs > 0.05 && bytes as f64 * 8.0 / secs > limit {
                cut_short = true;
                break;
            }
        }
        drop(sock); // leaves the group
        let secs = started.elapsed().as_secs_f64().max(0.05);
        let mut st = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut bps, mut pps) = (0.0, 0.0);
        for (k, (b, p)) in &seen {
            let (kb, kp) = (*b as f64 * 8.0 / secs, *p as f64 / secs);
            st.set_rate(*k, kb, kp);
            bps += kb;
            pps += kp;
        }
        Ok(Check { received: !seen.is_empty(), cut_short, bps, pps })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SDP: &str = "v=0\r\no=- 123 1 IN IP4 10.20.1.21\r\ns=CAM-PTZ1 Video\r\nt=0 0\r\nm=video 5004 RTP/AVP 96\r\nc=IN IP4 239.10.1.1/64\r\n\
a=source-filter: incl IN IP4 239.10.1.1 10.20.1.22\r\na=rtpmap:96 raw/90000\r\nm=audio 5004 RTP/AVP 97\r\nc=IN IP4 239.10.1.2/64\r\na=rtpmap:97 L24/48000/2\r\n";

    #[test]
    fn sdp_medias() {
        let m = parse_sdp(SDP);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].group, Some(Ipv4Addr::new(239, 10, 1, 1)));
        assert_eq!((m[0].port, m[0].kind, m[0].encoding.as_deref()), (5004, Some(Kind::Video), Some("raw")));
        assert_eq!(m[0].source, Some(Ipv4Addr::new(10, 20, 1, 22)));
        assert_eq!(m[1].source, Some(Ipv4Addr::new(10, 20, 1, 21)));
        assert_eq!(m[1].name.as_deref(), Some("CAM-PTZ1 Video"));
        assert_eq!((m[1].kind, m[1].encoding.as_deref()), (Some(Kind::Audio), Some("L24")));
    }

    #[test]
    fn sap_packets() {
        let mut p = vec![0x20, 0, 0x12, 0x34, 10, 20, 1, 21];
        p.extend_from_slice(b"application/sdp\0");
        p.extend_from_slice(SDP.as_bytes());
        let (delete, sdp) = parse_sap(&p).unwrap();
        assert!(!delete);
        assert!(sdp.starts_with("v=0"));
        let mut p2 = vec![0x24, 0, 0, 0, 10, 0, 0, 1];
        p2.extend_from_slice(b"v=0\r\n");
        assert!(parse_sap(&p2).unwrap().0);
    }

    #[test]
    fn packet_kinds() {
        let mut ts = vec![0u8; 188 * 7];
        ts[0] = 0x47;
        assert_eq!(classify(&ts, 1234).0, Kind::MpegTs);
        let mut r = vec![0x80, 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
        r.extend_from_slice(&[0u8; 1200]);
        assert_eq!(classify(&r, 5004), (Kind::Rtp, Some(96)));
        let mut rts = vec![0x80, 33, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
        rts.extend_from_slice(&ts);
        assert_eq!(classify(&rts, 5004).0, Kind::MpegTs);
        assert_eq!(classify(b"hello", 5004).0, Kind::Other);
    }

    #[test]
    fn announced_then_received() {
        let mut st = State::default();
        st.announce(&parse_sdp(SDP)[0], false);
        assert_eq!(st.streams.len(), 1);
        let mut r = vec![0x80, 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
        r.extend_from_slice(&[0u8; 1200]);
        st.record(Ipv4Addr::new(239, 10, 1, 1), 5004, Ipv4Addr::new(10, 20, 1, 22), &r);
        assert_eq!(st.streams.len(), 1);
        let s = st.streams.values().next().unwrap();
        assert_eq!((s.kind, s.name.as_deref(), s.announced), (Kind::Video, Some("CAM-PTZ1 Video"), true));
    }

    fn engine() -> Engine {
        Engine::new(Ipv4Addr::LOCALHOST)
    }

    const DUP: &str = "v=0\r\no=- 7 1 IN IP4 10.10.1.11\r\ns=CAM-01 Video\r\nt=0 0\r\na=group:DUP primary secondary\r\n\
m=video 5004 RTP/AVP 96\r\nc=IN IP4 239.10.1.1/64\r\na=source-filter: incl IN IP4 239.10.1.1 10.10.1.11\r\na=rtpmap:96 raw/90000\r\n\
a=fmtp:96 sampling=YCbCr-4:2:2; width=1920; height=1080; exactframerate=50; depth=10; colorimetry=BT709\r\n\
a=ts-refclk:ptp=IEEE1588-2008:EC-46-70-FF-FE-00-1A-2B:127\r\na=mediaclk:direct=0\r\na=mid:primary\r\n\
m=video 5004 RTP/AVP 96\r\nc=IN IP4 239.20.1.1/64\r\na=source-filter: incl IN IP4 239.20.1.1 10.20.1.11\r\na=rtpmap:96 raw/90000\r\n\
a=fmtp:96 sampling=YCbCr-4:2:2; width=1920; height=1080; exactframerate=50; depth=10; colorimetry=BT709\r\na=mid:secondary\r\n";

    #[test]
    fn sdp_dup_pairs_and_formats() {
        let m = parse_sdp(DUP);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].dup, Some(Leg { group: Ipv4Addr::new(239, 20, 1, 1), port: 5004, source: Some(Ipv4Addr::new(10, 20, 1, 11)) }));
        assert_eq!(m[1].dup, Some(Leg { group: Ipv4Addr::new(239, 10, 1, 1), port: 5004, source: Some(Ipv4Addr::new(10, 10, 1, 11)) }));
        let f = m[0].format.as_ref().unwrap();
        assert_eq!((f.width, f.height, f.frame_rate, f.ptp_domain), (Some(1920), Some(1080), Some(50.0), Some(127)));
        // The session's ts-refclk is not repeated: the second media has none of its own.
        assert_eq!(m[1].format.as_ref().unwrap().ptp_gm, None);
        assert!(parse_sdp(SDP).iter().all(|m| m.dup.is_none()));
        let mut st = State::default();
        st.announce_at(&m[0], false, 1000);
        st.announce_at(&m[0], false, 31_000);
        let s = st.streams.values().next().unwrap();
        assert_eq!((s.last_announced, s.sap_interval_ms), (Some(31_000), Some(30_000)));
        assert!((s.est_bps.unwrap() / 1e9 - 2.177).abs() < 0.01);
        assert!(s.dup.is_some());
    }

    #[test]
    fn floods_become_streams() {
        let mut st = State::default();
        st.announce(&parse_sdp(SDP)[0], false);
        let mut head = vec![0x80, 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
        head.resize(32, 0);
        let f = crate::flood::Seen { group: Ipv4Addr::new(239, 10, 1, 1), port: 5004, source: Ipv4Addr::new(10, 20, 1, 22), packets: 2000, bytes: 2_400_000, vlan: Some(110), head };
        st.flooded(&f, 2.0, 5000);
        assert_eq!(st.streams.len(), 1);
        let s = st.streams.values().next().unwrap();
        assert_eq!((s.flooded, s.flood_vlan, s.announced, s.kind), (5000, Some(110), true, Kind::Video));
        assert_eq!((s.bps, s.pps), (9_600_000.0, 1000.0));
    }

    #[test]
    fn ptp_packets_feed_the_tracker() {
        let mut st = State::default();
        let gm = [0xEC, 0x46, 0x70, 0xFF, 0xFE, 0, 0x1A, 0x2B];
        let a = ptp::Announce { priority1: 128, clock_class: 6, clock_accuracy: 0x21, variance: 0x4E5D, priority2: 128, gm, steps_removed: 0, time_source: 0x20, utc_offset: 37 };
        st.record(PTP_GROUP, 320, Ipv4Addr::new(10, 99, 0, 47), &ptp::build(ptp::MSG_ANNOUNCE, 127, gm, Some(&a)));
        st.record(PTP_GROUP, 319, Ipv4Addr::new(10, 10, 1, 11), &ptp::build(ptp::MSG_DELAY_REQ, 127, [1; 8], None));
        let s = st.ptp.snapshot();
        assert_eq!((s.announcers.len(), s.followers.len()), (1, 1));
        assert_eq!(s.announcers[0].gm, "EC:46:70:FF:FE:00:1A:2B");
    }

    #[test]
    fn routed_streams_merge_with_announcements() {
        let e = engine();
        e.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).announce(&parse_sdp(SDP)[0], false);
        e.report_routed(Ipv4Addr::new(239, 10, 1, 1), Ipv4Addr::new(10, 20, 1, 22), Some(1.2e9), Some(110_000.0));
        e.report_routed(Ipv4Addr::new(239, 9, 9, 9), Ipv4Addr::new(10, 0, 0, 9), None, None);
        let s = e.snapshot();
        assert_eq!(s.len(), 2);
        let cam = s.iter().find(|s| s.group == Ipv4Addr::new(239, 10, 1, 1)).unwrap();
        assert_eq!((cam.port, cam.routed, cam.announced, cam.bps), (5004, true, true, 1.2e9));
    }

    /// Sends to a group on loopback and checks it (skipped where loopback has no multicast).
    #[tokio::test]
    async fn check_receives_and_cuts_heavy_streams() {
        let group = Ipv4Addr::new(239, 77, 1, 1);
        let iface = Ipv4Addr::LOCALHOST;
        let e = engine();
        let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
        if sock.set_multicast_if_v4(&iface).is_err() || join(group, 45004, iface).is_err() {
            return;
        }
        let _ = sock.set_multicast_loop_v4(true);
        let sender: std::net::UdpSocket = sock.into();
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let t = std::thread::spawn(move || {
            let mut pkt = vec![0x80, 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
            pkt.extend_from_slice(&[0u8; 1000]);
            while !s2.load(Ordering::Relaxed) {
                let _ = sender.send_to(&pkt, (group, 45004));
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        let c = e.check(group, 45004, &CheckOptions { max_ms: 500, max_mbps: 100 }).await.unwrap();
        let heavy = e.check(group, 45004, &CheckOptions { max_ms: 2000, max_mbps: 1 }).await.unwrap();
        stop.store(true, Ordering::Relaxed);
        t.join().unwrap();
        if !c.received {
            eprintln!("no multicast on loopback, skipped");
            return;
        }
        assert!(!c.cut_short && c.pps > 10.0);
        assert!(heavy.cut_short);
        assert_eq!(e.snapshot()[0].group, group);
    }
}
