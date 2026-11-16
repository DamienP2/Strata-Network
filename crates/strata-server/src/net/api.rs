//! Routes of the network features (`docs/api-network.md`).

use std::collections::HashMap;
use std::net::Ipv4Addr;

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use super::model::*;
use super::{File, Net};
use crate::api::{bad, not_found, require, status, ApiResult, Session};
use crate::auth::User;
use crate::paths::new_id;
use crate::state::Shared;

pub fn router() -> Router<Shared> {
    Router::new()
        .route("/api/networks", get(networks).post(create_network))
        .route("/api/networks/{id}", put(update_network).delete(delete_network))
        .route("/api/networks/{id}/addresses", get(addresses))
        .route("/api/networks/{id}/scan", post(scan).delete(stop_scan))
        .route("/api/networks/{id}/next-free", get(next_free))
        .route("/api/networks/{id}/export.csv", get(export_csv))
        .route("/api/networks/{id}/import", post(import_csv))
        .route("/api/addresses/{ip}", patch(patch_address))
        .route("/api/addresses/{ip}/scan", post(scan_address))
        .route("/api/vlans", get(vlans).post(create_vlan))
        .route("/api/vlans/{id}", put(update_vlan).delete(delete_vlan))
        .route("/api/switches", get(switches).post(create_switch))
        .route("/api/switches/detect", post(detect_switch))
        .route("/api/switches/{id}", put(update_switch).delete(delete_switch))
        .route("/api/switches/{id}/ports", patch(patch_ports))
        .route("/api/switches/{id}/online", patch(set_online))
        .route("/api/switches/{id}/live", get(switch_live))
        .route("/api/switches/{id}/take", post(take_from_switch))
        .route("/api/switches/{id}/info", get(switch_info))
        .route("/api/switches/{id}/ports/{port}/history", get(port_history))
        .route("/api/switches/{id}/rules", put(set_rules))
        .route("/api/switches/{id}/link-neighbours", post(link_neighbours))
        .route("/api/switch-models", get(switch_models).post(create_switch_model))
        .route("/api/switch-models/{id}", axum::routing::delete(delete_switch_model))
        .route("/api/snmp", get(snmp_list).post(create_snmp))
        .route("/api/snmp/test", post(snmp_test))
        .route("/api/snmp/{id}", put(update_snmp).delete(delete_snmp))
        .route("/api/snmp/{id}/poll", post(snmp_poll))
        .route("/api/multicast", get(multicast))
        .route("/api/multicast/settings", put(multicast_settings))
        .route("/api/multicast/check", post(multicast_check))
        .route("/api/multicast/label", patch(multicast_label))
        .route("/api/services", get(services))
        .route("/api/services/browse", post(browse))
        .route("/api/alerts", get(alerts))
        .route("/api/alerts/seen", post(alerts_seen))
        .route("/api/history", get(history))
        .merge(super::notify::router())
        .merge(super::explore::router())
}

fn who(u: &User) -> String {
    if u.display_name.is_empty() {
        u.username.clone()
    } else {
        u.display_name.clone()
    }
}

fn editor(me: &Session) -> ApiResult<()> {
    require(me.user.can_edit())
}

fn net(s: &Shared) -> &std::sync::Arc<Net> {
    &s.net
}

fn ip(s: &str) -> ApiResult<Ipv4Addr> {
    s.parse().map_err(|_| bad("not an ipv4 address"))
}

// ── Networks ──

async fn networks(State(s): State<Shared>, _me: Session) -> Json<Vec<Network>> {
    Json(net(&s).store.lock().unwrap().networks.clone())
}

fn check_network(n: &mut Network) -> ApiResult<()> {
    n.name = n.name.trim().to_string();
    if n.name.is_empty() {
        return Err(bad("the name cannot be empty"));
    }
    let r = strata_net::cidr::parse(&n.range).map_err(bad)?;
    n.range = r.to_string();
    if let Some(p) = n.ports.as_deref() {
        if p.trim().is_empty() {
            n.ports = None;
        } else {
            strata_net::ports::parse_list(p).map_err(bad)?;
        }
    }
    if n.vlan.is_some_and(|v| v == 0 || v > 4094) {
        return Err(bad("a vlan is a number from 1 to 4094"));
    }
    Ok(())
}

async fn create_network(State(s): State<Shared>, me: Session, Json(mut n): Json<Network>) -> ApiResult<Json<Network>> {
    editor(&me)?;
    check_network(&mut n)?;
    n.id = new_id();
    n.last_scan = None;
    net(&s).store.lock().unwrap().networks.push(n.clone());
    net(&s).save(&[File::Networks]);
    net(&s).changed("networks");
    net(&s).log(&who(&me.user), "created network", &n.name, &n.range);
    Ok(Json(n))
}

async fn update_network(State(s): State<Shared>, me: Session, Path(id): Path<String>, Json(mut n): Json<Network>) -> ApiResult<Json<Network>> {
    editor(&me)?;
    check_network(&mut n)?;
    {
        let mut st = net(&s).store.lock().unwrap();
        let cur = st.networks.iter_mut().find(|x| x.id == id).ok_or_else(|| not_found("network"))?;
        n.id = id;
        n.last_scan = cur.last_scan.clone();
        *cur = n.clone();
    }
    net(&s).save(&[File::Networks]);
    net(&s).changed("networks");
    net(&s).log(&who(&me.user), "changed network", &n.name, &n.range);
    Ok(Json(n))
}

async fn delete_network(State(s): State<Shared>, me: Session, Path(id): Path<String>) -> ApiResult<StatusCode> {
    editor(&me)?;
    net(&s).stop_scan(&id);
    let name = {
        let mut st = net(&s).store.lock().unwrap();
        let i = st.networks.iter().position(|x| x.id == id).ok_or_else(|| not_found("network"))?;
        st.networks.remove(i).name
    };
    net(&s).save(&[File::Networks]);
    net(&s).changed("networks");
    net(&s).log(&who(&me.user), "deleted network", &name, "");
    Ok(StatusCode::NO_CONTENT)
}

fn range_of(s: &Shared, id: &str) -> ApiResult<(Network, strata_net::cidr::Range)> {
    let st = net(s).store.lock().unwrap();
    let n = st.network(id).cloned().ok_or_else(|| not_found("network"))?;
    let r = strata_net::cidr::parse(&n.range).map_err(bad)?;
    Ok((n, r))
}

async fn addresses(State(s): State<Shared>, _me: Session, Path(id): Path<String>) -> ApiResult<Json<Vec<Address>>> {
    let (_, r) = range_of(&s, &id)?;
    let st = net(&s).store.lock().unwrap();
    Ok(Json(st.addresses.range(r.start..=r.end).map(|(_, a)| a.clone()).collect()))
}

async fn scan(State(s): State<Shared>, me: Session, Path(id): Path<String>) -> ApiResult<StatusCode> {
    editor(&me)?;
    net(&s).start_scan(&id, &who(&me.user)).map_err(|(c, m)| status(c, m))?;
    Ok(StatusCode::ACCEPTED)
}

async fn stop_scan(State(s): State<Shared>, me: Session, Path(id): Path<String>) -> ApiResult<StatusCode> {
    editor(&me)?;
    net(&s).stop_scan(&id);
    Ok(StatusCode::NO_CONTENT)
}

async fn next_free(State(s): State<Shared>, _me: Session, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (n, r) = range_of(&s, &id)?;
    let st = net(&s).store.lock().unwrap();
    let free = r.iter().find(|ip| Some(*ip) != n.gateway && st.addresses.get(&u32::from(*ip)).is_none_or(|a| a.state() == "free"));
    Ok(Json(json!({ "ip": free })))
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

async fn export_csv(State(s): State<Shared>, _me: Session, Path(id): Path<String>) -> ApiResult<impl IntoResponse> {
    let (n, r) = range_of(&s, &id)?;
    let st = net(&s).store.lock().unwrap();
    let mut out = String::from("ip,state,label,kind,name,mac,vendor,ports,notes,reserved,lastSeen\r\n");
    for ip in r.iter() {
        let a = st.addresses.get(&u32::from(ip)).cloned().unwrap_or_else(|| Address::new(ip));
        let ports: Vec<String> = a.ports.iter().map(|p| p.port.to_string()).collect();
        let last = a.last_seen.and_then(|t| jiff::Timestamp::from_millisecond(t).ok()).map(|t| t.to_string()).unwrap_or_default();
        let row = [
            ip.to_string(),
            a.state().into(),
            a.label.clone(),
            a.kind.clone().unwrap_or_default(),
            a.name.clone().unwrap_or_default(),
            a.mac.clone().unwrap_or_default(),
            a.vendor.clone().unwrap_or_default(),
            ports.join(" "),
            a.notes.clone(),
            a.reserved.to_string(),
            last,
        ];
        out.push_str(&row.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    let file = format!("attachment; filename=\"{}.csv\"", n.name.replace(['"', '/', '\\'], "-"));
    Ok(([(header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, file)], out))
}

/// Splits CSV text into rows of fields (quotes, doubled quotes, commas or semicolons).
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let sep = if text.lines().next().is_some_and(|l| l.matches(';').count() > l.matches(',').count()) { ';' } else { ',' };
    let mut rows = Vec::new();
    let (mut row, mut field, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == sep && !quoted => row.push(std::mem::take(&mut field)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                if row.iter().any(|f| !f.trim().is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            c => field.push(c),
        }
    }
    row.push(field);
    if row.iter().any(|f| !f.trim().is_empty()) {
        rows.push(row);
    }
    rows
}

async fn import_csv(State(s): State<Shared>, me: Session, Path(id): Path<String>, body: String) -> ApiResult<Json<Value>> {
    editor(&me)?;
    let (n, r) = range_of(&s, &id)?;
    let rows = parse_csv(&body);
    let header: Vec<String> = rows.first().ok_or_else(|| bad("the file is empty"))?.iter().map(|h| h.trim().to_ascii_lowercase()).collect();
    let col = |name: &str| header.iter().position(|h| h == name);
    let ip_col = col("ip").ok_or_else(|| bad("the file has no ip column"))?;
    let mut count = 0;
    {
        let mut st = net(&s).store.lock().unwrap();
        for row in rows.iter().skip(1) {
            let get = |c: Option<usize>| c.and_then(|c| row.get(c)).map(|v| v.trim().to_string());
            let Some(addr) = get(Some(ip_col)).and_then(|v| v.parse::<Ipv4Addr>().ok()).filter(|a| r.contains(*a)) else { continue };
            let a = st.address(addr);
            if let Some(v) = get(col("label")) {
                a.label = v;
            }
            if let Some(v) = get(col("kind")).filter(|v| KINDS.contains(&v.as_str())) {
                a.kind = Some(v);
            }
            if let Some(v) = get(col("notes")) {
                a.notes = v;
            }
            if let Some(v) = get(col("reserved")) {
                a.reserved = matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "oui" | "x");
            }
            count += 1;
        }
        st.prune();
    }
    net(&s).save(&[File::Addresses]);
    net(&s).changed("addresses");
    net(&s).log(&who(&me.user), "imported csv", &n.name, &format!("{count} addresses"));
    Ok(Json(json!({ "imported": count })))
}

// ── Addresses ──

async fn patch_address(State(s): State<Shared>, me: Session, Path(addr): Path<String>, Json(body): Json<HashMap<String, Value>>) -> ApiResult<Json<Address>> {
    let addr = ip(&addr)?;
    // Opening a new machine (isNew → false) is allowed to anyone signed in.
    if !(body.len() == 1 && body.contains_key("isNew")) {
        editor(&me)?;
    }
    let mut changes = Vec::new();
    let a = {
        let mut st = net(&s).store.lock().unwrap();
        let a = st.address(addr);
        for (k, v) in &body {
            let text = || v.as_str().map(|x| x.trim().to_string()).unwrap_or_default();
            match k.as_str() {
                "label" => a.label = text(),
                "notes" => a.notes = text(),
                "kind" => a.kind = v.as_str().filter(|k| KINDS.contains(k)).map(Into::into),
                "reserved" => a.reserved = v.as_bool().unwrap_or(false),
                "watched" => a.watched = v.as_bool().unwrap_or(false),
                "isNew" => a.is_new = v.as_bool().unwrap_or(false),
                "parent" => a.parent = v.as_str().and_then(|x| x.parse().ok()).filter(|p| *p != addr),
                "switch" => a.switch = serde_json::from_value(v.clone()).ok(),
                _ => return Err(bad(format!("unknown field {k}"))),
            }
            if k != "isNew" {
                changes.push(k.clone());
            }
        }
        let a = a.clone();
        st.prune();
        a
    };
    net(&s).save(&[File::Addresses]);
    net(&s).changed("addresses");
    if !changes.is_empty() {
        net(&s).log(&who(&me.user), "changed address", &addr.to_string(), &changes.join(", "));
    }
    Ok(Json(a))
}

async fn scan_address(State(s): State<Shared>, me: Session, Path(addr): Path<String>) -> ApiResult<Json<Address>> {
    editor(&me)?;
    let addr = ip(&addr)?;
    Ok(Json(net(&s).scan_one(addr, &who(&me.user)).await))
}

// ── VLANs ──

async fn vlans(State(s): State<Shared>, _me: Session) -> Json<Vec<Vlan>> {
    Json(net(&s).store.lock().unwrap().vlans.values().cloned().collect())
}

fn check_vlan(v: &mut Vlan) -> ApiResult<()> {
    if v.id == 0 || v.id > 4094 {
        return Err(bad("a vlan is a number from 1 to 4094"));
    }
    v.name = v.name.trim().to_string();
    if let Some(sub) = v.subnet.as_deref() {
        if sub.trim().is_empty() {
            v.subnet = None;
        } else {
            let (net, p) = strata_net::cidr::parse_block(sub).map_err(bad)?;
            v.subnet = Some(format!("{net}/{p}"));
        }
    }
    let ok = v.color.len() == 7 && v.color.starts_with('#') && v.color[1..].chars().all(|c| c.is_ascii_hexdigit());
    if !ok {
        return Err(bad("the colour is written #rrggbb"));
    }
    Ok(())
}

async fn create_vlan(State(s): State<Shared>, me: Session, Json(mut v): Json<Vlan>) -> ApiResult<Json<Vlan>> {
    editor(&me)?;
    check_vlan(&mut v)?;
    {
        let mut st = net(&s).store.lock().unwrap();
        if st.vlans.contains_key(&v.id) {
            return Err(status(409, "this vlan exists"));
        }
        st.vlans.insert(v.id, v.clone());
    }
    net(&s).save(&[File::Vlans]);
    net(&s).changed("vlans");
    net(&s).log(&who(&me.user), "created vlan", &format!("{} {}", v.id, v.name), "");
    Ok(Json(v))
}

async fn update_vlan(State(s): State<Shared>, me: Session, Path(id): Path<u16>, Json(mut v): Json<Vlan>) -> ApiResult<Json<Vlan>> {
    editor(&me)?;
    check_vlan(&mut v)?;
    {
        let mut st = net(&s).store.lock().unwrap();
        if !st.vlans.contains_key(&id) {
            return Err(not_found("vlan"));
        }
        if v.id != id {
            if st.vlans.contains_key(&v.id) {
                return Err(status(409, "this vlan exists"));
            }
            st.vlans.remove(&id);
            // Renumbered: ports and networks follow.
            for p in st.switches.iter_mut().flat_map(|s| s.ports.iter_mut()) {
                if p.vlan == Some(id) {
                    p.vlan = Some(v.id);
                }
                for t in p.tagged.iter_mut().filter(|t| **t == id) {
                    *t = v.id;
                }
            }
            for n in st.networks.iter_mut().filter(|n| n.vlan == Some(id)) {
                n.vlan = Some(v.id);
            }
        }
        st.vlans.insert(v.id, v.clone());
        net(&s).check_vlans(&mut st);
    }
    net(&s).save(&[File::Vlans, File::Switches, File::Networks, File::Alerts]);
    for w in ["vlans", "switches", "networks", "alerts"] {
        net(&s).changed(w);
    }
    net(&s).log(&who(&me.user), "changed vlan", &format!("{} {}", v.id, v.name), "");
    Ok(Json(v))
}

async fn delete_vlan(State(s): State<Shared>, me: Session, Path(id): Path<u16>) -> ApiResult<StatusCode> {
    editor(&me)?;
    let v = net(&s).store.lock().unwrap().vlans.remove(&id).ok_or_else(|| not_found("vlan"))?;
    net(&s).save(&[File::Vlans]);
    net(&s).changed("vlans");
    net(&s).log(&who(&me.user), "deleted vlan", &format!("{} {}", v.id, v.name), "");
    Ok(StatusCode::NO_CONTENT)
}

// ── Switches ──

async fn switches(State(s): State<Shared>, _me: Session) -> Json<Vec<Switch>> {
    Json(net(&s).store.lock().unwrap().switches.clone())
}

fn check_switch(sw: &mut Switch) -> ApiResult<()> {
    sw.name = sw.name.trim().to_string();
    if sw.name.is_empty() {
        return Err(bad("the name cannot be empty"));
    }
    sw.migrate();
    check_groups(&sw.groups)?;
    sw.normalize();
    Ok(())
}

fn check_groups(groups: &[PortGroup]) -> ApiResult<()> {
    let total: u32 = groups.iter().map(|g| g.count as u32).sum();
    if total == 0 {
        return Err(bad("a switch has at least one port"));
    }
    if total > MAX_PORTS as u32 {
        return Err(bad(format!("a switch has at most {MAX_PORTS} ports")));
    }
    Ok(())
}

async fn create_switch(State(s): State<Shared>, me: Session, Json(mut sw): Json<Switch>) -> ApiResult<Json<Switch>> {
    editor(&me)?;
    sw.id = new_id();
    check_switch(&mut sw)?;
    net(&s).store.lock().unwrap().switches.push(sw.clone());
    net(&s).save(&[File::Switches]);
    net(&s).changed("switches");
    net(&s).log(&who(&me.user), "created switch", &sw.name, &sw.model);
    Ok(Json(sw))
}

async fn update_switch(State(s): State<Shared>, me: Session, Path(id): Path<String>, Json(mut sw): Json<Switch>) -> ApiResult<Json<Switch>> {
    editor(&me)?;
    sw.id = id.clone();
    {
        let st = net(&s).store.lock().unwrap();
        let cur = st.switches.iter().find(|x| x.id == id).ok_or_else(|| not_found("switch"))?;
        if sw.ports.is_empty() {
            sw.ports = cur.ports.clone();
        }
        // Online is changed by its own route; without an agent the switch cannot stay online.
        sw.online = cur.online && sw.snmp.is_some();
        sw.online_secs = cur.online_secs;
        // Rules too (their own route).
        sw.rules = cur.rules.clone();
    }
    let was_online = net(&s).live.lock().unwrap().contains_key(&id);
    check_switch(&mut sw)?;
    {
        let mut st = net(&s).store.lock().unwrap();
        if let Some(cur) = st.switches.iter_mut().find(|x| x.id == id) {
            *cur = sw.clone();
        }
        net(&s).check_vlans(&mut st);
    }
    net(&s).save(&[File::Switches, File::Alerts]);
    net(&s).changed("switches");
    net(&s).changed("alerts");
    net(&s).log(&who(&me.user), "changed switch", &sw.name, "");
    if was_online && !sw.online {
        net(&s).stop_live(&id);
        net(&s).log(&who(&me.user), "switch offline", &sw.name, "");
    } else {
        net(&s).live_changed(&id);
    }
    Ok(Json(sw))
}

async fn delete_switch(State(s): State<Shared>, me: Session, Path(id): Path<String>) -> ApiResult<StatusCode> {
    editor(&me)?;
    let name = {
        let mut st = net(&s).store.lock().unwrap();
        let i = st.switches.iter().position(|x| x.id == id).ok_or_else(|| not_found("switch"))?;
        let name = st.switches.remove(i).name;
        for p in st.switches.iter_mut().flat_map(|s| s.ports.iter_mut()) {
            if p.peer.as_ref().is_some_and(|r| r.id == id) {
                p.peer = None;
            }
        }
        for a in st.addresses.values_mut() {
            if a.switch.as_ref().is_some_and(|r| r.id == id) {
                a.switch = None;
            }
        }
        name
    };
    net(&s).stop_live(&id);
    net(&s).forget_rates(&id);
    net(&s).save(&[File::Switches, File::Addresses]);
    net(&s).changed("switches");
    net(&s).log(&who(&me.user), "deleted switch", &name, "");
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct PortsPatch {
    ports: Vec<u16>,
    set: HashMap<String, Value>,
}

async fn patch_ports(State(s): State<Shared>, me: Session, Path(id): Path<String>, Json(body): Json<PortsPatch>) -> ApiResult<Json<Switch>> {
    editor(&me)?;
    let (sw, detail) = {
        let mut st = net(&s).store.lock().unwrap();
        let vlans: Vec<u16> = st.vlans.keys().copied().collect();
        let sw = st.switches.iter_mut().find(|x| x.id == id).ok_or_else(|| not_found("switch"))?;
        let mut names = Vec::new();
        for p in sw.ports.iter_mut().filter(|p| body.ports.contains(&p.n)) {
            names.push(p.name.clone());
            for (k, v) in &body.set {
                match k.as_str() {
                    "label" => p.label = v.as_str().unwrap_or("").trim().to_string(),
                    "mode" => {
                        let m = v.as_str().unwrap_or("");
                        if !matches!(m, "access" | "trunk" | "off") {
                            return Err(bad("the mode is access, trunk or off"));
                        }
                        p.mode = m.into();
                    }
                    "vlan" => p.vlan = v.as_u64().map(|x| x as u16).filter(|x| vlans.contains(x)),
                    "tagged" => {
                        let mut t: Vec<u16> = serde_json::from_value(v.clone()).map_err(bad)?;
                        t.retain(|x| vlans.contains(x));
                        t.sort_unstable();
                        t.dedup();
                        p.tagged = t;
                    }
                    "host" => p.host = v.as_str().and_then(|x| x.parse().ok()),
                    "peer" => p.peer = serde_json::from_value(v.clone()).ok(),
                    "watched" => p.watched = v.as_bool().unwrap_or(false),
                    _ => return Err(bad(format!("unknown field {k}"))),
                }
            }
        }
        let sw = sw.clone();
        // Plugging a machine on a port: its address points back to the port.
        if let Some(Value::String(h)) = body.set.get("host") {
            if let (Ok(h), Some(n)) = (h.parse::<Ipv4Addr>(), body.ports.first()) {
                st.address(h).switch = Some(PortRef { id: id.clone(), port: *n });
            }
        }
        net(&s).check_vlans(&mut st);
        let keys: Vec<&str> = body.set.keys().map(String::as_str).collect();
        (sw, format!("{} · {}", names.join(", "), keys.join(", ")))
    };
    net(&s).save(&[File::Switches, File::Addresses, File::Alerts]);
    net(&s).changed("switches");
    net(&s).changed("addresses");
    net(&s).changed("alerts");
    net(&s).log(&who(&me.user), "changed port", &sw.name, &detail);
    net(&s).live_changed(&id);
    Ok(Json(sw))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnlineSet {
    online: Option<bool>,
    online_secs: Option<u32>,
}

async fn set_online(State(s): State<Shared>, me: Session, Path(id): Path<String>, Json(b): Json<OnlineSet>) -> ApiResult<StatusCode> {
    editor(&me)?;
    if b.online_secs.is_some_and(|x| !ONLINE_SECS.contains(&x)) {
        return Err(bad("the read interval is 5, 10, 30 or 60 seconds"));
    }
    let (name, was, on, secs) = {
        let mut st = net(&s).store.lock().unwrap();
        let sw = st.switches.iter_mut().find(|x| x.id == id).ok_or_else(|| not_found("switch"))?;
        let on = b.online.unwrap_or(sw.online);
        if on && sw.snmp.is_none() {
            return Err(bad("the switch has no snmp agent"));
        }
        let was = (sw.online, sw.online_secs);
        sw.online = on;
        sw.online_secs = b.online_secs.unwrap_or(sw.online_secs);
        (sw.name.clone(), was, on, sw.online_secs)
    };
    net(&s).save(&[File::Switches]);
    net(&s).changed("switches");
    let every = format!("every {secs} s");
    if on && (!was.0 || was.1 != secs) {
        net(&s).start_live(&id);
    } else if !on && was.0 {
        net(&s).stop_live(&id);
    }
    match (was.0, on) {
        (false, true) => net(&s).log(&who(&me.user), "switch online", &name, &every),
        (true, false) => net(&s).log(&who(&me.user), "switch offline", &name, ""),
        _ if was.1 != secs => net(&s).log(&who(&me.user), "changed read interval", &name, &every),
        _ => {}
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn switch_live(State(s): State<Shared>, _me: Session, Path(id): Path<String>) -> ApiResult<Json<super::live::Live>> {
    net(&s).live_of(&id).map(Json).ok_or_else(|| status(404, "the switch is not online"))
}

#[derive(Deserialize)]
struct Take {
    ports: Option<Vec<u16>>,
}

async fn take_from_switch(State(s): State<Shared>, me: Session, Path(id): Path<String>, Json(b): Json<Take>) -> ApiResult<Json<Switch>> {
    editor(&me)?;
    let live = net(&s).live_of(&id).ok_or_else(|| status(404, "the switch is not online"))?;
    if live.vlans.is_none() {
        return Err(status(409, "the vlan setup of the switch is not known yet"));
    }
    let (sw, names) = {
        let mut st = net(&s).store.lock().unwrap();
        let known: Vec<u16> = st.vlans.keys().copied().collect();
        let sw = st.switches.iter_mut().find(|x| x.id == id).ok_or_else(|| not_found("switch"))?;
        let names = super::live::take(sw, &live, b.ports.as_deref(), &known);
        let sw = sw.clone();
        net(&s).check_vlans(&mut st);
        (sw, names)
    };
    net(&s).save(&[File::Switches, File::Alerts]);
    net(&s).changed("switches");
    net(&s).changed("alerts");
    net(&s).log(&who(&me.user), "took config from switch", &sw.name, &names.join(", "));
    net(&s).live_changed(&id);
    Ok(Json(sw))
}

async fn switch_info(State(s): State<Shared>, _me: Session, Path(id): Path<String>) -> ApiResult<Json<super::info::InfoView>> {
    net(&s).info_of(&id).map(Json).ok_or_else(|| status(404, "the switch is not online"))
}

#[derive(Deserialize)]
struct RangeQ {
    range: Option<String>,
}

async fn port_history(State(s): State<Shared>, _me: Session, Path((id, port)): Path<(String, u16)>, Query(q): Query<RangeQ>) -> ApiResult<Json<super::rates::History>> {
    let range = super::rates::Range::parse(q.range.as_deref().unwrap_or("1h")).ok_or_else(|| bad("the range is 1h, 24h or 7d"))?;
    let secs = {
        let st = net(&s).store.lock().unwrap();
        let sw = st.switches.iter().find(|x| x.id == id).ok_or_else(|| not_found("switch"))?;
        if !sw.ports.iter().any(|p| p.n == port) {
            return Err(not_found("port"));
        }
        sw.online_secs
    };
    let now = crate::paths::now_ms();
    Ok(Json(net(&s).with_rates(&id, |r| super::rates::history(r.ports.get(&port), range, now, secs))))
}

async fn set_rules(State(s): State<Shared>, me: Session, Path(id): Path<String>, Json(mut rules): Json<AlertRules>) -> ApiResult<Json<Switch>> {
    editor(&me)?;
    rules.clamp();
    let sw = {
        let mut st = net(&s).store.lock().unwrap();
        let sw = st.switches.iter_mut().find(|x| x.id == id).ok_or_else(|| not_found("switch"))?;
        sw.rules = rules;
        sw.clone()
    };
    net(&s).save(&[File::Switches]);
    net(&s).changed("switches");
    net(&s).log(&who(&me.user), "changed alert rules", &sw.name, "");
    Ok(Json(sw))
}

async fn link_neighbours(State(s): State<Shared>, me: Session, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    editor(&me)?;
    let made = net(&s).link_neighbours(&id).ok_or_else(|| status(404, "the switch is not online"))?;
    if !made.is_empty() {
        let name = net(&s).store.lock().unwrap().switches.iter().find(|x| x.id == id).map(|x| x.name.clone()).unwrap_or_default();
        net(&s).save(&[File::Switches]);
        net(&s).changed("switches");
        let detail = made.iter().map(|(a, b, c)| format!("{a} ↔ {b} {c}")).collect::<Vec<_>>().join(", ");
        net(&s).log(&who(&me.user), "linked switches", &name, &detail);
        net(&s).send_info(&id);
    }
    Ok(Json(json!({ "linked": made.len() })))
}

// ── Switch models and detection ──

async fn switch_models(State(s): State<Shared>, _me: Session) -> Json<Value> {
    let mine = net(&s).store.lock().unwrap().templates.clone();
    Json(json!({"builtin": super::models::catalog(), "mine": mine}))
}

async fn create_switch_model(State(s): State<Shared>, me: Session, Json(mut m): Json<SwitchModel>) -> ApiResult<Json<SwitchModel>> {
    editor(&me)?;
    m.brand = m.brand.trim().to_string();
    m.model = m.model.trim().to_string();
    if m.model.is_empty() {
        return Err(bad("the name cannot be empty"));
    }
    check_groups(&m.groups)?;
    clean_groups(&mut m.groups);
    m.id = new_id();
    m.matches.clear();
    net(&s).store.lock().unwrap().templates.push(m.clone());
    net(&s).save(&[File::Templates]);
    net(&s).changed("switchModels");
    net(&s).log(&who(&me.user), "saved switch model", &m.model, "");
    Ok(Json(m))
}

async fn delete_switch_model(State(s): State<Shared>, me: Session, Path(id): Path<String>) -> ApiResult<StatusCode> {
    editor(&me)?;
    let name = {
        let mut st = net(&s).store.lock().unwrap();
        let i = st.templates.iter().position(|x| x.id == id).ok_or_else(|| not_found("switch model"))?;
        st.templates.remove(i).model
    };
    net(&s).save(&[File::Templates]);
    net(&s).changed("switchModels");
    net(&s).log(&who(&me.user), "deleted switch model", &name, "");
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Detect {
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    ip: Option<Ipv4Addr>,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    community: String,
    #[serde(default)]
    v3: Option<strata_net::snmpv3::V3>,
}

/// Reads a switch (identity and interfaces) and proposes its faceplate.
async fn detect_switch(State(s): State<Shared>, me: Session, Json(d): Json<Detect>) -> ApiResult<Json<Value>> {
    editor(&me)?;
    let a = {
        let st = net(&s).store.lock().unwrap();
        let known = d.agent_id.as_deref().map(|id| st.snmp.iter().find(|x| x.id == id).ok_or_else(|| not_found("snmp agent"))).transpose()?;
        match (d.ip, known) {
            // An agent chosen as it is.
            (None, Some(k)) => k.clone(),
            (None, None) => return Err(bad("not an ipv4 address")),
            (Some(ip), known) => {
                let mut a = SnmpAgent {
                    id: String::new(),
                    name: String::new(),
                    ip,
                    port: if d.port == 0 { 161 } else { d.port },
                    community: d.community,
                    v3: d.v3,
                    read_arp: false,
                    read_multicast: false,
                    read_switch: false,
                    last_poll: None,
                };
                unmask(&mut a, known);
                a
            }
        }
    };
    if let Some(v3) = &a.v3 {
        v3.check().map_err(bad)?;
    }
    let sess = super::snmp::session(&a).await.map_err(|e| status(502, e))?;
    let system = strata_net::snmp::system(&sess).await.map_err(|e| status(502, e.to_string()))?;
    let ifs = strata_net::snmp::if_list(&sess).await.map_err(|e| status(502, e.to_string()))?;
    let ports = super::models::physical(&ifs);
    let matched = super::models::recognise(&[&system.model, &system.description]);
    let (model, groups) = match matched {
        Some(m) => (format!("{} {}", m.brand, m.model), m.groups.clone()),
        None => (system.model.clone(), super::models::guess_groups(&ports)),
    };
    let mut groups = groups;
    clean_groups(&mut groups);
    Ok(Json(json!({
        "system": {
            "name": system.name,
            "description": system.description,
            "model": system.model,
            "serial": system.serial,
            "firmware": system.firmware,
            "location": system.location,
        },
        "interfaces": ports.len(),
        "names": ports.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        "suggestion": {"name": system.name, "model": model, "groups": groups, "matched": matched.map(|m| m.id.as_str())},
    })))
}

// ── SNMP ──

fn masked(mut a: SnmpAgent) -> SnmpAgent {
    if !a.community.is_empty() {
        a.community = MASKED.into();
    }
    if let Some(v3) = a.v3.as_mut() {
        for pw in [&mut v3.auth_password, &mut v3.priv_password] {
            if !pw.is_empty() {
                *pw = MASKED.into();
            }
        }
    }
    a
}

/// Secrets sent back masked keep their stored value.
fn unmask(a: &mut SnmpAgent, cur: Option<&SnmpAgent>) {
    let Some(cur) = cur else { return };
    if a.community == MASKED {
        a.community = cur.community.clone();
    }
    if let (Some(v3), Some(old)) = (a.v3.as_mut(), cur.v3.as_ref()) {
        if v3.auth_password == MASKED {
            v3.auth_password = old.auth_password.clone();
        }
        if v3.priv_password == MASKED {
            v3.priv_password = old.priv_password.clone();
        }
    }
}

async fn snmp_list(State(s): State<Shared>, _me: Session) -> Json<Vec<SnmpAgent>> {
    Json(net(&s).store.lock().unwrap().snmp.iter().cloned().map(masked).collect())
}

fn check_snmp(a: &mut SnmpAgent) -> ApiResult<()> {
    a.name = a.name.trim().to_string();
    if a.name.is_empty() {
        return Err(bad("the name cannot be empty"));
    }
    if a.port == 0 {
        a.port = 161;
    }
    Ok(())
}

async fn create_snmp(State(s): State<Shared>, me: Session, Json(mut a): Json<SnmpAgent>) -> ApiResult<Json<SnmpAgent>> {
    require(me.user.is_admin())?;
    check_snmp(&mut a)?;
    if let Some(v3) = &a.v3 {
        v3.check().map_err(bad)?;
    }
    a.id = new_id();
    a.last_poll = None;
    net(&s).store.lock().unwrap().snmp.push(a.clone());
    net(&s).save(&[File::Snmp]);
    net(&s).changed("snmp");
    net(&s).log(&who(&me.user), "created snmp agent", &a.name, &a.ip.to_string());
    let n = net(&s).clone();
    let id = a.id.clone();
    tokio::spawn(async move { n.poll(&id).await });
    Ok(Json(masked(a)))
}

async fn update_snmp(State(s): State<Shared>, me: Session, Path(id): Path<String>, Json(mut a): Json<SnmpAgent>) -> ApiResult<Json<SnmpAgent>> {
    require(me.user.is_admin())?;
    check_snmp(&mut a)?;
    {
        let mut st = net(&s).store.lock().unwrap();
        let cur = st.snmp.iter_mut().find(|x| x.id == id).ok_or_else(|| not_found("snmp agent"))?;
        unmask(&mut a, Some(cur));
        if let Some(v3) = &a.v3 {
            v3.check().map_err(bad)?;
        }
        a.id = id.clone();
        a.last_poll = cur.last_poll.clone();
        *cur = a.clone();
    }
    net(&s).save(&[File::Snmp]);
    net(&s).changed("snmp");
    net(&s).log(&who(&me.user), "changed snmp agent", &a.name, &a.ip.to_string());
    Ok(Json(masked(a)))
}

async fn delete_snmp(State(s): State<Shared>, me: Session, Path(id): Path<String>) -> ApiResult<StatusCode> {
    require(me.user.is_admin())?;
    let mut offline = Vec::new();
    let name = {
        let mut st = net(&s).store.lock().unwrap();
        let i = st.snmp.iter().position(|x| x.id == id).ok_or_else(|| not_found("snmp agent"))?;
        for sw in st.switches.iter_mut().filter(|w| w.snmp.as_deref() == Some(&id)) {
            sw.snmp = None;
            if sw.online {
                sw.online = false;
                offline.push(sw.id.clone());
            }
        }
        st.snmp.remove(i).name
    };
    for sw in offline {
        net(&s).stop_live(&sw);
    }
    net(&s).save(&[File::Snmp, File::Switches]);
    net(&s).changed("snmp");
    net(&s).changed("switches");
    net(&s).log(&who(&me.user), "deleted snmp agent", &name, "");
    Ok(StatusCode::NO_CONTENT)
}

async fn snmp_poll(State(s): State<Shared>, me: Session, Path(id): Path<String>) -> ApiResult<Json<SnmpAgent>> {
    editor(&me)?;
    let a = net(&s).poll(&id).await.ok_or_else(|| not_found("snmp agent"))?;
    Ok(Json(masked(a)))
}

#[derive(Deserialize)]
struct SnmpTest {
    ip: Ipv4Addr,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    community: String,
    #[serde(default)]
    v3: Option<strata_net::snmpv3::V3>,
    #[serde(default)]
    id: Option<String>,
}

async fn snmp_test(State(s): State<Shared>, me: Session, Json(t): Json<SnmpTest>) -> ApiResult<Json<strata_net::snmp::Identity>> {
    require(me.user.is_admin())?;
    let mut a = SnmpAgent {
        id: String::new(),
        name: String::new(),
        ip: t.ip,
        port: if t.port == 0 { 161 } else { t.port },
        community: t.community,
        v3: t.v3,
        read_arp: false,
        read_multicast: false,
        read_switch: false,
        last_poll: None,
    };
    {
        let st = net(&s).store.lock().unwrap();
        unmask(&mut a, t.id.as_deref().and_then(|id| st.snmp.iter().find(|x| x.id == id)));
    }
    if let Some(v3) = &a.v3 {
        v3.check().map_err(bad)?;
    }
    let sess = super::snmp::session(&a).await.map_err(|e| status(502, e))?;
    strata_net::snmp::identity(&sess).await.map(Json).map_err(|e| status(502, e.to_string()))
}

// ── Multicast and services ──

async fn multicast(State(s): State<Shared>, _me: Session) -> Json<Value> {
    let n = net(&s);
    let (iface, max, labels) = {
        let st = n.store.lock().unwrap();
        (st.multicast.iface.clone(), st.multicast.check_max_mbps, st.multicast.labels.clone())
    };
    let streams: Vec<Value> = n
        .mcast
        .lock()
        .unwrap()
        .as_ref()
        .map(|e| e.snapshot())
        .unwrap_or_default()
        .into_iter()
        .map(|st| {
            let key = format!("{}:{}:{}", st.group, st.port, st.source.map(|x| x.to_string()).unwrap_or_default());
            let mut v = serde_json::to_value(&st).unwrap_or_default();
            v["label"] = json!(labels.get(&key).cloned().unwrap_or_default());
            v
        })
        .collect();
    let ifaces: Vec<Value> = super::interfaces().into_iter().map(|(name, ip)| json!({"name": name, "ip": ip})).collect();
    let current = n.mcast.lock().unwrap().as_ref().map(|e| e.iface());
    let iface = if iface.is_empty() { current.map(|i| i.to_string()).unwrap_or_default() } else { iface };
    Json(json!({ "iface": iface, "ifaces": ifaces, "streams": streams, "checkMaxMbps": max }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct McastSet {
    iface: String,
    check_max_mbps: u64,
}

async fn multicast_settings(State(s): State<Shared>, me: Session, Json(b): Json<McastSet>) -> ApiResult<StatusCode> {
    require(me.user.is_admin())?;
    {
        let mut st = net(&s).store.lock().unwrap();
        st.multicast.iface = b.iface.clone();
        st.multicast.check_max_mbps = b.check_max_mbps.clamp(1, 10_000);
    }
    net(&s).save(&[File::Multicast]);
    super::multicast::start(net(&s));
    net(&s).changed("multicast");
    net(&s).log(&who(&me.user), "changed multicast settings", &b.iface, &format!("{} Mb/s", b.check_max_mbps));
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct McastTarget {
    group: Ipv4Addr,
    port: u16,
}

async fn multicast_check(State(s): State<Shared>, me: Session, Json(t): Json<McastTarget>) -> ApiResult<Json<strata_net::mcast::Check>> {
    editor(&me)?;
    if t.port == 0 {
        return Err(bad("the port of this stream is not known"));
    }
    net(&s).check_guard(t.group, t.port)?;
    let max = net(&s).store.lock().unwrap().multicast.check_max_mbps;
    let engine = net(&s).mcast.lock().unwrap().clone().ok_or_else(|| status(503, "no network interface"))?;
    let opts = strata_net::mcast::CheckOptions { max_ms: 2000, max_mbps: max };
    let r = engine.check(t.group, t.port, &opts).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::WouldBlock {
            status(409, e.to_string())
        } else {
            bad(e)
        }
    })?;
    net(&s).changed("multicast");
    net(&s).log(&who(&me.user), "checked stream", &format!("{}:{}", t.group, t.port), if r.received { "received" } else { "nothing" });
    Ok(Json(r))
}

#[derive(Deserialize)]
struct McastLabel {
    group: Ipv4Addr,
    port: u16,
    #[serde(default)]
    source: Option<Ipv4Addr>,
    label: String,
}

async fn multicast_label(State(s): State<Shared>, me: Session, Json(b): Json<McastLabel>) -> ApiResult<StatusCode> {
    editor(&me)?;
    let key = format!("{}:{}:{}", b.group, b.port, b.source.map(|x| x.to_string()).unwrap_or_default());
    {
        let mut st = net(&s).store.lock().unwrap();
        if b.label.trim().is_empty() {
            st.multicast.labels.remove(&key);
        } else {
            st.multicast.labels.insert(key.clone(), b.label.trim().to_string());
        }
    }
    net(&s).save(&[File::Multicast]);
    net(&s).changed("multicast");
    net(&s).log(&who(&me.user), "labelled stream", &key, b.label.trim());
    Ok(StatusCode::NO_CONTENT)
}

async fn services(State(s): State<Shared>, _me: Session) -> Json<Value> {
    let (at, list) = net(&s).services.lock().unwrap().clone();
    Json(json!({ "at": at, "services": list }))
}

async fn browse(State(s): State<Shared>, me: Session) -> ApiResult<StatusCode> {
    editor(&me)?;
    let n = net(&s).clone();
    tokio::spawn(async move { n.browse().await });
    Ok(StatusCode::ACCEPTED)
}

// ── Alerts and history ──

async fn alerts(State(s): State<Shared>, _me: Session) -> Json<Vec<Alert>> {
    Json(net(&s).store.lock().unwrap().alerts.clone())
}

#[derive(Deserialize)]
struct Seen {
    ids: Option<Vec<String>>,
}

async fn alerts_seen(State(s): State<Shared>, _me: Session, Json(b): Json<Seen>) -> ApiResult<StatusCode> {
    {
        let mut st = net(&s).store.lock().unwrap();
        for a in st.alerts.iter_mut() {
            if b.ids.as_ref().is_none_or(|ids| ids.contains(&a.id)) {
                a.seen = true;
            }
        }
    }
    net(&s).save(&[File::Alerts]);
    net(&s).changed("alerts");
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Limit {
    limit: Option<usize>,
}

async fn history(State(s): State<Shared>, me: Session, Query(q): Query<Limit>) -> ApiResult<Json<Vec<HistoryEntry>>> {
    require(me.user.is_admin())?;
    let st = net(&s).store.lock().unwrap();
    Ok(Json(st.history.iter().rev().take(q.limit.unwrap_or(2000).min(20_000)).cloned().collect()))
}

#[cfg(test)]
mod tests {
    use super::parse_csv;

    #[test]
    fn csv_rows() {
        let r = parse_csv("ip,label\r\n10.0.0.1,\"PTZ, plateau \"\"B\"\"\"\r\n\r\n10.0.0.2,NAS\n");
        assert_eq!(r.len(), 3);
        assert_eq!(r[1][1], "PTZ, plateau \"B\"");
        assert_eq!(r[2], vec!["10.0.0.2", "NAS"]);
        let fr = parse_csv("ip;label\n10.0.0.3;Caméra");
        assert_eq!(fr[1][1], "Caméra");
    }
}
