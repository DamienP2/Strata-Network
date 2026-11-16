//! The NMOS registry in the flows (`docs/api-network.md` § NMOS): its senders
//! become flows, merged by group and port with what SAP, packets and snooping
//! said; its receivers' subscriptions become the flows' subscribers; its nodes
//! are placed on switch ports and matched with the inventory.

use super::*;
use crate::net::nmos::{norm_mac, Node, Registry};

/// What a flow takes from its NMOS sender.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NmosRef {
    pub sender_id: String,
    /// The sender's label.
    pub sender: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// The node's label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// The node is known to the inventory (one of its addresses or MACs).
    pub inventory: bool,
    /// `subscription.active` of the sender (v1.2+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
}

/// An NMOS receiver set to receive the flow's sender.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscriber {
    pub id: String,
    /// The receiver's label.
    pub label: String,
    /// Its node's label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<Attach>,
    /// A snooping receiver is on the same switch port (counted once).
    pub igmp: bool,
}

/// A node of the registry, for the NMOS page.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeView {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// "Cobalt · BIDI4" (asset tags).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maker: Option<String>,
    /// Its API address, else a media address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<Ipv4Addr>,
    pub ips: Vec<Ipv4Addr>,
    /// Its name in the inventory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub senders: usize,
    pub receivers: usize,
    /// The switch ports it is plugged on (first: the most certain).
    pub at: Vec<Attach>,
    pub inventory: bool,
    /// Last read it was in (unix ms); `present`: in the last read.
    pub seen: i64,
    pub present: bool,
}

/// Where a node is.
#[derive(Clone, Debug, Default)]
pub(super) struct Place {
    pub ips: Vec<Ipv4Addr>,
    pub at: Vec<Attach>,
    /// The port each address was found on.
    pub at_by_ip: HashMap<Ipv4Addr, Attach>,
    pub inventory: bool,
    pub name: Option<String>,
}

/// The registry with its nodes placed.
pub(super) struct Index<'a> {
    pub reg: &'a Registry,
    places: HashMap<&'a str, Place>,
    node_of_device: HashMap<&'a str, &'a Node>,
}

fn host_part(s: &str) -> String {
    s.split('.').next().unwrap_or(s).trim().to_ascii_lowercase()
}

fn push_at(list: &mut Vec<Attach>, a: Attach) {
    if !list.iter().any(|x| x.switch_id == a.switch_id && x.n == a.n && x.port == a.port) {
        list.push(a);
    }
}

impl<'a> Index<'a> {
    pub fn new(reg: &'a Registry, cx: &Ctx) -> Index<'a> {
        let node_of_device: HashMap<&str, &Node> = reg.devices.iter().filter_map(|d| Some((d.id.as_str(), reg.nodes.iter().chain(reg.gone.iter()).find(|n| n.id == d.node_id)?))).collect();
        // Media addresses of each node: the sources of its senders' SDPs.
        let mut media: HashMap<&str, Vec<Ipv4Addr>> = HashMap::new();
        for s in &reg.senders {
            let Some(n) = node_of_device.get(s.device_id.as_str()) else { continue };
            for ip in reg.medias(&s.id).iter().filter_map(|m| m.source) {
                let v = media.entry(n.id.as_str()).or_default();
                if !v.contains(&ip) {
                    v.push(ip);
                }
            }
        }
        let chassis = chassis_of_switches(cx);
        let inventory_macs: HashSet<String> = cx.inp.addresses.values().filter_map(|a| a.mac.as_deref().and_then(norm_mac)).collect();
        let places = reg.nodes.iter().chain(reg.gone.iter()).map(|n| (n.id.as_str(), place(n, media.get(n.id.as_str()).map(Vec::as_slice).unwrap_or(&[]), cx, &chassis, &inventory_macs))).collect();
        Index { reg, places, node_of_device }
    }

    pub fn node(&self, device_id: &str) -> Option<&'a Node> {
        self.node_of_device.get(device_id).copied()
    }

    pub fn place(&self, node_id: &str) -> Option<&Place> {
        self.places.get(node_id)
    }
}

/// Chassis IDs of the inventory switches, as their neighbours see them: chassis → switch id.
fn chassis_of_switches(cx: &Ctx) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for sw in &cx.inp.switches {
        for (_, nb) in &sw.neighbours {
            let other = cx.inp.switches.iter().find(|o| {
                o.id != sw.id
                    && ((!nb.remote_name.is_empty() && (host_part(&o.name) == host_part(&nb.remote_name) || o.name.eq_ignore_ascii_case(&nb.remote_name)))
                        || (nb.remote_ip.is_some() && nb.remote_ip == o.ip))
            });
            if let Some(o) = other {
                let key = norm_mac(&nb.chassis_id).unwrap_or_else(|| nb.chassis_id.trim().to_ascii_lowercase());
                if !key.is_empty() {
                    out.insert(key, o.id.clone());
                }
            }
        }
    }
    out
}

/// Places a node: the inventory switch port of its addresses, the LLDP neighbour
/// tables of the switches (its chassis or port MAC, its address, its host name),
/// its v1.3 `attached_network_device`, then the switches' MAC tables.
fn place(n: &Node, media: &[Ipv4Addr], cx: &Ctx, chassis: &HashMap<String, String>, inventory_macs: &HashSet<String>) -> Place {
    let mut p = Place::default();
    for ip in n.ips.iter().chain(media.iter()) {
        if !p.ips.contains(ip) {
            p.ips.push(*ip);
        }
    }
    let mut macs: Vec<String> = n.macs.clone();
    for ip in &p.ips {
        if let Some(m) = cx.mac_of(*ip).as_deref().and_then(norm_mac) {
            if !macs.contains(&m) {
                macs.push(m);
            }
        }
    }
    let addrs = &cx.inp.addresses;
    p.name = p.ips.iter().find_map(|ip| cx.addr_name(*ip));
    p.inventory = p.ips.iter().any(|ip| addrs.contains_key(ip)) || macs.iter().any(|m| inventory_macs.contains(m));
    // 1. Set by hand on the address.
    for ip in &p.ips {
        if let Some(a) = cx.locate(Some(*ip), None) {
            p.at_by_ip.entry(*ip).or_insert_with(|| a.clone());
            push_at(&mut p.at, a);
        }
    }
    // 2. LLDP neighbours of the switches (edge ports only).
    let names: Vec<String> = [n.hostname.as_deref(), Some(n.label.as_str())].into_iter().flatten().map(host_part).filter(|s| !s.is_empty()).collect();
    for sw in &cx.inp.switches {
        for (at, nb) in &sw.neighbours {
            let port = at.n.and_then(|x| sw.ports.iter().find(|q| q.n == x));
            if port.is_some_and(|q| q.peer.is_some()) {
                continue;
            }
            let by_mac = norm_mac(&nb.chassis_id).is_some_and(|m| macs.contains(&m));
            let by_ip = nb.remote_ip.filter(|ip| p.ips.contains(ip));
            let by_name = !nb.remote_name.is_empty() && names.contains(&host_part(&nb.remote_name));
            if !(by_mac || by_ip.is_some() || by_name) {
                continue;
            }
            let a = Attach { switch_id: sw.id.clone(), switch_name: sw.name.clone(), n: at.n, port: port.map(|q| q.name.clone()).filter(|x| !x.is_empty()).unwrap_or_else(|| at.name.clone()), edge: true, via: "lldp" };
            if let Some(ip) = by_ip {
                p.at_by_ip.entry(ip).or_insert_with(|| a.clone());
            }
            push_at(&mut p.at, a);
        }
    }
    // 3. The switch port the node says it is attached to (v1.3).
    for (ch, port_id) in n.interfaces.iter().filter_map(|i| i.attached.as_ref()) {
        let key = norm_mac(ch).unwrap_or_else(|| ch.trim().to_ascii_lowercase());
        let Some(sw) = chassis.get(&key).and_then(|id| cx.by_id.get(id.as_str())) else { continue };
        let Some(q) = sw.ports.iter().find(|q| !q.name.is_empty() && strata_net::switchinfo::same_port(&q.name, port_id)) else { continue };
        push_at(&mut p.at, Attach { switch_id: sw.id.clone(), switch_name: sw.name.clone(), n: Some(q.n), port: q.name.clone(), edge: q.peer.is_none(), via: "lldp" });
    }
    // 4. MAC tables.
    for ip in &p.ips {
        if p.at_by_ip.contains_key(ip) {
            continue;
        }
        if let Some(a) = cx.mac_of(*ip).and_then(|m| cx.locate(None, Some(&m))) {
            p.at_by_ip.insert(*ip, a.clone());
            push_at(&mut p.at, a);
        }
    }
    for m in &macs {
        if let Some(a) = cx.locate(None, Some(m)) {
            push_at(&mut p.at, a);
        }
    }
    p
}

impl Flow {
    /// A flow known from nothing yet.
    fn empty(group: Ipv4Addr, port: u16, source: Option<Ipv4Addr>, now: u64) -> Flow {
        Flow {
            key: stream_key(group, port, source),
            group,
            port,
            source,
            kind: Kind::Other,
            encoding: None,
            name: None,
            label: String::new(),
            group_label: String::new(),
            sap_name: None,
            announced: false,
            routed: false,
            flooded: false,
            flood_vlan: None,
            snoop_only: false,
            guessed: false,
            ambiguous: false,
            live: false,
            state: "ok",
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
            vlan: None,
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
            origins: Vec::new(),
            nmos: None,
            idle: false,
            subscribers: Vec::new(),
        }
    }
}

/// The kind of an NMOS format ("urn:x-nmos:format:audio").
fn kind_of(format: Option<&str>) -> Option<Kind> {
    match format?.rsplit(':').next()? {
        "video" => Some(Kind::Video),
        "audio" => Some(Kind::Audio),
        "data" => Some(Kind::Data),
        "mux" => Some(Kind::MpegTs),
        _ => None,
    }
}

pub(super) fn add_origin(f: &mut Flow, o: &'static str) {
    if !f.origins.contains(&o) {
        f.origins.push(o);
        f.origins.sort_by_key(|x| ["nmos", "sap", "igmp"].iter().position(|y| y == x));
    }
}

/// The registry's senders as flows. Each multicast media of a sender's SDP (two for
/// an ST 2022-7 pair) goes to the flow of the same group and port (the same source
/// first), else makes a new one. Liveness: the sender is in the registry and not
/// turned off (`subscription.active` not false), or what made the flow live already
/// (SAP, packets, router tables); a sender turned off is `idle`, not gone.
pub(super) fn merge_senders(flows: &mut Vec<Flow>, idx: &Index, cx: &Ctx) {
    let inp = cx.inp;
    let reg = idx.reg;
    let now = inp.now;
    for s in &reg.senders {
        let node = idx.node(&s.device_id);
        let place = node.and_then(|n| idx.place(&n.id));
        let nref = NmosRef {
            sender_id: s.id.clone(),
            sender: s.label.clone(),
            node_id: node.map(|n| n.id.clone()),
            node: node.map(|n| n.label.clone()),
            inventory: place.is_some_and(|p| p.inventory),
            active: s.active,
        };
        let on = s.active != Some(false);
        let kind = kind_of(reg.sender_format(s));
        for m in reg.medias(&s.id) {
            let Some(group) = m.group.filter(|g| g.is_multicast() && !control_group(*g)) else { continue };
            let free: Vec<usize> = flows.iter().enumerate().filter(|(_, f)| f.group == group && f.port == m.port && f.nmos.is_none()).map(|(i, _)| i).collect();
            let pick = free
                .iter()
                .copied()
                .find(|i| m.source.is_some() && flows[*i].source == m.source)
                .or_else(|| free.iter().copied().find(|i| flows[*i].source.is_none()))
                .or_else(|| free.first().copied().filter(|_| m.source.is_none()));
            let i = match pick {
                Some(i) => i,
                None => {
                    let key = stream_key(group, m.port, m.source);
                    if flows.iter().any(|f| f.key == key) {
                        // Two senders of one group, port and source: the first one is kept.
                        continue;
                    }
                    let mut f = Flow::empty(group, m.port, m.source, now);
                    f.label = inp.labels.get(&key).cloned().unwrap_or_default();
                    f.group_label = inp.more.group_labels.get(&group).cloned().unwrap_or_default();
                    f.first_seen = reg.at.max(0) as u64;
                    f.vlan = m.source.and_then(|ip| cx.vlan_of_ip(ip)).or_else(|| inp.more.ranges.iter().find(|r| in_range(r, group)).and_then(|r| r.vlan));
                    flows.push(f);
                    flows.len() - 1
                }
            };
            let f = &mut flows[i];
            add_origin(f, "nmos");
            f.nmos = Some(nref.clone());
            if f.label.is_empty() && f.group_label.is_empty() && !s.label.is_empty() {
                f.name = Some(s.label.clone());
            }
            if f.source.is_none() {
                f.source = m.source;
            }
            if f.format.is_none() {
                f.format.clone_from(&m.format);
                f.est_bps = m.format.as_ref().and_then(Format::est_bps);
            }
            if f.est_bps.is_none() {
                f.est_bps = m.format.as_ref().and_then(Format::est_bps);
            }
            if f.encoding.is_none() {
                f.encoding.clone_from(&m.encoding);
            }
            if matches!(f.kind, Kind::Other | Kind::Rtp) {
                if let Some(k) = m.kind.or(kind) {
                    f.kind = k;
                }
            }
            if f.dup.is_none() {
                f.dup.clone_from(&m.dup);
            }
            f.announced = true;
            f.last_seen = f.last_seen.max(reg.at.max(0) as u64);
            f.live |= on;
            f.idle = !f.live;
        }
    }
}

/// The receivers subscribed to the flow's sender; a snooping receiver on the
/// same port as one of them is that subscriber (named after its node when the
/// inventory has no name for it).
pub(super) fn subscribers(f: &mut Flow, idx: &Index) {
    let Some(sid) = f.nmos.as_ref().map(|n| n.sender_id.clone()) else { return };
    for r in idx.reg.receivers.iter().filter(|r| r.subscribed() == Some(sid.as_str())) {
        let node = idx.node(&r.device_id);
        let place = node.and_then(|n| idx.place(&n.id));
        let ats: &[Attach] = place.map(|p| p.at.as_slice()).unwrap_or(&[]);
        let hit = f.receivers.iter_mut().filter(|x| !x.router && x.peer.is_none()).find(|x| ats.iter().any(|a| a.switch_id == x.switch_id && a.n.is_some() && a.n == x.n));
        let igmp = hit.is_some();
        let mut at = ats.first().cloned();
        if let Some(x) = hit {
            at = ats.iter().find(|a| a.switch_id == x.switch_id && a.n == x.n).cloned();
            if x.host.is_none() {
                x.host = node.map(|n| place.and_then(|p| p.name.clone()).unwrap_or_else(|| n.label.clone()));
            }
        }
        if !f.subscribers.iter().any(|s| s.id == r.id) {
            f.subscribers.push(Subscriber { id: r.id.clone(), label: r.label.clone(), node: node.map(|n| place.and_then(|p| p.name.clone()).unwrap_or_else(|| n.label.clone())), at, igmp });
        }
    }
    f.subscribers.sort_by(|a, b| (&a.node, &a.label).cmp(&(&b.node, &b.label)));
}

/// The emitter of an NMOS flow: its node's name when the inventory has none, its
/// node's port when the source address was not placed.
pub(super) fn emitter(f: &mut Flow, idx: &Index) {
    let Some(node_id) = f.nmos.as_ref().and_then(|n| n.node_id.clone()) else { return };
    let node = idx.reg.nodes.iter().chain(idx.reg.gone.iter()).find(|n| n.id == node_id);
    let Some(place) = idx.place(&node_id) else { return };
    let label = node.map(|n| n.label.clone());
    match f.emitter.as_mut() {
        Some(e) => {
            if e.name.is_none() {
                e.name = place.name.clone().or(label);
            }
            if e.at.is_none() {
                e.at = place.at_by_ip.get(&e.ip).cloned().or_else(|| (place.at.len() == 1).then(|| place.at[0].clone()));
            }
        }
        None => {
            if let Some(ip) = place.ips.first() {
                f.emitter = Some(Emitter { ip: *ip, name: place.name.clone().or(label), mac: None, at: place.at.first().cloned() });
            }
        }
    }
}

/// The nodes of the registry, placed: the NMOS page.
pub fn nodes(inp: &Input) -> Vec<NodeView> {
    let Some(reg) = inp.nmos.as_deref() else { return Vec::new() };
    let cx = Ctx::new(inp);
    let idx = Index::new(reg, &cx);
    let mut out: Vec<NodeView> = reg
        .nodes
        .iter()
        .map(|n| (n, true))
        .chain(reg.gone.iter().map(|n| (n, false)))
        .map(|(n, present)| {
            let devices: Vec<&crate::net::nmos::Device> = reg.devices.iter().filter(|d| d.node_id == n.id).collect();
            let mine = |id: &str| devices.iter().any(|d| d.id == id);
            let p = idx.place(&n.id).cloned().unwrap_or_default();
            NodeView {
                id: n.id.clone(),
                label: n.label.clone(),
                hostname: n.hostname.clone(),
                maker: n.maker(&devices),
                ip: n.ips.first().copied().or_else(|| p.ips.first().copied()),
                ips: p.ips,
                name: p.name,
                senders: reg.senders.iter().filter(|s| mine(&s.device_id)).count(),
                receivers: reg.receivers.iter().filter(|r| mine(&r.device_id)).count(),
                at: p.at,
                inventory: p.inventory,
                seen: reg.seen.get(&n.id).copied().unwrap_or(reg.at),
                present,
            }
        })
        .collect();
    out.sort_by(|a, b| b.present.cmp(&a.present).then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase())));
    out
}

#[cfg(test)]
mod tests;
