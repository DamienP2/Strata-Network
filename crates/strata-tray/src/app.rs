//! The menu bar / notification area icon and its menu (macOS, Windows).

use std::net::TcpListener;
use std::time::Duration;

use serde_json::{json, Value};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::common::{self, Texts, PRODUCT};

enum UserEvent {
    /// The server's state (`None` = it does not answer), its data folder and port.
    State(Option<Value>, u16, Option<std::path::PathBuf>),
    Menu(MenuEvent),
}

/// A round icon drawn in code (no image file to ship).
fn icon(running: bool) -> Option<Icon> {
    let size = 32u32;
    let (r, g, b) = if running { (0x8f, 0xa3, 0xff) } else { (0x8a, 0x90, 0xad) };
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let c = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let ring = (11.0..=15.0).contains(&d);
            let dot = d <= 6.0;
            let a = if ring || dot { 255 } else { 0 };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    Icon::from_rgba(rgba, size, size).ok()
}

struct Ui {
    tray: TrayIcon,
    status: MenuItem,
    info: MenuItem,
    copy: MenuItem,
    open: MenuItem,
    network: Submenu,
    net_all: CheckMenuItem,
    net_cards: Vec<(CheckMenuItem, String)>,
    net_list: Vec<String>,
    port: MenuItem,
    start_stop: MenuItem,
    restart: MenuItem,
    update: MenuItem,
    log: MenuItem,
    uninstall: MenuItem,
    quit: MenuItem,
}

impl Ui {
    fn new(t: &Texts) -> Ui {
        let status = MenuItem::new(t.checking(PRODUCT), false, None);
        let info = MenuItem::new("…", false, None);
        let copy = MenuItem::new(t.copy(""), false, None);
        let open = MenuItem::new(t.open_web(), false, None);
        let network = Submenu::new(t.network(), false);
        let header = MenuItem::new(t.web_interface(), false, None);
        let net_all = CheckMenuItem::new(t.all_cards(), true, false, None);
        let _ = network.append_items(&[&header, &net_all]);
        let port = MenuItem::new(t.port(common::DEFAULT_PORT), false, None);
        let start_stop = MenuItem::new(t.start(), true, None);
        let restart = MenuItem::new(t.restart(), false, None);
        let update = MenuItem::new(t.check_update(), false, None);
        let log = MenuItem::new(t.log(), true, None);
        let uninstall = MenuItem::new(t.uninstall(), true, None);
        let quit = MenuItem::new(t.quit(), true, None);
        let menu = Menu::new();
        let sep = || PredefinedMenuItem::separator();
        let _ = menu.append_items(&[
            &status,
            &info,
            &copy,
            &sep(),
            &open,
            &sep(),
            &network,
            &port,
            &sep(),
            &start_stop,
            &restart,
            &update,
            &sep(),
            &log,
            &uninstall,
            &quit,
        ]);
        let mut builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip(PRODUCT);
        if let Some(i) = icon(false) {
            builder = builder.with_icon(i);
        }
        let tray = builder.build().expect("tray icon");
        Ui { tray, status, info, copy, open, network, net_all, net_cards: Vec::new(), net_list: Vec::new(), port, start_stop, restart, update, log, uninstall, quit }
    }

    fn set_cards(&mut self, t: &Texts, cards: &[(String, String)]) {
        let list: Vec<String> = cards.iter().map(|(n, ip)| format!("{n} {ip}")).collect();
        if list == self.net_list {
            return;
        }
        for (item, _) in self.net_cards.drain(..) {
            let _ = self.network.remove(&item);
        }
        for (name, ip) in cards {
            let item = CheckMenuItem::new(format!("{name} — {ip}"), true, false, None);
            let _ = self.network.append(&item);
            self.net_cards.push((item, ip.clone()));
        }
        self.net_list = list;
        let _ = t;
    }

    fn refresh(&mut self, t: &Texts, state: Option<&Value>, port: u16) {
        let running = state.is_some();
        let s = |k: &str| state.and_then(|v| v.get(k)).and_then(Value::as_str).unwrap_or("").to_string();
        let name = Some(s("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| PRODUCT.to_string());
        let _ = self.tray.set_icon(icon(running));
        let _ = self.tray.set_tooltip(Some(if running { t.running(&name) } else { t.stopped(&name) }));
        self.status.set_text(if running { t.running(&name) } else { t.stopped(&name) });
        let uptime = state.and_then(|v| v.get("uptimeSecs")).and_then(Value::as_u64).map(common::fmt_uptime).unwrap_or_else(|| "—".into());
        let version = Some(s("version")).filter(|v| !v.is_empty()).unwrap_or_else(|| "—".into());
        self.info.set_text(t.info(&version, port, &uptime));
        let urls: Vec<String> = state.and_then(|v| v.get("urls")).and_then(Value::as_array).map(|a| a.iter().filter_map(|u| u.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let url = urls.iter().find(|u| !u.contains("localhost") && !u.contains("127.0.0.1")).or(urls.first()).cloned().unwrap_or_else(|| format!("http://localhost:{port}"));
        self.copy.set_text(t.copy(&url));
        self.copy.set_enabled(running);
        self.open.set_enabled(running);
        let cards: Vec<(String, String)> = state
            .and_then(|v| v.get("interfaces"))
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|i| (i["name"].as_str().unwrap_or("").to_string(), i["ip"].as_str().unwrap_or("").to_string())).collect())
            .unwrap_or_default();
        self.set_cards(t, &cards);
        let control = s("control");
        let all = control.is_empty() || control == "0.0.0.0" || control == "::";
        self.net_all.set_checked(all);
        for (item, ip) in &self.net_cards {
            item.set_checked(!all && *ip == control);
        }
        self.network.set_enabled(running && state.is_some_and(|v| v.get("interfaces").is_some()));
        self.port.set_text(t.port(port));
        self.port.set_enabled(running);
        self.start_stop.set_text(if running { t.stop() } else { t.start() });
        self.restart.set_enabled(running);
        let upd = state.and_then(|v| v.get("update"));
        let phase = upd.and_then(|u| u.get("phase")).and_then(Value::as_str).unwrap_or("idle");
        let available = upd.and_then(|u| u.get("available")).and_then(Value::as_str).map(str::to_string);
        match (phase, &available) {
            ("downloading" | "installing" | "restarting", _) => {
                let pct = (upd.and_then(|u| u.get("progress")).and_then(Value::as_f64).unwrap_or(0.0) * 100.0).round() as u32;
                self.update.set_text(t.downloading(available.as_deref().unwrap_or(""), pct));
                self.update.set_enabled(false);
            }
            (_, Some(v)) => {
                self.update.set_text(t.install(v));
                self.update.set_enabled(running);
            }
            _ => {
                self.update.set_text(t.check_update());
                self.update.set_enabled(running && upd.is_some() && phase != "checking");
            }
        }
    }
}

/// Runs a request off the menu thread and shows the server's error, if any.
fn background(f: impl FnOnce() -> Result<(), String> + Send + 'static) {
    std::thread::spawn(move || {
        if let Err(e) = f() {
            common::alert(PRODUCT, &e);
        }
    });
}

pub fn run() {
    // One launcher at a time.
    let Ok(_instance) = TcpListener::bind(common::SINGLE_INSTANCE) else { return };
    let t = Texts { lang: common::system_lang() };

    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let proxy = event_loop.create_proxy();
    {
        let p = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = p.send_event(UserEvent::Menu(e));
        }));
    }
    {
        let p = proxy.clone();
        std::thread::spawn(move || loop {
            let (dir, port) = common::locate();
            let st = common::fetch_state(port);
            if p.send_event(UserEvent::State(st, port, dir)).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        });
    }

    let mut ui: Option<Ui> = None;
    let mut state: Option<Value> = None;
    let mut port = common::DEFAULT_PORT;
    let mut data_dir: Option<std::path::PathBuf> = None;
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            // The icon is made once the loop runs (required on macOS).
            Event::NewEvents(StartCause::Init) => ui = Some(Ui::new(&t)),
            Event::UserEvent(UserEvent::State(s, p, d)) => {
                state = s;
                port = p;
                data_dir = d;
                if let Some(ui) = ui.as_mut() {
                    ui.refresh(&t, state.as_ref(), port);
                }
            }
            Event::UserEvent(UserEvent::Menu(e)) => {
                let Some(ui) = ui.as_ref() else { return };
                let id: &MenuId = &e.id;
                let running = state.is_some();
                if id == ui.quit.id() {
                    *control_flow = ControlFlow::Exit;
                } else if id == ui.copy.id() {
                    let url = ui.copy.text();
                    if let Some(u) = url.split_whitespace().last() {
                        common::copy(u);
                    }
                } else if id == ui.open.id() {
                    common::open(&format!("http://localhost:{port}/"));
                } else if id == ui.net_all.id() {
                    background(move || common::send("PUT", port, "/api/system/network", Some(json!({ "control": "0.0.0.0" }))));
                } else if let Some((_, ip)) = ui.net_cards.iter().find(|(i, _)| i.id() == id) {
                    let ip = ip.clone();
                    background(move || common::send("PUT", port, "/api/system/network", Some(json!({ "control": ip }))));
                } else if id == ui.port.id() {
                    let prompt = t.port_prompt();
                    background(move || {
                        let Some(answer) = common::ask_text(PRODUCT, &prompt, &port.to_string()) else { return Ok(()) };
                        let p: u16 = answer.trim().parse().map_err(|_| "1-65535".to_string())?;
                        common::send("PUT", port, "/api/system/network", Some(json!({ "port": p })))
                    });
                } else if id == ui.start_stop.id() {
                    if running {
                        background(move || common::send("POST", port, "/api/system/stop", None));
                    } else {
                        background(common::start_server);
                    }
                } else if id == ui.restart.id() {
                    background(move || common::send("POST", port, "/api/system/restart-server", None));
                } else if id == ui.update.id() {
                    let available = state.as_ref().and_then(|v| v["update"]["available"].as_str()).map(str::to_string);
                    match available {
                        Some(v) => {
                            let question = t.install_confirm(&v);
                            background(move || {
                                if common::confirm(PRODUCT, &question) {
                                    common::send("POST", port, &format!("/api/system/update/install?version={v}"), None)
                                } else {
                                    Ok(())
                                }
                            });
                        }
                        None => background(move || common::send("POST", port, "/api/system/update/check", None)),
                    }
                } else if id == ui.log.id() {
                    let file = data_dir.as_ref().map(|d| d.join("logs").join("strata-network.log")).filter(|f| f.exists());
                    match file {
                        Some(f) => common::open(&f.display().to_string()),
                        None => common::open(&format!("http://localhost:{port}/system/health")),
                    }
                } else if id == ui.uninstall.id() {
                    let question = t.uninstall_confirm();
                    background(move || if common::confirm(PRODUCT, &question) { common::uninstall() } else { Ok(()) });
                }
            }
            _ => {}
        }
    });
}
