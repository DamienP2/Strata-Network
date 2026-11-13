#![cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]

//! What the launcher does without a window system: where the server is, its texts in French or
//! English, the HTTP calls, and the system helpers (open, copy, dialogs).

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

pub const PRODUCT: &str = "Strata Network";
/// Only one launcher at a time: it holds this port.
pub const SINGLE_INSTANCE: &str = "127.0.0.1:47819";
pub const DEFAULT_PORT: u16 = 8080;
#[cfg(target_os = "macos")]
pub const LAUNCHD_LABEL: &str = "com.strata.network.server";
#[cfg(windows)]
pub const WINDOWS_SERVICE: &str = "StrataNetwork";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Fr,
}

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let mut c = Command::new(cmd);
    c.args(args);
    hide_window(&mut c);
    let out = c.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// French when the system speaks French, else English.
pub fn system_lang() -> Lang {
    for k in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() && v != "C" && v != "POSIX" {
                return if v.to_ascii_lowercase().starts_with("fr") { Lang::Fr } else { Lang::En };
            }
        }
    }
    #[cfg(target_os = "macos")]
    let sys = output("defaults", &["read", "-g", "AppleLocale"]);
    #[cfg(windows)]
    let sys = output("powershell", &["-NoProfile", "-Command", "(Get-UICulture).TwoLetterISOLanguageName"]);
    #[cfg(not(any(target_os = "macos", windows)))]
    let sys: Option<String> = None;
    match sys {
        Some(s) if s.to_ascii_lowercase().starts_with("fr") => Lang::Fr,
        _ => Lang::En,
    }
}

/// The menu texts.
pub struct Texts {
    pub lang: Lang,
}

impl Texts {
    fn pick(&self, en: &str, fr: &str) -> String {
        if self.lang == Lang::Fr { fr.to_string() } else { en.to_string() }
    }
    pub fn running(&self, name: &str) -> String {
        format!("● {name} — {}", self.pick("running", "en marche"))
    }
    pub fn stopped(&self, name: &str) -> String {
        format!("○ {name} — {}", self.pick("stopped", "arrêté"))
    }
    pub fn checking(&self, name: &str) -> String {
        format!("{name} — {}", self.pick("checking…", "vérification…"))
    }
    pub fn info(&self, version: &str, port: u16, uptime: &str) -> String {
        match self.lang {
            Lang::Fr => format!("Version {version} · port {port} · en marche depuis {uptime}"),
            Lang::En => format!("Version {version} · port {port} · up for {uptime}"),
        }
    }
    pub fn copy(&self, url: &str) -> String {
        format!("{}  {url}", self.pick("Copy the address", "Copier l’adresse"))
    }
    pub fn open_web(&self) -> String {
        self.pick("Open the web page", "Ouvrir la page web")
    }
    pub fn network(&self) -> String {
        self.pick("Network card", "Carte réseau")
    }
    pub fn web_interface(&self) -> String {
        self.pick("Web interface:", "Interface web\u{a0}:")
    }
    pub fn all_cards(&self) -> String {
        self.pick("All cards", "Toutes les cartes")
    }
    pub fn port(&self, port: u16) -> String {
        match self.lang {
            Lang::Fr => format!("Port de connexion\u{a0}: {port}…"),
            Lang::En => format!("Connection port: {port}…"),
        }
    }
    pub fn port_prompt(&self) -> String {
        self.pick("Port of the web interface (1-65535):", "Port de l’interface web (1-65535)\u{a0}:")
    }
    pub fn start(&self) -> String {
        self.pick("Start the server", "Démarrer le serveur")
    }
    pub fn stop(&self) -> String {
        self.pick("Stop the server", "Arrêter le serveur")
    }
    pub fn restart(&self) -> String {
        self.pick("Restart the server", "Redémarrer le serveur")
    }
    pub fn check_update(&self) -> String {
        self.pick("Check for an update", "Rechercher une mise à jour")
    }
    pub fn install(&self, v: &str) -> String {
        match self.lang {
            Lang::Fr => format!("Installer la version {v}…"),
            Lang::En => format!("Install version {v}…"),
        }
    }
    pub fn install_confirm(&self, v: &str) -> String {
        match self.lang {
            Lang::Fr => format!("Installer {PRODUCT} {v}\u{a0}? Le serveur redémarre\u{a0}: il est indisponible quelques secondes."),
            Lang::En => format!("Install {PRODUCT} {v}? The server restarts: it is unavailable for a few seconds."),
        }
    }
    pub fn downloading(&self, v: &str, pct: u32) -> String {
        match self.lang {
            Lang::Fr => format!("Téléchargement de la version {v}… {pct}\u{a0}%"),
            Lang::En => format!("Downloading version {v}… {pct} %"),
        }
    }
    pub fn log(&self) -> String {
        self.pick("Server log…", "Journal du serveur…")
    }
    pub fn uninstall(&self) -> String {
        match self.lang {
            Lang::Fr => format!("Désinstaller {PRODUCT}…"),
            Lang::En => format!("Uninstall {PRODUCT}…"),
        }
    }
    pub fn uninstall_confirm(&self) -> String {
        match self.lang {
            Lang::Fr => format!("Désinstaller {PRODUCT}\u{a0}? Les données sont gardées."),
            Lang::En => format!("Uninstall {PRODUCT}? The data is kept."),
        }
    }
    pub fn quit(&self) -> String {
        self.pick("Quit", "Quitter")
    }
}

/// `2d 3h`, `3h 12m`, `5m 10s`.
pub fn fmt_uptime(secs: u64) -> String {
    let (d, h, m, s) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m {s}s")
    }
}

/// The server's data folders: the installed service's first, then the user's.
pub fn data_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    #[cfg(target_os = "macos")]
    out.push(PathBuf::from("/Library/Application Support/Strata Network"));
    #[cfg(windows)]
    out.push(std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData")).join("Strata Network"));
    #[cfg(not(any(target_os = "macos", windows)))]
    out.push(PathBuf::from("/var/lib/strata-network"));
    if let Some(d) = directories::ProjectDirs::from("com", "Strata", "Strata Network") {
        out.push(d.data_dir().to_path_buf());
    }
    out
}

fn port_of(bind: &str) -> Option<u16> {
    bind.rsplit_once(':').and_then(|(_, p)| p.parse().ok()).filter(|p| *p != 0)
}

/// The server's port: `runtime.json`, else `config.json`, else 8080.
pub fn port_from(runtime: Option<&Value>, config: Option<&Value>) -> u16 {
    runtime
        .and_then(|r| r.get("bind")?.as_str().and_then(port_of))
        .or_else(|| config.and_then(|c| c.get("bind")?.as_str().and_then(port_of)))
        .unwrap_or(DEFAULT_PORT)
}

fn read_json(p: &std::path::Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

/// The data folder in use (the one with a `runtime.json`) and the port.
pub fn locate() -> (Option<PathBuf>, u16) {
    let dirs = data_dirs();
    for d in &dirs {
        if let Some(r) = read_json(&d.join("runtime.json")) {
            return (Some(d.clone()), port_from(Some(&r), None));
        }
    }
    for d in &dirs {
        if let Some(c) = read_json(&d.join("config.json")) {
            return (Some(d.clone()), port_from(None, Some(&c)));
        }
    }
    (None, DEFAULT_PORT)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(5))).http_status_as_error(false).build().new_agent()
}

fn read_body(mut res: ureq::http::Response<ureq::Body>) -> (u16, String) {
    let code = res.status().as_u16();
    let text = res.body_mut().read_to_string().unwrap_or_default();
    (code, text)
}

/// The server's state for the menu, or `None` when it does not answer.
pub fn fetch_state(port: u16) -> Option<Value> {
    let a = agent();
    if let Ok(res) = a.get(&format!("http://127.0.0.1:{port}/api/system/tray")).call() {
        let (code, text) = read_body(res);
        if code == 200 {
            return serde_json::from_str(&text).ok();
        }
    }
    let res = a.get(&format!("http://127.0.0.1:{port}/api/health")).call().ok()?;
    let (code, text) = read_body(res);
    (code == 200).then(|| serde_json::from_str(&text).ok()).flatten()
}

/// Sends a request to the server; the error is the server's text.
pub fn send(method: &str, port: u16, path: &str, body: Option<Value>) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{port}{path}");
    let a = agent();
    let res = match (method, body) {
        ("PUT", Some(b)) => a.put(&url).header("Content-Type", "application/json").send(b.to_string()),
        ("PUT", None) => a.put(&url).send_empty(),
        (_, Some(b)) => a.post(&url).header("Content-Type", "application/json").send(b.to_string()),
        (_, None) => a.post(&url).send_empty(),
    }
    .map_err(|e| e.to_string())?;
    let (code, text) = read_body(res);
    if (200..300).contains(&code) {
        Ok(())
    } else {
        Err(if text.is_empty() { format!("HTTP {code}") } else { text })
    }
}

#[cfg(windows)]
pub fn hide_window(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    c.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn hide_window(_c: &mut Command) {}

fn spawn(cmd: &str, args: &[&str]) {
    let mut c = Command::new(cmd);
    c.args(args);
    hide_window(&mut c);
    let _ = c.spawn();
}

/// Opens a web page or a file with the system.
pub fn open(target: &str) {
    #[cfg(target_os = "macos")]
    spawn("open", &[target]);
    #[cfg(windows)]
    spawn("rundll32", &["url.dll,FileProtocolHandler", target]);
    #[cfg(not(any(target_os = "macos", windows)))]
    spawn("xdg-open", &[target]);
}

/// Text to the clipboard.
pub fn copy(text: &str) {
    use std::io::Write;
    #[cfg(target_os = "macos")]
    let mut c = Command::new("pbcopy");
    #[cfg(windows)]
    let mut c = {
        let mut c = Command::new("clip");
        hide_window(&mut c);
        c
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut c = Command::new("xclip");
    if let Ok(mut child) = c.stdin(std::process::Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

/// Quotes for AppleScript or PowerShell single-quoted strings.
#[cfg_attr(windows, allow(dead_code))]
pub fn quote_applescript(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn quote_powershell(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The system's text input box; `None` when cancelled.
pub fn ask_text(title: &str, prompt: &str, default: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "text returned of (display dialog {} default answer {} with title {})",
            quote_applescript(prompt),
            quote_applescript(default),
            quote_applescript(title)
        );
        output("osascript", &["-e", &script])
    }
    #[cfg(windows)]
    {
        let script = format!(
            "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.Interaction]::InputBox({}, {}, {})",
            quote_powershell(prompt),
            quote_powershell(title),
            quote_powershell(default)
        );
        output("powershell", &["-NoProfile", "-Command", &script]).filter(|s| !s.is_empty())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (title, prompt, default);
        None
    }
}

/// A yes / no question.
pub fn confirm(title: &str, message: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        let script = format!("display dialog {} with title {}", quote_applescript(message), quote_applescript(title));
        output("osascript", &["-e", &script]).is_some()
    }
    #[cfg(windows)]
    {
        let script = format!(
            "Add-Type -AssemblyName PresentationFramework; [System.Windows.MessageBox]::Show({}, {}, 'OKCancel')",
            quote_powershell(message),
            quote_powershell(title)
        );
        output("powershell", &["-NoProfile", "-Command", &script]).is_some_and(|s| s == "OK")
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (title, message);
        false
    }
}

/// A message (errors of the server).
pub fn alert(title: &str, message: &str) {
    #[cfg(target_os = "macos")]
    {
        let script = format!("display alert {} message {}", quote_applescript(title), quote_applescript(message));
        let _ = output("osascript", &["-e", &script]);
    }
    #[cfg(windows)]
    {
        let script = format!(
            "Add-Type -AssemblyName PresentationFramework; [System.Windows.MessageBox]::Show({}, {}) | Out-Null",
            quote_powershell(message),
            quote_powershell(title)
        );
        let _ = output("powershell", &["-NoProfile", "-Command", &script]);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    eprintln!("{title}: {message}");
}

fn server_exe() -> Option<PathBuf> {
    let name = if cfg!(windows) { "strata-network.exe" } else { "strata-network" };
    let here = std::env::current_exe().ok()?;
    let p = here.with_file_name(name);
    p.exists().then_some(p)
}

/// Starts the server: the installed service (asks for the administrator password), else the
/// program next to the launcher.
pub fn start_server() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if std::path::Path::new(&format!("/Library/LaunchDaemons/{LAUNCHD_LABEL}.plist")).exists() {
        let script = format!("do shell script \"launchctl kickstart system/{LAUNCHD_LABEL}\" with administrator privileges");
        return output("osascript", &["-e", &script]).map(|_| ()).ok_or_else(|| "the service did not start".into());
    }
    #[cfg(windows)]
    if output("sc", &["query", WINDOWS_SERVICE]).is_some() {
        // Signed-in users may start the service; elevation only if this machine refuses.
        if output("sc", &["start", WINDOWS_SERVICE]).is_some() {
            return Ok(());
        }
        let script = format!("Start-Process sc.exe -ArgumentList 'start {WINDOWS_SERVICE}' -Verb RunAs -WindowStyle Hidden");
        return output("powershell", &["-NoProfile", "-Command", &script]).map(|_| ()).ok_or_else(|| "the service did not start".into());
    }
    let exe = server_exe().ok_or("the server program is not next to the launcher")?;
    let mut c = Command::new(exe);
    hide_window(&mut c);
    c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    c.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Runs the uninstaller.
pub fn uninstall() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            PathBuf::from("/Library/Application Support/Strata Network/uninstall.sh"),
            std::env::current_exe().ok().and_then(|e| Some(e.parent()?.parent()?.join("Resources/uninstall.sh"))).unwrap_or_default(),
        ];
        let script = candidates.iter().find(|p| p.is_file()).ok_or("the uninstaller is missing")?;
        let cmd = format!("do shell script {} with administrator privileges", quote_applescript(&format!("/bin/sh '{}'", script.display())));
        output("osascript", &["-e", &cmd]).map(|_| ()).ok_or_else(|| "uninstall cancelled".into())
    }
    #[cfg(windows)]
    {
        open("ms-settings:appsfeatures");
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    Err("not supported here".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uptime() {
        assert_eq!(fmt_uptime(5 * 60 + 10), "5m 10s");
        assert_eq!(fmt_uptime(3 * 3600 + 12 * 60 + 5), "3h 12m");
        assert_eq!(fmt_uptime(2 * 86400 + 3 * 3600), "2d 3h");
    }

    #[test]
    fn ports() {
        assert_eq!(port_from(Some(&json!({"bind": "0.0.0.0:9090"})), None), 9090);
        assert_eq!(port_from(Some(&json!({"bind": "[::]:8081"})), None), 8081);
        assert_eq!(port_from(None, Some(&json!({"bind": "127.0.0.1:7000"}))), 7000);
        assert_eq!(port_from(None, Some(&json!({"bind": null}))), 8080);
        assert_eq!(port_from(None, None), 8080);
    }

    #[test]
    fn texts() {
        let fr = Texts { lang: Lang::Fr };
        assert_eq!(fr.running("Strata Network"), "● Strata Network — en marche");
        assert_eq!(fr.port(8080), "Port de connexion\u{a0}: 8080…");
        assert_eq!(fr.downloading("1.2.0", 42), "Téléchargement de la version 1.2.0… 42\u{a0}%");
        let en = Texts { lang: Lang::En };
        assert_eq!(en.stopped("X"), "○ X — stopped");
    }

    #[test]
    fn quoting() {
        assert_eq!(quote_applescript("a \"b\""), "\"a \\\"b\\\"\"");
        assert_eq!(quote_powershell("l'adresse"), "'l''adresse'");
    }
}
