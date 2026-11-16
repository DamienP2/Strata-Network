use super::*;
use crate::net::nmos::{Device, Essence, Iface, Receiver as NReceiver, SdpEntry, Sender};
use strata_net::mcast::parse_sdp;
use strata_net::switchinfo::{PortNeighbour, Proto};

fn ip(s: &str) -> Ipv4Addr {
    s.parse().unwrap()
}

fn switch(id: &str, name: &str, n: u16) -> SwitchData {
    let mut s: crate::net::model::Switch =
        serde_json::from_value(serde_json::json!({"id": id, "name": name, "groups": [{"kind": "sfp", "count": n, "naming": "Eth1/{n}"}]})).unwrap();
    s.normalize();
    SwitchData { id: id.into(), name: name.into(), ports: s.ports, online: true, read: true, ..SwitchData::default() }
}

fn at(n: u16) -> PortAt {
    PortAt { n: Some(n), name: format!("Ethernet1/{n}") }
}

fn lldp(n: u16, name: &str, chassis: &str, remote_ip: Option<&str>) -> (PortAt, PortNeighbour) {
    (at(n), PortNeighbour { if_index: u32::from(n), remote_name: name.into(), remote_port: "eth0".into(), remote_ip: remote_ip.map(ip), platform: String::new(), chassis_id: chassis.into(), protocol: Proto::Lldp })
}

const DUP: &str = "v=0\r\no=- 1 1 IN IP4 10.10.1.7\r\ns=BIDI4-01 SDI 1\r\nt=0 0\r\na=group:DUP primary secondary\r\n\
m=video 5004 RTP/AVP 96\r\nc=IN IP4 239.120.1.1/64\r\na=source-filter: incl IN IP4 239.120.1.1 10.10.1.7\r\na=rtpmap:96 raw/90000\r\n\
a=fmtp:96 sampling=YCbCr-4:2:2; width=1920; height=1080; exactframerate=50; depth=10\r\na=mid:primary\r\n\
m=video 5004 RTP/AVP 96\r\nc=IN IP4 239.130.1.1/64\r\na=source-filter: incl IN IP4 239.130.1.1 10.20.1.7\r\na=rtpmap:96 raw/90000\r\n\
a=fmtp:96 sampling=YCbCr-4:2:2; width=1920; height=1080; exactframerate=50; depth=10\r\na=mid:secondary\r\n";

fn single(group: &str, src: &str, media: &str) -> String {
    format!("v=0\r\no=- 1 1 IN IP4 {src}\r\ns=x\r\nt=0 0\r\n{media}\r\nc=IN IP4 {group}/64\r\na=source-filter: incl IN IP4 {group} {src}\r\n")
}

fn audio_sdp() -> String {
    single("239.120.2.1", "10.10.1.7", "m=audio 5004 RTP/AVP 97\r\na=rtpmap:97 L24/48000/16\r\na=ptime:1")
}

fn video_sdp(group: &str) -> String {
    single(group, "10.10.1.7", "m=video 5004 RTP/AVP 96\r\na=rtpmap:96 raw/90000\r\na=fmtp:96 sampling=YCbCr-4:2:2; width=1920; height=1080; exactframerate=50; depth=10")
}

fn node(id: &str, label: &str, api: &str, port_mac: &str) -> Node {
    Node {
        id: id.into(),
        version: "1:0".into(),
        label: label.into(),
        hostname: Some(format!("{label}.studio.lan")),
        ips: vec![ip(api)],
        macs: crate::net::nmos::norm_mac(port_mac).into_iter().collect(),
        interfaces: vec![Iface { name: "eth0".into(), chassis_id: None, port_id: Some(port_mac.into()), attached: None }],
        tags: vec![("urn:x-nmos:tag:asset:manufacturer/v1.0".into(), vec!["Cobalt".into()]), ("urn:x-nmos:tag:asset:product/v1.0".into(), vec!["BIDI4".into()])],
    }
}

fn snd(id: &str, label: &str, active: Option<bool>, sdp: &str, reg: &mut Registry) {
    reg.senders.push(Sender { id: id.into(), version: "1:0".into(), label: label.into(), flow_id: Some(format!("f-{id}")), device_id: "dA".into(), transport: "urn:x-nmos:transport:rtp.mcast".into(), manifest_href: Some(format!("http://10.1.1.7/{id}.sdp")), active });
    reg.flows.push(Essence { id: format!("f-{id}"), format: if sdp.contains("m=audio") { "urn:x-nmos:format:audio".into() } else { "urn:x-nmos:format:video".into() }, source_id: None });
    reg.sdp.insert(id.into(), SdpEntry { version: "1:0".into(), href: String::new(), at: 0, text: Some(sdp.into()), medias: parse_sdp(sdp), error: None });
}

fn rcv(id: &str, label: &str, sender: Option<&str>, active: Option<bool>) -> NReceiver {
    NReceiver { id: id.into(), version: "1:0".into(), label: label.into(), device_id: "dB".into(), format: "urn:x-nmos:format:video".into(), sender_id: sender.map(str::to_string), active }
}

/// cobalt-bidi4-01 (10.1.1.7) on SW2110-01 Eth1/12 (LLDP by address), multiviewer-01
/// (10.1.1.40) on Eth1/33 (LLDP by MAC), dante-bridge-01 (10.1.1.90) unknown to the
/// inventory, its MAC learnt on Eth1/7; a node on Eth1/5 that says so (v1.3); SW2110-02
/// sees SW2110-01's chassis on its Eth1/1.
fn registry(s3_active: bool) -> Registry {
    let mut reg = Registry { at: 1_000_000, ..Registry::default() };
    reg.nodes = vec![
        node("nA", "cobalt-bidi4-01", "10.1.1.7", "74-26-ac-00-01-03"),
        node("nB", "multiviewer-01", "10.1.1.40", "74-26-ac-00-02-03"),
        Node { tags: vec![], ..node("nC", "dante-bridge-01", "10.1.1.90", "00-1d-c1-00-00-90") },
        Node { tags: vec![], interfaces: vec![Iface { name: "eth0".into(), chassis_id: None, port_id: None, attached: Some(("70-69-5a-ee-6f-79".into(), "Ethernet1/5".into())) }], ..node("nD", "gw-05", "10.1.1.50", "00-00-00-00-00-00") },
    ];
    reg.devices = vec![
        Device { id: "dA".into(), version: "1:0".into(), label: "BIDI4".into(), node_id: "nA".into(), tags: vec![] },
        Device { id: "dB".into(), version: "1:0".into(), label: "MV".into(), node_id: "nB".into(), tags: vec![] },
    ];
    snd("s1", "BIDI4-01 · SDI 1 → IP", Some(true), DUP, &mut reg);
    snd("s2", "BIDI4-01 · SDI 1 audio", Some(true), &audio_sdp(), &mut reg);
    snd("s3", "BIDI4-01 · SDI 2 → IP", Some(s3_active), &video_sdp("239.120.1.2"), &mut reg);
    reg.receivers = vec![
        rcv("r1", "MV in 1", Some("s1"), Some(true)),
        rcv("r2", "MV audio", Some("s2"), Some(false)),
        // v1.1: no `active`.
        rcv("r3", "MV in 2", Some("s3"), None),
        rcv("r4", "MV in 3", None, Some(false)),
    ];
    reg
}

fn input(reg: Registry) -> Input {
    let now = 1_000_000;
    let mut a = switch("s1", "SW2110-01", 40);
    let mut b = switch("s2", "SW2110-02", 8);
    a.neighbours = vec![lldp(12, "BIDI4", "", Some("10.1.1.7")), lldp(33, "mv", "74:26:AC:00:02:03", None)];
    b.neighbours = vec![lldp(1, "SW2110-01.studio.lan", "70:69:5A:EE:6F:79", None)];
    a.ports[6].learnt = vec!["00:1D:C1:00:00:90".into()];
    let g = ip("239.120.1.1");
    a.snoop = vec![Snoop { vlan: 10, group: Some(g), mac: group_mac(g), ports: vec![at(33)] }];
    let mut addresses = HashMap::new();
    addresses.insert(ip("10.1.1.7"), Addr { name: Some("cobalt-bidi4-01".into()), mac: None, switch: None });
    addresses.insert(ip("10.1.1.40"), Addr { name: Some("multiviewer-01".into()), mac: None, switch: None });
    // SAP announces the audio (not the video).
    let sdp = &parse_sdp(&audio_sdp().replace("s=x", "s=SDI 1 audio"))[0];
    let mut audio: Stream = serde_json::from_value(serde_json::json!({
        "group": "239.120.2.1", "port": 5004, "source": "10.10.1.7", "kind": "audio", "bps": 0.0, "pps": 0.0, "packets": 0,
        "firstSeen": now, "lastSeen": now, "history": [], "announced": true
    }))
    .unwrap();
    audio.name = sdp.name.clone();
    audio.encoding = Some("L24".into());
    audio.format = sdp.format.clone();
    audio.last_announced = Some(now);
    Input { now, streams: vec![audio], switches: vec![a, b], addresses, nmos: Some(Arc::new(reg)), ..Input::default() }
}

fn flow<'a>(v: &'a FlowsView, g: &str) -> &'a Flow {
    v.flows.iter().find(|f| f.group == ip(g)).unwrap_or_else(|| panic!("{g}"))
}

#[test]
fn senders_become_flows_merged_by_origin() {
    let v = build(&input(registry(true)));
    // One flow a group and port: the SAP audio and the NMOS sender are one.
    assert_eq!(v.flows.iter().filter(|f| f.group == ip("239.120.2.1")).count(), 1);
    let a = flow(&v, "239.120.2.1");
    assert_eq!(a.origins, vec!["nmos", "sap"]);
    assert_eq!(a.name.as_deref(), Some("BIDI4-01 · SDI 1 audio"));
    assert_eq!(a.sap_name.as_deref(), Some("SDI 1 audio"));
    assert_eq!(a.nmos.as_ref().map(|n| (n.sender_id.as_str(), n.node.as_deref(), n.inventory)), Some(("s2", Some("cobalt-bidi4-01"), true)));
    // Video only in NMOS: a flow anyway, announced (by the registry), live, its format and rate from the SDP.
    let r = flow(&v, "239.120.1.1");
    assert_eq!((r.announced, r.live, r.kind), (true, true, Kind::Video));
    assert!(!r.issues.contains(&"unannounced"));
    assert!((r.est_bps.unwrap() / 1e9 - 2.177).abs() < 0.01);
    // Its snooping entry adds the IGMP origin.
    assert_eq!(r.origins, vec!["nmos", "igmp"]);
    // Emitter: the inventory name, on the port LLDP gives for its node.
    let e = r.emitter.as_ref().unwrap();
    assert_eq!((e.ip, e.name.as_deref()), (ip("10.10.1.7"), Some("cobalt-bidi4-01")));
    assert_eq!(e.at.as_ref().map(|a| (a.switch_name.as_str(), a.port.as_str(), a.via)), Some(("SW2110-01", "Eth1/12", "lldp")));
    assert_eq!(v.summary.unannounced, 0);
}

#[test]
fn st2022_7_from_one_sender() {
    let v = build(&input(registry(true)));
    let (red, blue) = (flow(&v, "239.120.1.1"), flow(&v, "239.130.1.1"));
    assert_eq!((red.pair_by, red.leg, red.pair_state), (Some("sdp"), Some("a"), Some("ok")));
    assert_eq!((blue.pair_by, blue.leg), (Some("sdp"), Some("b")));
    assert_eq!(red.pair.as_ref().map(|p| p.group), Some(ip("239.130.1.1")));
    assert_eq!(red.name, blue.name);
    assert_eq!(blue.origins, vec!["nmos"]);
    assert_eq!(blue.source, Some(ip("10.20.1.7")));
    assert_eq!(v.summary.pairs, 1);
    // Two senders named like legs ("… 1", "… 2") are two streams, not a pair.
    let mut reg = registry(true);
    snd("s8", "MV · sortie 1", Some(true), &video_sdp("239.140.1.1"), &mut reg);
    snd("s9", "MV · sortie 2", Some(true), &video_sdp("239.140.1.2"), &mut reg);
    let v = build(&input(reg));
    assert_eq!((flow(&v, "239.140.1.1").pair_state, v.summary.pairs), (None, 1));
}

#[test]
fn subscriptions_are_receivers() {
    let v = build(&input(registry(true)));
    let red = flow(&v, "239.120.1.1");
    // The multiviewer is subscribed and joined on Eth1/33: one receiver, named after the node.
    assert_eq!(red.subscribers.len(), 1);
    let s = &red.subscribers[0];
    assert_eq!((s.id.as_str(), s.node.as_deref(), s.igmp), ("r1", Some("multiviewer-01"), true));
    assert_eq!(s.at.as_ref().map(|a| a.port.as_str()), Some("Eth1/33"));
    assert_eq!(red.receiver_count, 1);
    assert_eq!(red.receivers[0].host.as_deref(), Some("multiviewer-01"));
    // Blue leg: subscribed, nothing in the snooping tables.
    let blue = flow(&v, "239.130.1.1");
    assert_eq!((blue.subscribers.len(), blue.subscribers[0].igmp, blue.receiver_count), (1, false, 1));
    assert!(!blue.issues.contains(&"noReceivers"));
    // A receiver turned off is not subscribed; one of v1.1 (no `active`) is.
    assert!(flow(&v, "239.120.2.1").subscribers.is_empty());
    assert_eq!(flow(&v, "239.120.1.2").subscribers.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["r3"]);
}

#[test]
fn idle_senders_are_not_gone() {
    let v = build(&input(registry(false)));
    let f = flow(&v, "239.120.1.2");
    assert_eq!((f.live, f.idle, f.state), (false, true, "off"));
    assert!(!f.issues.contains(&"gone"));
    // Alerts: turned off is not gone; removed from the registry is.
    let mut w = Watch::default();
    let on = build(&input(registry(true)));
    assert!(evaluate(&mut w, &on, 1, 1_000_000).0.is_empty());
    let (alerts, _) = evaluate(&mut w, &v, 1, 1_005_000);
    assert!(alerts.iter().all(|a| a.0 != "streamGone"), "{alerts:?}");
    let mut reg = registry(true);
    reg.senders.retain(|s| s.id != "s1");
    let gone = build(&input(reg));
    let (alerts, _) = evaluate(&mut w, &gone, 1, 1_010_000);
    let words: Vec<&str> = alerts.iter().filter(|a| a.0 == "streamGone").map(|a| a.2.as_str()).collect();
    assert!(words.iter().any(|w| w.starts_with("239.130.1.1:5004")), "{words:?}");
    // The registry dropped (turned off, another address): a new generation, no alert.
    let (alerts, _) = evaluate(&mut w, &build(&Input { nmos: None, ..input(registry(true)) }), 2, 1_015_000);
    assert!(alerts.is_empty(), "{alerts:?}");
}

#[test]
fn nodes_are_placed() {
    let list = nodes(&input(registry(true)));
    let by = |id: &str| list.iter().find(|n| n.id == id).unwrap();
    let a = by("nA");
    assert_eq!((a.maker.as_deref(), a.ip, a.senders, a.receivers, a.inventory, a.present), (Some("Cobalt · BIDI4"), Some(ip("10.1.1.7")), 3, 0, true, true));
    assert_eq!(a.at.first().map(|x| (x.port.as_str(), x.via)), Some(("Eth1/12", "lldp")));
    // Media addresses come from its SDPs.
    assert!(a.ips.contains(&ip("10.20.1.7")));
    let b = by("nB");
    assert_eq!((b.receivers, b.at.first().map(|x| x.port.as_str())), (4, Some("Eth1/33")));
    // Unknown to the inventory, found in a MAC table.
    let c = by("nC");
    assert_eq!((c.inventory, c.at.first().map(|x| (x.port.as_str(), x.via))), (false, Some(("Eth1/7", "macTable"))));
    // v1.3 attached_network_device: SW2110-01's chassis, as SW2110-02 sees it.
    let d = by("nD");
    assert_eq!(d.at.first().map(|x| (x.switch_name.as_str(), x.port.as_str())), Some(("SW2110-01", "Eth1/5")));
    assert!(nodes(&Input::default()).is_empty());
}
