//! Alert notifications: e-mail (SMTP) and webhooks (Teams, Slack, any JSON
//! endpoint). Alerts are queued by `Net::alert` / `Net::switch_alert` without
//! waiting, grouped for `groupSecs`, then sent as one message per channel.
//! Messages only go to the mail server and the webhook URLs set here, never to
//! the network equipment.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::model::{Alert, MASKED};
use super::{Net, Store};
use crate::api::{bad, require, status, ApiResult, Session};
use crate::paths::{new_id, now_ms, read_json};
use crate::state::Shared;

/// The settings file in `<data>/network/` (written 0600: it holds the SMTP password and the webhook URLs).
pub const FILE: &str = "notifications.json";
/// Alerts waiting to be grouped; more are dropped (and logged).
const QUEUE: usize = 1000;
/// Alerts listed in one message; the others are counted.
const MAX_LINES: usize = 50;
const SMTP_TIMEOUT: Duration = Duration::from_secs(15);
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(10);

// ── Settings ──

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    None,
    #[default]
    Starttls,
    Tls,
}

fn default_smtp_port() -> u16 {
    587
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Email {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    #[serde(default)]
    pub security: Security,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: Vec<String>,
}

impl Default for Email {
    fn default() -> Self {
        Email { enabled: false, host: String::new(), port: default_smtp_port(), security: Security::Starttls, user: String::new(), password: String::new(), from: String::new(), to: Vec::new() }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookKind {
    #[default]
    Teams,
    Slack,
    Generic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Webhook {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: HookKind,
    #[serde(default)]
    pub url: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

fn default_group_secs() -> u32 {
    60
}

/// Which alerts are sent: `None` = every kind ("all").
#[derive(Clone, Debug, PartialEq)]
pub struct Kinds(pub Option<Vec<String>>);

impl Serialize for Kinds {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match &self.0 {
            None => s.serialize_str("all"),
            Some(v) => v.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for Kinds {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Word(String),
            List(Vec<String>),
        }
        match Raw::deserialize(d)? {
            Raw::Word(w) if w == "all" => Ok(Kinds(None)),
            Raw::Word(w) => Err(serde::de::Error::custom(format!("unknown alert kinds {w:?}"))),
            Raw::List(v) => Ok(Kinds(Some(v))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    #[serde(default = "all_kinds")]
    pub kinds: Kinds,
    /// Alerts raised within this many seconds go in one message (0 = each alert at once).
    #[serde(default = "default_group_secs")]
    pub group_secs: u32,
}

fn all_kinds() -> Kinds {
    Kinds(None)
}

impl Default for Filter {
    fn default() -> Self {
        Filter { kinds: Kinds(None), group_secs: default_group_secs() }
    }
}

impl Filter {
    pub fn wants(&self, kind: &str) -> bool {
        self.kinds.0.as_ref().is_none_or(|k| k.iter().any(|x| x == kind))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub email: Email,
    #[serde(default)]
    pub webhooks: Vec<Webhook>,
    #[serde(default)]
    pub filter: Filter,
    /// The address of the web app as the recipients reach it ("https://strata.example.lan"); '' = no link.
    #[serde(default)]
    pub app_url: String,
}

/// "https://host/…" → "https://host/••••••••" (the path and query of a webhook URL are its secret).
pub fn mask_url(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    if scheme.is_empty() {
        MASKED.to_string()
    } else {
        format!("{scheme}://{host}/{MASKED}")
    }
}

impl Settings {
    /// A copy safe to send to the web app.
    pub fn masked(&self) -> Settings {
        let mut s = self.clone();
        if !s.email.password.is_empty() {
            s.email.password = MASKED.into();
        }
        for w in s.webhooks.iter_mut() {
            w.url = mask_url(&w.url);
        }
        s
    }

    /// Secrets sent back masked keep their stored value (webhooks matched by id).
    pub fn unmask(&mut self, cur: &Settings) {
        if self.email.password == MASKED {
            self.email.password = cur.email.password.clone();
        }
        for w in self.webhooks.iter_mut() {
            if w.url.contains(MASKED) {
                w.url = cur.webhooks.iter().find(|c| !w.id.is_empty() && c.id == w.id).map(|c| c.url.clone()).unwrap_or_default();
            }
        }
    }

    /// Trims and checks what was sent; gives new webhooks an id.
    pub fn check(&mut self) -> Result<(), String> {
        let e = &mut self.email;
        e.host = e.host.trim().to_string();
        e.user = e.user.trim().to_string();
        e.from = e.from.trim().to_string();
        e.to = e.to.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
        if e.port == 0 {
            e.port = match e.security {
                Security::Tls => 465,
                Security::Starttls => 587,
                Security::None => 25,
            };
        }
        if e.enabled {
            if e.host.is_empty() {
                return Err("the smtp server cannot be empty".into());
            }
            if e.to.is_empty() {
                return Err("the email has no recipient".into());
            }
        }
        for a in e.to.iter().chain(std::iter::once(&e.from).filter(|f| !f.is_empty())) {
            a.parse::<lettre::message::Mailbox>().map_err(|_| format!("not an email address: {a}"))?;
        }
        if e.enabled && e.from.is_empty() {
            return Err("the sender address cannot be empty".into());
        }
        for w in self.webhooks.iter_mut() {
            w.name = w.name.trim().to_string();
            w.url = w.url.trim().to_string();
            if w.id.is_empty() {
                w.id = new_id();
            }
            if !(w.url.starts_with("https://") || w.url.starts_with("http://")) || w.url.len() < 10 {
                return Err("the webhook url starts with https://".into());
            }
        }
        let mut ids: Vec<&str> = self.webhooks.iter().map(|w| w.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() != self.webhooks.len() {
            return Err("two webhooks have the same id".into());
        }
        if let Some(k) = self.filter.kinds.0.as_mut() {
            k.retain(|x| !x.trim().is_empty());
            k.sort();
            k.dedup();
        }
        self.filter.group_secs = self.filter.group_secs.min(3600);
        self.app_url = self.app_url.trim().trim_end_matches('/').to_string();
        if !self.app_url.is_empty() && !(self.app_url.starts_with("https://") || self.app_url.starts_with("http://")) {
            return Err("the app address starts with https://".into());
        }
        Ok(())
    }

    fn any_channel(&self) -> bool {
        self.email.enabled || self.webhooks.iter().any(|w| w.enabled)
    }

    /// What the history says of these settings (no secret).
    pub fn summary(&self) -> String {
        let hooks = self.webhooks.iter().filter(|w| w.enabled).count();
        let kinds = match &self.filter.kinds.0 {
            None => "all alerts".to_string(),
            Some(k) => format!("{} kinds", k.len()),
        };
        format!(
            "email {} · {hooks}/{} webhooks · {kinds} · {} s",
            if self.email.enabled { "on" } else { "off" },
            self.webhooks.len(),
            self.filter.group_secs
        )
    }
}

// ── Queue ──

/// An alert with the names it is shown with (read from the store when raised).
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub alert: Alert,
    pub switch_name: Option<String>,
    /// The faceplate port: its interface name, and its label when it has one.
    pub port_name: Option<String>,
    /// The address's label, else its name.
    pub host: Option<String>,
}

impl Item {
    pub fn of(st: &Store, a: &Alert) -> Item {
        let sw = a.switch_id.as_deref().and_then(|id| st.switches.iter().find(|s| s.id == id));
        let port = sw.zip(a.port).and_then(|(s, n)| s.ports.iter().find(|p| p.n == n));
        let port_name = port.map(|p| if p.label.is_empty() { p.name.clone() } else { format!("{} «\u{a0}{}\u{a0}»", p.name, p.label) });
        let host = if sw.is_some() {
            None
        } else {
            st.addresses.get(&u32::from(a.ip)).and_then(|x| if x.label.is_empty() { x.name.clone() } else { Some(x.label.clone()) })
        };
        Item { alert: a.clone(), switch_name: sw.map(|s| s.name.clone()), port_name, host }
    }
}

pub struct Notifier {
    pub settings: Mutex<Settings>,
    tx: mpsc::Sender<Item>,
    rx: Mutex<Option<mpsc::Receiver<Item>>>,
}

impl Notifier {
    pub fn load(dir: &std::path::Path) -> Notifier {
        let settings = match read_json::<Settings>(&dir.join(FILE)) {
            Ok(v) => v.unwrap_or_default(),
            Err(e) => {
                tracing::error!("cannot read {FILE}: {e}; notifications off");
                Settings::default()
            }
        };
        let (tx, rx) = mpsc::channel(QUEUE);
        Notifier { settings: Mutex::new(settings), tx, rx: Mutex::new(Some(rx)) }
    }

    /// Queues the newest alert of the store (call right after inserting it, store locked: never waits).
    pub fn raised(&self, st: &Store) {
        let Some(a) = st.alerts.first() else { return };
        if let Err(e) = self.tx.try_send(Item::of(st, a)) {
            match e {
                mpsc::error::TrySendError::Full(i) => tracing::warn!("notification queue full: alert {} {} not sent", i.alert.kind, i.alert.id),
                mpsc::error::TrySendError::Closed(_) => {}
            }
        }
    }
}

/// Alerts waiting for the end of their grouping window.
#[derive(Default)]
pub struct Batch {
    pub items: Vec<Item>,
    /// When the window of the first waiting alert ends (unix ms).
    pub until: Option<i64>,
}

impl Batch {
    pub fn push(&mut self, item: Item, now: i64, group_secs: u32) {
        if self.items.is_empty() {
            self.until = Some(now + i64::from(group_secs) * 1000);
        }
        self.items.push(item);
    }

    /// The alerts to send now, if the window has ended.
    pub fn take_due(&mut self, now: i64) -> Option<Vec<Item>> {
        if self.until.is_some_and(|u| u <= now) {
            self.until = None;
            Some(std::mem::take(&mut self.items))
        } else {
            None
        }
    }

    /// How long to wait for more alerts (`None`: nothing waiting).
    pub fn wait(&self, now: i64) -> Option<Duration> {
        self.until.map(|u| Duration::from_millis(u.saturating_sub(now).max(0) as u64))
    }
}

/// Starts the task that groups and sends the alerts. Call once.
pub fn spawn(net: Arc<Net>) {
    let Some(mut rx) = net.notify.rx.lock().unwrap().take() else { return };
    tokio::spawn(async move {
        let mut batch = Batch::default();
        loop {
            let got = match batch.wait(now_ms()) {
                None => rx.recv().await.map(Some),
                Some(d) => match tokio::time::timeout(d, rx.recv()).await {
                    Ok(r) => r.map(Some),
                    Err(_) => Some(None),
                },
            };
            let Some(got) = got else { break };
            if let Some(item) = got {
                let (on, group) = {
                    let s = net.notify.settings.lock().unwrap();
                    (s.any_channel() && s.filter.wants(&item.alert.kind), s.filter.group_secs)
                };
                if on {
                    batch.push(item, now_ms(), group);
                }
            }
            if let Some(items) = batch.take_due(now_ms()) {
                deliver(&net, &items).await;
            }
        }
    });
}

/// The server's name and time zone, the settings.
fn context(net: &Net) -> (String, jiff::tz::TimeZone, Settings) {
    let (name, tz) = match net.app.read().unwrap().upgrade() {
        Some(app) => {
            let c = app.config.load();
            (c.name.clone(), jiff::tz::TimeZone::get(&c.timezone_name()).unwrap_or_else(|_| jiff::tz::TimeZone::system()))
        }
        None => (crate::config::PRODUCT.to_string(), jiff::tz::TimeZone::system()),
    };
    (name, tz, net.notify.settings.lock().unwrap().clone())
}

async fn deliver(net: &Net, items: &[Item]) {
    let (server, tz, s) = context(net);
    let digest = Digest::new(&server, items, &tz, &s.app_url);
    if s.email.enabled {
        if let Err(e) = send_email(&s.email, &digest.subject, &digest.text()).await {
            tracing::warn!("alert e-mail not sent ({} alerts): {e}", items.len());
        }
    }
    for w in s.webhooks.iter().filter(|w| w.enabled) {
        let body = payload(w.kind, &digest, items);
        if let Err(e) = post_webhook(&w.url, body).await {
            tracing::warn!("alert webhook {} not sent ({} alerts): {e}", w.name, items.len());
        }
    }
}

/// Sends a test message to one channel ("email" or a webhook id), enabled or not.
pub async fn test(net: &Net, channel: &str) -> Result<String, (u16, String)> {
    let (server, _, s) = context(net);
    let digest = Digest::test(&server, &s.app_url);
    if channel == "email" {
        if s.email.host.is_empty() || s.email.to.is_empty() || s.email.from.is_empty() {
            return Err((400, "the email is not set up".into()));
        }
        send_email(&s.email, &digest.subject, &digest.text()).await.map_err(|e| (502, e))?;
        return Ok("email".into());
    }
    let w = s.webhooks.iter().find(|w| w.id == channel).ok_or((404, "no such webhook".to_string()))?;
    let body = match w.kind {
        HookKind::Generic => json!({ "test": true, "text": digest.text(), "alerts": [] }),
        k => payload(k, &digest, &[]),
    };
    post_webhook(&w.url, body).await.map_err(|e| (502, e))?;
    Ok(w.name.clone())
}

// ── Routes ──

pub fn router() -> Router<Shared> {
    Router::new().route("/api/notifications", get(get_settings).put(put_settings)).route("/api/notifications/test", post(test_route))
}

fn who(me: &Session) -> String {
    if me.user.display_name.is_empty() {
        me.user.username.clone()
    } else {
        me.user.display_name.clone()
    }
}

async fn get_settings(State(s): State<Shared>, me: Session) -> ApiResult<Json<Settings>> {
    require(me.user.is_admin())?;
    Ok(Json(s.net.notify.settings.lock().unwrap().masked()))
}

async fn put_settings(State(s): State<Shared>, me: Session, Json(mut b): Json<Settings>) -> ApiResult<Json<Settings>> {
    require(me.user.is_admin())?;
    let saved = {
        let mut cur = s.net.notify.settings.lock().unwrap();
        b.unmask(&cur);
        b.check().map_err(bad)?;
        *cur = b.clone();
        serde_json::to_vec_pretty(&b)
    };
    match saved {
        Ok(bytes) => s.net.write_file(FILE, bytes),
        Err(e) => tracing::error!("cannot serialize {FILE}: {e}"),
    }
    s.net.changed("notifications");
    s.net.log(&who(&me), "changed notifications", "", &b.summary());
    Ok(Json(b.masked()))
}

#[derive(Deserialize)]
struct TestBody {
    channel: String,
}

async fn test_route(State(s): State<Shared>, me: Session, Json(b): Json<TestBody>) -> ApiResult<Json<Value>> {
    require(me.user.is_admin())?;
    let started = std::time::Instant::now();
    let r = test(&s.net, &b.channel).await;
    let target = match &r {
        Ok(name) => name.clone(),
        Err(_) => b.channel.clone(),
    };
    match r {
        Ok(_) => {
            s.net.log(&who(&me), "tested notification", &target, "sent");
            Ok(Json(json!({ "sent": true, "ms": started.elapsed().as_millis() as u64 })))
        }
        Err((code, msg)) => {
            if code == 502 {
                s.net.log(&who(&me), "tested notification", &target, &msg);
            }
            Err(status(code, msg))
        }
    }
}

// ── Messages (French: they are read outside the app) ──

pub fn kind_label(kind: &str) -> &str {
    match kind {
        "newHost" => "Nouvelle machine",
        "hostGone" => "Machine disparue",
        "ipConflict" => "Conflit d’IP",
        "vlanMismatch" => "VLAN incohérent",
        "watchDown" => "Surveillée injoignable",
        "watchUp" => "Surveillée de retour",
        "portDown" => "Lien coupé",
        "portFlap" => "Port instable",
        "portErrors" => "Erreurs",
        "portDiscards" => "Rejets",
        "portSaturated" => "Port saturé",
        "configDrift" => "Écart de config",
        "opticLow" => "Optique faible",
        "psuFault" => "Alimentation",
        "fanFault" => "Ventilateur",
        "tempHigh" => "Température",
        "poeBudget" => "Budget PoE",
        "neighbourNew" => "Nouveau voisin",
        "neighbourGone" => "Voisin disparu",
        "streamGone" => "Flux arrêté",
        "noReceivers" => "Plus de récepteur",
        "groupConflict" => "Groupe émis deux fois",
        "flooding" => "Inondation multicast",
        "pairMissing" => "Paire 2022-7 incomplète",
        "ptpGmChange" => "Changement de grandmaster",
        "ptpDomain" => "Domaine PTP différent",
        k => k,
    }
}

/// A number the French way: comma, at most `d` decimals, true minus sign.
pub fn dec(v: f64, d: usize) -> String {
    let s = format!("{:.*}", d, v.abs());
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    let s = s.replace('.', ",");
    if v < 0.0 && s != "0" {
        format!("\u{2212}{s}")
    } else {
        s
    }
}

/// The detail of an alert in French words (the server's facts otherwise).
pub fn detail_fr(a: &Alert) -> String {
    let v = a.value.unwrap_or(0.0);
    let lim = a.limit.unwrap_or(0.0);
    match a.kind.as_str() {
        "portFlap" => format!("{} coupure{} en 10\u{a0}min", dec(v, 0), if v >= 2.0 { "s" } else { "" }),
        "portErrors" => format!("{} par minute (seuil {})", dec(v, 0), dec(lim, 0)),
        "portDiscards" => format!("{} paquets/s pendant 1\u{a0}min (seuil {})", dec(v, 1), dec(lim, 1)),
        "portSaturated" => format!("{}\u{a0}% pendant 1\u{a0}min (seuil {}\u{a0}%)", dec(v, 1), dec(lim, 0)),
        "configDrift" => {
            let (sw, inv) = a.detail.split_once(" / ").unwrap_or((&a.detail, ""));
            format!("switch\u{a0}: {sw} · inventaire\u{a0}: {inv}")
        }
        "opticLow" => [a.detail.clone(), format!("Rx {}\u{a0}dBm (seuil {})", dec(v, 1), dec(lim, 1))].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · "),
        "tempHigh" => format!("{} · {}\u{a0}°C (seuil {}\u{a0}°C)", a.detail, dec(v, 1), dec(lim, 0)),
        "poeBudget" => format!("{}\u{a0}% du budget (seuil {}\u{a0}%)", dec(v, 1), dec(lim, 0)),
        _ => a.detail.replace(" and ", " et "),
    }
}

/// One alert as it is listed.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub time: String,
    pub label: String,
    /// The switch and port, else the IP (and its name).
    pub place: String,
    pub detail: String,
}

impl Line {
    pub fn of(i: &Item, tz: &jiff::tz::TimeZone) -> Line {
        let time = jiff::Timestamp::from_millisecond(i.alert.at).map(|t| t.to_zoned(tz.clone()).strftime("%d/%m/%Y %H:%M:%S").to_string()).unwrap_or_default();
        let place = match (&i.switch_name, i.alert.port) {
            (Some(sw), Some(n)) => match &i.port_name {
                Some(p) => format!("{sw} · port {n} ({p})"),
                None => format!("{sw} · port {n}"),
            },
            (Some(sw), None) => sw.clone(),
            (None, _) => match &i.host {
                Some(h) => format!("{} ({h})", i.alert.ip),
                None => i.alert.ip.to_string(),
            },
        };
        Line { time, label: kind_label(&i.alert.kind).to_string(), place, detail: detail_fr(&i.alert) }
    }

    pub fn text(&self) -> String {
        let mut s = format!("{} — {} — {}", self.time, self.label, self.place);
        if !self.detail.is_empty() {
            s.push_str(&format!(" — {}", self.detail));
        }
        s
    }
}

/// The message sent for a group of alerts.
#[derive(Clone, Debug, PartialEq)]
pub struct Digest {
    pub subject: String,
    pub intro: String,
    pub lines: Vec<Line>,
    /// Alerts not listed.
    pub more: usize,
    pub link: Option<String>,
}

impl Digest {
    pub fn new(server: &str, items: &[Item], tz: &jiff::tz::TimeZone, app_url: &str) -> Digest {
        let lines: Vec<Line> = items.iter().take(MAX_LINES).map(|i| Line::of(i, tz)).collect();
        let subject = match lines.as_slice() {
            [one] => format!("[{server}] {} · {}", one.label, one.place),
            _ => format!("[{server}] {} alertes", items.len()),
        };
        let intro = if items.len() == 1 { format!("Nouvelle alerte sur {server}\u{a0}:") } else { format!("{} nouvelles alertes sur {server}\u{a0}:", items.len()) };
        Digest { subject, intro, lines, more: items.len().saturating_sub(MAX_LINES), link: (!app_url.is_empty()).then(|| format!("{app_url}/alerts")) }
    }

    pub fn test(server: &str, app_url: &str) -> Digest {
        Digest {
            subject: format!("[{server}] Message de test"),
            intro: format!("Message de test de {server}\u{a0}: les notifications d’alertes arrivent bien ici."),
            lines: Vec::new(),
            more: 0,
            link: (!app_url.is_empty()).then(|| app_url.to_string()),
        }
    }

    /// The text, lines separated by `sep` ("\n" for e-mail and Slack, "\n\n" for Teams' markdown).
    pub fn render(&self, sep: &str, bullet: &str) -> String {
        let mut out = vec![self.intro.clone()];
        out.extend(self.lines.iter().map(|l| format!("{bullet}{}", l.text())));
        if self.more > 0 {
            out.push(format!("… et {} autre{}", self.more, if self.more > 1 { "s" } else { "" }));
        }
        if let Some(l) = &self.link {
            out.push(format!("Ouvrir Strata Network\u{a0}: {l}"));
        }
        out.join(sep)
    }

    pub fn text(&self) -> String {
        self.render("\n", "• ")
    }
}

/// The JSON posted to a webhook.
pub fn payload(kind: HookKind, d: &Digest, items: &[Item]) -> Value {
    match kind {
        HookKind::Teams => json!({ "text": d.render("\n\n", "- ") }),
        HookKind::Slack => json!({ "text": d.render("\n", "• ") }),
        HookKind::Generic => json!({
            "alerts": items.iter().map(|i| json!({
                "kind": i.alert.kind,
                "at": i.alert.at,
                "switchId": i.alert.switch_id,
                "switchName": i.switch_name,
                "port": i.alert.port,
                "ip": i.alert.ip,
                "detail": i.alert.detail,
                "value": i.alert.value,
                "limit": i.alert.limit,
            })).collect::<Vec<_>>(),
        }),
    }
}

// ── Sending ──

/// A short English key for an SMTP failure (translated by the web app).
fn smtp_error(e: &lettre::transport::smtp::Error) -> String {
    if e.is_timeout() {
        return "smtp: timed out".into();
    }
    // A TLS handshake that failed comes as a connection error holding the rustls error.
    let mut src = std::error::Error::source(e);
    let mut tls = e.is_tls();
    while let Some(x) = src {
        tls |= x.is::<rustls::Error>() || x.downcast_ref::<std::io::Error>().and_then(|io| io.get_ref()).is_some_and(|i| i.is::<rustls::Error>());
        src = x.source();
    }
    if tls {
        return "smtp: tls failed".into();
    }
    if e.is_client() && e.to_string().contains("STARTTLS") {
        return "smtp: the server does not offer starttls".into();
    }
    if let Some(code) = e.status() {
        let code = code.to_string();
        return match code.as_str() {
            "530" | "534" | "535" | "538" | "454" => "smtp: authentication failed".into(),
            "550" | "551" | "553" | "501" | "554" => format!("smtp: the server refused the message ({code})"),
            _ => format!("smtp: the server answered {code}"),
        };
    }
    if e.is_response() || e.is_client() {
        return format!("smtp: {e}");
    }
    "smtp: cannot connect to the server".into()
}

pub async fn send_email(e: &Email, subject: &str, body: &str) -> Result<(), String> {
    use lettre::message::{header::ContentType, Mailbox};
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::transport::smtp::client::{Tls, TlsParameters};
    use lettre::transport::smtp::extension::ClientId;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

    let from: Mailbox = e.from.parse().map_err(|_| format!("not an email address: {}", e.from))?;
    let hello = gethostname::gethostname().to_string_lossy().split('.').next().filter(|h| !h.is_empty()).map(str::to_string).unwrap_or_else(|| "strata".into());
    let mut msg = Message::builder().from(from).subject(subject).message_id(Some(format!("<{}{}@{hello}>", new_id(), new_id())));
    for t in &e.to {
        msg = msg.to(t.parse::<Mailbox>().map_err(|_| format!("not an email address: {t}"))?);
    }
    let msg = msg.header(ContentType::TEXT_PLAIN).body(body.to_string()).map_err(|e| format!("smtp: {e}"))?;
    let tls = match e.security {
        Security::None => Tls::None,
        s => {
            let p = TlsParameters::new(e.host.clone()).map_err(|_| "smtp: tls failed".to_string())?;
            if s == Security::Tls {
                Tls::Wrapper(p)
            } else {
                Tls::Required(p)
            }
        }
    };
    let mut t = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(e.host.as_str()).port(e.port).tls(tls).timeout(Some(SMTP_TIMEOUT)).hello_name(ClientId::Domain(hello));
    if !e.user.is_empty() {
        t = t.credentials(Credentials::new(e.user.clone(), e.password.clone()));
    }
    let t = t.build();
    match tokio::time::timeout(SMTP_TIMEOUT, t.send(msg)).await {
        Err(_) => Err("smtp: timed out".into()),
        Ok(Err(err)) => {
            tracing::debug!("smtp {}: {err:?}", e.host);
            Err(smtp_error(&err))
        }
        Ok(Ok(_)) => Ok(()),
    }
}

/// A short English key for a webhook failure.
fn http_error(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => "webhook: timed out".into(),
        ureq::Error::HostNotFound => "webhook: unknown host".into(),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => "webhook: tls failed".into(),
        ureq::Error::BadUri(_) => "webhook: bad url".into(),
        _ => "webhook: cannot reach the server".into(),
    }
}

pub async fn post_webhook(url: &str, body: Value) -> Result<(), String> {
    let url = url.to_string();
    let r = tokio::task::spawn_blocking(move || {
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_global(Some(WEBHOOK_TIMEOUT))
            .http_status_as_error(false)
            .max_redirects(3)
            .user_agent(concat!("strata/", env!("CARGO_PKG_VERSION")))
            .build()
            .new_agent();
        let bytes = serde_json::to_vec(&body).unwrap_or_default();
        match agent.post(&url).header("Content-Type", "application/json").send(&bytes[..]) {
            Ok(res) if res.status().is_success() => Ok(()),
            Ok(res) => Err(format!("webhook: the server answered {}", res.status().as_u16())),
            Err(e) => {
                tracing::debug!("webhook: {e}");
                Err(http_error(&e))
            }
        }
    })
    .await;
    r.unwrap_or_else(|e| Err(format!("webhook: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn alert(kind: &str, at: i64) -> Alert {
        Alert {
            id: "a1".into(),
            at,
            kind: kind.into(),
            ip: Ipv4Addr::new(10, 0, 0, 5),
            network_id: None,
            detail: String::new(),
            seen: false,
            switch_id: None,
            port: None,
            value: None,
            limit: None,
        }
    }

    fn item(kind: &str) -> Item {
        Item { alert: alert(kind, 1_763_300_000_000), switch_name: None, port_name: None, host: None }
    }

    #[test]
    fn grouping() {
        let mut b = Batch::default();
        assert_eq!(b.wait(0), None);
        b.push(item("newHost"), 1000, 60);
        assert_eq!(b.wait(1000), Some(Duration::from_secs(60)));
        b.push(item("hostGone"), 30_000, 60);
        // The window is the first alert's: the second does not push it back.
        assert_eq!(b.wait(30_000), Some(Duration::from_secs(31)));
        assert!(b.take_due(60_999).is_none());
        let sent = b.take_due(61_000).unwrap();
        assert_eq!(sent.len(), 2);
        assert!(b.items.is_empty() && b.until.is_none());
        // A new window starts with the next alert; 0 s sends at once.
        b.push(item("watchDown"), 70_000, 0);
        assert_eq!(b.take_due(70_000).unwrap().len(), 1);
    }

    #[test]
    fn masking_round_trip() {
        let stored = Settings {
            email: Email { enabled: true, host: "smtp.example.com".into(), password: "hunter2".into(), from: "a@b.fr".into(), to: vec!["c@d.fr".into()], ..Email::default() },
            webhooks: vec![Webhook { id: "w1".into(), name: "Teams".into(), kind: HookKind::Teams, url: "https://example.webhook.office.com/webhookb2/abc?x=1".into(), enabled: true }],
            ..Settings::default()
        };
        let m = stored.masked();
        assert_eq!(m.email.password, MASKED);
        assert_eq!(m.webhooks[0].url, format!("https://example.webhook.office.com/{MASKED}"));
        assert!(!serde_json::to_string(&m).unwrap().contains("abc"));
        let mut back = m.clone();
        back.unmask(&stored);
        assert_eq!(back, stored);
        // A new value replaces; a masked URL on an unknown webhook is not kept.
        let mut changed = m;
        changed.email.password = "new".into();
        changed.webhooks.push(Webhook { id: "w9".into(), name: String::new(), kind: HookKind::Slack, url: format!("https://hooks.slack.com/{MASKED}"), enabled: true });
        changed.unmask(&stored);
        assert_eq!(changed.email.password, "new");
        assert_eq!(changed.webhooks[1].url, "");
        assert!(changed.check().is_err());
        assert_eq!(mask_url(""), "");
        assert_eq!(mask_url("http://user:pw@h:8080/p"), format!("http://h:8080/{MASKED}"));
    }

    #[test]
    fn settings_json() {
        let s: Settings = serde_json::from_str(r#"{"filter":{"kinds":["portDown","newHost"]},"webhooks":[{"kind":"generic","url":"http://127.0.0.1:9/x"}]}"#).unwrap();
        assert!(s.filter.wants("portDown") && !s.filter.wants("hostGone"));
        assert_eq!(s.filter.group_secs, 60);
        assert!(s.webhooks[0].enabled);
        let v = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(v["filter"]["kinds"], "all");
        assert_eq!(v["email"]["security"], "starttls");
        assert!(serde_json::from_str::<Settings>(r#"{"filter":{"kinds":"some"}}"#).is_err());
        let mut s = s;
        s.check().unwrap();
        assert_eq!(s.webhooks[0].id.len(), 12);
        let mut bad = Settings::default();
        bad.email.enabled = true;
        bad.email.host = "smtp".into();
        bad.email.to = vec!["nobody".into()];
        assert_eq!(bad.check().unwrap_err(), "not an email address: nobody");
    }

    #[test]
    fn french_text() {
        assert_eq!(dec(-11.84, 1), "\u{2212}11,8");
        assert_eq!(dec(95.0, 1), "95");
        assert_eq!(dec(2.5, 0), "2");
        let mut a = alert("portSaturated", 0);
        a.value = Some(95.26);
        a.limit = Some(90.0);
        assert_eq!(detail_fr(&a), "95,3\u{a0}% pendant 1\u{a0}min (seuil 90\u{a0}%)");
        a.kind = "portDiscards".into();
        a.value = Some(20.0);
        a.limit = Some(10.0);
        assert_eq!(detail_fr(&a), "20 paquets/s pendant 1\u{a0}min (seuil 10)");
        assert_eq!(kind_label("portDiscards"), "Rejets");
        a.kind = "configDrift".into();
        a.detail = "access VLAN 10 / VLAN 20".into();
        assert_eq!(detail_fr(&a), "switch\u{a0}: access VLAN 10 · inventaire\u{a0}: VLAN 20");
        a.kind = "ipConflict".into();
        a.detail = "00:11:32:AB:CD:EF and 3C:52:82:5A:00:E1".into();
        assert_eq!(detail_fr(&a), "00:11:32:AB:CD:EF et 3C:52:82:5A:00:E1");
    }

    #[test]
    fn payloads() {
        let tz = jiff::tz::TimeZone::get("Europe/Paris").unwrap();
        let mut sw = item("portDown");
        sw.alert.at = 1_763_301_792_000; // 2025-11-16 14:03:12 UTC
        sw.alert.switch_id = Some("s1".into());
        sw.alert.port = Some(12);
        sw.alert.detail = "PTZ".into();
        sw.switch_name = Some("SW-CORE-1".into());
        sw.port_name = Some("Gi1/0/12".into());
        let mut host = item("newHost");
        host.host = Some("Caméra".into());

        let one = Digest::new("Strata", std::slice::from_ref(&sw), &tz, "https://strata.lan");
        assert_eq!(one.subject, "[Strata] Lien coupé · SW-CORE-1 · port 12 (Gi1/0/12)");
        assert_eq!(one.lines[0].time, "16/11/2025 15:03:12");
        let text = one.text();
        assert!(text.contains("• 16/11/2025 15:03:12 — Lien coupé — SW-CORE-1 · port 12 (Gi1/0/12) — PTZ"), "{text}");
        assert!(text.ends_with("Ouvrir Strata Network\u{a0}: https://strata.lan/alerts"));

        let items = vec![sw.clone(), host];
        let two = Digest::new("Strata", &items, &tz, "");
        assert_eq!(two.subject, "[Strata] 2 alertes");
        assert_eq!(two.lines[1].place, "10.0.0.5 (Caméra)");
        assert!(two.link.is_none());

        let teams = payload(HookKind::Teams, &two, &items);
        assert!(teams["text"].as_str().unwrap().contains("\n\n- "));
        let slack = payload(HookKind::Slack, &two, &items);
        assert!(slack["text"].as_str().unwrap().starts_with("2 nouvelles alertes sur Strata\u{a0}:\n• "));
        let g = payload(HookKind::Generic, &two, &items);
        assert_eq!(g["alerts"][0]["switchName"], "SW-CORE-1");
        assert_eq!(g["alerts"][0]["port"], 12);
        assert_eq!(g["alerts"][1]["ip"], "10.0.0.5");
        assert_eq!(g["alerts"][1]["switchId"], Value::Null);

        let many: Vec<Item> = (0..MAX_LINES + 3).map(|_| item("hostGone")).collect();
        let d = Digest::new("S", &many, &tz, "");
        assert_eq!(d.lines.len(), MAX_LINES);
        assert!(d.text().ends_with("… et 3 autres"));
    }
}
