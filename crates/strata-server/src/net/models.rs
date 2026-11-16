//! Switch models: the built-in catalog, recognising a switch from what it
//! says over SNMP, and guessing a faceplate from its interfaces.

use std::cmp::Ordering;
use std::sync::OnceLock;

use strata_net::snmp::IfInfo;

use super::model::{group_starts, PortGroup, PortKind, SwitchModel};

/// The models compiled into the server.
pub fn catalog() -> &'static [SwitchModel] {
    static CATALOG: OnceLock<Vec<SwitchModel>> = OnceLock::new();
    CATALOG.get_or_init(|| serde_json::from_str(include_str!("switch-models.json")).expect("switch-models.json"))
}

/// `needle` in `hay` (ignoring case) as a whole word: no letter or digit right before or after.
fn has_word(hay: &str, needle: &str) -> bool {
    let (hay, needle) = (hay.to_ascii_uppercase(), needle.to_ascii_uppercase());
    let alnum = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    hay.match_indices(&needle).any(|(i, _)| !alnum(hay[..i].chars().next_back()) && !alnum(hay[i + needle.len()..].chars().next()))
}

/// The catalog entry named in these texts (ENTITY model, sysDescr); the longest match wins.
pub fn recognise(texts: &[&str]) -> Option<&'static SwitchModel> {
    catalog()
        .iter()
        .flat_map(|m| m.matches.iter().map(move |k| (m, k)))
        .filter(|(_, k)| texts.iter().any(|t| has_word(t, k)))
        .max_by_key(|(_, k)| k.len())
        .map(|(m, _)| m)
}

/// Letters and numbers compared as numbers: `Eth1/2` < `Eth1/10`.
pub fn natural(a: &str, b: &str) -> Ordering {
    fn chunks(s: &str) -> Vec<(bool, &str)> {
        let mut out = Vec::new();
        let mut start = 0;
        let b = s.as_bytes();
        for i in 1..=b.len() {
            if i == b.len() || b[i].is_ascii_digit() != b[start].is_ascii_digit() {
                out.push((b[start].is_ascii_digit(), &s[start..i]));
                start = i;
            }
        }
        out
    }
    let (x, y) = (chunks(a), chunks(b));
    for (p, q) in x.iter().zip(y.iter()) {
        let o = match (p, q) {
            ((true, m), (true, n)) => {
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                m.len().cmp(&n.len()).then_with(|| m.cmp(n))
            }
            ((_, m), (_, n)) => m.to_ascii_lowercase().cmp(&n.to_ascii_lowercase()),
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    x.len().cmp(&y.len())
}

/// Not a port of the faceplate: VLAN interfaces, port-channels, loopbacks, management…
fn virtual_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let digit_after = |p: &str| n.strip_prefix(p).is_some_and(|r| r.is_empty() || r.starts_with(|c: char| c.is_ascii_digit()));
    ["vlan", "port-channel", "loopback", "mgmt", "null", "tunnel", "nve", "sup-eth", "bridge-aggregation", "ae"].iter().any(|p| n.starts_with(p) && (*p != "ae" || digit_after("ae")))
        || digit_after("po")
        || digit_after("lo")
        || digit_after("trk")
}

/// Names that look like an Ethernet port.
fn ethernet_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let starts_num = |p: &str| n.strip_prefix(p).is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()));
    ["ethernet", "fastethernet", "gigabitethernet", "tengigabitethernet", "twentyfivegige", "fortygigabitethernet", "hundredgige", "ether", "sfp"].iter().any(|p| n.starts_with(p))
        || ["eth", "gi", "te", "twe", "fo", "hu", "fa", "xe-", "ge-", "et-", "mge-"].iter().any(|p| starts_num(p) || (p.ends_with('-') && n.starts_with(p)))
}

/// The interfaces that are ports of the faceplate, in natural order.
pub fn physical(ifs: &[IfInfo]) -> Vec<IfInfo> {
    let mut out: Vec<IfInfo> = ifs
        .iter()
        .filter(|i| !virtual_name(&i.name) && (matches!(i.if_type, 6 | 62 | 69 | 117) || (i.if_type == 0 && ethernet_name(&i.name))))
        .cloned()
        .collect();
    out.sort_by(|a, b| natural(&a.name, &b.name));
    out
}

/// `Ethernet1/12` → (`Ethernet1/{n}`, 12).
fn pattern(name: &str) -> (String, Option<u32>) {
    let digits = name.len() - name.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    match name[name.len() - digits..].parse::<u32>() {
        Ok(n) if digits > 0 => (format!("{}{{n}}", &name[..name.len() - digits]), Some(n)),
        _ => (name.to_string(), None),
    }
}

fn kind_of(speed_mbps: u64) -> Option<PortKind> {
    match speed_mbps {
        0 => None,
        s if s >= 40_000 => Some(PortKind::Qsfp),
        s if s >= 10_000 => Some(PortKind::Sfp),
        _ => Some(PortKind::Rj45),
    }
}

/// A faceplate from the ports: the same naming pattern, following numbers and the same speed class make a group.
pub fn guess_groups(ports: &[IfInfo]) -> Vec<PortGroup> {
    let mut groups: Vec<PortGroup> = Vec::new();
    let mut last: Option<u32> = None;
    for p in ports {
        let (naming, num) = pattern(&p.name);
        let prev = groups.last();
        // A port whose speed is not known (link down on some switches) goes with the one before.
        let kind = kind_of(p.speed_mbps).or(prev.map(|g| g.kind)).unwrap_or_default();
        let follows = prev.is_some_and(|g| g.naming == naming && g.kind == kind && num.is_some() && num == last.map(|x| x + 1));
        if follows {
            groups.last_mut().unwrap().count += 1;
        } else {
            groups.push(PortGroup { kind, count: 1, naming, start: num.and_then(|n| u16::try_from(n).ok()) });
        }
        last = num;
    }
    // Keep `start` only where the default rule does not give it already.
    let mut out: Vec<PortGroup> = Vec::with_capacity(groups.len());
    for mut g in groups {
        let explicit = g.start;
        g.start = None;
        out.push(g);
        let default = *group_starts(&out).last().unwrap();
        if explicit.is_some_and(|s| s as u32 != default) {
            out.last_mut().unwrap().start = explicit;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::model::port_names;

    fn ifc(name: &str, if_type: u32, speed: u64) -> IfInfo {
        IfInfo { if_index: 0, name: name.into(), if_type, speed_mbps: speed }
    }

    #[test]
    fn catalog_loads_and_names_are_sane() {
        let c = catalog();
        assert!(c.len() >= 20);
        let mut ids: Vec<&str> = c.iter().map(|m| m.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), c.len());
        let n = c.iter().find(|m| m.id == "cisco-n9k-c93240yc-fx2").unwrap();
        assert_eq!(port_names(&n.groups).last().unwrap().0, "Eth1/60");
        let x = c.iter().find(|m| m.model == "Catalyst 2960X-48FPS-L").unwrap();
        assert_eq!(port_names(&x.groups)[48].0, "Gi1/0/49");
        assert!(c.iter().all(|m| !m.matches.is_empty() && !m.groups.is_empty()));
    }

    #[test]
    fn recognises_models() {
        assert_eq!(recognise(&["N9K-C93240YC-FX2", "Cisco NX-OS(tm) nxos.10.2.5"]).unwrap().id, "cisco-n9k-c93240yc-fx2");
        assert_eq!(recognise(&["N9K-C93180YC-FX"]).unwrap().id, "cisco-n9k-c93180yc-fx");
        assert!(recognise(&["N9K-C93180YC-FX3"]).is_none());
        assert_eq!(recognise(&["C9300-48P"]).unwrap().id, "cisco-c9300-48p");
        assert!(recognise(&["C9300-48PXG"]).is_none());
        assert_eq!(recognise(&["WS-C2960X-48FPS-L"]).unwrap().model, "Catalyst 2960X-48FPS-L");
        assert_eq!(recognise(&["", "RouterOS CRS326-24G-2S+"]).unwrap().brand, "MikroTik");
        assert!(recognise(&["Linux test 6.1", ""]).is_none());
    }

    #[test]
    fn keeps_ethernet_ports_in_order() {
        let ifs = vec![
            ifc("Ethernet1/10", 6, 25_000),
            ifc("mgmt0", 6, 1000),
            ifc("Vlan1", 53, 1000),
            ifc("port-channel1", 53, 100_000),
            ifc("loopback0", 24, 0),
            ifc("Ethernet1/2", 6, 25_000),
            ifc("Po12", 53, 0),
            ifc("Null0", 1, 0),
            ifc("Gi1/0/3", 0, 0),
            ifc("sup-eth1", 6, 1000),
            ifc("eth0", 6, 10_000),
            ifc("lo", 24, 10),
        ];
        let names: Vec<String> = physical(&ifs).into_iter().map(|i| i.name).collect();
        assert_eq!(names, vec!["eth0", "Ethernet1/2", "Ethernet1/10", "Gi1/0/3"]);
        assert_eq!(natural("Eth1/2", "Eth1/10"), Ordering::Less);
        assert_eq!(natural("ge-0/0/9", "ge-0/0/10"), Ordering::Less);
    }

    #[test]
    fn guesses_groups_by_speed() {
        let mut ifs: Vec<IfInfo> = (1..=48).map(|n| ifc(&format!("Ethernet1/{n}"), 6, 25_000)).collect();
        ifs.extend((49..=54).map(|n| ifc(&format!("Ethernet1/{n}"), 6, 100_000)));
        let g = guess_groups(&ifs);
        assert_eq!(g, vec![PortGroup::new(PortKind::Sfp, 48, "Ethernet1/{n}"), PortGroup::new(PortKind::Qsfp, 6, "Ethernet1/{n}")]);
        let mut c: Vec<IfInfo> = (1..=24).map(|n| ifc(&format!("Gi1/0/{n}"), 6, if n == 5 { 0 } else { 1000 })).collect();
        c.extend((1..=4).map(|n| ifc(&format!("Te1/1/{n}"), 6, 10_000)));
        assert_eq!(guess_groups(&c), vec![PortGroup::new(PortKind::Rj45, 24, "Gi1/0/{n}"), PortGroup::new(PortKind::Sfp, 4, "Te1/1/{n}")]);
        let j: Vec<IfInfo> = (0..4).map(|n| ifc(&format!("ge-0/0/{n}"), 6, 1000)).collect();
        assert_eq!(guess_groups(&j)[0].start, Some(0));
        let names: Vec<String> = port_names(&guess_groups(&j)).into_iter().map(|x| x.0).collect();
        assert_eq!(names, vec!["ge-0/0/0", "ge-0/0/1", "ge-0/0/2", "ge-0/0/3"]);
    }
}
