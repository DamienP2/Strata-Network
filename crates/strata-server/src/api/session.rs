//! Sign-in: state, password login, first admin, logout, directory (SSO) sign-in.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::{blocking, clear_cookie, session_cookie, token_from, ApiResult, MaybeUser};
use crate::auth::{Auth, Role, User, UserInput};
use crate::guard::{client_ip, is_https, request_host, this_machine};
use crate::sso::{Fail, SignedIn};
use crate::state::{Audience, Shared};

/// `GET /api/health` (public): for the launcher and the update page.
pub async fn health(State(s): State<Shared>) -> Json<serde_json::Value> {
    let cfg = s.config.load();
    Json(json!({
        "name": cfg.name,
        "version": env!("CARGO_PKG_VERSION"),
        "bind": s.bind.to_string(),
        "needsSetup": s.auth.needs_setup(),
    }))
}

/// `GET /api/auth/state`
pub async fn state(State(s): State<Shared>, MaybeUser(user): MaybeUser) -> Json<serde_json::Value> {
    let cfg = s.config.load();
    let sso = &cfg.security.sso;
    Json(json!({
        "needsSetup": s.auth.needs_setup(),
        "user": user.map(|u| u.public()),
        "serverName": cfg.name,
        "sso": if sso.active() { Some(sso.button_name()) } else { None },
        "localAccounts": sso.local_accounts_str(),
    }))
}

/// `{user, token}` with the session cookie.
fn signed_in(user: &User, token: &str, secure: bool) -> Response {
    let mut res = Json(json!({ "user": user.public(), "token": token })).into_response();
    res.headers_mut().insert(header::SET_COOKIE, session_cookie(token, secure));
    res
}

#[derive(Deserialize)]
pub struct Login {
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
}

/// `POST /api/auth/login`
pub async fn login(State(s): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(b): Json<Login>) -> ApiResult<Response> {
    let ip = client_ip(peer, &headers);
    if let Err(wait) = s.logins.check(ip, &b.username, Instant::now()) {
        return Err((StatusCode::TOO_MANY_REQUESTS, format!("too many tries: wait {wait} s")));
    }
    let Some(user) = s.auth.verify(b.username.clone(), b.password).await else {
        s.logins.failed(ip, &b.username, Instant::now());
        tracing::info!("failed sign-in for \"{}\" from {ip}", b.username);
        return Err((StatusCode::UNAUTHORIZED, "wrong user name or password".into()));
    };
    s.logins.succeeded(ip, &b.username);
    let cfg = s.config.load();
    if !cfg.security.sso.allows_local(user.role) && !this_machine(peer, &headers) {
        return Err((StatusCode::FORBIDDEN, format!("sign in with {}", cfg.security.sso.button_name())));
    }
    let token = s.auth.open_session(&user.id);
    tracing::info!("{} signed in from {ip}", user.username);
    Ok(signed_in(&user, &token, is_https(&headers)))
}

/// `POST /api/auth/logout`
pub async fn logout(State(s): State<Shared>, headers: HeaderMap) -> Response {
    if let Some(t) = token_from(&headers) {
        s.auth.close_session(&t);
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut().insert(header::SET_COOKIE, clear_cookie(is_https(&headers)));
    res
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    display_name: String,
}

/// `POST /api/auth/setup`: the first admin, from the server itself only.
pub async fn setup(State(s): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(b): Json<Setup>) -> ApiResult<Response> {
    if !this_machine(peer, &headers) {
        return Err((StatusCode::FORBIDDEN, "the first account can only be created on the server itself".into()));
    }
    if !s.auth.needs_setup() {
        return Err((StatusCode::CONFLICT, "already set up".into()));
    }
    let auth = s.auth.clone();
    let input = UserInput { username: b.username, display_name: b.display_name, password: b.password, role: Some(Role::Admin), ..Default::default() };
    let user = blocking(move || auth.create(input, false)).await?.map_err(<(StatusCode, String)>::from)?;
    tracing::info!("first admin \"{}\" created", user.username);
    let token = s.auth.open_session(&user.id);
    s.changed_for(Audience::Admins, "users");
    Ok(signed_in(&user, &token, is_https(&headers)))
}

fn origin_of(headers: &HeaderMap) -> String {
    format!("{}://{}", if is_https(headers) { "https" } else { "http" }, request_host(headers))
}

fn redirect(to: &str) -> Response {
    let mut res = StatusCode::SEE_OTHER.into_response();
    if let Ok(v) = to.parse() {
        res.headers_mut().insert(header::LOCATION, v);
    }
    res.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    res
}

/// `GET /api/auth/sso/start?next=/path`
pub async fn sso_start(State(s): State<Shared>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let cfg = s.config.load_full();
    let next = q.get("next").cloned().unwrap_or_else(|| "/".into());
    let origin = origin_of(&headers);
    let sso = s.sso.clone();
    match blocking(move || sso.start(&cfg.security.sso, &origin, &next)).await {
        Ok(Ok(url)) => redirect(&url),
        Ok(Err(e)) => {
            tracing::warn!("sso sign-in cannot start: {e}");
            redirect(&Fail::failed(e).redirect())
        }
        Err((_, e)) => redirect(&Fail::failed(e).redirect()),
    }
}

/// Finds or creates the account of a directory identity.
fn account_for(auth: &Auth, signed: &SignedIn) -> Result<(User, bool), Fail> {
    let id = &signed.identity;
    let existing = auth.find_by_sso(&id.key).or_else(|| {
        let addr = id.address.as_deref()?;
        auth.list().into_iter().find(|u| u.sso.is_empty() && u.username.contains('@') && u.username.eq_ignore_ascii_case(addr))
    });
    let to_fail = |e: crate::auth::AuthError| Fail::failed(e.to_string());
    match existing {
        Some(u) => {
            if u.disabled {
                return Err(Fail { kind: "disabled", detail: String::new() });
            }
            let role = match signed.role {
                Some(r) => r,
                None if u.role_fixed => u.role,
                None => return Err(Fail { kind: "noGroup", detail: String::new() }),
            };
            let changed = u.sso != id.key || (!u.role_fixed && u.role != role);
            let user = auth.sso_sync(&u.id, &id.key, role).map_err(to_fail)?;
            Ok((user, changed))
        }
        None => {
            let role = signed.role.ok_or(Fail { kind: "noGroup", detail: String::new() })?;
            let user = auth.create_sso(&id.login, &id.name, role, &id.key).map_err(to_fail)?;
            tracing::info!("account \"{}\" created from the directory", user.username);
            Ok((user, true))
        }
    }
}

/// `GET /api/auth/sso/callback`
pub async fn sso_callback(State(s): State<Shared>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let cfg = s.config.load_full();
    let sso = s.sso.clone();
    let auth = s.auth.clone();
    let outcome = blocking(move || {
        let signed = sso.callback(&cfg.security.sso, &q)?;
        let (user, changed) = account_for(&auth, &signed)?;
        Ok::<_, Fail>((user, changed, signed.next))
    })
    .await;
    match outcome {
        Ok(Ok((user, changed, next))) => {
            if changed {
                s.changed_for(Audience::Admins, "users");
            }
            let token = s.auth.open_session(&user.id);
            tracing::info!("{} signed in with the directory", user.username);
            let mut res = redirect(&next);
            res.headers_mut().insert(header::SET_COOKIE, session_cookie(&token, is_https(&headers)));
            res
        }
        Ok(Err(fail)) => {
            if fail.kind == "failed" {
                tracing::warn!("sso sign-in failed: {}", fail.detail);
            }
            redirect(&fail.redirect())
        }
        Err((_, e)) => redirect(&Fail::failed(e).redirect()),
    }
}
