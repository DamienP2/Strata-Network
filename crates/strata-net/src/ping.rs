//! ICMP echo. Uses an unprivileged ICMP socket when the system allows it
//! (`net.ipv4.ping_group_range` on Linux), otherwise a raw socket (root or
//! CAP_NET_RAW). Without either, scans fall back to TCP and ARP.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use surge_ping::{Client, Config, PingIdentifier, PingSequence, ICMP};

pub struct Pinger {
    client: Client,
}

static NEXT_ID: AtomicU16 = AtomicU16::new(1);

impl Pinger {
    /// `None` when this process may not send ICMP.
    pub fn new() -> Option<Self> {
        match Client::new(&Config::builder().kind(ICMP::V4).build()) {
            Ok(client) => Some(Pinger { client }),
            Err(e) => {
                tracing::info!("icmp unavailable ({e}): hosts are found by tcp and arp only");
                None
            }
        }
    }

    /// The round trip, or `None` if no answer within `timeout` (tries `tries` times).
    pub async fn ping(&self, ip: Ipv4Addr, timeout: Duration, tries: u16) -> Option<Duration> {
        let id = PingIdentifier(NEXT_ID.fetch_add(1, Ordering::Relaxed) ^ rand::random::<u16>());
        let mut p = self.client.pinger(IpAddr::V4(ip), id).await;
        p.timeout(timeout);
        for seq in 0..tries.max(1) {
            if let Ok((_, rtt)) = p.ping(PingSequence(seq), &[0u8; 16]).await {
                return Some(rtt);
            }
        }
        None
    }
}
