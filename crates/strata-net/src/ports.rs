//! TCP ports: the default list, service names and the connect test.

use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::net::TcpStream;

/// Ports tested by default: common services, web consoles and broadcast equipment.
pub const DEFAULT_PORTS: &[u16] = &[
    21, 22, 23, 25, 53, 80, 81, 88, 110, 135, 139, 143, 389, 443, 445, 502, 554, 631, 873, 1883, 1935, 2000, 3000, 3306, 3389,
    4352, 4443, 5000, 5001, 5060, 5432, 5900, 5960, 7000, 8000, 8008, 8080, 8081, 8088, 8443, 8554, 8888, 9000, 9090, 9100,
    9443, 9910, 9993, 10000, 49152,
];

/// The usual service on a port.
pub fn service(port: u16) -> Option<&'static str> {
    Some(match port {
        21 => "ftp",
        22 => "ssh",
        23 => "telnet",
        25 => "smtp",
        53 => "dns",
        80 | 81 | 8000 | 8008 | 8080 | 8081 | 8088 | 8888 | 9000 | 9090 | 3000 | 5000 | 10000 => "http",
        88 => "kerberos",
        110 => "pop3",
        135 => "msrpc",
        139 => "netbios",
        143 => "imap",
        389 => "ldap",
        443 | 4443 | 5001 | 8443 | 9443 => "https",
        445 => "smb",
        502 => "modbus",
        554 | 8554 => "rtsp",
        631 => "ipp",
        873 => "rsync",
        1883 => "mqtt",
        1935 => "rtmp",
        2000 => "sccp",
        3306 => "mysql",
        3389 => "rdp",
        4352 => "pjlink",
        5060 => "sip",
        5432 => "postgres",
        5900 => "vnc",
        5960 => "ndi",
        7000 => "airplay",
        9100 => "jetdirect",
        9910 => "atem",
        9993 => "videohub",
        49152 => "upnp",
        _ => return None,
    })
}

/// Ports that never speak HTTP: not worth a web probe.
pub fn surely_not_web(port: u16) -> bool {
    matches!(port, 21 | 22 | 23 | 25 | 53 | 110 | 135 | 139 | 143 | 389 | 445 | 502 | 873 | 1883 | 1935 | 3306 | 3389 | 5432 | 5900 | 9100 | 9910 | 9993)
}

/// What a TCP connection attempt says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// The port accepted the connection.
    Open,
    /// The machine refused: it exists, the port is closed.
    Refused,
    /// No answer (filtered, or nobody there).
    Silent,
}

pub async fn connect(ip: Ipv4Addr, port: u16, timeout: Duration) -> Probe {
    match tokio::time::timeout(timeout, TcpStream::connect(SocketAddr::from((ip, port)))).await {
        Ok(Ok(_)) => Probe::Open,
        Ok(Err(e)) if e.kind() == ErrorKind::ConnectionRefused => Probe::Refused,
        _ => Probe::Silent,
    }
}

/// Reads a port list: `22, 80, 8000-8010`. At most 4096 ports.
pub fn parse_list(s: &str) -> Result<Vec<u16>, String> {
    let mut out = Vec::new();
    for part in s.split([',', ' ', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = part.split_once('-').unwrap_or((part, part));
        let a: u16 = a.trim().parse().ok().filter(|p| *p > 0).ok_or_else(|| format!("“{part}” is not a port"))?;
        let b: u16 = b.trim().parse().ok().filter(|p| *p >= a).ok_or_else(|| format!("“{part}” is not a port range"))?;
        out.extend(a..=b);
        if out.len() > 4096 {
            return Err("at most 4096 ports".into());
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_lists() {
        assert_eq!(parse_list("80, 22 8000-8002").unwrap(), vec![22, 80, 8000, 8001, 8002]);
        assert!(parse_list("0").is_err());
        assert!(parse_list("90-80").is_err());
        assert!(parse_list("1-65535").is_err());
    }

    #[tokio::test]
    async fn connect_says_open_and_refused() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let lo = Ipv4Addr::LOCALHOST;
        assert_eq!(connect(lo, port, Duration::from_secs(1)).await, Probe::Open);
        drop(l);
        assert_eq!(connect(lo, port, Duration::from_secs(1)).await, Probe::Refused);
    }
}
