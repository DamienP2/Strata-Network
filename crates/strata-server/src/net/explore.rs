//! Raw SNMP explorer: walks and gets on an agent, shown as they are read.
//! Read-only: GET and GETBULK only (there is no SET anywhere in Strata Network).

use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use strata_net::snmp::{self as snmp, Oid, Value};

use crate::api::{bad, not_found, require, status, ApiResult, Session};
use crate::state::Shared;

pub const DEFAULT_MAX: usize = 2000;
pub const MAX_ROWS: usize = 10_000;
/// The longest a walk or a get may take.
pub const DEADLINE: Duration = Duration::from_secs(20);
/// OIDs in one `get` request.
pub const MAX_GET: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    pub oid: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hex: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Walk {
    pub rows: Vec<Row>,
    /// More cells were there (`max` reached), or the walk was stopped (deadline or an error after the first rows).
    pub truncated: bool,
    /// Why the walk stopped before its end, when it did after reading rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub ms: u64,
}

/// A numeric OID ("1.3.6.1.2.1.1", a leading dot allowed).
pub fn parse_oid(s: &str) -> Result<Oid, String> {
    let s = s.trim().trim_start_matches('.');
    let parts: Result<Vec<u32>, _> = s.split('.').map(str::parse::<u32>).collect();
    match parts {
        Ok(v) if v.len() >= 2 && v.len() <= 128 && v[0] <= 2 && (v[0] == 2 || v[1] < 40) => Ok(Oid(v)),
        _ => Err("the oid is written with numbers and dots (1.3.6.1.2.1.1)".into()),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

/// Printable text: valid UTF-8 without control characters (tabs and line ends allowed; trailing NULs ignored).
fn printable(b: &[u8]) -> Option<&str> {
    let end = b.iter().rposition(|x| *x != 0).map_or(0, |i| i + 1);
    let s = std::str::from_utf8(&b[..end]).ok()?;
    s.chars().all(|c| !c.is_control() || matches!(c, '\t' | '\r' | '\n')).then_some(s)
}

/// The text shown for a value.
pub fn text(v: &Value) -> String {
    match v {
        Value::Int(i) => i.to_string(),
        Value::Bytes(b) => match printable(b) {
            Some(s) if !b.is_empty() && (b.len() != 6 || !s.is_empty()) => s.to_string(),
            _ if b.len() == 6 => b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(":"),
            _ => hex(b),
        },
        Value::Oid(o) => o.to_string(),
        Value::Ip(ip) => ip.to_string(),
        Value::Counter(n) | Value::Counter64(n) => n.to_string(),
        Value::Gauge(n) | Value::Ticks(n) => n.to_string(),
        Value::Null | Value::Missing | Value::NoSuchObject | Value::NoSuchInstance | Value::EndOfView => String::new(),
    }
}

pub fn row(o: &Oid, v: &Value) -> Row {
    let hex = match v {
        Value::Bytes(b) => Some(hex(b)),
        _ => None,
    };
    Row { oid: o.to_string(), kind: v.kind(), text: text(v), hex }
}

/// Every cell under `base` (GETBULK steps), at most `max`. A base with nothing under it is read
/// with a GET (a scalar like sysDescr.0). Fails only when nothing could be read.
pub async fn walk(sess: &snmp::Session, base: &Oid, max: usize) -> Result<Walk, String> {
    let started = Instant::now();
    let max = max.clamp(1, MAX_ROWS);
    let mut rows = Vec::new();
    let mut cur = base.clone();
    let mut truncated = false;
    let mut error = None;
    'outer: loop {
        let left = DEADLINE.saturating_sub(started.elapsed());
        if left.is_zero() {
            truncated = true;
            error = Some("timed out".to_string());
            break;
        }
        let reps = (max + 1 - rows.len()).clamp(1, 25) as i64;
        let binds = match tokio::time::timeout(left, sess.bulk(&cur, reps)).await {
            Ok(Ok(b)) => b,
            Ok(Err(e)) if rows.is_empty() => return Err(e.to_string()),
            Ok(Err(e)) => {
                truncated = true;
                error = Some(e.to_string());
                break;
            }
            Err(_) if rows.is_empty() => return Err("no snmp answer (address, community or firewall?)".into()),
            Err(_) => {
                truncated = true;
                error = Some("timed out".to_string());
                break;
            }
        };
        if binds.is_empty() {
            break;
        }
        for (o, v) in binds {
            if !o.starts_with(base) || v == Value::EndOfView || o <= cur {
                break 'outer;
            }
            if rows.len() == max {
                truncated = true;
                break 'outer;
            }
            rows.push(row(&o, &v));
            cur = o;
        }
    }
    if rows.is_empty() && error.is_none() {
        let left = DEADLINE.saturating_sub(started.elapsed()).max(Duration::from_secs(1));
        if let Ok(Ok(got)) = tokio::time::timeout(left, sess.get(std::slice::from_ref(base))).await {
            rows.extend(got.iter().filter(|(_, v)| !v.is_absent()).map(|(o, v)| row(o, v)));
        }
    }
    Ok(Walk { rows, truncated, error, ms: started.elapsed().as_millis() as u64 })
}

/// The cells asked (a missing one comes back as NoSuchObject / NoSuchInstance).
pub async fn get(sess: &snmp::Session, oids: &[Oid]) -> Result<Vec<Row>, String> {
    let started = Instant::now();
    let mut rows = Vec::new();
    for chunk in oids.chunks(16) {
        let left = DEADLINE.saturating_sub(started.elapsed());
        let got = tokio::time::timeout(left, sess.get(chunk)).await.map_err(|_| "timed out".to_string())?.map_err(|e| e.to_string())?;
        rows.extend(got.iter().map(|(o, v)| row(o, v)));
    }
    Ok(rows)
}

// ── Routes ──

pub fn router() -> Router<Shared> {
    Router::new().route("/api/snmp/{id}/walk", post(walk_route)).route("/api/snmp/{id}/get", post(get_route))
}

async fn open(s: &Shared, id: &str) -> ApiResult<snmp::Session> {
    let a = s.net.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner).snmp.iter().find(|x| x.id == id).cloned().ok_or_else(|| not_found("snmp agent"))?;
    super::snmp::session(&a).await.map_err(|e| status(502, e))
}

#[derive(Deserialize)]
struct WalkBody {
    oid: String,
    #[serde(default)]
    max: Option<usize>,
}

async fn walk_route(State(s): State<Shared>, me: Session, Path(id): Path<String>, Json(b): Json<WalkBody>) -> ApiResult<Json<Walk>> {
    require(me.user.is_admin())?;
    let base = parse_oid(&b.oid).map_err(bad)?;
    let sess = open(&s, &id).await?;
    walk(&sess, &base, b.max.unwrap_or(DEFAULT_MAX)).await.map(Json).map_err(|e| status(502, e))
}

#[derive(Deserialize)]
struct GetBody {
    oids: Vec<String>,
}

async fn get_route(State(s): State<Shared>, me: Session, Path(id): Path<String>, Json(b): Json<GetBody>) -> ApiResult<Json<serde_json::Value>> {
    require(me.user.is_admin())?;
    if b.oids.is_empty() {
        return Err(bad("no oid to read"));
    }
    if b.oids.len() > MAX_GET {
        return Err(bad(format!("at most {MAX_GET} oids at once")));
    }
    let oids = b.oids.iter().map(|o| parse_oid(o)).collect::<Result<Vec<_>, _>>().map_err(bad)?;
    let started = Instant::now();
    let sess = open(&s, &id).await?;
    let rows = get(&sess, &oids).await.map_err(|e| status(502, e))?;
    Ok(Json(json!({ "rows": rows, "ms": started.elapsed().as_millis() as u64 })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn oids() {
        assert_eq!(parse_oid(".1.3.6.1.2.1.1").unwrap(), Oid(vec![1, 3, 6, 1, 2, 1, 1]));
        assert!(parse_oid("1").is_err());
        assert!(parse_oid("1.3.x").is_err());
        assert!(parse_oid("1.3..6").is_err());
        assert!(parse_oid("3.1").is_err());
        assert!(parse_oid("1.40").is_err());
        assert!(parse_oid("sysDescr").is_err());
    }

    #[test]
    fn values() {
        let r = row(&Oid(vec![1, 3, 6, 1, 2, 1, 1, 1, 0]), &Value::Bytes(b"Cisco NX-OS\r\n".to_vec()));
        assert_eq!((r.kind, r.text.as_str(), r.hex.as_deref()), ("OctetString", "Cisco NX-OS\r\n", Some("43 69 73 63 6F 20 4E 58 2D 4F 53 0D 0A")));
        // Six bytes that are not text: a MAC.
        assert_eq!(text(&Value::Bytes(vec![0x00, 0x11, 0x32, 0xab, 0xcd, 0xef])), "00:11:32:AB:CD:EF");
        // Six printable bytes stay text.
        assert_eq!(text(&Value::Bytes(b"Uplink".to_vec())), "Uplink");
        assert_eq!(text(&Value::Bytes(vec![0x80, 0x00, 0x01])), "80 00 01");
        assert_eq!(text(&Value::Bytes(b"name\0".to_vec())), "name");
        assert_eq!(row(&Oid(vec![1, 3]), &Value::Bytes(vec![])).hex.as_deref(), Some(""));
        assert_eq!(text(&Value::Oid(Oid(vec![1, 3, 6, 1, 4, 1, 9]))), "1.3.6.1.4.1.9");
        assert_eq!(text(&Value::Ip(Ipv4Addr::new(10, 0, 0, 1))), "10.0.0.1");
        assert_eq!(text(&Value::Int(-3)), "-3");
        assert_eq!(text(&Value::Ticks(123_456)), "123456");
        let r = row(&Oid(vec![1, 3]), &Value::Counter64(1 << 40));
        assert_eq!((r.kind, r.text.as_str(), r.hex), ("Counter64", "1099511627776", None));
        assert_eq!(Value::Counter(5).kind(), "Counter32");
        assert_eq!(Value::Gauge(5).kind(), "Gauge32");
        assert_eq!(Value::NoSuchInstance.kind(), "NoSuchInstance");
        assert_eq!(text(&Value::NoSuchObject), "");
        let j = serde_json::to_value(row(&Oid(vec![1, 3]), &Value::EndOfView)).unwrap();
        assert_eq!(j, serde_json::json!({"oid": "1.3", "type": "EndOfMibView", "text": ""}));
    }
}
