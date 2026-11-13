//! Request guards: cross-site refusal (CSRF), "this machine" check, common answer headers and
//! sign-in throttling.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::state::Shared;

/// The value of a header as text ("" when absent or not text).
pub fn hval<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("").trim()
}

/// The first value of a comma-separated header (proxies append theirs).
fn first(v: &str) -> &str {
    v.split(',').next().unwrap_or("").trim()
}

/// Whether the request came over HTTPS (directly, or through a proxy that says so).
pub fn is_https(headers: &HeaderMap) -> bool {
    first(hval(headers, "x-forwarded-proto")).eq_ignore_ascii_case("https")
}

/// The host the browser addressed: `X-Forwarded-Host`, else `Host`.
pub fn request_host(headers: &HeaderMap) -> String {
    let fwd = first(hval(headers, "x-forwarded-host"));
    if !fwd.is_empty() {
        return fwd.to_string();
    }
    hval(headers, "host").to_string()
}

fn is_ws_upgrade(headers: &HeaderMap) -> bool {
    hval(headers, "upgrade").eq_ignore_ascii_case("websocket")
}

/// `scheme://host[:port]` → (`scheme`, `host[:port]` without the default port), lowercase.
fn split_origin(origin: &str) -> Option<(String, String)> {
    let origin = origin.trim().to_ascii_lowercase();
    let (scheme, rest) = origin.split_once("://")?;
    let auth = rest.split('/').next().unwrap_or("");
    if auth.is_empty() {
        return None;
    }
    Some((scheme.to_string(), strip_default_port(scheme, auth)))
}

fn strip_default_port(scheme: &str, authority: &str) -> String {
    let a = authority.to_ascii_lowercase();
    match scheme {
        "http" => a.strip_suffix(":80").map(str::to_string).unwrap_or(a),
        "https" => a.strip_suffix(":443").map(str::to_string).unwrap_or(a),
        _ => a,
    }
}

/// True when the request comes from another web site (Sec-Fetch-Site, or an Origin that is not
/// the host the request was sent to), unless that origin is trusted.
pub fn from_other_site(headers: &HeaderMap, trusted: &[String]) -> bool {
    let origin = hval(headers, "origin");
    if !origin.is_empty() && trusted.iter().any(|t| t.eq_ignore_ascii_case(origin.trim_end_matches('/'))) {
        return false;
    }
    if hval(headers, "sec-fetch-site").eq_ignore_ascii_case("cross-site") {
        return true;
    }
    if origin.is_empty() {
        return false;
    }
    let Some((scheme, auth)) = split_origin(origin) else { return true }; // "null" and garbage
    let hosts = [first(hval(headers, "x-forwarded-host")), hval(headers, "host")];
    !hosts.iter().any(|h| !h.is_empty() && strip_default_port(&scheme, h) == auth)
}

/// Whether a request must pass the cross-site check: it changes something (or opens a
/// WebSocket) and is carried by the cookie alone (no `Authorization`).
pub fn needs_csrf_check(method: &Method, headers: &HeaderMap) -> bool {
    let changes = !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) || is_ws_upgrade(headers);
    changes && headers.get(header::AUTHORIZATION).is_none()
}

/// Middleware: refuses changes sent by another web site with the session cookie.
pub async fn csrf(State(s): State<Shared>, req: Request, next: Next) -> Response {
    if needs_csrf_check(req.method(), req.headers()) {
        let cfg = s.config.load();
        if from_other_site(req.headers(), &cfg.trusted_origins) {
            return (StatusCode::FORBIDDEN, "request sent by another site refused").into_response();
        }
    }
    next.run(req).await
}

/// Middleware: headers on every answer.
pub async fn every_answer(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    res
}

fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
    }
}

/// The host name of `host[:port]` (brackets of IPv6 removed), lowercase.
fn host_name(authority: &str) -> String {
    let a = authority.trim().to_ascii_lowercase();
    if let Some(rest) = a.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_string();
    }
    match a.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h.to_string(),
        _ => a,
    }
}

fn local_name(host: &str) -> bool {
    matches!(host_name(host).as_str(), "localhost" | "127.0.0.1" | "::1")
}

/// The request comes from the server itself: loopback peer, not relayed by a proxy, not sent by
/// another site, and addressed to a local name.
pub fn this_machine(peer: SocketAddr, headers: &HeaderMap) -> bool {
    if !is_loopback(peer.ip()) {
        return false;
    }
    if !hval(headers, "x-forwarded-for").is_empty() || !hval(headers, "x-forwarded-host").is_empty() || !hval(headers, "forwarded").is_empty() {
        return false;
    }
    if hval(headers, "sec-fetch-site").eq_ignore_ascii_case("cross-site") {
        return false;
    }
    let host = hval(headers, "host");
    if !host.is_empty() && !local_name(host) {
        return false;
    }
    let origin = hval(headers, "origin");
    if !origin.is_empty() {
        match split_origin(origin) {
            Some((_, auth)) if local_name(&auth) => {}
            _ => return false,
        }
    }
    true
}

/// The client's address: the peer, or the first `X-Forwarded-For` when the peer is a proxy on
/// this machine.
pub fn client_ip(peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    if is_loopback(peer.ip()) {
        if let Ok(ip) = first(hval(headers, "x-forwarded-for")).parse::<IpAddr>() {
            return ip;
        }
    }
    peer.ip()
}

/// The peer address of a request (set by `into_make_service_with_connect_info`).
pub fn peer_of(req: &Request) -> SocketAddr {
    req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0).unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 0)))
}

/// Sign-in throttling: per (address + name) 5 free tries, per address 20; then a wait of
/// `2^(tries over)` seconds (5 min at most); forgotten after 15 min without a try.
pub struct Logins {
    by_name: Mutex<HashMap<(IpAddr, String), Tries>>,
    by_ip: Mutex<HashMap<IpAddr, Tries>>,
}

#[derive(Clone, Copy)]
struct Tries {
    count: u32,
    last: Instant,
}

const FREE_PER_NAME: u32 = 5;
const FREE_PER_IP: u32 = 20;
const MAX_WAIT: u64 = 300;
const FORGET: Duration = Duration::from_secs(15 * 60);

fn wait_left(t: Option<&Tries>, free: u32, now: Instant) -> u64 {
    let Some(t) = t else { return 0 };
    if t.count < free || now.duration_since(t.last) >= FORGET {
        return 0;
    }
    let over = t.count - free + 1;
    let wait = if over >= 9 { MAX_WAIT } else { (1u64 << over).min(MAX_WAIT) };
    let elapsed = now.duration_since(t.last).as_secs();
    wait.saturating_sub(elapsed)
}

impl Default for Logins {
    fn default() -> Self {
        Self::new()
    }
}

impl Logins {
    pub fn new() -> Logins {
        Logins { by_name: Mutex::new(HashMap::new()), by_ip: Mutex::new(HashMap::new()) }
    }

    /// `Err(seconds)` when this try must wait.
    pub fn check(&self, ip: IpAddr, name: &str, now: Instant) -> Result<(), u64> {
        let key = (ip, name.trim().to_lowercase());
        let mut by_name = self.by_name.lock().unwrap();
        let mut by_ip = self.by_ip.lock().unwrap();
        by_name.retain(|_, t| now.duration_since(t.last) < FORGET);
        by_ip.retain(|_, t| now.duration_since(t.last) < FORGET);
        let wait = wait_left(by_name.get(&key), FREE_PER_NAME, now).max(wait_left(by_ip.get(&ip), FREE_PER_IP, now));
        if wait > 0 {
            Err(wait)
        } else {
            Ok(())
        }
    }

    pub fn failed(&self, ip: IpAddr, name: &str, now: Instant) {
        let key = (ip, name.trim().to_lowercase());
        let mut by_name = self.by_name.lock().unwrap();
        let t = by_name.entry(key).or_insert(Tries { count: 0, last: now });
        t.count += 1;
        t.last = now;
        let mut by_ip = self.by_ip.lock().unwrap();
        let t = by_ip.entry(ip).or_insert(Tries { count: 0, last: now });
        t.count += 1;
        t.last = now;
    }

    pub fn succeeded(&self, ip: IpAddr, name: &str) {
        self.by_name.lock().unwrap().remove(&(ip, name.trim().to_lowercase()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn csrf_origin_check() {
        let none: Vec<String> = vec![];
        assert!(!from_other_site(&headers(&[("host", "localhost:8080")]), &none), "no origin: not a browser form");
        assert!(!from_other_site(&headers(&[("host", "localhost:8080"), ("origin", "http://localhost:8080")]), &none));
        assert!(from_other_site(&headers(&[("host", "localhost:8080"), ("origin", "http://localhost:9090")]), &none), "port counts");
        assert!(from_other_site(&headers(&[("host", "srv:8080"), ("origin", "http://evil.example")]), &none));
        assert!(from_other_site(&headers(&[("host", "srv:8080"), ("origin", "null")]), &none));
        assert!(from_other_site(&headers(&[("host", "srv:8080"), ("sec-fetch-site", "cross-site")]), &none));
        assert!(!from_other_site(&headers(&[("host", "srv"), ("origin", "https://srv:443")]), &none), "default port");
        assert!(!from_other_site(&headers(&[("host", "127.0.0.1:8080"), ("x-forwarded-host", "net.example.com"), ("origin", "https://net.example.com")]), &none));
        assert!(!from_other_site(&headers(&[("host", "SRV:8080"), ("origin", "http://srv:8080")]), &none), "case");
        let trusted = vec!["https://portal.example".to_string()];
        assert!(!from_other_site(&headers(&[("host", "srv:8080"), ("origin", "https://portal.example"), ("sec-fetch-site", "cross-site")]), &trusted));
    }

    #[test]
    fn csrf_applies_to_changes_by_cookie() {
        assert!(!needs_csrf_check(&Method::GET, &HeaderMap::new()));
        assert!(needs_csrf_check(&Method::POST, &HeaderMap::new()));
        assert!(needs_csrf_check(&Method::GET, &headers(&[("upgrade", "websocket")])));
        assert!(!needs_csrf_check(&Method::POST, &headers(&[("authorization", "Bearer x")])));
    }

    #[test]
    fn this_machine_rules() {
        let lo: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let far: SocketAddr = "192.168.1.9:5000".parse().unwrap();
        assert!(this_machine(lo, &headers(&[("host", "localhost:8080")])));
        assert!(this_machine(lo, &headers(&[("host", "[::1]:8080"), ("origin", "http://[::1]:8080")])));
        assert!(!this_machine(far, &headers(&[("host", "localhost:8080")])));
        assert!(!this_machine(lo, &headers(&[("host", "srv.example:8080")])));
        assert!(!this_machine(lo, &headers(&[("host", "localhost:8080"), ("x-forwarded-for", "10.0.0.4")])));
        assert!(!this_machine(lo, &headers(&[("host", "localhost:8080"), ("origin", "http://evil.example")])));
        let mapped: SocketAddr = "[::ffff:127.0.0.1]:5000".parse().unwrap();
        assert!(this_machine(mapped, &headers(&[("host", "127.0.0.1:8080")])));
    }

    #[test]
    fn login_throttling() {
        let l = Logins::new();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        let t0 = Instant::now();
        for _ in 0..5 {
            assert!(l.check(ip, "admin", t0).is_ok());
            l.failed(ip, "admin", t0);
        }
        // 6th try right away: wait 2 s.
        assert_eq!(l.check(ip, "Admin", t0), Err(2));
        assert!(l.check(ip, "admin", t0 + Duration::from_secs(2)).is_ok());
        l.failed(ip, "admin", t0 + Duration::from_secs(2));
        assert_eq!(l.check(ip, "admin", t0 + Duration::from_secs(2)), Err(4));
        // Another name from the same address is still free.
        assert!(l.check(ip, "bob", t0).is_ok());
        // Success resets the (address + name) counter.
        l.succeeded(ip, "admin");
        assert!(l.check(ip, "admin", t0 + Duration::from_secs(2)).is_ok());
        // Forgotten after 15 minutes.
        for _ in 0..8 {
            l.failed(ip, "carol", t0);
        }
        assert!(l.check(ip, "carol", t0).is_err());
        assert!(l.check(ip, "carol", t0 + FORGET).is_ok());
        // Per address: 20 tries over many names.
        let ip2: IpAddr = "10.0.0.2".parse().unwrap();
        for i in 0..20 {
            l.failed(ip2, &format!("u{i}"), t0);
        }
        assert!(l.check(ip2, "fresh", t0).is_err());
        // Cap at 5 minutes.
        for _ in 0..40 {
            l.failed(ip, "dave", t0);
        }
        assert_eq!(l.check(ip, "dave", t0), Err(300));
    }
}
