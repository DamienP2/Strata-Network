use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn ip(s: &str) -> Ipv4Addr {
    s.parse().unwrap()
}

#[test]
fn node_addresses() {
    assert_eq!(node_base("10.1.1.7:8080").unwrap(), "http://10.1.1.7:8080");
    assert_eq!(node_base(" HTTP://10.1.1.7:8080/ ").unwrap(), "http://10.1.1.7:8080");
    assert_eq!(node_base("https://BIDI4.studio.lan/x-nmos/node/v1.3/").unwrap(), "https://bidi4.studio.lan");
    assert_eq!(node_base("http://10.1.1.7").unwrap(), "http://10.1.1.7");
    for bad in ["", "ftp://x", "http://", "http://a b", "http://x:99999", "http://x/other", "http://u@x", "http://x/?q"] {
        assert!(node_base(bad).is_err(), "{bad}");
    }
    assert_eq!(base_ip("http://10.1.1.7:8080"), Some(ip("10.1.1.7")));
    assert_eq!(base_ip("http://gw.lan"), None);
}

#[test]
fn highest_supported_version() {
    assert_eq!(pick_version(r#"["v1.0/", "v1.1/", "v1.2/", "v1.3/"]"#).unwrap(), "v1.3");
    assert_eq!(pick_version(r#"["v1.1/", "v1.2/"]"#).unwrap(), "v1.2");
    assert_eq!(pick_version(r#"["v1.1"]"#).unwrap(), "v1.1");
    assert_eq!(pick_version(r#"["v1.0/"]"#).unwrap_err(), "nmos: no supported node api version");
    assert_eq!(pick_version(r#"{"code": 404}"#).unwrap_err(), "nmos: not a node api");
    assert_eq!(pick_version("<html>").unwrap_err(), "nmos: not a node api");
}

fn sender(id: &str, flow: &str) -> Sender {
    Sender { id: id.into(), version: "1:0".into(), label: "BIDI4 · SDI 1".into(), flow_id: Some(flow.into()), device_id: "d1".into(), transport: "urn:x-nmos:transport:rtp.mcast".into(), ..Sender::default() }
}

fn video_flow() -> (Vec<Essence>, Vec<Essence>) {
    let f = json!({"id": "f1", "source_id": "src1", "format": "urn:x-nmos:format:video", "media_type": "video/raw", "frame_width": 1920, "frame_height": 1080,
        "grain_rate": {"numerator": 50}, "interlace_mode": "progressive", "colorspace": "BT709",
        "components": [{"name": "Y", "width": 1920, "height": 1080, "bit_depth": 10}, {"name": "Cb", "width": 960, "height": 1080, "bit_depth": 10}, {"name": "Cr", "width": 960, "height": 1080, "bit_depth": 10}]});
    let s = json!({"id": "src1", "format": "urn:x-nmos:format:video"});
    (vec![Essence::parse(&f).unwrap()], vec![Essence::parse(&s).unwrap()])
}

#[test]
fn is05_active_two_legs() {
    // ST 2022-7: two transport params; the port as a number or a string; an "auto" source.
    let body = r#"{"sender_id": null, "master_enable": true, "activation": {"mode": null},
        "transport_params": [
          {"source_ip": "10.10.1.7", "destination_ip": "239.120.1.1", "source_port": 5004, "destination_port": 5004, "rtp_enabled": true},
          {"source_ip": "10.20.1.7", "destination_ip": "239.130.1.1", "destination_port": "5006", "rtp_enabled": true}]}"#;
    let a = parse_active(body).unwrap();
    assert_eq!(a.master_enable, Some(true));
    assert_eq!(a.legs.len(), 2);
    assert_eq!((a.legs[1].dest, a.legs[1].source, a.legs[1].port), (Some(ip("239.130.1.1")), Some(ip("10.20.1.7")), Some(5006)));
    let (flows, sources) = video_flow();
    let m = medias_of_active(&sender("s1", "f1"), &a, &flows, &sources);
    assert_eq!(m.len(), 2);
    assert_eq!((m[0].group, m[0].port, m[0].source, m[0].kind), (Some(ip("239.120.1.1")), 5004, Some(ip("10.10.1.7")), Some(Kind::Video)));
    assert_eq!(m[0].dup.as_ref().map(|l| (l.group, l.port)), Some((ip("239.130.1.1"), 5006)));
    assert_eq!(m[1].dup.as_ref().map(|l| l.group), Some(ip("239.120.1.1")));
    // The IS-04 flow gives the format: ST 2110-20 1080p50 4:2:2 10 bits → its rate.
    let f = m[0].format.as_ref().unwrap();
    assert_eq!((f.standard.as_deref(), f.width, f.height, f.sampling.as_deref(), f.depth), (Some("ST 2110-20"), Some(1920), Some(1080), Some("YCbCr-4:2:2"), Some(10)));
    assert!(f.est_bps().is_some_and(|b| b > 2e9));
    assert_eq!(m[0].encoding.as_deref(), Some("raw"));
    // One leg off, a unicast destination, "auto" left out: one media, not a pair.
    let body = r#"{"master_enable": true, "transport_params": [
          {"source_ip": "auto", "destination_ip": "239.120.1.1", "destination_port": "auto", "rtp_enabled": true},
          {"destination_ip": "239.130.1.1", "rtp_enabled": false}, {"destination_ip": "10.1.1.9"}]}"#;
    let a = parse_active(body).unwrap();
    let m = medias_of_active(&sender("s1", "f1"), &a, &flows, &sources);
    assert_eq!(m.len(), 1);
    assert_eq!((m[0].port, m[0].source, m[0].dup.is_none()), (5004, None, true));
    // Turned off: nothing.
    let a = parse_active(r#"{"master_enable": false, "transport_params": [{"destination_ip": "239.120.1.1"}]}"#).unwrap();
    assert!(medias_of_active(&sender("s1", "f1"), &a, &flows, &sources).is_empty());
    // Not an object, not JSON.
    assert!(parse_active("[]").is_err() && parse_active("x").is_err());
    // Receiver: multicast_ip, interface_ip, sender_id.
    let a = parse_active(r#"{"sender_id": "s9", "master_enable": true, "transport_params": [{"multicast_ip": "239.120.1.1", "source_ip": "10.10.1.7", "interface_ip": "10.10.1.40", "destination_port": 5004}]}"#).unwrap();
    assert_eq!((a.sender_id.as_deref(), a.legs[0].dest, a.legs[0].iface), (Some("s9"), Some(ip("239.120.1.1")), Some(ip("10.10.1.40"))));
}

#[test]
fn audio_format_from_is04() {
    let f = media_of(&json!({"media_type": "audio/L24", "sample_rate": {"numerator": 48000}, "bit_depth": 24})).unwrap();
    assert_eq!((f.standard.as_deref(), f.audio_rate, f.depth), (Some("ST 2110-30"), Some(48000), Some(24)));
    let s = media_of(&json!({"channels": [{"label": "L"}, {"label": "R"}]})).unwrap();
    assert_eq!(s.channels, Some(2));
    assert!(media_of(&json!({"format": "urn:x-nmos:format:data"})).is_none());
}

#[test]
fn connection_api_of_a_device() {
    let d = json!({"controls": [
        {"type": "urn:x-nmos:control:sr-ctrl/v1.0", "href": "http://10.1.1.7:8080/x-nmos/connection/v1.0/"},
        {"type": "urn:x-nmos:control:sr-ctrl/v1.1", "href": "http://10.1.1.7:8080/x-nmos/connection/v1.1/"},
        {"type": "urn:x-manufacturer:control:x", "href": "http://10.1.1.7/x"}]});
    assert_eq!(connection_api(&d).as_deref(), Some("http://10.1.1.7:8080/x-nmos/connection/v1.1"));
    assert_eq!(connection_api(&json!({"controls": [{"type": "urn:x-nmos:control:sr-ctrl/v1.1", "href": "ftp://x"}]})), None);
    assert_eq!(connection_api(&json!({"controls": "x"})), None);
}

/// A node at http://10.1.1.7:8080: v1.2 and v1.3, one sender with an SDP, one
/// without (IS-05), one receiver known by IS-05 only.
fn fake_node(calls: &Mutex<Vec<String>>) -> impl Fn(&str, u64) -> Result<Page, String> + Sync + '_ {
    move |url: &str, _| {
        calls.lock().unwrap().push(url.to_string());
        let path = url.strip_prefix("http://10.1.1.7:8080").unwrap_or(url);
        let body = match path {
            "/x-nmos/node/" => json!(["v1.2/", "v1.3/"]),
            "/x-nmos/node/v1.3/self" => json!({"id": "n1", "label": "bidi4-01", "api": {"endpoints": [{"host": "10.99.0.7", "port": 8080}]}}),
            "/x-nmos/node/v1.3/devices" => json!([{"id": "d1", "node_id": "", "controls": [{"type": "urn:x-nmos:control:sr-ctrl/v1.1", "href": "http://10.1.1.7:8080/x-nmos/connection/v1.1/"}]}]),
            "/x-nmos/node/v1.3/sources" => json!([]),
            "/x-nmos/node/v1.3/flows" => json!([{"id": "f1", "format": "urn:x-nmos:format:video"}]),
            "/x-nmos/node/v1.3/senders" => json!([
                {"id": "s1", "version": "1:0", "flow_id": "f1", "device_id": "d1", "transport": "urn:x-nmos:transport:rtp.mcast", "manifest_href": "http://10.1.1.7:8080/sdp/s1"},
                {"id": "s2", "version": "1:0", "flow_id": "f1", "device_id": "d1", "transport": "urn:x-nmos:transport:rtp.mcast", "subscription": {"active": true}}]),
            "/x-nmos/node/v1.3/receivers" => json!([{"id": "r1", "version": "1:0", "device_id": "d1", "format": "urn:x-nmos:format:video", "subscription": {"sender_id": null, "active": true}}]),
            "/sdp/s1" => {
                return Ok(Page { status: 200, body: "v=0\r\no=- 1 1 IN IP4 10.10.1.7\r\ns=x\r\nt=0 0\r\nm=video 5004 RTP/AVP 96\r\nc=IN IP4 239.120.1.1/64\r\na=rtpmap:96 raw/90000\r\n".into(), ..Page::default() })
            }
            "/x-nmos/connection/v1.1/single/senders/s2/active" => json!({"master_enable": true, "transport_params": [{"source_ip": "10.10.1.7", "destination_ip": "239.120.1.2", "destination_port": 5004}]}),
            "/x-nmos/connection/v1.1/single/receivers/r1/active" => json!({"master_enable": true, "transport_params": [{"multicast_ip": "239.120.9.9", "interface_ip": "10.10.1.7", "destination_port": 5004}]}),
            _ => return Ok(Page { status: 404, body: "{}".into(), ..Page::default() }),
        };
        Ok(Page { status: 200, body: body.to_string(), ..Page::default() })
    }
}

#[test]
fn reads_a_node_and_its_connection_api() {
    let calls = Mutex::new(Vec::new());
    let get = fake_node(&calls);
    let d = read_node("http://10.1.1.7:8080", None, None, &get, 1_000).unwrap();
    assert_eq!(d.api, "v1.3");
    assert_eq!(d.node.id, "n1");
    // The API endpoint (management) and the address reached.
    assert_eq!(d.node.ips, vec![ip("10.99.0.7"), ip("10.1.1.7")]);
    assert_eq!(d.devices[0].node_id, "n1");
    assert!(d.sdp["s1"].medias.len() == 1);
    // IS-05 for the sender without SDP and for the receiver, not for s1.
    assert_eq!(d.active["s2"].legs[0].dest, Some(ip("239.120.1.2")));
    assert_eq!(d.active["r1"].legs[0].dest, Some(ip("239.120.9.9")));
    assert!(!d.active.contains_key("s1"));
    let urls = calls.lock().unwrap().clone();
    assert!(urls.iter().all(|u| !u.contains("/staged")), "{urls:?}");
    // Again, nothing changed: no SDP, no IS-05, no version asked.
    calls.lock().unwrap().clear();
    let d2 = read_node("http://10.1.1.7:8080", Some("v1.3"), Some(&d), &get, 6_000).unwrap();
    assert_eq!(d2.active, d.active);
    let urls = calls.lock().unwrap().clone();
    assert_eq!(urls.len(), 6, "{urls:?}");
    // After a minute, or a new version, IS-05 is read again.
    calls.lock().unwrap().clear();
    read_node("http://10.1.1.7:8080", Some("v1.3"), Some(&d), &get, 1_000 + ACTIVE_REFRESH_MS).unwrap();
    assert_eq!(calls.lock().unwrap().iter().filter(|u| u.contains("/active")).count(), 2);
    // A node that answers something else.
    let bad = |_: &str, _: u64| Ok(Page { status: 200, body: "<html>".into(), ..Page::default() });
    assert_eq!(read_node("http://10.1.1.9", None, None, &bad, 0).unwrap_err(), "nmos: not a node api");
    let down = |_: &str, _: u64| Err("nmos: cannot reach the registry".to_string());
    assert_eq!(read_node("http://10.1.1.9", None, None, &down, 0).unwrap_err(), "nmos: cannot reach the node");
    let refused = |_: &str, _: u64| Ok(Page { status: 503, ..Page::default() });
    assert_eq!(read_node("http://10.1.1.9", Some("v1.3"), None, &refused, 0).unwrap_err(), "nmos: the node answered 503");
}

fn peer(url: &str, found: FoundBy, d: &NodeRead, last_ok: i64, misses: u32) -> PeerState {
    PeerState { info: PeerInfo { url: url.into(), found: vec![found], last_ok, api: Some("v1.3".into()), ..PeerInfo::default() }, data: Some(Arc::new(d.clone())), misses, ..PeerState::default() }
}

#[test]
fn receivers_by_multicast_group_and_senders_from_is05() {
    let calls = Mutex::new(Vec::new());
    let d = read_node("http://10.1.1.7:8080", None, None, &fake_node(&calls), 1_000).unwrap();
    let p = peer("http://10.1.1.7:8080", FoundBy::Mdns, &d, 1_000, 0);
    let reg = aggregate(&[&p], 2_000);
    assert_eq!(reg.nodes.len(), 1);
    // s2 has medias from IS-05; s1 from its SDP.
    assert_eq!(reg.medias("s2")[0].group, Some(ip("239.120.1.2")));
    assert!(reg.sdp["s2"].text.is_none());
    assert_eq!(reg.medias("s1")[0].group, Some(ip("239.120.1.1")));
    // The receiver: no sender_id, but set to 239.120.9.9 on 10.10.1.7.
    let r = &reg.receivers[0];
    assert_eq!(r.subscribed(), None);
    assert!(r.receives(ip("239.120.9.9"), 5004, None));
    assert!(r.receives(ip("239.120.9.9"), 0, Some(ip("10.10.9.9"))));
    assert!(!r.receives(ip("239.120.9.9"), 5006, None));
    assert!(!r.receives(ip("239.120.1.1"), 5004, None));
    assert_eq!(r.ifaces, vec![ip("10.10.1.7")]);
    assert_eq!(reg.direct["n1"].found, vec![FoundBy::Mdns]);
}

#[test]
fn grace_period_then_gone_then_dropped() {
    let calls = Mutex::new(Vec::new());
    let d = read_node("http://10.1.1.7:8080", None, None, &fake_node(&calls), 1_000).unwrap();
    let mut p = peer("http://10.1.1.7:8080", FoundBy::Manual, &d, 1_000, 0);
    // Two missed polls: still there, data kept, live.
    p.record(Err("nmos: cannot reach the node".into()), 6_000, 5_000);
    p.record(Err("nmos: cannot reach the node".into()), 11_000, 5_000);
    assert!(p.present());
    let reg = aggregate(&[&p], 11_000);
    assert_eq!((reg.nodes.len(), reg.gone.len()), (1, 0));
    assert!(reg.senders.iter().all(|s| !s.stale));
    assert_eq!(p.info.error.as_deref(), Some("nmos: cannot reach the node"));
    assert_eq!(p.next, 16_000);
    // The third: gone, its senders kept but stale (not live, never "gone"), no receivers.
    p.record(Err("nmos: cannot reach the node".into()), 16_000, 5_000);
    assert!(!p.present());
    assert_eq!(p.next, 16_000 + UNREACHABLE_EVERY_MS);
    let reg = aggregate(&[&p], 16_000);
    assert_eq!((reg.nodes.len(), reg.gone.len(), reg.receivers.len()), (0, 1, 0));
    assert!(reg.senders.iter().all(|s| s.stale));
    assert_eq!(reg.seen["n1"], 1_000);
    // Answering again: back at once.
    let mut q = p.clone();
    q.record(Ok(d.clone()), 21_000, 5_000);
    assert!(q.present() && q.misses == 0 && q.info.error.is_none());
    // Ten minutes after the last good read: dropped.
    let reg = aggregate(&[&p], 1_000 + NODE_KEPT_MS);
    assert!(reg.gone.is_empty() && reg.senders.is_empty());
    // A Node API that never answered is listed apart.
    let never = PeerState { info: PeerInfo { url: "http://10.1.1.9:80".into(), found: vec![FoundBy::Scan], error: Some("nmos: timed out".into()), ..PeerInfo::default() }, misses: 2, ..PeerState::default() };
    let reg = aggregate(&[&never], 0);
    assert_eq!(reg.unread.len(), 1);
}

#[test]
fn one_node_at_two_addresses_is_one_node() {
    let calls = Mutex::new(Vec::new());
    let d = read_node("http://10.1.1.7:8080", None, None, &fake_node(&calls), 1_000).unwrap();
    let a = peer("http://10.1.1.7:8080", FoundBy::Scan, &d, 1_000, 0);
    let b = peer("http://bidi4.studio.lan:8080", FoundBy::Mdns, &d, 1_000, 0);
    let c = peer("http://10.99.0.7:8080", FoundBy::Manual, &d, 1_000, 0);
    let reg = aggregate(&[&a, &b, &c], 2_000);
    assert_eq!(reg.nodes.len(), 1);
    assert_eq!(reg.senders.len(), 2);
    assert_eq!(reg.direct["n1"].url, "http://bidi4.studio.lan:8080");
    assert_eq!(reg.direct["n1"].found, vec![FoundBy::Mdns, FoundBy::Scan, FoundBy::Manual]);
}

#[test]
fn polled_set_follows_mdns_scan_and_settings() {
    let mut p = Peers::default();
    p.mdns.insert("http://10.1.1.7:8080".into(), Heard { name: "bidi4-01".into(), seen: 0 });
    p.scan.insert("http://10.1.1.8:80".into(), 0);
    assert!(p.reconcile(&["10.1.1.7:8080".into(), "http://10.2.2.2".into()], 1_000));
    assert_eq!(p.nodes.len(), 3);
    assert_eq!(p.nodes["http://10.1.1.7:8080"].info.found, vec![FoundBy::Mdns, FoundBy::Manual]);
    assert_eq!(p.nodes["http://10.1.1.7:8080"].info.name.as_deref(), Some("bidi4-01"));
    assert!(!p.reconcile(&["10.1.1.7:8080".into(), "http://10.2.2.2".into()], 2_000));
    // Not heard for 10 min: an mDNS node that answers stays, a scan one that does not goes.
    p.nodes.get_mut("http://10.1.1.7:8080").unwrap().info.last_ok = MDNS_KEPT_MS;
    p.reconcile(&[], MDNS_KEPT_MS + 1);
    assert_eq!(p.nodes.keys().cloned().collect::<Vec<_>>(), vec!["http://10.1.1.7:8080".to_string()]);
    assert_eq!(p.nodes["http://10.1.1.7:8080"].info.found, vec![FoundBy::Mdns]);
    p.reconcile(&[], 3 * MDNS_KEPT_MS);
    assert!(p.nodes.is_empty());
}

#[test]
fn ranges_and_interfaces() {
    let ifaces = vec![("eth0".to_string(), ip("10.99.0.2")), ("eth1".to_string(), ip("10.10.1.2")), ("eth2".to_string(), ip("192.168.1.20"))];
    let networks = vec!["10.99.0.0/16".to_string(), "10.10.1.0/24".to_string(), "172.16.0.1-172.16.0.50".to_string()];
    // Automatic: the scan networks a local interface is in (OOB and IB), mDNS on both NICs.
    let s = Scope::new(&[], &networks, &ifaces);
    assert_eq!(s.ifaces, vec![ip("10.10.1.2"), ip("10.99.0.2")]);
    assert!(s.allows(ip("10.99.200.1")) && s.allows(ip("10.10.1.9")));
    assert!(!s.allows(ip("172.16.0.9")) && !s.allows(ip("192.168.1.5")));
    // Chosen: only the OOB range (a /16 is not limited in size).
    let s = Scope::new(&["10.99.0.0/16".into()], &networks, &ifaces);
    assert_eq!(s.ifaces, vec![ip("10.99.0.2")]);
    assert!(s.allows(ip("10.99.255.254")) && !s.allows(ip("10.10.1.9")));
    // A range no interface is in: no mDNS there, the probe still filtered.
    let s = Scope::new(&["10.50.0.10-10.50.0.20".into()], &networks, &ifaces);
    assert!(s.ifaces.is_empty());
    assert!(s.allows(ip("10.50.0.15")) && !s.allows(ip("10.50.0.21")));
    // No scan network reachable: everywhere, every interface.
    let s = Scope::new(&[], &["172.16.0.0/24".into()], &ifaces);
    assert_eq!(s.ifaces.len(), 3);
    assert!(s.allows(ip("8.8.8.8")));
}

#[test]
fn scan_probe_throttling() {
    let web = |p: u16| vec![(p, "http".to_string())];
    let hosts: Vec<WebHost> = vec![(ip("10.99.0.7"), web(80)), (ip("10.99.0.8"), web(8080)), (ip("10.99.0.9"), web(443)), (ip("10.10.1.7"), web(80)), (ip("10.99.0.10"), vec![]), (ip("10.99.0.11"), web(80))];
    let scope = Scope::new(&["10.99.0.0/16".into()], &[], &[]);
    let known: HashSet<Ipv4Addr> = [ip("10.99.0.11")].into();
    let mut probed: HashMap<Ipv4Addr, i64> = HashMap::new();
    probed.insert(ip("10.99.0.8"), 100_000);
    probed.insert(ip("10.99.0.9"), -PROBE_EVERY_MS);
    let now = 100_000 + PROBE_EVERY_MS / 2;
    // Outside the range, without web port, known, asked less than an hour ago: not asked.
    let due = due_hosts(&hosts, &probed, &known, &scope, now, 16);
    assert_eq!(due.iter().map(|h| h.0).collect::<Vec<_>>(), vec![ip("10.99.0.7"), ip("10.99.0.9")]);
    // At most `max` a round, never asked first.
    assert_eq!(due_hosts(&hosts, &probed, &known, &scope, now, 1)[0].0, ip("10.99.0.7"));
    // An hour later, again.
    let due = due_hosts(&hosts, &probed, &known, &scope, 100_000 + PROBE_EVERY_MS, 16);
    assert_eq!(due.len(), 3);
    // The probe: GET /x-nmos/node/ only.
    let calls = AtomicUsize::new(0);
    let get = |u: &str, _: u64| {
        calls.fetch_add(1, Ordering::Relaxed);
        Ok(if u == "https://10.99.0.9:443/x-nmos/node/" { Page { status: 200, body: r#"["v1.3/"]"#.into(), ..Page::default() } } else { Page { status: 404, ..Page::default() } })
    };
    assert_eq!(probe_port(ip("10.99.0.9"), 443, "https", &get).as_deref(), Some("https://10.99.0.9:443"));
    assert_eq!(probe_port(ip("10.99.0.7"), 80, "http", &get), None);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[test]
fn mdns_answers_to_bases() {
    use strata_net::mdns::Service;
    let s = |name: &str, ip: Option<&str>, host: &str, port: u16, txt: &[&str]| Service { kind: "NMOS".into(), name: name.into(), host: host.into(), ip: ip.map(|x| x.parse().unwrap()), port, txt: txt.iter().map(|t| t.to_string()).collect() };
    let h = heard(&[
        s("bidi4-01", Some("10.99.0.7"), "bidi4-01.local", 8080, &["api_proto=http", "api_ver=v1.2,v1.3"]),
        s("mv-01", None, "MV-01.local.", 443, &["api_proto=https"]),
        s("bidi4-01 again", Some("10.99.0.7"), "bidi4-01.local", 8080, &[]),
        s("no port", Some("10.99.0.8"), "x.local", 0, &[]),
    ]);
    assert_eq!(h, vec![("http://10.99.0.7:8080".to_string(), "bidi4-01".to_string()), ("https://mv-01.local:443".to_string(), "mv-01".to_string())]);
}

#[test]
fn probe_ports_web_then_learned_then_settings() {
    let web = vec![(8080u16, "http".to_string()), (443u16, "https".to_string())];
    let got = candidate_ports(&web, &[5005, 8080], &[80, 443, 3000, 5005]);
    let ports: Vec<(u16, &str)> = got.iter().map(|(p, s)| (*p, s.as_str())).collect();
    assert_eq!(ports, vec![(8080, "http"), (443, "https"), (5005, "http"), (80, "http"), (3000, "http")]);
    let none = candidate_ports(&[], &[], &[8443]);
    assert_eq!(none, vec![(8443, "https".to_string())]);
}
