//! Health: a journal of warnings and errors on disk (30 days), crash reports (90 days), and the
//! summary of the Health page.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::paths;

const JOURNAL_DAYS: i64 = 30;
const CRASH_DAYS: i64 = 90;
/// A day's journal stops growing past this size (an error loop must not fill the disk).
const JOURNAL_MAX_BYTES: u64 = 20 * 1024 * 1024;

struct Health {
    dir: PathBuf,
    write: Mutex<()>,
}

static HEALTH: OnceLock<Health> = OnceLock::new();
static START: OnceLock<(Instant, i64)> = OnceLock::new();

fn start_time() -> &'static (Instant, i64) {
    START.get_or_init(|| (Instant::now(), paths::now_ms()))
}

pub fn uptime_secs() -> u64 {
    start_time().0.elapsed().as_secs()
}

pub fn started_at_ms() -> i64 {
    start_time().1
}

fn day_of(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.to_zoned(jiff::tz::TimeZone::system()).date().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct JournalLine {
    pub at: i64,
    pub level: String,
    pub target: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fields: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Crash {
    pub id: String,
    pub at: i64,
    /// "panic" | "unexpectedStop"
    pub kind: String,
    pub version: String,
    pub message: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Running {
    pid: u32,
    version: String,
    started_at: i64,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Seen {
    crash_seen: i64,
}

/// Starts the journal: prunes old files, reports an unclean previous stop, catches panics.
pub fn start(data_dir: &Path) {
    let _ = start_time();
    let dir = data_dir.join("health");
    let _ = std::fs::create_dir_all(dir.join("crashes"));
    if HEALTH.set(Health { dir: dir.clone(), write: Mutex::new(()) }).is_err() {
        return;
    }
    prune(&dir);
    // The previous run did not stop by itself?
    let marker = dir.join("running.json");
    if let Ok(Some(prev)) = paths::read_json::<Running>(&marker) {
        let reported = list_crashes(&dir).iter().any(|c| c.at >= prev.started_at);
        if !reported {
            write_crash(
                "unexpectedStop",
                "the server stopped unexpectedly",
                &format!(
                    "version {} (pid {}) started {} and ended without a clean stop (power cut, killed, or crash)",
                    prev.version,
                    prev.pid,
                    fmt_ms(prev.started_at)
                ),
            );
        }
    }
    let _ = paths::write_json(&marker, &Running { pid: std::process::id(), version: env!("CARGO_PKG_VERSION").into(), started_at: started_at_ms() }, false);
    append(&JournalLine { at: paths::now_ms(), level: "INFO".into(), target: "server".into(), message: format!("started version {}", env!("CARGO_PKG_VERSION")), fields: String::new() });

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "panic".to_string()
        };
        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let bt = std::backtrace::Backtrace::force_capture();
        write_crash("panic", &message, &format!("thread '{thread}' at {location}\n\n{bt}"));
        previous(info);
    }));
}

/// Marks a clean stop (not counted as a crash).
pub fn stopped() {
    if let Some(h) = HEALTH.get() {
        append(&JournalLine { at: paths::now_ms(), level: "INFO".into(), target: "server".into(), message: "stopped".into(), fields: String::new() });
        let _ = std::fs::remove_file(h.dir.join("running.json"));
    }
}

fn fmt_ms(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms).map(|t| t.to_string()).unwrap_or_default()
}

fn prune(dir: &Path) {
    let now = paths::now_ms();
    let day_ms = 24 * 3600 * 1000;
    let cut_journal = day_of(now - JOURNAL_DAYS * day_ms);
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(day) = name.strip_prefix("journal-").and_then(|n| n.strip_suffix(".jsonl")) {
                if day < cut_journal.as_str() {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
    for c in list_crashes(dir) {
        if c.at < now - CRASH_DAYS * day_ms {
            let _ = std::fs::remove_file(dir.join("crashes").join(format!("{}.json", c.id)));
        }
    }
}

fn append(line: &JournalLine) {
    let Some(h) = HEALTH.get() else { return };
    let Ok(_w) = h.write.lock() else { return };
    write_line(h, line);
}

/// From the panic hook: never waits for the lock (the panic may come from inside `append`).
fn append_nowait(line: &JournalLine) {
    let Some(h) = HEALTH.get() else { return };
    let Ok(_w) = h.write.try_lock() else { return };
    write_line(h, line);
}

fn write_line(h: &Health, line: &JournalLine) {
    let path = h.dir.join(format!("journal-{}.jsonl", day_of(line.at)));
    if std::fs::metadata(&path).map(|m| m.len() > JOURNAL_MAX_BYTES).unwrap_or(false) {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        if let Ok(mut s) = serde_json::to_string(line) {
            s.push('\n');
            let _ = f.write_all(s.as_bytes());
        }
    }
}

/// Called by the log layer for each warning and error.
pub fn record(level: &str, target: &str, message: &str, fields: &str, at: i64) {
    append(&JournalLine { at, level: level.into(), target: target.into(), message: message.into(), fields: fields.into() });
}

fn write_crash(kind: &str, message: &str, detail: &str) {
    let Some(h) = HEALTH.get() else { return };
    let at = paths::now_ms();
    let stamp = jiff::Timestamp::from_millisecond(at).map(|t| t.strftime("%Y%m%d-%H%M%S").to_string()).unwrap_or_default();
    let id = format!("{stamp}-{}", &paths::new_id()[..6]);
    let crash = Crash { id: id.clone(), at, kind: kind.into(), version: env!("CARGO_PKG_VERSION").into(), message: message.into(), detail: detail.into() };
    let _ = paths::write_json(&h.dir.join("crashes").join(format!("{id}.json")), &crash, false);
    append_nowait(&JournalLine { at, level: "CRASH".into(), target: "crash".into(), message: message.into(), fields: format!("crash={id}") });
}

fn list_crashes(dir: &Path) -> Vec<Crash> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir.join("crashes")) {
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x == "json") {
                if let Ok(Some(c)) = paths::read_json::<Crash>(&e.path()) {
                    out.push(c);
                }
            }
        }
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.at));
    out
}

fn read_journal(dir: &Path, days: i64) -> Vec<JournalLine> {
    let cut = day_of(paths::now_ms() - days * 24 * 3600 * 1000);
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    n.strip_prefix("journal-").and_then(|n| n.strip_suffix(".jsonl")).is_some_and(|d| d >= cut.as_str())
                })
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        if let Ok(file) = std::fs::File::open(&f) {
            for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
                if let Ok(l) = serde_json::from_str::<JournalLine>(&line) {
                    out.push(l);
                }
            }
        }
    }
    out
}

/// The journal of the last `days` days, newest first, `limit` lines at most.
pub fn journal(days: i64, limit: usize) -> Vec<JournalLine> {
    let Some(h) = HEALTH.get() else { return Vec::new() };
    let mut lines = read_journal(&h.dir, days);
    lines.reverse();
    lines.truncate(limit);
    lines
}

fn category(target: &str) -> String {
    let mut parts = target.split("::");
    let krate = parts.next().unwrap_or("");
    if krate == "strata_server" || krate == "strata_network" {
        return parts.next().unwrap_or("server").to_string();
    }
    match krate {
        "tower_http" | "hyper" | "hyper_util" | "axum" => "http".into(),
        "" => "server".into(),
        other => other.to_string(),
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Incident {
    /// "crash" | "log"
    pub kind: &'static str,
    /// "CRASH" | "ERROR" | "WARN"
    pub level: String,
    pub cat: String,
    pub title: String,
    pub detail: String,
    pub first: i64,
    pub last: i64,
    pub count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crash: Option<String>,
}

/// Groups the same message with its count.
pub fn incidents(lines: &[JournalLine], crashes: &[Crash]) -> Vec<Incident> {
    let mut map: HashMap<(String, String, String), Incident> = HashMap::new();
    for l in lines.iter().filter(|l| l.level == "ERROR" || l.level == "WARN") {
        let key = (l.level.clone(), l.target.clone(), l.message.clone());
        let e = map.entry(key).or_insert_with(|| Incident {
            kind: "log",
            level: l.level.clone(),
            cat: category(&l.target),
            title: l.message.clone(),
            detail: l.fields.clone(),
            first: l.at,
            last: l.at,
            count: 0,
            crash: None,
        });
        e.count += 1;
        e.first = e.first.min(l.at);
        if l.at >= e.last {
            e.last = l.at;
            e.detail = l.fields.clone();
        }
    }
    let mut out: Vec<Incident> = map.into_values().collect();
    for c in crashes {
        out.push(Incident {
            kind: "crash",
            level: "CRASH".into(),
            cat: "server".into(),
            title: c.message.clone(),
            detail: c.detail.clone(),
            first: c.at,
            last: c.at,
            count: 1,
            crash: Some(c.id.clone()),
        });
    }
    out.sort_by_key(|i| std::cmp::Reverse(i.last));
    out.truncate(300);
    out
}

/// `GET /api/system/health` (blocking: reads files).
pub fn summary(disk: Value) -> Value {
    let Some(h) = HEALTH.get() else { return json!({}) };
    let lines = read_journal(&h.dir, JOURNAL_DAYS);
    let crashes = list_crashes(&h.dir);
    let seen = paths::read_json::<Seen>(&h.dir.join("seen.json")).ok().flatten().unwrap_or_default();
    let now = paths::now_ms();
    let errors = lines.iter().filter(|l| l.level == "ERROR").count();
    let warnings = lines.iter().filter(|l| l.level == "WARN").count();
    let last_hour = lines.iter().filter(|l| l.level == "ERROR" && l.at > now - 3600 * 1000).count();
    let last_crash = crashes.first().map(|c| c.at);
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "startedAt": started_at_ms(),
        "uptimeSecs": uptime_secs(),
        "crashes": crashes.len(),
        "lastCrash": last_crash,
        "unseenCrash": last_crash.is_some_and(|at| at > seen.crash_seen),
        "errors": errors,
        "warnings": warnings,
        "lastHourErrors": last_hour,
        "disk": disk,
        "incidents": incidents(&lines, &crashes),
    })
}

/// `POST /api/system/health/seen`
pub fn mark_seen() -> std::io::Result<()> {
    let Some(h) = HEALTH.get() else { return Ok(()) };
    paths::write_json(&h.dir.join("seen.json"), &Seen { crash_seen: paths::now_ms() }, false)
}

/// Everything for the support, as a zip (blocking).
pub fn export_zip(data_dir: &Path, extra: &[(&str, Vec<u8>)]) -> std::io::Result<Vec<u8>> {
    use zip::write::SimpleFileOptions;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let add = |zip: &mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>, name: &str, bytes: &[u8]| -> std::io::Result<()> {
        zip.start_file(name, opts).map_err(std::io::Error::other)?;
        zip.write_all(bytes)
    };
    for (name, bytes) in extra {
        add(&mut zip, name, bytes)?;
    }
    let health = data_dir.join("health");
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for sub in ["", "crashes"] {
        if let Ok(rd) = std::fs::read_dir(health.join(sub)) {
            for e in rd.flatten() {
                if e.path().is_file() {
                    let rel = if sub.is_empty() { format!("health/{}", e.file_name().to_string_lossy()) } else { format!("health/{sub}/{}", e.file_name().to_string_lossy()) };
                    files.push((rel, e.path()));
                }
            }
        }
    }
    for rel in ["update/history.json", "update/pending.json", "runtime.json", "logs/strata-network.log"] {
        let p = data_dir.join(rel);
        if p.is_file() {
            files.push((rel.to_string(), p));
        }
    }
    files.sort();
    for (rel, p) in files {
        if let Ok(bytes) = std::fs::read(&p) {
            add(&mut zip, &rel, &bytes)?;
        }
    }
    let cursor = zip.finish().map_err(std::io::Error::other)?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(level: &str, msg: &str, at: i64) -> JournalLine {
        JournalLine { at, level: level.into(), target: "strata_server::update".into(), message: msg.into(), fields: String::new() }
    }

    #[test]
    fn groups_incidents() {
        let lines = vec![line("ERROR", "boom", 10), line("ERROR", "boom", 30), line("WARN", "slow", 20), line("INFO", "hi", 40)];
        let crashes = vec![Crash { id: "c1".into(), at: 25, kind: "panic".into(), version: "0.1.0".into(), message: "oops".into(), detail: String::new() }];
        let inc = incidents(&lines, &crashes);
        assert_eq!(inc.len(), 3);
        assert_eq!(inc[0].title, "boom");
        assert_eq!(inc[0].count, 2);
        assert_eq!(inc[0].first, 10);
        assert_eq!(inc[0].cat, "update");
        assert_eq!(inc[1].level, "CRASH");
        assert_eq!(inc[1].crash.as_deref(), Some("c1"));
    }

    #[test]
    fn categories() {
        assert_eq!(category("strata_server::sso"), "sso");
        assert_eq!(category("strata_network"), "server");
        assert_eq!(category("tower_http::trace"), "http");
    }
}
