//! Names of a machine: reverse DNS, then mDNS (Apple, Linux, printers, Dante,
//! NDI…), then NetBIOS (Windows).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Dns,
    Mdns,
    Netbios,
}

/// The first name found, and where it came from.
pub async fn lookup(ip: Ipv4Addr, timeout: Duration) -> Option<(String, Source)> {
    if let Some(n) = reverse_dns(ip, timeout).await {
        return Some((n, Source::Dns));
    }
    if let Some(n) = mdns(ip, timeout).await {
        return Some((n, Source::Mdns));
    }
    netbios(ip, timeout).await.map(|n| (n, Source::Netbios))
}

/// PTR through the system resolver.
pub async fn reverse_dns(ip: Ipv4Addr, timeout: Duration) -> Option<String> {
    let task = tokio::task::spawn_blocking(move || dns_lookup::lookup_addr(&IpAddr::V4(ip)).ok());
    let name = tokio::time::timeout(timeout * 4, task).await.ok()?.ok()??;
    let name = name.trim_end_matches('.').to_string();
    (!name.is_empty() && name.parse::<IpAddr>().is_err()).then_some(name)
}

async fn ask(ip: Ipv4Addr, port: u16, query: &[u8], timeout: Duration) -> Option<Vec<u8>> {
    let sock = UdpSocket::bind("0.0.0.0:0").await.ok()?;
    sock.send_to(query, SocketAddr::from((ip, port))).await.ok()?;
    let mut buf = vec![0u8; 1500];
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let (n, from) = tokio::time::timeout_at(deadline, sock.recv_from(&mut buf)).await.ok()?.ok()?;
        if from.ip() == IpAddr::V4(ip) {
            buf.truncate(n);
            return Some(buf);
        }
    }
}

/// Unicast mDNS PTR query sent to the machine itself (port 5353).
pub async fn mdns(ip: Ipv4Addr, timeout: Duration) -> Option<String> {
    let o = ip.octets();
    let qname = format!("{}.{}.{}.{}.in-addr.arpa", o[3], o[2], o[1], o[0]);
    let id = rand::random::<u16>();
    let q = dns_query(id, &qname, 12);
    let answer = ask(ip, 5353, &q, timeout).await?;
    parse_ptr(&answer).map(|n| n.trim_end_matches('.').to_string())
}

pub fn dns_query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
    let mut q = Vec::with_capacity(64);
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.split('.').filter(|l| !l.is_empty()) {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&1u16.to_be_bytes());
    q
}

/// A (possibly compressed) DNS name at `pos`; returns it and the position after it.
pub fn read_name(msg: &[u8], mut pos: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut end = None;
    for _ in 0..64 {
        let len = *msg.get(pos)? as usize;
        if len == 0 {
            end.get_or_insert(pos + 1);
            return Some((labels.join("."), end?));
        }
        if len & 0xC0 == 0xC0 {
            let ptr = ((len & 0x3F) << 8) | *msg.get(pos + 1)? as usize;
            end.get_or_insert(pos + 2);
            pos = ptr;
            continue;
        }
        let label = msg.get(pos + 1..pos + 1 + len)?;
        labels.push(String::from_utf8_lossy(label).into_owned());
        pos += 1 + len;
    }
    None
}

/// The first PTR answer of a DNS message.
pub fn parse_ptr(msg: &[u8]) -> Option<String> {
    if msg.len() < 12 {
        return None;
    }
    let qd = u16::from_be_bytes([msg[4], msg[5]]);
    let an = u16::from_be_bytes([msg[6], msg[7]]);
    let mut pos = 12;
    for _ in 0..qd {
        pos = read_name(msg, pos)?.1 + 4;
    }
    for _ in 0..an {
        let (_, p) = read_name(msg, pos)?;
        let typ = u16::from_be_bytes([*msg.get(p)?, *msg.get(p + 1)?]);
        let len = u16::from_be_bytes([*msg.get(p + 8)?, *msg.get(p + 9)?]) as usize;
        if typ == 12 {
            return read_name(msg, p + 10).map(|(n, _)| n).filter(|n| !n.is_empty());
        }
        pos = p + 10 + len;
    }
    None
}

/// NetBIOS node status (UDP 137): the machine's own (unique) name.
pub async fn netbios(ip: Ipv4Addr, timeout: Duration) -> Option<String> {
    let mut q = Vec::with_capacity(50);
    q.extend_from_slice(&rand::random::<u16>().to_be_bytes());
    q.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0x20]);
    // "*" padded with zeros, half-byte encoded.
    for b in std::iter::once(b'*').chain(std::iter::repeat_n(0u8, 15)) {
        q.push(b'A' + (b >> 4));
        q.push(b'A' + (b & 0x0F));
    }
    q.extend_from_slice(&[0, 0, 0x21, 0, 1]);
    parse_nbstat(&ask(ip, 137, &q, timeout).await?)
}

pub fn parse_nbstat(r: &[u8]) -> Option<String> {
    // Header (12) + name (34) + type, class (4) + ttl (4) + length (2).
    let mut pos = 12 + read_name(r, 12).map(|(_, p)| p - 12)? + 10;
    let count = *r.get(pos)? as usize;
    pos += 1;
    for i in 0..count {
        let e = r.get(pos + i * 18..pos + i * 18 + 18)?;
        let (suffix, group) = (e[15], e[16] & 0x80 != 0);
        if suffix == 0 && !group {
            let n = String::from_utf8_lossy(&e[..15]).trim().to_string();
            if !n.is_empty() {
                return Some(n);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ptr_answers_with_compression() {
        let mut m = dns_query(7, "23.0.10.10.in-addr.arpa", 12);
        m[2] = 0x84; // answer
        m[7] = 1; // one answer
        m.extend_from_slice(&[0xC0, 12, 0, 12, 0, 1, 0, 0, 0, 120]);
        let rdata = [3, b'p', b't', b'z', 5, b'l', b'o', b'c', b'a', b'l', 0];
        m.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        m.extend_from_slice(&rdata);
        assert_eq!(parse_ptr(&m).as_deref(), Some("ptz.local"));
    }

    #[test]
    fn nbstat_answers() {
        let mut r = vec![0u8; 12];
        r.push(0x20);
        r.extend_from_slice(b"CK");
        r.extend_from_slice(&[b'A'; 30]);
        r.push(0);
        r.extend_from_slice(&[0, 0x21, 0, 1, 0, 0, 0, 0, 0, 65]);
        r.push(2);
        let mut group = b"WORKGROUP      \x00".to_vec();
        group.extend_from_slice(&[0x84, 0]);
        let mut own = b"DESKTOP-K3L9   \x00".to_vec();
        own.extend_from_slice(&[0x04, 0]);
        r.extend(group);
        r.extend(own);
        assert_eq!(parse_nbstat(&r).as_deref(), Some("DESKTOP-K3L9"));
    }
}
