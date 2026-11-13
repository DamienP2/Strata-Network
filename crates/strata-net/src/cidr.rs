//! IPv4 address ranges: `10.0.0.0/24`, `10.0.0.5`, `10.0.0.1-10.0.0.50`, `10.0.0.1-50`.

use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

/// The most addresses one scan may cover (a /16).
pub const MAX_ADDRESSES: u64 = 65_536;

/// An inclusive range of IPv4 addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: u32,
    pub end: u32,
    /// The prefix length when written as a CIDR block (network and broadcast left out).
    pub prefix: Option<u8>,
}

impl Range {
    pub fn single(ip: Ipv4Addr) -> Self {
        let n = u32::from(ip);
        Range { start: n, end: n, prefix: None }
    }

    pub fn len(&self) -> u64 {
        u64::from(self.end) - u64::from(self.start) + 1
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        let n = u32::from(ip);
        n >= self.start && n <= self.end
    }

    pub fn first(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.start)
    }

    pub fn last(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.end)
    }

    pub fn iter(&self) -> impl Iterator<Item = Ipv4Addr> + Clone + Send + 'static {
        (self.start..=self.end).map(Ipv4Addr::from)
    }

    /// The network address of a CIDR block, if this range came from one.
    pub fn network(&self) -> Option<Ipv4Addr> {
        let p = self.prefix?;
        Some(Ipv4Addr::from(self.start & mask(p)))
    }
}

fn mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.prefix, self.network()) {
            (Some(p), Some(net)) => write!(f, "{net}/{p}"),
            _ if self.start == self.end => write!(f, "{}", self.first()),
            _ => write!(f, "{}-{}", self.first(), self.last()),
        }
    }
}

impl FromStr for Range {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        parse(s)
    }
}

fn ip(s: &str) -> Result<Ipv4Addr, String> {
    s.trim().parse::<Ipv4Addr>().map_err(|_| format!("“{}” is not an ipv4 address", s.trim()))
}

/// Reads a range. A CIDR block leaves out its network and broadcast addresses
/// (except /31 and /32). Refuses more than [`MAX_ADDRESSES`] addresses.
pub fn parse(s: &str) -> Result<Range, String> {
    let s = s.trim();
    let r = if let Some((a, p)) = s.split_once('/') {
        let base = u32::from(ip(a)?);
        let p: u8 = p.trim().parse().ok().filter(|p| *p <= 32).ok_or("the prefix is a number from 0 to 32")?;
        let m = mask(p);
        let (net, bcast) = (base & m, base | !m);
        let (start, end) = if p >= 31 { (net, bcast) } else { (net + 1, bcast - 1) };
        Range { start, end, prefix: Some(p) }
    } else if let Some((a, b)) = s.split_once('-') {
        let start = ip(a)?;
        let b = b.trim();
        let end = if b.contains('.') {
            ip(b)?
        } else {
            let last: u8 = b.parse().map_err(|_| format!("“{b}” is not an address"))?;
            let o = start.octets();
            Ipv4Addr::new(o[0], o[1], o[2], last)
        };
        let (start, end) = (u32::from(start), u32::from(end));
        if end < start {
            return Err("the range ends before it starts".into());
        }
        Range { start, end, prefix: None }
    } else {
        Range::single(ip(s)?)
    };
    if r.len() > MAX_ADDRESSES {
        return Err(format!("the range is too large (at most {MAX_ADDRESSES} addresses)"));
    }
    Ok(r)
}

/// Whether `ip` is in the CIDR block `net/prefix` (prefix as a number).
pub fn in_block(ip: Ipv4Addr, net: Ipv4Addr, prefix: u8) -> bool {
    let m = mask(prefix.min(32));
    u32::from(ip) & m == u32::from(net) & m
}

/// Reads `a.b.c.d/p` as (network, prefix) without the size limit (multicast plans, VLAN subnets).
pub fn parse_block(s: &str) -> Result<(Ipv4Addr, u8), String> {
    let (a, p) = s.trim().split_once('/').ok_or("write the subnet as a.b.c.d/prefix")?;
    let p: u8 = p.trim().parse().ok().filter(|p| *p <= 32).ok_or("the prefix is a number from 0 to 32")?;
    Ok((Ipv4Addr::from(u32::from(ip(a)?) & mask(p)), p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_leaves_out_network_and_broadcast() {
        let r = parse("192.168.1.77/24").unwrap();
        assert_eq!(r.first(), Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(r.last(), Ipv4Addr::new(192, 168, 1, 254));
        assert_eq!(r.len(), 254);
        assert_eq!(r.to_string(), "192.168.1.0/24");
        assert_eq!(parse("10.0.0.0/31").unwrap().len(), 2);
        assert_eq!(parse("10.0.0.9/32").unwrap().len(), 1);
    }

    #[test]
    fn dashed_ranges() {
        let r = parse("10.0.0.10-10.0.0.20").unwrap();
        assert_eq!(r.len(), 11);
        assert_eq!(parse("10.0.0.10-20").unwrap(), r);
        assert_eq!(r.to_string(), "10.0.0.10-10.0.0.20");
        assert!(parse("10.0.0.20-10").is_err());
        assert_eq!(parse(" 10.1.2.3 ").unwrap().len(), 1);
    }

    #[test]
    fn limits() {
        assert!(parse("10.0.0.0/16").is_ok());
        assert!(parse("10.0.0.0/15").is_err());
        assert!(parse("10.0.0.0/33").is_err());
        assert!(parse("nope").is_err());
    }

    #[test]
    fn blocks() {
        assert!(in_block(Ipv4Addr::new(10, 20, 3, 4), Ipv4Addr::new(10, 20, 0, 0), 22));
        assert!(!in_block(Ipv4Addr::new(10, 20, 4, 4), Ipv4Addr::new(10, 20, 0, 0), 22));
        assert_eq!(parse_block("239.1.2.3/16").unwrap(), (Ipv4Addr::new(239, 1, 0, 0), 16));
    }
}
