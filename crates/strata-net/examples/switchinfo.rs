//! cargo run -p strata-net --example switchinfo -- 127.0.0.1:1171 public
//! Reads everything `switchinfo` knows of a switch (SNMP v2c, read-only) and prints it.
use std::time::{Duration, Instant};

use strata_net::{mac, snmp, switchinfo};

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let target = a.first().map(String::as_str).unwrap_or("127.0.0.1:161");
    let (ip, port) = target.split_once(':').map_or((target, 161), |(h, p)| (h, p.parse().unwrap_or(161)));
    let agent = snmp::Agent { ip: ip.parse().expect("an IPv4 address"), port, community: a.get(1).cloned().unwrap_or_else(|| "public".into()) };
    let s = match snmp::Session::open(&agent, Duration::from_secs(2)).await {
        Ok(s) => s,
        Err(e) => return println!("open: {e}"),
    };
    let names = snmp::interfaces(&s).await.unwrap_or_default();
    let name = |i: u32| names.get(&i).cloned().unwrap_or_else(|| format!("if{i}"));
    let started = Instant::now();
    let info = switchinfo::all(&s).await;
    let took = started.elapsed();
    let opt = |v: Option<f32>, unit: &str| v.map_or("-".to_string(), |v| format!("{v:.2}{unit}"));

    println!("== neighbours ({})", info.neighbours.len());
    for n in &info.neighbours {
        let ip = n.remote_ip.map(|i| i.to_string()).unwrap_or_default();
        println!("  {:<14} {:?}  {} {}  {}  [{}] id={}", name(n.if_index), n.protocol, n.remote_name, n.remote_port, ip, n.platform, n.chassis_id);
    }
    println!("== optics ({})", info.optics.len());
    for o in &info.optics {
        println!(
            "  {:<14} {} / {} / {} / {}  rx {} (lanes {:?})  tx {}  {}  {}  {}  low warn {} alarm {}",
            name(o.if_index),
            o.kind,
            o.part,
            o.vendor,
            o.serial,
            opt(o.rx_dbm, " dBm"),
            o.rx_lanes_dbm,
            opt(o.tx_dbm, " dBm"),
            opt(o.temp_c, " C"),
            opt(o.bias_ma, " mA"),
            opt(o.voltage, " V"),
            opt(o.rx_low_warn_dbm, ""),
            opt(o.rx_low_alarm_dbm, "")
        );
    }
    println!("== health");
    let h = &info.health;
    println!("  cpu {} (5 min {})", opt(h.cpu_pct, " %"), opt(h.cpu_5min_pct, " %"));
    let mb = |b: Option<u64>| b.map_or("-".to_string(), |b| format!("{} MB", b / 1_000_000));
    println!("  memory {} used of {}", mb(h.mem_used_bytes), mb(h.mem_total_bytes));
    for (n, c) in &h.temps {
        println!("  temp  {n}: {c:.1} C");
    }
    for f in &h.fans {
        println!("  fan   {}: {} ({})", f.name, if f.ok { "ok" } else { "FAIL" }, f.detail);
    }
    for p in &h.psus {
        println!("  psu   {}: {} ({}) {}", p.name, if p.ok { "ok" } else { "FAIL" }, p.detail, opt(p.watts, " W"));
    }
    println!("== lags ({})", info.lags.len());
    for (m, g) in &info.lags {
        println!("  {} -> {}", name(*m), name(*g));
    }
    let count = |want: switchinfo::StpState| info.stp.iter().filter(|(_, st)| *st == want).count();
    let other: Vec<String> = info
        .stp
        .iter()
        .filter(|(_, st)| !matches!(st, switchinfo::StpState::Forwarding | switchinfo::StpState::Disabled))
        .map(|(i, st)| format!("{} {:?}", name(*i), st))
        .collect();
    println!(
        "== stp ({} ports: {} forwarding, {} disabled) {}",
        info.stp.len(),
        count(switchinfo::StpState::Forwarding),
        count(switchinfo::StpState::Disabled),
        other.join(", ")
    );
    println!("== igmp snooping ({})", info.mcast_groups.len());
    for g in &info.mcast_groups {
        let ports: Vec<String> = g.ports.iter().map(|i| name(*i)).collect();
        println!("  vlan {:<4} {} {:?} -> {}", g.vlan, mac::format(&g.mac), g.group, ports.join(", "));
    }
    for (v, ports) in &info.mcast_router_ports {
        println!("  vlan {v} router/forward-all ports: {}", ports.iter().map(|i| name(*i)).collect::<Vec<_>>().join(", "));
    }
    match &info.poe {
        None => println!("== poe: none"),
        Some(p) => {
            println!("== poe: budget {} used {} ({} ports)", opt(p.budget_w, " W"), opt(p.used_w, " W"), p.ports.len());
            for x in p.ports.iter().filter(|x| x.on || !matches!(x.status, switchinfo::PoeStatus::Searching)) {
                println!(
                    "  {:<14} {}.{} admin {} {:?} class {:?} {}",
                    if x.if_index == 0 { "?".to_string() } else { name(x.if_index) },
                    x.group,
                    x.port,
                    x.admin_on,
                    x.status,
                    x.class,
                    opt(x.watts, " W")
                );
            }
        }
    }
    if !info.errors.is_empty() {
        println!("== errors: {:?}", info.errors);
    }
    println!("({} ms)", took.as_millis());
}
