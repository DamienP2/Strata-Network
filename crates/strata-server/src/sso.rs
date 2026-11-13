//! Single sign-on with OpenID Connect (authorization code + PKCE), id_token checked with the
//! provider's JWKS keys (RS256, PS256, ES256) using `ring`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::auth::Role;
use crate::config::{GroupRole, SsoConfig};

const DISCOVERY_TTL: Duration = Duration::from_secs(3600);
const PENDING_TTL: Duration = Duration::from_secs(600);
/// Allowed clock difference on `exp`.
const LEEWAY: i64 = 120;

#[derive(Deserialize, Clone, Debug)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Jwk {
    #[serde(default)]
    pub kid: String,
    #[serde(default)]
    pub kty: String,
    #[serde(default)]
    pub alg: String,
    #[serde(default)]
    pub n: String,
    #[serde(default)]
    pub e: String,
    #[serde(default)]
    pub crv: String,
    #[serde(default)]
    pub x: String,
    #[serde(default)]
    pub y: String,
}

#[derive(Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

struct Pending {
    issuer: String,
    nonce: String,
    verifier: String,
    redirect_uri: String,
    next: String,
    created: Instant,
}

/// Why a directory sign-in failed; turned into `/?sso_error=<kind>&sso_detail=<detail>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Fail {
    /// "noGroup" | "disabled" | "expired" | "failed"
    pub kind: &'static str,
    pub detail: String,
}

impl Fail {
    pub fn failed(detail: impl Into<String>) -> Fail {
        Fail { kind: "failed", detail: detail.into() }
    }
    pub fn redirect(&self) -> String {
        format!("/?sso_error={}&sso_detail={}", self.kind, enc(&self.detail))
    }
}

/// Who signed in, from the id_token claims.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    /// "issuer|subject"
    pub key: String,
    pub login: String,
    /// Verified e-mail or UPN: may match an account created in advance.
    pub address: Option<String>,
    pub name: String,
    pub groups: Vec<String>,
}

/// The result of a successful callback.
pub struct SignedIn {
    pub identity: Identity,
    pub role: Option<Role>,
    pub next: String,
}

/// Long-lived SSO state (survives hot restarts).
#[derive(Default)]
pub struct Sso {
    discovery: Mutex<Option<(String, Instant, Discovery)>>,
    jwks: Mutex<HashMap<String, Vec<Jwk>>>,
    pending: Mutex<HashMap<String, Pending>>,
}

fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .user_agent(concat!("strata/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

fn get_json(url: &str) -> Result<Value, String> {
    let mut res = agent().get(url).header("Accept", "application/json").call().map_err(|e| format!("cannot reach {url}: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("{url} answered {}", res.status().as_u16()));
    }
    res.body_mut().with_config().limit(4 * 1024 * 1024).read_json::<Value>().map_err(|e| format!("{url} did not send JSON: {e}"))
}

/// Characters kept as they are in a URL query value.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn random_b64(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    SystemRandom::new().fill(&mut buf).expect("system random");
    URL_SAFE_NO_PAD.encode(buf)
}

/// Only a local path: starts with `/`, without `//` or `\`.
pub fn safe_next(next: &str) -> String {
    let n = next.trim();
    if n.starts_with('/') && !n.contains("//") && !n.contains('\\') && !n.chars().any(char::is_control) {
        n.to_string()
    } else {
        "/".to_string()
    }
}

fn norm_group(g: &str) -> String {
    g.trim().trim_start_matches('/').to_lowercase()
}

/// The highest role among the groups listed in the settings, else the default role.
pub fn role_for(groups: &[String], map: &[GroupRole], default: Option<Role>) -> Option<Role> {
    let mine: Vec<String> = groups.iter().map(|g| norm_group(g)).collect();
    map.iter().filter(|m| mine.contains(&norm_group(&m.group))).map(|m| m.role).max_by_key(|r| r.rank()).or(default)
}

fn claim<'a>(claims: &'a Value, path: &str) -> Option<&'a Value> {
    if let Some(v) = claims.get(path) {
        return Some(v);
    }
    path.split('.').try_fold(claims, |v, k| v.get(k))
}

fn str_claim(claims: &Value, name: &str) -> Option<String> {
    claims.get(name).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Who signed in: key `issuer|sub`, login = email (unless `email_verified` is false), else
/// `preferred_username`, else `upn`; guaranteed address = verified e-mail or UPN.
pub fn identity(claims: &Value, groups_claim: &str) -> Result<Identity, String> {
    let iss = str_claim(claims, "iss").ok_or("the id_token has no issuer")?;
    let sub = str_claim(claims, "sub").ok_or("the id_token has no subject")?;
    let email = str_claim(claims, "email");
    let verified = claims.get("email_verified").and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true")));
    let upn = str_claim(claims, "upn");
    let login = email
        .clone()
        .filter(|_| verified != Some(false))
        .or_else(|| str_claim(claims, "preferred_username"))
        .or_else(|| upn.clone())
        .ok_or("the directory sent no e-mail or user name")?;
    let address = email.filter(|_| verified == Some(true)).or(upn);
    let name = str_claim(claims, "name").unwrap_or_default();
    let groups = match claim(claims, groups_claim) {
        Some(Value::Array(a)) => a.iter().filter_map(|g| g.as_str().map(str::to_string)).collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    };
    Ok(Identity { key: format!("{iss}|{sub}"), login, address, name, groups })
}

/// Checks a JWS signature with one key.
pub fn verify_signature(alg: &str, jwk: &Jwk, signing_input: &[u8], sig: &[u8]) -> bool {
    let d = |s: &str| URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok();
    match alg {
        "RS256" | "PS256" => {
            if jwk.kty != "RSA" {
                return false;
            }
            let (Some(n), Some(e)) = (d(&jwk.n), d(&jwk.e)) else { return false };
            let params: &signature::RsaParameters = if alg == "RS256" { &signature::RSA_PKCS1_2048_8192_SHA256 } else { &signature::RSA_PSS_2048_8192_SHA256 };
            signature::RsaPublicKeyComponents { n: &n, e: &e }.verify(params, signing_input, sig).is_ok()
        }
        "ES256" => {
            if jwk.kty != "EC" || (!jwk.crv.is_empty() && jwk.crv != "P-256") {
                return false;
            }
            let (Some(x), Some(y)) = (d(&jwk.x), d(&jwk.y)) else { return false };
            let mut point = vec![4u8];
            point.extend_from_slice(&x);
            point.extend_from_slice(&y);
            signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, point).verify(signing_input, sig).is_ok()
        }
        _ => false,
    }
}

/// The parts of an id_token: header, claims, signing input, signature.
fn split_jwt(token: &str) -> Result<(Value, Value, String, Vec<u8>), String> {
    let mut parts = token.split('.');
    let (Some(h), Some(p), Some(s), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err("the id_token is not a JWT".into());
    };
    let header: Value = URL_SAFE_NO_PAD.decode(h).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or("bad id_token header")?;
    let claims: Value = URL_SAFE_NO_PAD.decode(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or("bad id_token claims")?;
    let sig = URL_SAFE_NO_PAD.decode(s).map_err(|_| "bad id_token signature")?;
    Ok((header, claims, format!("{h}.{p}"), sig))
}

/// Checks the claims of an id_token: issuer, audience, expiry (+120 s), nonce.
pub fn check_claims(claims: &Value, issuer: &str, client_id: &str, nonce: &str, now: i64) -> Result<(), String> {
    let iss = claims.get("iss").and_then(Value::as_str).unwrap_or("");
    if iss.trim_end_matches('/') != issuer.trim_end_matches('/') {
        return Err(format!("the id_token comes from another issuer ({iss})"));
    }
    let aud_ok = match claims.get("aud") {
        Some(Value::String(a)) => a == client_id,
        Some(Value::Array(a)) => a.iter().any(|x| x.as_str() == Some(client_id)),
        _ => false,
    };
    if !aud_ok {
        return Err("the id_token is for another application".into());
    }
    let exp = claims.get("exp").and_then(Value::as_i64).ok_or("the id_token has no expiry")?;
    if exp + LEEWAY < now {
        return Err("the id_token has expired".into());
    }
    if claims.get("nonce").and_then(Value::as_str) != Some(nonce) {
        return Err("the id_token nonce does not match".into());
    }
    Ok(())
}

impl Sso {
    pub fn new() -> Sso {
        Sso::default()
    }

    /// The provider's endpoints (cached 1 h; blocking).
    pub fn discover(&self, issuer: &str, force: bool) -> Result<Discovery, String> {
        if !force {
            if let Some((iss, at, d)) = self.discovery.lock().unwrap().as_ref() {
                if iss == issuer && at.elapsed() < DISCOVERY_TTL {
                    return Ok(d.clone());
                }
            }
        }
        let url = format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/'));
        let v = get_json(&url)?;
        let d: Discovery = serde_json::from_value(v).map_err(|e| format!("the openid configuration is incomplete: {e}"))?;
        *self.discovery.lock().unwrap() = Some((issuer.to_string(), Instant::now(), d.clone()));
        Ok(d)
    }

    /// The signing keys (blocking).
    pub fn keys(&self, jwks_uri: &str, force: bool) -> Result<Vec<Jwk>, String> {
        if !force {
            if let Some(k) = self.jwks.lock().unwrap().get(jwks_uri) {
                return Ok(k.clone());
            }
        }
        let set: JwkSet = serde_json::from_value(get_json(jwks_uri)?).map_err(|e| format!("bad jwks: {e}"))?;
        self.jwks.lock().unwrap().insert(jwks_uri.to_string(), set.keys.clone());
        Ok(set.keys)
    }

    /// `POST /api/security/sso/test` (blocking).
    pub fn test(&self, issuer: &str) -> Result<Value, String> {
        let issuer = issuer.trim().trim_end_matches('/');
        if issuer.is_empty() {
            return Err("the issuer is empty".into());
        }
        let d = self.discover(issuer, true)?;
        let keys = self.keys(&d.jwks_uri, true)?;
        Ok(serde_json::json!({
            "issuer": d.issuer,
            "authorizationEndpoint": d.authorization_endpoint,
            "tokenEndpoint": d.token_endpoint,
            "keys": keys.len(),
        }))
    }

    /// The URL of the provider's sign-in page (blocking).
    pub fn start(&self, cfg: &SsoConfig, origin: &str, next: &str) -> Result<String, String> {
        if !cfg.active() {
            return Err("sso is off".into());
        }
        let d = self.discover(&cfg.issuer, false)?;
        let state = random_b64(24);
        let nonce = random_b64(24);
        let verifier = random_b64(48);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let redirect_uri = format!("{origin}/api/auth/sso/callback");
        {
            let mut p = self.pending.lock().unwrap();
            p.retain(|_, v| v.created.elapsed() < PENDING_TTL);
            if p.len() > 10_000 {
                return Err("too many sign-ins in progress".into());
            }
            p.insert(
                state.clone(),
                Pending { issuer: cfg.issuer.clone(), nonce: nonce.clone(), verifier, redirect_uri: redirect_uri.clone(), next: safe_next(next), created: Instant::now() },
            );
        }
        let sep = if d.authorization_endpoint.contains('?') { '&' } else { '?' };
        Ok(format!(
            "{}{sep}response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}&code_challenge={}&code_challenge_method=S256",
            d.authorization_endpoint,
            enc(&cfg.client_id),
            enc(&redirect_uri),
            enc("openid profile email"),
            enc(&state),
            enc(&nonce),
            enc(&challenge)
        ))
    }

    /// Finishes a sign-in: exchanges the code and checks the id_token (blocking).
    pub fn callback(&self, cfg: &SsoConfig, params: &HashMap<String, String>) -> Result<SignedIn, Fail> {
        let state = params.get("state").cloned().unwrap_or_default();
        let pending = self.pending.lock().unwrap().remove(&state);
        if let Some(err) = params.get("error") {
            let detail = params.get("error_description").cloned().unwrap_or_else(|| err.clone());
            return Err(Fail::failed(detail));
        }
        let Some(p) = pending.filter(|p| p.created.elapsed() < PENDING_TTL) else {
            return Err(Fail { kind: "expired", detail: String::new() });
        };
        if !cfg.active() || p.issuer != cfg.issuer {
            return Err(Fail::failed("the sso settings changed during the sign-in"));
        }
        let code = params.get("code").filter(|c| !c.is_empty()).ok_or_else(|| Fail::failed("the directory sent no code"))?;
        let d = self.discover(&cfg.issuer, false).map_err(Fail::failed)?;

        let mut form: Vec<(&str, &str)> = vec![("grant_type", "authorization_code"), ("code", code), ("redirect_uri", &p.redirect_uri), ("code_verifier", &p.verifier)];
        let mut req = agent().post(&d.token_endpoint).header("Accept", "application/json");
        if cfg.client_secret.is_empty() {
            form.push(("client_id", &cfg.client_id));
        } else {
            let basic = STANDARD.encode(format!("{}:{}", enc(&cfg.client_id), enc(&cfg.client_secret)));
            req = req.header("Authorization", &format!("Basic {basic}"));
        }
        let mut res = req.send_form(form).map_err(|e| Fail::failed(format!("cannot reach the directory: {e}")))?;
        let status = res.status();
        let body: Value = res.body_mut().with_config().limit(1024 * 1024).read_json().unwrap_or(Value::Null);
        if !status.is_success() {
            let detail = body
                .get("error_description")
                .or_else(|| body.get("error"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("the token endpoint answered {}", status.as_u16()));
            return Err(Fail::failed(detail));
        }
        let token = body.get("id_token").and_then(Value::as_str).ok_or_else(|| Fail::failed("the directory sent no id_token"))?;
        let claims = self.verify_id_token(token, &d, &cfg.client_id, &p.nonce).map_err(Fail::failed)?;
        let identity = identity(&claims, &cfg.groups_claim).map_err(Fail::failed)?;
        let role = role_for(&identity.groups, &cfg.groups, cfg.default_role);
        Ok(SignedIn { identity, role, next: p.next })
    }

    /// Checks the signature (keys reloaded once if the key is unknown) and the claims.
    fn verify_id_token(&self, token: &str, d: &Discovery, client_id: &str, nonce: &str) -> Result<Value, String> {
        let (header, claims, input, sig) = split_jwt(token)?;
        let alg = header.get("alg").and_then(Value::as_str).unwrap_or("");
        if !matches!(alg, "RS256" | "PS256" | "ES256") {
            return Err(format!("unsupported id_token algorithm {alg}"));
        }
        let kid = header.get("kid").and_then(Value::as_str).unwrap_or("");
        let find = |keys: &[Jwk]| -> Option<bool> {
            let candidates: Vec<&Jwk> = keys.iter().filter(|k| kid.is_empty() || k.kid == kid).collect();
            if candidates.is_empty() {
                return None;
            }
            Some(candidates.iter().any(|k| verify_signature(alg, k, input.as_bytes(), &sig)))
        };
        let ok = match find(&self.keys(&d.jwks_uri, false)?) {
            Some(ok) => ok,
            None => find(&self.keys(&d.jwks_uri, true)?).ok_or("the id_token is signed with an unknown key")?,
        };
        if !ok {
            return Err("the id_token signature is wrong".into());
        }
        check_claims(&claims, &d.issuer, client_id, nonce, jiff::Timestamp::now().as_second())?;
        Ok(claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
    use serde_json::json;

    fn gr(group: &str, role: Role) -> GroupRole {
        GroupRole { group: group.into(), role }
    }

    #[test]
    fn group_to_role() {
        let map = vec![gr("Net-Viewers", Role::Viewer), gr("/net-ops", Role::Operator), gr("Net-Admins", Role::Admin)];
        let g = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(role_for(&g(&["net-viewers", "/Net-Ops"]), &map, None), Some(Role::Operator));
        assert_eq!(role_for(&g(&["NET-ADMINS", "net-viewers"]), &map, None), Some(Role::Admin));
        assert_eq!(role_for(&g(&["other"]), &map, None), None);
        assert_eq!(role_for(&g(&["other"]), &map, Some(Role::Viewer)), Some(Role::Viewer));
        assert_eq!(role_for(&[], &[], Some(Role::Operator)), Some(Role::Operator));
    }

    #[test]
    fn next_paths() {
        assert_eq!(safe_next("/system/users"), "/system/users");
        assert_eq!(safe_next("//evil.example"), "/");
        assert_eq!(safe_next("https://evil.example"), "/");
        assert_eq!(safe_next("/\\evil"), "/");
        assert_eq!(safe_next("/a//b"), "/");
        assert_eq!(safe_next(""), "/");
        assert_eq!(safe_next("/x\ny"), "/");
    }

    #[test]
    fn identity_claims() {
        let c = json!({"iss": "https://id", "sub": "42", "email": "a@corp.fr", "email_verified": true, "name": "Ann", "groups": ["g1"]});
        let i = identity(&c, "groups").unwrap();
        assert_eq!(i.key, "https://id|42");
        assert_eq!(i.login, "a@corp.fr");
        assert_eq!(i.address.as_deref(), Some("a@corp.fr"));
        assert_eq!(i.groups, vec!["g1"]);
        let c = json!({"iss": "https://id", "sub": "42", "email": "a@corp.fr", "email_verified": false, "preferred_username": "ann", "upn": "ann@corp.fr"});
        let i = identity(&c, "groups").unwrap();
        assert_eq!(i.login, "ann");
        assert_eq!(i.address.as_deref(), Some("ann@corp.fr"));
        let c = json!({"iss": "https://id", "sub": "42", "email": "a@corp.fr"});
        let i = identity(&c, "groups").unwrap();
        assert_eq!(i.login, "a@corp.fr");
        assert_eq!(i.address, None, "unverified e-mail is not guaranteed");
        let c = json!({"iss": "https://id", "sub": "1", "preferred_username": "x", "realm_access": {"roles": ["admins"]}});
        assert_eq!(identity(&c, "realm_access.roles").unwrap().groups, vec!["admins"]);
        assert!(identity(&json!({"iss": "https://id"}), "groups").is_err());
    }

    #[test]
    fn claims_checks() {
        let now = 1_000_000;
        let c = json!({"iss": "https://id/", "aud": ["app"], "exp": now - 60, "nonce": "n"});
        assert!(check_claims(&c, "https://id", "app", "n", now).is_ok());
        assert!(check_claims(&c, "https://other", "app", "n", now).is_err());
        assert!(check_claims(&c, "https://id", "nope", "n", now).is_err());
        assert!(check_claims(&c, "https://id", "app", "x", now).is_err());
        assert!(check_claims(&c, "https://id", "app", "n", now + 200).is_err());
    }

    #[test]
    fn es256_signature() {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let kp = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
        let pubkey = kp.public_key().as_ref();
        let jwk = Jwk { kid: "k1".into(), kty: "EC".into(), crv: "P-256".into(), x: URL_SAFE_NO_PAD.encode(&pubkey[1..33]), y: URL_SAFE_NO_PAD.encode(&pubkey[33..65]), ..Default::default() };
        let h = URL_SAFE_NO_PAD.encode(br#"{"alg":"ES256","kid":"k1"}"#);
        let p = URL_SAFE_NO_PAD.encode(br#"{"iss":"https://id","sub":"1"}"#);
        let input = format!("{h}.{p}");
        let sig = kp.sign(&rng, input.as_bytes()).unwrap();
        let token = format!("{input}.{}", URL_SAFE_NO_PAD.encode(sig.as_ref()));
        let (header, claims, si, s) = split_jwt(&token).unwrap();
        assert_eq!(header["kid"], "k1");
        assert_eq!(claims["sub"], "1");
        assert!(verify_signature("ES256", &jwk, si.as_bytes(), &s));
        let tampered = format!("{h}.{}", URL_SAFE_NO_PAD.encode(br#"{"iss":"https://id","sub":"2"}"#));
        assert!(!verify_signature("ES256", &jwk, tampered.as_bytes(), &s));
        assert!(!verify_signature("RS256", &jwk, si.as_bytes(), &s));
    }

    #[test]
    fn url_encoding() {
        assert_eq!(enc("a b/c?d=é"), "a%20b%2Fc%3Fd%3D%C3%A9");
        let f = Fail { kind: "noGroup", detail: "x y".into() };
        assert_eq!(f.redirect(), "/?sso_error=noGroup&sso_detail=x%20y");
    }
}
