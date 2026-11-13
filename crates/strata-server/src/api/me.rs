//! The signed-in user's own settings: language, theme, display preferences, shortcuts.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::{bad, blocking, ApiResult, CurrentUser};
use crate::auth::PublicUser;
use crate::prefs;
use crate::state::Shared;

#[derive(Deserialize)]
pub struct Lang {
    #[serde(default)]
    lang: String,
}

/// `PUT /api/me/lang {lang}`
pub async fn lang(State(s): State<Shared>, CurrentUser(me): CurrentUser, Json(b): Json<Lang>) -> ApiResult<Json<PublicUser>> {
    let auth = s.auth.clone();
    let user = blocking(move || auth.set_lang(&me.id, &b.lang)).await?.map_err(<(StatusCode, String)>::from)?;
    Ok(Json(user.public()))
}

#[derive(Deserialize)]
pub struct Theme {
    #[serde(default)]
    theme: String,
}

/// `PUT /api/me/theme {theme}`
pub async fn theme(State(s): State<Shared>, CurrentUser(me): CurrentUser, Json(b): Json<Theme>) -> ApiResult<Json<PublicUser>> {
    let auth = s.auth.clone();
    let user = blocking(move || auth.set_theme(&me.id, &b.theme)).await?.map_err(<(StatusCode, String)>::from)?;
    Ok(Json(user.public()))
}

/// `GET /api/me/display`
pub async fn display(State(s): State<Shared>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Value>> {
    let dir = s.data_dir.clone();
    let m = blocking(move || prefs::display(&dir, &me.id)).await?;
    Ok(Json(Value::Object(m)))
}

/// `PATCH /api/me/display {key: value}` (`null` removes)
pub async fn patch_display(State(s): State<Shared>, CurrentUser(me): CurrentUser, Json(patch): Json<Map<String, Value>>) -> ApiResult<StatusCode> {
    let dir = s.data_dir.clone();
    blocking(move || prefs::patch_display(&dir, &me.id, patch)).await?.map_err(bad)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/me/shortcuts` → `{bindings: [...] | null}`
pub async fn shortcuts(State(s): State<Shared>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Value>> {
    let dir = s.data_dir.clone();
    let b = blocking(move || prefs::shortcuts(&dir, &me.id)).await?;
    Ok(Json(json!({ "bindings": b })))
}

#[derive(Deserialize)]
pub struct Shortcuts {
    #[serde(default)]
    bindings: Option<Value>,
}

/// `PUT /api/me/shortcuts {bindings: [{action, keys}] | null}` (`null` = the defaults)
pub async fn set_shortcuts(State(s): State<Shared>, CurrentUser(me): CurrentUser, Json(b): Json<Shortcuts>) -> ApiResult<Json<Value>> {
    let dir = s.data_dir.clone();
    let bindings = b.bindings.filter(|v| !v.is_null());
    let echo = bindings.clone();
    blocking(move || prefs::set_shortcuts(&dir, &me.id, bindings)).await?.map_err(bad)?;
    Ok(Json(json!({ "bindings": echo })))
}
