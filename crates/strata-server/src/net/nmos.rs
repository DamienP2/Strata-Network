//! NMOS, read only: a registry (AMWA IS-04 Query API) or, without one, the Node
//! APIs of the nodes themselves (`peers`: found by mDNS, among the scanned
//! machines, or typed). The nodes, devices, sources, flows, senders and receivers
//! are read with HTTP GET every 5 s ("live") or every 60 s, and each sender's SDP
//! (`manifest_href`) with GET when the sender's version changes. Nothing is ever
//! sent to the registry or to a node besides those GETs: no registration, no
//! IS-05 connection change (only `GET …/active` of the Connection API, nodes
//! mode), no Query API subscription (the "live" mode is polling).

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use strata_net::mcast::{parse_sdp, Kind, Sdp};
use strata_net::mformat::Format;
use strata_net::mdns::Service;
use tokio::sync::Notify;

use super::Net;
use crate::api::{bad, require, status, ApiResult, Session};
use crate::paths::{now_ms, read_json};
use crate::state::Shared;

/// The settings file in `<data>/network/`.
pub const FILE: &str = "nmos.json";
/// Items asked for in one page.
const PAGE_LIMIT: usize = 500;
/// Pages read at most for one resource.
const MAX_PAGES: usize = 200;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest answer read from the registry (JSON), and from a node (SDP).
const JSON_LIMIT: u64 = 64 << 20;
const SDP_LIMIT: u64 = 256 << 10;
/// SDP files read at the same time.
const SDP_WORKERS: usize = 8;
/// A SDP that could not be read is asked again after this long (same sender version).
const SDP_RETRY_MS: i64 = 60_000;
/// Nodes that left the registry stay listed this long.
const NODE_KEPT_MS: i64 = 10 * 60_000;
/// mDNS answers older than this are asked again when the registries found are asked for.
const BROWSE_AGE_MS: i64 = 60_000;
/// The mDNS kind of `_nmos-query._tcp` (`strata_net::mdns::TYPES`).
pub const QUERY_KIND: &str = "NMOS Query";

pub mod peers;

// ── Settings ──

fn default_true() -> bool {
    true
}

fn default_version() -> String {
    "v1.3".into()
}

fn default_refresh() -> String {
    "live".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub enabled: bool,
    /// The Query API, as typed: `http://10.1.1.20:8010` or `…/x-nmos/query/v1.3`.
    #[serde(default)]
    pub url: String,
    /// "v1.2" or "v1.3".
    #[serde(default = "default_version")]
    pub version: String,
    /// "live" (read every 5 s) or "60s".
    #[serde(default = "default_refresh")]
    pub refresh: String,
    /// "registry" (the Query API at `url`) or "nodes" (the Node APIs, no registry).
    /// Missing (settings saved before): "registry" when `url` is set, else "nodes".
    #[serde(default)]
    pub source: String,
    /// Nodes mode: Node APIs typed by hand (`http://10.1.1.7:8080`), for the
    /// nodes mDNS does not reach (another VLAN).
    #[serde(default)]
    pub node_urls: Vec<String>,
    /// Nodes mode: also looks for Node APIs on the web ports of the scanned machines.
    #[serde(default = "default_true")]
    pub probe_scan: bool,
    /// Nodes mode: the ranges searched (CIDR `10.1.1.0/24` or `10.1.1.10-10.1.1.50`):
    /// mDNS on each local interface inside one, the scan probe of the machines
    /// inside. Empty: the scan networks a local interface is in (else everywhere).
    #[serde(default)]
    pub ranges: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { enabled: false, url: String::new(), version: default_version(), refresh: default_refresh(), source: "nodes".into(), node_urls: Vec::new(), probe_scan: true, ranges: Vec::new() }
    }
}

/// Node addresses typed by hand, at most.
pub const MAX_NODE_URLS: usize = 100;

impl Settings {
    /// The source of settings saved before it existed.
    pub fn migrate(&mut self) {
        if self.source.is_empty() {
            self.source = if self.url.trim().is_empty() { "nodes" } else { "registry" }.into();
        }
    }

    pub fn nodes_mode(&self) -> bool {
        self.source == "nodes"
    }

    /// Trims and checks what was sent.
    pub fn check(&mut self) -> Result<(), String> {
        self.migrate();
        self.url = self.url.trim().trim_end_matches('/').to_string();
        if !matches!(self.source.as_str(), "registry" | "nodes") {
            return Err("the nmos source is registry or nodes".into());
        }
        let mut urls: Vec<String> = Vec::new();
        for u in &self.node_urls {
            if u.trim().is_empty() {
                continue;
            }
            let base = peers::node_base(u)?;
            if !urls.contains(&base) {
                urls.push(base);
            }
        }
        if urls.len() > MAX_NODE_URLS {
            return Err(format!("at most {MAX_NODE_URLS} node addresses"));
        }
        self.node_urls = urls;
        let mut ranges: Vec<String> = Vec::new();
        for r in self.ranges.iter().map(|r| r.trim()).filter(|r| !r.is_empty()) {
            peers::Span::parse(r).map_err(|e| format!("nmos range {r}: {e}"))?;
            if !ranges.iter().any(|x| x == r) {
                ranges.push(r.to_string());
            }
        }
        if ranges.len() > MAX_NODE_URLS {
            return Err(format!("at most {MAX_NODE_URLS} nmos ranges"));
        }
        self.ranges = ranges;
        if !matches!(self.version.as_str(), "v1.2" | "v1.3") {
            return Err("the nmos version is v1.2 or v1.3".into());
        }
        if !matches!(self.refresh.as_str(), "live" | "60s") {
            return Err("the nmos refresh is live or 60s".into());
        }
        if self.url.is_empty() {
            return if self.enabled && !self.nodes_mode() { Err("the registry address cannot be empty".into()) } else { Ok(()) };
        }
        query_base(&self.url, &self.version).map(|_| ())
    }

    /// The Query API read, when turned on in registry mode.
    pub fn base(&self) -> Option<String> {
        if !self.enabled || self.nodes_mode() {
            return None;
        }
        query_base(&self.url, &self.version).ok()
    }

    pub fn interval(&self) -> Duration {
        Duration::from_secs(if self.refresh == "60s" { 60 } else { 5 })
    }

    /// What the history says of these settings.
    pub fn summary(&self) -> String {
        let on = if self.enabled { "on" } else { "off" };
        if self.nodes_mode() {
            let urls = if self.node_urls.is_empty() { "no address".to_string() } else { self.node_urls.join(", ") };
            return format!("{on} · nodes · {urls} · scan {} · {}", if self.probe_scan { "on" } else { "off" }, self.refresh);
        }
        format!("{on} · {} · {} · {}", if self.url.is_empty() { "no address" } else { &self.url }, self.version, self.refresh)
    }
}

/// The Query API base of an address: `http://10.1.1.20:8010` or `…/x-nmos/query/v1.2`
/// → `http://10.1.1.20:8010/x-nmos/query/<version>`.
pub fn query_base(url: &str, version: &str) -> Result<String, String> {
    let u = url.trim().trim_end_matches('/');
    let wrong = || "the registry address is written http://host:port".to_string();
    let rest = u.strip_prefix("http://").or_else(|| u.strip_prefix("https://")).ok_or_else(wrong)?;
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() || host.starts_with(':') || u.contains(|c: char| c.is_whitespace() || c == '?' || c == '#') {
        return Err(wrong());
    }
    let root = match u.find("/x-nmos/query") {
        Some(i) => &u[..i],
        None => u,
    };
    Ok(format!("{root}/x-nmos/query/{version}"))
}

/// `host:port` of a URL (port 80 / 443 when not written).
pub(crate) fn host_port(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let auth = rest.split(['/', '?', '#']).next()?;
    let auth = auth.rsplit('@').next()?;
    // `[::1]:80`, `[::1]`, `host:80`, `host`.
    let (host, port) = match auth.rsplit_once(':').filter(|(h, _)| !h.contains('[') || h.ends_with(']')) {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (auth, if scheme.eq_ignore_ascii_case("https") { 443 } else { 80 }),
    };
    (!host.is_empty()).then(|| (host.trim_matches(['[', ']']).to_ascii_lowercase(), port))
}

// ── What the registry holds (only what is used) ──

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Iface {
    pub name: String,
    /// As the node gives them (LLDP identifiers of its own interface).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chassis_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port_id: Option<String>,
    /// v1.3 `attached_network_device`: the switch's chassis ID and port ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attached: Option<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub id: String,
    pub version: String,
    pub label: String,
    pub hostname: Option<String>,
    /// IPv4 addresses of its API endpoints and `href`.
    pub ips: Vec<Ipv4Addr>,
    /// MACs among its interfaces' chassis and port IDs ("00:11:22:AA:BB:CC").
    pub macs: Vec<String>,
    pub interfaces: Vec<Iface>,
    pub tags: Vec<(String, Vec<String>)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    pub id: String,
    pub version: String,
    pub label: String,
    pub node_id: String,
    pub tags: Vec<(String, Vec<String>)>,
    /// Its IS-05 Connection API (`controls`, `urn:x-nmos:control:sr-ctrl/…`):
    /// `http://10.1.1.7:8080/x-nmos/connection/v1.1` (read with GET only).
    pub conn: Option<String>,
}

/// A source or a flow: its format ("urn:x-nmos:format:video").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Essence {
    pub id: String,
    pub format: String,
    /// Flows: their source.
    pub source_id: Option<String>,
    /// Flows: what their IS-04 description says of the media (size, rate, sampling…),
    /// for the senders known without SDP. Sources: `channels` of an audio source.
    pub media: Option<Format>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sender {
    pub id: String,
    pub version: String,
    pub label: String,
    pub flow_id: Option<String>,
    pub device_id: String,
    pub transport: String,
    pub manifest_href: Option<String>,
    /// `subscription.active` (v1.2+): the sender is enabled; `None` before v1.2.
    pub active: Option<bool>,
    /// Nodes mode: its node no longer answers (past the grace period): kept, not live.
    pub stale: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Receiver {
    pub id: String,
    pub version: String,
    pub label: String,
    pub device_id: String,
    pub format: String,
    /// `subscription.sender_id`: the sender it is set to receive.
    pub sender_id: Option<String>,
    /// `subscription.active` (v1.2+); `None` before v1.2.
    pub active: Option<bool>,
    /// Nodes mode: what its IS-05 `active` parameters say it receives (one leg,
    /// two for ST 2022-7), when `master_enable` is not false.
    pub legs: Vec<RxLeg>,
    /// Nodes mode: IS-05 `interface_ip` of its legs (the node's media addresses).
    pub ifaces: Vec<Ipv4Addr>,
}

/// A multicast group a receiver is set to (IS-05 `transport_params`).
#[derive(Clone, Debug, PartialEq)]
pub struct RxLeg {
    pub group: Ipv4Addr,
    /// `source_ip` (SSM).
    pub source: Option<Ipv4Addr>,
    /// `destination_port`.
    pub port: Option<u16>,
}

impl Receiver {
    /// Set to receive a sender, and not turned off.
    pub fn subscribed(&self) -> Option<&str> {
        self.sender_id.as_deref().filter(|_| self.active != Some(false))
    }

    /// Set (IS-05) to receive this group, port and source (a port or source not
    /// known on either side matches).
    pub fn receives(&self, group: Ipv4Addr, port: u16, source: Option<Ipv4Addr>) -> bool {
        self.legs.iter().any(|l| l.group == group && (port == 0 || l.port.is_none_or(|p| p == port)) && (source.is_none() || l.source.is_none() || l.source == source))
    }
}

/// The SDP of a sender, as read for one version of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SdpEntry {
    pub version: String,
    pub href: String,
    /// When it was read (unix ms).
    pub at: i64,
    pub text: Option<String>,
    /// Its multicast medias (two for an ST 2022-7 pair).
    pub medias: Vec<Sdp>,
    pub error: Option<String>,
}

/// One read of the registry.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    /// When it was read (unix ms).
    pub at: i64,
    pub nodes: Vec<Node>,
    pub devices: Vec<Device>,
    pub sources: Vec<Essence>,
    pub flows: Vec<Essence>,
    pub senders: Vec<Sender>,
    pub receivers: Vec<Receiver>,
    /// By sender id.
    pub sdp: HashMap<String, SdpEntry>,
    /// Node id → last read it was in (unix ms).
    pub seen: HashMap<String, i64>,
    /// Nodes no longer in the registry, kept [`NODE_KEPT_MS`].
    pub gone: Vec<Node>,
    /// Nodes mode: how each node (by id) is read; and the Node APIs found that
    /// never answered (no node id).
    pub direct: HashMap<String, peers::PeerInfo>,
    pub unread: Vec<peers::PeerInfo>,
}

impl Registry {
    pub fn node_of_device(&self, device_id: &str) -> Option<&Node> {
        let d = self.devices.iter().find(|d| d.id == device_id)?;
        self.nodes.iter().find(|n| n.id == d.node_id)
    }

    pub fn medias(&self, sender_id: &str) -> &[Sdp] {
        self.sdp.get(sender_id).map(|e| e.medias.as_slice()).unwrap_or(&[])
    }

    /// The format of a sender's flow (else of its source): "urn:x-nmos:format:video"…
    pub fn sender_format(&self, s: &Sender) -> Option<&str> {
        let f = self.flows.iter().find(|f| Some(&f.id) == s.flow_id.as_ref())?;
        Some(f.format.as_str()).filter(|x| !x.is_empty()).or_else(|| self.sources.iter().find(|x| Some(&x.id) == f.source_id.as_ref()).map(|x| x.format.as_str()).filter(|x| !x.is_empty()))
    }

    /// Video senders: by their flow's format, else by their SDP.
    pub fn is_video(&self, s: &Sender) -> bool {
        match self.sender_format(s) {
            Some(f) => f.ends_with(":video"),
            None => self.medias(&s.id).iter().any(|m| m.kind == Some(Kind::Video)),
        }
    }

    pub fn subscribed(&self) -> usize {
        self.receivers.iter().filter(|r| r.subscribed().is_some() || !r.legs.is_empty()).count()
    }

    /// A fingerprint of what the flows take from the registry (changes when they would).
    pub fn fingerprint(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for n in &self.nodes {
            (&n.id, &n.version).hash(&mut h);
        }
        for d in &self.devices {
            (&d.id, &d.version).hash(&mut h);
        }
        for s in &self.senders {
            (&s.id, &s.version, s.active, self.medias(&s.id).len()).hash(&mut h);
        }
        for s in &self.senders {
            s.stale.hash(&mut h);
            for m in self.medias(&s.id) {
                (m.group, m.port, m.source).hash(&mut h);
            }
        }
        for r in &self.receivers {
            (&r.id, &r.version, &r.sender_id, r.active).hash(&mut h);
            for l in &r.legs {
                (l.group, l.port, l.source).hash(&mut h);
            }
        }
        h.finish()
    }

    /// The SDP text that describes `group:port`.
    pub fn sdp_text(&self, group: Ipv4Addr, port: u16) -> Option<String> {
        self.sdp.values().find(|e| e.medias.iter().any(|m| m.group == Some(group) && m.port == port)).and_then(|e| e.text.clone())
    }
}

// ── Reading JSON (never trusted: every field may be missing or of another type) ──

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k)?.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn tags(v: &Value) -> Vec<(String, Vec<String>)> {
    let Some(m) = v.get("tags").and_then(Value::as_object) else { return Vec::new() };
    m.iter().map(|(k, x)| (k.clone(), x.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default())).collect()
}

fn ip_of_host(h: &str) -> Option<Ipv4Addr> {
    h.trim().trim_matches(['[', ']']).parse().ok()
}

pub fn norm_mac(s: &str) -> Option<String> {
    strata_net::mac::parse(s).filter(|m| m.iter().any(|b| *b != 0)).map(|m| strata_net::mac::format(&m))
}

fn subscription(v: &Value) -> (Option<String>, Option<bool>, Option<String>) {
    let s = v.get("subscription");
    let active = s.and_then(|s| s.get("active")).and_then(Value::as_bool);
    (s.and_then(|s| text(s, "sender_id")), active, s.and_then(|s| text(s, "receiver_id")))
}

impl Node {
    fn parse(v: &Value) -> Option<Node> {
        let id = text(v, "id")?;
        let mut ips = Vec::new();
        if let Some(eps) = v.get("api").and_then(|a| a.get("endpoints")).and_then(Value::as_array) {
            for e in eps {
                if let Some(ip) = text(e, "host").as_deref().and_then(ip_of_host) {
                    if !ips.contains(&ip) {
                        ips.push(ip);
                    }
                }
            }
        }
        if let Some(ip) = text(v, "href").as_deref().and_then(host_port).and_then(|(h, _)| ip_of_host(&h)) {
            if !ips.contains(&ip) {
                ips.push(ip);
            }
        }
        ips.retain(|ip| !ip.is_loopback() && !ip.is_unspecified());
        let interfaces: Vec<Iface> = v
            .get("interfaces")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|i| Iface {
                        name: text(i, "name").unwrap_or_default(),
                        chassis_id: text(i, "chassis_id"),
                        port_id: text(i, "port_id"),
                        attached: i.get("attached_network_device").and_then(|d| Some((text(d, "chassis_id")?, text(d, "port_id")?))),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut macs: Vec<String> = Vec::new();
        for m in interfaces.iter().flat_map(|i| [i.port_id.as_deref(), i.chassis_id.as_deref()]).flatten().filter_map(norm_mac) {
            if !macs.contains(&m) {
                macs.push(m);
            }
        }
        Some(Node {
            label: text(v, "label").unwrap_or_else(|| text(v, "hostname").unwrap_or_else(|| id.clone())),
            version: text(v, "version").unwrap_or_default(),
            hostname: text(v, "hostname"),
            ips,
            macs,
            interfaces,
            tags: tags(v),
            id,
        })
    }

    /// "Cobalt · BIDI4": the asset tags (BCP-002-02) of the node, else of its devices.
    pub fn maker(&self, devices: &[&Device]) -> Option<String> {
        let tag = |list: &[(String, Vec<String>)], k: &str| list.iter().find(|(t, _)| t == k).and_then(|(_, v)| v.first().cloned()).filter(|s| !s.trim().is_empty());
        let pick = |k: &str| tag(&self.tags, k).or_else(|| devices.iter().find_map(|d| tag(&d.tags, k)));
        let parts: Vec<String> = [pick("urn:x-nmos:tag:asset:manufacturer/v1.0"), pick("urn:x-nmos:tag:asset:product/v1.0")].into_iter().flatten().collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

impl Device {
    fn parse(v: &Value) -> Option<Device> {
        Some(Device {
            id: text(v, "id")?,
            version: text(v, "version").unwrap_or_default(),
            label: text(v, "label").unwrap_or_default(),
            node_id: text(v, "node_id").unwrap_or_default(),
            tags: tags(v),
            conn: peers::connection_api(v),
        })
    }
}

impl Essence {
    fn parse(v: &Value) -> Option<Essence> {
        Some(Essence { id: text(v, "id")?, format: text(v, "format").unwrap_or_default(), source_id: text(v, "source_id"), media: peers::media_of(v) })
    }
}

impl Sender {
    fn parse(v: &Value) -> Option<Sender> {
        let (_, active, _) = subscription(v);
        Some(Sender {
            id: text(v, "id")?,
            version: text(v, "version").unwrap_or_default(),
            label: text(v, "label").unwrap_or_default(),
            flow_id: text(v, "flow_id"),
            device_id: text(v, "device_id").unwrap_or_default(),
            transport: text(v, "transport").unwrap_or_default(),
            manifest_href: text(v, "manifest_href"),
            active,
            stale: false,
        })
    }
}

impl Receiver {
    fn parse(v: &Value) -> Option<Receiver> {
        let (sender_id, active, _) = subscription(v);
        Some(Receiver {
            id: text(v, "id")?,
            version: text(v, "version").unwrap_or_default(),
            label: text(v, "label").unwrap_or_default(),
            device_id: text(v, "device_id").unwrap_or_default(),
            format: text(v, "format").unwrap_or_default(),
            sender_id,
            active,
            legs: Vec::new(),
            ifaces: Vec::new(),
        })
    }
}

// ── HTTP (GET only) ──

/// One answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub status: u16,
    /// `Link` header.
    pub link: Option<String>,
    /// `X-Paging-Until`, `X-Paging-Limit`.
    pub until: Option<String>,
    pub limit: Option<usize>,
    pub body: String,
}

/// A GET: (url, largest body) → answer. Real one: [`http_get`]; tests give their own.
pub type Get<'a> = dyn Fn(&str, u64) -> Result<Page, String> + Sync + 'a;

pub fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(REQUEST_TIMEOUT))
        .http_status_as_error(false)
        .max_redirects(3)
        // Equipment of the local network: never through a proxy.
        .proxy(None)
        .user_agent(concat!("strata/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

fn error_text(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => "nmos: timed out".into(),
        ureq::Error::HostNotFound => "nmos: unknown host".into(),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => "nmos: tls failed".into(),
        ureq::Error::BadUri(_) | ureq::Error::Http(_) => "nmos: bad url".into(),
        ureq::Error::BodyExceedsLimit(_) => "nmos: the answer is too big".into(),
        _ => "nmos: cannot reach the registry".into(),
    }
}

/// A GET with the agent.
pub fn http_get(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Page, String> {
    let mut res = agent.get(url).header("Accept", "application/json, application/sdp;q=0.9, */*;q=0.5").call().map_err(|e| {
        tracing::debug!("nmos: GET {url}: {e}");
        error_text(&e)
    })?;
    let header = |k: &str| res.headers().get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let (link, until, lim) = (header("link"), header("x-paging-until"), header("x-paging-limit").and_then(|v| v.trim().parse().ok()));
    let status = res.status().as_u16();
    let body = res.body_mut().with_config().limit(limit).read_to_string().map_err(|e| error_text(&e))?;
    Ok(Page { status, link, until, limit: lim, body })
}

/// The URL of `rel` in a `Link` header (`<url>; rel="next", <url>; rel="prev"`).
pub fn link_rel(header: &str, rel: &str) -> Option<String> {
    let mut rest = header;
    while let Some(start) = rest.find('<') {
        let end = start + rest[start..].find('>')?;
        let url = &rest[start + 1..end];
        let after = &rest[end + 1..];
        let params = &after[..after.find('<').unwrap_or(after.len())];
        let wanted = params.split(';').filter_map(|p| p.trim().strip_prefix("rel=")).any(|r| r.trim().trim_end_matches(',').trim().trim_matches('"').split_whitespace().any(|x| x.eq_ignore_ascii_case(rel)));
        if wanted {
            return Some(url.trim().to_string());
        }
        rest = after;
    }
    None
}

/// A URL of a `Link` header against the URL it came from.
fn resolve(base_url: &str, link: &str) -> Option<String> {
    if link.starts_with("http://") || link.starts_with("https://") {
        return Some(link.to_string());
    }
    let (scheme, rest) = base_url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    if link.starts_with('/') {
        return Some(format!("{scheme}://{host}{link}"));
    }
    let path = base_url.split(['?', '#']).next()?;
    if link.starts_with('?') {
        return Some(format!("{path}{link}"));
    }
    let dir = &path[..path.rfind('/')? + 1];
    Some(format!("{dir}{link}"))
}

/// Every item of a resource of the Query API, page after page: from the oldest
/// (`paging.since=0:0`), following `Link: rel="next"` (else `X-Paging-Until`) until a
/// page brings nothing new. A registry that does not page answers everything at once.
pub fn read_all(get: &Get, base: &str, res: &str) -> Result<Vec<Value>, String> {
    let plain = format!("{base}/{res}");
    let mut url = format!("{plain}?paging.since=0:0&paging.limit={PAGE_LIMIT}");
    let mut first = true;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for _ in 0..MAX_PAGES {
        let page = get(&url, JSON_LIMIT)?;
        if first && page.status == 400 {
            // Paging not understood: everything in one answer.
            first = false;
            url = plain.clone();
            continue;
        }
        first = false;
        if !(200..300).contains(&page.status) {
            return Err(format!("nmos: the registry answered {}", page.status));
        }
        let items = match serde_json::from_str::<Value>(&page.body) {
            Ok(Value::Array(a)) => a,
            _ => return Err("nmos: not a query api".into()),
        };
        let count = items.len();
        let mut new = 0;
        for it in items {
            // Items without an id are not resources.
            if let Some(id) = text(&it, "id") {
                if seen.insert(id) {
                    out.push(it);
                    new += 1;
                }
            }
        }
        if count == 0 || new == 0 || page.limit.is_some_and(|l| count < l) {
            break;
        }
        let limit = page.limit.unwrap_or(PAGE_LIMIT);
        let next = page
            .link
            .as_deref()
            .and_then(|l| link_rel(l, "next"))
            .and_then(|l| resolve(&url, &l))
            .or_else(|| page.until.as_ref().map(|u| format!("{plain}?paging.since={}&paging.limit={limit}", u.trim())));
        match next {
            Some(n) if n != url => url = n,
            _ => break,
        }
    }
    Ok(out)
}

/// Reads one SDP.
fn read_sdp(get: &Get, href: &str, version: &str, now: i64) -> SdpEntry {
    let mut e = SdpEntry { version: version.to_string(), href: href.to_string(), at: now, ..SdpEntry::default() };
    match get(href, SDP_LIMIT) {
        Ok(p) if (200..300).contains(&p.status) => {
            e.medias = parse_sdp(&p.body);
            if e.medias.is_empty() {
                e.error = Some("sdp: no multicast media".into());
            }
            e.text = Some(p.body);
        }
        Ok(p) => e.error = Some(format!("sdp: the node answered {}", p.status)),
        Err(x) => e.error = Some(x),
    }
    e
}

/// The SDP of every sender: kept from `prev` while the sender's version and
/// `manifest_href` stay the same, else read again (at most [`SDP_WORKERS`] at once).
pub fn read_sdps(senders: &[Sender], prev: Option<&HashMap<String, SdpEntry>>, get: &Get, now: i64) -> HashMap<String, SdpEntry> {
    read_sdps_with(senders, prev, get, now, SDP_WORKERS)
}

/// [`read_sdps`] with `workers` reads at once.
pub fn read_sdps_with(senders: &[Sender], prev: Option<&HashMap<String, SdpEntry>>, get: &Get, now: i64, workers: usize) -> HashMap<String, SdpEntry> {
    let mut out = HashMap::new();
    let mut todo: Vec<(String, String, String)> = Vec::new();
    for s in senders {
        let Some(href) = s.manifest_href.clone().filter(|h| h.starts_with("http://") || h.starts_with("https://")) else { continue };
        match prev.and_then(|p| p.get(&s.id)) {
            Some(e) if e.version == s.version && e.href == href && (e.error.is_none() || now - e.at < SDP_RETRY_MS) => {
                out.insert(s.id.clone(), e.clone());
            }
            _ => todo.push((s.id.clone(), s.version.clone(), href)),
        }
    }
    if todo.is_empty() {
        return out;
    }
    let workers = workers.clamp(1, todo.len());
    let queue = Mutex::new(todo);
    let done: Mutex<Vec<(String, SdpEntry)>> = Mutex::new(Vec::new());
    std::thread::scope(|sc| {
        for _ in 0..workers {
            sc.spawn(|| loop {
                let job = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pop();
                let Some((id, version, href)) = job else { break };
                let e = read_sdp(get, &href, &version, now);
                done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((id, e));
            });
        }
    });
    out.extend(done.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner));
    out
}

/// Reads the registry: every resource, then the SDPs that changed.
pub fn read(base: &str, prev: Option<&Registry>, get: &Get, now: i64) -> Result<Registry, String> {
    let list = |res: &str| read_all(get, base, res);
    let nodes: Vec<Node> = list("nodes")?.iter().filter_map(Node::parse).collect();
    let devices: Vec<Device> = list("devices")?.iter().filter_map(Device::parse).collect();
    let sources: Vec<Essence> = list("sources")?.iter().filter_map(Essence::parse).collect();
    let flows: Vec<Essence> = list("flows")?.iter().filter_map(Essence::parse).collect();
    let senders: Vec<Sender> = list("senders")?.iter().filter_map(Sender::parse).collect();
    let receivers: Vec<Receiver> = list("receivers")?.iter().filter_map(Receiver::parse).collect();
    let sdp = read_sdps(&senders, prev.map(|p| &p.sdp), get, now);
    let mut r = Registry { at: now, nodes, devices, sources, flows, senders, receivers, sdp, ..Registry::default() };
    for n in &r.nodes {
        r.seen.insert(n.id.clone(), now);
    }
    if let Some(p) = prev {
        for n in p.nodes.iter().chain(p.gone.iter()) {
            if r.seen.contains_key(&n.id) {
                continue;
            }
            let seen = p.seen.get(&n.id).copied().unwrap_or(p.at);
            if now - seen < NODE_KEPT_MS {
                r.seen.insert(n.id.clone(), seen);
                r.gone.push(n.clone());
            }
        }
    }
    Ok(r)
}

/// Counts of a test read: nodes, devices, senders, receivers.
pub fn probe(base: &str, get: &Get) -> Result<[usize; 4], String> {
    let n = |res: &str| read_all(get, base, res).map(|v| v.len());
    Ok([n("nodes")?, n("devices")?, n("senders")?, n("receivers")?])
}

// ── Registries found by mDNS ──

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    /// The mDNS instance name.
    pub name: String,
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<Ipv4Addr>,
    pub port: u16,
    /// The Query API to use: `http://10.1.1.20:8010/x-nmos/query/v1.3`.
    pub url: String,
    /// TXT `pri` (lower first).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    /// TXT `api_proto`: http or https.
    pub proto: String,
    /// TXT `api_ver`.
    pub versions: Vec<String>,
}

/// The Query APIs among the mDNS services, preferred first (`pri`, then name).
pub fn found(services: &[Service]) -> Vec<Found> {
    let mut out: Vec<Found> = services
        .iter()
        .filter(|s| s.kind == QUERY_KIND && s.port != 0)
        .map(|s| {
            let txt = |k: &str| s.txt.iter().find_map(|t| t.split_once('=').filter(|(a, _)| a.trim().eq_ignore_ascii_case(k)).map(|(_, v)| v.trim().to_string()));
            let proto = txt("api_proto").filter(|p| p.eq_ignore_ascii_case("https")).map(|_| "https").unwrap_or("http").to_string();
            let versions: Vec<String> = txt("api_ver").map(|v| v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
            let best = ["v1.3", "v1.2"].into_iter().find(|v| versions.is_empty() || versions.iter().any(|x| x == v)).unwrap_or("v1.3");
            let host = s.ip.map(|ip| ip.to_string()).unwrap_or_else(|| s.host.clone());
            Found { name: s.name.clone(), host: s.host.clone(), ip: s.ip, port: s.port, url: format!("{proto}://{host}:{}/x-nmos/query/{best}", s.port), priority: txt("pri").and_then(|p| p.parse().ok()), proto, versions }
        })
        .collect();
    out.sort_by(|a, b| (a.priority.unwrap_or(i64::MAX), &a.name).cmp(&(b.priority.unwrap_or(i64::MAX), &b.name)));
    out.dedup_by(|a, b| a.url == b.url);
    out
}

/// The registry found by mDNS that a Query API base is.
fn found_for<'a>(list: &'a [Found], base: &str) -> Option<&'a Found> {
    let (host, port) = host_port(base)?;
    list.iter().find(|f| f.port == port && (f.ip.is_some_and(|ip| ip.to_string() == host) || f.host.trim_end_matches('.').eq_ignore_ascii_case(&host)))
}

// ── State and poller ──

#[derive(Default)]
struct Reading {
    /// The base the registry was read from.
    base: String,
    registry: Option<Arc<Registry>>,
    /// Last try (unix ms) and its error.
    tried: i64,
    error: Option<String>,
    fingerprint: u64,
}

pub struct Nmos {
    pub settings: Mutex<Settings>,
    reading: Mutex<Reading>,
    wake: Notify,
    /// Nodes mode: discovery and polling.
    pub peers: Mutex<peers::Peers>,
}

impl Nmos {
    pub fn load(dir: &std::path::Path) -> Nmos {
        let settings = match read_json::<Settings>(&dir.join(FILE)) {
            Ok(Some(mut v)) => {
                v.migrate();
                v
            }
            Ok(None) => Settings::default(),
            Err(e) => {
                tracing::error!("cannot read {FILE}: {e}; nmos off");
                Settings::default()
            }
        };
        Nmos { settings: Mutex::new(settings), reading: Mutex::default(), wake: Notify::new(), peers: Mutex::default() }
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// The last good read (kept while the registry does not answer).
    pub fn registry(&self) -> Option<Arc<Registry>> {
        self.reading.lock().unwrap_or_else(std::sync::PoisonError::into_inner).registry.clone()
    }

    pub fn sdp_text(&self, group: Ipv4Addr, port: u16) -> Option<String> {
        self.registry()?.sdp_text(group, port)
    }

    /// Reads now (settings changed).
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Forgets what was read; whether there was something.
    fn clear(&self) -> bool {
        let mut r = self.reading.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let had = r.registry.is_some() || r.error.is_some();
        if r.registry.is_some() {
            new_generation();
        }
        *r = Reading::default();
        had
    }

    /// Keeps a read; whether the flows change (fingerprint).
    fn store(&self, base: &str, got: Result<Registry, String>, tried: i64) -> bool {
        let mut r = self.reading.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if r.base != base {
            // Another registry: what the old one said goes.
            if r.registry.is_some() {
                new_generation();
            }
            *r = Reading { base: base.to_string(), ..Reading::default() };
        }
        r.tried = tried;
        match got {
            Ok(reg) => {
                let fp = reg.fingerprint();
                let changed = r.registry.is_none() || fp != r.fingerprint;
                r.fingerprint = fp;
                r.registry = Some(Arc::new(reg));
                r.error = None;
                changed
            }
            Err(e) => {
                if r.error.as_ref() != Some(&e) {
                    tracing::warn!("nmos: {base}: {e}");
                }
                r.error = Some(e);
                false
            }
        }
    }
}

/// The flows of a registry dropped (turned off, another address) are not "gone"
/// streams: the multicast alerts start again from what they see.
fn new_generation() {
    super::mflows::GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Reads the registry while NMOS is on: every 5 s or 60 s, at once when the settings change.
pub fn spawn(net: Arc<Net>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let mut poller = peers::Poller::default();
        loop {
            let s = net.nmos.settings();
            if s.enabled && s.nodes_mode() {
                poller.tick(&net, &s).await;
                continue;
            }
            poller.stop(&net);
            let Some(base) = s.base() else {
                if net.nmos.clear() {
                    net.changed("nmos");
                    net.changed("multicast");
                }
                net.nmos.wake.notified().await;
                continue;
            };
            let prev = net.nmos.registry().filter(|_| net.nmos.reading.lock().unwrap_or_else(std::sync::PoisonError::into_inner).base == base);
            let b = base.clone();
            let tried = now_ms();
            let got = tokio::task::spawn_blocking(move || {
                let agent = agent();
                read(&b, prev.as_deref(), &|u: &str, limit: u64| http_get(&agent, u, limit), now_ms())
            })
            .await
            .unwrap_or_else(|e| Err(format!("nmos: {e}")));
            // Settings changed during the read: it is not used.
            if net.nmos.settings().base().as_deref() == Some(base.as_str()) {
                if net.nmos.store(&base, got, tried) {
                    net.changed("multicast");
                }
                net.changed("nmos");
            }
            tokio::select! {
                _ = tokio::time::sleep(s.interval()) => {}
                _ = net.nmos.wake.notified() => {}
            }
        }
    });
}

// ── Views ──

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusView {
    pub enabled: bool,
    /// "registry" or "nodes".
    pub source: String,
    /// The Query API read ('' when off or not set).
    pub url: String,
    pub version: String,
    pub refresh: String,
    /// The registry's mDNS name, else its host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    /// Another registry found by mDNS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
    /// Last good read (unix ms; 0 = none), last try, and its error.
    pub at: i64,
    pub tried: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub nodes: usize,
    pub devices: usize,
    pub senders: usize,
    pub video: usize,
    pub receivers: usize,
    pub subscribed: usize,
    /// Nodes none of whose addresses or MACs the inventory knows.
    pub unknown: usize,
    /// Senders whose SDP could not be read (not in the flows).
    pub without_sdp: usize,
    /// Nodes mode: Node APIs polled; answering ones; the others (not answering,
    /// or never answered); found by mDNS, the scan, typed (one may be several);
    /// the Node API versions read; the local interfaces mDNS asks on.
    pub polled: usize,
    pub answering: usize,
    pub unreachable: usize,
    pub mdns: usize,
    pub scan: usize,
    pub manual: usize,
    pub apis: Vec<String>,
    pub ifaces: Vec<Ipv4Addr>,
}

impl Net {
    pub fn nmos_status(&self) -> StatusView {
        let s = self.nmos.settings();
        let base = s.base().unwrap_or_default();
        let (reg, at, tried, error) = {
            let r = self.nmos.reading.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            (r.registry.clone(), r.registry.as_ref().map(|x| x.at).unwrap_or(0), r.tried, r.error.clone())
        };
        let list = found(&self.services.lock().unwrap_or_else(std::sync::PoisonError::into_inner).1);
        let mine = found_for(&list, &base);
        let nodes_mode = s.nodes_mode();
        let mut v = StatusView {
            enabled: s.enabled,
            source: s.source.clone(),
            registry: (!base.is_empty()).then(|| mine.map(|f| f.name.clone()).or_else(|| host_port(&base).map(|(h, _)| h))).flatten(),
            backup: (!base.is_empty()).then(|| list.iter().find(|f| Some(*f) != mine).map(|f| f.name.clone())).flatten(),
            url: base,
            version: s.version,
            refresh: s.refresh,
            at,
            tried,
            error,
            ..StatusView::default()
        };
        if let Some(reg) = reg {
            v.nodes = reg.nodes.len();
            v.devices = reg.devices.len();
            v.senders = reg.senders.len();
            v.video = reg.senders.iter().filter(|x| reg.is_video(x)).count();
            v.receivers = reg.receivers.len();
            v.subscribed = reg.subscribed();
            v.without_sdp = reg.senders.iter().filter(|x| reg.medias(&x.id).is_empty()).count();
            let inp = self.flows_input();
            v.unknown = super::mflows::registry::nodes(&inp).iter().filter(|n| n.present && !n.inventory).count();
        }
        if nodes_mode {
            let p = self.nmos.peers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            v.polled = p.nodes.len();
            v.answering = p.nodes.values().filter(|x| x.data.is_some() && x.misses == 0).count();
            v.unreachable = v.polled - v.answering;
            let by = |f: peers::FoundBy| p.nodes.values().filter(|x| x.info.found.contains(&f)).count();
            v.mdns = by(peers::FoundBy::Mdns);
            v.scan = by(peers::FoundBy::Scan);
            v.manual = by(peers::FoundBy::Manual);
            let mut apis: Vec<String> = p.nodes.values().filter_map(|x| x.info.api.clone()).collect();
            apis.sort();
            apis.dedup();
            v.apis = apis;
            v.ifaces.clone_from(&p.ifaces);
            v.at = p.nodes.values().map(|x| x.info.last_ok).max().unwrap_or(0);
            v.tried = p.nodes.values().map(|x| x.info.tried).max().unwrap_or(v.tried);
            // An error only when no node answers.
            v.error = if v.answering == 0 { p.nodes.values().find_map(|x| x.info.error.clone()) } else { None };
        }
        v
    }
}

// ── Routes ──

pub fn router() -> Router<Shared> {
    Router::new()
        .route("/api/nmos", get(get_settings).put(put_settings))
        .route("/api/nmos/test", post(test_route))
        .route("/api/nmos/discovered", get(discovered))
        .route("/api/nmos/status", get(status_route))
        .route("/api/nmos/nodes", get(nodes_route))
}

fn who(me: &Session) -> String {
    if me.user.display_name.is_empty() {
        me.user.username.clone()
    } else {
        me.user.display_name.clone()
    }
}

async fn get_settings(State(s): State<Shared>, me: Session) -> ApiResult<Json<Settings>> {
    require(me.user.is_admin())?;
    Ok(Json(s.net.nmos.settings()))
}

async fn put_settings(State(s): State<Shared>, me: Session, Json(mut b): Json<Settings>) -> ApiResult<Json<Settings>> {
    require(me.user.is_admin())?;
    b.check().map_err(bad)?;
    let (old, bytes) = {
        let mut cur = s.net.nmos.settings.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let old = std::mem::replace(&mut *cur, b.clone());
        (old, serde_json::to_vec_pretty(&b))
    };
    match bytes {
        Ok(bytes) => s.net.write_file(FILE, bytes),
        Err(e) => tracing::error!("cannot serialize {FILE}: {e}"),
    }
    s.net.nmos.wake();
    s.net.changed("nmos");
    if old != b {
        s.net.log(&who(&me), "changed nmos settings", "", &b.summary());
    }
    Ok(Json(b))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestBody {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    node_urls: Option<Vec<String>>,
    #[serde(default)]
    ranges: Option<Vec<String>>,
}

/// Nodes mode test: asks mDNS on the interfaces of the ranges, then reads each
/// Node API heard and each one typed (version, `self`, senders, receivers).
async fn test_nodes(s: &Shared, b: TestBody) -> ApiResult<Json<Value>> {
    let mut set = s.net.nmos.settings();
    set.source = "nodes".into();
    if let Some(u) = b.node_urls {
        set.node_urls = u;
    }
    if let Some(r) = b.ranges {
        set.ranges = r;
    }
    set.check().map_err(bad)?;
    let started = Instant::now();
    let scope = s.net.nmos_scope(&set);
    let heard = peers::heard(&peers::browse_nodes(&scope.ifaces).await);
    {
        let now = now_ms();
        let mut p = s.net.nmos.peers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for (b, name) in &heard {
            p.mdns.insert(b.clone(), peers::Heard { name: name.clone(), seen: now });
        }
    }
    let mut jobs: Vec<(String, Vec<peers::FoundBy>)> = heard.iter().map(|(b, _)| (b.clone(), vec![peers::FoundBy::Mdns])).collect();
    for u in &set.node_urls {
        match jobs.iter_mut().find(|j| &j.0 == u) {
            Some(j) => j.1.push(peers::FoundBy::Manual),
            None => jobs.push((u.clone(), vec![peers::FoundBy::Manual])),
        }
    }
    let mut nodes = tokio::task::spawn_blocking(move || {
        let agent = peers::node_agent(Duration::from_secs(2), Duration::from_secs(5));
        let get = |u: &str, limit: u64| http_get(&agent, u, limit);
        peers::par(jobs, peers::POLL_WORKERS, &|(base, found): (String, Vec<peers::FoundBy>)| peers::check_node(&base, found, &get))
    })
    .await
    .unwrap_or_default();
    nodes.sort_by(|a, b| a.url.cmp(&b.url));
    s.net.nmos.wake();
    Ok(Json(json!({ "ifaces": scope.ifaces, "mdns": heard.len(), "nodes": nodes, "ms": started.elapsed().as_millis() as u64 })))
}

/// Reads the registry once (the address sent, else the saved one): counts, or why not.
async fn test_route(State(s): State<Shared>, me: Session, Json(b): Json<TestBody>) -> ApiResult<Json<Value>> {
    require(me.user.is_admin())?;
    let saved = s.net.nmos.settings();
    if b.source.as_deref().unwrap_or(&saved.source) == "nodes" {
        return test_nodes(&s, b).await;
    }
    let url = b.url.unwrap_or(saved.url);
    let version = b.version.unwrap_or(saved.version);
    if !matches!(version.as_str(), "v1.2" | "v1.3") {
        return Err(bad("the nmos version is v1.2 or v1.3"));
    }
    if url.trim().is_empty() {
        return Err(bad("the registry address cannot be empty"));
    }
    let base = query_base(&url, &version).map_err(bad)?;
    let started = Instant::now();
    let b2 = base.clone();
    let r = tokio::task::spawn_blocking(move || {
        let agent = agent();
        probe(&b2, &|u: &str, limit: u64| http_get(&agent, u, limit))
    })
    .await
    .unwrap_or_else(|e| Err(format!("nmos: {e}")));
    match r {
        Ok([nodes, devices, senders, receivers]) => Ok(Json(json!({ "url": base, "nodes": nodes, "devices": devices, "senders": senders, "receivers": receivers, "ms": started.elapsed().as_millis() as u64 }))),
        Err(e) => Err(status(502, e)),
    }
}

/// The Query APIs announced by mDNS (`_nmos-query._tcp`); asks the network again when the answers are old.
async fn discovered(State(s): State<Shared>, me: Session) -> ApiResult<Json<Value>> {
    require(me.user.is_admin())?;
    let at = s.net.services.lock().unwrap_or_else(std::sync::PoisonError::into_inner).0;
    if now_ms() - at > BROWSE_AGE_MS {
        s.net.browse().await;
    }
    let (at, list) = {
        let g = s.net.services.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        (g.0, found(&g.1))
    };
    Ok(Json(json!({ "at": at, "registries": list })))
}

async fn status_route(State(s): State<Shared>, _me: Session) -> Json<StatusView> {
    let n = s.net.clone();
    Json(tokio::task::spawn_blocking(move || n.nmos_status()).await.unwrap_or_default())
}

async fn nodes_route(State(s): State<Shared>, _me: Session) -> Json<Vec<super::mflows::registry::NodeView>> {
    let n = s.net.clone();
    Json(tokio::task::spawn_blocking(move || super::mflows::registry::nodes(&n.flows_input())).await.unwrap_or_default())
}

#[cfg(test)]
mod tests;
