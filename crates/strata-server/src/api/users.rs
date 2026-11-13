//! Accounts (admins only).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use super::{blocking, require, ApiResult, Session};
use crate::auth::{PublicUser, UserInput};
use crate::prefs;
use crate::state::{Audience, Shared};

/// `GET /api/users`
pub async fn list(State(s): State<Shared>, me: Session) -> ApiResult<Json<Vec<PublicUser>>> {
    require(me.user.is_admin())?;
    Ok(Json(s.auth.list().iter().map(|u| u.public()).collect()))
}

/// `POST /api/users {username, displayName, role, disabled, password, roleFixed}`
pub async fn create(State(s): State<Shared>, me: Session, Json(input): Json<UserInput>) -> ApiResult<Json<PublicUser>> {
    require(me.user.is_admin())?;
    let sso_only = s.config.load().security.sso.active();
    let auth = s.auth.clone();
    let user = blocking(move || auth.create(input, sso_only)).await?.map_err(<(StatusCode, String)>::from)?;
    tracing::info!("{} created the account \"{}\"", me.user.username, user.username);
    s.changed_for(Audience::Admins, "users");
    Ok(Json(user.public()))
}

/// `PUT /api/users/{id}` (empty password = unchanged)
pub async fn update(State(s): State<Shared>, me: Session, Path(id): Path<String>, Json(input): Json<UserInput>) -> ApiResult<Json<PublicUser>> {
    require(me.user.is_admin())?;
    if id == me.user.id && input.disabled {
        return Err((StatusCode::BAD_REQUEST, "you cannot disable your own account".into()));
    }
    let auth = s.auth.clone();
    let keep = me.token.clone();
    let user = blocking(move || auth.update(&id, input, Some(&keep))).await?.map_err(<(StatusCode, String)>::from)?;
    s.changed_for(Audience::Admins, "users");
    Ok(Json(user.public()))
}

/// `DELETE /api/users/{id}`
pub async fn delete(State(s): State<Shared>, me: Session, Path(id): Path<String>) -> ApiResult<StatusCode> {
    require(me.user.is_admin())?;
    let auth = s.auth.clone();
    let dir = s.data_dir.clone();
    let my_id = me.user.id.clone();
    blocking(move || {
        auth.delete(&id, &my_id)?;
        prefs::forget(&dir, &id);
        Ok::<_, crate::auth::AuthError>(())
    })
    .await?
    .map_err(<(StatusCode, String)>::from)?;
    s.changed_for(Audience::Admins, "users");
    Ok(StatusCode::NO_CONTENT)
}
