//! HTTPS certificates: a self-signed one made by the server, or one the admin brings (PEM).

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::config::HttpsMode;
use crate::paths;

fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("tls")
}

/// (certificate, key) files of a mode.
pub fn files(data_dir: &Path, mode: HttpsMode) -> (PathBuf, PathBuf) {
    let d = dir(data_dir);
    match mode {
        HttpsMode::Custom => (d.join("custom.crt"), d.join("custom.key")),
        _ => (d.join("self-signed.crt"), d.join("self-signed.key")),
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn parse_pair(cert_pem: &[u8], key_pem: &[u8]) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>), String> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem).collect::<Result<_, _>>().map_err(|_| "the certificate is not valid PEM".to_string())?;
    if certs.is_empty() {
        return Err("no certificate in the PEM text".into());
    }
    let key = PrivateKeyDer::from_pem_slice(key_pem).map_err(|_| "the private key is not valid PEM (PKCS#8, PKCS#1 or SEC1)".to_string())?;
    Ok((certs, key))
}

fn server_config(cert_pem: &[u8], key_pem: &[u8]) -> Result<rustls::ServerConfig, String> {
    let (certs, key) = parse_pair(cert_pem, key_pem)?;
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("the key does not fit the certificate: {e}"))?;
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(cfg)
}

/// Checks a certificate and its key (they must match).
pub fn check_pair(cert_pem: &str, key_pem: &str) -> Result<(), String> {
    server_config(cert_pem.as_bytes(), key_pem.as_bytes()).map(|_| ())
}

/// Names the self-signed certificate covers: localhost, the machine name, its addresses.
fn local_names() -> Vec<String> {
    let mut names = vec!["localhost".to_string()];
    let host = gethostname::gethostname().to_string_lossy().to_string();
    let plain = host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    if !host.is_empty() && host != "localhost" && plain {
        names.push(host.clone());
        if !host.contains('.') {
            names.push(format!("{host}.local"));
        }
    }
    names.push("127.0.0.1".into());
    names.push("::1".into());
    for i in if_addrs::get_if_addrs().unwrap_or_default() {
        if !i.is_loopback() {
            let ip = i.ip().to_string();
            if !names.contains(&ip) {
                names.push(ip);
            }
        }
    }
    names
}

/// Makes a new self-signed certificate (blocking).
pub fn generate_self_signed(data_dir: &Path) -> Result<(), String> {
    let names = local_names();
    let mut params = rcgen::CertificateParams::new(names).map_err(|e| e.to_string())?;
    let host = gethostname::gethostname().to_string_lossy().to_string();
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.distinguished_name.push(rcgen::DnType::CommonName, format!("Strata Network ({host})"));
    params.distinguished_name.push(rcgen::DnType::OrganizationName, "Strata");
    let today = jiff::Zoned::now().date();
    params.not_before = rcgen::date_time_ymd(today.year() as i32 - 1, today.month() as u8, 1);
    params.not_after = rcgen::date_time_ymd(today.year() as i32 + 10, today.month() as u8, 1);
    let key = rcgen::KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = params.self_signed(&key).map_err(|e| e.to_string())?;
    let (c, k) = files(data_dir, HttpsMode::SelfSigned);
    paths::write_atomic(&k, key.serialize_pem().as_bytes(), true).map_err(|e| e.to_string())?;
    paths::write_atomic(&c, cert.pem().as_bytes(), false).map_err(|e| e.to_string())?;
    tracing::info!("new self-signed certificate made");
    Ok(())
}

/// Saves the admin's certificate and key (blocking).
pub fn install_custom(data_dir: &Path, cert_pem: &str, key_pem: &str) -> Result<(), String> {
    check_pair(cert_pem, key_pem)?;
    let (c, k) = files(data_dir, HttpsMode::Custom);
    paths::write_atomic(&k, key_pem.as_bytes(), true).map_err(|e| e.to_string())?;
    paths::write_atomic(&c, cert_pem.as_bytes(), false).map_err(|e| e.to_string())?;
    Ok(())
}

/// The rustls configuration of a mode, making the self-signed certificate when missing
/// (blocking).
pub fn load(data_dir: &Path, mode: HttpsMode) -> Result<rustls::ServerConfig, String> {
    let (c, k) = files(data_dir, mode);
    if mode == HttpsMode::SelfSigned && !(c.exists() && k.exists()) {
        generate_self_signed(data_dir)?;
    }
    let cert = std::fs::read(&c).map_err(|_| "no certificate yet".to_string())?;
    let key = std::fs::read(&k).map_err(|_| "no private key yet".to_string())?;
    server_config(&cert, &key)
}

fn x509_time_ms(t: &x509_parser::time::ASN1Time) -> i64 {
    t.timestamp() * 1000
}

/// What the Security page shows of a certificate file (blocking).
pub fn info(path: &Path) -> Option<Value> {
    let pem = std::fs::read(path).ok()?;
    let der = CertificateDer::pem_slice_iter(&pem).next()?.ok()?;
    let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).ok()?;
    let mut names = Vec::new();
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for n in &san.value.general_names {
            match n {
                x509_parser::extensions::GeneralName::DNSName(d) => names.push(d.to_string()),
                x509_parser::extensions::GeneralName::IPAddress(b) => {
                    let ip = match b.len() {
                        4 => Some(IpAddr::from(<[u8; 4]>::try_from(*b).ok()?)),
                        16 => Some(IpAddr::from(<[u8; 16]>::try_from(*b).ok()?)),
                        _ => None,
                    };
                    if let Some(ip) = ip {
                        names.push(ip.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    let fp = Sha256::digest(der.as_ref()).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":");
    let validity = cert.validity();
    Some(json!({
        "subject": cert.subject().to_string(),
        "issuer": cert.issuer().to_string(),
        "names": names,
        "notBefore": x509_time_ms(&validity.not_before),
        "notAfter": x509_time_ms(&validity.not_after),
        "selfSigned": cert.subject() == cert.issuer(),
        "fingerprint": fp,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_signed_roundtrip() {
        let dir = std::env::temp_dir().join(format!("sn-tls-{}", paths::new_id()));
        generate_self_signed(&dir).unwrap();
        assert!(load(&dir, HttpsMode::SelfSigned).is_ok());
        let (c, k) = files(&dir, HttpsMode::SelfSigned);
        let i = info(&c).unwrap();
        assert!(i["names"].as_array().unwrap().iter().any(|n| n == "localhost"));
        assert!(i["names"].as_array().unwrap().iter().any(|n| n == "127.0.0.1"));
        assert_eq!(i["selfSigned"], true);
        let cert = std::fs::read_to_string(&c).unwrap();
        let key = std::fs::read_to_string(&k).unwrap();
        assert!(check_pair(&cert, &key).is_ok());
        assert!(check_pair(&cert, "nope").is_err());
        install_custom(&dir, &cert, &key).unwrap();
        assert!(load(&dir, HttpsMode::Custom).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }
}
