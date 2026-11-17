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
    ("_nmos-query._tcp.local", "NMOS Query"),
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
    #[serde(default)]
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

fn services(r: &Records, only: &[&str]) -> Vec<Service> {
    let mut out = Vec::new();
    for (ty, instance) in &r.ptr {
        if !only.iter().any(|t| t.eq_ignore_ascii_case(ty)) {
            continue;
        }
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
    let types: Vec<&str> = TYPES.iter().map(|(t, _)| *t).collect();
    browse_types(iface, &types, 2, wait).await
}

/// A PTR (or A) question; `qu`: the "unicast response" bit (RFC 6762 § 5.4).
fn question(name: &str, qtype: u16, qu: bool) -> Vec<u8> {
    let mut q = dns_query(0, name, qtype);
    if qu {
        let n = q.len();
        q[n - 2] |= 0x80;
    }
    q
}

/// A socket on 5353 that has joined 224.0.0.251 on `iface`: it hears the answers
/// sent to the group (devices that ignore the "legacy unicast" rule) and the
/// unsolicited announcements. Shared with any other responder of the machine.
fn group_socket(iface: Ipv4Addr) -> std::io::Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(unix)]
    s.set_reuse_port(true)?;
    s.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, 5353)).into())?;
    s.join_multicast_v4(&GROUP, &iface)?;
    s.set_multicast_if_v4(&iface)?;
    s.set_multicast_ttl_v4(255)?;
    s.set_nonblocking(true)?;
    UdpSocket::from_std(s.into())
}

/// Asks for `types` `rounds` times, 1 s apart, in both styles: from a passing port
/// (answered directly, "legacy unicast") and, when port 5353 can be shared, from
/// it (QM then QU: answered to the group or directly). Hosts named by a SRV
/// without their address are asked for it. Listens until `wait` after the last
/// question. Answers of every round are put together.
pub async fn browse_types(iface: Ipv4Addr, types: &[&str], rounds: usize, wait: Duration) -> std::io::Result<Vec<Service>> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_multicast_if_v4(&iface)?;
    s.set_multicast_ttl_v4(255)?;
    s.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)).into())?;
    s.set_nonblocking(true)?;
    let sock = UdpSocket::from_std(s.into())?;
    let group = group_socket(iface).ok();
    let rounds = rounds.max(1);
    let mut r = Records::default();
    let mut buf = vec![0u8; 9000];
    let mut buf2 = vec![0u8; 9000];
    let start = tokio::time::Instant::now();
    let deadline = start + Duration::from_secs(rounds as u64 - 1) + wait;
    for round in 0..rounds {
        let ask = |name: &str, qtype: u16, qu: bool| question(name, qtype, qu);
        for ty in types {
            let _ = sock.send_to(&ask(ty, 12, false), (GROUP, 5353)).await;
            if let Some(g) = &group {
                let _ = g.send_to(&ask(ty, 12, round % 2 == 1), (GROUP, 5353)).await;
            }
        }
        // Hosts whose address did not come with their SRV.
        let missing: Vec<String> = r.srv.values().map(|(h, _)| h.to_ascii_lowercase()).filter(|h| !h.is_empty() && !r.a.contains_key(h)).collect();
        for h in missing.iter().take(32) {
            let _ = sock.send_to(&ask(h, 1, false), (GROUP, 5353)).await;
        }
        let until = if round + 1 == rounds { deadline } else { start + Duration::from_secs(round as u64 + 1) };
        loop {
            let got = tokio::time::timeout_at(until, async {
                match &group {
                    Some(g) => tokio::select! {
                        x = sock.recv_from(&mut buf) => x.map(|(n, _)| (n, false)),
                        x = g.recv_from(&mut buf2) => x.map(|(n, _)| (n, true)),
                    },
                    None => sock.recv_from(&mut buf).await.map(|(n, _)| (n, false)),
                }
            })
            .await;
            let Ok(Ok((n, from_group))) = got else { break };
            let msg = if from_group { &buf2[..n] } else { &buf[..n] };
            // Answers only (QR set): the questions of other askers are not records.
            if msg.len() > 2 && msg[2] & 0x80 != 0 {
                collect(msg, &mut r);
            }
        }
    }
    Ok(services(&r, types))
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
        let s = services(&r, &["_ndi._tcp.local"]);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].kind.as_str(), s[0].name.as_str(), s[0].port), ("NDI", "VMIX-01 (Output 1)", 5960));
        assert_eq!((s[0].ip, s[0].host.as_str()), (Some(Ipv4Addr::new(10, 10, 0, 120)), "vmix-01.local"));
        assert_eq!(s[0].txt, vec!["groups=prod".to_string()]);
    }

    /// The web page reads `txt` on every service: it is always sent, empty or not.
    #[test]
    fn txt_always_sent() {
        let s = Service { kind: "NDI".into(), name: "x".into(), host: "h".into(), ip: None, port: 1, txt: vec![] };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["txt"], serde_json::json!([]));
        assert!(v["ip"].is_null());
    }
}
