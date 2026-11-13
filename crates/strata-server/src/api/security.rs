//! Security page: HTTPS certificates and the SSO connection test (admins only).

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{bad, blocking, require, status, ApiResult, CurrentUser};
use crate::config::HttpsMode;
use crate::state::{Audience, Shared};
use crate::tls;

async fn tls_json(s: &Shared) -> ApiResult<Value> {
    let cfg = s.config.load_full();
    let dir = s.data_dir.clone();
    let https_bind = s.https_bind;
    blocking(move || {
        let h = &cfg.security.https;
        json!({
            "mode": h.mode,
            "port": h.port,
            "http": h.http,
            "active": https_bind.is_some(),
            "httpsBind": https_bind.map(|b| b.to_string()),
            "selfSigned": tls::info(&tls::files(&dir, HttpsMode::SelfSigned).0),
            "custom": tls::info(&tls::files(&dir, HttpsMode::Custom).0),
        })
    })
    .await
}

/// `GET /api/security/tls`
pub async fn tls(State(s): State<Shared>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Value>> {
    require(me.is_admin())?;
    Ok(Json(tls_json(&s).await?))
}

#[derive(Deserialize)]
pub struct Certificate {
    #[serde(default)]
    cert: String,
    #[serde(default)]
    key: String,
}

/// `POST /api/security/tls/certificate {cert, key}` (PEM)
pub async fn tls_certificate(State(s): State<Shared>, CurrentUser(me): CurrentUser, Json(b): Json<Certificate>) -> ApiResult<Json<Value>> {
    require(me.is_admin())?;
    let dir = s.data_dir.clone();
    blocking(move || tls::install_custom(&dir, &b.cert, &b.key)).await?.map_err(bad)?;
    tracing::info!("{} installed an https certificate", me.username);
    let v = tls_json(&s).await?;
    s.changed_for(Audience::Admins, "tls");
    if s.config.load().security.https.mode == HttpsMode::Custom {
        s.restart.notify_one();
    }
    Ok(Json(v))
}

/// `POST /api/security/tls/self-signed`: makes a new self-signed certificate.
pub async fn tls_self_signed(State(s): State<Shared>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Value>> {
    require(me.is_admin())?;
    let dir = s.data_dir.clone();
    blocking(move || tls::generate_self_signed(&dir)).await?.map_err(bad)?;
    let v = tls_json(&s).await?;
    s.changed_for(Audience::Admins, "tls");
    if s.config.load().security.https.mode == HttpsMode::SelfSigned {
        s.restart.notify_one();
    }
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct SsoTest {
    #[serde(default)]
    issuer: String,
}

/// `POST /api/security/sso/test {issuer}` → `{issuer, authorizationEndpoint, tokenEndpoint, keys}`
pub async fn sso_test(State(s): State<Shared>, CurrentUser(me): CurrentUser, Json(b): Json<SsoTest>) -> ApiResult<Json<Value>> {
    require(me.is_admin())?;
    let sso = s.sso.clone();
    let v = blocking(move || sso.test(&b.issuer)).await?.map_err(|e| status(502, e))?;
    Ok(Json(v))
}
