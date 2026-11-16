//! Multicast flooding: media packets that reach this machine for groups it has
//! not joined, which means IGMP snooping does not hold them back on that VLAN.
//!
//! Receive only: a packet socket of the interface listens for a moment with
//! the interface accepting every multicast frame (all-multicast, local to
//! this machine). Nothing is sent and no group is joined. Linux only, and
//! the server needs the right to open packet sockets (root or CAP_NET_RAW).

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::Ipv4Addr;

/// A group received without being joined, during one sample.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub group: Ipv4Addr,
    pub port: u16,
    pub source: Ipv4Addr,
    pub packets: u64,
    /// UDP payload bytes.
    pub bytes: u64,
    /// 802.1Q tag of the frames, when they carry one.
    pub vlan: Option<u16>,
    /// The beginning of the first UDP payload (RTP header…).
    pub head: Vec<u8>,
}

/// A UDP datagram to a multicast group, from an Ethernet frame.
#[derive(Debug, PartialEq)]
pub struct Frame<'a> {
    pub group: Ipv4Addr,
    pub port: u16,
    pub source: Ipv4Addr,
    pub vlan: Option<u16>,
    /// UDP payload length (of the whole frame, even when only its start was read).
    pub len: usize,
    /// The start of the UDP payload that was read.
    pub head: &'a [u8],
}

/// A frame (`f`: what was read of it; `real_len`: its length), when it is UDP to a multicast group.
pub fn parse_frame(f: &[u8], real_len: usize) -> Option<Frame<'_>> {
    let (mut off, mut vlan) = (12, None);
    let mut ethertype = u16::from_be_bytes([*f.get(off)?, *f.get(off + 1)?]);
    if ethertype == 0x8100 {
        vlan = Some(u16::from_be_bytes([*f.get(14)?, *f.get(15)?]) & 0x0FFF);
        off += 4;
        ethertype = u16::from_be_bytes([*f.get(off)?, *f.get(off + 1)?]);
    }
    if ethertype != 0x0800 {
        return None;
    }
    let ip = f.get(off + 2..)?;
    if ip.len() < 20 || ip[0] >> 4 != 4 || ip[9] != 17 {
        return None;
    }
    let ihl = usize::from(ip[0] & 0x0F) * 4;
    // A fragment that is not the first carries no UDP header.
    if u16::from_be_bytes([ip[6], ip[7]]) & 0x1FFF != 0 {
        return None;
    }
    let source = Ipv4Addr::new(ip[12], ip[13], ip[14], ip[15]);
    let group = Ipv4Addr::new(ip[16], ip[17], ip[18], ip[19]);
    if !group.is_multicast() {
        return None;
    }
    let udp = ip.get(ihl..)?;
    if udp.len() < 8 {
        return None;
    }
    let port = u16::from_be_bytes([udp[2], udp[3]]);
    let payload_len = usize::from(u16::from_be_bytes([udp[4], udp[5]])).saturating_sub(8);
    let payload_len = payload_len.min(real_len.saturating_sub(off + 2 + ihl + 8));
    Some(Frame { group, port, source, vlan, len: payload_len, head: &udp[8..] })
}

/// Media, not discovery or control traffic: RTP version 2, or MPEG-TS packets.
pub fn looks_media(head: &[u8], payload_len: usize) -> bool {
    let rtp = head.len() >= 12 && head[0] >> 6 == 2 && payload_len >= 12;
    let ts = head.first() == Some(&0x47) && payload_len >= 188 && payload_len.is_multiple_of(188);
    let ts_in_rtp = rtp && payload_len >= 200 && (payload_len - 12).is_multiple_of(188);
    rtp || ts || ts_in_rtp
}

/// Groups that must not count as flooded: link-local control (224.0.0.0/24) and
/// the groups joined on this machine.
pub fn ignored(group: Ipv4Addr, joined: &HashSet<Ipv4Addr>) -> bool {
    let o = group.octets();
    (o[0] == 224 && o[1] == 0 && o[2] == 0) || joined.contains(&group)
}

/// The groups joined on the interface `ifname` (`/proc/net/igmp`).
pub fn joined_groups(ifname: &str) -> HashSet<Ipv4Addr> {
    let text = std::fs::read_to_string("/proc/net/igmp").unwrap_or_default();
    parse_proc_igmp(&text, ifname)
}

pub fn parse_proc_igmp(text: &str, ifname: &str) -> HashSet<Ipv4Addr> {
    let mut out = HashSet::new();
    let mut current = false;
    for line in text.lines().skip(1) {
        if !line.starts_with(char::is_whitespace) {
            // "4	eth0      :     1      V3"
            current = line.split_whitespace().nth(1).map(|n| n.trim_end_matches(':')) == Some(ifname);
            continue;
        }
        if current {
            if let Some(hex) = line.split_whitespace().next() {
                if let Ok(v) = u32::from_str_radix(hex, 16) {
                    // Little-endian on the hosts this runs on.
                    out.insert(Ipv4Addr::from(v.swap_bytes()));
                }
            }
        }
    }
    out
}

/// Accumulates the frames of one sample.
#[derive(Default)]
pub struct Sampler {
    seen: HashMap<(Ipv4Addr, u16, Ipv4Addr), Seen>,
}

impl Sampler {
    pub fn frame(&mut self, f: &[u8], real_len: usize, joined: &HashSet<Ipv4Addr>) {
        let Some(Frame { group, port, source, vlan, len, head }) = parse_frame(f, real_len) else { return };
        if ignored(group, joined) || !looks_media(head, len) {
            return;
        }
        let e = self.seen.entry((group, port, source)).or_insert_with(|| Seen { group, port, source, packets: 0, bytes: 0, vlan, head: head[..head.len().min(32)].to_vec() });
        e.packets += 1;
        e.bytes += len as u64;
    }

    /// What was seen; counts scaled by `scale` (frames the kernel dropped).
    pub fn finish(self, scale: f64, joined_after: &HashSet<Ipv4Addr>) -> Vec<Seen> {
        let mut v: Vec<Seen> = self
            .seen
            .into_values()
            .filter(|s| !joined_after.contains(&s.group))
            .map(|mut s| {
                s.packets = (s.packets as f64 * scale) as u64;
                s.bytes = (s.bytes as f64 * scale) as u64;
                s
            })
            .collect();
        v.sort_by_key(|s| (u32::from(s.group), s.port));
        v
    }
}

/// Listens on `ifname` for `ms` milliseconds (blocking).
#[cfg(target_os = "linux")]
pub fn sample(ifname: &str, ms: u64) -> io::Result<Vec<Seen>> {
    use std::time::{Duration, Instant};
    let joined = joined_groups(ifname);
    let c_name = std::ffi::CString::new(ifname).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: plain libc calls on a socket owned here, closed by `Fd` on every path.
    unsafe {
        let index = libc::if_nametoindex(c_name.as_ptr());
        if index == 0 {
            return Err(io::Error::last_os_error());
        }
        let proto = (libc::ETH_P_ALL as u16).to_be();
        let fd = libc::socket(libc::AF_PACKET, libc::SOCK_RAW | libc::SOCK_CLOEXEC, i32::from(proto));
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        struct Fd(i32);
        impl Drop for Fd {
            fn drop(&mut self) {
                // SAFETY: the descriptor is owned and closed once.
                unsafe { libc::close(self.0) };
            }
        }
        let fd = Fd(fd);
        // Only IPv4 UDP to a multicast group (plain or 802.1Q-tagged), 164 bytes kept.
        let prog: [libc::sock_filter; 18] = [
            bpf(0x28, 0, 0, 12),
            bpf(0x15, 0, 6, 0x0800),
            bpf(0x30, 0, 0, 23),
            bpf(0x15, 0, 13, 17),
            bpf(0x30, 0, 0, 30),
            bpf(0x54, 0, 0, 0xF0),
            bpf(0x15, 0, 10, 0xE0),
            bpf(0x06, 0, 0, 160),
            bpf(0x15, 0, 8, 0x8100),
            bpf(0x28, 0, 0, 16),
            bpf(0x15, 0, 6, 0x0800),
            bpf(0x30, 0, 0, 27),
            bpf(0x15, 0, 4, 17),
            bpf(0x30, 0, 0, 34),
            bpf(0x54, 0, 0, 0xF0),
            bpf(0x15, 0, 1, 0xE0),
            bpf(0x06, 0, 0, 164),
            bpf(0x06, 0, 0, 0),
        ];
        let fprog = libc::sock_fprog { len: prog.len() as u16, filter: prog.as_ptr() as *mut _ };
        if libc::setsockopt(fd.0, libc::SOL_SOCKET, libc::SO_ATTACH_FILTER, &fprog as *const _ as *const libc::c_void, std::mem::size_of::<libc::sock_fprog>() as u32) != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut sll: libc::sockaddr_ll = std::mem::zeroed();
        sll.sll_family = libc::AF_PACKET as u16;
        sll.sll_protocol = proto;
        sll.sll_ifindex = index as i32;
        if libc::bind(fd.0, &sll as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_ll>() as u32) != 0 {
            return Err(io::Error::last_os_error());
        }
        // All-multicast on this interface for the time of the sample (dropped with the socket).
        let mreq = libc::packet_mreq { mr_ifindex: index as i32, mr_type: libc::PACKET_MR_ALLMULTI as u16, mr_alen: 0, mr_address: [0; 8] };
        if libc::setsockopt(fd.0, libc::SOL_PACKET, libc::PACKET_ADD_MEMBERSHIP, &mreq as *const _ as *const libc::c_void, std::mem::size_of::<libc::packet_mreq>() as u32) != 0 {
            return Err(io::Error::last_os_error());
        }
        let tv = libc::timeval { tv_sec: 0, tv_usec: 200_000 };
        libc::setsockopt(fd.0, libc::SOL_SOCKET, libc::SO_RCVTIMEO, &tv as *const _ as *const libc::c_void, std::mem::size_of::<libc::timeval>() as u32);
        let rcvbuf: libc::c_int = 4 << 20;
        libc::setsockopt(fd.0, libc::SOL_SOCKET, libc::SO_RCVBUF, &rcvbuf as *const _ as *const libc::c_void, std::mem::size_of::<libc::c_int>() as u32);
        let mut sampler = Sampler::default();
        let mut buf = [0u8; 256];
        let (mut handled, end) = (0u64, Instant::now() + Duration::from_millis(ms));
        while Instant::now() < end {
            let mut from: libc::sockaddr_ll = std::mem::zeroed();
            let mut flen = std::mem::size_of::<libc::sockaddr_ll>() as u32;
            let n = libc::recvfrom(fd.0, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), libc::MSG_TRUNC, &mut from as *mut _ as *mut libc::sockaddr, &mut flen);
            if n < 0 {
                let e = io::Error::last_os_error();
                if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) {
                    continue;
                }
                return Err(e);
            }
            // Frames this machine sends are not floods.
            if from.sll_pkttype == libc::PACKET_OUTGOING {
                continue;
            }
            handled += 1;
            let real = n as usize;
            sampler.frame(&buf[..real.min(buf.len())], real, &joined);
        }
        // Frames the kernel dropped for lack of room: scale the counts.
        let mut stats: libc::tpacket_stats = std::mem::zeroed();
        let mut slen = std::mem::size_of::<libc::tpacket_stats>() as u32;
        let scale = if libc::getsockopt(fd.0, libc::SOL_PACKET, libc::PACKET_STATISTICS, &mut stats as *mut _ as *mut libc::c_void, &mut slen) == 0 && handled > 0 {
            (f64::from(stats.tp_packets) / handled as f64).max(1.0)
        } else {
            1.0
        };
        Ok(sampler.finish(scale, &joined_groups(ifname)))
    }
}

#[cfg(target_os = "linux")]
const fn bpf(code: u16, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

#[cfg(not(target_os = "linux"))]
pub fn sample(_ifname: &str, _ms: u64) -> io::Result<Vec<Seen>> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "flood detection needs linux"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(vlan: Option<u16>, group: [u8; 4], port: u16, payload: &[u8]) -> Vec<u8> {
        let mut f = vec![0x01, 0x00, 0x5e, group[1] & 0x7f, group[2], group[3], 0, 1, 2, 3, 4, 5];
        if let Some(v) = vlan {
            f.extend_from_slice(&[0x81, 0x00]);
            f.extend_from_slice(&v.to_be_bytes());
        }
        f.extend_from_slice(&[0x08, 0x00]);
        let total = 20 + 8 + payload.len();
        f.extend_from_slice(&[0x45, 0, (total >> 8) as u8, total as u8, 0, 0, 0x40, 0, 64, 17, 0, 0, 10, 10, 1, 11]);
        f.extend_from_slice(&group);
        let ulen = 8 + payload.len();
        f.extend_from_slice(&[0x13, 0x8c, (port >> 8) as u8, port as u8, (ulen >> 8) as u8, ulen as u8, 0, 0]);
        f.extend_from_slice(payload);
        f
    }

    #[test]
    fn frames() {
        let mut rtp = vec![0x80, 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
        rtp.extend_from_slice(&[0u8; 1200]);
        let f = frame(Some(120), [239, 10, 1, 1], 5004, &rtp);
        let x = parse_frame(&f[..160], f.len()).unwrap();
        assert_eq!((x.group, x.port, x.source, x.vlan, x.len), (Ipv4Addr::new(239, 10, 1, 1), 5004, Ipv4Addr::new(10, 10, 1, 11), Some(120), 1212));
        assert!(looks_media(x.head, x.len));
        let f2 = frame(None, [239, 255, 255, 250], 1900, b"M-SEARCH * HTTP/1.1\r\n");
        let y = parse_frame(&f2, f2.len()).unwrap();
        assert_eq!(y.vlan, None);
        assert!(!looks_media(y.head, y.len));
        let joined: HashSet<Ipv4Addr> = [Ipv4Addr::new(239, 10, 1, 2)].into();
        let mut s = Sampler::default();
        for _ in 0..10 {
            s.frame(&f[..160], f.len(), &joined);
        }
        s.frame(&frame(None, [239, 10, 1, 2], 5004, &rtp), 1254, &joined);
        s.frame(&frame(None, [224, 0, 0, 251], 5353, &rtp), 1254, &joined);
        let seen = s.finish(2.0, &HashSet::new());
        assert_eq!(seen.len(), 1);
        assert_eq!((seen[0].packets, seen[0].bytes, seen[0].vlan), (20, 2 * 10 * 1212, Some(120)));
    }

    #[test]
    fn proc_igmp() {
        let text = "Idx\tDevice    : Count Querier\tGroup    Users Timer\tReporter\n1\tlo        :     1      V3\n\t\t\t\t010000E0     1 0:00000000\t\t0\n\
4\teth0      :     2      V3\n\t\t\t\tFBFFFFEF     1 0:00000000\t\t0\n\t\t\t\t810100E0     1 0:00000000\t\t0\n";
        let g = parse_proc_igmp(text, "eth0");
        assert!(g.contains(&Ipv4Addr::new(239, 255, 255, 251)));
        assert!(g.contains(&Ipv4Addr::new(224, 0, 1, 129)));
        assert!(!g.contains(&Ipv4Addr::new(224, 0, 0, 1)));
    }
}
