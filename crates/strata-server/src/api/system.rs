//! System page: information, settings, restart, metrics, launcher routes, logs, health, updates.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{bad, blocking, internal, require, status, ApiResult, CurrentUser, MaybeUser};
use crate::config::{ServerConfig, MASK};
use crate::guard::this_machine;
use crate::state::{AppState, Audience, Shared};
use crate::{health, logs, perf, update};

/// The machine's network cards: `[{name, ip}]` (IPv4, without loopback).
pub fn interfaces() -> Vec<Value> {
    let mut out: Vec<Value> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback() && i.ip().is_ipv4())
        .map(|i| json!({ "name": i.name, "ip": i.ip().to_string() }))
        .collect();
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    out
}

fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().to_string()
}

/// The information block of `GET /api/system` (also in the support export).
pub fn system_json(s: &AppState, admin: bool) -> Value {
    let cfg = s.config.load();
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "hostname": hostname(),
        "cpus": perf::cpu_threads(),
        "dataDir": s.data_dir.display().to_string(),
        "bind": s.bind.to_string(),
        "httpsBind": s.https_bind.map(|b| b.to_string()),
        "uptimeSecs": s.uptime_secs(),
        "timezone": cfg.timezone_name(),
        "config": if admin { serde_json::to_value(cfg.masked()).ok() } else { None },
        "interfaces": interfaces(),
    })
}

/// `GET /api/system`
pub async fn info(State(s): State<Shared>, CurrentUser(me): CurrentUser) -> Json<Value> {
    Json(system_json(&s, me.is_admin()))
}

/// Whether the server could listen on `addr` (the address it holds now counts as free).
fn test_bind(addr: SocketAddr, current: SocketAddr) -> Result<(), std::io::Error> {
    if addr == current {
        return Ok(());
    }
    match std::net::TcpListener::bind(addr) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && addr.port() == current.port() => Ok(()),
        Err(e) => Err(e),
    }
}

/// `PUT /api/system/config` (admin): saves and restarts the server in place.
pub async fn put_config(State(s): State<Shared>, CurrentUser(me): CurrentUser, Json(mut cfg): Json<ServerConfig>) -> ApiResult<StatusCode> {
    require(me.is_admin())?;
    cfg.normalize();
    cfg.validate().map_err(bad)?;
    let old = s.config.load_full();
    if cfg.security.sso.client_secret == MASK {
        cfg.security.sso.client_secret = old.security.sso.client_secret.clone();
    }
    if cfg.bind != old.bind {
        if let Some(b) = cfg.bind {
            test_bind(b, s.bind).map_err(|e| bad(format!("cannot listen on {b}: {e}")))?;
        }
    }
    let https = &cfg.security.https;
    if https.mode != crate::config::HttpsMode::Off {
        let addr = SocketAddr::new(cfg.bind.unwrap_or(s.bind).ip(), https.port);
        let current = s.https_bind.unwrap_or(SocketAddr::new(s.bind.ip(), 0));
        test_bind(addr, current).map_err(|e| bad(format!("cannot listen on {addr}: {e}")))?;
    }
    let dir = s.data_dir.clone();
    let to_save = cfg.clone();
    blocking(move || to_save.save(&dir)).await?.map_err(internal)?;
    tracing::info!("{} changed the server settings", me.username);
    s.config.store(Arc::new(cfg));
    s.changed("config");
    s.restart.notify_one();
    Ok(StatusCode::ACCEPTED)
}

/// `POST /api/system/restart` (admin): restart in place.
pub async fn restart(State(s): State<Shared>, CurrentUser(me): CurrentUser) -> ApiResult<StatusCode> {
    require(me.is_admin())?;
    tracing::info!("{} restarted the server", me.username);
    s.restart.notify_one();
    Ok(StatusCode::ACCEPTED)
}

/// `GET /api/system/metrics`
pub async fn metrics(CurrentUser(me): CurrentUser) -> Json<Value> {
    Json(perf::metrics(me.is_admin()))
}

/// The launcher on this machine, or an admin.
fn machine_or_admin(peer: SocketAddr, headers: &HeaderMap, user: &Option<crate::auth::User>) -> ApiResult<()> {
    if this_machine(peer, headers) || user.as_ref().is_some_and(|u| u.is_admin()) {
        Ok(())
    } else if user.is_none() {
        Err((StatusCode::UNAUTHORIZED, "login required".into()))
    } else {
        require(false)
    }
}

fn urls(s: &AppState) -> Vec<String> {
    let mut out = Vec::new();
    let mut add = |scheme: &str, b: SocketAddr| {
        if b.ip().is_unspecified() {
            out.push(format!("{scheme}://localhost:{}", b.port()));
            for i in interfaces() {
                out.push(format!("{scheme}://{}:{}", i["ip"].as_str().unwrap_or(""), b.port()));
            }
        } else {
            out.push(format!("{scheme}://{b}"));
        }
    };
    if let Some(h) = s.https_bind {
        add("https", h);
    }
    add("http", s.bind);
    out
}

/// `GET /api/system/tray` (this machine or admin)
pub async fn tray(State(s): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, MaybeUser(user): MaybeUser) -> ApiResult<Json<Value>> {
    machine_or_admin(peer, &headers, &user)?;
    let cfg = s.config.load();
    Ok(Json(json!({
        "name": cfg.name,
        "version": env!("CARGO_PKG_VERSION"),
        "bind": s.bind.to_string(),
        "uptimeSecs": s.uptime_secs(),
        "urls": urls(&s),
        "interfaces": interfaces(),
        "control": s.bind.ip().to_string(),
        "update": update::tray_status(),
    })))
}

#[derive(Deserialize)]
pub struct Network {
    /// The address of the card the web app listens on; "" or "0.0.0.0" = all cards.
    #[serde(default)]
    control: Option<String>,
    #[serde(default)]
    port: Option<u16>,
}

/// `PUT /api/system/network {control?, port?}` (this machine or admin)
pub async fn network(
    State(s): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    MaybeUser(user): MaybeUser,
    Json(b): Json<Network>,
) -> ApiResult<StatusCode> {
    machine_or_admin(peer, &headers, &user)?;
    let ip: IpAddr = match b.control.as_deref().map(str::trim) {
        None => s.bind.ip(),
        Some("") | Some("all") => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        Some(c) => c.parse().map_err(|_| bad("unknown network card"))?,
    };
    if !ip.is_unspecified() && !ip.is_loopback() && !interfaces().iter().any(|i| i["ip"].as_str() == Some(&ip.to_string())) {
        return Err(bad("this address is not on this machine"));
    }
    let port = b.port.unwrap_or(s.bind.port());
    if port == 0 {
        return Err(bad("the port is between 1 and 65535"));
    }
    let addr = SocketAddr::new(ip, port);
    if let Err(e) = test_bind(addr, s.bind) {
        return Err(if e.kind() == std::io::ErrorKind::AddrInUse { status(409, format!("port {port} is already in use")) } else { bad(format!("cannot listen on {addr}: {e}")) });
    }
    let mut cfg = (*s.config.load_full()).clone();
    cfg.bind = Some(addr);
    let dir = s.data_dir.clone();
    let to_save = cfg.clone();
    blocking(move || to_save.save(&dir)).await?.map_err(internal)?;
    tracing::info!("the web app now listens on {addr}");
    s.config.store(Arc::new(cfg));
    s.changed("config");
    s.restart.notify_one();
    Ok(StatusCode::ACCEPTED)
}

/// `POST /api/system/restart-server` (this machine or admin): restarts the process.
pub async fn restart_server(State(s): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, MaybeUser(user): MaybeUser) -> ApiResult<StatusCode> {
    machine_or_admin(peer, &headers, &user)?;
    tracing::info!("server process restart asked");
    update::restart_soon(s.supervised);
    Ok(StatusCode::ACCEPTED)
}

/// `POST /api/system/stop` (this machine or admin)
pub async fn stop(State(s): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, MaybeUser(user): MaybeUser) -> ApiResult<StatusCode> {
    machine_or_admin(peer, &headers, &user)?;
    tracing::info!("server stop asked");
    let stop = s.stop.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        #[cfg(unix)]
        {
            let _ = &stop;
            // SAFETY: sending SIGTERM to ourselves; the signal handler stops the server cleanly.
            unsafe {
                libc::kill(libc::getpid(), libc::SIGTERM);
            }
        }
        #[cfg(not(unix))]
        let _ = stop.send(true);
    });
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
pub struct LogQuery {
    #[serde(default)]
    after: u64,
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// `GET /api/system/logs?after=&level=&limit=` (admin)
pub async fn logs(CurrentUser(me): CurrentUser, Query(q): Query<LogQuery>) -> ApiResult<Json<Value>> {
    require(me.is_admin())?;
    let limit = q.limit.unwrap_or(1000).clamp(1, logs::KEEP);
    let (entries, last) = logs::query(q.after, q.level.as_deref().filter(|l| !l.is_empty()), limit);
    Ok(Json(json!({ "entries": entries, "last": last, "file": logs::file() })))
}

/// `GET /api/system/logs/download` (admin)
pub async fn logs_download(CurrentUser(me): CurrentUser) -> ApiResult<Response> {
    require(me.is_admin())?;
    let text = logs::as_text();
    Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8"), (header::CONTENT_DISPOSITION, "attachment; filename=\"strata.log\"")], text).into_response())
}

/// `GET /api/system/health` (admin)
pub async fn health(State(s): State<Shared>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Value>> {
    require(me.is_admin())?;
    let dir = s.data_dir.clone();
    let v = blocking(move || health::summary(perf::disk_of(&dir))).await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct JournalQuery {
    #[serde(default)]
    days: Option<i64>,
}

/// `GET /api/system/health/journal?days=7` (admin) → `{entries}` newest first
pub async fn journal(CurrentUser(me): CurrentUser, Query(q): Query<JournalQuery>) -> ApiResult<Json<Value>> {
    require(me.is_admin())?;
    let days = q.days.unwrap_or(7).clamp(1, 30);
    let entries = blocking(move || health::journal(days, 5000)).await?;
    Ok(Json(json!({ "entries": entries })))
}

/// `POST /api/system/health/seen` (admin)
pub async fn health_seen(State(s): State<Shared>, CurrentUser(me): CurrentUser) -> ApiResult<StatusCode> {
    require(me.is_admin())?;
    blocking(health::mark_seen).await?.map_err(internal)?;
    s.changed_for(Audience::Admins, "health");
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/system/health/export.zip` (admin): everything for the support.
pub async fn export(State(s): State<Shared>, CurrentUser(me): CurrentUser) -> ApiResult<Response> {
    require(me.is_admin())?;
    let system = serde_json::to_vec_pretty(&system_json(&s, true)).unwrap_or_default();
    let metrics = serde_json::to_vec_pretty(&perf::metrics(true)).unwrap_or_default();
    let dir = s.data_dir.clone();
    let zip = blocking(move || {
        let summary = serde_json::to_vec_pretty(&health::summary(perf::disk_of(&dir))).unwrap_or_default();
        let log = logs::as_text().into_bytes();
        health::export_zip(&dir, &[("system.json", system), ("metrics.json", metrics), ("health.json", summary), ("strata.log", log)])
    })
    .await?
    .map_err(internal)?;
    let day = jiff::Zoned::now().date();
    let disposition = format!("attachment; filename=\"strata-network-{day}.zip\"");
    Ok(([(header::CONTENT_TYPE, "application/zip".to_string()), (header::CONTENT_DISPOSITION, disposition)], zip).into_response())
}

// ---------------------------------------------------------------- updates

/// `GET /api/system/update` (admin)
pub async fn update_info(State(s): State<Shared>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Value>> {
    require(me.is_admin())?;
    let dir = s.data_dir.clone();
    let supervised = s.supervised;
    Ok(Json(blocking(move || update::info(&dir, supervised)).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettings {
    #[serde(default)]
    repo: Option<String>,
    /// `null` (or absent) = keep, `""` = remove.
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    auto_check: Option<bool>,
}

/// `PUT /api/system/update/settings` (admin)
pub async fn update_settings(State(s): State<Shared>, CurrentUser(me): CurrentUser, Json(b): Json<UpdateSettings>) -> ApiResult<StatusCode> {
    require(me.is_admin())?;
    let dir = s.data_dir.clone();
    blocking(move || update::set_settings(&dir, b.repo.as_deref(), b.token.as_deref(), b.auto_check)).await?.map_err(|(c, m)| status(c, m))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/system/update/check` (admin or the launcher) → the latest release or `null`
pub async fn update_check(State(s): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, MaybeUser(user): MaybeUser) -> ApiResult<Json<Value>> {
    machine_or_admin(peer, &headers, &user)?;
    let dir = s.data_dir.clone();
    let latest = blocking(move || update::check(&dir)).await?.map_err(|e| status(502, e))?;
    Ok(Json(serde_json::to_value(latest).unwrap_or(Value::Null)))
}

#[derive(Deserialize)]
pub struct InstallQuery {
    #[serde(default)]
    version: Option<String>,
}

/// `POST /api/system/update/install[?version=x.y.z]` (admin or the launcher) → 202
pub async fn update_install(
    State(s): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    MaybeUser(user): MaybeUser,
    Query(q): Query<InstallQuery>,
) -> ApiResult<StatusCode> {
    machine_or_admin(peer, &headers, &user)?;
    let dir = s.data_dir.clone();
    let supervised = s.supervised;
    blocking(move || update::start_install(&dir, q.version.filter(|v| !v.is_empty()), supervised)).await?.map_err(|(c, m)| status(c, m))?;
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadQuery {
    #[serde(default)]
    allow_older: bool,
}

/// `POST /api/system/update/upload?allowOlder=false` (admin; body = the package)
pub async fn update_upload(State(s): State<Shared>, CurrentUser(me): CurrentUser, Query(q): Query<UploadQuery>, body: Bytes) -> ApiResult<Json<Value>> {
    require(me.is_admin())?;
    if body.is_empty() {
        return Err(bad("the file is empty"));
    }
    let dir = s.data_dir.clone();
    let version = blocking(move || update::install_bytes(&dir, &body, q.allow_older)).await?.map_err(bad)?;
    tracing::info!("{} installed version {version} from a package", me.username);
    update::restart_soon(s.supervised);
    Ok(Json(json!({ "version": version })))
}

/// `POST /api/system/update/rollback` (admin)
pub async fn update_rollback(State(s): State<Shared>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Value>> {
    require(me.is_admin())?;
    let dir = s.data_dir.clone();
    let version = blocking(move || update::rollback(&dir)).await?.map_err(bad)?;
    tracing::info!("{} went back to version {version}", me.username);
    update::restart_soon(s.supervised);
    Ok(Json(json!({ "version": version })))
}
