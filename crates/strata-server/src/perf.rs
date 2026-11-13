//! Machine and process figures (CPU, memory), sampled every 2 s by a background thread, with
//! the last 5 minutes kept for the Performance page.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use sysinfo::{Disks, Pid, ProcessRefreshKind, ProcessesToUpdate, System};

const EVERY: Duration = Duration::from_secs(2);
const KEEP: usize = 150;

#[derive(Serialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub at: i64,
    /// Whole machine, percent 0..100.
    pub cpu: f32,
    pub mem_used: u64,
    /// This server process: percent of one core × cores (0..100 of the machine).
    pub proc_cpu: f32,
    pub proc_mem: u64,
}

struct Perf {
    last: Sample,
    history: VecDeque<Sample>,
    mem_total: u64,
    cores: usize,
    cpu_name: String,
}

static PERF: OnceLock<Mutex<Perf>> = OnceLock::new();

fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().to_string()
}

/// Starts the sampling thread (once).
pub fn start() {
    if PERF.get().is_some() {
        return;
    }
    let mut sys = System::new();
    sys.refresh_cpu_all();
    sys.refresh_memory();
    let cores = sys.cpus().len().max(1);
    let cpu_name = sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default();
    let perf = Perf { last: Sample::default(), history: VecDeque::with_capacity(KEEP), mem_total: sys.total_memory(), cores, cpu_name };
    if PERF.set(Mutex::new(perf)).is_err() {
        return;
    }
    let pid = Pid::from_u32(std::process::id());
    let _ = std::thread::Builder::new().name("perf".into()).spawn(move || loop {
        std::thread::sleep(EVERY);
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing().with_cpu().with_memory());
        let (proc_cpu, proc_mem) = sys.process(pid).map(|p| (p.cpu_usage() / cores as f32, p.memory())).unwrap_or((0.0, 0));
        let s = Sample { at: crate::paths::now_ms(), cpu: sys.global_cpu_usage(), mem_used: sys.used_memory(), proc_cpu, proc_mem };
        if let Some(p) = PERF.get() {
            let mut p = p.lock().unwrap();
            p.last = s;
            p.mem_total = sys.total_memory();
            if p.history.len() >= KEEP {
                p.history.pop_front();
            }
            p.history.push_back(s);
        }
    });
}

/// `GET /api/system/metrics`
pub fn metrics(with_history: bool) -> Value {
    let Some(p) = PERF.get() else {
        return json!({ "machine": { "cpu": 0, "cores": 0, "cpuName": "", "memUsed": 0, "memTotal": 0, "hostname": hostname() } });
    };
    let p = p.lock().unwrap();
    let mut v = json!({
        "machine": {
            "cpu": (p.last.cpu * 10.0).round() / 10.0,
            "cores": p.cores,
            "cpuName": p.cpu_name,
            "memUsed": p.last.mem_used,
            "memTotal": p.mem_total,
            "hostname": hostname(),
        },
        "process": {
            "cpu": (p.last.proc_cpu * 10.0).round() / 10.0,
            "mem": p.last.proc_mem,
            "pid": std::process::id(),
        },
    });
    if with_history {
        v["history"] = json!(p.history);
    }
    v
}

/// Size and free space of the disk holding `dir` (blocking).
pub fn disk_of(dir: &Path) -> Value {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let disks = Disks::new_with_refreshed_list();
    let best = disks.list().iter().filter(|d| dir.starts_with(d.mount_point())).max_by_key(|d| d.mount_point().as_os_str().len());
    match best {
        Some(d) => json!({ "total": d.total_space(), "free": d.available_space() }),
        None => json!({ "total": 0, "free": 0 }),
    }
}

pub fn cpu_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}
