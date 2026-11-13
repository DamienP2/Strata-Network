//! The network engine of Strata Network: finding machines on IPv4 ranges
//! (ICMP, TCP, ARP), naming them (DNS, mDNS, NetBIOS), reading their MAC and
//! maker, looking at what their open ports serve, and listening to
//! multicast streams.

pub mod cidr;
pub mod mac;
pub mod mcast;
pub mod names;
pub mod ping;
pub mod ports;
pub mod probe;
pub mod scan;

/// Raises the open-file limit to its hard maximum (scans hold many sockets);
/// returns the limit now in force.
#[cfg(unix)]
pub fn raise_file_limit() -> usize {
    use std::sync::OnceLock;
    static LIMIT: OnceLock<usize> = OnceLock::new();
    *LIMIT.get_or_init(|| unsafe {
        let mut r = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut r) != 0 {
            return 1024;
        }
        // macOS refuses more than OPEN_MAX for the soft limit.
        let want = if cfg!(target_os = "macos") { r.rlim_max.min(10_240) } else { r.rlim_max };
        if want > r.rlim_cur {
            let n = libc::rlimit { rlim_cur: want, rlim_max: r.rlim_max };
            if libc::setrlimit(libc::RLIMIT_NOFILE, &n) == 0 {
                r.rlim_cur = want;
            }
        }
        usize::try_from(r.rlim_cur).unwrap_or(usize::MAX).min(1 << 20)
    })
}
