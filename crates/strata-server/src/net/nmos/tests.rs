use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const BASE: &str = "http://10.1.1.20:8010/x-nmos/query/v1.3";

fn ok(body: String) -> Page {
    Page { status: 200, body, ..Page::default() }
}

/// A registry of `n` nodes that pages like IS-04: `paging.since` gives the oldest
/// after it, `X-Paging-Limit` caps the limit, `Link: rel="next"` goes on.
fn paged(n: usize, cap: usize, calls: &AtomicUsize) -> impl Fn(&str, u64) -> Result<Page, String> + Sync + '_ {
    move |url: &str, _| {
        calls.fetch_add(1, Ordering::Relaxed);
        let q = url.split_once('?').map(|x| x.1).unwrap_or("");
        let param = |k: &str| q.split('&').find_map(|p| p.strip_prefix(k).and_then(|v| v.strip_prefix('=')));
        let since: usize = param("paging.since").and_then(|s| s.split(':').next()?.parse().ok()).unwrap_or(0);
        let limit = param("paging.limit").and_then(|s| s.parse().ok()).unwrap_or(cap).min(cap);
        // Item i was updated at time i + 1.
        let items: Vec<Value> = (since..n).take(limit).map(|i| json!({"id": format!("node-{i}"), "label": format!("n{i}")})).collect();
        let until = since + items.len();
        Ok(Page {
            status: 200,
            link: Some(format!("<{BASE}/nodes?paging.since={until}:0&paging.limit={limit}>; rel=\"next\", <{BASE}/nodes?paging.until={since}:0&paging.limit={limit}>; rel=\"prev\"")),
            until: Some(format!("{until}:0")),
            limit: Some(limit),
            body: serde_json::to_string(&items).unwrap(),
        })
    }
}

#[test]
fn reads_every_page() {
    let calls = AtomicUsize::new(0);
    let get = paged(1234, 100, &calls);
    let all = read_all(&get, BASE, "nodes").unwrap();
    assert_eq!(all.len(), 1234);
    let ids: HashSet<&str> = all.iter().filter_map(|v| v["id"].as_str()).collect();
    assert_eq!(ids.len(), 1234);
    // 12 full pages, then a short one ends it without asking for an empty page.
    assert_eq!(calls.load(Ordering::Relaxed), 13);
    // A multiple of the page: one more (empty) answer ends it.
    let calls = AtomicUsize::new(0);
    assert_eq!(read_all(&paged(300, 100, &calls), BASE, "nodes").unwrap().len(), 300);
    assert_eq!(calls.load(Ordering::Relaxed), 4);
}

#[test]
fn reads_registries_that_do_not_page() {
    // Everything at once, no paging header: one request.
    let calls = AtomicUsize::new(0);
    let get = |_: &str, _: u64| {
        calls.fetch_add(1, Ordering::Relaxed);
        Ok(ok(serde_json::to_string(&(0..700).map(|i| json!({"id": i.to_string()})).collect::<Vec<_>>()).unwrap()))
    };
    assert_eq!(read_all(&get, BASE, "senders").unwrap().len(), 700);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    // Refuses the paging parameters: asked again without them.
    let urls = Mutex::new(Vec::new());
    let get = |u: &str, _: u64| {
        urls.lock().unwrap().push(u.to_string());
        Ok(if u.contains("paging") { Page { status: 400, ..Page::default() } } else { ok(r#"[{"id":"a"},{"id":"b"}]"#.into()) })
    };
    assert_eq!(read_all(&get, BASE, "devices").unwrap().len(), 2);
    assert_eq!(urls.lock().unwrap().last().map(String::as_str), Some("http://10.1.1.20:8010/x-nmos/query/v1.3/devices"));
    // Only X-Paging-Until (no Link): the next page is built from it.
    let get = |u: &str, _: u64| {
        let second = u.contains("paging.since=5:0");
        let body = if second { r#"[{"id":"c"}]"# } else { r#"[{"id":"a"},{"id":"b"}]"# };
        Ok(Page { status: 200, until: Some("5:0".into()), limit: Some(2), body: body.into(), ..Page::default() })
    };
    assert_eq!(read_all(&get, BASE, "flows").unwrap().len(), 3);
    // The same page again (a registry that ignores `since`): stops.
    let calls = AtomicUsize::new(0);
    let get = |_: &str, _: u64| {
        calls.fetch_add(1, Ordering::Relaxed);
        Ok(Page { status: 200, link: Some(format!("<{BASE}/flows?paging.since=9:0>; rel=\"next\"")), body: r#"[{"id":"a"}]"#.into(), ..Page::default() })
    };
    assert_eq!(read_all(&get, BASE, "flows").unwrap().len(), 1);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[test]
fn registry_errors_are_readable() {
    let get = |_: &str, _: u64| Ok(Page { status: 503, ..Page::default() });
    assert_eq!(read_all(&get, BASE, "nodes").unwrap_err(), "nmos: the registry answered 503");
    let get = |_: &str, _: u64| Ok(ok(r#"{"code": 404, "error": "not found"}"#.into()));
    assert_eq!(read_all(&get, BASE, "nodes").unwrap_err(), "nmos: not a query api");
    let get = |_: &str, _: u64| Ok(ok("<html>".into()));
    assert_eq!(read_all(&get, BASE, "nodes").unwrap_err(), "nmos: not a query api");
    let get = |_: &str, _: u64| Err("nmos: timed out".to_string());
    assert_eq!(read(BASE, None, &get, 0).unwrap_err(), "nmos: timed out");
}

#[test]
fn link_headers() {
    let h = r#"<http://r/x-nmos/query/v1.3/nodes?paging.since=12:5&paging.limit=10>; rel="next", <http://r/x-nmos/query/v1.3/nodes?paging.until=1:0>; rel="prev""#;
    assert_eq!(link_rel(h, "next").as_deref(), Some("http://r/x-nmos/query/v1.3/nodes?paging.since=12:5&paging.limit=10"));
    assert_eq!(link_rel(h, "prev").as_deref(), Some("http://r/x-nmos/query/v1.3/nodes?paging.until=1:0"));
    assert_eq!(link_rel("<a>; rel=first", "next"), None);
    assert_eq!(link_rel("garbage <unclosed", "next"), None);
    let from = "http://r:8010/x-nmos/query/v1.3/nodes?paging.since=0:0";
    assert_eq!(resolve(from, "/x-nmos/query/v1.3/nodes?paging.since=3:0").as_deref(), Some("http://r:8010/x-nmos/query/v1.3/nodes?paging.since=3:0"));
    assert_eq!(resolve(from, "?paging.since=3:0").as_deref(), Some("http://r:8010/x-nmos/query/v1.3/nodes?paging.since=3:0"));
    assert_eq!(resolve(from, "nodes?paging.since=3:0").as_deref(), Some("http://r:8010/x-nmos/query/v1.3/nodes?paging.since=3:0"));
}

#[test]
fn addresses_and_settings() {
    assert_eq!(query_base("http://10.1.1.20:8010", "v1.3").unwrap(), BASE);
    assert_eq!(query_base("http://10.1.1.20:8010/x-nmos/query/v1.2/", "v1.3").unwrap(), BASE);
    assert_eq!(query_base(" https://reg.local/x-nmos/query ", "v1.2").unwrap(), "https://reg.local/x-nmos/query/v1.2");
    for bad in ["10.1.1.20:8010", "ftp://x", "http://", "http://a b", "http://x/?q"] {
        assert!(query_base(bad, "v1.3").is_err(), "{bad}");
    }
    let mut s = Settings { enabled: true, url: " http://10.1.1.20:8010/ ".into(), source: "registry".into(), ..Settings::default() };
    s.check().unwrap();
    assert_eq!(s.url, "http://10.1.1.20:8010");
    assert_eq!(s.base().as_deref(), Some(BASE));
    assert_eq!(s.interval(), Duration::from_secs(5));
    s.refresh = "60s".into();
    assert_eq!(s.interval(), Duration::from_secs(60));
    s.version = "v1.1".into();
    assert_eq!(s.check().unwrap_err(), "the nmos version is v1.2 or v1.3");
    let mut s = Settings { enabled: true, source: "registry".into(), ..Settings::default() };
    assert_eq!(s.check().unwrap_err(), "the registry address cannot be empty");
    s.enabled = false;
    assert!(s.check().is_ok() && s.base().is_none());
    assert_eq!(host_port("http://10.1.1.20:8010/x"), Some(("10.1.1.20".into(), 8010)));
    assert_eq!(host_port("https://Reg.Local/x"), Some(("reg.local".into(), 443)));
    assert_eq!(host_port("http://[fd00::1]/x"), Some(("fd00::1".into(), 80)));
    assert_eq!(host_port("http://[fd00::1]:8010"), Some(("fd00::1".into(), 8010)));
    assert_eq!(host_port("http://reg:port/"), None);
    // Saved settings without the newer fields.
    let mut s: Settings = serde_json::from_str(r#"{"enabled": true, "url": "http://r:80"}"#).unwrap();
    assert_eq!((s.version.as_str(), s.refresh.as_str()), ("v1.3", "live"));
    // … a registry set: it stays the source; none: the nodes, the scan probe on.
    s.migrate();
    assert_eq!(s.source, "registry");
    let mut s: Settings = serde_json::from_str(r#"{"enabled": false, "url": ""}"#).unwrap();
    s.migrate();
    assert_eq!(s.source, "nodes");
    assert!(s.probe_scan && s.node_urls.is_empty() && s.ranges.is_empty());
    // Nodes mode: no registry address needed, no Query API read; node addresses and ranges checked.
    let mut s = Settings { enabled: true, source: "nodes".into(), node_urls: vec!["10.1.1.7:8080".into(), " http://10.1.1.7:8080/ ".into(), "".into(), "https://GW.lan/x-nmos/node/v1.3".into()], ranges: vec!["10.99.0.0/16".into(), " 10.10.1.10-10.10.1.50 ".into()], ..Settings::default() };
    s.check().unwrap();
    assert!(s.base().is_none());
    assert_eq!(s.node_urls, vec!["http://10.1.1.7:8080", "https://gw.lan"]);
    assert_eq!(s.ranges, vec!["10.99.0.0/16", "10.10.1.10-10.10.1.50"]);
    s.node_urls = vec!["ftp://x".into()];
    assert_eq!(s.check().unwrap_err(), "the node address is written http://host:port");
    s.node_urls.clear();
    s.ranges = vec!["10.1.1.0/33".into()];
    assert!(s.check().unwrap_err().starts_with("nmos range 10.1.1.0/33"));
    s.ranges.clear();
    s.source = "mdns".into();
    assert_eq!(s.check().unwrap_err(), "the nmos source is registry or nodes");
}

#[test]
fn malformed_resources_are_skipped_not_fatal() {
    let n = Node::parse(&json!({
        "id": "n1", "label": 7, "hostname": "cobalt-bidi4-01.studio.lan", "href": "http://10.1.1.7:80/",
        "api": {"endpoints": [{"host": "10.1.1.7", "port": 80}, {"host": "cobalt.local"}, {"host": 12}], "versions": "x"},
        "interfaces": [{"name": "eth0", "chassis_id": "74-26-ac-00-01-02", "port_id": "74-26-ac-00-01-03",
                        "attached_network_device": {"chassis_id": "70-69-5a-ee-6f-79", "port_id": "Ethernet1/12"}},
                       {"name": 5}, "garbage"],
        "tags": {"urn:x-nmos:tag:asset:manufacturer/v1.0": ["Cobalt"], "urn:x-nmos:tag:asset:product/v1.0": ["BIDI4"], "bad": 3}
    }))
    .unwrap();
    assert_eq!(n.label, "cobalt-bidi4-01.studio.lan");
    assert_eq!(n.ips, vec![Ipv4Addr::new(10, 1, 1, 7)]);
    assert_eq!(n.macs, vec!["74:26:AC:00:01:03".to_string(), "74:26:AC:00:01:02".to_string()]);
    assert_eq!(n.interfaces[0].attached, Some(("70-69-5a-ee-6f-79".into(), "Ethernet1/12".into())));
    assert_eq!(n.maker(&[]).as_deref(), Some("Cobalt · BIDI4"));
    assert!(Node::parse(&json!({"label": "no id"})).is_none());
    assert!(Node::parse(&json!("text")).is_none());
    let s = Sender::parse(&json!({"id": "s", "subscription": {"receiver_id": null, "active": "yes"}, "manifest_href": 3})).unwrap();
    assert_eq!((s.active, s.manifest_href), (None, None));
    let r = Receiver::parse(&json!({"id": "r", "subscription": {"sender_id": "s", "active": false}})).unwrap();
    assert_eq!((r.sender_id.as_deref(), r.subscribed()), (Some("s"), None));
    // v1.0 / v1.1: no `active`, the sender id says it.
    let r = Receiver::parse(&json!({"id": "r", "subscription": {"sender_id": "s"}})).unwrap();
    assert_eq!(r.subscribed(), Some("s"));
}

const SDP_V: &str = "v=0\r\no=- 1 1 IN IP4 10.10.1.7\r\ns=BIDI4-01 SDI 1\r\nt=0 0\r\nm=video 5004 RTP/AVP 96\r\nc=IN IP4 239.120.1.1/64\r\n\
a=source-filter: incl IN IP4 239.120.1.1 10.10.1.7\r\na=rtpmap:96 raw/90000\r\na=fmtp:96 sampling=YCbCr-4:2:2; width=1920; height=1080; exactframerate=50; depth=10\r\n";

fn sender(id: &str, version: &str, href: Option<&str>) -> Sender {
    Sender { id: id.into(), version: version.into(), label: id.into(), manifest_href: href.map(str::to_string), ..Sender::default() }
}

#[test]
fn sdp_read_again_only_when_the_sender_changes() {
    let calls = Mutex::new(Vec::<String>::new());
    let fail = std::sync::atomic::AtomicBool::new(false);
    let get = |u: &str, limit: u64| {
        assert_eq!(limit, SDP_LIMIT);
        calls.lock().unwrap().push(u.to_string());
        if fail.load(Ordering::Relaxed) && u.ends_with("/b") {
            return Err("nmos: timed out".to_string());
        }
        Ok(ok(SDP_V.into()))
    };
    let senders = vec![sender("a", "1:0", Some("http://n/a")), sender("b", "1:0", Some("http://n/b")), sender("c", "1:0", None), sender("d", "1:0", Some("file:///etc/passwd"))];
    let first = read_sdps(&senders, None, &get, 1000);
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert_eq!(first.len(), 2);
    assert_eq!(first["a"].medias.len(), 1);
    assert_eq!(first["a"].medias[0].group, Some(Ipv4Addr::new(239, 120, 1, 1)));
    assert!(first["a"].text.as_deref().is_some_and(|t| t.starts_with("v=0")));
    // Same versions: nothing read.
    calls.lock().unwrap().clear();
    let again = read_sdps(&senders, Some(&first), &get, 6000);
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(again, first);
    // One sender changed (an activation): only its SDP is read.
    let mut changed = senders.clone();
    changed[1].version = "2:0".into();
    fail.store(true, Ordering::Relaxed);
    let third = read_sdps(&changed, Some(&again), &get, 11_000);
    assert_eq!(*calls.lock().unwrap(), vec!["http://n/b".to_string()]);
    assert_eq!(third["b"].error.as_deref(), Some("nmos: timed out"));
    assert!(third["b"].medias.is_empty());
    // A failed read is not asked again at each poll, only after a while.
    calls.lock().unwrap().clear();
    let fourth = read_sdps(&changed, Some(&third), &get, 11_000 + SDP_RETRY_MS - 1);
    assert!(calls.lock().unwrap().is_empty());
    fail.store(false, Ordering::Relaxed);
    let fifth = read_sdps(&changed, Some(&fourth), &get, 11_000 + SDP_RETRY_MS);
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(fifth["b"].error.is_none());
    // An answer that is not an SDP of multicast medias.
    let get = |_: &str, _: u64| Ok(ok("hello".into()));
    let e = read_sdps(&senders[..1], None, &get, 0);
    assert_eq!(e["a"].error.as_deref(), Some("sdp: no multicast media"));
    let get = |_: &str, _: u64| Ok(Page { status: 404, ..Page::default() });
    assert_eq!(read_sdps(&senders[..1], None, &get, 0)["a"].error.as_deref(), Some("sdp: the node answered 404"));
}

#[test]
fn sdp_reads_are_parallel_but_bounded() {
    let (now, max) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let get = |_: &str, _: u64| {
        let n = now.fetch_add(1, Ordering::SeqCst) + 1;
        max.fetch_max(n, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(20));
        now.fetch_sub(1, Ordering::SeqCst);
        Ok(ok(SDP_V.into()))
    };
    let senders: Vec<Sender> = (0..40).map(|i| sender(&format!("s{i}"), "1:0", Some(&format!("http://n/{i}")))).collect();
    let got = read_sdps(&senders, None, &get, 0);
    assert_eq!(got.len(), 40);
    let m = max.load(Ordering::SeqCst);
    assert!(m > 1 && m <= SDP_WORKERS, "{m}");
}

/// A whole registry read, then the same one with a node gone.
#[test]
fn reads_a_registry_and_keeps_gone_nodes_a_while() {
    let nodes = Mutex::new(json!([{"id": "n1", "label": "bidi4-01", "version": "1:0"}, {"id": "n2", "label": "mv-01", "version": "1:0"}]));
    let get = |u: &str, _: u64| {
        let path = u.split('?').next().unwrap_or("");
        let body = match path.rsplit('/').next().unwrap_or("") {
            "nodes" => nodes.lock().unwrap().clone(),
            "devices" => json!([{"id": "d1", "node_id": "n1"}, {"id": "d2", "node_id": "n2"}]),
            "senders" => json!([{"id": "s1", "device_id": "d1", "version": "1:0", "flow_id": "f1", "manifest_href": "http://n1/sdp", "subscription": {"active": true}}]),
            "receivers" => json!([{"id": "r1", "device_id": "d2", "subscription": {"sender_id": "s1", "active": true}}, {"id": "r2", "device_id": "d2", "subscription": {"sender_id": null, "active": false}}]),
            "flows" => json!([{"id": "f1", "format": "urn:x-nmos:format:video", "source_id": "src1"}]),
            "sources" => json!([{"id": "src1", "format": "urn:x-nmos:format:video"}]),
            "sdp" => return Ok(ok(SDP_V.into())),
            other => panic!("{other}"),
        };
        Ok(ok(body.to_string()))
    };
    let r = read(BASE, None, &get, 10_000).unwrap();
    assert_eq!((r.nodes.len(), r.devices.len(), r.senders.len(), r.receivers.len(), r.flows.len(), r.sources.len()), (2, 2, 1, 2, 1, 1));
    assert_eq!((r.subscribed(), r.is_video(&r.senders[0])), (1, true));
    assert_eq!(r.node_of_device("d1").map(|n| n.label.as_str()), Some("bidi4-01"));
    assert_eq!(r.sdp_text(Ipv4Addr::new(239, 120, 1, 1), 5004).as_deref(), Some(SDP_V));
    *nodes.lock().unwrap() = json!([{"id": "n1", "label": "bidi4-01", "version": "1:0"}]);
    let r2 = read(BASE, Some(&r), &get, 20_000).unwrap();
    assert_eq!(r2.gone.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), vec!["n2"]);
    assert_eq!((r2.seen["n1"], r2.seen["n2"]), (20_000, 10_000));
    assert_ne!(r.fingerprint(), r2.fingerprint());
    let r3 = read(BASE, Some(&r2), &get, 10_000 + NODE_KEPT_MS).unwrap();
    assert!(r3.gone.is_empty());
}

#[test]
fn query_apis_found_by_mdns() {
    let svc = |name: &str, ip: [u8; 4], port: u16, txt: &[&str]| Service {
        kind: QUERY_KIND.into(),
        name: name.into(),
        host: format!("{name}.local"),
        ip: Some(Ipv4Addr::from(ip)),
        port,
        txt: txt.iter().map(|s| s.to_string()).collect(),
    };
    let list = vec![
        svc("nmos-registry-02", [10, 1, 1, 21], 8010, &["api_proto=http", "api_ver=v1.0,v1.1,v1.2,v1.3", "pri=20", "api_auth=false"]),
        svc("nmos-registry-01", [10, 1, 1, 20], 8010, &["api_proto=http", "api_ver=v1.2", "pri=10"]),
        svc("tls-registry", [10, 1, 1, 22], 443, &["api_proto=https", "pri=bad"]),
        Service { kind: "NMOS".into(), name: "a node".into(), host: "n.local".into(), ip: None, port: 80, txt: vec![] },
    ];
    let f = found(&list);
    assert_eq!(f.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["nmos-registry-01", "nmos-registry-02", "tls-registry"]);
    assert_eq!(f[0].url, "http://10.1.1.20:8010/x-nmos/query/v1.2");
    assert_eq!(f[1].url, "http://10.1.1.21:8010/x-nmos/query/v1.3");
    assert_eq!((f[1].priority, f[1].versions.len()), (Some(20), 4));
    assert_eq!((f[2].url.as_str(), f[2].priority), ("https://10.1.1.22:443/x-nmos/query/v1.3", None));
    assert_eq!(found_for(&f, BASE).map(|x| x.name.as_str()), Some("nmos-registry-01"));
    assert_eq!(found_for(&f, "http://nmos-registry-02.local:8010/x-nmos/query/v1.3").map(|x| x.name.as_str()), Some("nmos-registry-02"));
}
