//! The web app served by the binary, and on-the-fly gzip of API answers.

use std::path::PathBuf;

use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::Router;
use rust_embed::RustEmbed;
use tower_http::services::{ServeDir, ServeFile};

use crate::state::Shared;

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct WebApp;

/// Gzip copies made by `build.rs`: (path, bytes).
static GZ: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/web_gz.rs"));

const NOT_BUILT: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Strata Network</title></head>\
<body style=\"font:14px system-ui;background:#0b0c12;color:#e6e8f2;display:grid;place-items:center;height:100vh;margin:0\">\
<p>Web app not built — run <code>cd web &amp;&amp; npm install &amp;&amp; npm run build</code></p></body></html>";

/// Whether the client accepts gzip (`q=0` respected).
pub fn accepts_gzip(headers: &HeaderMap) -> bool {
    let v = headers.get(header::ACCEPT_ENCODING).and_then(|v| v.to_str().ok()).unwrap_or("");
    let mut star = false;
    for part in v.split(',') {
        let mut it = part.split(';');
        let name = it.next().unwrap_or("").trim().to_ascii_lowercase();
        let q = it
            .find_map(|p| p.trim().strip_prefix("q=").map(|q| q.trim().parse::<f32>().unwrap_or(0.0)))
            .unwrap_or(1.0);
        if name == "gzip" || name == "x-gzip" {
            return q > 0.0;
        }
        if name == "*" {
            star = q > 0.0;
        }
    }
    star
}

fn gz_of(path: &str) -> Option<&'static [u8]> {
    GZ.iter().find(|(p, _)| *p == path).map(|(_, b)| *b)
}

fn serve_file(path: &str, headers: &HeaderMap) -> Option<Response> {
    let file = WebApp::get(path)?;
    let mime = file.metadata.mimetype().to_string();
    let cache = if path.starts_with("static/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    let gz = gz_of(path);
    let mut res = match gz {
        Some(bytes) if accepts_gzip(headers) => {
            let mut r = Response::new(Body::from(Bytes::from_static(bytes)));
            r.headers_mut().insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
            r
        }
        _ => Response::new(Body::from(file.data.into_owned())),
    };
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&mime) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if gz.is_some() {
        h.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    }
    Some(res)
}

/// The fallback: the file asked for, else `index.html` (routes of the web app).
pub async fn embedded(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path == "api" || path.starts_with("api/") || path == "ws" {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let path = if path.is_empty() { "index.html" } else { path };
    if path.contains("..") {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    if let Some(res) = serve_file(path, &headers) {
        return res;
    }
    // A missing hashed file is a real 404, not a page of the app.
    if path.starts_with("static/") || path.starts_with("assets/") {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match serve_file("index.html", &headers) {
        Some(res) => res,
        None => (StatusCode::SERVICE_UNAVAILABLE, Html(NOT_BUILT)).into_response(),
    }
}

/// `--web-dir`: serves a folder instead of the embedded copy.
pub fn with_dir(router: Router<Shared>, dir: PathBuf) -> Router<Shared> {
    let index = dir.join("index.html");
    router.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
}

fn compressible(ct: &str) -> bool {
    let ct = ct.to_ascii_lowercase();
    ct.starts_with("application/json") || ct.starts_with("text/") || ct.starts_with("image/svg+xml") || ct.contains("xml") || ct.contains("javascript")
}

fn gzip(bytes: &[u8], level: u32) -> std::io::Result<Vec<u8>> {
    use std::io::Write;
    let mut enc = flate2::write::GzEncoder::new(Vec::with_capacity(bytes.len() / 3), flate2::Compression::new(level));
    enc.write_all(bytes)?;
    enc.finish()
}

const MIN: u64 = 1024;
const MAX: u64 = 64 * 1024 * 1024;
const BIG: u64 = 256 * 1024;

/// Middleware: gzip of 200 JSON / text / SVG / XML answers between 1 KB and 64 MB.
pub async fn compress(req: Request, next: Next) -> Response {
    let ok = accepts_gzip(req.headers());
    let res = next.run(req).await;
    if !ok || res.status() != StatusCode::OK || res.headers().contains_key(header::CONTENT_ENCODING) {
        return res;
    }
    let ct = res.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !compressible(ct) {
        return res;
    }
    let Some(size) = axum::body::HttpBody::size_hint(res.body()).exact() else { return res };
    if !(MIN..=MAX).contains(&size) {
        return res;
    }
    let (mut parts, body) = res.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX as usize).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "cannot read the answer").into_response(),
    };
    let packed = if size > BIG {
        let b = bytes.clone();
        tokio::task::spawn_blocking(move || gzip(&b, 1)).await.ok().and_then(Result::ok)
    } else {
        gzip(&bytes, 6).ok()
    };
    let body = match packed {
        Some(gz) => {
            parts.headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
            parts.headers.remove(header::CONTENT_LENGTH);
            parts.headers.append(header::VARY, HeaderValue::from_static("accept-encoding"));
            Body::from(gz)
        }
        None => Body::from(bytes),
    };
    Response::from_parts(parts, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(v: &str) -> HeaderMap {
        let mut m = HeaderMap::new();
        m.insert(header::ACCEPT_ENCODING, HeaderValue::from_str(v).unwrap());
        m
    }

    #[test]
    fn gzip_negotiation() {
        assert!(accepts_gzip(&h("gzip, deflate, br")));
        assert!(!accepts_gzip(&h("gzip;q=0, br")));
        assert!(accepts_gzip(&h("br, *")));
        assert!(!accepts_gzip(&h("*;q=0")));
        assert!(!accepts_gzip(&h("br")));
        assert!(!accepts_gzip(&HeaderMap::new()));
        assert!(accepts_gzip(&h("gzip;q=0.5")));
    }
}
