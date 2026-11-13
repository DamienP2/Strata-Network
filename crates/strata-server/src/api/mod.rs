//! The HTTP API (`/api/...`) and its helpers.

use std::fmt::Display;

use axum::extract::{DefaultBodyLimit, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::routing::{get, post, put};
use axum::Router;

use crate::auth::User;
use crate::state::Shared;

mod me;
mod security;
mod session;
mod system;
mod users;

pub const COOKIE: &str = "sn_session";
const COOKIE_MAX_AGE: u64 = 1_209_600;

/// Errors are short lowercase English plain text.
pub type ApiResult<T> = Result<T, (StatusCode, String)>;

pub fn bad(e: impl Display) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, e.to_string())
}

pub fn internal(e: impl Display) -> (StatusCode, String) {
    tracing::error!("{e}");
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub fn not_found(what: &str) -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, format!("no such {what}"))
}

pub fn require(allowed: bool) -> ApiResult<()> {
    if allowed {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, "you do not have the rights for this".into()))
    }
}

pub fn status(code: u16, msg: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), msg.into())
}

/// Runs blocking work (Argon2, files, outgoing HTTP) off the async threads.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> ApiResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(internal)
}

/// The token of a request: `Authorization: Bearer <token>`, else the session cookie.
pub fn token_from(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(t) = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")) {
            return Some(t.trim().to_string());
        }
    }
    for h in headers.get_all(header::COOKIE) {
        let Ok(h) = h.to_str() else { continue };
        for part in h.split(';') {
            if let Some((k, v)) = part.trim().split_once('=') {
                if k == COOKIE && !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

pub fn session_cookie(token: &str, secure: bool) -> HeaderValue {
    let s = format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={COOKIE_MAX_AGE}{}", if secure { "; Secure" } else { "" });
    HeaderValue::from_str(&s).expect("cookie")
}

pub fn clear_cookie(secure: bool) -> HeaderValue {
    let s = format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}", if secure { "; Secure" } else { "" });
    HeaderValue::from_str(&s).expect("cookie")
}

/// The signed-in user (401 "login required" otherwise).
pub struct CurrentUser(pub User);

/// The signed-in user and the token of the session.
pub struct Session {
    pub user: User,
    pub token: String,
}

/// The signed-in user, if any.
pub struct MaybeUser(pub Option<User>);

impl FromRequestParts<Shared> for Session {
    type Rejection = (StatusCode, String);
    async fn from_request_parts(parts: &mut Parts, s: &Shared) -> Result<Self, Self::Rejection> {
        let token = token_from(&parts.headers).ok_or((StatusCode::UNAUTHORIZED, "login required".to_string()))?;
        let user = s.auth.session_user(&token).ok_or((StatusCode::UNAUTHORIZED, "login required".to_string()))?;
        Ok(Session { user, token })
    }
}

impl FromRequestParts<Shared> for CurrentUser {
    type Rejection = (StatusCode, String);
    async fn from_request_parts(parts: &mut Parts, s: &Shared) -> Result<Self, Self::Rejection> {
        Session::from_request_parts(parts, s).await.map(|x| CurrentUser(x.user))
    }
}

impl FromRequestParts<Shared> for MaybeUser {
    type Rejection = (StatusCode, String);
    async fn from_request_parts(parts: &mut Parts, s: &Shared) -> Result<Self, Self::Rejection> {
        Ok(MaybeUser(token_from(&parts.headers).and_then(|t| s.auth.session_user(&t))))
    }
}

pub fn router() -> Router<Shared> {
    Router::new()
        // Session and sign-in
        .route("/api/health", get(session::health))
        .route("/api/auth/state", get(session::state))
        .route("/api/auth/login", post(session::login))
        .route("/api/auth/logout", post(session::logout))
        .route("/api/auth/setup", post(session::setup))
        .route("/api/auth/sso/start", get(session::sso_start))
        .route("/api/auth/sso/callback", get(session::sso_callback))
        // The signed-in user
        .route("/api/me/lang", put(me::lang))
        .route("/api/me/theme", put(me::theme))
        .route("/api/me/display", get(me::display).patch(me::patch_display))
        .route("/api/me/shortcuts", get(me::shortcuts).put(me::set_shortcuts))
        // Users
        .route("/api/users", get(users::list).post(users::create))
        .route("/api/users/{id}", put(users::update).delete(users::delete))
        // System
        .route("/api/system", get(system::info))
        .route("/api/system/config", put(system::put_config))
        .route("/api/system/restart", post(system::restart))
        .route("/api/system/metrics", get(system::metrics))
        .route("/api/system/tray", get(system::tray))
        .route("/api/system/network", put(system::network))
        .route("/api/system/restart-server", post(system::restart_server))
        .route("/api/system/stop", post(system::stop))
        .route("/api/system/logs", get(system::logs))
        .route("/api/system/logs/download", get(system::logs_download))
        .route("/api/system/health", get(system::health))
        .route("/api/system/health/journal", get(system::journal))
        .route("/api/system/health/seen", post(system::health_seen))
        .route("/api/system/health/export.zip", get(system::export))
        .route("/api/system/update", get(system::update_info))
        .route("/api/system/update/settings", put(system::update_settings))
        .route("/api/system/update/check", post(system::update_check))
        .route("/api/system/update/install", post(system::update_install))
        .route("/api/system/update/upload", post(system::update_upload).layer(DefaultBodyLimit::max(crate::update::MAX_PACKAGE as usize)))
        .route("/api/system/update/rollback", post(system::update_rollback))
        // Security
        .route("/api/security/tls", get(security::tls))
        .route("/api/security/tls/certificate", post(security::tls_certificate))
        .route("/api/security/tls/self-signed", post(security::tls_self_signed))
        .route("/api/security/sso/test", post(security::sso_test))
        // Real time
        .route("/ws", get(crate::ws::handler))
        .merge(crate::net::api::router())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("a=1; sn_session=abc; b=2"));
        assert_eq!(token_from(&h).as_deref(), Some("abc"));
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer xyz"));
        assert_eq!(token_from(&h).as_deref(), Some("xyz"));
        assert_eq!(token_from(&HeaderMap::new()), None);
        assert!(session_cookie("t", true).to_str().unwrap().ends_with("Max-Age=1209600; Secure"));
        assert_eq!(clear_cookie(false).to_str().unwrap(), "sn_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    }
}
