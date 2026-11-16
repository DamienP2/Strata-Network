//! cargo run --example snmpv3 -- 127.0.0.1 1162 user sha|md5|sha256 authpass des|aes|none privpass
use std::time::Duration;
use strata_net::{snmp, snmpv3};

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let user = snmpv3::V3 {
        user: a[2].clone(),
        auth: match a[3].as_str() { "md5" => snmpv3::Auth::Md5, "sha" => snmpv3::Auth::Sha, "sha256" => snmpv3::Auth::Sha256, _ => snmpv3::Auth::None },
        auth_password: a[4].clone(),
        privacy: match a[5].as_str() { "des" => snmpv3::Privacy::Des, "aes" => snmpv3::Privacy::Aes128, _ => snmpv3::Privacy::None },
        priv_password: a.get(6).cloned().unwrap_or_default(),
    };
    let s = match snmp::Session::open_v3(a[0].parse().unwrap(), a[1].parse().unwrap(), &user, Duration::from_secs(2)).await {
        Ok(s) => s,
        Err(e) => return println!("open: {e}"),
    };
    match snmp::system(&s).await {
        Ok(sys) => println!("system: {} · {} · up {} s", sys.name, sys.description, sys.uptime_secs),
        Err(e) => println!("system: {e}"),
    }
    match snmp::if_stats(&s).await {
        Ok(v) => println!("interfaces: {} ({})", v.len(), v.iter().map(|i| format!("{} {}", i.name, if i.up { "up" } else { "down" })).collect::<Vec<_>>().join(", ")),
        Err(e) => println!("if_stats: {e}"),
    }
}
