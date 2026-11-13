//! Multicast streams.
//!
//! - SAP announcements (239.255.255.255 and 224.2.127.254, port 9875) are
//!   listened to all the time: they give the streams' names and formats (SDP).
//! - PTP (224.0.1.129, ports 319/320) is listened to all the time: domain and
//!   grandmaster.
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
        }
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
}

/// The medias of an SDP description.
pub fn parse_sdp(text: &str) -> Vec<Sdp> {
    let mut session = Sdp::default();
    let mut medias: Vec<Sdp> = Vec::new();
    let mut origin = None;
    for line in text.lines().map(str::trim) {
        let Some((k, v)) = line.split_once('=') else { continue };
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
                }
            }
            _ => {}
        }
    }
    if medias.is_empty() {
        medias.push(session.clone());
    }
    for m in &mut medias {
        m.name = m.name.clone().or_else(|| session.name.clone());
        m.group = m.group.or(session.group);
        m.source = m.source.or(session.source).or(origin);
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
            if let Some(a) = &announced {
                s.name.clone_from(&a.name);
                s.encoding.clone_from(&a.encoding);
                s.announced = true;
                if a.kind != Kind::Rtp {
                    s.kind = a.kind;
                }
            }
            s
        });
        if s.kind == Kind::Ptp {
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
        let (Some(group), port) = (sdp.group, sdp.port) else { return };
        if delete {
            self.streams.retain(|k, s| !(k.group == group && k.port == port && s.packets == 0));
            return;
        }
        let mut found = false;
        for s in self.streams.values_mut().filter(|s| s.group == group && s.port == port) {
            found = true;
            s.announced = true;
            s.name.clone_from(&sdp.name);
            s.encoding.clone_from(&sdp.encoding);
            if let Some(k) = sdp.kind {
                s.kind = k;
            }
        }
        if !found {
            let now = now_ms();
            let mut s = Stream::new(group, port, sdp.source, sdp.kind.unwrap_or(Kind::Rtp), now);
            s.encoding.clone_from(&sdp.encoding);
            s.name.clone_from(&sdp.name);
            s.announced = true;
            self.streams.insert(Key { group, port, source: None }, s);
        }
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
}

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
        let e = Engine { iface, state: Arc::default(), closing: Arc::default(), checking: Arc::default() };
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
        e
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
        let mut v: Vec<Stream> = self.state.lock().unwrap().streams.values().cloned().collect();
        v.sort_by_key(|s| (u32::from(s.group), s.port, s.source.map(u32::from)));
        v
    }

    /// Forgets the streams.
    pub fn clear(&self) {
        let mut st = self.state.lock().unwrap();
        st.streams.clear();
        st.bytes.clear();
        st.ticks.clear();
    }

    /// Rates of the continuously listened streams (SAP, PTP).
    fn tick(&self) {
        let mut st = self.state.lock().unwrap();
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
                let mut st = state.lock().unwrap();
                st.record(group, port, *from.ip(), p);
                if port == SAP_PORT {
                    if let Some((delete, sdp)) = parse_sap(p) {
                        for m in parse_sdp(&sdp) {
                            st.announce(&m, delete);
                        }
                    }
                }
            }
        });
    }

    /// A stream seen in a router's multicast routing table (SNMP): no join,
    /// no traffic. The port is unknown (0) unless an announcement gives it.
    pub fn report_routed(&self, group: Ipv4Addr, source: Ipv4Addr, bps: Option<f64>, pps: Option<f64>) {
        let mut st = self.state.lock().unwrap();
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
            let key = self.state.lock().unwrap().record(group, port, *from.ip(), &buf[..n]);
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
        let mut st = self.state.lock().unwrap();
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
        Engine { iface: Ipv4Addr::LOCALHOST, state: Arc::default(), closing: Arc::default(), checking: Arc::default() }
    }

    #[test]
    fn routed_streams_merge_with_announcements() {
        let e = engine();
        e.state.lock().unwrap().announce(&parse_sdp(SDP)[0], false);
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
