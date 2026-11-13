//! MAC addresses: reading, writing, vendor (IEEE registry) and the ARP table.

use std::collections::HashMap;
use std::io::Read;
use std::net::Ipv4Addr;
use std::sync::OnceLock;

pub type Mac = [u8; 6];

/// `00:1B:21:3A:4F:92`.
pub fn format(mac: &Mac) -> String {
    mac.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// Reads `00:1b:21:3a:4f:92`, `00-1B-21-3A-4F-92`, `0:1b:21:3a:4f:92` or `001b.213a.4f92`.
pub fn parse(s: &str) -> Option<Mac> {
    let s = s.trim();
    let parts: Vec<&str> = if s.contains(':') || s.contains('-') {
        s.split([':', '-']).collect()
    } else {
        let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if hex.len() != 12 {
            return None;
        }
        return (0..6).map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()).collect::<Option<Vec<_>>>()?.try_into().ok();
    };
    if parts.len() != 6 {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || p.len() > 2 {
            return None;
        }
        mac[i] = u8::from_str_radix(p, 16).ok()?;
    }
    Some(mac)
}

/// A locally administered address (random MAC of phones and laptops, virtual machines).
pub fn is_local(mac: &Mac) -> bool {
    mac[0] & 0x02 != 0
}

static OUI: &[u8] = include_bytes!("../data/oui.tsv.gz");

struct Vendors {
    /// 24-bit (MA-L) and 36-bit (MA-S) blocks.
    l: HashMap<u32, Box<str>>,
    s: HashMap<u64, Box<str>>,
}

fn vendors() -> &'static Vendors {
    static V: OnceLock<Vendors> = OnceLock::new();
    V.get_or_init(|| {
        let mut text = String::new();
        let _ = flate2::read::GzDecoder::new(OUI).read_to_string(&mut text);
        let mut v = Vendors { l: HashMap::with_capacity(40_000), s: HashMap::with_capacity(8_000) };
        for line in text.lines() {
            let Some((hex, name)) = line.split_once('\t') else { continue };
            match hex.len() {
                6 => {
                    if let Ok(k) = u32::from_str_radix(hex, 16) {
                        v.l.insert(k, name.into());
                    }
                }
                9 => {
                    if let Ok(k) = u64::from_str_radix(hex, 16) {
                        v.s.insert(k, name.into());
                    }
                }
                _ => {}
            }
        }
        v
    })
}

/// The maker of the network card, from the IEEE registry.
pub fn vendor(mac: &Mac) -> Option<&'static str> {
    if is_local(mac) {
        return None;
    }
    let v = vendors();
    let s = (u64::from(mac[0]) << 28) | (u64::from(mac[1]) << 20) | (u64::from(mac[2]) << 12) | (u64::from(mac[3]) << 4) | u64::from(mac[4] >> 4);
    if let Some(n) = v.s.get(&s) {
        return Some(n);
    }
    v.l.get(&((u32::from(mac[0]) << 16) | (u32::from(mac[1]) << 8) | u32::from(mac[2]))).map(|n| &**n)
}

/// The system's ARP table: the MAC of each neighbour that answered.
pub fn arp_table() -> HashMap<Ipv4Addr, Mac> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/net/arp").map(|t| parse_proc_arp(&t)).unwrap_or_default()
    }
    #[cfg(not(target_os = "linux"))]
    {
        std::process::Command::new("arp")
            .arg(if cfg!(windows) { "-a" } else { "-an" })
            .output()
            .map(|o| parse_arp_command(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default()
    }
}

/// `/proc/net/arp`: `IP address  HW type  Flags  HW address  Mask  Device`; flag 0x2 = complete.
pub fn parse_proc_arp(text: &str) -> HashMap<Ipv4Addr, Mac> {
    let mut out = HashMap::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 4 {
            continue;
        }
        let flags = u32::from_str_radix(f[2].trim_start_matches("0x"), 16).unwrap_or(0);
        if flags & 0x2 == 0 {
            continue;
        }
        if let (Ok(ip), Some(mac)) = (f[0].parse(), parse(f[3])) {
            if mac != [0; 6] {
                out.insert(ip, mac);
            }
        }
    }
    out
}

/// `arp -an` (macOS, BSD): `? (10.0.0.1) at 0:1a:2b:3c:4d:5e on en0 …`;
/// `arp -a` (Windows): `  10.0.0.1   00-1a-2b-3c-4d-5e   dynamic`.
pub fn parse_arp_command(text: &str) -> HashMap<Ipv4Addr, Mac> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let ip = words.iter().find_map(|w| w.trim_matches(|c| c == '(' || c == ')').parse::<Ipv4Addr>().ok());
        let mac = words.iter().find_map(|w| parse(w));
        if let (Some(ip), Some(mac)) = (ip, mac) {
            if mac != [0; 6] && mac != [0xff; 6] {
                out.insert(ip, mac);
            }
        }
    }
    out
}

/// Neighbours with their MAC and whether they answered recently. On Linux,
/// from `ip -4 neigh` (REACHABLE / PERMANENT count as live, STALE does not);
/// elsewhere the ARP table, all counted as live.
pub fn neighbours() -> HashMap<Ipv4Addr, (Mac, bool)> {
    #[cfg(target_os = "linux")]
    if let Ok(o) = std::process::Command::new("ip").args(["-4", "neigh", "show"]).output() {
        if o.status.success() {
            return parse_ip_neigh(&String::from_utf8_lossy(&o.stdout));
        }
    }
    arp_table().into_iter().map(|(ip, mac)| (ip, (mac, true))).collect()
}

/// `10.0.0.1 dev eth0 lladdr 00:11:32:ab:cd:ef REACHABLE`.
pub fn parse_ip_neigh(text: &str) -> HashMap<Ipv4Addr, (Mac, bool)> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        let Some(ip) = w.first().and_then(|s| s.parse().ok()) else { continue };
        let Some(mac) = w.iter().position(|x| *x == "lladdr").and_then(|i| w.get(i + 1)).and_then(|m| parse(m)) else { continue };
        let live = w.iter().any(|x| matches!(*x, "REACHABLE" | "PERMANENT" | "NOARP"));
        out.insert(ip, (mac, live));
    }
    out
}

/// The MAC of this machine's own interfaces (absent from the ARP table).
#[cfg(not(target_os = "linux"))]
pub fn own_macs() -> HashMap<Ipv4Addr, Mac> {
    HashMap::new()
}

/// The MAC of this machine's own interfaces (absent from the ARP table).
#[cfg(target_os = "linux")]
pub fn own_macs() -> HashMap<Ipv4Addr, Mac> {
    let mut out = HashMap::new();
    if let Ok(list) = if_addrs::get_if_addrs() {
        for i in list {
            if let std::net::IpAddr::V4(ip) = i.ip() {
                if let Some(mac) = std::fs::read_to_string(format!("/sys/class/net/{}/address", i.name)).ok().and_then(|s| parse(&s)) {
                    if mac != [0; 6] {
                        out.insert(ip, mac);
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes() {
        let m = [0x00, 0x1b, 0x21, 0x3a, 0x4f, 0x92];
        assert_eq!(parse("00:1b:21:3a:4f:92"), Some(m));
        assert_eq!(parse("00-1B-21-3A-4F-92"), Some(m));
        assert_eq!(parse("0:1b:21:3a:4f:92"), Some(m));
        assert_eq!(parse("001b.213a.4f92"), Some(m));
        assert_eq!(parse("00:1b:21:3a:4f"), None);
        assert_eq!(format(&m), "00:1B:21:3A:4F:92");
    }

    #[test]
    fn vendors_from_the_registry() {
        assert_eq!(vendor(&parse("00:11:32:01:02:03").unwrap()), Some("Synology"));
        assert_eq!(vendor(&parse("7C:2E:0D:00:00:01").unwrap()), Some("Blackmagic Design"));
        assert_eq!(vendor(&parse("02:11:32:01:02:03").unwrap()), None);
    }

    #[test]
    fn arp_tables() {
        let proc = "IP address       HW type     Flags       HW address            Mask     Device\n\
                    10.0.0.1         0x1         0x2         00:11:32:ab:cd:ef     *        eth0\n\
                    10.0.0.9         0x1         0x0         00:00:00:00:00:00     *        eth0\n";
        let t = parse_proc_arp(proc);
        assert_eq!(t.len(), 1);
        assert_eq!(t[&Ipv4Addr::new(10, 0, 0, 1)], [0x00, 0x11, 0x32, 0xab, 0xcd, 0xef]);
        let mac = "? (10.0.0.1) at 0:11:32:ab:cd:ef on en0 ifscope [ethernet]\n? (10.0.0.2) at (incomplete) on en0";
        assert_eq!(parse_arp_command(mac).len(), 1);
        let win = "  10.0.0.1          00-11-32-ab-cd-ef     dynamic\n  10.0.0.255        ff-ff-ff-ff-ff-ff     static";
        assert_eq!(parse_arp_command(win).len(), 1);
        let n = parse_ip_neigh("10.0.0.1 dev eth0 lladdr 00:11:32:ab:cd:ef REACHABLE\n10.0.0.2 dev eth0 lladdr 00:11:32:ab:cd:01 STALE\n10.0.0.3 dev eth0 FAILED\n");
        assert_eq!(n.len(), 2);
        assert!(n[&Ipv4Addr::new(10, 0, 0, 1)].1);
        assert!(!n[&Ipv4Addr::new(10, 0, 0, 2)].1);
    }
}
