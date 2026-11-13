//! The log in memory: a `tracing` layer that keeps the last 5000 lines.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

pub const KEEP: usize = 5000;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub seq: u64,
    /// Milliseconds since the epoch.
    pub at: i64,
    pub level: &'static str,
    pub target: String,
    pub message: String,
    /// "k=v k=v"
    pub fields: String,
}

struct Lines {
    next: u64,
    list: VecDeque<Entry>,
}

fn lines() -> &'static Mutex<Lines> {
    static LINES: OnceLock<Mutex<Lines>> = OnceLock::new();
    LINES.get_or_init(|| Mutex::new(Lines { next: 1, list: VecDeque::with_capacity(KEEP) }))
}

static FILE: OnceLock<PathBuf> = OnceLock::new();

/// The log file when the server writes one (as a service).
pub fn set_file(path: PathBuf) {
    let _ = FILE.set(path);
}

pub fn file() -> Option<String> {
    FILE.get().map(|p| p.display().to_string())
}

/// The layer to add to the subscriber.
pub struct Ring;

#[derive(Default)]
struct Fields {
    message: String,
    rest: String,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.push(field.name(), value);
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.push(field.name(), &format!("{value:?}"));
        }
    }
}

impl Fields {
    fn push(&mut self, k: &str, v: &str) {
        if k.starts_with("log.") {
            return;
        }
        if !self.rest.is_empty() {
            self.rest.push(' ');
        }
        let _ = write!(self.rest, "{k}={v}");
    }
}

pub fn level_str(l: &Level) -> &'static str {
    match *l {
        Level::ERROR => "ERROR",
        Level::WARN => "WARN",
        Level::INFO => "INFO",
        Level::DEBUG => "DEBUG",
        Level::TRACE => "TRACE",
    }
}

/// ERROR = 5 … TRACE = 1, for "at least" filters.
pub fn level_rank(l: &str) -> u8 {
    match l.to_ascii_uppercase().as_str() {
        "ERROR" => 5,
        "WARN" | "WARNING" => 4,
        "INFO" => 3,
        "DEBUG" => 2,
        "TRACE" => 1,
        _ => 0,
    }
}

impl<S: Subscriber> Layer<S> for Ring {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut f = Fields::default();
        event.record(&mut f);
        let level = level_str(meta.level());
        let at = crate::paths::now_ms();
        let target = meta.target().to_string();
        if *meta.level() <= Level::WARN {
            crate::health::record(level, &target, &f.message, &f.rest, at);
        }
        let mut l = lines().lock().unwrap();
        let seq = l.next;
        l.next += 1;
        if l.list.len() >= KEEP {
            l.list.pop_front();
        }
        l.list.push_back(Entry { seq, at, level, target, message: f.message, fields: f.rest });
    }
}

/// Lines after `after`, of at least `level`, the last `limit` of them; and the last seq.
pub fn query(after: u64, level: Option<&str>, limit: usize) -> (Vec<Entry>, u64) {
    let min = level.map(level_rank).unwrap_or(0);
    let l = lines().lock().unwrap();
    let last = l.next - 1;
    let mut out: Vec<Entry> = l.list.iter().filter(|e| e.seq > after && level_rank(e.level) >= min).cloned().collect();
    if out.len() > limit {
        out.drain(..out.len() - limit);
    }
    (out, last)
}

fn fmt_time(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms).map(|t| t.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()).unwrap_or_default()
}

/// One line of text per entry.
pub fn format_line(e: &Entry) -> String {
    let mut s = format!("{} {:5} {}: {}", fmt_time(e.at), e.level, e.target, e.message);
    if !e.fields.is_empty() {
        s.push(' ');
        s.push_str(&e.fields);
    }
    s
}

/// The whole log in memory as text.
pub fn as_text() -> String {
    let l = lines().lock().unwrap();
    let mut out = String::with_capacity(l.list.len() * 100);
    for e in &l.list {
        out.push_str(&format_line(e));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks() {
        assert!(level_rank("ERROR") > level_rank("WARN"));
        assert!(level_rank("warn") > level_rank("INFO"));
        assert_eq!(level_rank("nope"), 0);
    }
}
