//! Multicast flows (`docs/api-network.md` § Multicast flows): every stream with
//! its receivers (IGMP snooping of the online switches), its emitter's switch
//! port, the path between them over the switch links, its ST 2022-7 pair; the
//! group plan; the PTP clocks; the multicast alerts. Read only: everything comes
//! from what the passive engine heard and what the switches' slow reads said.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use strata_net::mcast::{FloodStatus, Kind, Stream};
use strata_net::mformat::Format;
use strata_net::ptp;
use strata_net::switchinfo::group_mac;

use super::info::Ifs;
use super::model::{PortRef, SwitchPort};
use super::{File, Net};
use crate::api::{bad, not_found, require, status, ApiResult, Session};
use crate::paths::{new_id, now_ms};
use crate::state::Shared;

// ── Stored in multicast.json ──

/// What multicast.json keeps besides the interface, the check limit and the stream labels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct McastMore {
    /// Names given to groups (unannounced ones), by group address.
    pub group_labels: BTreeMap<Ipv4Addr, String>,
    pub ranges: Vec<Range>,
    pub reservations: Vec<Reservation>,
    pub rules: McastRules,
    /// Grandmaster changes, newest last (at most [`GM_CHANGES_KEPT`]).
    pub ptp_changes: Vec<GmChange>,
    /// The on-demand check (a 2-second join) is allowed (off by default).
    pub check_enabled: bool,
    /// Flood sampling of the interface (on by default).
    #[serde(default = "yes")]
    pub flood_watch: bool,
}

impl Default for McastMore {
    fn default() -> Self {
        McastMore {
            group_labels: BTreeMap::new(),
            ranges: Vec::new(),
            reservations: Vec::new(),
            rules: McastRules::default(),
            ptp_changes: Vec::new(),
            check_enabled: false,
            flood_watch: true,
        }
    }
}

/// The multicast options of `PUT /api/multicast/options`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McastOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flood_watch: Option<bool>,
}

/// A range of the group plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    #[serde(default)]
    pub id: String,
    /// "239.10.1.0/24".
    pub cidr: String,
    #[serde(default)]
    pub usage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
}

/// A group kept for a future stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reservation {
    pub group: Ipv4Addr,
    #[serde(default)]
    pub name: String,
}

fn yes() -> bool {
    true
}

/// The multicast alert rules (all on by default).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McastRules {
    /// An announced stream not seen any more.
    #[serde(default = "yes")]
    pub stream_gone: bool,
    /// A stream that had receivers has none any more.
    #[serde(default = "yes")]
    pub no_receivers: bool,
    /// One group sent by two sources.
    #[serde(default = "yes")]
    pub conflict: bool,
    /// Media of groups not joined reach this machine.
    #[serde(default = "yes")]
    pub flooding: bool,
    /// One leg of an ST 2022-7 pair missing.
    #[serde(default = "yes")]
    pub pair_missing: bool,
    #[serde(default = "yes")]
    pub gm_change: bool,
    /// A PTP device in another domain than the grandmaster's.
    #[serde(default = "yes")]
    pub ptp_domain: bool,
}

impl Default for McastRules {
    fn default() -> Self {
        McastRules { stream_gone: true, no_receivers: true, conflict: true, flooding: true, pair_missing: true, gm_change: true, ptp_domain: true }
    }
}

/// A change of grandmaster in one domain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmChange {
    pub at: i64,
    pub domain: u8,
    /// Clock identities; `None`: no grandmaster.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_ip: Option<Ipv4Addr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_ip: Option<Ipv4Addr>,
}

pub const GM_CHANGES_KEPT: usize = 50;

// ── Timing ──

/// An announced stream is gone when neither an announcement nor a packet came for this long
/// (or 3 announcement intervals when the source announces less often).
pub const GONE_MS: u64 = 30_000;
/// Packets received by a check count for this long.
const TRAFFIC_MS: u64 = 60_000;
/// A flood sample counts until two samples were missed.
const FLOOD_MS: u64 = 11 * 60_000;
/// Routers' tables are read every 5 minutes.
const ROUTED_MS: u64 = 11 * 60_000;

// ── What the computation reads ──

/// A port as a snooping table names it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortAt {
    pub n: Option<u16>,
    pub name: String,
}

/// One IGMP snooping entry of a switch.
#[derive(Clone, Debug, PartialEq)]
pub struct Snoop {
    pub vlan: u16,
    pub group: Option<Ipv4Addr>,
    pub mac: [u8; 6],
    pub ports: Vec<PortAt>,
}

/// A switch of the inventory, with what its slow reads said when it is online.
#[derive(Clone, Debug, Default)]
pub struct SwitchData {
    pub id: String,
    pub name: String,
    pub ip: Option<Ipv4Addr>,
    pub ports: Vec<SwitchPort>,
    pub online: bool,
    /// Its snooping tables were read.
    pub read: bool,
    pub at: i64,
    pub snoop: Vec<Snoop>,
    pub router: Vec<(u16, Vec<PortAt>)>,
    /// Faceplate member → aggregate name.
    pub lag_of: HashMap<u16, String>,
}

/// An address of the inventory.
#[derive(Clone, Debug, Default)]
pub struct Addr {
    pub name: Option<String>,
    pub mac: Option<String>,
    pub switch: Option<PortRef>,
}

#[derive(Default)]
pub struct Input {
    pub now: u64,
    pub streams: Vec<Stream>,
    /// Stream labels by "group:port:source".
    pub labels: BTreeMap<String, String>,
    pub more: McastMore,
    pub switches: Vec<SwitchData>,
    pub addresses: HashMap<Ipv4Addr, Addr>,
    /// Router ARP tables: ip → MAC.
    pub arp: HashMap<Ipv4Addr, String>,
    /// VLAN subnets: (vlan, network, prefix).
    pub vlan_nets: Vec<(u16, Ipv4Addr, u8)>,
    pub ptp: ptp::Snapshot,
    pub flood: FloodStatus,
    pub iface_ip: Option<Ipv4Addr>,
    pub check_max_mbps: u64,
}

// ── What the clients get ──

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attach {
    pub switch_id: String,
    pub switch_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u16>,
    /// Interface name.
    pub port: String,
    /// An edge port (not a switch link, an aggregate or a trunk).
    pub edge: bool,
    /// "inventory" (set on the address) or "macTable".
    pub via: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Emitter {
    pub ip: Ipv4Addr,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<Attach>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receiver {
    pub switch_id: String,
    pub switch_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u16>,
    pub port: String,
    pub vlan: u16,
    /// The port's label in the inventory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The machine plugged in (inventory name, else IP).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_ip: Option<Ipv4Addr>,
    /// A multicast router port.
    pub router: bool,
    /// The port leads to this other switch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    /// The aggregate the port belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agg: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathNode {
    pub switch_id: String,
    pub switch_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The parent's port (or aggregate) towards this switch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub egress: Option<String>,
    /// This switch's port towards the parent (for the first switch: the emitter's port).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress: Option<String>,
    /// Snooping ports of this group on this switch, and the receivers among them.
    pub igmp_ports: usize,
    pub receivers: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRef {
    pub key: String,
    pub group: Ipv4Addr,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Ipv4Addr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<Attach>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Flow {
    /// "group:port:source" (port 0 and no source for a group known from snooping only).
    pub key: String,
    pub group: Ipv4Addr,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Ipv4Addr>,
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// Label of the stream, else of the group, else the SAP name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Label set by hand on the stream ('' = none).
    pub label: String,
    /// Name set by hand on the group ('' = none).
    pub group_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sap_name: Option<String>,
    pub announced: bool,
    pub routed: bool,
    /// Received here without being joined (last flood sample within a minute).
    pub flooded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flood_vlan: Option<u16>,
    /// Known only from switches' snooping tables.
    pub snoop_only: bool,
    /// The group was not read from the switch: one of the 32 groups of its MAC, guessed.
    pub guessed: bool,
    /// The snooping MAC matches several known groups (all are given the receivers).
    pub ambiguous: bool,
    pub live: bool,
    /// ok, warn (an issue), unk (not announced), off (not live).
    pub state: &'static str,
    /// unannounced, noReceivers, pairMissing, conflict, flooded, gone.
    pub issues: Vec<&'static str>,
    pub bps: f64,
    pub pps: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub est_bps: Option<f64>,
    pub history: Vec<f32>,
    pub first_seen: u64,
    pub last_seen: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_announced: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sap_interval_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emitter: Option<Emitter>,
    /// Every source of the group when there are several (conflict).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Ipv4Addr>,
    pub conflict: bool,
    pub receivers: Vec<Receiver>,
    /// Receivers that are edge ports (not router ports, not switch links).
    pub receiver_count: usize,
    pub path: Vec<PathNode>,
    /// Receiving switches no link chain reaches from the emitter's switch.
    pub unreachable: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair: Option<PairRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair_state: Option<&'static str>,
    /// "sdp" (a=group:DUP) or "name".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair_by: Option<&'static str>,
    /// First (Red, A, 1) or second (Blue, B, 2) leg.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leg: Option<&'static str>,
    /// The other leg named by the SDP.
    #[serde(skip)]
    pub dup: Option<strata_net::mcast::Leg>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub flows: usize,
    pub live: usize,
    pub by_kind: BTreeMap<String, usize>,
    pub est_bps: f64,
    pub bps: f64,
    pub receivers: usize,
    pub pairs: usize,
    pub pairs_incomplete: usize,
    pub unannounced: usize,
    pub no_receivers: usize,
    pub conflicts: usize,
    pub flooded: usize,
    pub switches_online: usize,
    pub switches_read: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopoSwitch {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<Ipv4Addr>,
    pub online: bool,
    pub read: bool,
    /// Last slow read (unix ms; 0 before the first one).
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopoLink {
    pub a: String,
    pub b: String,
    /// Port or aggregate names on each side.
    pub a_port: String,
    pub b_port: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupUse {
    pub group: Ipv4Addr,
    /// used, reserved, conflict.
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeView {
    #[serde(flatten)]
    pub range: Range,
    pub first: Ipv4Addr,
    pub last: Ipv4Addr,
    pub size: u64,
    pub used: usize,
    pub reserved: usize,
    pub conflicts: usize,
    pub free: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_free: Option<Ipv4Addr>,
    /// The groups not free, in order.
    pub groups: Vec<GroupUse>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outside {
    pub used: usize,
    pub reserved: usize,
    pub conflicts: usize,
    pub groups: Vec<GroupUse>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub ranges: Vec<RangeView>,
    pub outside: Outside,
    pub reservations: Vec<Reservation>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtpClock {
    #[serde(flatten)]
    pub clock: ptp::Announcer,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// gm (best of its domain), backup, boundary (relays a grandmaster).
    pub role: &'static str,
    pub live: bool,
    pub wrong_domain: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<Attach>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtpDevice {
    #[serde(flatten)]
    pub device: ptp::Follower,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    pub live: bool,
    pub wrong_domain: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<Attach>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtpView {
    /// The best clock heard (all domains), its domain is the main one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gm: Option<PtpClock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<u8>,
    /// Best first.
    pub clocks: Vec<PtpClock>,
    pub devices: Vec<PtpDevice>,
    /// Newest first.
    pub changes: Vec<GmChange>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FloodView {
    #[serde(flatten)]
    pub status: FloodStatus,
    /// The VLAN of the interface (802.1Q tag of the frames, else the VLAN whose subnet holds its address).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowsView {
    pub at: u64,
    pub flows: Vec<Flow>,
    pub summary: Summary,
    pub switches: Vec<TopoSwitch>,
    pub links: Vec<TopoLink>,
    pub plan: Plan,
    pub ptp: PtpView,
    pub flood: FloodView,
    pub rules: McastRules,
    /// The on-demand check: allowed, and its limit (Mb/s).
    pub check: CheckView,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckView {
    pub enabled: bool,
    pub max_mbps: u64,
}

// ── Helpers ──

/// Discovery and control groups, not flows: link-local, PTP, SAP, SSDP.
pub fn control_group(g: Ipv4Addr) -> bool {
    let o = g.octets();
    (o[0] == 224 && o[1] == 0 && o[2] == 0)
        || (o[0] == 224 && o[1] == 0 && o[2] == 1 && (129..=132).contains(&o[3]))
        || strata_net::mcast::SAP_GROUPS.contains(&g)
        || g == Ipv4Addr::new(239, 255, 255, 250)
}

/// The 32 groups of a multicast MAC.
pub fn mac_groups(mac: [u8; 6]) -> Vec<Ipv4Addr> {
    let mut v = Vec::with_capacity(32);
    for first in 224..=239u8 {
        for high in [0u8, 0x80] {
            v.push(Ipv4Addr::new(first, mac[3] | high, mac[4], mac[5]));
        }
    }
    v
}

fn block(cidr: &str) -> Option<(Ipv4Addr, Ipv4Addr)> {
    let (net, p) = strata_net::cidr::parse_block(cidr).ok()?;
    let size = 1u64 << (32 - u32::from(p));
    Some((net, Ipv4Addr::from((u64::from(u32::from(net)) + size - 1) as u32)))
}

fn in_range(r: &Range, g: Ipv4Addr) -> bool {
    block(&r.cidr).is_some_and(|(a, b)| (u32::from(a)..=u32::from(b)).contains(&u32::from(g)))
}

/// Stream key, as the labels use it.
pub fn stream_key(group: Ipv4Addr, port: u16, source: Option<Ipv4Addr>) -> String {
    format!("{group}:{port}:{}", source.map(|s| s.to_string()).unwrap_or_default())
}

fn seen_at(s: &Stream) -> u64 {
    s.last_seen.max(s.last_announced.unwrap_or(0))
}

/// Something says the stream is still sent: a recent announcement, packets, a router's table.
pub fn is_live(s: &Stream, now: u64) -> bool {
    let age = |t: u64| now.saturating_sub(t);
    let gone_after = GONE_MS.max(s.sap_interval_ms.unwrap_or(0).saturating_mul(3));
    s.last_announced.is_some_and(|t| age(t) <= gone_after)
        || (s.packets > 0 && age(s.last_seen) <= TRAFFIC_MS)
        || (s.flooded > 0 && age(s.flooded) <= FLOOD_MS)
        || (s.routed && age(s.last_seen) <= ROUTED_MS)
}

/// Words that name a 2022-7 leg, and which leg: 'a' (first) or 'b' (second).
fn leg_word(w: &str, last: bool) -> Option<(&'static str, &'static str)> {
    // (leg, family): a family pairs only with itself.
    match w {
        "rouge" | "red" => Some(("a", "color")),
        "bleu" | "blue" => Some(("b", "color")),
        "primary" | "pri" | "main" => Some(("a", "role")),
        "secondary" | "sec" | "backup" => Some(("b", "role")),
        "a" if last => Some(("a", "letter")),
        "b" if last => Some(("b", "letter")),
        "1" if last => Some(("a", "digit")),
        "2" if last => Some(("b", "digit")),
        _ => None,
    }
}

/// A name without its leg word: ("cam-01 video", leg, family); `None` without one.
pub fn split_leg(name: &str) -> Option<(String, &'static str, &'static str)> {
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| c.is_whitespace() || "_-.()[]/|:".contains(c)).filter(|w| !w.is_empty()).collect();
    let n = words.len();
    // The last leg word wins (colours anywhere, letters and digits only at the end).
    let (i, (leg, family)) = words.iter().enumerate().rev().find_map(|(i, w)| leg_word(w, i + 1 == n).map(|x| (i, x)))?;
    let base: Vec<&str> = words.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, w)| *w).collect();
    (!base.is_empty()).then(|| (base.join(" "), leg, family))
}

/// What must match between the two legs of a pair: kind, encoding, width, height, frame rate (mHz), channels.
type Signature = (Kind, Option<String>, Option<u32>, Option<u32>, Option<u64>, Option<u16>);

fn signature(f: &Flow) -> Signature {
    let fm = f.format.as_ref();
    (
        f.kind,
        f.encoding.as_ref().map(|e| e.to_ascii_lowercase()),
        fm.and_then(|x| x.width),
        fm.and_then(|x| x.height),
        fm.and_then(|x| x.frame_rate).map(|r| (r * 1000.0).round() as u64),
        fm.and_then(|x| x.channels),
    )
}

/// Pairs the ST 2022-7 legs: by the SDP's `a=group:DUP`, else by name
/// (same name but the leg word, same format, different group or network).
/// Returns, for each flow, (partner index, by, leg).
pub fn pair_flows(flows: &[Flow]) -> Vec<Option<(usize, &'static str, &'static str)>> {
    let mut out: Vec<Option<(usize, &'static str, &'static str)>> = vec![None; flows.len()];
    let by_target: HashMap<(Ipv4Addr, u16), Vec<usize>> = flows.iter().enumerate().fold(HashMap::new(), |mut m, (i, f)| {
        m.entry((f.group, f.port)).or_default().push(i);
        m
    });
    for (i, f) in flows.iter().enumerate() {
        let Some(d) = f.dup.as_ref() else { continue };
        let Some(cands) = by_target.get(&(d.group, d.port)) else { continue };
        let j = cands.iter().copied().find(|j| *j != i && (d.source.is_none() || flows[*j].source == d.source)).or_else(|| cands.iter().copied().find(|j| *j != i));
        if let Some(j) = j {
            // The leg: the lower group (then source) is the first.
            let leg = if (u32::from(f.group), f.source.map(u32::from)) <= (u32::from(flows[j].group), flows[j].source.map(u32::from)) { "a" } else { "b" };
            out[i] = Some((j, "sdp", leg));
        }
    }
    let names: Vec<Option<(String, &'static str, &'static str)>> = flows.iter().map(|f| f.name.as_deref().and_then(split_leg)).collect();
    for i in 0..flows.len() {
        if out[i].is_some() {
            continue;
        }
        let Some((base, leg, family)) = &names[i] else { continue };
        let cands: Vec<usize> = (0..flows.len())
            .filter(|j| *j != i && out[*j].is_none())
            .filter(|j| names[*j].as_ref().is_some_and(|(b, l, fam)| b == base && fam == family && l != leg))
            .filter(|j| signature(&flows[*j]) == signature(&flows[i]))
            .filter(|j| flows[*j].group != flows[i].group || flows[*j].vlan != flows[i].vlan || flows[*j].source != flows[i].source)
            .collect();
        let j = match cands.len() {
            0 => continue,
            1 => cands[0],
            _ => match cands.iter().copied().filter(|j| flows[*j].port == flows[i].port).collect::<Vec<_>>()[..] {
                [j] => j,
                _ => continue,
            },
        };
        out[i] = Some((j, "name", leg));
        out[j] = Some((i, "name", if *leg == "a" { "b" } else { "a" }));
    }
    out
}

// ── The computation ──

struct Ctx<'a> {
    inp: &'a Input,
    by_id: HashMap<&'a str, &'a SwitchData>,
}

impl<'a> Ctx<'a> {
    fn new(inp: &'a Input) -> Self {
        Ctx { inp, by_id: inp.switches.iter().map(|s| (s.id.as_str(), s)).collect() }
    }

    fn addr_name(&self, ip: Ipv4Addr) -> Option<String> {
        self.inp.addresses.get(&ip).and_then(|a| a.name.clone())
    }

    fn vlan_of_ip(&self, ip: Ipv4Addr) -> Option<u16> {
        self.inp.vlan_nets.iter().filter(|(_, n, p)| strata_net::cidr::in_block(ip, *n, *p)).max_by_key(|(_, _, p)| *p).map(|(v, _, _)| *v)
    }

    fn mac_of(&self, ip: Ipv4Addr) -> Option<String> {
        self.inp.addresses.get(&ip).and_then(|a| a.mac.clone()).or_else(|| self.inp.arp.get(&ip).cloned())
    }

    /// Where a machine is plugged: set on its address, else where a switch learnt its MAC (edge ports first).
    fn locate(&self, ip: Option<Ipv4Addr>, mac: Option<&str>) -> Option<Attach> {
        if let Some(r) = ip.and_then(|ip| self.inp.addresses.get(&ip)).and_then(|a| a.switch.as_ref()) {
            if let Some(sw) = self.by_id.get(r.id.as_str()) {
                let p = sw.ports.iter().find(|p| p.n == r.port);
                return Some(Attach {
                    switch_id: sw.id.clone(),
                    switch_name: sw.name.clone(),
                    n: Some(r.port),
                    port: p.map(|p| p.name.clone()).unwrap_or_default(),
                    edge: p.is_none_or(|p| p.peer.is_none()),
                    via: "inventory",
                });
            }
        }
        let mac = mac?.to_ascii_uppercase();
        let mut best: Option<((bool, bool, bool, usize), Attach)> = None;
        for sw in &self.inp.switches {
            for p in sw.ports.iter().filter(|p| p.learnt.iter().any(|m| m.eq_ignore_ascii_case(&mac))) {
                let (peer, lag, trunk) = (p.peer.is_some(), sw.lag_of.contains_key(&p.n), p.mode == "trunk");
                let rank = (peer, lag, trunk, p.learnt.len());
                if best.as_ref().is_none_or(|(r, _)| rank < *r) {
                    let at = Attach { switch_id: sw.id.clone(), switch_name: sw.name.clone(), n: Some(p.n), port: p.name.clone(), edge: !peer && !lag && !trunk, via: "macTable" };
                    best = Some((rank, at));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Ports of `a` linked to `b`: (a's port numbers, b's port numbers).
    fn link_ports(&self, a: &SwitchData, b: &SwitchData) -> (Vec<u16>, Vec<u16>) {
        let mut pa: Vec<u16> = a.ports.iter().filter(|p| p.peer.as_ref().is_some_and(|r| r.id == b.id)).map(|p| p.n).collect();
        let mut pb: Vec<u16> = b.ports.iter().filter(|p| p.peer.as_ref().is_some_and(|r| r.id == a.id)).map(|p| p.n).collect();
        // Links recorded on one side only.
        for p in b.ports.iter().filter_map(|p| p.peer.as_ref().filter(|r| r.id == a.id)) {
            if !pa.contains(&p.port) {
                pa.push(p.port);
            }
        }
        for p in a.ports.iter().filter_map(|p| p.peer.as_ref().filter(|r| r.id == b.id)) {
            if !pb.contains(&p.port) {
                pb.push(p.port);
            }
        }
        pa.sort_unstable();
        pb.sort_unstable();
        (pa, pb)
    }

    /// "port-channel1" when all are members of one aggregate, else the port names.
    fn ports_name(sw: &SwitchData, ns: &[u16]) -> String {
        let aggs: HashSet<&String> = ns.iter().filter_map(|n| sw.lag_of.get(n)).collect();
        if aggs.len() == 1 && ns.iter().all(|n| sw.lag_of.contains_key(n)) {
            return aggs.into_iter().next().cloned().unwrap_or_default();
        }
        ns.iter().map(|n| sw.ports.iter().find(|p| p.n == *n).map(|p| p.name.clone()).unwrap_or_else(|| n.to_string())).collect::<Vec<_>>().join(" · ")
    }

    fn neighbours(&self, id: &str) -> Vec<&'a str> {
        let mut v: Vec<&'a str> = Vec::new();
        let Some(sw) = self.by_id.get(id) else { return v };
        for p in &sw.ports {
            if let Some(r) = &p.peer {
                if let Some((k, _)) = self.by_id.get_key_value(r.id.as_str()) {
                    if !v.contains(k) {
                        v.push(k);
                    }
                }
            }
        }
        for o in &self.inp.switches {
            if o.ports.iter().any(|p| p.peer.as_ref().is_some_and(|r| r.id == id)) && !v.contains(&o.id.as_str()) {
                v.push(o.id.as_str());
            }
        }
        v.sort_unstable();
        v
    }

    /// Shortest link chains from `from` to each of `to` (breadth first), as one tree.
    fn path(&self, from: &str, to: &[String], ingress: Option<String>, counts: &HashMap<String, (usize, usize)>) -> (Vec<PathNode>, Vec<String>) {
        let mut parent: HashMap<&str, &str> = HashMap::new();
        let mut seen: HashSet<&str> = HashSet::from([from]);
        let mut queue = VecDeque::from([from]);
        while let Some(x) = queue.pop_front() {
            for y in self.neighbours(x) {
                if seen.insert(y) {
                    parent.insert(y, x);
                    queue.push_back(y);
                }
            }
        }
        let mut on: Vec<&str> = vec![from];
        let mut unreachable = Vec::new();
        for t in to {
            if t == from {
                continue;
            }
            if !seen.contains(t.as_str()) {
                unreachable.push(t.clone());
                continue;
            }
            let mut chain = vec![t.as_str()];
            let mut cur = t.as_str();
            while let Some(p) = parent.get(cur) {
                chain.push(p);
                cur = p;
            }
            for c in chain.into_iter().rev() {
                if !on.contains(&c) {
                    on.push(c);
                }
            }
        }
        // In breadth-first order: parents before children.
        let depth = |id: &str| {
            let (mut d, mut c) = (0, id);
            while let Some(p) = parent.get(c) {
                d += 1;
                c = p;
            }
            d
        };
        on.sort_by_key(|id| (depth(id), self.by_id.get(id).map(|s| s.name.clone()).unwrap_or_default()));
        let nodes = on
            .into_iter()
            .map(|id| {
                let sw = self.by_id[id];
                let (igmp_ports, receivers) = counts.get(id).copied().unwrap_or((0, 0));
                let mut n = PathNode { switch_id: id.to_string(), switch_name: sw.name.clone(), parent: None, egress: None, ingress: None, igmp_ports, receivers };
                if id == from {
                    n.ingress.clone_from(&ingress);
                } else if let Some(p) = parent.get(id) {
                    let psw = self.by_id[p];
                    let (pa, pb) = self.link_ports(psw, sw);
                    n.parent = Some(p.to_string());
                    n.egress = Some(Self::ports_name(psw, &pa)).filter(|s| !s.is_empty());
                    n.ingress = Some(Self::ports_name(sw, &pb)).filter(|s| !s.is_empty());
                }
                n
            })
            .collect();
        (nodes, unreachable)
    }

    /// A snooping port as a receiver.
    fn receiver(&self, sw: &SwitchData, vlan: u16, at: &PortAt, router: bool) -> Receiver {
        let p = at.n.and_then(|n| sw.ports.iter().find(|p| p.n == n));
        let peer_name = |id: &str| self.by_id.get(id).map(|s| s.name.clone()).unwrap_or_else(|| id.to_string());
        let mut peer = p.and_then(|p| p.peer.as_ref()).map(|r| peer_name(&r.id));
        let agg = at.n.and_then(|n| sw.lag_of.get(&n).cloned());
        if peer.is_none() && at.n.is_none() {
            // An aggregate: a link to a switch when one of its members is.
            peer = sw.lag_of.iter().filter(|(_, a)| **a == at.name).find_map(|(n, _)| sw.ports.iter().find(|p| p.n == *n).and_then(|p| p.peer.as_ref())).map(|r| peer_name(&r.id));
        }
        // A link to a switch carries no host (the MAC table may have put one there).
        let host_ip = p.filter(|p| p.peer.is_none()).and_then(|p| p.host);
        let host = host_ip
            .map(|ip| self.addr_name(ip).unwrap_or_else(|| ip.to_string()))
            .or_else(|| p.filter(|p| p.peer.is_none()).and_then(|p| p.lldp.as_ref()).map(|l| l.name.clone()).filter(|n| !n.is_empty()));
        Receiver {
            switch_id: sw.id.clone(),
            switch_name: sw.name.clone(),
            n: at.n,
            port: p.map(|p| p.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| at.name.clone()),
            vlan,
            label: p.map(|p| p.label.clone()).filter(|l| !l.is_empty()),
            host,
            host_ip,
            router,
            peer,
            agg,
        }
    }
}

/// Snooping entries attributed to groups: group → (switch index, entry index, ambiguous, guessed).
fn attribute(inp: &Input, known: &[(Ipv4Addr, Option<u16>)]) -> BTreeMap<u32, Vec<(usize, usize, bool, bool)>> {
    let mut out: BTreeMap<u32, Vec<(usize, usize, bool, bool)>> = BTreeMap::new();
    for (si, sw) in inp.switches.iter().enumerate() {
        for (ei, e) in sw.snoop.iter().enumerate() {
            let (groups, ambiguous, guessed) = match e.group {
                Some(g) => (vec![g], false, false),
                None => {
                    let all = mac_groups(e.mac);
                    if all.iter().any(|g| control_group(*g)) {
                        continue;
                    }
                    let mut cands: Vec<Ipv4Addr> = known.iter().filter(|(g, _)| group_mac(*g) == e.mac).map(|(g, _)| *g).collect();
                    cands.sort_unstable();
                    cands.dedup();
                    // 32 groups share a MAC: those of the same VLAN first.
                    let same: Vec<Ipv4Addr> = cands.iter().copied().filter(|g| known.iter().any(|(k, v)| k == g && *v == Some(e.vlan))).collect();
                    let cands = if same.is_empty() { cands } else { same };
                    if cands.is_empty() {
                        // Unknown: a group of the plan (same VLAN first), else 239.x.y.z.
                        let in_plan = |vlan_match: bool| all.iter().copied().find(|g| inp.more.ranges.iter().any(|r| in_range(r, *g) && (!vlan_match || r.vlan == Some(e.vlan))));
                        let g = in_plan(true).or_else(|| in_plan(false)).unwrap_or(Ipv4Addr::new(239, e.mac[3], e.mac[4], e.mac[5]));
                        (vec![g], false, true)
                    } else {
                        let amb = cands.len() > 1;
                        (cands, amb, false)
                    }
                }
            };
            if groups.iter().any(|g| control_group(*g)) {
                continue;
            }
            for g in groups {
                out.entry(u32::from(g)).or_default().push((si, ei, ambiguous, guessed));
            }
        }
    }
    out
}

fn add_issue(f: &mut Flow, i: &'static str) {
    if !f.issues.contains(&i) {
        f.issues.push(i);
    }
}

/// Builds every flow and the views around them.
pub fn build(inp: &Input) -> FlowsView {
    let cx = Ctx::new(inp);
    let now = inp.now;
    let more = &inp.more;
    let range_vlan = |g: Ipv4Addr| more.ranges.iter().find(|r| in_range(r, g)).and_then(|r| r.vlan);

    // Flows from the streams.
    let mut flows: Vec<Flow> = inp
        .streams
        .iter()
        .filter(|s| !matches!(s.kind, Kind::Ptp | Kind::Sap) && !control_group(s.group))
        .map(|s| {
            let key = stream_key(s.group, s.port, s.source);
            let label = inp.labels.get(&key).cloned().unwrap_or_default();
            let group_label = more.group_labels.get(&s.group).cloned().unwrap_or_default();
            let name = Some(label.clone()).filter(|l| !l.is_empty()).or_else(|| Some(group_label.clone()).filter(|l| !l.is_empty())).or_else(|| s.name.clone());
            let vlan = s.flood_vlan.or_else(|| s.source.and_then(|ip| cx.vlan_of_ip(ip))).or_else(|| range_vlan(s.group));
            Flow {
                key,
                group: s.group,
                port: s.port,
                source: s.source,
                kind: s.kind,
                encoding: s.encoding.clone(),
                name,
                label,
                group_label,
                sap_name: s.name.clone(),
                announced: s.announced,
                routed: s.routed,
                flooded: s.flooded > 0 && now.saturating_sub(s.flooded) <= FLOOD_MS,
                flood_vlan: s.flood_vlan,
                snoop_only: false,
                guessed: false,
                ambiguous: false,
                live: is_live(s, now),
                state: "ok",
                issues: Vec::new(),
                bps: s.bps,
                pps: s.pps,
                est_bps: s.est_bps,
                history: s.history.clone(),
                first_seen: s.first_seen,
                last_seen: seen_at(s),
                last_announced: s.last_announced,
                sap_interval_ms: s.sap_interval_ms,
                format: s.format.clone(),
                vlan,
                emitter: None,
                sources: Vec::new(),
                conflict: false,
                receivers: Vec::new(),
                receiver_count: 0,
                path: Vec::new(),
                unreachable: Vec::new(),
                pair: None,
                pair_state: None,
                pair_by: None,
                leg: None,
                dup: s.dup.clone(),
            }
        })
        .collect();

    // Receivers from the snooping tables.
    let mut known: Vec<(Ipv4Addr, Option<u16>)> = flows.iter().map(|f| (f.group, f.vlan)).collect();
    known.extend(more.group_labels.keys().map(|g| (*g, range_vlan(*g))));
    known.extend(more.reservations.iter().map(|r| (r.group, range_vlan(r.group))));
    let snooped = attribute(inp, &known);
    for (g, entries) in &snooped {
        let g = Ipv4Addr::from(*g);
        if flows.iter().any(|f| f.group == g) {
            continue;
        }
        let vlans: HashSet<u16> = entries.iter().map(|(si, ei, _, _)| inp.switches[*si].snoop[*ei].vlan).collect();
        let group_label = more.group_labels.get(&g).cloned().unwrap_or_default();
        flows.push(Flow {
            key: stream_key(g, 0, None),
            group: g,
            port: 0,
            source: None,
            kind: Kind::Other,
            encoding: None,
            name: Some(group_label.clone()).filter(|l| !l.is_empty()),
            label: String::new(),
            group_label,
            sap_name: None,
            announced: false,
            routed: false,
            flooded: false,
            flood_vlan: None,
            snoop_only: true,
            guessed: entries.iter().all(|e| e.3),
            ambiguous: false,
            live: true,
            state: "unk",
            issues: Vec::new(),
            bps: 0.0,
            pps: 0.0,
            est_bps: None,
            history: Vec::new(),
            first_seen: now,
            last_seen: now,
            last_announced: None,
            sap_interval_ms: None,
            format: None,
            vlan: if vlans.len() == 1 { vlans.into_iter().next() } else { range_vlan(g) },
            emitter: None,
            sources: Vec::new(),
            conflict: false,
            receivers: Vec::new(),
            receiver_count: 0,
            path: Vec::new(),
            unreachable: Vec::new(),
            pair: None,
            pair_state: None,
            pair_by: None,
            leg: None,
            dup: None,
        });
    }

    // Sources of each group (live, in the same network): conflicts.
    let mut sources: HashMap<Ipv4Addr, Vec<Ipv4Addr>> = HashMap::new();
    for f in flows.iter().filter(|f| f.live && (f.bps > 0.0 || f.routed || f.flooded || f.announced)) {
        if let Some(s) = f.source {
            let v = sources.entry(f.group).or_default();
            if !v.contains(&s) {
                v.push(s);
            }
        }
    }
    let conflicting = |list: &[Ipv4Addr]| -> bool {
        // Two sources in two different known VLANs are the two legs of a pair on one group, not a conflict.
        list.iter().enumerate().any(|(i, a)| {
            list[i + 1..].iter().any(|b| {
                let (va, vb) = (cx.vlan_of_ip(*a), cx.vlan_of_ip(*b));
                va.is_none() || vb.is_none() || va == vb
            })
        })
    };

    for f in flows.iter_mut() {
        if let Some(list) = sources.get(&f.group).filter(|l| l.len() > 1) {
            if conflicting(list) && f.source.is_some_and(|s| list.contains(&s)) {
                f.conflict = true;
                f.sources = list.clone();
                f.sources.sort_unstable();
            }
        }
        // Receivers.
        let mut counts: HashMap<String, (usize, usize)> = HashMap::new();
        if let Some(entries) = snooped.get(&u32::from(f.group)) {
            for (si, ei, amb, guessed) in entries {
                let sw = &inp.switches[*si];
                let e = &sw.snoop[*ei];
                f.ambiguous |= *amb;
                if f.snoop_only {
                    f.guessed &= *guessed;
                }
                let routers: Vec<&PortAt> = sw.router.iter().filter(|(v, _)| *v == e.vlan).flat_map(|(_, p)| p.iter()).collect();
                let dup = |rs: &[Receiver], p: &PortAt| rs.iter().any(|r| r.switch_id == sw.id && r.n == p.n && (p.n.is_some() || r.port == p.name));
                for p in &e.ports {
                    if dup(&f.receivers, p) {
                        continue;
                    }
                    let r = cx.receiver(sw, e.vlan, p, routers.contains(&p));
                    let c = counts.entry(sw.id.clone()).or_default();
                    c.0 += 1;
                    if !r.router && r.peer.is_none() {
                        c.1 += 1;
                    }
                    f.receivers.push(r);
                }
                for p in routers {
                    if !dup(&f.receivers, p) {
                        f.receivers.push(cx.receiver(sw, e.vlan, p, true));
                    }
                }
            }
        }
        f.receiver_count = f.receivers.iter().filter(|r| !r.router && r.peer.is_none()).count();
        // Emitter and path.
        if let Some(ip) = f.source {
            let mac = cx.mac_of(ip);
            let at = cx.locate(Some(ip), mac.as_deref());
            f.emitter = Some(Emitter { ip, name: cx.addr_name(ip), mac, at });
        }
        let mut to: Vec<String> = f.receivers.iter().map(|r| r.switch_id.clone()).collect();
        to.sort();
        to.dedup();
        if let Some(at) = f.emitter.as_ref().and_then(|e| e.at.as_ref()) {
            let (nodes, unreachable) = cx.path(&at.switch_id, &to, Some(at.port.clone()), &counts);
            f.path = nodes;
            f.unreachable = unreachable;
        }
    }

    // ST 2022-7 pairs.
    let pairs = pair_flows(&flows);
    let refs: Vec<(PairRef, bool)> = flows
        .iter()
        .map(|f| (PairRef { key: f.key.clone(), group: f.group, port: f.port, source: f.source, name: f.name.clone(), at: f.emitter.as_ref().and_then(|e| e.at.clone()) }, f.live))
        .collect();
    for (i, f) in flows.iter_mut().enumerate() {
        if let Some((j, by, leg)) = pairs[i] {
            let (pair, live) = refs[j].clone();
            f.pair = Some(pair);
            f.pair_by = Some(by);
            f.leg = Some(leg);
            f.pair_state = Some(if live && f.live { "ok" } else { "missing" });
        } else if let Some((_, leg, "color")) = f.name.as_deref().and_then(split_leg) {
            // Named Red or Blue, and no other leg.
            f.leg = Some(leg);
            f.pair_state = Some("missing");
        }
    }

    // State and issues.
    for f in flows.iter_mut() {
        if !f.announced {
            add_issue(f, "unannounced");
        }
        if f.conflict {
            add_issue(f, "conflict");
        }
        if f.pair_state == Some("missing") {
            add_issue(f, "pairMissing");
        }
        if f.flooded {
            add_issue(f, "flooded");
        }
        if f.live && f.receiver_count == 0 && inp.switches.iter().any(|s| s.read) {
            add_issue(f, "noReceivers");
        }
        if !f.live {
            add_issue(f, "gone");
        }
        f.state = if !f.live {
            "off"
        } else if f.conflict || f.pair_state == Some("missing") || f.flooded {
            "warn"
        } else if !f.announced {
            "unk"
        } else {
            "ok"
        };
    }
    flows.sort_by_key(|f| (u32::from(f.group), f.port, f.source.map(u32::from)));

    // Summary.
    let mut sm = Summary { flows: flows.len(), switches_online: inp.switches.iter().filter(|s| s.online).count(), switches_read: inp.switches.iter().filter(|s| s.read).count(), ..Summary::default() };
    let mut pair_keys: HashSet<(String, String)> = HashSet::new();
    let mut incomplete: HashSet<(String, String)> = HashSet::new();
    let mut rx_ports: HashSet<(String, String)> = HashSet::new();
    let mut conflict_groups: HashSet<Ipv4Addr> = HashSet::new();
    for f in &flows {
        *sm.by_kind.entry(serde_json::to_value(f.kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()).or_default() += 1;
        if f.live {
            sm.live += 1;
            sm.est_bps += f.est_bps.unwrap_or(0.0);
            sm.bps += f.bps;
        }
        for r in f.receivers.iter().filter(|r| !r.router && r.peer.is_none()) {
            rx_ports.insert((r.switch_id.clone(), r.port.clone()));
        }
        let pk = match &f.pair {
            Some(p) => {
                let (a, b) = if f.key < p.key { (f.key.clone(), p.key.clone()) } else { (p.key.clone(), f.key.clone()) };
                Some((a, b))
            }
            None => f.pair_state.map(|_| (f.key.clone(), String::new())),
        };
        if let Some(pk) = pk {
            if f.pair_state == Some("missing") {
                incomplete.insert(pk.clone());
            }
            pair_keys.insert(pk);
        }
        sm.unannounced += usize::from(!f.announced);
        sm.no_receivers += usize::from(f.issues.contains(&"noReceivers"));
        if f.conflict {
            conflict_groups.insert(f.group);
        }
        sm.flooded += usize::from(f.flooded);
    }
    sm.receivers = rx_ports.len();
    sm.pairs = pair_keys.len();
    sm.pairs_incomplete = incomplete.len();
    sm.conflicts = conflict_groups.len();

    let flood_vlan = flows.iter().find_map(|f| f.flooded.then_some(f.flood_vlan).flatten()).or_else(|| inp.iface_ip.and_then(|ip| cx.vlan_of_ip(ip)));
    FlowsView {
        at: now,
        plan: plan(more, &flows),
        ptp: ptp_view(&cx),
        switches: inp.switches.iter().map(|s| TopoSwitch { id: s.id.clone(), name: s.name.clone(), ip: s.ip, online: s.online, read: s.read, at: s.at }).collect(),
        links: topo_links(&cx),
        flows,
        summary: sm,
        flood: FloodView { status: inp.flood.clone(), vlan: flood_vlan },
        rules: more.rules.clone(),
        check: CheckView { enabled: more.check_enabled, max_mbps: inp.check_max_mbps },
    }
}

fn topo_links(cx: &Ctx) -> Vec<TopoLink> {
    let mut out = Vec::new();
    for a in &cx.inp.switches {
        for b in cx.neighbours(&a.id) {
            if a.id.as_str() >= b {
                continue;
            }
            let bsw = cx.by_id[b];
            let (pa, pb) = cx.link_ports(a, bsw);
            out.push(TopoLink { a: a.id.clone(), b: b.to_string(), a_port: Ctx::ports_name(a, &pa), b_port: Ctx::ports_name(bsw, &pb) });
        }
    }
    out
}

/// The group plan: each range's groups used (seen or announced), reserved, in conflict.
pub fn plan(more: &McastMore, flows: &[Flow]) -> Plan {
    // group → (state, name)
    let mut uses: BTreeMap<u32, GroupUse> = BTreeMap::new();
    for f in flows.iter().filter(|f| !f.guessed) {
        let e = uses.entry(u32::from(f.group)).or_insert(GroupUse { group: f.group, state: "used", name: None });
        if f.conflict {
            e.state = "conflict";
        }
        if e.name.is_none() {
            e.name.clone_from(&f.name);
        }
    }
    for r in &more.reservations {
        uses.entry(u32::from(r.group)).or_insert(GroupUse { group: r.group, state: "reserved", name: Some(r.name.clone()).filter(|n| !n.is_empty()) });
    }
    let mut placed: HashSet<u32> = HashSet::new();
    let mut ranges = Vec::new();
    for r in &more.ranges {
        let Some((first, last)) = block(&r.cidr) else { continue };
        let (a, b) = (u32::from(first), u32::from(last));
        let groups: Vec<GroupUse> = uses.range(a..=b).map(|(_, g)| g.clone()).collect();
        placed.extend(uses.range(a..=b).map(|(k, _)| *k));
        let size = u64::from(b - a) + 1;
        let next_free = (a..=b).find(|g| !uses.contains_key(g) && !matches!(g & 0xFF, 0 | 255)).map(Ipv4Addr::from);
        let count = |s: &str| groups.iter().filter(|g| g.state == s).count();
        let (used, reserved, conflicts) = (count("used") + count("conflict"), count("reserved"), count("conflict"));
        ranges.push(RangeView { range: r.clone(), first, last, size, used, reserved, conflicts, free: size - groups.len() as u64, next_free, groups });
    }
    let rest: Vec<GroupUse> = uses.iter().filter(|(k, _)| !placed.contains(k)).map(|(_, g)| g.clone()).collect();
    let count = |s: &str| rest.iter().filter(|g| g.state == s).count();
    let outside = Outside { used: count("used") + count("conflict"), reserved: count("reserved"), conflicts: count("conflict"), groups: rest };
    Plan { ranges, outside, reservations: more.reservations.clone() }
}

fn ptp_view(cx: &Ctx) -> PtpView {
    let inp = cx.inp;
    let now = inp.now;
    let live = |t: u64| now.saturating_sub(t) <= ptp::ANNOUNCE_STALE_MS;
    let best = ptp::best(&inp.ptp.announcers, None, now).cloned();
    let domain = best.as_ref().map(|b| b.domain);
    let attach = |ip: Ipv4Addr, clock: &str| {
        let mac = cx.mac_of(ip).or_else(|| ptp::identity_mac(clock));
        (cx.locate(Some(ip), mac.as_deref()), mac)
    };
    let clocks: Vec<PtpClock> = inp
        .ptp
        .announcers
        .iter()
        .map(|a| {
            let role = if ptp::best(&inp.ptp.announcers, Some(a.domain), now).is_some_and(|b| b.ip == a.ip && b.clock == a.clock) {
                "gm"
            } else if a.steps_removed > 0 || a.gm != a.clock {
                "boundary"
            } else {
                "backup"
            };
            PtpClock { clock: a.clone(), name: cx.addr_name(a.ip), role, live: live(a.last_seen), wrong_domain: domain.is_some_and(|d| d != a.domain), at: attach(a.ip, &a.clock).0 }
        })
        .collect();
    let devices = inp
        .ptp
        .followers
        .iter()
        .map(|f| {
            let (at, mac) = attach(f.ip, &f.clock);
            PtpDevice { device: f.clone(), name: cx.addr_name(f.ip), mac, live: now.saturating_sub(f.last_seen) <= 30_000, wrong_domain: domain.is_some_and(|d| d != f.domain), at }
        })
        .collect();
    let gm = best.and_then(|b| clocks.iter().find(|c| c.clock.ip == b.ip && c.clock.clock == b.clock).cloned());
    let mut changes = inp.more.ptp_changes.clone();
    changes.reverse();
    PtpView { gm, domain, clocks, devices, changes }
}

// ── Gathering what the computation reads ──

impl Net {
    /// Everything `build` reads, taken under the locks one after the other.
    pub fn flows_input(&self) -> Input {
        let mut inp = Input { now: now_ms() as u64, ..Input::default() };
        let mut online: Vec<String> = Vec::new();
        {
            let st = self.store.lock().unwrap();
            inp.labels = st.multicast.labels.clone();
            inp.check_max_mbps = st.multicast.check_max_mbps;
            inp.more = st.multicast.more.clone();
            inp.addresses = st
                .addresses
                .values()
                .map(|a| {
                    let name = Some(a.label.clone()).filter(|l| !l.is_empty()).or_else(|| a.name.clone());
                    (a.ip, Addr { name, mac: a.mac.clone(), switch: a.switch.clone() })
                })
                .collect();
            inp.vlan_nets = st.vlans.values().filter_map(|v| strata_net::cidr::parse_block(v.subnet.as_deref()?).ok().map(|(n, p)| (v.id, n, p))).collect();
            for n in &st.networks {
                if let (Some(v), Ok((net, p))) = (n.vlan, strata_net::cidr::parse_block(&n.range)) {
                    inp.vlan_nets.push((v, net, p));
                }
            }
            inp.switches = st
                .switches
                .iter()
                .map(|s| {
                    if s.online {
                        online.push(s.id.clone());
                    }
                    SwitchData { id: s.id.clone(), name: s.name.clone(), ip: s.ip, ports: s.ports.clone(), online: s.online, ..SwitchData::default() }
                })
                .collect();
        }
        {
            let map = self.live.lock().unwrap();
            for sw in inp.switches.iter_mut().filter(|s| online.contains(&s.id)) {
                let Some(r) = map.get(&sw.id) else { continue };
                let Some(d) = &r.info.data else { continue };
                let ifs = Ifs::of(r.names(), &r.live.ports);
                let at = |i: &u32| PortAt { n: ifs.ports.get(i).copied(), name: ifs.names.get(i).cloned().unwrap_or_else(|| format!("if{i}")) };
                sw.read = true;
                sw.at = r.info.at;
                sw.snoop = d.mcast_groups.iter().map(|g| Snoop { vlan: g.vlan, group: g.group, mac: g.mac, ports: g.ports.iter().map(at).collect() }).collect();
                sw.router = d.mcast_router_ports.iter().map(|(v, p)| (*v, p.iter().map(at).collect())).collect();
                sw.lag_of = d.lags.iter().filter_map(|(m, a)| Some((*ifs.ports.get(m)?, ifs.names.get(a).cloned().unwrap_or_else(|| format!("if{a}"))))).collect();
            }
        }
        inp.arp = self.snmp_arp();
        if let Some(e) = self.mcast.lock().unwrap().as_ref() {
            inp.streams = e.snapshot();
            inp.ptp = e.ptp();
            inp.flood = e.flood_status();
            inp.iface_ip = Some(e.iface());
        }
        inp
    }

    pub fn flows(&self) -> FlowsView {
        build(&self.flows_input())
    }

    /// Whether the on-demand check of `group:port` may join it: turned on, a known
    /// stream, and not heavier than the limit by its SDP or by what was measured
    /// (a video whose rate is not known is refused too).
    pub fn check_guard(&self, group: Ipv4Addr, port: u16) -> ApiResult<()> {
        let (enabled, max) = {
            let st = self.store.lock().unwrap();
            (st.multicast.more.check_enabled, st.multicast.check_max_mbps)
        };
        if !enabled {
            return Err(status(403, "the stream check is turned off"));
        }
        let streams: Vec<Stream> = self.mcast.lock().unwrap().as_ref().map(|e| e.snapshot()).unwrap_or_default().into_iter().filter(|s| s.group == group && s.port == port).collect();
        check_allowed(&streams, max)
    }
}

/// The rules of [`Net::check_guard`] on the streams of one group and port.
pub fn check_allowed(streams: &[Stream], max_mbps: u64) -> ApiResult<()> {
    if streams.is_empty() {
        return Err(not_found("stream"));
    }
    let limit = max_mbps as f64 * 1e6;
    let est = streams.iter().filter_map(|s| s.est_bps).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
    let measured = streams.iter().map(|s| s.bps).fold(0.0, f64::max);
    if est.is_some_and(|e| e > limit) || measured > limit {
        return Err(status(409, "the stream is heavier than the limit"));
    }
    let video = streams.iter().any(|s| {
        let e = s.encoding.as_deref().unwrap_or("").to_ascii_lowercase();
        s.kind == Kind::Video || matches!(e.as_str(), "raw" | "jxsv" | "smpte2022-6") || s.format.as_ref().and_then(|f| f.standard.as_deref()) == Some("ST 2022-6")
    });
    if est.is_none() && video {
        return Err(status(409, "the rate of this video stream is not known"));
    }
    Ok(())
}

// ── Alerts ──

/// What the alert watcher remembers.
#[derive(Default)]
pub struct Watch {
    /// Engine generation seen (a new engine starts empty: nothing is "gone").
    generation: u64,
    started: bool,
    /// Announced flows live at the last pass.
    live: HashSet<String>,
    /// Groups with receivers: the switches they were on.
    had_rx: HashMap<Ipv4Addr, HashSet<String>>,
    /// Grandmaster of each domain at the last pass.
    gm: HashMap<u8, (String, Ipv4Addr)>,
    /// Conditions and since when they hold (ms); raised ones.
    since: HashMap<String, u64>,
    raised: HashSet<String>,
}

/// Conditions must hold this long before they are raised (legs announced one after the other…).
const HOLD_MS: u64 = 15_000;

/// An alert to raise: (kind, ip, detail).
pub type McastAlert = (&'static str, Ipv4Addr, String);

fn flow_words(f: &Flow) -> String {
    let at = if f.port == 0 { f.group.to_string() } else { format!("{}:{}", f.group, f.port) };
    match &f.name {
        Some(n) => format!("{at} {n}"),
        None => at,
    }
}

/// One pass of the rules over the view: the alerts to raise now, and the grandmaster changes.
pub fn evaluate(w: &mut Watch, v: &FlowsView, generation: u64, now: u64) -> (Vec<McastAlert>, Vec<GmChange>) {
    let rules = &v.rules;
    let mut out: Vec<McastAlert> = Vec::new();
    let mut changes = Vec::new();
    if w.generation != generation || !w.started {
        // A new engine: start from what it sees, nothing changed yet.
        *w = Watch { generation, started: true, gm: std::mem::take(&mut w.gm), ..Watch::default() };
    }
    // Transitions: announced streams gone.
    let live_now: HashSet<String> = v.flows.iter().filter(|f| f.announced && f.live).map(|f| f.key.clone()).collect();
    for key in w.live.difference(&live_now) {
        // Merged into a received stream (same group and port, a source now): not gone.
        let (g, rest) = key.split_once(':').unwrap_or_default();
        let port = rest.split(':').next().unwrap_or_default();
        if live_now.iter().any(|k| k.starts_with(&format!("{g}:{port}:"))) {
            continue;
        }
        if rules.stream_gone {
            let f = v.flows.iter().find(|f| &f.key == key);
            let ip = f.and_then(|f| f.source).or_else(|| g.parse().ok()).unwrap_or(Ipv4Addr::UNSPECIFIED);
            let words = f.map(flow_words).unwrap_or_else(|| format!("{g}:{port}"));
            out.push(("streamGone", ip, words));
        }
    }
    w.live = live_now;
    // Grandmaster changes, by domain.
    let mut gms: HashMap<u8, (String, Ipv4Addr)> = HashMap::new();
    for c in v.ptp.clocks.iter().filter(|c| c.role == "gm" && c.live) {
        gms.insert(c.clock.domain, (c.clock.gm.clone(), c.clock.ip));
    }
    let mut domains: Vec<u8> = gms.keys().chain(w.gm.keys()).copied().collect();
    domains.sort_unstable();
    domains.dedup();
    for d in domains {
        let (old, new) = (w.gm.get(&d), gms.get(&d));
        if old.map(|o| &o.0) == new.map(|n| &n.0) {
            continue;
        }
        if old.is_none() && !v.ptp.changes.iter().any(|c| c.domain == d) {
            // The first grandmaster heard is not a change.
            continue;
        }
        let old = old.cloned().or_else(|| v.ptp.changes.iter().find(|c| c.domain == d).and_then(|c| Some((c.to.clone()?, c.to_ip?))));
        if old.as_ref().map(|o| &o.0) == new.map(|n| &n.0) {
            continue;
        }
        let ch = GmChange { at: now as i64, domain: d, from: old.as_ref().map(|o| o.0.clone()), to: new.map(|n| n.0.clone()), from_ip: old.as_ref().map(|o| o.1), to_ip: new.map(|n| n.1) };
        if rules.gm_change {
            let name = |id: &Option<String>, ip: Option<Ipv4Addr>| match (id, ip) {
                (Some(id), Some(ip)) => format!("{id} ({ip})"),
                _ => "none".to_string(),
            };
            out.push(("ptpGmChange", ch.to_ip.or(ch.from_ip).unwrap_or(Ipv4Addr::UNSPECIFIED), format!("domain {d}: {} → {}", name(&ch.from, ch.from_ip), name(&ch.to, ch.to_ip))));
        }
        changes.push(ch);
    }
    w.gm = gms;
    // Conditions that hold.
    let mut holds: Vec<(String, McastAlert)> = Vec::new();
    let read: HashSet<&str> = v.switches.iter().filter(|s| s.read).map(|s| s.id.as_str()).collect();
    let mut groups_done: HashSet<Ipv4Addr> = HashSet::new();
    for f in &v.flows {
        let ip = f.source.unwrap_or(f.group);
        if rules.conflict && f.conflict && groups_done.insert(f.group) {
            let list = f.sources.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(" and ");
            holds.push((format!("conflict:{}", f.group), ("groupConflict", f.group, format!("{} from {list}", f.group))));
        }
        if rules.pair_missing && f.live && f.pair_state == Some("missing") {
            let other = match &f.pair {
                Some(p) => format!(" · {}:{} missing", p.group, p.port),
                None => " · other leg missing".to_string(),
            };
            holds.push((format!("pair:{}", f.key), ("pairMissing", ip, format!("{}{other}", flow_words(f)))));
        }
        if f.snoop_only {
            continue;
        }
        // Receivers: on which read switches.
        let on: HashSet<String> = f.receivers.iter().filter(|r| !r.router && r.peer.is_none()).map(|r| r.switch_id.clone()).collect();
        if !on.is_empty() {
            w.had_rx.entry(f.group).or_default().extend(on);
        } else if let Some(prev) = w.had_rx.get(&f.group) {
            // The switches that had them are still read: the receivers left
            // (a switch not read any more says nothing).
            if rules.no_receivers && f.live && prev.iter().all(|s| read.contains(s.as_str())) {
                holds.push((format!("norx:{}", f.group), ("noReceivers", ip, flow_words(f))));
            }
        }
    }
    w.had_rx.retain(|g, _| v.flows.iter().any(|f| f.group == *g && !f.snoop_only));
    if rules.flooding && !v.flood.status.groups.is_empty() && v.flood.status.available && v.flood.status.enabled {
        let vlan = v.flood.vlan.map(|x| format!("VLAN {x}")).unwrap_or_else(|| "no VLAN".into());
        let list = v.flood.status.groups.iter().take(4).map(|g| g.to_string()).collect::<Vec<_>>().join(", ");
        let more = v.flood.status.groups.len().saturating_sub(4);
        let detail = format!("{} · {vlan} · {} groups: {list}{}", v.flood.status.iface, v.flood.status.groups.len(), if more > 0 { format!(" +{more}") } else { String::new() });
        holds.push((format!("flood:{}:{vlan}", v.flood.status.iface), ("flooding", Ipv4Addr::UNSPECIFIED, detail)));
    }
    if rules.ptp_domain {
        if let Some(d) = v.ptp.domain {
            for c in v.ptp.clocks.iter().filter(|c| c.live && c.wrong_domain) {
                holds.push((format!("dom:{}:{}", c.clock.ip, c.clock.domain), ("ptpDomain", c.clock.ip, format!("{} domain {}, grandmaster domain {d}", c.name.clone().unwrap_or_else(|| c.clock.ip.to_string()), c.clock.domain))));
            }
            for x in v.ptp.devices.iter().filter(|x| x.live && x.wrong_domain) {
                holds.push((format!("dom:{}:{}", x.device.ip, x.device.domain), ("ptpDomain", x.device.ip, format!("{} domain {}, grandmaster domain {d}", x.name.clone().unwrap_or_else(|| x.device.ip.to_string()), x.device.domain))));
            }
        }
    }
    let keys: HashSet<String> = holds.iter().map(|h| h.0.clone()).collect();
    w.since.retain(|k, _| keys.contains(k));
    w.raised.retain(|k| keys.contains(k));
    for (k, a) in holds {
        let since = *w.since.entry(k.clone()).or_insert(now);
        if now.saturating_sub(since) >= HOLD_MS && w.raised.insert(k) {
            out.push(a);
        }
    }
    (out, changes)
}

/// Engine generation: bumped by `multicast::start`.
pub static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Checks the multicast rules every 5 s.
pub fn spawn_watch(net: Arc<Net>) {
    tokio::spawn(async move {
        let watch = Mutex::new(Watch::default());
        tokio::time::sleep(Duration::from_secs(5)).await;
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            let v = net.flows();
            let generation = GENERATION.load(std::sync::atomic::Ordering::Relaxed);
            let (alerts, changes) = evaluate(&mut watch.lock().unwrap(), &v, generation, v.at);
            if alerts.is_empty() && changes.is_empty() {
                continue;
            }
            let mut raised = false;
            {
                let mut st = net.store.lock().unwrap();
                let flood_ip = net.mcast.lock().unwrap().as_ref().map(|e| e.iface());
                for (kind, ip, detail) in alerts {
                    let ip = if kind == "flooding" { flood_ip.unwrap_or(ip) } else { ip };
                    raised |= net.alert(&mut st, kind, ip, detail);
                }
                if !changes.is_empty() {
                    let list = &mut st.multicast.more.ptp_changes;
                    list.extend(changes.iter().cloned());
                    let n = list.len().saturating_sub(GM_CHANGES_KEPT);
                    list.drain(..n);
                }
            }
            if raised {
                net.save(&[File::Alerts]);
                net.changed("alerts");
            }
            if !changes.is_empty() {
                net.save(&[File::Multicast]);
                net.changed("multicast");
            }
        }
    });
}

// ── Routes ──

pub fn router() -> Router<Shared> {
    Router::new()
        .route("/api/multicast/flows", get(flows))
        .route("/api/multicast/labels/{group}", put(set_group_label))
        .route("/api/multicast/ranges", get(ranges).post(create_range))
        .route("/api/multicast/ranges/{id}", put(update_range).delete(delete_range))
        .route("/api/multicast/reservations", get(reservations).post(create_reservation))
        .route("/api/multicast/reservations/{group}", put(update_reservation).delete(delete_reservation))
        .route("/api/multicast/rules", get(rules).put(set_rules))
        .route("/api/multicast/sdp/{group}/{port}", get(sdp))
        .route("/api/multicast/options", put(set_options))
}

fn who(me: &Session) -> String {
    if me.user.display_name.is_empty() {
        me.user.username.clone()
    } else {
        me.user.display_name.clone()
    }
}

fn editor(me: &Session) -> ApiResult<()> {
    require(me.user.can_edit())
}

fn group_arg(s: &str) -> ApiResult<Ipv4Addr> {
    s.parse::<Ipv4Addr>().ok().filter(|g| g.is_multicast()).ok_or_else(|| bad("not a multicast group"))
}

async fn flows(State(s): State<Shared>, _me: Session) -> Json<FlowsView> {
    let n = s.net.clone();
    Json(tokio::task::spawn_blocking(move || n.flows()).await.unwrap_or_default())
}

#[derive(Deserialize)]
struct LabelBody {
    label: String,
}

async fn set_group_label(State(s): State<Shared>, me: Session, Path(group): Path<String>, Json(b): Json<LabelBody>) -> ApiResult<StatusCode> {
    editor(&me)?;
    let g = group_arg(&group)?;
    let label = b.label.trim().to_string();
    {
        let mut st = s.net.store.lock().unwrap();
        if label.is_empty() {
            st.multicast.more.group_labels.remove(&g);
        } else {
            st.multicast.more.group_labels.insert(g, label.clone());
        }
    }
    s.net.save(&[File::Multicast]);
    s.net.changed("multicast");
    s.net.log(&who(&me), "named group", &g.to_string(), &label);
    Ok(StatusCode::NO_CONTENT)
}

fn check_range(r: &mut Range) -> ApiResult<()> {
    let (net, p) = strata_net::cidr::parse_block(&r.cidr).map_err(bad)?;
    if !net.is_multicast() || p < 4 {
        return Err(bad("a range is inside 224.0.0.0/4"));
    }
    if p < 16 {
        return Err(bad("a range holds at most 65536 groups (/16)"));
    }
    r.cidr = format!("{net}/{p}");
    r.usage = r.usage.trim().to_string();
    if r.vlan.is_some_and(|v| v == 0 || v > 4094) {
        return Err(bad("a vlan is a number from 1 to 4094"));
    }
    Ok(())
}

fn overlaps(list: &[Range], r: &Range) -> bool {
    let Some((a, b)) = block(&r.cidr) else { return false };
    list.iter().filter(|x| x.id != r.id).filter_map(|x| block(&x.cidr)).any(|(c, d)| u32::from(a) <= u32::from(d) && u32::from(c) <= u32::from(b))
}

async fn ranges(State(s): State<Shared>, _me: Session) -> Json<Vec<Range>> {
    Json(s.net.store.lock().unwrap().multicast.more.ranges.clone())
}

async fn create_range(State(s): State<Shared>, me: Session, Json(mut r): Json<Range>) -> ApiResult<Json<Range>> {
    editor(&me)?;
    check_range(&mut r)?;
    r.id = new_id();
    {
        let mut st = s.net.store.lock().unwrap();
        if overlaps(&st.multicast.more.ranges, &r) {
            return Err(status(409, "this range overlaps another one"));
        }
        st.multicast.more.ranges.push(r.clone());
        st.multicast.more.ranges.sort_by_key(|x| block(&x.cidr).map(|b| u32::from(b.0)));
    }
    s.net.save(&[File::Multicast]);
    s.net.changed("multicast");
    s.net.log(&who(&me), "created multicast range", &r.cidr, &r.usage);
    Ok(Json(r))
}

async fn update_range(State(s): State<Shared>, me: Session, Path(id): Path<String>, Json(mut r): Json<Range>) -> ApiResult<Json<Range>> {
    editor(&me)?;
    check_range(&mut r)?;
    r.id = id.clone();
    {
        let mut st = s.net.store.lock().unwrap();
        if overlaps(&st.multicast.more.ranges, &r) {
            return Err(status(409, "this range overlaps another one"));
        }
        let x = st.multicast.more.ranges.iter_mut().find(|x| x.id == id).ok_or_else(|| not_found("range"))?;
        *x = r.clone();
        st.multicast.more.ranges.sort_by_key(|x| block(&x.cidr).map(|b| u32::from(b.0)));
    }
    s.net.save(&[File::Multicast]);
    s.net.changed("multicast");
    s.net.log(&who(&me), "changed multicast range", &r.cidr, &r.usage);
    Ok(Json(r))
}

async fn delete_range(State(s): State<Shared>, me: Session, Path(id): Path<String>) -> ApiResult<StatusCode> {
    editor(&me)?;
    let r = {
        let mut st = s.net.store.lock().unwrap();
        let i = st.multicast.more.ranges.iter().position(|x| x.id == id).ok_or_else(|| not_found("range"))?;
        st.multicast.more.ranges.remove(i)
    };
    s.net.save(&[File::Multicast]);
    s.net.changed("multicast");
    s.net.log(&who(&me), "deleted multicast range", &r.cidr, "");
    Ok(StatusCode::NO_CONTENT)
}

async fn reservations(State(s): State<Shared>, _me: Session) -> Json<Vec<Reservation>> {
    Json(s.net.store.lock().unwrap().multicast.more.reservations.clone())
}

async fn create_reservation(State(s): State<Shared>, me: Session, Json(mut r): Json<Reservation>) -> ApiResult<Json<Reservation>> {
    editor(&me)?;
    if !r.group.is_multicast() {
        return Err(bad("not a multicast group"));
    }
    r.name = r.name.trim().to_string();
    {
        let mut st = s.net.store.lock().unwrap();
        let list = &mut st.multicast.more.reservations;
        if list.iter().any(|x| x.group == r.group) {
            return Err(status(409, "this group is already reserved"));
        }
        list.push(r.clone());
        list.sort_by_key(|x| u32::from(x.group));
    }
    s.net.save(&[File::Multicast]);
    s.net.changed("multicast");
    s.net.log(&who(&me), "reserved group", &r.group.to_string(), &r.name);
    Ok(Json(r))
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

async fn update_reservation(State(s): State<Shared>, me: Session, Path(group): Path<String>, Json(b): Json<NameBody>) -> ApiResult<Json<Reservation>> {
    editor(&me)?;
    let g = group_arg(&group)?;
    let r = {
        let mut st = s.net.store.lock().unwrap();
        let x = st.multicast.more.reservations.iter_mut().find(|x| x.group == g).ok_or_else(|| not_found("reservation"))?;
        x.name = b.name.trim().to_string();
        x.clone()
    };
    s.net.save(&[File::Multicast]);
    s.net.changed("multicast");
    s.net.log(&who(&me), "changed reservation", &g.to_string(), &r.name);
    Ok(Json(r))
}

async fn delete_reservation(State(s): State<Shared>, me: Session, Path(group): Path<String>) -> ApiResult<StatusCode> {
    editor(&me)?;
    let g = group_arg(&group)?;
    {
        let mut st = s.net.store.lock().unwrap();
        let list = &mut st.multicast.more.reservations;
        let i = list.iter().position(|x| x.group == g).ok_or_else(|| not_found("reservation"))?;
        list.remove(i);
    }
    s.net.save(&[File::Multicast]);
    s.net.changed("multicast");
    s.net.log(&who(&me), "released group", &g.to_string(), "");
    Ok(StatusCode::NO_CONTENT)
}

async fn rules(State(s): State<Shared>, _me: Session) -> Json<McastRules> {
    Json(s.net.store.lock().unwrap().multicast.more.rules.clone())
}

async fn set_rules(State(s): State<Shared>, me: Session, Json(r): Json<McastRules>) -> ApiResult<Json<McastRules>> {
    editor(&me)?;
    s.net.store.lock().unwrap().multicast.more.rules = r.clone();
    s.net.save(&[File::Multicast]);
    s.net.changed("multicast");
    s.net.log(&who(&me), "changed multicast alert rules", "", "");
    Ok(Json(r))
}

async fn set_options(State(s): State<Shared>, me: Session, Json(o): Json<McastOptions>) -> ApiResult<StatusCode> {
    require(me.user.is_admin())?;
    let flood = {
        let mut st = s.net.store.lock().unwrap();
        let m = &mut st.multicast.more;
        if let Some(v) = o.check_enabled {
            m.check_enabled = v;
        }
        if let Some(v) = o.flood_watch {
            m.flood_watch = v;
        }
        m.flood_watch
    };
    if let Some(e) = s.net.mcast.lock().unwrap().as_ref() {
        e.set_flood_watch(flood);
    }
    s.net.save(&[File::Multicast]);
    s.net.changed("multicast");
    let words = [o.check_enabled.map(|v| format!("check {}", if v { "on" } else { "off" })), o.flood_watch.map(|v| format!("flood watch {}", if v { "on" } else { "off" }))];
    s.net.log(&who(&me), "changed multicast settings", "", &words.into_iter().flatten().collect::<Vec<_>>().join(", "));
    Ok(StatusCode::NO_CONTENT)
}

async fn sdp(State(s): State<Shared>, _me: Session, Path((group, port)): Path<(String, u16)>) -> ApiResult<impl IntoResponse> {
    let g = group_arg(&group)?;
    let text = s.net.mcast.lock().unwrap().as_ref().and_then(|e| e.sdp(g, port)).ok_or_else(|| not_found("sdp"))?;
    Ok(([(header::CONTENT_TYPE, "application/sdp; charset=utf-8")], text))
}

#[cfg(test)]
mod tests;
