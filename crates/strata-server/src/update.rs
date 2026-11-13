//! Updates from the GitHub releases: never installed automatically, the check only informs.

use std::cmp::Ordering;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::paths;

pub const DEFAULT_REPO: &str = "DamienP2/Strata-Network";
pub const BIN: &str = "strata-network";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
pub const MAX_PACKAGE: u64 = 512 * 1024 * 1024;
const AUTO_FIRST: Duration = Duration::from_secs(30);
const AUTO_EVERY: Duration = Duration::from_secs(6 * 3600);
const MAX_FAILED_STARTS: u32 = 3;

/// The platform of this binary, as in the package names.
pub fn platform() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "linux-arm64"
    } else if cfg!(target_os = "linux") {
        "linux-x86_64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-arm64"
    } else if cfg!(target_os = "macos") {
        "macos-x86_64"
    } else {
        "windows-x86_64"
    }
}

fn package_ext() -> &'static str {
    if cfg!(windows) {
        "zip"
    } else {
        "tar.gz"
    }
}

pub fn package_name(version: &str) -> String {
    format!("Strata-{version}-{}.{}", platform(), package_ext())
}

fn exe_name() -> String {
    if cfg!(windows) {
        format!("{BIN}.exe")
    } else {
        BIN.to_string()
    }
}

/// Numeric comparison of "x.y.z" ("1.2.10" > "1.2.9"); a leading `v` and any suffix after `-`
/// or `+` are ignored.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    fn parts(v: &str) -> Vec<u64> {
        let v = v.trim().trim_start_matches(['v', 'V']);
        let v = v.split(['-', '+']).next().unwrap_or("");
        v.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect()
    }
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let o = pa.get(i).copied().unwrap_or(0).cmp(&pb.get(i).copied().unwrap_or(0));
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

/// "strata-network 1.2.3" → "1.2.3".
pub fn parse_version_output(out: &str) -> Option<String> {
    let line = out.lines().next()?.trim();
    let (name, version) = line.rsplit_once(' ')?;
    if name.trim() != BIN || version.is_empty() || !version.chars().next()?.is_ascii_digit() {
        return None;
    }
    Some(version.to_string())
}

/// `owner/name` or a github.com URL → `owner/name`.
pub fn parse_repo(s: &str) -> Result<String, String> {
    let s = s.trim().trim_end_matches('/').trim_end_matches(".git");
    let s = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(s);
    let s = s.strip_prefix("www.").unwrap_or(s);
    let s = s.strip_prefix("github.com/").unwrap_or(s);
    let mut it = s.split('/');
    let ok = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    match (it.next(), it.next(), it.next()) {
        (Some(o), Some(n), None) if ok(o) && ok(n) => Ok(format!("{o}/{n}")),
        _ => Err("the repository is written owner/name".into()),
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "default_repo")]
    pub repo: String,
    #[serde(default)]
    pub token: String,
    #[serde(default = "yes")]
    pub auto_check: bool,
}

fn default_repo() -> String {
    DEFAULT_REPO.into()
}
fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Settings { repo: default_repo(), token: String::new(), auto_check: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    Downloading,
    Installing,
    Restarting,
    Error,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    #[serde(skip)]
    pub id: u64,
    pub name: String,
    pub size: u64,
    #[serde(skip)]
    pub digest: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    pub tag: String,
    pub name: String,
    pub notes: String,
    pub published_at: String,
    pub url: String,
    pub asset: Asset,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub phase: Phase,
    pub progress: f32,
    pub message: String,
    pub latest: Option<Release>,
    pub releases: Vec<Release>,
    pub last_check: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub from: String,
    pub to: String,
    pub at: i64,
    pub outcome: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct Pending {
    from: String,
    to: String,
    starts: u32,
}

struct Updater {
    exe: PathBuf,
    status: Mutex<Status>,
    previous: Mutex<Option<(SystemTime, String)>>,
}

static UPDATER: OnceLock<Updater> = OnceLock::new();

fn updater() -> &'static Updater {
    UPDATER.get_or_init(|| Updater {
        exe: std::env::current_exe().ok().and_then(|p| p.canonicalize().ok()).unwrap_or_else(|| PathBuf::from(exe_name())),
        status: Mutex::new(Status::default()),
        previous: Mutex::new(None),
    })
}

fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("update")
}

pub fn load_settings(data_dir: &Path) -> Settings {
    paths::read_json(&dir(data_dir).join("settings.json")).ok().flatten().unwrap_or_default()
}

fn save_settings(data_dir: &Path, s: &Settings) -> std::io::Result<()> {
    paths::write_json(&dir(data_dir).join("settings.json"), s, true)
}

pub fn history(data_dir: &Path) -> Vec<HistoryEntry> {
    paths::read_json(&dir(data_dir).join("history.json")).ok().flatten().unwrap_or_default()
}

fn add_history(data_dir: &Path, from: &str, to: &str, outcome: &str) {
    let mut h = history(data_dir);
    h.push(HistoryEntry { from: from.into(), to: to.into(), at: paths::now_ms(), outcome: outcome.into() });
    if h.len() > 50 {
        h.drain(..h.len() - 50);
    }
    if let Err(e) = paths::write_json(&dir(data_dir).join("history.json"), &h, false) {
        tracing::error!("cannot write the update history: {e}");
    }
}

fn set_status(f: impl FnOnce(&mut Status)) {
    if let Some(u) = UPDATER.get() {
        f(&mut u.status.lock().unwrap());
    }
}

pub fn status() -> Status {
    UPDATER.get().map(|u| u.status.lock().unwrap().clone()).unwrap_or_default()
}

fn busy() -> bool {
    matches!(status().phase, Phase::Checking | Phase::Downloading | Phase::Installing | Phase::Restarting)
}

fn previous_path(exe: &Path) -> PathBuf {
    exe.with_file_name(format!("{}.previous", exe_name()))
}

/// Runs `<bin> --version` (10 s at most).
fn run_version(path: &Path) -> Result<String, String> {
    let mut child = Command::new(path)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("the new program does not start: {e}"))?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > Duration::from_secs(10) => {
                let _ = child.kill();
                return Err("the new program does not answer".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut out);
    }
    parse_version_output(&out).ok_or_else(|| format!("this is not {BIN}"))
}

/// The version of `<bin>.previous`, if there is one (blocking, cached).
pub fn previous_version() -> Option<String> {
    let u = UPDATER.get()?;
    let p = previous_path(&u.exe);
    let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
    if let Some((t, v)) = u.previous.lock().unwrap().as_ref() {
        if *t == mtime {
            return Some(v.clone());
        }
    }
    let v = run_version(&p).ok()?;
    *u.previous.lock().unwrap() = Some((mtime, v.clone()));
    Some(v)
}

/// `GET /api/system/update` (blocking).
pub fn info(data_dir: &Path, supervised: bool) -> Value {
    updater();
    let s = load_settings(data_dir);
    json!({
        "current": CURRENT,
        "platform": platform(),
        "package": package_name(CURRENT),
        "repo": s.repo,
        "hasToken": !s.token.is_empty(),
        "autoCheck": s.auto_check,
        "previous": previous_version(),
        "history": history(data_dir),
        "status": status(),
        "supervised": supervised,
    })
}

/// `PUT /api/system/update/settings`: token `None` = keep, `""` = remove.
pub fn set_settings(data_dir: &Path, repo: Option<&str>, token: Option<&str>, auto_check: Option<bool>) -> Result<(), (u16, String)> {
    let mut s = load_settings(data_dir);
    if let Some(r) = repo {
        s.repo = parse_repo(r).map_err(|e| (400, e))?;
    }
    if let Some(t) = token {
        s.token = t.trim().to_string();
    }
    if let Some(a) = auto_check {
        s.auto_check = a;
    }
    save_settings(data_dir, &s).map_err(|e| (500, e.to_string()))
}

fn agent(redirects: u32) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(600)))
        .max_redirects(redirects)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .user_agent(concat!("strata/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

/// The package of this platform in a release's assets, by order of preference.
fn pick_asset(assets: &[Value]) -> Option<Asset> {
    let plat = platform();
    let mut wanted = vec![format!("-{plat}.{}", package_ext())];
    if cfg!(target_os = "macos") {
        wanted.push(format!("-{plat}.pkg"));
        wanted.push(".pkg".into());
    }
    if cfg!(windows) {
        wanted.push(format!("-{plat}.msi"));
        wanted.push(".msi".into());
    }
    for w in wanted {
        for a in assets {
            let name = a.get("name").and_then(Value::as_str).unwrap_or("");
            let uploaded = a.get("state").and_then(Value::as_str) == Some("uploaded");
            if uploaded && name.ends_with(&w) {
                return Some(Asset {
                    id: a.get("id").and_then(Value::as_u64).unwrap_or(0),
                    name: name.to_string(),
                    size: a.get("size").and_then(Value::as_u64).unwrap_or(0),
                    digest: a.get("digest").and_then(Value::as_str).map(str::to_string),
                });
            }
        }
    }
    None
}

/// The releases usable here, newest first.
pub fn parse_releases(list: &Value) -> Vec<Release> {
    let mut out: Vec<Release> = list
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter(|r| !r.get("draft").and_then(Value::as_bool).unwrap_or(false) && !r.get("prerelease").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|r| {
            let tag = r.get("tag_name").and_then(Value::as_str)?.to_string();
            let version = tag.trim_start_matches(['v', 'V']).to_string();
            if !version.chars().next()?.is_ascii_digit() {
                return None;
            }
            let asset = pick_asset(r.get("assets").and_then(Value::as_array).map(|a| a.as_slice()).unwrap_or(&[]))?;
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            Some(Release { version, tag, name: s("name"), notes: s("body"), published_at: s("published_at"), url: s("html_url"), asset })
        })
        .collect();
    out.sort_by(|a, b| compare_versions(&b.version, &a.version));
    out
}

/// Asks GitHub for the releases (blocking). The newest one, or `None`.
pub fn check(data_dir: &Path) -> Result<Option<Release>, String> {
    updater();
    if busy() {
        return Err("an update is already running".into());
    }
    set_status(|s| {
        s.phase = Phase::Checking;
        s.message = "checking for updates".into();
        s.progress = 0.0;
    });
    let out = fetch_releases(data_dir);
    set_status(|s| {
        s.last_check = Some(paths::now_ms());
        match &out {
            Ok(list) => {
                s.phase = Phase::Idle;
                s.message.clear();
                s.latest = list.first().cloned();
                s.releases = list.clone();
            }
            Err(e) => {
                s.phase = Phase::Error;
                s.message = e.clone();
            }
        }
    });
    out.map(|l| l.into_iter().next())
}

fn fetch_releases(data_dir: &Path) -> Result<Vec<Release>, String> {
    let s = load_settings(data_dir);
    let url = format!("https://api.github.com/repos/{}/releases?per_page=50", s.repo);
    let mut req = agent(0).get(&url).header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28");
    if !s.token.is_empty() {
        req = req.header("Authorization", &format!("Bearer {}", s.token));
    }
    let mut res = req.call().map_err(|e| format!("cannot reach GitHub: {e}"))?;
    match res.status().as_u16() {
        200 => {}
        401 => return Err("GitHub refused the token (expired or revoked?)".into()),
        403 | 429 => return Err(format!("GitHub refused the request (rate limit, or the token cannot read {})", s.repo)),
        404 if s.token.is_empty() => return Err(format!("{} not found: the repository is private, add a read-only token", s.repo)),
        404 => return Err(format!("{} not found", s.repo)),
        code => return Err(format!("GitHub answered {code}")),
    }
    let bytes = res.body_mut().with_config().limit(32 * 1024 * 1024).read_to_vec().map_err(|e| format!("cannot read the answer of GitHub: {e}"))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("GitHub sent a bad answer: {e}"))?;
    Ok(parse_releases(&v))
}

/// Starts an installation in the background. `version` = one picked in the list (may be older).
pub fn start_install(data_dir: &Path, version: Option<String>, supervised: bool) -> Result<(), (u16, String)> {
    updater();
    if busy() {
        return Err((409, "an update is already running".into()));
    }
    let data_dir = data_dir.to_path_buf();
    let st = status();
    let explicit = version.is_some();
    let release = match &version {
        Some(v) => st.releases.iter().find(|r| r.version == *v).cloned(),
        None => st.latest.clone(),
    };
    let release = match release {
        Some(r) => r,
        None => {
            // Not checked yet: ask GitHub first.
            let latest = check(&data_dir).map_err(|e| (502, e))?;
            let st = status();
            match &version {
                Some(v) => st.releases.iter().find(|r| r.version == *v).cloned(),
                None => latest,
            }
            .ok_or((404, "no such release".to_string()))?
        }
    };
    if !explicit && compare_versions(&release.version, CURRENT) != Ordering::Greater {
        return Err((409, format!("version {CURRENT} is already the latest")));
    }
    set_status(|s| {
        s.phase = Phase::Downloading;
        s.progress = 0.0;
        s.message = format!("downloading {}", release.version);
    });
    std::thread::spawn(move || {
        let result = download(&data_dir, &release).and_then(|bytes| {
            set_status(|s| {
                s.phase = Phase::Installing;
                s.progress = 1.0;
                s.message = format!("installing {}", release.version);
            });
            install_bytes(&data_dir, &bytes, explicit)
        });
        match result {
            Ok(v) => {
                tracing::info!("version {v} installed, restarting");
                restart_soon(supervised);
            }
            Err(e) => {
                tracing::error!("update failed: {e}");
                set_status(|s| {
                    s.phase = Phase::Error;
                    s.message = e;
                });
            }
        }
    });
    Ok(())
}

fn download(data_dir: &Path, r: &Release) -> Result<Vec<u8>, String> {
    let s = load_settings(data_dir);
    let url = format!("https://api.github.com/repos/{}/releases/assets/{}", s.repo, r.asset.id);
    let mut req = agent(0).get(&url).header("Accept", "application/octet-stream").header("X-GitHub-Api-Version", "2022-11-28");
    if !s.token.is_empty() {
        req = req.header("Authorization", &format!("Bearer {}", s.token));
    }
    let mut res = req.call().map_err(|e| format!("cannot reach GitHub: {e}"))?;
    if res.status().is_redirection() {
        // The storage link is followed without the token.
        let loc = res.headers().get("location").and_then(|v| v.to_str().ok()).ok_or("GitHub sent a redirection without address")?.to_string();
        res = agent(5).get(&loc).call().map_err(|e| format!("cannot download the package: {e}"))?;
    }
    if !res.status().is_success() {
        return Err(format!("the download answered {}", res.status().as_u16()));
    }
    let total = res.body().content_length().unwrap_or(r.asset.size).max(1);
    if total > MAX_PACKAGE {
        return Err("the package is larger than 512 MB".into());
    }
    let mut reader = res.into_body().into_with_config().limit(MAX_PACKAGE).reader();
    let mut bytes = Vec::with_capacity(total as usize);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("the download stopped: {e}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        let p = (bytes.len() as f32 / total as f32).min(1.0);
        set_status(|s| s.progress = p);
    }
    if let Some(d) = r.asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")) {
        let got = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
        if !got.eq_ignore_ascii_case(d) {
            return Err("the package is damaged (SHA-256 differs)".into());
        }
    }
    Ok(bytes)
}

enum Kind {
    TarGz,
    Zip,
    Pkg,
    Msi,
    Binary,
}

fn kind_of(bytes: &[u8]) -> Option<Kind> {
    let b = |m: &[u8]| bytes.starts_with(m);
    if b(&[0x1f, 0x8b]) {
        Some(Kind::TarGz)
    } else if b(b"PK\x03\x04") {
        Some(Kind::Zip)
    } else if b(b"xar!") {
        Some(Kind::Pkg)
    } else if b(&[0xd0, 0xcf, 0x11, 0xe0]) {
        Some(Kind::Msi)
    } else if b(b"\x7fELF") || b(&[0xfe, 0xed, 0xfa, 0xce]) || b(&[0xfe, 0xed, 0xfa, 0xcf]) || b(&[0xce, 0xfa, 0xed, 0xfe]) || b(&[0xcf, 0xfa, 0xed, 0xfe]) || b(&[0xca, 0xfe, 0xba, 0xbe]) || b(b"MZ") {
        Some(Kind::Binary)
    } else {
        None
    }
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find_file(&p, name) {
                return Some(f);
            }
        } else if p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
    }
    None
}

/// The server program inside a package.
pub fn extract(bytes: &[u8], work: &Path) -> Result<Vec<u8>, String> {
    let exe = exe_name();
    match kind_of(bytes).ok_or("this file is not a package of the server")? {
        Kind::Binary => Ok(bytes.to_vec()),
        Kind::TarGz => {
            let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
            for entry in ar.entries().map_err(|e| format!("bad package: {e}"))? {
                let mut entry = entry.map_err(|e| format!("bad package: {e}"))?;
                let is_it = entry.path().ok().and_then(|p| p.file_name().map(|n| n == exe.as_str())).unwrap_or(false);
                if is_it && entry.header().entry_type().is_file() {
                    let mut out = Vec::new();
                    entry.read_to_end(&mut out).map_err(|e| format!("bad package: {e}"))?;
                    return Ok(out);
                }
            }
            Err(format!("{exe} is not in the package"))
        }
        Kind::Zip => {
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("bad package: {e}"))?;
            for i in 0..z.len() {
                let mut f = z.by_index(i).map_err(|e| format!("bad package: {e}"))?;
                let name = f.name().map(|n| n.replace('\\', "/")).unwrap_or_default();
                if f.is_file() && (name == exe || name.ends_with(&format!("/{exe}"))) {
                    let mut out = Vec::new();
                    f.read_to_end(&mut out).map_err(|e| format!("bad package: {e}"))?;
                    return Ok(out);
                }
            }
            Err(format!("{exe} is not in the package"))
        }
        Kind::Pkg => {
            let pkg = work.join("package.pkg");
            std::fs::write(&pkg, bytes).map_err(|e| e.to_string())?;
            let out = work.join("expanded");
            let ok = Command::new("pkgutil").arg("--expand-full").arg(&pkg).arg(&out).status().map(|s| s.success()).unwrap_or(false);
            if !ok {
                return Err("cannot open the .pkg (pkgutil)".into());
            }
            let f = find_file(&out, &exe).ok_or(format!("{exe} is not in the package"))?;
            std::fs::read(f).map_err(|e| e.to_string())
        }
        Kind::Msi => {
            let msi = work.join("package.msi");
            std::fs::write(&msi, bytes).map_err(|e| e.to_string())?;
            let out = work.join("expanded");
            let ok = Command::new("msiexec").arg("/a").arg(&msi).arg("/qn").arg(format!("TARGETDIR={}", out.display())).status().map(|s| s.success()).unwrap_or(false);
            if !ok {
                return Err("cannot open the .msi (msiexec)".into());
            }
            let f = find_file(&out, &exe).ok_or(format!("{exe} is not in the package"))?;
            std::fs::read(f).map_err(|e| e.to_string())
        }
    }
}

fn permission_hint(e: &std::io::Error, exe: &Path) -> String {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        format!("no permission to replace {}: run the installer again", exe.display())
    } else {
        format!("cannot replace {}: {e}", exe.display())
    }
}

/// Installs a package (blocking): checks the new program, keeps the current one as
/// `<bin>.previous` and puts the new one in its place. Returns the new version.
pub fn install_bytes(data_dir: &Path, bytes: &[u8], allow_older: bool) -> Result<String, String> {
    let u = updater();
    let work = dir(data_dir).join(format!("work-{}", paths::new_id()));
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let result = (|| {
        let program = extract(bytes, &work)?;
        let new = u.exe.with_file_name(format!("{}.new", exe_name()));
        std::fs::write(&new, &program).map_err(|e| permission_hint(&e, &u.exe))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755));
        }
        let version = match run_version(&new) {
            Ok(v) => v,
            Err(e) => {
                let _ = std::fs::remove_file(&new);
                return Err(e);
            }
        };
        if !allow_older && compare_versions(&version, CURRENT) != Ordering::Greater {
            let _ = std::fs::remove_file(&new);
            return Err(format!("version {version} is not newer than {CURRENT}"));
        }
        swap_in(&u.exe, &new)?;
        paths::write_json(&dir(data_dir).join("pending.json"), &Pending { from: CURRENT.into(), to: version.clone(), starts: 0 }, false).map_err(|e| e.to_string())?;
        add_history(data_dir, CURRENT, &version, "installed");
        Ok(version)
    })();
    let _ = std::fs::remove_dir_all(&work);
    if result.is_ok() {
        set_status(|s| {
            s.phase = Phase::Restarting;
            s.message = "restarting".into();
        });
    }
    result
}

/// current → `.previous`, new → current (atomic renames; Windows lets a running exe be renamed).
fn swap_in(exe: &Path, new: &Path) -> Result<(), String> {
    let prev = previous_path(exe);
    let _ = std::fs::remove_file(&prev);
    std::fs::rename(exe, &prev).map_err(|e| permission_hint(&e, exe))?;
    if let Err(e) = std::fs::rename(new, exe) {
        let _ = std::fs::rename(&prev, exe);
        return Err(permission_hint(&e, exe));
    }
    Ok(())
}

/// `POST /api/system/update/rollback` (blocking): back to `<bin>.previous`.
pub fn rollback(data_dir: &Path) -> Result<String, String> {
    let u = updater();
    let prev = previous_path(&u.exe);
    if !prev.exists() {
        return Err("there is no previous version".into());
    }
    let version = run_version(&prev)?;
    let tmp = u.exe.with_file_name(format!("{}.rollback", exe_name()));
    let _ = std::fs::remove_file(&tmp);
    std::fs::rename(&u.exe, &tmp).map_err(|e| permission_hint(&e, &u.exe))?;
    if let Err(e) = std::fs::rename(&prev, &u.exe) {
        let _ = std::fs::rename(&tmp, &u.exe);
        return Err(permission_hint(&e, &u.exe));
    }
    let _ = std::fs::rename(&tmp, &prev);
    let _ = std::fs::remove_file(dir(data_dir).join("pending.json"));
    add_history(data_dir, CURRENT, &version, "rolled back");
    set_status(|s| {
        s.phase = Phase::Restarting;
        s.message = "restarting".into();
    });
    Ok(version)
}

/// Restarts 800 ms later (the answer gets out first).
pub fn restart_soon(supervised: bool) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        restart_process(supervised);
    });
}

/// Restarts the process: under a service manager, exit 75 (it starts us again); in the
/// foreground, start the same program with the same arguments and leave.
pub fn restart_process(supervised: bool) {
    crate::health::stopped();
    if supervised {
        std::process::exit(75);
    }
    let exe = UPDATER.get().map(|u| u.exe.clone()).or_else(|| std::env::current_exe().ok());
    let Some(exe) = exe else { std::process::exit(75) };
    match Command::new(&exe).args(std::env::args_os().skip(1)).spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => {
            tracing::error!("cannot start {}: {e}", exe.display());
            set_status(|s| {
                s.phase = Phase::Error;
                s.message = format!("cannot restart: {e}");
            });
        }
    }
}

/// At start: counts the starts of a freshly installed version; after 3 failed starts, puts
/// `<bin>.previous` back and restarts.
pub fn startup_guard(data_dir: &Path, supervised: bool) {
    let u = updater();
    let path = dir(data_dir).join("pending.json");
    let Ok(Some(mut p)) = paths::read_json::<Pending>(&path) else { return };
    if p.to != CURRENT {
        let _ = std::fs::remove_file(&path);
        return;
    }
    p.starts += 1;
    if p.starts <= MAX_FAILED_STARTS {
        let _ = paths::write_json(&path, &p, false);
        return;
    }
    let prev = previous_path(&u.exe);
    if !prev.exists() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    tracing::error!("version {} failed to start {} times: going back to {}", p.to, MAX_FAILED_STARTS, p.from);
    let failed = u.exe.with_file_name(format!("{}.failed", exe_name()));
    let _ = std::fs::remove_file(&failed);
    if std::fs::rename(&u.exe, &failed).is_ok() && std::fs::rename(&prev, &u.exe).is_ok() {
        let _ = std::fs::remove_file(&path);
        add_history(data_dir, &p.to, &p.from, "restored automatically");
        restart_process(supervised);
    }
}

/// After `after` of running, the installed version is confirmed (`pending.json` removed).
pub fn confirm_after(data_dir: PathBuf, after: Duration) {
    tokio::spawn(async move {
        tokio::time::sleep(after).await;
        let path = dir(&data_dir).join("pending.json");
        if let Ok(Some(p)) = paths::read_json::<Pending>(&path) {
            if p.to == CURRENT {
                let _ = std::fs::remove_file(&path);
                tracing::info!("version {CURRENT} confirmed");
            }
        }
    });
}

/// Checks 30 s after start, then every 6 h, when allowed and nothing is running.
pub fn spawn_auto_check(data_dir: PathBuf) {
    updater();
    tokio::spawn(async move {
        tokio::time::sleep(AUTO_FIRST).await;
        loop {
            let d = data_dir.clone();
            if load_settings(&d).auto_check && !busy() {
                match tokio::task::spawn_blocking(move || check(&d)).await {
                    Ok(Ok(Some(r))) if compare_versions(&r.version, CURRENT) == Ordering::Greater => tracing::info!("version {} is available", r.version),
                    Ok(Err(e)) => tracing::info!("update check failed: {e}"),
                    _ => {}
                }
            }
            tokio::time::sleep(AUTO_EVERY).await;
        }
    });
}

/// For the launcher: `{available, phase, progress, message}`.
pub fn tray_status() -> Value {
    let s = status();
    let available = s.latest.as_ref().filter(|r| compare_versions(&r.version, CURRENT) == Ordering::Greater).map(|r| r.version.clone());
    json!({ "available": available, "phase": s.phase, "progress": s.progress, "message": s.message })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(compare_versions("1.2.10", "1.2.9"), Ordering::Greater);
        assert_eq!(compare_versions("v1.2.0", "1.2"), Ordering::Equal);
        assert_eq!(compare_versions("0.9.9", "1.0.0"), Ordering::Less);
        assert_eq!(compare_versions("2.0.0-rc1", "2.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("10.0.0", "9.99.99"), Ordering::Greater);
    }

    #[test]
    fn version_output() {
        assert_eq!(parse_version_output("strata-network 1.2.3\n").as_deref(), Some("1.2.3"));
        assert_eq!(parse_version_output("other 1.2.3"), None);
        assert_eq!(parse_version_output(""), None);
    }

    #[test]
    fn repos() {
        assert_eq!(parse_repo("DamienP2/Strata-Network").unwrap(), "DamienP2/Strata-Network");
        assert_eq!(parse_repo("https://github.com/DamienP2/Strata-Network.git").unwrap(), "DamienP2/Strata-Network");
        assert_eq!(parse_repo("github.com/a/b/").unwrap(), "a/b");
        assert!(parse_repo("nope").is_err());
        assert!(parse_repo("a/b/c").is_err());
        assert!(parse_repo("a b/c").is_err());
    }

    #[test]
    fn release_list() {
        let plat = platform();
        let ext = package_ext();
        let asset = |v: &str, state: &str| json!({"id": 7, "name": format!("Strata-{v}-{plat}.{ext}"), "size": 1000, "state": state, "digest": "sha256:00"});
        let list = json!([
            {"tag_name": "v1.2.9", "draft": false, "prerelease": false, "assets": [asset("1.2.9", "uploaded")]},
            {"tag_name": "v1.2.10", "draft": false, "prerelease": false, "assets": [asset("1.2.10", "uploaded")]},
            {"tag_name": "v1.3.0", "draft": false, "prerelease": true, "assets": [asset("1.3.0", "uploaded")]},
            {"tag_name": "v1.4.0", "draft": true, "prerelease": false, "assets": [asset("1.4.0", "uploaded")]},
            {"tag_name": "v1.5.0", "draft": false, "prerelease": false, "assets": [asset("1.5.0", "starter")]},
            {"tag_name": "v1.6.0", "draft": false, "prerelease": false, "assets": [{"id": 1, "name": "Strata-1.6.0-other-os.tar.gz", "state": "uploaded"}]},
        ]);
        let r = parse_releases(&list);
        assert_eq!(r.iter().map(|r| r.version.as_str()).collect::<Vec<_>>(), vec!["1.2.10", "1.2.9"]);
        assert_eq!(r[0].asset.id, 7);
    }

    #[test]
    fn package_kinds() {
        let work = std::env::temp_dir().join(format!("sn-upd-{}", paths::new_id()));
        std::fs::create_dir_all(&work).unwrap();
        // tar.gz holding <dir>/<bin>
        let mut tar_bytes = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut tar_bytes, flate2::Compression::fast());
            let mut b = tar::Builder::new(gz);
            let data = b"\x7fELF fake";
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            h.set_cksum();
            b.append_data(&mut h, format!("Strata-1.0.0-{}/{}", platform(), exe_name()), &data[..]).unwrap();
            b.into_inner().unwrap().finish().unwrap();
        }
        assert_eq!(extract(&tar_bytes, &work).unwrap(), b"\x7fELF fake");
        assert!(extract(b"hello", &work).is_err());
        assert_eq!(extract(b"\x7fELF raw", &work).unwrap(), b"\x7fELF raw");
        let _ = std::fs::remove_dir_all(work);
    }
}
