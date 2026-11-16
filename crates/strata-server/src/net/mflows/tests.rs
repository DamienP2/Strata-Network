use super::*;
use strata_net::mcast::{parse_sdp, Leg};

fn ip(s: &str) -> Ipv4Addr {
    s.parse().unwrap()
}

fn sw(id: &str, name: &str, n: u16) -> SwitchData {
    let mut s: super::super::model::Switch =
        serde_json::from_value(serde_json::json!({"id": id, "name": name, "groups": [{"kind": "sfp", "count": n, "naming": "Eth1/{n}"}]})).unwrap();
    s.normalize();
    SwitchData { id: id.into(), name: name.into(), ports: s.ports, online: true, read: true, ..SwitchData::default() }
}

fn link(a: &mut SwitchData, pa: u16, b: &mut SwitchData, pb: u16) {
    a.ports[usize::from(pa) - 1].peer = Some(PortRef { id: b.id.clone(), port: pb });
    b.ports[usize::from(pb) - 1].peer = Some(PortRef { id: a.id.clone(), port: pa });
}

fn at(n: u16) -> PortAt {
    PortAt { n: Some(n), name: format!("Ethernet1/{n}") }
}

fn stream(group: &str, port: u16, source: Option<&str>, name: Option<&str>, now: u64) -> Stream {
    let sdp = format!(
        "v=0\r\no=- 1 1 IN IP4 {src}\r\ns={name}\r\nt=0 0\r\nm=video {port} RTP/AVP 96\r\nc=IN IP4 {group}/64\r\na=rtpmap:96 raw/90000\r\n\
         a=fmtp:96 sampling=YCbCr-4:2:2; width=1920; height=1080; exactframerate=50; depth=10\r\n",
        src = source.unwrap_or("10.0.0.1"),
        name = name.unwrap_or("-"),
    );
    let m = &parse_sdp(&sdp)[0];
    let mut s: Stream = serde_json::from_value(serde_json::json!({
        "group": group, "port": port, "source": source, "kind": "video", "bps": 0.0, "pps": 0.0, "packets": 0,
        "firstSeen": now, "lastSeen": now, "history": [], "announced": name.is_some()
    }))
    .unwrap();
    s.encoding = Some("raw".into());
    s.format = m.format.clone();
    s.est_bps = s.format.as_ref().and_then(Format::est_bps);
    s.name = name.map(str::to_string);
    if name.is_some() {
        s.last_announced = Some(now);
    }
    s
}

/// Emitter on SW-A Eth1/1; SW-A Eth1/4 — SW-Core Eth1/1; SW-Core Eth1/2 — SW-B Eth1/4; receivers on SW-B Eth1/2, Eth1/3.
fn fabric() -> Input {
    let now = 1_000_000;
    let (mut a, mut core, mut b) = (sw("a", "SW-A", 4), sw("c", "SW-Core", 4), sw("b", "SW-B", 4));
    link(&mut a, 4, &mut core, 1);
    link(&mut core, 2, &mut b, 4);
    a.ports[0].learnt = vec!["00:11:22:33:44:55".into()];
    // The emitter's MAC is learnt on the uplink too: the edge port wins.
    core.ports[0].learnt = vec!["00:11:22:33:44:55".into(), "AA:AA:AA:AA:AA:AA".into()];
    b.ports[1].host = Some(ip("10.10.1.40"));
    b.ports[2].label = "Multiviewer".into();
    // The MAC table put a host on the uplink: not shown on a switch link.
    b.ports[3].host = Some(ip("10.10.1.11"));
    let g = ip("239.10.1.1");
    b.snoop = vec![Snoop { vlan: 110, group: None, mac: group_mac(g), ports: vec![at(2), at(3), at(4)] }];
    b.router = vec![(110, vec![at(4)])];
    core.snoop = vec![Snoop { vlan: 110, group: Some(g), mac: group_mac(g), ports: vec![at(2)] }];
    // Unannounced group seen in snooping only (its MAC, no group read).
    a.snoop = vec![Snoop { vlan: 110, group: None, mac: group_mac(ip("239.69.4.10")), ports: vec![at(2)] }];
    let mut addresses = HashMap::new();
    addresses.insert(ip("10.10.1.11"), Addr { name: Some("cam-01-red".into()), mac: Some("00:11:22:33:44:55".into()), switch: None });
    addresses.insert(ip("10.10.1.40"), Addr { name: Some("mixer-a".into()), mac: None, switch: None });
    let mut more = McastMore::default();
    more.ranges.push(Range { id: "r1".into(), cidr: "239.10.1.0/24".into(), usage: "Caméras Rouge".into(), vlan: Some(110) });
    more.ranges.push(Range { id: "r2".into(), cidr: "239.20.1.0/24".into(), usage: "Caméras Bleu".into(), vlan: Some(120) });
    more.reservations.push(Reservation { group: ip("239.10.1.3"), name: "CAM-03".into() });
    Input {
        now,
        streams: vec![
            stream("239.10.1.1", 5004, Some("10.10.1.11"), Some("CAM-01 Vidéo Rouge"), now),
            stream("239.20.1.1", 5004, Some("10.20.1.11"), Some("CAM-01 Vidéo Bleu"), now),
            stream("239.10.1.2", 5004, Some("10.10.1.12"), Some("CAM-02 Vidéo Rouge"), now),
        ],
        switches: vec![a, core, b],
        addresses,
        vlan_nets: vec![(110, ip("10.10.1.0"), 24), (120, ip("10.20.1.0"), 24)],
        more,
        ..Input::default()
    }
}

#[test]
fn receivers_emitter_and_path() {
    let v = build(&fabric());
    let f = v.flows.iter().find(|f| f.group == ip("239.10.1.1")).unwrap();
    // SW-B: Eth1/2 (mixer-a), Eth1/3 (Multiviewer), Eth1/4 (router port, link to the core); SW-Core Eth1/2 (link to SW-B).
    assert_eq!(f.receivers.len(), 4);
    assert_eq!(f.receiver_count, 2);
    let mixer = f.receivers.iter().find(|r| r.n == Some(2) && r.switch_id == "b").unwrap();
    assert_eq!((mixer.host.as_deref(), mixer.port.as_str(), mixer.router), (Some("mixer-a"), "Eth1/2", false));
    let up = f.receivers.iter().find(|r| r.n == Some(4)).unwrap();
    assert_eq!((up.router, up.peer.as_deref()), (true, Some("SW-Core")));
    assert_eq!(up.host, None);
    assert_eq!(f.receivers.iter().find(|r| r.n == Some(3)).unwrap().label.as_deref(), Some("Multiviewer"));
    let e = f.emitter.as_ref().unwrap();
    assert_eq!(e.name.as_deref(), Some("cam-01-red"));
    let at = e.at.as_ref().unwrap();
    assert_eq!((at.switch_id.as_str(), at.n, at.edge, at.via), ("a", Some(1), true, "macTable"));
    // Path: SW-A → (Eth1/4) SW-Core → (Eth1/2) SW-B.
    let hops: Vec<(&str, Option<&str>, Option<&str>)> = f.path.iter().map(|n| (n.switch_id.as_str(), n.parent.as_deref(), n.egress.as_deref())).collect();
    assert_eq!(hops, vec![("a", None, None), ("c", Some("a"), Some("Eth1/4")), ("b", Some("c"), Some("Eth1/2"))]);
    assert_eq!(f.path[0].ingress.as_deref(), Some("Eth1/1"));
    assert_eq!(f.path[2].ingress.as_deref(), Some("Eth1/4"));
    assert_eq!((f.path[2].igmp_ports, f.path[2].receivers), (3, 2));
    assert!(f.unreachable.is_empty());
    assert_eq!(v.links.len(), 2);
}

#[test]
fn path_bfs_shortest_and_aggregates() {
    let mut inp = fabric();
    // A direct link SW-A Eth1/3 — SW-B Eth1/1, as an aggregate on SW-A: shorter than through the core.
    let (a, rest) = inp.switches.split_at_mut(1);
    link(&mut a[0], 3, &mut rest[1], 1);
    a[0].lag_of.insert(3, "port-channel7".into());
    let v = build(&inp);
    let f = v.flows.iter().find(|f| f.group == ip("239.10.1.1")).unwrap();
    let hops: Vec<(&str, Option<&str>)> = f.path.iter().map(|n| (n.switch_id.as_str(), n.egress.as_deref())).collect();
    // SW-Core has snooping entries for the group: it is reached directly too.
    assert_eq!(hops, vec![("a", None), ("b", Some("port-channel7")), ("c", Some("Eth1/4"))]);
    // A switch with no link: unreachable.
    let mut lone = sw("z", "SW-Lone", 2);
    lone.snoop = vec![Snoop { vlan: 110, group: Some(ip("239.10.1.1")), mac: group_mac(ip("239.10.1.1")), ports: vec![at(1)] }];
    inp.switches.push(lone);
    let v = build(&inp);
    let f = v.flows.iter().find(|f| f.group == ip("239.10.1.1")).unwrap();
    assert_eq!(f.unreachable, vec!["z".to_string()]);
}

#[test]
fn unannounced_and_ambiguous_groups() {
    let mut inp = fabric();
    let v = build(&inp);
    // 239.69.4.10 is in no range: guessed as 239.69.4.10 (239.x first).
    let u = v.flows.iter().find(|f| f.snoop_only).unwrap();
    assert_eq!((u.group, u.guessed, u.announced, u.state), (ip("239.69.4.10"), true, false, "unk"));
    assert!(u.issues.contains(&"unannounced"));
    // Named by hand: the MAC now matches a known group.
    inp.more.group_labels.insert(ip("239.69.4.10"), "Dante bridge".into());
    let v = build(&inp);
    let u = v.flows.iter().find(|f| f.snoop_only).unwrap();
    assert_eq!((u.guessed, u.name.as_deref()), (false, Some("Dante bridge")));
    // 239.138.1.1 has the same MAC as 239.10.1.1 (32:1). A stream on it in VLAN 120:
    // the VLAN 110 entry of SW-B goes to 239.10.1.1 only.
    inp.streams.push(stream("239.138.1.1", 5004, Some("10.20.1.99"), Some("Other"), inp.now));
    let v = build(&inp);
    let other = v.flows.iter().find(|f| f.group == ip("239.138.1.1")).unwrap();
    assert!(other.receivers.is_empty());
    // Same VLAN for both: both get it, marked ambiguous.
    inp.vlan_nets.push((110, ip("10.20.1.99"), 32));
    let v = build(&inp);
    let other = v.flows.iter().find(|f| f.group == ip("239.138.1.1")).unwrap();
    assert!(other.ambiguous && !other.receivers.is_empty());
    // Control groups never become flows (SSDP's MAC).
    inp.switches[0].snoop.push(Snoop { vlan: 20, group: None, mac: group_mac(ip("239.255.255.250")), ports: vec![at(1)] });
    assert!(build(&inp).flows.iter().all(|f| !control_group(f.group) && f.group != ip("239.127.255.250")));
}

#[test]
fn pairing_by_name_and_dup() {
    let v = build(&fabric());
    let red = v.flows.iter().find(|f| f.group == ip("239.10.1.1")).unwrap();
    let p = red.pair.as_ref().unwrap();
    assert_eq!((p.group, red.pair_by, red.leg, red.pair_state), (ip("239.20.1.1"), Some("name"), Some("a"), Some("ok")));
    let blue = v.flows.iter().find(|f| f.group == ip("239.20.1.1")).unwrap();
    assert_eq!((blue.leg, blue.pair.as_ref().unwrap().group), (Some("b"), ip("239.10.1.1")));
    // CAM-02 Rouge has no Blue leg.
    let lone = v.flows.iter().find(|f| f.group == ip("239.10.1.2")).unwrap();
    assert_eq!((lone.pair.is_none(), lone.pair_state, lone.state), (true, Some("missing"), "warn"));
    assert_eq!((v.summary.pairs, v.summary.pairs_incomplete), (2, 1));

    // Names: suffixes and families.
    assert_eq!(split_leg("CAM-01 Vidéo Rouge"), Some(("cam 01 vidéo".into(), "a", "color")));
    assert_eq!(split_leg("PGM_B"), Some(("pgm".into(), "b", "letter")));
    assert_eq!(split_leg("Mic 2"), Some(("mic".into(), "b", "digit")));
    assert_eq!(split_leg("Blue Room 1"), Some(("blue room".into(), "a", "digit")));
    assert_eq!(split_leg("A cam"), None);
    assert_eq!(split_leg("Rouge"), None);

    // Format differs: no pair.
    let mut inp = fabric();
    inp.streams[1].format.as_mut().unwrap().height = Some(720);
    let v = build(&inp);
    assert!(v.flows.iter().find(|f| f.group == ip("239.10.1.1")).unwrap().pair.is_none());

    // a=group:DUP pairs whatever the names; the leg not received any more is missing.
    let mut inp = fabric();
    inp.streams[0].name = Some("Main".into());
    inp.streams[1].name = Some("Other name".into());
    inp.streams[0].dup = Some(Leg { group: ip("239.20.1.1"), port: 5004, source: Some(ip("10.20.1.11")) });
    inp.streams[1].dup = Some(Leg { group: ip("239.10.1.1"), port: 5004, source: Some(ip("10.10.1.11")) });
    inp.streams[1].last_announced = Some(inp.now - 60_000);
    let v = build(&inp);
    let red = v.flows.iter().find(|f| f.group == ip("239.10.1.1")).unwrap();
    assert_eq!((red.pair_by, red.leg, red.pair_state), (Some("sdp"), Some("a"), Some("missing")));
    assert!(red.issues.contains(&"pairMissing"));
    let blue = v.flows.iter().find(|f| f.group == ip("239.20.1.1")).unwrap();
    assert_eq!((blue.live, blue.state), (false, "off"));
}

#[test]
fn range_stats() {
    let mut inp = fabric();
    // A conflict: 239.10.1.2 from two sources of VLAN 110.
    inp.streams.push(stream("239.10.1.2", 5004, Some("10.10.1.13"), Some("CAM-02 bis"), inp.now));
    let v = build(&inp);
    let r1 = &v.plan.ranges[0];
    assert_eq!((r1.size, r1.used, r1.reserved, r1.conflicts), (256, 2, 1, 1));
    assert_eq!(r1.free, 256 - 3);
    // .0 is skipped, .1 and .2 used, .3 reserved.
    assert_eq!(r1.next_free, Some(ip("239.10.1.4")));
    assert_eq!(r1.groups.iter().map(|g| (g.group, g.state)).collect::<Vec<_>>(), vec![(ip("239.10.1.1"), "used"), (ip("239.10.1.2"), "conflict"), (ip("239.10.1.3"), "reserved")]);
    let r2 = &v.plan.ranges[1];
    assert_eq!((r2.used, r2.next_free), (1, Some(ip("239.20.1.2"))));
    // Outside the plan: the unannounced group is guessed, so not counted.
    assert_eq!(v.plan.outside.used, 0);
    inp.more.group_labels.insert(ip("239.69.4.10"), "Dante".into());
    let v = build(&inp);
    assert_eq!((v.plan.outside.used, v.plan.outside.groups[0].group), (1, ip("239.69.4.10")));
    let c = v.flows.iter().filter(|f| f.conflict).count();
    assert_eq!((c, v.summary.conflicts), (2, 1));
    // Two sources in two different VLANs on one group: the legs of a pair, not a conflict.
    let mut inp = fabric();
    inp.streams.push(stream("239.10.1.2", 5004, Some("10.20.1.12"), Some("CAM-02 Vidéo Bleu"), inp.now));
    assert!(build(&inp).flows.iter().all(|f| !f.conflict));
}

#[test]
fn liveness() {
    let now = 1_000_000;
    let mut s = stream("239.10.1.1", 5004, None, Some("x"), now - 31_000);
    assert!(!is_live(&s, now));
    // Announced every 20 s: gone after 60 s.
    s.sap_interval_ms = Some(20_000);
    assert!(is_live(&s, now));
    s.last_announced = None;
    s.routed = true;
    s.last_seen = now - 300_000;
    assert!(is_live(&s, now));
}

#[test]
fn alerts_once_per_condition() {
    let mut inp = fabric();
    let mut w = Watch::default();
    let (a, _) = evaluate(&mut w, &build(&inp), 1, inp.now);
    assert!(a.is_empty());
    // Pair missing raised after it held 15 s, once.
    inp.now += 16_000;
    for s in inp.streams.iter_mut() {
        s.last_announced = Some(inp.now);
    }
    let (a, _) = evaluate(&mut w, &build(&inp), 1, inp.now);
    assert_eq!(a.iter().map(|x| x.0).collect::<Vec<_>>(), vec!["pairMissing"]);
    assert!(a[0].2.starts_with("239.10.1.2:5004 CAM-02 Vidéo Rouge"));
    inp.now += 5_000;
    for s in inp.streams.iter_mut() {
        s.last_announced = Some(inp.now);
    }
    assert!(evaluate(&mut w, &build(&inp), 1, inp.now).0.is_empty());
    // Receivers leave SW-B: noReceivers after 15 s; the core still lists the group (peer port only).
    inp.switches[2].snoop.clear();
    let mut raised = Vec::new();
    for _ in 0..5 {
        inp.now += 5_000;
        for s in inp.streams.iter_mut() {
            s.last_announced = Some(inp.now);
        }
        raised.extend(evaluate(&mut w, &build(&inp), 1, inp.now).0);
    }
    assert_eq!(raised.iter().filter(|x| x.0 == "noReceivers").count(), 1);
    // The Blue leg stops being announced: gone at once (a transition), and the Red leg's pair is missing later.
    inp.now += 31_000;
    inp.streams[0].last_announced = Some(inp.now);
    inp.streams[2].last_announced = Some(inp.now);
    let (a, _) = evaluate(&mut w, &build(&inp), 1, inp.now);
    assert!(a.iter().any(|x| x.0 == "streamGone" && x.2.contains("239.20.1.1:5004")));
    // A new engine (generation): nothing is gone.
    inp.streams.clear();
    assert!(evaluate(&mut w, &build(&inp), 2, inp.now).0.is_empty());
}

#[test]
fn ptp_roles_changes_and_domains() {
    let mut inp = fabric();
    let gm1 = [0xEC, 0x46, 0x70, 0xFF, 0xFE, 0, 0x1A, 0x2B];
    let gm2 = [0xEC, 0x46, 0x70, 0xFF, 0xFE, 0, 0x1A, 0x3C];
    let ann = |p2: u8, gm: [u8; 8]| ptp::Announce { priority1: 128, clock_class: 6, clock_accuracy: 0x21, variance: 0x4E5D, priority2: p2, gm, steps_removed: 0, time_source: 0x20, utc_offset: 37 };
    let mut t = ptp::Tracker::default();
    let now = inp.now;
    t.record(ip("10.10.1.47"), &ptp::build(ptp::MSG_ANNOUNCE, 127, gm1, Some(&ann(128, gm1))), now);
    t.record(ip("10.10.1.48"), &ptp::build(ptp::MSG_ANNOUNCE, 127, gm2, Some(&ann(129, gm2))), now);
    t.record(ip("10.10.1.11"), &ptp::build(ptp::MSG_DELAY_REQ, 127, [1; 8], None), now);
    t.record(ip("10.10.1.70"), &ptp::build(ptp::MSG_DELAY_REQ, 0, gm1, None), now);
    inp.ptp = t.snapshot();
    let v = build(&inp);
    assert_eq!(v.ptp.gm.as_ref().unwrap().clock.ip, ip("10.10.1.47"));
    assert_eq!(v.ptp.domain, Some(127));
    assert_eq!(v.ptp.clocks.iter().map(|c| c.role).collect::<Vec<_>>(), vec!["gm", "backup"]);
    // The device is located by its address (cam-01-red: MAC learnt on SW-A Eth1/1).
    let cam = v.ptp.devices.iter().find(|d| d.device.ip == ip("10.10.1.11")).unwrap();
    assert_eq!((cam.name.as_deref(), cam.at.as_ref().map(|a| a.n)), (Some("cam-01-red"), Some(Some(1))));
    let wrong = v.ptp.devices.iter().find(|d| d.device.ip == ip("10.10.1.70")).unwrap();
    assert!(wrong.wrong_domain);
    let mut w = Watch::default();
    let (a, ch) = evaluate(&mut w, &v, 1, now);
    assert!(ch.is_empty() && a.is_empty());
    // GM1 goes silent: GM2 takes over.
    t.record(ip("10.10.1.48"), &ptp::build(ptp::MSG_ANNOUNCE, 127, gm2, Some(&ann(129, gm2))), now + 20_000);
    t.record(ip("10.10.1.70"), &ptp::build(ptp::MSG_DELAY_REQ, 0, gm1, None), now + 20_000);
    inp.ptp = t.snapshot();
    inp.now = now + 20_000;
    let (a, ch) = evaluate(&mut w, &build(&inp), 1, inp.now);
    assert_eq!(ch.len(), 1);
    assert_eq!((ch[0].from.as_deref(), ch[0].to.as_deref()), (Some("EC:46:70:FF:FE:00:1A:2B"), Some("EC:46:70:FF:FE:00:1A:3C")));
    let kinds: Vec<&str> = a.iter().map(|x| x.0).collect();
    assert!(kinds.contains(&"ptpGmChange"));
    // The wrong domain held 20 s: raised.
    assert!(kinds.contains(&"ptpDomain"), "{kinds:?}");
}

#[test]
fn mac_groups_and_control() {
    let g = mac_groups(group_mac(ip("239.10.1.1")));
    assert_eq!(g.len(), 32);
    assert!(g.contains(&ip("239.10.1.1")) && g.contains(&ip("224.138.1.1")));
    assert!(control_group(ip("224.0.1.129")) && control_group(ip("239.255.255.255")) && !control_group(ip("239.10.1.1")));
}

#[test]
fn check_guard_rules() {
    let now = 1_000;
    let video = stream("239.10.1.1", 5004, Some("10.10.1.11"), Some("CAM"), now);
    // 2.18 Gb/s by its SDP: heavier than 100 Mb/s.
    assert_eq!(check_allowed(std::slice::from_ref(&video), 100).unwrap_err().1, "the stream is heavier than the limit");
    assert!(check_allowed(std::slice::from_ref(&video), 10_000).is_ok());
    // A video whose rate is not known.
    let mut unknown = video.clone();
    unknown.est_bps = None;
    unknown.format = None;
    assert_eq!(check_allowed(&[unknown.clone()], 10_000).unwrap_err().1, "the rate of this video stream is not known");
    // Audio without estimate, measured light: allowed; measured heavy: refused.
    let mut audio = unknown;
    audio.kind = Kind::Audio;
    audio.encoding = Some("L24".into());
    assert!(check_allowed(std::slice::from_ref(&audio), 100).is_ok());
    audio.bps = 150e6;
    assert!(check_allowed(&[audio], 100).is_err());
    assert_eq!(check_allowed(&[], 100).unwrap_err().0, StatusCode::NOT_FOUND);
}
