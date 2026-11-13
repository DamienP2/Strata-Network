//! Server settings (`config.json`).

use std::net::SocketAddr;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::auth::Role;
use crate::paths;

pub const BRAND: &str = "Strata";
pub const PRODUCT: &str = "Strata Network";
/// What a secret looks like when it is sent to the web app.
pub const MASK: &str = "••••••••";

fn default_name() -> String {
    PRODUCT.to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default)]
    pub bind: Option<SocketAddr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_origins: Vec<String>,
    /// IANA time zone ("Europe/Paris"); empty = the machine's.
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub security: Security,
    // ---- Settings of the network features go here (each with #[serde(default)]). ----
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig { name: default_name(), bind: None, trusted_origins: Vec::new(), timezone: String::new(), security: Security::default() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Security {
    #[serde(default)]
    pub https: HttpsConfig,
    #[serde(default)]
    pub sso: SsoConfig,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HttpsMode {
    #[default]
    Off,
    SelfSigned,
    Custom,
}

/// What the plain HTTP port does when HTTPS is on.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HttpMode {
    #[default]
    Redirect,
    HttpsOnly,
    Open,
}

fn default_https_port() -> u16 {
    8443
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HttpsConfig {
    #[serde(default)]
    pub mode: HttpsMode,
    #[serde(default = "default_https_port")]
    pub port: u16,
    #[serde(default)]
    pub http: HttpMode,
}

impl Default for HttpsConfig {
    fn default() -> Self {
        HttpsConfig { mode: HttpsMode::Off, port: default_https_port(), http: HttpMode::Redirect }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    #[default]
    Entra,
    Okta,
    Keycloak,
    Google,
    Other,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LocalAccounts {
    None,
    #[default]
    AdminOnly,
    All,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroupRole {
    #[serde(default)]
    pub group: String,
    pub role: Role,
}

fn default_groups_claim() -> String {
    "groups".into()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SsoConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub provider: Provider,
    #[serde(default)]
    pub issuer: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    #[serde(default = "default_groups_claim")]
    pub groups_claim: String,
    #[serde(default)]
    pub groups: Vec<GroupRole>,
    #[serde(default)]
    pub default_role: Option<Role>,
    #[serde(default)]
    pub local_accounts: LocalAccounts,
}

impl Default for SsoConfig {
    fn default() -> Self {
        SsoConfig {
            enabled: false,
            provider: Provider::Entra,
            issuer: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            groups_claim: default_groups_claim(),
            groups: Vec::new(),
            default_role: None,
            local_accounts: LocalAccounts::AdminOnly,
        }
    }
}

impl SsoConfig {
    /// On when enabled and the issuer and client id are filled.
    pub fn active(&self) -> bool {
        self.enabled && !self.issuer.is_empty() && !self.client_id.is_empty()
    }

    /// The name on the sign-in button.
    pub fn button_name(&self) -> &'static str {
        match self.provider {
            Provider::Entra => "Microsoft",
            Provider::Okta => "Okta",
            Provider::Keycloak => "Keycloak",
            Provider::Google => "Google",
            Provider::Other => "SSO",
        }
    }

    /// Whether a local (password) sign-in is allowed for this role while SSO is on.
    pub fn allows_local(&self, role: Role) -> bool {
        if !self.active() {
            return true;
        }
        match self.local_accounts {
            LocalAccounts::All => true,
            LocalAccounts::AdminOnly => role == Role::Admin,
            LocalAccounts::None => false,
        }
    }

    pub fn local_accounts_str(&self) -> &'static str {
        if !self.active() {
            return "all";
        }
        match self.local_accounts {
            LocalAccounts::None => "none",
            LocalAccounts::AdminOnly => "adminOnly",
            LocalAccounts::All => "all",
        }
    }
}

impl ServerConfig {
    fn path(data_dir: &Path) -> std::path::PathBuf {
        data_dir.join("config.json")
    }

    /// Reads `config.json`: defaults if it is missing or invalid, migrated and normalized, and
    /// saved again when that changed it.
    pub fn load(data_dir: &Path) -> ServerConfig {
        let path = Self::path(data_dir);
        let raw: Option<serde_json::Value> = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::error!("config.json is not valid JSON ({e}): using the default settings");
                    None
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                tracing::error!("cannot read config.json ({e}): using the default settings");
                None
            }
        };
        let existed = raw.is_some();
        let mut raw = raw.unwrap_or_else(|| serde_json::json!({}));
        let migrated = migrate(&mut raw);
        let mut cfg: ServerConfig = match serde_json::from_value(raw.clone()) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("config.json has an invalid value ({e}): using the default settings");
                return ServerConfig::default();
            }
        };
        cfg.normalize();
        let changed = migrated || serde_json::to_value(&cfg).ok().as_ref() != Some(&raw);
        if changed || !existed {
            if let Err(e) = cfg.save(data_dir) {
                tracing::error!("cannot write config.json: {e}");
            }
        }
        cfg
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        paths::write_json(&Self::path(data_dir), self, true)
    }

    /// Trims and tidies the values (never fails).
    pub fn normalize(&mut self) {
        self.name = self.name.trim().to_string();
        self.timezone = self.timezone.trim().to_string();
        let mut origins: Vec<String> = Vec::new();
        for o in &self.trusted_origins {
            let o = o.trim().trim_end_matches('/').to_ascii_lowercase();
            if !o.is_empty() && !origins.contains(&o) {
                origins.push(o);
            }
        }
        self.trusted_origins = origins;
        if self.security.https.port == 0 {
            self.security.https.port = default_https_port();
        }
        let sso = &mut self.security.sso;
        sso.issuer = sso.issuer.trim().trim_end_matches('/').to_string();
        sso.client_id = sso.client_id.trim().to_string();
        sso.client_secret = sso.client_secret.trim().to_string();
        sso.groups_claim = sso.groups_claim.trim().to_string();
        if sso.groups_claim.is_empty() {
            sso.groups_claim = default_groups_claim();
        }
        for g in &mut sso.groups {
            g.group = g.group.trim().to_string();
        }
        sso.groups.retain(|g| !g.group.is_empty());
    }

    /// Checks the values a person typed.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("the server name cannot be empty".into());
        }
        if self.name.chars().count() > 64 {
            return Err("the server name is too long (64 characters at most)".into());
        }
        if !self.timezone.is_empty() && jiff::tz::TimeZone::get(&self.timezone).is_err() {
            return Err(format!("unknown time zone: {}", self.timezone));
        }
        for o in &self.trusted_origins {
            if !(o.starts_with("http://") || o.starts_with("https://")) || o.len() < 9 {
                return Err(format!("a trusted origin is written http(s)://host[:port]: {o}"));
            }
        }
        let https = &self.security.https;
        if https.mode != HttpsMode::Off {
            if let Some(b) = self.bind {
                if b.port() == https.port {
                    return Err("the https port must differ from the http port".into());
                }
            }
        }
        let sso = &self.security.sso;
        if sso.enabled {
            if sso.issuer.is_empty() {
                return Err("the sso issuer is missing".into());
            }
            if !(sso.issuer.starts_with("https://") || sso.issuer.starts_with("http://localhost")) {
                return Err("the sso issuer must start with https://".into());
            }
            if sso.client_id.is_empty() {
                return Err("the sso client id is missing".into());
            }
            if sso.groups_claim.is_empty() {
                return Err("the groups claim cannot be empty".into());
            }
        }
        Ok(())
    }

    /// A copy safe to send to the web app (secret masked).
    pub fn masked(&self) -> ServerConfig {
        let mut c = self.clone();
        if !c.security.sso.client_secret.is_empty() {
            c.security.sso.client_secret = MASK.to_string();
        }
        c
    }

    /// The configured time zone, else the machine's, else UTC.
    pub fn timezone_name(&self) -> String {
        if !self.timezone.is_empty() {
            return self.timezone.clone();
        }
        jiff::tz::TimeZone::system().iana_name().unwrap_or("UTC").to_string()
    }
}

/// Upgrades older settings files in place; true when something changed.
fn migrate(raw: &mut serde_json::Value) -> bool {
    let Some(obj) = raw.as_object_mut() else {
        *raw = serde_json::json!({});
        return true;
    };
    // Older files may hold "bind": "" for "not set".
    if obj.get("bind").and_then(|b| b.as_str()) == Some("") {
        obj.insert("bind".into(), serde_json::Value::Null);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_normalize() {
        let mut c: ServerConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c.name, PRODUCT);
        assert_eq!(c.security.https.port, 8443);
        assert_eq!(c.security.sso.groups_claim, "groups");
        assert_eq!(c.security.sso.local_accounts, LocalAccounts::AdminOnly);
        c.name = "  Lab  ".into();
        c.trusted_origins = vec![" https://A.example/ ".into(), "https://a.example".into(), "".into()];
        c.security.sso.issuer = "https://login.example/tenant/v2.0/".into();
        c.security.sso.groups = vec![GroupRole { group: " ".into(), role: Role::Admin }, GroupRole { group: " Ops ".into(), role: Role::Operator }];
        c.security.sso.groups_claim = " ".into();
        c.normalize();
        assert_eq!(c.name, "Lab");
        assert_eq!(c.trusted_origins, vec!["https://a.example".to_string()]);
        assert_eq!(c.security.sso.issuer, "https://login.example/tenant/v2.0");
        assert_eq!(c.security.sso.groups.len(), 1);
        assert_eq!(c.security.sso.groups[0].group, "Ops");
        assert_eq!(c.security.sso.groups_claim, "groups");
        // trustedOrigins omitted when empty
        let v = serde_json::to_value(ServerConfig::default()).unwrap();
        assert!(v.get("trustedOrigins").is_none());
        assert_eq!(v["security"]["https"]["mode"], "off");
        assert_eq!(v["security"]["sso"]["localAccounts"], "adminOnly");
    }

    #[test]
    fn validate_rules() {
        let mut c = ServerConfig::default();
        assert!(c.validate().is_ok());
        c.name = String::new();
        assert_eq!(c.validate().unwrap_err(), "the server name cannot be empty");
        c.name = "x".into();
        c.timezone = "Mars/Olympus".into();
        assert!(c.validate().is_err());
        c.timezone = "Europe/Paris".into();
        assert!(c.validate().is_ok());
        c.trusted_origins = vec!["example.com".into()];
        assert!(c.validate().is_err());
        c.trusted_origins.clear();
        c.security.sso.enabled = true;
        assert!(c.validate().is_err());
        c.security.sso.issuer = "https://id.example".into();
        c.security.sso.client_id = "abc".into();
        assert!(c.validate().is_ok());
        c.bind = Some("0.0.0.0:8443".parse().unwrap());
        c.security.https.mode = HttpsMode::SelfSigned;
        assert!(c.validate().is_err());
    }

    #[test]
    fn sso_local_accounts() {
        let mut s = SsoConfig { enabled: true, issuer: "https://i".into(), client_id: "c".into(), ..Default::default() };
        assert!(s.active());
        assert!(s.allows_local(Role::Admin));
        assert!(!s.allows_local(Role::Viewer));
        s.local_accounts = LocalAccounts::None;
        assert!(!s.allows_local(Role::Admin));
        s.enabled = false;
        assert!(s.allows_local(Role::Viewer));
        assert_eq!(s.local_accounts_str(), "all");
    }

    #[test]
    fn load_writes_and_masks() {
        let dir = std::env::temp_dir().join(format!("sn-cfg-{}", paths::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), r#"{"name":" Lab ","bind":""}"#).unwrap();
        let c = ServerConfig::load(&dir);
        assert_eq!(c.name, "Lab");
        assert_eq!(c.bind, None);
        let again: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("config.json")).unwrap()).unwrap();
        assert_eq!(again["name"], "Lab");
        let mut c = c;
        c.security.sso.client_secret = "s3cret".into();
        assert_eq!(c.masked().security.sso.client_secret, MASK);
        let _ = std::fs::remove_dir_all(dir);
    }
}
