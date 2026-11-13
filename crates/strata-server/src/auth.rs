//! User accounts (`users.json`), passwords and in-memory sessions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Semaphore};

use crate::paths;

/// Sessions last 14 days, prolonged by each request.
pub const SESSION_TTL: Duration = Duration::from_secs(14 * 24 * 3600);
pub const MIN_PASSWORD: usize = 8;
pub const MAX_USERNAME: usize = 64;
pub const LANGS: [&str; 2] = ["en", "fr"];
pub const THEMES: [&str; 3] = ["dark", "light", "system"];

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Admin,
    /// Runs scans and edits the network inventory.
    Operator,
    /// Read only.
    Viewer,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Admin, Role::Operator, Role::Viewer];

    /// Higher is more powerful: Admin > Operator > Viewer.
    pub fn rank(self) -> u8 {
        match self {
            Role::Admin => 3,
            Role::Operator => 2,
            Role::Viewer => 1,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Operator => "operator",
            Role::Viewer => "viewer",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.as_str().eq_ignore_ascii_case(s.trim()))
    }

    pub fn is_admin(self) -> bool {
        self == Role::Admin
    }

    pub fn can_edit(self) -> bool {
        matches!(self, Role::Admin | Role::Operator)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    pub role: Role,
    #[serde(default)]
    pub disabled: bool,
    /// "en" | "fr" ; empty = the browser's.
    #[serde(default)]
    pub lang: String,
    /// "dark" | "light" | "system" ; empty = dark.
    #[serde(default)]
    pub theme: String,
    /// "issuer|subject" of a directory (SSO) account.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sso: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub role_fixed: bool,
    #[serde(default)]
    password_hash: String,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role.is_admin()
    }
    pub fn can_edit(&self) -> bool {
        self.role.can_edit()
    }
    pub fn public(&self) -> PublicUser {
        PublicUser {
            id: self.id.clone(),
            username: self.username.clone(),
            display_name: if self.display_name.is_empty() { self.username.clone() } else { self.display_name.clone() },
            role: self.role,
            disabled: self.disabled,
            lang: self.lang.clone(),
            theme: self.theme.clone(),
            sso: !self.sso.is_empty(),
            role_fixed: self.role_fixed,
        }
    }
    fn active_admin(&self) -> bool {
        self.role == Role::Admin && !self.disabled
    }
}

/// What the web app sees of a user (never the password hash).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PublicUser {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub role: Role,
    pub disabled: bool,
    pub lang: String,
    pub theme: String,
    pub sso: bool,
    pub role_fixed: bool,
}

/// An error of the account functions, turned into an HTTP answer by the API.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("{0}")]
    Bad(String),
    #[error("no such user")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl From<AuthError> for (StatusCode, String) {
    fn from(e: AuthError) -> Self {
        let code = match e {
            AuthError::Bad(_) => StatusCode::BAD_REQUEST,
            AuthError::NotFound => StatusCode::NOT_FOUND,
            AuthError::Conflict(_) => StatusCode::CONFLICT,
            AuthError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (code, e.to_string())
    }
}

fn bad(s: &str) -> AuthError {
    AuthError::Bad(s.to_string())
}

/// Checks a user name: letters, digits, `. - _ @`, 64 characters at most.
pub fn check_username(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("the user name cannot be empty".into());
    }
    if name.chars().count() > MAX_USERNAME {
        return Err("the user name is too long (64 characters at most)".into());
    }
    if !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | '@')) {
        return Err("a user name only has letters, digits and . - _ @".into());
    }
    Ok(())
}

pub fn check_password(pw: &str) -> Result<(), String> {
    if pw.chars().count() < MIN_PASSWORD {
        return Err("the password needs at least 8 characters".into());
    }
    if pw.len() > 1024 {
        return Err("the password is too long".into());
    }
    Ok(())
}

pub fn hash_password(pw: &str) -> String {
    let salt = SaltString::encode_b64(uuid::Uuid::new_v4().as_bytes()).expect("16-byte salt");
    Argon2::default().hash_password(pw.as_bytes(), &salt).expect("argon2 hash").to_string()
}

fn verify_hash(hash: &str, pw: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default().verify_password(pw.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

/// A hash to verify against when the user is unknown, so the answer takes the same time.
fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| hash_password("not a real password, only for timing"))
}

/// A random password nobody knows (directory-only accounts).
pub fn random_password() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// A new session token: two UUID v4 (64 hex characters).
pub fn new_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// The fields of a new or changed account.
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct UserInput {
    pub username: String,
    pub display_name: String,
    pub role: Option<Role>,
    pub disabled: bool,
    /// Empty = unchanged (edit), or random when SSO is on (creation).
    pub password: String,
    pub role_fixed: bool,
}

struct Session {
    user_id: String,
    expires: Instant,
}

struct UsersFile {
    list: Vec<User>,
    mtime: Option<SystemTime>,
}

pub struct Auth {
    path: PathBuf,
    data_dir: PathBuf,
    users: RwLock<UsersFile>,
    /// Serializes changes to `users.json`.
    write: Mutex<()>,
    sessions: Mutex<HashMap<String, Session>>,
    /// Tokens of sessions closed because the same user signed in elsewhere.
    pub ended: broadcast::Sender<String>,
    /// Ids of users whose account changed (role, disabled, password, deleted): their sockets close.
    pub changed: broadcast::Sender<String>,
    /// At most 4 Argon2 verifications at once.
    verify_slots: Semaphore,
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn read_users(path: &Path) -> Vec<User> {
    match paths::read_json::<Vec<User>>(path) {
        Ok(list) => list.unwrap_or_default(),
        Err(e) => {
            tracing::error!("cannot read {}: {e}", path.display());
            Vec::new()
        }
    }
}

impl Auth {
    pub fn load(data_dir: &Path) -> Auth {
        let path = data_dir.join("users.json");
        let list = read_users(&path);
        let file_mtime = mtime(&path);
        Auth {
            path,
            data_dir: data_dir.to_path_buf(),
            users: RwLock::new(UsersFile { list, mtime: file_mtime }),
            write: Mutex::new(()),
            sessions: Mutex::new(HashMap::new()),
            ended: broadcast::channel(64).0,
            changed: broadcast::channel(64).0,
            verify_slots: Semaphore::new(4),
        }
    }

    /// Re-reads `users.json` when its modification time changed (the CLI may act while the
    /// server runs).
    fn refresh(&self) {
        let now = mtime(&self.path);
        if self.users.read().unwrap().mtime == now {
            return;
        }
        let list = read_users(&self.path);
        let mut u = self.users.write().unwrap();
        let old: HashMap<String, User> = u.list.iter().map(|x| (x.id.clone(), x.clone())).collect();
        u.list = list;
        u.mtime = now;
        // Close the sessions of users the CLI changed.
        let gone: Vec<String> = old
            .values()
            .filter(|o| match u.list.iter().find(|n| n.id == o.id) {
                None => true,
                Some(n) => n.disabled != o.disabled || n.password_hash != o.password_hash || n.role != o.role,
            })
            .map(|o| o.id.clone())
            .collect();
        drop(u);
        for id in gone {
            self.close_user_sessions(&id, None);
            let _ = self.changed.send(id);
        }
    }

    pub fn needs_setup(&self) -> bool {
        self.refresh();
        self.users.read().unwrap().list.is_empty()
    }

    pub fn list(&self) -> Vec<User> {
        self.refresh();
        self.users.read().unwrap().list.clone()
    }

    pub fn get(&self, id: &str) -> Option<User> {
        self.refresh();
        self.users.read().unwrap().list.iter().find(|u| u.id == id).cloned()
    }

    pub fn find_by_name(&self, name: &str) -> Option<User> {
        self.refresh();
        self.users.read().unwrap().list.iter().find(|u| u.username.eq_ignore_ascii_case(name.trim())).cloned()
    }

    pub fn find_by_sso(&self, key: &str) -> Option<User> {
        self.refresh();
        self.users.read().unwrap().list.iter().find(|u| !u.sso.is_empty() && u.sso == key).cloned()
    }

    /// Runs `f` on the list under the write lock and saves it if `f` succeeds.
    fn mutate<T>(&self, f: impl FnOnce(&mut Vec<User>) -> Result<T, AuthError>) -> Result<T, AuthError> {
        let _w = self.write.lock().unwrap();
        self.refresh();
        let mut list = self.users.read().unwrap().list.clone();
        let out = f(&mut list)?;
        paths::write_json(&self.path, &list, true)?;
        paths::give_back(&self.path, &self.data_dir);
        let mut u = self.users.write().unwrap();
        u.list = list;
        u.mtime = mtime(&self.path);
        Ok(out)
    }

    /// Creates an account. `sso_only` = a random password is set when none is given.
    pub fn create(&self, input: UserInput, sso_only: bool) -> Result<User, AuthError> {
        let username = input.username.trim().to_string();
        check_username(&username).map_err(AuthError::Bad)?;
        let password = if input.password.is_empty() && sso_only { random_password() } else { input.password.clone() };
        check_password(&password).map_err(AuthError::Bad)?;
        let hash = hash_password(&password);
        let role = input.role.unwrap_or(Role::Viewer);
        self.mutate(|list| {
            if list.iter().any(|u| u.username.eq_ignore_ascii_case(&username)) {
                return Err(AuthError::Conflict("this user name is already taken".into()));
            }
            let user = User {
                id: paths::new_id(),
                display_name: input.display_name.trim().to_string(),
                username: username.clone(),
                role,
                disabled: input.disabled,
                lang: String::new(),
                theme: String::new(),
                sso: String::new(),
                role_fixed: input.role_fixed,
                password_hash: hash,
            };
            if list.is_empty() && !user.active_admin() {
                return Err(bad("the first account must be an active admin"));
            }
            list.push(user.clone());
            Ok(user)
        })
    }

    /// Creates an account for a directory (SSO) identity, without password.
    pub fn create_sso(&self, username: &str, display_name: &str, role: Role, key: &str) -> Result<User, AuthError> {
        self.mutate(|list| {
            let base: String = username.chars().filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | '@')).take(MAX_USERNAME - 4).collect();
            let base = if base.is_empty() { "user".to_string() } else { base };
            let mut name = base.clone();
            let mut n = 2;
            while list.iter().any(|u| u.username.eq_ignore_ascii_case(&name)) {
                name = format!("{base}-{n}");
                n += 1;
            }
            let user = User {
                id: paths::new_id(),
                username: name,
                display_name: display_name.trim().to_string(),
                role,
                disabled: false,
                lang: String::new(),
                theme: String::new(),
                sso: key.to_string(),
                role_fixed: false,
                password_hash: String::new(),
            };
            list.push(user.clone());
            Ok(user)
        })
    }

    /// Links an existing account to a directory identity and/or updates its role from the
    /// directory groups (never removing the last active admin).
    pub fn sso_sync(&self, id: &str, key: &str, role: Role) -> Result<User, AuthError> {
        let (user, role_changed) = self.mutate(|list| {
            let others_admin = list.iter().any(|u| u.id != id && u.active_admin());
            let u = list.iter_mut().find(|u| u.id == id).ok_or(AuthError::NotFound)?;
            u.sso = key.to_string();
            let old = u.role;
            if !u.role_fixed && (role == Role::Admin || !u.active_admin() || others_admin) {
                u.role = role;
            }
            Ok((u.clone(), old != u.role))
        })?;
        if role_changed {
            let _ = self.changed.send(user.id.clone());
        }
        Ok(user)
    }

    /// Changes an account. `keep` = a session token that stays open when the password changes
    /// (the admin changing their own password).
    pub fn update(&self, id: &str, input: UserInput, keep: Option<&str>) -> Result<User, AuthError> {
        let username = input.username.trim().to_string();
        check_username(&username).map_err(AuthError::Bad)?;
        let hash = if input.password.is_empty() {
            None
        } else {
            check_password(&input.password).map_err(AuthError::Bad)?;
            Some(hash_password(&input.password))
        };
        let (user, close, changed) = self.mutate(|list| {
            if list.iter().any(|u| u.id != id && u.username.eq_ignore_ascii_case(&username)) {
                return Err(AuthError::Conflict("this user name is already taken".into()));
            }
            let u = list.iter_mut().find(|u| u.id == id).ok_or(AuthError::NotFound)?;
            let before = u.clone();
            u.username = username.clone();
            u.display_name = input.display_name.trim().to_string();
            if let Some(r) = input.role {
                u.role = r;
            }
            u.disabled = input.disabled;
            u.role_fixed = input.role_fixed;
            if let Some(h) = hash.clone() {
                if !u.sso.is_empty() {
                    return Err(bad("this account signs in with the directory (sso)"));
                }
                u.password_hash = h;
            }
            let after = u.clone();
            if !list.iter().any(User::active_admin) {
                return Err(bad("there must stay at least one active admin"));
            }
            let close = after.disabled || hash.is_some();
            let changed = close || before.role != after.role;
            Ok((after, close, changed))
        })?;
        if close {
            self.close_user_sessions(&user.id, keep);
        }
        if changed {
            let _ = self.changed.send(user.id.clone());
        }
        Ok(user)
    }

    pub fn delete(&self, id: &str, me: &str) -> Result<(), AuthError> {
        if id == me {
            return Err(bad("you cannot delete your own account"));
        }
        self.mutate(|list| {
            let before = list.len();
            list.retain(|u| u.id != id);
            if list.len() == before {
                return Err(AuthError::NotFound);
            }
            if !list.iter().any(User::active_admin) {
                return Err(bad("there must stay at least one active admin"));
            }
            Ok(())
        })?;
        self.close_user_sessions(id, None);
        let _ = self.changed.send(id.to_string());
        Ok(())
    }

    /// Changes the password of a named account (CLI).
    pub fn set_password(&self, name: &str, password: &str) -> Result<(), AuthError> {
        check_password(password).map_err(AuthError::Bad)?;
        let hash = hash_password(password);
        let id = self.mutate(|list| {
            let u = list.iter_mut().find(|u| u.username.eq_ignore_ascii_case(name)).ok_or(AuthError::NotFound)?;
            u.password_hash = hash;
            Ok(u.id.clone())
        })?;
        self.close_user_sessions(&id, None);
        Ok(())
    }

    pub fn set_lang(&self, id: &str, lang: &str) -> Result<User, AuthError> {
        if !LANGS.contains(&lang) {
            return Err(bad("unknown language"));
        }
        self.mutate(|list| {
            let u = list.iter_mut().find(|u| u.id == id).ok_or(AuthError::NotFound)?;
            u.lang = lang.to_string();
            Ok(u.clone())
        })
    }

    pub fn set_theme(&self, id: &str, theme: &str) -> Result<User, AuthError> {
        if !THEMES.contains(&theme) {
            return Err(bad("unknown theme"));
        }
        self.mutate(|list| {
            let u = list.iter_mut().find(|u| u.id == id).ok_or(AuthError::NotFound)?;
            u.theme = theme.to_string();
            Ok(u.clone())
        })
    }

    /// Checks a user name and password (blocking: Argon2). Unknown names are checked against a
    /// dummy hash so the time taken reveals nothing. Disabled accounts never pass.
    pub fn verify_blocking(&self, username: &str, password: &str) -> Option<User> {
        let user = self.find_by_name(username);
        let hash = match &user {
            Some(u) if !u.password_hash.is_empty() => u.password_hash.clone(),
            _ => dummy_hash().to_string(),
        };
        let ok = verify_hash(&hash, password);
        match user {
            Some(u) if ok && !u.disabled && !u.password_hash.is_empty() => Some(u),
            _ => None,
        }
    }

    /// Same as [`verify_blocking`], at most 4 at once, off the async threads.
    pub async fn verify(self: &std::sync::Arc<Self>, username: String, password: String) -> Option<User> {
        let _slot = self.verify_slots.acquire().await.ok()?;
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.verify_blocking(&username, &password)).await.ok().flatten()
    }

    /// Opens a session and closes every other session of the same user.
    pub fn open_session(&self, user_id: &str) -> String {
        let token = new_token();
        let mut s = self.sessions.lock().unwrap();
        let now = Instant::now();
        s.retain(|_, v| v.expires > now);
        let others: Vec<String> = s.iter().filter(|(_, v)| v.user_id == user_id).map(|(k, _)| k.clone()).collect();
        for t in &others {
            s.remove(t);
        }
        s.insert(token.clone(), Session { user_id: user_id.to_string(), expires: now + SESSION_TTL });
        drop(s);
        for t in others {
            let _ = self.ended.send(t);
        }
        token
    }

    /// The user of a session (prolongs it), or `None` if it expired, or the account is gone or
    /// disabled.
    pub fn session_user(&self, token: &str) -> Option<User> {
        if token.is_empty() {
            return None;
        }
        let user_id = {
            let mut s = self.sessions.lock().unwrap();
            let now = Instant::now();
            let sess = s.get_mut(token)?;
            if sess.expires <= now {
                s.remove(token);
                return None;
            }
            sess.expires = now + SESSION_TTL;
            sess.user_id.clone()
        };
        match self.get(&user_id) {
            Some(u) if !u.disabled => Some(u),
            _ => {
                self.sessions.lock().unwrap().remove(token);
                None
            }
        }
    }

    pub fn close_session(&self, token: &str) {
        self.sessions.lock().unwrap().remove(token);
    }

    /// Closes every session of a user except `keep`.
    pub fn close_user_sessions(&self, user_id: &str, keep: Option<&str>) {
        self.sessions.lock().unwrap().retain(|k, v| v.user_id != user_id || Some(k.as_str()) == keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_auth() -> (Auth, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sn-auth-{}", paths::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        (Auth::load(&dir), dir)
    }

    fn input(name: &str, role: Role) -> UserInput {
        UserInput { username: name.into(), password: "password1".into(), role: Some(role), ..Default::default() }
    }

    #[test]
    fn username_rules() {
        assert!(check_username("admin").is_ok());
        assert!(check_username("jean.dupont@corp.fr").is_ok());
        assert!(check_username("a_b-c").is_ok());
        assert!(check_username("élodie").is_ok());
        assert!(check_username("").is_err());
        assert!(check_username("with space").is_err());
        assert!(check_username("semi;colon").is_err());
        assert!(check_username(&"x".repeat(64)).is_ok());
        assert!(check_username(&"x".repeat(65)).is_err());
    }

    #[test]
    fn password_rules() {
        assert!(check_password("1234567").is_err());
        assert!(check_password("12345678").is_ok());
    }

    #[test]
    fn roles() {
        assert!(Role::Admin.rank() > Role::Operator.rank());
        assert!(Role::Operator.rank() > Role::Viewer.rank());
        assert!(Role::Operator.can_edit() && !Role::Viewer.can_edit() && Role::Admin.can_edit());
        assert_eq!(serde_json::to_string(&Role::Operator).unwrap(), "\"operator\"");
        assert_eq!(Role::parse("Viewer"), Some(Role::Viewer));
    }

    #[test]
    fn accounts_and_sessions() {
        let (auth, dir) = temp_auth();
        assert!(auth.needs_setup());
        assert!(auth.create(input("bob", Role::Viewer), false).is_err(), "first must be admin");
        let admin = auth.create(input("admin", Role::Admin), false).unwrap();
        assert!(matches!(auth.create(input("ADMIN", Role::Viewer), false), Err(AuthError::Conflict(_))));
        let bob = auth.create(input("bob", Role::Viewer), false).unwrap();
        assert!(auth.verify_blocking("Bob", "password1").is_some());
        assert!(auth.verify_blocking("bob", "wrong-password").is_none());
        assert!(auth.verify_blocking("nobody", "password1").is_none());

        // Last admin cannot go.
        let mut demote = input("admin", Role::Viewer);
        demote.password.clear();
        assert!(auth.update(&admin.id, demote, None).is_err());
        assert!(auth.delete(&admin.id, &admin.id).is_err());

        // One session per user.
        let t1 = auth.open_session(&bob.id);
        let mut ended = auth.ended.subscribe();
        let t2 = auth.open_session(&bob.id);
        assert!(auth.session_user(&t1).is_none());
        assert_eq!(auth.session_user(&t2).unwrap().id, bob.id);
        assert_eq!(ended.try_recv().unwrap(), t1);

        // Disabling closes sessions.
        let mut dis = input("bob", Role::Viewer);
        dis.password.clear();
        dis.disabled = true;
        auth.update(&bob.id, dis, None).unwrap();
        assert!(auth.session_user(&t2).is_none());
        assert!(auth.verify_blocking("bob", "password1").is_none());

        // Reload from disk.
        let again = Auth::load(&dir);
        assert_eq!(again.list().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
