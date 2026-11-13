//! Scanning a range: every address is tested by ICMP, TCP (an open port or a
//! refusal both prove a machine is there) and, on the local network, ARP.
//! Machines found are then named, their MAC and maker read, and their open
//! ports examined (web page and title, banner).

use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::cidr::Range;
use crate::mac;
use crate::names::{self, Source};
use crate::ping::Pinger;
use crate::ports::{self, Probe};
use crate::probe::{self, Web};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    /// TCP ports tested on every address.
    pub ports: Vec<u16>,
    /// Wait for one answer (ping, connection), in milliseconds.
    pub timeout_ms: u64,
    /// Addresses tested at the same time.
    pub parallel: usize,
    pub ping: bool,
    pub names: bool,
    /// Look for web pages and banners on open ports.
    pub services: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { ports: ports::DEFAULT_PORTS.to_vec(), timeout_ms: 600, parallel: 64, ping: true, names: true, services: true }
    }
}

/// How the machine showed it was there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Via {
    Icmp,
    Tcp,
    Arp,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Port {
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<Web>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner: Option<String>,
}

/// A machine that answered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub ip: Ipv4Addr,
    pub via: Via,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_source: Option<Source>,
    #[serde(default)]
    pub ports: Vec<Port>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Event {
    /// A machine answered (sent once it is fully examined).
    Host(Host),
    Progress { done: u64, total: u64 },
    /// The scan is over: every address of the range not reported is free.
    Done { up: u64, total: u64, secs: f32, stopped: bool, icmp: bool },
}

/// Stops a scan from another task.
#[derive(Clone, Default)]
pub struct Stop(Arc<AtomicBool>);

impl Stop {
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn stopped(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// How many sockets a scan may hold open at once (bounded by the file limit).
fn socket_budget() -> usize {
    #[cfg(unix)]
    {
        crate::raise_file_limit().saturating_sub(64).clamp(64, 4096) / 2
    }
    #[cfg(not(unix))]
    {
        1024
    }
}

/// Scans `range`; sends events on `tx` until `Done`.
pub async fn run(range: Range, opts: Options, tx: mpsc::UnboundedSender<Event>, stop: Stop) {
    let started = Instant::now();
    let timeout = Duration::from_millis(opts.timeout_ms.clamp(100, 10_000));
    let pinger = if opts.ping { Pinger::new().map(Arc::new) } else { None };
    let icmp = pinger.is_some();
    let total = range.len();
    let done = Arc::new(AtomicU64::new(0));
    let up = Arc::new(AtomicU64::new(0));
    let parallel = (socket_budget() / opts.ports.len().max(1)).clamp(1, opts.parallel.clamp(1, 512));
    let opts = Arc::new(opts);
    let found = Arc::new(std::sync::Mutex::new(HashSet::new()));
    let own = Arc::new(mac::own_macs());

    stream::iter(range.iter())
        .for_each_concurrent(parallel, |ip| {
            let (pinger, tx, stop, opts) = (pinger.clone(), tx.clone(), stop.clone(), opts.clone());
            let (done, up, found, own) = (done.clone(), up.clone(), found.clone(), own.clone());
            async move {
                if stop.stopped() {
                    return;
                }
                if let Some(mut host) = test(ip, &opts, pinger.as_deref(), timeout).await {
                    found.lock().unwrap().insert(ip);
                    up.fetch_add(1, Ordering::Relaxed);
                    examine(&mut host, &opts, timeout, &own).await;
                    let _ = tx.send(Event::Host(host));
                }
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                let _ = tx.send(Event::Progress { done: d, total });
            }
        })
        .await;

    // Machines that answered ARP only (everything else blocked).
    if !stop.stopped() {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let seen = found.lock().unwrap().clone();
        let quiet: Vec<_> = mac::neighbours().into_iter().filter(|(ip, (_, live))| *live && range.contains(*ip) && !seen.contains(ip)).collect();
        for (ip, _) in quiet {
            let mut host = Host { ip, via: Via::Arp, rtt_ms: None, mac: None, vendor: None, name: None, name_source: None, ports: vec![] };
            examine(&mut host, &opts, timeout, &own).await;
            up.fetch_add(1, Ordering::Relaxed);
            let _ = tx.send(Event::Host(host));
        }
    }
    let _ = tx.send(Event::Done {
        up: up.load(Ordering::Relaxed),
        total,
        secs: started.elapsed().as_secs_f32(),
        stopped: stop.stopped(),
        icmp,
    });
}

/// Is a machine at `ip`? Ping and the TCP ports at the same time.
async fn test(ip: Ipv4Addr, opts: &Options, pinger: Option<&Pinger>, timeout: Duration) -> Option<Host> {
    let ping = async {
        match pinger {
            Some(p) => p.ping(ip, timeout, 2).await,
            None => None,
        }
    };
    let tcp = stream::iter(opts.ports.iter().copied())
        .map(|port| async move { (port, ports::connect(ip, port, timeout).await) })
        .buffer_unordered(opts.ports.len().max(1))
        .collect::<Vec<_>>();
    let (rtt, results) = tokio::join!(ping, tcp);
    let mut open: Vec<u16> = results.iter().filter(|(_, r)| *r == Probe::Open).map(|(p, _)| *p).collect();
    open.sort_unstable();
    let refused = results.iter().any(|(_, r)| *r == Probe::Refused);
    let via = if rtt.is_some() {
        Via::Icmp
    } else if !open.is_empty() || refused {
        Via::Tcp
    } else {
        return None;
    };
    Some(Host {
        ip,
        via,
        rtt_ms: rtt.map(|d| (d.as_secs_f32() * 10_000.0).round() / 10.0),
        mac: None,
        vendor: None,
        name: None,
        name_source: None,
        ports: open.into_iter().map(|port| Port { port, service: ports::service(port).map(Into::into), ..Port::default() }).collect(),
    })
}

/// Name, MAC, maker, and what each open port serves.
async fn examine(host: &mut Host, opts: &Options, timeout: Duration, own: &std::collections::HashMap<Ipv4Addr, mac::Mac>) {
    let ip = host.ip;
    let neighbour = mac::neighbours().get(&ip).map(|(m, _)| *m).or_else(|| own.get(&ip).copied());
    if let Some(m) = neighbour {
        host.mac = Some(mac::format(&m));
        host.vendor = mac::vendor(&m).map(Into::into);
    }
    let names = async {
        if opts.names {
            names::lookup(ip, timeout).await
        } else {
            None
        }
    };
    let services = async {
        if !opts.services {
            return;
        }
        stream::iter(host.ports.iter_mut())
            .for_each_concurrent(8, |p| async move {
                if !ports::surely_not_web(p.port) {
                    p.web = probe::web(ip, p.port, timeout).await;
                }
                if p.web.is_none() && matches!(p.port, 21 | 22 | 23 | 25 | 110 | 143) {
                    p.banner = probe::banner(ip, p.port, timeout).await;
                }
                if let Some(w) = &p.web {
                    p.service = Some(w.scheme.clone());
                }
            })
            .await;
    };
    let (name, ()) = tokio::join!(names, services);
    if let Some((n, src)) = name {
        host.name = Some(n);
        host.name_source = Some(src);
    }
}
