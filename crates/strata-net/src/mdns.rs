//! Service discovery by mDNS / DNS-SD: one question sent to 224.0.0.251,
//! answers collected for a moment (NDI sources, Dante devices, AES67/RAVENNA
//! sessions, web consoles, printers…). Light: a single small packet per type.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;

use crate::names::{dns_query, read_name};

pub const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

/// Service types looked for, with what they are.
pub const TYPES: &[(&str, &str)] = &[
    ("_ndi._tcp.local", "NDI"),
    ("_netaudio-arc._udp.local", "Dante"),
    ("_ravenna_session._sub._rtsp._tcp.local", "AES67 / RAVENNA"),
    ("_ravenna._sub._http._tcp.local", "RAVENNA"),
    ("_nmos-node._tcp.local", "NMOS"),
    ("_nmos-register._tcp.local", "NMOS"),
    ("_http._tcp.local", "Web"),
    ("_ipp._tcp.local", "Printer"),
    ("_smb._tcp.local", "SMB"),
    ("_ssh._tcp.local", "SSH"),
    ("_airplay._tcp.local", "AirPlay"),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    /// `NDI`, `Dante`…
    pub kind: String,
    /// The instance name (`VMIX-01 (Output 1)`).
    pub name: String,
    pub host: String,
    pub ip: Option<Ipv4Addr>,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub txt: Vec<String>,
}

#[derive(Default)]
struct Records {
    ptr: Vec<(String, String)>,
    srv: HashMap<String, (String, u16)>,
    a: HashMap<String, Ipv4Addr>,
    txt: HashMap<String, Vec<String>>,
}

/// Reads every record of a DNS message into `r`.
fn collect(msg: &[u8], r: &mut Records) -> Option<()> {
    let count = |i: usize| u16::from_be_bytes([msg[i], msg[i + 1]]) as usize;
    if msg.len() < 12 {
        return None;
    }
    let (qd, records) = (count(4), count(6) + count(8) + count(10));
    let mut pos = 12;
    for _ in 0..qd {
        pos = read_name(msg, pos)?.1 + 4;
    }
    for _ in 0..records {
        let (name, p) = read_name(msg, pos)?;
        let typ = u16::from_be_bytes([*msg.get(p)?, *msg.get(p + 1)?]);
        let len = u16::from_be_bytes([*msg.get(p + 8)?, *msg.get(p + 9)?]) as usize;
        let data = p + 10;
        let rd = msg.get(data..data + len)?;
        let key = name.to_ascii_lowercase();
        match typ {
            12 => r.ptr.push((key, read_name(msg, data)?.0)),
            33 if len >= 6 => {
                let port = u16::from_be_bytes([rd[4], rd[5]]);
                r.srv.insert(key, (read_name(msg, data + 6)?.0, port));
            }
            1 if len == 4 => {
                r.a.insert(key, Ipv4Addr::new(rd[0], rd[1], rd[2], rd[3]));
            }
            16 => {
                let mut t = Vec::new();
                let mut i = 0;
                while i < rd.len() {
                    let l = rd[i] as usize;
                    if let Some(s) = rd.get(i + 1..i + 1 + l) {
                        if !s.is_empty() {
                            t.push(String::from_utf8_lossy(s).into_owned());
                        }
                    }
                    i += 1 + l;
                }
                r.txt.insert(key, t);
            }
            _ => {}
        }
        pos = data + len;
    }
    Some(())
}

fn services(r: &Records) -> Vec<Service> {
    let mut out = Vec::new();
    for (ty, instance) in &r.ptr {
        let Some(kind) = TYPES.iter().find(|(t, _)| t.eq_ignore_ascii_case(ty)).map(|(_, k)| *k) else { continue };
        let key = instance.to_ascii_lowercase();
        let (host, port) = r.srv.get(&key).cloned().unwrap_or_default();
        let name = instance.strip_suffix(&format!(".{ty}")).or_else(|| instance.split_once("._").map(|(n, _)| n)).unwrap_or(instance);
        out.push(Service {
            kind: kind.to_string(),
            name: name.replace("\\032", " ").replace("\\.", "."),
            ip: r.a.get(&host.to_ascii_lowercase()).copied(),
            host: host.trim_end_matches('.').to_string(),
            port,
            txt: r.txt.get(&key).cloned().unwrap_or_default(),
        });
    }
    out.sort_by(|a, b| (&a.kind, &a.name).cmp(&(&b.kind, &b.name)));
    out.dedup();
    out
}

/// Asks the network of interface `iface` for the known service types and
/// listens `wait` for answers.
pub async fn browse(iface: Ipv4Addr, wait: Duration) -> std::io::Result<Vec<Service>> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_multicast_if_v4(&iface)?;
    s.set_multicast_ttl_v4(255)?;
    s.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)).into())?;
    s.set_nonblocking(true)?;
    let sock = UdpSocket::from_std(s.into())?;
    // A query from a port other than 5353 gets direct (unicast) answers.
    for (ty, _) in TYPES {
        let _ = sock.send_to(&dns_query(0, ty, 12), (GROUP, 5353)).await;
    }
    let mut r = Records::default();
    let mut buf = vec![0u8; 9000];
    let deadline = tokio::time::Instant::now() + wait;
    while let Ok(Ok((n, _))) = tokio::time::timeout_at(deadline, sock.recv_from(&mut buf)).await {
        collect(&buf[..n], &mut r);
    }
    Ok(services(&r))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(out: &mut Vec<u8>, n: &str) {
        for l in n.split('.') {
            out.push(l.len() as u8);
            out.extend_from_slice(l.as_bytes());
        }
        out.push(0);
    }

    fn record(out: &mut Vec<u8>, owner: &str, typ: u16, data: &[u8]) {
        name(out, owner);
        out.extend_from_slice(&typ.to_be_bytes());
        out.extend_from_slice(&[0, 1, 0, 0, 0, 120]);
        out.extend_from_slice(&(data.len() as u16).to_be_bytes());
        out.extend_from_slice(data);
    }

    #[test]
    fn reads_an_ndi_answer() {
        let mut m = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 3];
        let mut ptr = Vec::new();
        name(&mut ptr, "VMIX-01 (Output 1)._ndi._tcp.local");
        record(&mut m, "_ndi._tcp.local", 12, &ptr);
        let mut srv = vec![0, 0, 0, 0, 0x17, 0x48];
        name(&mut srv, "vmix-01.local");
        record(&mut m, "VMIX-01 (Output 1)._ndi._tcp.local", 33, &srv);
        record(&mut m, "vmix-01.local", 1, &[10, 10, 0, 120]);
        record(&mut m, "VMIX-01 (Output 1)._ndi._tcp.local", 16, b"\x0bgroups=prod");
        let mut r = Records::default();
        collect(&m, &mut r).unwrap();
        let s = services(&r);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].kind.as_str(), s[0].name.as_str(), s[0].port), ("NDI", "VMIX-01 (Output 1)", 5960));
        assert_eq!((s[0].ip, s[0].host.as_str()), (Some(Ipv4Addr::new(10, 10, 0, 120)), "vmix-01.local"));
        assert_eq!(s[0].txt, vec!["groups=prod".to_string()]);
    }
}
