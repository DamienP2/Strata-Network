//! Strata Network server: one program that serves the API and the embedded web app.
//!
//! The library holds the server; `main.rs` is the command line.

use std::future::Future;
use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use clap::Args;
use tokio::sync::{watch, Notify};
use tower_http::cors::CorsLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

pub mod api;
pub mod auth;
pub mod config;
pub mod guard;
pub mod health;
pub mod logs;
pub mod net;
pub mod paths;
pub mod perf;
pub mod prefs;
pub mod service;
pub mod sso;
pub mod state;
pub mod tls;
pub mod update;
pub mod web;
pub mod ws;

use config::{HttpMode, HttpsMode, ServerConfig};
use state::{AppState, Events, Shared};

pub const DEFAULT_BIND: &str = "0.0.0.0:8080";

#[derive(Args, Clone, Debug, Default)]
pub struct RunArgs {
    /// Address to listen on (overrides the settings)
    #[arg(long, env = "SN_BIND")]
    pub bind: Option<SocketAddr>,
    /// Data folder
    #[arg(long, env = "SN_DATA_DIR")]
    pub data_dir: Option<PathBuf>,
    /// Serve the web app from this folder instead of the embedded copy
    #[arg(long, env = "SN_WEB_DIR")]
    pub web_dir: Option<PathBuf>,
    /// Address used when the settings have none (installers)
    #[arg(long, hide = true)]
    pub default_bind: Option<SocketAddr>,
    /// Started by the Windows service manager
    #[arg(long, hide = true)]
    pub service: bool,
}

/// `strata-network [run]`: logging, then the server until Ctrl-C / SIGTERM.
pub fn run(args: RunArgs) -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    if args.service {
        service::set_windows_service();
    }
    let data_dir = args.data_dir.clone().unwrap_or_else(paths::user_data_dir);
    std::fs::create_dir_all(&data_dir)?;
    init_logging(&data_dir, service::supervised());
    #[cfg(windows)]
    if args.service {
        let mut args = args;
        args.data_dir = Some(data_dir);
        return service::windows_run::start(args);
    }
    let mut args = args;
    args.data_dir = Some(data_dir);
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(serve(args, shutdown_signal()))
}

/// Console (colors on a terminal), or `logs/strata-network.log` under a service manager, plus
/// the in-memory ring.
pub fn init_logging(data_dir: &Path, to_file: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,tower_http=warn"));
    let registry = tracing_subscriber::registry().with(filter).with(logs::Ring);
    if to_file {
        let dir = data_dir.join("logs");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{}.log", update::BIN));
        if std::fs::metadata(&path).map(|m| m.len() > 10 * 1024 * 1024).unwrap_or(false) {
            let _ = std::fs::rename(&path, dir.join(format!("{}.log.1", update::BIN)));
        }
        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            logs::set_file(path);
            registry.with(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(Mutex::new(file))).init();
            return;
        }
    }
    registry.with(tracing_subscriber::fmt::layer().with_ansi(std::io::stdout().is_terminal())).init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    tracing::info!("stopping");
}

/// Binds, retrying 20 × 250 ms while the address is in use (the old process frees the port
/// after an update); another failure falls back to 0.0.0.0:8080 to keep the web app reachable.
async fn bind_retry(wanted: SocketAddr) -> anyhow::Result<tokio::net::TcpListener> {
    async fn attempt(addr: SocketAddr) -> std::io::Result<tokio::net::TcpListener> {
        let mut last = None;
        for _ in 0..20 {
            match tokio::net::TcpListener::bind(addr).await {
                Ok(l) => return Ok(l),
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                    last = Some(e);
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| std::io::Error::other("cannot bind")))
    }
    match attempt(wanted).await {
        Ok(l) => Ok(l),
        Err(e) => {
            let fallback: SocketAddr = DEFAULT_BIND.parse()?;
            if wanted == fallback {
                return Err(anyhow::anyhow!("cannot listen on {wanted}: {e}"));
            }
            tracing::error!("cannot listen on {wanted} ({e}): using {fallback}");
            attempt(fallback).await.map_err(|e| anyhow::anyhow!("cannot listen on {fallback}: {e}"))
        }
    }
}

/// Requests of the HTTPS listener say so (secure cookies, redirect URLs).
async fn mark_https(mut req: Request, next: Next) -> Response {
    req.headers_mut().insert("x-forwarded-proto", HeaderValue::from_static("https"));
    next.run(req).await
}

/// The plain HTTP listener while HTTPS is on: other machines are sent to HTTPS (or refused);
/// the machine itself (launcher) still gets HTTP.
async fn https_gate(State((mode, port)): State<(HttpMode, u16)>, req: Request, next: Next) -> Response {
    let peer = guard::peer_of(&req);
    if mode == HttpMode::Open || guard::this_machine(peer, req.headers()) || guard::is_https(req.headers()) {
        return next.run(req).await;
    }
    match mode {
        HttpMode::Redirect => {
            let host = guard::request_host(req.headers());
            let name = match host.rsplit_once(':') {
                Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !h.is_empty() => h.to_string(),
                _ => host.clone(),
            };
            let pq = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/");
            let to = if port == 443 { format!("https://{name}{pq}") } else { format!("https://{name}:{port}{pq}") };
            let mut res = StatusCode::PERMANENT_REDIRECT.into_response();
            if let Ok(v) = HeaderValue::from_str(&to) {
                res.headers_mut().insert(axum::http::header::LOCATION, v);
            }
            res
        }
        _ => (StatusCode::FORBIDDEN, "this server only answers over https").into_response(),
    }
}

fn build_app(shared: &Shared, args: &RunArgs) -> Router {
    let router = api::router();
    let router = match &args.web_dir {
        Some(dir) => web::with_dir(router, dir.clone()),
        None => router.fallback(web::embedded),
    };
    router
        .layer(middleware::from_fn(web::compress))
        .layer(middleware::from_fn_with_state(shared.clone(), guard::csrf))
        .layer(middleware::from_fn(guard::every_answer))
        .layer(CorsLayer::permissive())
        .with_state(shared.clone())
}

struct Https {
    addr: SocketAddr,
    handle: axum_server::Handle<SocketAddr>,
    task: tokio::task::JoinHandle<()>,
}

/// Starts the HTTPS listener of this cycle when HTTPS is on.
async fn start_https(data_dir: &Path, config: &ServerConfig, ip: std::net::IpAddr) -> Option<(SocketAddr, std::net::TcpListener, axum_server::tls_rustls::RustlsConfig)> {
    let https = &config.security.https;
    if https.mode == HttpsMode::Off {
        return None;
    }
    let dir = data_dir.to_path_buf();
    let mode = https.mode;
    let tls = match tokio::task::spawn_blocking(move || tls::load(&dir, mode)).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            tracing::error!("https is on but cannot start: {e}");
            return None;
        }
        Err(e) => {
            tracing::error!("https: {e}");
            return None;
        }
    };
    let addr = SocketAddr::new(ip, https.port);
    let listener = match bind_retry_exact(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("cannot listen on {addr} for https: {e}");
            return None;
        }
    };
    let addr = listener.local_addr().unwrap_or(addr);
    let std = match listener.into_std() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("https listener: {e}");
            return None;
        }
    };
    Some((addr, std, axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(tls))))
}

async fn bind_retry_exact(addr: SocketAddr) -> std::io::Result<tokio::net::TcpListener> {
    let mut last = None;
    for _ in 0..20 {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => return Ok(l),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("cannot bind")))
}

/// Runs the server until `shutdown` resolves. Settings changes rebuild the state and the
/// listeners without leaving the process ("hot restart").
pub async fn serve(args: RunArgs, shutdown: impl Future<Output = ()> + Send + 'static) -> anyhow::Result<()> {
    let data_dir = args.data_dir.clone().unwrap_or_else(paths::user_data_dir);
    std::fs::create_dir_all(&data_dir)?;
    let supervised = service::supervised();
    health::start(&data_dir);
    update::startup_guard(&data_dir, supervised);
    update::confirm_after(data_dir.clone(), Duration::from_secs(60));
    update::spawn_auto_check(data_dir.clone());
    perf::start();
    // Outside the loop: sessions, throttling and SSO sign-ins survive hot restarts.
    let auth = Arc::new(auth::Auth::load(&data_dir));
    let logins = Arc::new(guard::Logins::new());
    let sso = Arc::new(sso::Sso::new());
    // The network inventory, scans and listeners live across hot restarts too.
    let network = net::Net::start(&data_dir);
    let (stop_tx, stop_rx) = watch::channel(false);
    {
        let stop_tx = stop_tx.clone();
        tokio::spawn(async move {
            shutdown.await;
            let _ = stop_tx.send(true);
        });
    }
    tracing::info!("Strata Network {} starting (data in {})", env!("CARGO_PKG_VERSION"), data_dir.display());
    let mut first = true;
    loop {
        let config = ServerConfig::load(&data_dir);
        let wanted = args.bind.or(config.bind).or(args.default_bind).unwrap_or(DEFAULT_BIND.parse()?);
        let listener = bind_retry(wanted).await?;
        let bind = listener.local_addr()?;
        if first && auth.needs_setup() {
            tracing::warn!(
                "no user yet: open http://localhost:{}/ on this machine to create the admin account, or run `{} user add admin`",
                bind.port(),
                update::BIN
            );
        }
        first = false;
        let runtime = serde_json::json!({ "bind": bind.to_string(), "version": env!("CARGO_PKG_VERSION"), "pid": std::process::id() });
        if let Err(e) = paths::write_json(&data_dir.join("runtime.json"), &runtime, false) {
            tracing::error!("cannot write runtime.json: {e}");
        }
        let https = start_https(&data_dir, &config, bind.ip()).await;
        let https_mode = config.security.https.http;
        let https_port = config.security.https.port;
        let shared: Shared = Arc::new(AppState {
            data_dir: data_dir.clone(),
            auth: auth.clone(),
            config: ArcSwap::from_pointee(config),
            events: Events::new(256),
            bind,
            https_bind: https.as_ref().map(|h| h.0),
            started_at: Instant::now(),
            started_at_ms: paths::now_ms(),
            restart: Notify::new(),
            closing: watch::channel(false).0,
            stop: stop_tx.clone(),
            logins: logins.clone(),
            sso: sso.clone(),
            supervised,
            net: network.clone(),
        });
        network.attach(&shared);
        let app = build_app(&shared, &args);
        let https = https.map(|(addr, std, cfg)| {
            let handle = axum_server::Handle::<SocketAddr>::new();
            let app = app.clone().layer(middleware::from_fn(mark_https));
            let server = axum_server::from_tcp_rustls(std, cfg).map(|s| s.handle(handle.clone()));
            let task = tokio::spawn(async move {
                match server {
                    Ok(server) => {
                        if let Err(e) = server.serve(app.into_make_service_with_connect_info::<SocketAddr>()).await {
                            tracing::error!("https server: {e}");
                        }
                    }
                    Err(e) => tracing::error!("https server: {e}"),
                }
            });
            tracing::info!("listening on https://{addr}");
            Https { addr, handle, task }
        });
        let app = if https.is_some() { app.layer(middleware::from_fn_with_state((https_mode, https_port), https_gate)) } else { app };
        tracing::info!("listening on http://{bind}");

        let cycle = shared.clone();
        let mut stop = stop_rx.clone();
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(async move {
                tokio::select! {
                    _ = stop.wait_for(|v| *v) => {},
                    _ = cycle.restart.notified() => {},
                }
                let _ = cycle.closing.send(true);
            })
            .await?;
        if let Some(h) = https {
            h.handle.graceful_shutdown(Some(Duration::from_secs(5)));
            let _ = h.task.await;
            tracing::debug!("https on {} stopped", h.addr);
        }
        if *stop_rx.borrow() {
            break;
        }
        tracing::info!("restarting in place");
    }
    health::stopped();
    Ok(())
}

