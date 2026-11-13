//! Gzip copies of the web app (`../../web/dist`) embedded next to the originals, and the
//! Windows icon.

use std::io::Write;
use std::path::{Path, PathBuf};

const TEXT: &[&str] = &["js", "mjs", "css", "html", "svg", "json", "txt", "map", "ico", "xml"];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist = manifest.join("../../web/dist");
    // The server compiles without the web app.
    std::fs::create_dir_all(&dist).expect("create web/dist");
    println!("cargo:rerun-if-changed=../../web/dist");
    println!("cargo:rerun-if-changed=build.rs");

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let gz_dir = out.join("web-gz");
    let _ = std::fs::remove_dir_all(&gz_dir);
    std::fs::create_dir_all(&gz_dir).unwrap();

    let mut files = Vec::new();
    walk(&dist, &mut files);
    files.sort();
    let mut list = String::from("&[\n");
    for f in files {
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if !TEXT.contains(&ext.as_str()) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&f) else { continue };
        if bytes.len() < 512 {
            continue;
        }
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        enc.write_all(&bytes).unwrap();
        let gz = enc.finish().unwrap();
        // Keep only when it saves at least 10 %.
        if gz.len() * 10 > bytes.len() * 9 {
            continue;
        }
        let rel = f.strip_prefix(&dist).unwrap().to_string_lossy().replace('\\', "/");
        let target = gz_dir.join(format!("{rel}.gz"));
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, &gz).unwrap();
        list.push_str(&format!("    ({rel:?}, include_bytes!({:?}) as &[u8]),\n", target.to_string_lossy()));
    }
    list.push(']');
    std::fs::write(out.join("web_gz.rs"), list).unwrap();

    #[cfg(windows)]
    {
        let icon = manifest.join("../../web/public/favicon.ico");
        let mut res = winresource::WindowsResource::new();
        if icon.exists() {
            res.set_icon(icon.to_str().unwrap());
        }
        res.set("FileDescription", "Strata Network");
        res.set("ProductName", "Strata Network");
        if let Err(e) = res.compile() {
            println!("cargo:warning=windows resources: {e}");
        }
    }
}
