//! NMOS without a registry ("nodes" mode): the Node APIs (IS-04, peer-to-peer)
//! are found by mDNS (`_nmos-node._tcp`, on every local interface inside the
//! ranges searched), on the web ports of the scanned machines (inside the ranges,
//! once an hour a machine), or typed by hand, then read with GET: `self`,
//! `devices`, `sources`, `flows`, `senders`, `receivers`, the senders' SDP
//! (`manifest_href`) and, when a sender has no usable SDP and for every receiver,
//! the IS-05 `GET …/single/{senders|receivers}/{id}/active` of the device's
//! Connection API. Nothing else is ever sent: no PUT/PATCH/POST, no activation.
//!
//! What the nodes say is put together as one [`Registry`], so the flows read it
//! exactly as a registry's (`mflows::registry`).

use std::collections::BTreeMap;

use super::*;
use strata_net::mcast::Leg;

/// mDNS type of the Node APIs.
pub const NODE_TYPE: &str = "_nmos-node._tcp.local";
/// Polls a node may miss before it is "gone" (its data kept until then).
pub const GRACE_MISSES: u32 = 3;
/// A node past its grace period is polled at most this often.
const UNREACHABLE_EVERY_MS: i64 = 30_000;
/// The mDNS question is asked again this often.
const BROWSE_EVERY_MS: i64 = 60_000;
/// A node heard by mDNS stays a candidate this long without being heard again.
const MDNS_KEPT_MS: i64 = 10 * 60_000;
/// A scanned machine is asked whether it has a Node API at most this often.
const PROBE_EVERY_MS: i64 = 60 * 60_000;
/// Machines asked in one round (every second at most, one round at a time).
const PROBE_HOSTS: usize = 16;
const PROBE_WORKERS: usize = 4;
/// Nodes read at the same time.
pub const POLL_WORKERS: usize = 8;
/// SDP and IS-05 reads at the same time for one node.
const NODE_WORKERS: usize = 2;
/// IS-05 `active` reads of one poll at most (the rest next time).
const ACTIVE_PER_READ: usize = 64;
/// An IS-05 `active` is read again when the resource's version changes, else this often.
const ACTIVE_REFRESH_MS: i64 = 60_000;
const NODE_CONNECT: Duration = Duration::from_secs(2);
const NODE_REQUEST: Duration = Duration::from_secs(5);
const PROBE_CONNECT: Duration = Duration::from_secs(1);
const PROBE_REQUEST: Duration = Duration::from_secs(2);
/// A port that does not take a TCP connection this fast is not asked.
const PROBE_TCP: Duration = Duration::from_millis(600);
/// Largest answer of a Node API (JSON).
const NODE_JSON_LIMIT: u64 = 16 << 20;
const SMALL_LIMIT: u64 = 256 << 10;
/// IS-05 default RTP destination port.
const RTP_PORT: u16 = 5004;

// ── Addresses ──

/// How a node was found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FoundBy {
    Mdns,
    Scan,
    Manual,
    #[default]
    Registry,
}

/// The base of a Node API as typed: `10.1.1.7:8080`, `http://10.1.1.7:8080/`,
/// `https://gw.lan/x-nmos/node/v1.3` → `http://10.1.1.7:8080`, `https://gw.lan`.
pub fn node_base(url: &str) -> Result<String, String> {
    let wrong = || "the node address is written http://host:port".to_string();
    let u = url.trim();
    if u.is_empty() || u.contains(|c: char| c.is_whitespace() || c == '?' || c == '#' || c == '@') {
        return Err(wrong());
    }
    let (scheme, rest) = match u.split_once("://") {
        Some((s, r)) if s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https") => (s.to_ascii_lowercase(), r),
        Some(_) => return Err(wrong()),
        None => ("http".to_string(), u),
    };
    let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, String::new()));
    let path = path.trim_end_matches('/');
    if !(path.is_empty() || path.starts_with("/x-nmos")) {
        return Err(wrong());
    }
    let (name, port) = match host.rsplit_once(':') {
        Some((h, p)) => (h, Some(p.parse::<u16>().ok().filter(|p| *p != 0).ok_or_else(wrong)?)),
        None => (host, None),
    };
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_') {
        return Err(wrong());
    }
    let name = name.to_ascii_lowercase();
    Ok(match port {
        Some(p) => format!("{scheme}://{name}:{p}"),
        None => format!("{scheme}://{name}"),
    })
}

/// The IPv4 address of a base, when it is written with one.
pub fn base_ip(base: &str) -> Option<Ipv4Addr> {
    host_port(base).and_then(|(h, _)| h.parse().ok())
}

/// One range searched: a CIDR block (no size limit) or a range of `cidr::parse`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub first: u32,
    pub last: u32,
}

impl Span {
    pub fn parse(s: &str) -> Result<Span, String> {
        let s = s.trim();
        if s.contains('/') {
            let (net, p) = strata_net::cidr::parse_block(s)?;
            let mask = if p == 0 { 0 } else { u32::MAX << (32 - u32::from(p)) };
            let first = u32::from(net) & mask;
            return Ok(Span { first, last: first | !mask });
        }
        let r = strata_net::cidr::parse(s)?;
        Ok(Span { first: u32::from(r.first()), last: u32::from(r.last()) })
    }

    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        (self.first..=self.last).contains(&u32::from(ip))
    }
}

/// Where nodes are looked for: the ranges (none: anywhere) and the local
/// interfaces whose address is in one of them (mDNS is asked on each).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scope {
    pub spans: Vec<Span>,
    pub ifaces: Vec<Ipv4Addr>,
}

impl Scope {
    /// `ranges`: the settings' (empty: automatic); `networks`: the scan networks'
    /// ranges; `ifaces`: the local IPv4 interfaces. Automatic: the scan networks a
    /// local interface is in, else anywhere and every interface.
    pub fn new(ranges: &[String], networks: &[String], ifaces: &[(String, Ipv4Addr)]) -> Scope {
        let mut spans: Vec<Span> = ranges.iter().filter_map(|r| Span::parse(r).ok()).collect();
        if ranges.is_empty() {
            spans = networks.iter().filter_map(|r| Span::parse(r).ok()).filter(|sp| ifaces.iter().any(|(_, ip)| sp.contains(*ip))).collect();
        }
        let mut out: Vec<Ipv4Addr> = ifaces.iter().map(|(_, ip)| *ip).filter(|ip| !ip.is_loopback() && (spans.is_empty() && ranges.is_empty() || spans.iter().any(|s| s.contains(*ip)))).collect();
        out.sort();
        out.dedup();
        Scope { spans, ifaces: out }
    }

    pub fn allows(&self, ip: Ipv4Addr) -> bool {
        self.spans.is_empty() || self.spans.iter().any(|s| s.contains(ip))
    }
}

// ── Reading JSON of a node ──

/// The IS-05 Connection API of a device (`controls`): v1.1 first, then v1.0.
pub fn connection_api(device: &Value) -> Option<String> {
    let list = device.get("controls")?.as_array()?;
    let mut best: Option<(u8, String)> = None;
    for c in list {
        let Some(ty) = text(c, "type") else { continue };
        let Some(v) = ty.strip_prefix("urn:x-nmos:control:sr-ctrl/") else { continue };
        let Some(href) = text(c, "href").filter(|h| h.starts_with("http://") || h.starts_with("https://")) else { continue };
        let rank = match v {
            "v1.1" => 2,
            "v1.0" => 1,
            _ => 0,
        };
        if best.as_ref().is_none_or(|(r, _)| rank > *r) {
            best = Some((rank, href.trim_end_matches('/').to_string()));
        }
    }
    best.map(|b| b.1)
}

fn rational(v: Option<&Value>) -> Option<f64> {
    let v = v?;
    let n = v.get("numerator")?.as_f64()?;
    let d = v.get("denominator").and_then(Value::as_f64).unwrap_or(1.0);
    (n > 0.0 && d > 0.0).then(|| n / d)
}

fn rational_text(v: Option<&Value>) -> Option<String> {
    let v = v?;
    let n = v.get("numerator")?.as_u64()?;
    Some(match v.get("denominator").and_then(Value::as_u64).filter(|d| *d > 1) {
        Some(d) => format!("{n}/{d}"),
        None => n.to_string(),
    })
}

/// What an IS-04 flow (or source) says of its media, as an SDP format would.
pub fn media_of(v: &Value) -> Option<Format> {
    let mut f = Format::default();
    if let Some(ch) = v.get("channels").and_then(Value::as_array) {
        f.channels = u16::try_from(ch.len()).ok().filter(|n| *n > 0);
    }
    let mt = text(v, "media_type").map(|m| m.to_ascii_lowercase());
    f.standard = match mt.as_deref() {
        Some("video/raw") => Some("ST 2110-20"),
        Some("video/jxsv") => Some("ST 2110-22"),
        Some("audio/l24") | Some("audio/l16") => Some("ST 2110-30"),
        Some("audio/am824") => Some("ST 2110-31"),
        Some("video/smpte291") => Some("ST 2110-40"),
        Some("video/smpte2022-6") => Some("ST 2022-6"),
        Some("video/mp2t") => Some("MPEG-TS"),
        _ => None,
    }
    .map(str::to_string);
    let num = |k: &str| v.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).filter(|n| *n > 0);
    f.width = num("frame_width");
    f.height = num("frame_height");
    f.frame_rate = rational(v.get("grain_rate")).filter(|r| *r < 1000.0);
    f.frame_rate_text = f.frame_rate.and_then(|_| rational_text(v.get("grain_rate")));
    f.interlaced = text(v, "interlace_mode").is_some_and(|m| m != "progressive");
    f.colorimetry = text(v, "colorspace");
    f.tcs = text(v, "transfer_characteristic");
    if let Some(comps) = v.get("components").and_then(Value::as_array) {
        let comp = |n: &str| comps.iter().find(|c| text(c, "name").as_deref() == Some(n));
        let wh = |c: &Value| (c.get("width").and_then(Value::as_u64).unwrap_or(0), c.get("height").and_then(Value::as_u64).unwrap_or(0));
        f.depth = comps.first().and_then(|c| c.get("bit_depth")).and_then(Value::as_u64).and_then(|d| u8::try_from(d).ok());
        f.sampling = match (comp("Y"), comp("Cb"), comp("R")) {
            (Some(y), Some(cb), _) => {
                let ((yw, yh), (cw, ch)) = (wh(y), wh(cb));
                if cw == yw && ch == yh {
                    Some("YCbCr-4:4:4")
                } else if cw * 2 == yw && ch == yh {
                    Some("YCbCr-4:2:2")
                } else if cw * 2 == yw && ch * 2 == yh {
                    Some("YCbCr-4:2:0")
                } else {
                    None
                }
            }
            (_, _, Some(_)) => Some("RGB"),
            _ => None,
        }
        .map(str::to_string);
    }
    if let Some(rate) = rational(v.get("sample_rate")) {
        f.audio_rate = Some(rate as u32);
        f.clock_rate = Some(rate as u32);
    }
    if let Some(d) = v.get("bit_depth").and_then(Value::as_u64).and_then(|d| u8::try_from(d).ok()) {
        f.depth = Some(d);
    }
    (f != Format::default()).then_some(f)
}

/// The highest Node API version a node offers (`GET /x-nmos/node/`: `["v1.2/", "v1.3/"]`).
pub fn pick_version(body: &str) -> Result<String, String> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(body) else { return Err("nmos: not a node api".into()) };
    let have: Vec<String> = list.iter().filter_map(Value::as_str).map(|v| v.trim().trim_matches('/').to_string()).collect();
    ["v1.3", "v1.2", "v1.1"].into_iter().find(|v| have.iter().any(|h| h == v)).map(str::to_string).ok_or_else(|| "nmos: no supported node api version".into())
}

/// One entry of IS-05 `transport_params`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TxLeg {
    /// Senders: `destination_ip`; receivers: `multicast_ip`.
    pub dest: Option<Ipv4Addr>,
    pub source: Option<Ipv4Addr>,
    pub port: Option<u16>,
    /// Receivers: `interface_ip`.
    pub iface: Option<Ipv4Addr>,
    /// `rtp_enabled` not false.
    pub enabled: bool,
}

/// What `GET …/single/{senders|receivers}/{id}/active` answered.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Active {
    /// The IS-04 version of the resource when it was read, and when.
    pub version: String,
    pub href: String,
    pub at: i64,
    pub master_enable: Option<bool>,
    pub sender_id: Option<String>,
    pub legs: Vec<TxLeg>,
    pub error: Option<String>,
}

fn ip_field(v: &Value, k: &str) -> Option<Ipv4Addr> {
    v.get(k)?.as_str()?.trim().parse().ok().filter(|ip: &Ipv4Addr| !ip.is_unspecified())
}

/// Reads an IS-05 `active` (sender or receiver): `master_enable`, `sender_id`,
/// `transport_params` (the other fields left empty).
pub fn parse_active(body: &str) -> Result<Active, String> {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return Err("nmos: not a connection api".into()) };
    if !v.is_object() {
        return Err("nmos: not a connection api".into());
    }
    let legs = v
        .get("transport_params")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|t| TxLeg {
                    dest: ip_field(t, "destination_ip").or_else(|| ip_field(t, "multicast_ip")),
                    source: ip_field(t, "source_ip"),
                    port: match t.get("destination_port") {
                        Some(Value::Number(n)) => n.as_u64().and_then(|p| u16::try_from(p).ok()).filter(|p| *p != 0),
                        Some(Value::String(s)) => s.trim().parse().ok().filter(|p| *p != 0),
                        _ => None,
                    },
                    iface: ip_field(t, "interface_ip"),
                    enabled: t.get("rtp_enabled").and_then(Value::as_bool) != Some(false),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Active { master_enable: v.get("master_enable").and_then(Value::as_bool), sender_id: text(&v, "sender_id"), legs, ..Active::default() })
}

// ── One node ──

/// One good read of a node.
#[derive(Clone, Debug, Default)]
pub struct NodeRead {
    /// The Node API version read ("v1.3").
    pub api: String,
    pub node: Node,
    pub devices: Vec<Device>,
    pub sources: Vec<Essence>,
    pub flows: Vec<Essence>,
    pub senders: Vec<Sender>,
    pub receivers: Vec<Receiver>,
    /// By sender id: the SDP of `manifest_href`.
    pub sdp: HashMap<String, SdpEntry>,
    /// By sender or receiver id: IS-05 `active`.
    pub active: HashMap<String, Active>,
}

fn node_err(e: String) -> String {
    match e.as_str() {
        "nmos: timed out" => "nmos: the node does not answer in time".into(),
        "nmos: cannot reach the registry" => "nmos: cannot reach the node".into(),
        _ => e.replace("the registry answered", "the node answered"),
    }
}

fn get_json(get: &Get, url: &str, limit: u64) -> Result<Value, String> {
    let p = get(url, limit).map_err(node_err)?;
    if !(200..300).contains(&p.status) {
        return Err(format!("nmos: the node answered {}", p.status));
    }
    serde_json::from_str(&p.body).map_err(|_| "nmos: not a node api".to_string())
}

/// The version of the Node API at `base` (`GET {base}/x-nmos/node/`).
pub fn node_version(base: &str, get: &Get) -> Result<String, String> {
    let p = get(&format!("{base}/x-nmos/node/"), SMALL_LIMIT).map_err(node_err)?;
    if !(200..300).contains(&p.status) {
        return Err(format!("nmos: the node answered {}", p.status));
    }
    pick_version(&p.body)
}

/// Runs `jobs` on at most `workers` threads.
pub(crate) fn par<T: Send, R: Send>(jobs: Vec<T>, workers: usize, f: &(dyn Fn(T) -> R + Sync)) -> Vec<R> {
    if jobs.is_empty() {
        return Vec::new();
    }
    let workers = workers.clamp(1, jobs.len());
    let queue = Mutex::new(jobs);
    let done = Mutex::new(Vec::new());
    std::thread::scope(|sc| {
        for _ in 0..workers {
            sc.spawn(|| loop {
                let job = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pop();
                let Some(job) = job else { break };
                let r = f(job);
                done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(r);
            });
        }
    });
    done.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Reads a node: its version (`api`: the one read last time, else asked), its
/// resources, the SDPs that changed, and the IS-05 `active` that are due.
pub fn read_node(base: &str, api: Option<&str>, prev: Option<&NodeRead>, get: &Get, now: i64) -> Result<NodeRead, String> {
    let api = match api {
        Some(v) => v.to_string(),
        None => node_version(base, get)?,
    };
    let root = format!("{base}/x-nmos/node/{api}");
    let me = get_json(get, &format!("{root}/self"), SMALL_LIMIT)?;
    let mut node = Node::parse(&me).ok_or("nmos: not a node api")?;
    if let Some(ip) = base_ip(base).filter(|ip| !ip.is_loopback() && !ip.is_unspecified()) {
        if !node.ips.contains(&ip) {
            node.ips.push(ip);
        }
    }
    let list = |res: &str| -> Result<Vec<Value>, String> {
        match get_json(get, &format!("{root}/{res}"), NODE_JSON_LIMIT)? {
            Value::Array(a) => Ok(a),
            _ => Err("nmos: not a node api".into()),
        }
    };
    let mut devices: Vec<Device> = list("devices")?.iter().filter_map(Device::parse).collect();
    for d in devices.iter_mut() {
        d.node_id.clone_from(&node.id);
    }
    let sources: Vec<Essence> = list("sources")?.iter().filter_map(Essence::parse).collect();
    let flows: Vec<Essence> = list("flows")?.iter().filter_map(Essence::parse).collect();
    let senders: Vec<Sender> = list("senders")?.iter().filter_map(Sender::parse).collect();
    let receivers: Vec<Receiver> = list("receivers")?.iter().filter_map(Receiver::parse).collect();
    let sdp = read_sdps_with(&senders, prev.map(|p| &p.sdp), get, now, NODE_WORKERS);
    // IS-05: senders without a usable SDP, and every receiver.
    let conn = |device_id: &str| devices.iter().find(|d| d.id == device_id).and_then(|d| d.conn.clone());
    let mut wanted: Vec<(String, String, String)> = Vec::new();
    for s in senders.iter().filter(|s| s.transport.is_empty() || s.transport.contains("rtp")) {
        if sdp.get(&s.id).is_some_and(|e| !e.medias.is_empty()) {
            continue;
        }
        if let Some(c) = conn(&s.device_id) {
            wanted.push((s.id.clone(), s.version.clone(), format!("{c}/single/senders/{}/active", s.id)));
        }
    }
    for r in &receivers {
        if let Some(c) = conn(&r.device_id) {
            wanted.push((r.id.clone(), r.version.clone(), format!("{c}/single/receivers/{}/active", r.id)));
        }
    }
    let mut active: HashMap<String, Active> = HashMap::new();
    let mut todo: Vec<(String, String, String, bool)> = Vec::new();
    for (id, version, href) in wanted {
        match prev.and_then(|p| p.active.get(&id)) {
            Some(a) if a.href == href && a.version == version && now - a.at < ACTIVE_REFRESH_MS => {
                active.insert(id, a.clone());
            }
            Some(a) => {
                // Kept until read again (the refresh may wait for the next poll).
                active.insert(id.clone(), a.clone());
                todo.push((id, version, href, true));
            }
            None => todo.push((id, version, href, false)),
        }
    }
    // The ones never read first.
    todo.sort_by_key(|t| t.3);
    todo.truncate(ACTIVE_PER_READ);
    let jobs: Vec<(String, String, String)> = todo.into_iter().map(|(a, b, c, _)| (a, b, c)).collect();
    let read = par(jobs, NODE_WORKERS, &|(id, version, href): (String, String, String)| {
        let mut a = Active { version, href: href.clone(), at: now, ..Active::default() };
        match get(&href, SMALL_LIMIT).map_err(node_err) {
            Ok(p) if (200..300).contains(&p.status) => match parse_active(&p.body) {
                Ok(x) => {
                    a.master_enable = x.master_enable;
                    a.sender_id = x.sender_id;
                    a.legs = x.legs;
                }
                Err(e) => a.error = Some(e),
            },
            Ok(p) => a.error = Some(format!("nmos: the node answered {}", p.status)),
            Err(e) => a.error = Some(e),
        }
        (id, a)
    });
    active.extend(read);
    Ok(NodeRead { api, node, devices, sources, flows, senders, receivers, sdp, active })
}

/// A test read of a Node API: its version, label, senders and receivers.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checked {
    pub url: String,
    pub found: Vec<FoundBy>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    pub senders: usize,
    pub receivers: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn check_node(base: &str, found: Vec<FoundBy>, get: &Get) -> Checked {
    let mut c = Checked { url: base.to_string(), found, ..Checked::default() };
    let run = || -> Result<(String, String, usize, usize), String> {
        let api = node_version(base, get)?;
        let root = format!("{base}/x-nmos/node/{api}");
        let node = Node::parse(&get_json(get, &format!("{root}/self"), SMALL_LIMIT)?).ok_or("nmos: not a node api")?;
        let count = |res: &str| get_json(get, &format!("{root}/{res}"), NODE_JSON_LIMIT).map(|v| v.as_array().map(Vec::len).unwrap_or(0));
        Ok((api, node.label, count("senders")?, count("receivers")?))
    };
    match run() {
        Ok((api, label, s, r)) => {
            c.ok = true;
            c.api = Some(api);
            c.label = Some(label);
            c.senders = s;
            c.receivers = r;
        }
        Err(e) => c.error = Some(e),
    }
    c
}

/// Whether `ip:port` (`scheme`) serves a Node API: its base. A port that
/// refuses or does not answer a TCP connection quickly is skipped without an
/// HTTP request.
pub fn probe_port(ip: Ipv4Addr, port: u16, scheme: &str, get: &Get) -> Option<String> {
    let addr = std::net::SocketAddr::from((ip, port));
    std::net::TcpStream::connect_timeout(&addr, PROBE_TCP).ok()?;
    let scheme = if scheme == "https" { "https" } else { "http" };
    let base = format!("{scheme}://{ip}:{port}");
    node_version(&base, get).ok().map(|_| base)
}

/// The scheme first tried on a port with no known web service.
fn guess_scheme(port: u16) -> &'static str {
    if matches!(port, 443 | 8443) {
        "https"
    } else {
        "http"
    }
}

/// The ports to try on one machine: its web ports (with their scheme), then the
/// ports the nodes found use, then the ports of the settings; each once.
pub fn candidate_ports(web: &[(u16, String)], learned: &[u16], configured: &[u16]) -> Vec<(u16, String)> {
    let mut out: Vec<(u16, String)> = Vec::new();
    for (port, scheme) in web {
        if !out.iter().any(|(p, _)| p == port) {
            out.push((*port, scheme.clone()));
        }
    }
    for port in learned.iter().chain(configured) {
        if !out.iter().any(|(p, _)| p == port) {
            out.push((*port, guess_scheme(*port).to_string()));
        }
    }
    out
}

/// A scanned machine and its web ports: (address, [(port, "http" | "https")]).
pub type WebHost = (Ipv4Addr, Vec<(u16, String)>);

/// The scanned machines to ask now: in the scope, not a known node's address,
/// not asked in the last hour; at most `max`.
pub fn due_hosts(hosts: &[WebHost], probed: &HashMap<Ipv4Addr, i64>, known: &HashSet<Ipv4Addr>, scope: &Scope, now: i64, max: usize) -> Vec<WebHost> {
    let mut out: Vec<(WebHost, i64)> = hosts
        .iter()
        .filter(|(ip, ports)| !ports.is_empty() && scope.allows(*ip) && !known.contains(ip))
        .filter_map(|(ip, ports)| {
            let last = probed.get(ip).copied();
            last.is_none_or(|t| now - t >= PROBE_EVERY_MS).then(|| ((*ip, ports.clone()), last.unwrap_or(i64::MIN)))
        })
        .collect();
    // Never asked first, then the longest ago.
    out.sort_by_key(|x| (x.1, u32::from(x.0 .0)));
    out.into_iter().take(max).map(|x| x.0).collect()
}

// ── All the nodes, as one registry ──

/// How one node (or one Node API that never answered) is read, for the pages.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    /// The Node API base: `http://10.1.1.7:8080`.
    pub url: String,
    pub found: Vec<FoundBy>,
    /// Its mDNS instance name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The Node API version read ("v1.3").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    /// Last good read (unix ms; 0 = never), last try and its error.
    pub last_ok: i64,
    pub tried: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One Node API polled.
#[derive(Clone, Debug, Default)]
pub struct PeerState {
    pub info: PeerInfo,
    pub data: Option<Arc<NodeRead>>,
    /// Polls missed in a row.
    pub misses: u32,
    /// Next poll (unix ms), and a poll is running.
    pub next: i64,
    pub busy: bool,
}

impl PeerState {
    /// Answering, or within its grace period.
    pub fn present(&self) -> bool {
        self.data.is_some() && self.misses < GRACE_MISSES
    }

    /// Keeps a poll.
    pub fn record(&mut self, got: Result<NodeRead, String>, tried: i64, interval_ms: i64) {
        self.busy = false;
        self.info.tried = tried;
        self.next = tried + interval_ms;
        match got {
            Ok(d) => {
                self.info.api = Some(d.api.clone());
                self.data = Some(Arc::new(d));
                self.info.last_ok = tried;
                self.info.error = None;
                self.misses = 0;
            }
            Err(e) => {
                if self.info.error.as_ref() != Some(&e) {
                    tracing::info!("nmos: {}: {e}", self.info.url);
                }
                self.info.error = Some(e);
                self.misses = self.misses.saturating_add(1);
                // Past its grace period: asked every 30 s at most often (it may be off for long).
                if self.misses >= GRACE_MISSES {
                    self.next = tried + interval_ms.max(UNREACHABLE_EVERY_MS);
                }
            }
        }
    }
}

fn kind_of_format(format: &str) -> Option<Kind> {
    match format.rsplit(':').next()? {
        "video" => Some(Kind::Video),
        "audio" => Some(Kind::Audio),
        "data" => Some(Kind::Data),
        "mux" => Some(Kind::MpegTs),
        _ => None,
    }
}

fn encoding_of(f: &Format) -> Option<String> {
    Some(
        match f.standard.as_deref()? {
            "ST 2110-20" => "raw",
            "ST 2110-22" => "jxsv",
            "ST 2110-30" if f.depth == Some(16) => "L16",
            "ST 2110-30" => "L24",
            "ST 2110-31" => "AM824",
            "ST 2110-40" => "smpte291",
            "ST 2022-6" => "SMPTE2022-6",
            "MPEG-TS" => "MP2T",
            _ => return None,
        }
        .to_string(),
    )
}

/// The medias of a sender known by its IS-05 `active` only: one a multicast
/// destination of an enabled leg (two: an ST 2022-7 pair), with the format its
/// IS-04 flow describes.
pub fn medias_of_active(s: &Sender, a: &Active, flows: &[Essence], sources: &[Essence]) -> Vec<Sdp> {
    if a.master_enable == Some(false) {
        return Vec::new();
    }
    let flow = flows.iter().find(|f| Some(&f.id) == s.flow_id.as_ref());
    let source = flow.and_then(|f| sources.iter().find(|x| Some(&x.id) == f.source_id.as_ref()));
    let mut format = flow.and_then(|f| f.media.clone());
    if let (Some(f), Some(ch)) = (format.as_mut(), source.and_then(|x| x.media.as_ref()).and_then(|m| m.channels)) {
        f.channels = Some(ch);
    }
    let kind = flow.and_then(|f| kind_of_format(&f.format)).or_else(|| source.and_then(|x| kind_of_format(&x.format)));
    let legs: Vec<Leg> = a.legs.iter().filter(|l| l.enabled).filter_map(|l| Some(Leg { group: l.dest.filter(Ipv4Addr::is_multicast)?, port: l.port.unwrap_or(RTP_PORT), source: l.source })).collect();
    legs.iter()
        .enumerate()
        .map(|(i, l)| Sdp {
            name: Some(s.label.clone()).filter(|x| !x.is_empty()),
            group: Some(l.group),
            port: l.port,
            kind,
            encoding: format.as_ref().and_then(encoding_of),
            source: l.source,
            format: format.clone(),
            mid: None,
            dup: if legs.len() == 2 { Some(legs[1 - i].clone()) } else { None },
        })
        .collect()
}

/// Puts the nodes together: answering ones (and those in their grace period)
/// as the registry's nodes; those past it (kept [`NODE_KEPT_MS`] after their last
/// good read) as gone, their senders `stale` (not live, never "gone"), without
/// their receivers. A node reached at two addresses is one node (the first
/// found, the others' ways of finding it added). IS-05 fills what IS-04 lacks:
/// the medias of senders without SDP, the groups receivers are set to, their
/// `sender_id` and `active` when IS-04 has none.
pub fn aggregate(list: &[&PeerState], now: i64) -> Registry {
    let mut order: Vec<&PeerState> = list.to_vec();
    let best = |p: &PeerState| p.info.found.iter().min().copied().unwrap_or(FoundBy::Registry);
    order.sort_by(|a, b| b.present().cmp(&a.present()).then_with(|| best(a).cmp(&best(b))).then_with(|| a.info.url.cmp(&b.info.url)));
    let mut reg = Registry { at: now, ..Registry::default() };
    for p in order {
        let Some(d) = p.data.as_deref() else {
            reg.unread.push(p.info.clone());
            continue;
        };
        let id = d.node.id.clone();
        if let Some(info) = reg.direct.get_mut(&id) {
            for f in &p.info.found {
                if !info.found.contains(f) {
                    info.found.push(*f);
                }
            }
            info.found.sort();
            continue;
        }
        let present = p.present();
        if !present && now - p.info.last_ok >= NODE_KEPT_MS {
            continue;
        }
        reg.direct.insert(id.clone(), p.info.clone());
        reg.seen.insert(id.clone(), p.info.last_ok);
        if present {
            reg.nodes.push(d.node.clone());
        } else {
            reg.gone.push(d.node.clone());
        }
        reg.devices.extend(d.devices.iter().cloned());
        reg.sources.extend(d.sources.iter().cloned());
        reg.flows.extend(d.flows.iter().cloned());
        for s in &d.senders {
            let mut s = s.clone();
            s.stale = !present;
            let act = d.active.get(&s.id).filter(|a| a.error.is_none());
            if s.active.is_none() {
                s.active = act.and_then(|a| a.master_enable);
            }
            let manifest = d.sdp.get(&s.id);
            let from_active = act.map(|a| (a, medias_of_active(&s, a, &d.flows, &d.sources))).filter(|(_, m)| !m.is_empty());
            match (manifest, from_active) {
                (Some(e), _) if !e.medias.is_empty() => {
                    reg.sdp.insert(s.id.clone(), e.clone());
                }
                (_, Some((a, medias))) => {
                    reg.sdp.insert(s.id.clone(), SdpEntry { version: s.version.clone(), href: a.href.clone(), at: a.at, text: None, medias, error: None });
                }
                (Some(e), None) => {
                    reg.sdp.insert(s.id.clone(), e.clone());
                }
                (None, None) => {}
            }
            reg.senders.push(s);
        }
        if !present {
            continue;
        }
        for r in &d.receivers {
            let mut r = r.clone();
            if let Some(a) = d.active.get(&r.id).filter(|a| a.error.is_none()) {
                if r.sender_id.is_none() {
                    r.sender_id.clone_from(&a.sender_id);
                }
                if r.active.is_none() {
                    r.active = a.master_enable;
                }
                if a.master_enable != Some(false) {
                    r.legs = a.legs.iter().filter(|l| l.enabled).filter_map(|l| Some(RxLeg { group: l.dest.filter(Ipv4Addr::is_multicast)?, source: l.source, port: l.port })).collect();
                }
                r.ifaces = a.legs.iter().filter_map(|l| l.iface).filter(|ip| !ip.is_unspecified() && !ip.is_loopback()).collect();
            }
            reg.receivers.push(r);
        }
    }
    reg.unread.sort_by(|a, b| a.url.cmp(&b.url));
    reg
}

// ── Discovery and polling ──

/// A Node API heard by mDNS.
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    pub name: String,
    /// Last time it was heard (unix ms).
    pub seen: i64,
}

/// The Node APIs of an mDNS answer: `<api_proto>://<ip, else host>:<port>` → instance name.
pub fn heard(services: &[strata_net::mdns::Service]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for s in services.iter().filter(|s| s.port != 0) {
        let txt = |k: &str| s.txt.iter().find_map(|t| t.split_once('=').filter(|(a, _)| a.trim().eq_ignore_ascii_case(k)).map(|(_, v)| v.trim().to_string()));
        let proto = if txt("api_proto").is_some_and(|p| p.eq_ignore_ascii_case("https")) { "https" } else { "http" };
        let host = s.ip.map(|ip| ip.to_string()).unwrap_or_else(|| s.host.trim_end_matches('.').to_ascii_lowercase());
        if host.is_empty() {
            continue;
        }
        let base = format!("{proto}://{host}:{}", s.port);
        if !out.iter().any(|(b, _)| *b == base) {
            out.push((base, s.name.clone()));
        }
    }
    out
}

/// Discovery and polling state (nodes mode).
#[derive(Default)]
pub struct Peers {
    pub mdns: HashMap<String, Heard>,
    pub browsed: i64,
    pub browsing: bool,
    /// Scanned machines asked (unix ms), and the Node APIs found there (when last found).
    pub probed: HashMap<Ipv4Addr, i64>,
    pub scan: HashMap<String, i64>,
    pub probing: bool,
    pub nodes: BTreeMap<String, PeerState>,
    /// The interfaces mDNS was last asked on.
    pub ifaces: Vec<Ipv4Addr>,
    /// Last `changed: nmos` sent (unix ms).
    pub told: i64,
}

impl Peers {
    /// Makes the polled set what the settings, mDNS and the scan say: typed
    /// addresses always; mDNS and scan ones while heard / found in the last 10
    /// min or answering. Whether a node that was answering left the set.
    pub fn reconcile(&mut self, urls: &[String], now: i64) -> bool {
        let answering = |nodes: &BTreeMap<String, PeerState>, b: &str| nodes.get(b).is_some_and(|p| p.info.last_ok > 0 && now - p.info.last_ok < MDNS_KEPT_MS);
        let nodes = &self.nodes;
        self.mdns.retain(|b, h| now - h.seen < MDNS_KEPT_MS || answering(nodes, b));
        self.scan.retain(|b, at| now - *at < MDNS_KEPT_MS || answering(nodes, b));
        let mut want: BTreeMap<String, (Vec<FoundBy>, Option<String>)> = BTreeMap::new();
        for (b, h) in &self.mdns {
            let e = want.entry(b.clone()).or_default();
            e.0.push(FoundBy::Mdns);
            e.1 = Some(h.name.clone());
        }
        for b in self.scan.keys() {
            want.entry(b.clone()).or_default().0.push(FoundBy::Scan);
        }
        for u in urls {
            if let Ok(b) = node_base(u) {
                want.entry(b).or_default().0.push(FoundBy::Manual);
            }
        }
        let before = self.nodes.len();
        let mut dropped_live = false;
        self.nodes.retain(|b, p| {
            let keep = want.contains_key(b);
            if !keep && p.present() {
                dropped_live = true;
            }
            keep
        });
        let mut changed = before != self.nodes.len();
        for (b, (mut found, name)) in want {
            found.sort();
            found.dedup();
            let p = self.nodes.entry(b.clone()).or_insert_with(|| {
                changed = true;
                PeerState { info: PeerInfo { url: b.clone(), ..PeerInfo::default() }, next: now, ..PeerState::default() }
            });
            if p.info.found != found || p.info.name != name {
                p.info.found = found;
                p.info.name = name;
                changed = true;
            }
        }
        if dropped_live {
            // Removed by hand (or by the settings): not "gone" streams.
            new_generation();
        }
        changed
    }

    /// The node addresses known (not asked again by the scan probe).
    pub fn known_ips(&self) -> HashSet<Ipv4Addr> {
        let mut out: HashSet<Ipv4Addr> = self.nodes.keys().filter_map(|b| base_ip(b)).collect();
        for p in self.nodes.values() {
            if let Some(d) = &p.data {
                out.extend(d.node.ips.iter().copied());
            }
        }
        out
    }

    /// The ports the nodes found use (most used first), tried on the other machines.
    pub fn known_ports(&self) -> Vec<u16> {
        let mut count: HashMap<u16, usize> = HashMap::new();
        for b in self.nodes.keys().chain(self.mdns.keys()).chain(self.scan.keys()) {
            if let Some((_, port)) = host_port(b) {
                *count.entry(port).or_default() += 1;
            }
        }
        let mut out: Vec<(u16, usize)> = count.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out.into_iter().map(|x| x.0).take(8).collect()
    }
}

/// A Node API reader for the local network: short timeouts, no proxy, and
/// equipment certificates accepted (self-signed): only GETs of public data go there.
pub fn node_agent(connect: Duration, global: Duration) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().disable_verification(true).build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_connect(Some(connect))
        .timeout_global(Some(global))
        .http_status_as_error(false)
        .max_redirects(3)
        .proxy(None)
        .user_agent(concat!("strata/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

impl Net {
    /// The search scope of nodes mode now.
    pub fn nmos_scope(&self, s: &Settings) -> Scope {
        let networks: Vec<String> = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).networks.iter().map(|n| n.range.clone()).collect();
        Scope::new(&s.ranges, &networks, &super::super::interfaces())
    }

    /// The scanned machines that answer, with their web ports: (address, [(port, scheme)]).
    fn web_hosts(&self) -> Vec<WebHost> {
        let st = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        st.addresses.values().filter(|a| a.up || !a.ports.is_empty()).map(|a| (a.ip, a.ports.iter().filter_map(|p| p.web.as_ref().map(|w| (p.port, w.scheme.clone()))).collect::<Vec<_>>())).collect()
    }
}

/// Asks mDNS for Node APIs on every interface (3 questions 1 s apart, both
/// styles, 3 s for the answers).
pub async fn browse_nodes(ifaces: &[Ipv4Addr]) -> Vec<strata_net::mdns::Service> {
    let mut set = tokio::task::JoinSet::new();
    for ip in ifaces.iter().copied() {
        set.spawn(async move { strata_net::mdns::browse_types(ip, &[NODE_TYPE], 3, Duration::from_secs(3)).await });
    }
    let mut out = Vec::new();
    while let Some(r) = set.join_next().await {
        match r {
            Ok(Ok(list)) => out.extend(list),
            Ok(Err(e)) => tracing::debug!("nmos: mdns: {e}"),
            Err(e) => tracing::debug!("nmos: mdns: {e}"),
        }
    }
    out
}

type PollDone = (String, Result<NodeRead, String>, i64);

/// The nodes-mode loop's own state (between ticks).
#[derive(Default)]
pub struct Poller {
    set: tokio::task::JoinSet<PollDone>,
    agent: Option<ureq::Agent>,
}

impl Poller {
    /// Forgets the polls running (nodes mode left).
    pub fn stop(&mut self, net: &Net) {
        self.set.abort_all();
        let mut p = net.nmos.peers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !p.nodes.is_empty() || !p.mdns.is_empty() {
            *p = Peers::default();
        }
    }

    /// One step: starts the mDNS question and the scan probe when due, makes the
    /// polled set, starts the polls due (at most [`POLL_WORKERS`] running), waits
    /// a second (or a settings change, or a poll ending), keeps what ended.
    pub async fn tick(&mut self, net: &Arc<Net>, s: &Settings) {
        let now = now_ms();
        let interval = s.interval().as_millis() as i64;
        let agent = self.agent.get_or_insert_with(|| node_agent(NODE_CONNECT, NODE_REQUEST)).clone();
        let scope = net.nmos_scope(s);
        let (browse, probe) = {
            let mut p = net.nmos.peers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let browse = !p.browsing && (now - p.browsed >= BROWSE_EVERY_MS || p.ifaces != scope.ifaces) && !scope.ifaces.is_empty();
            if browse {
                p.browsing = true;
                p.browsed = now;
                p.ifaces.clone_from(&scope.ifaces);
            }
            let probe = if s.probe_scan && !p.probing {
                let learned = p.known_ports();
                let hosts: Vec<WebHost> = net.web_hosts().into_iter().map(|(ip, web)| (ip, candidate_ports(&web, &learned, &s.node_ports))).collect();
                let due = due_hosts(&hosts, &p.probed, &p.known_ips(), &scope, now, PROBE_HOSTS);
                for (ip, _) in &due {
                    p.probed.insert(*ip, now);
                }
                p.probing = !due.is_empty();
                due
            } else {
                Vec::new()
            };
            (browse, probe)
        };
        if browse {
            let n = net.clone();
            let ifaces = scope.ifaces.clone();
            tokio::spawn(async move {
                let list = browse_nodes(&ifaces).await;
                let at = now_ms();
                let mut p = n.nmos.peers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                for (b, name) in heard(&list) {
                    p.mdns.insert(b, Heard { name, seen: at });
                }
                p.browsing = false;
                drop(p);
                n.nmos.wake();
            });
        }
        if !probe.is_empty() {
            let n = net.clone();
            tokio::spawn(async move {
                let found = tokio::task::spawn_blocking(move || {
                    let agent = node_agent(PROBE_CONNECT, PROBE_REQUEST);
                    let get = |u: &str, limit: u64| http_get(&agent, u, limit);
                    par(probe, PROBE_WORKERS, &|(ip, ports): (Ipv4Addr, Vec<(u16, String)>)| ports.iter().find_map(|(port, scheme)| probe_port(ip, *port, scheme, &get)))
                })
                .await
                .unwrap_or_default();
                let at = now_ms();
                let mut p = n.nmos.peers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                for b in found.into_iter().flatten() {
                    p.scan.insert(b, at);
                }
                p.probing = false;
                drop(p);
                n.nmos.wake();
            });
        }
        // The polled set, and the polls due.
        let mut changed = {
            let mut p = net.nmos.peers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let changed = p.reconcile(&s.node_urls, now);
            let running = self.set.len();
            let mut due: Vec<(&String, &mut PeerState)> = p.nodes.iter_mut().filter(|(_, x)| !x.busy && x.next <= now).collect();
            due.sort_by_key(|(_, x)| x.next);
            for (base, x) in due.into_iter().take(POLL_WORKERS.saturating_sub(running)) {
                x.busy = true;
                let (base, prev) = (base.clone(), x.data.clone());
                // The version is asked again after a failed poll.
                let api = x.info.api.clone().filter(|_| x.misses == 0);
                let agent = agent.clone();
                self.set.spawn_blocking(move || {
                    let tried = now_ms();
                    let got = read_node(&base, api.as_deref(), prev.as_deref(), &|u: &str, limit: u64| http_get(&agent, u, limit), tried);
                    (base, got, tried)
                });
            }
            changed
        };
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            _ = net.nmos.wake.notified() => {}
            Some(r) = self.set.join_next(), if !self.set.is_empty() => {
                changed |= self.keep(net, r, interval);
            }
        }
        while let Some(r) = self.set.try_join_next() {
            changed |= self.keep(net, r, interval);
        }
        if changed {
            self.publish(net);
        }
    }

    fn keep(&self, net: &Net, r: Result<PollDone, tokio::task::JoinError>, interval: i64) -> bool {
        let Ok((base, got, tried)) = r else { return false };
        let mut p = net.nmos.peers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match p.nodes.get_mut(&base) {
            Some(x) => {
                x.record(got, tried, interval);
                true
            }
            None => false,
        }
    }

    /// Puts the nodes together and tells the pages (the flows at once when they
    /// change, the NMOS page at most every 5 s otherwise).
    fn publish(&self, net: &Net) {
        let now = now_ms();
        let (reg, tried, force) = {
            let p = net.nmos.peers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let list: Vec<&PeerState> = p.nodes.values().collect();
            let tried = list.iter().map(|x| x.info.tried).max().unwrap_or(0);
            (aggregate(&list, now), tried, now - p.told >= 5_000)
        };
        let flows = net.nmos.store(NODES_BASE, Ok(reg), tried);
        if flows {
            net.changed("multicast");
        }
        if flows || force {
            net.nmos.peers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).told = now;
            net.changed("nmos");
        }
    }
}

/// The "base" of the nodes-mode reading (another base than any Query API).
pub const NODES_BASE: &str = "nodes:";

#[cfg(test)]
mod tests;
