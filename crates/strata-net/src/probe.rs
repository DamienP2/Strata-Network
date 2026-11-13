//! What an open port serves: a web page (http or https, with its title) or a
//! text banner (ssh, ftp, smtp…).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

/// A web page found on a port.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Web {
    /// `http` or `https`.
    pub scheme: String,
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
}

const READ_LIMIT: usize = 96 * 1024;

/// Looks for a web page on `ip:port`: plain HTTP first (TLS first on the
/// usual https ports), then the other.
pub async fn web(ip: Ipv4Addr, port: u16, timeout: Duration) -> Option<Web> {
    let tls_first = matches!(port, 443 | 4443 | 5001 | 8443 | 9443 | 10443);
    for tls in if tls_first { [true, false] } else { [false, true] } {
        if let Some(w) = try_web(ip, port, tls, timeout).await {
            // A TLS port answering plain HTTP with 400 "plain HTTP sent to HTTPS port": keep looking.
            if !tls && w.status == 400 && !tls_first {
                if let Some(w2) = try_web(ip, port, true, timeout).await {
                    return Some(w2);
                }
            }
            return Some(w);
        }
    }
    None
}

async fn try_web(ip: Ipv4Addr, port: u16, tls: bool, timeout: Duration) -> Option<Web> {
    let host = if port == 80 || port == 443 { ip.to_string() } else { format!("{ip}:{port}") };
    let req = format!("GET / HTTP/1.1\r\nHost: {host}\r\nUser-Agent: Strata-Network\r\nAccept: text/html,*/*\r\nConnection: close\r\n\r\n");
    let work = async {
        let tcp = TcpStream::connect(SocketAddr::from((ip, port))).await.ok()?;
        let raw = if tls {
            let conn = tls_connector().connect(ServerName::IpAddress(IpAddr::V4(ip).into()), tcp).await.ok()?;
            exchange(conn, req.as_bytes()).await
        } else {
            exchange(tcp, req.as_bytes()).await
        }?;
        parse_http(&raw, if tls { "https" } else { "http" })
    };
    tokio::time::timeout(timeout * 3, work).await.ok().flatten()
}

async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(mut s: S, req: &[u8]) -> Option<Vec<u8>> {
    s.write_all(req).await.ok()?;
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    loop {
        match s.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() >= READ_LIMIT || find_ci(&buf, b"</title>").is_some() {
                    break;
                }
            }
        }
    }
    (!buf.is_empty()).then_some(buf)
}

/// Reads an HTTP answer: status, `Server`, and the page title.
pub fn parse_http(raw: &[u8], scheme: &str) -> Option<Web> {
    if !raw.starts_with(b"HTTP/") {
        return None;
    }
    let text = String::from_utf8_lossy(raw);
    let status = text.split_whitespace().nth(1)?.parse().ok()?;
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let server = head
        .lines()
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("server")).map(|(_, v)| v.trim().to_string()))
        .filter(|s| !s.is_empty());
    Some(Web { scheme: scheme.into(), status, title: title(body), server })
}

fn find_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle))
}

/// The `<title>` of a page, entities decoded, spaces collapsed, 120 characters at most.
pub fn title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open = start + lower[start..].find('>')? + 1;
    let end = open + lower[open..].find("</title")?;
    let t = html[open..end]
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty()).then(|| t.chars().take(120).collect())
}

/// The first line a service sends on its own (ssh, ftp, smtp, telnet…).
pub async fn banner(ip: Ipv4Addr, port: u16, timeout: Duration) -> Option<String> {
    let work = async {
        let mut s = TcpStream::connect(SocketAddr::from((ip, port))).await.ok()?;
        let mut buf = [0u8; 512];
        let n = s.read(&mut buf).await.ok()?;
        let line = String::from_utf8_lossy(&buf[..n]);
        let line: String = line.lines().next()?.chars().filter(|c| !c.is_control()).take(100).collect();
        let line = line.trim().to_string();
        (!line.is_empty()).then_some(line)
    };
    tokio::time::timeout(timeout * 3, work).await.ok().flatten()
}

fn tls_connector() -> tokio_rustls::TlsConnector {
    static C: OnceLock<tokio_rustls::TlsConnector> = OnceLock::new();
    C.get_or_init(|| {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .expect("tls versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyCert(provider)))
            .with_no_client_auth();
        tokio_rustls::TlsConnector::from(Arc::new(cfg))
    })
    .clone()
}

/// Equipment consoles use self-signed certificates: we only want to know
/// whether the port speaks TLS, so any certificate is accepted.
#[derive(Debug)]
struct AnyCert(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for AnyCert {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_http_answers() {
        let raw = b"HTTP/1.1 200 OK\r\nServer: lighttpd\r\nContent-Type: text/html\r\n\r\n<html><head><TITLE>\n AW-UE150 &amp; Web\n</TITLE></head>";
        let w = parse_http(raw, "http").unwrap();
        assert_eq!(w.status, 200);
        assert_eq!(w.server.as_deref(), Some("lighttpd"));
        assert_eq!(w.title.as_deref(), Some("AW-UE150 & Web"));
        assert!(parse_http(b"SSH-2.0-OpenSSH_9.6", "http").is_none());
        assert_eq!(title("<title></title>"), None);
    }

    #[tokio::test]
    async fn finds_a_local_page() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (mut s, _) = l.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut b = [0u8; 1024];
                    let n = s.read(&mut b).await.unwrap_or(0);
                    if b[..n].starts_with(b"GET") {
                        let _ = s.write_all(b"HTTP/1.0 200 OK\r\n\r\n<title>Hello</title>").await;
                    }
                });
            }
        });
        let w = web(Ipv4Addr::LOCALHOST, port, Duration::from_millis(500)).await.unwrap();
        assert_eq!((w.scheme.as_str(), w.title.as_deref()), ("http", Some("Hello")));
    }
}
