//! The system service: systemd (Linux), launchd (macOS), Windows service.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const NAME: &str = "strata-network";
#[cfg(target_os = "macos")]
const LABEL: &str = "com.strata.network.server";
#[cfg(windows)]
pub const WINDOWS_NAME: &str = "StrataNetwork";

static WINDOWS_SERVICE: AtomicBool = AtomicBool::new(false);

/// Called when started by the Windows service manager (`--service`).
pub fn set_windows_service() {
    WINDOWS_SERVICE.store(true, Ordering::Relaxed);
}

/// Whether a service manager watches this process (it restarts us after `exit(75)`).
pub fn supervised() -> bool {
    let set = |k: &str| std::env::var(k).is_ok_and(|v| !v.is_empty() && v != "0");
    WINDOWS_SERVICE.load(Ordering::Relaxed)
        || set("INVOCATION_ID")
        || std::env::var("XPC_SERVICE_NAME").is_ok_and(|v| !v.is_empty() && v != "0" && !v.starts_with("application."))
        || std::env::var("SN_SUPERVISED").is_ok_and(|v| v == "1")
}

#[cfg(unix)]
fn run(cmd: &str, args: &[&str]) -> anyhow::Result<()> {
    let st = std::process::Command::new(cmd).args(args).status().map_err(|e| anyhow::anyhow!("cannot run {cmd}: {e}"))?;
    if !st.success() {
        anyhow::bail!("{cmd} {} failed ({st})", args.join(" "));
    }
    Ok(())
}

fn exe() -> anyhow::Result<PathBuf> {
    Ok(std::env::current_exe()?.canonicalize()?)
}

// ---------------------------------------------------------------- Linux (systemd)

#[cfg(all(unix, not(target_os = "macos")))]
fn unit_path() -> PathBuf {
    PathBuf::from(format!("/etc/systemd/system/{NAME}.service"))
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn unit_text(exe: &Path, bind: &str, data: &Path) -> String {
    format!(
        "[Unit]\nDescription=Strata Network\nAfter=network-online.target\nWants=network-online.target\n\n\
[Service]\nType=simple\nUser={NAME}\nGroup={NAME}\nExecStart=\"{}\" run --default-bind {bind} --data-dir \"{}\"\n\
Restart=always\nRestartSec=2\nNoNewPrivileges=yes\nProtectSystem=full\nReadWritePaths={}\n\
AmbientCapabilities=CAP_NET_BIND_SERVICE CAP_NET_RAW\nLimitNOFILE=65536\n\n[Install]\nWantedBy=multi-user.target\n",
        exe.display(),
        data.display(),
        data.display()
    )
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn install(bind: &str, data: &Path) -> anyhow::Result<()> {
    let exe = exe()?;
    if std::process::Command::new("id").arg("-u").arg(NAME).output().map(|o| !o.status.success()).unwrap_or(true) {
        run("useradd", &["--system", "--no-create-home", "--home-dir", &data.display().to_string(), "--shell", "/usr/sbin/nologin", NAME])?;
    }
    std::fs::create_dir_all(data)?;
    run("chown", &["-R", &format!("{NAME}:{NAME}"), &data.display().to_string()])?;
    std::fs::write(unit_path(), unit_text(&exe, bind, data))?;
    run("systemctl", &["daemon-reload"])?;
    run("systemctl", &["enable", "--now", NAME])?;
    println!("{NAME} installed and started (data in {})", data.display());
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn uninstall() -> anyhow::Result<()> {
    let _ = run("systemctl", &["disable", "--now", NAME]);
    let _ = std::fs::remove_file(unit_path());
    run("systemctl", &["daemon-reload"])?;
    println!("{NAME} removed (the data folder is kept)");
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn control(action: &str) -> anyhow::Result<()> {
    run("systemctl", &[action, NAME])
}

// ---------------------------------------------------------------- macOS (launchd)

#[cfg(target_os = "macos")]
fn plist_path() -> PathBuf {
    PathBuf::from(format!("/Library/LaunchDaemons/{LABEL}.plist"))
}

#[cfg(target_os = "macos")]
fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(target_os = "macos")]
pub fn install(bind: &str, data: &Path) -> anyhow::Result<()> {
    let exe = exe()?;
    std::fs::create_dir_all(data)?;
    let args = [exe.display().to_string(), "run".into(), "--default-bind".into(), bind.to_string(), "--data-dir".into(), data.display().to_string()];
    let args: String = args.iter().map(|a| format!("    <string>{}</string>\n", xml(a))).collect();
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{LABEL}</string>\n  <key>ProgramArguments</key>\n  <array>\n{args}  </array>\n\
  <key>RunAtLoad</key>\n  <true/>\n  <key>KeepAlive</key>\n  <dict>\n    <key>SuccessfulExit</key>\n    <false/>\n  </dict>\n\
  <key>SoftResourceLimits</key>\n  <dict>\n    <key>NumberOfFiles</key>\n    <integer>65536</integer>\n  </dict>\n</dict>\n</plist>\n"
    );
    let _ = run("launchctl", &["bootout", &format!("system/{LABEL}")]);
    std::fs::write(plist_path(), plist)?;
    run("launchctl", &["bootstrap", "system", &plist_path().display().to_string()])?;
    println!("{NAME} installed and started (data in {})", data.display());
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> anyhow::Result<()> {
    let _ = run("launchctl", &["bootout", &format!("system/{LABEL}")]);
    let _ = std::fs::remove_file(plist_path());
    println!("{NAME} removed (the data folder is kept)");
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn control(action: &str) -> anyhow::Result<()> {
    let target = format!("system/{LABEL}");
    match action {
        "start" => run("launchctl", &["kickstart", &target]),
        "stop" => run("launchctl", &["kill", "SIGTERM", &target]),
        _ => run("launchctl", &["print", &target]),
    }
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
pub fn install(bind: &str, data: &Path) -> anyhow::Result<()> {
    use std::ffi::OsString;
    use std::time::Duration;
    use windows_service::service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl, ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType,
        ServiceType,
    };
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    let exe = exe()?;
    std::fs::create_dir_all(data)?;
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
    let info = ServiceInfo {
        name: OsString::from(WINDOWS_NAME),
        display_name: OsString::from("Strata Network"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec!["run".into(), "--service".into(), "--default-bind".into(), bind.into(), "--data-dir".into(), data.as_os_str().to_owned()],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let service = manager.create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)?;
    let restart = ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(2) };
    service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86400)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![restart.clone(), restart.clone(), restart]),
    })?;
    service.set_failure_actions_on_non_crash_failures(true)?;
    service.start::<&str>(&[])?;
    println!("{NAME} installed and started (data in {})", data.display());
    Ok(())
}

#[cfg(windows)]
pub fn uninstall() -> anyhow::Result<()> {
    use windows_service::service::ServiceAccess;
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(WINDOWS_NAME, ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS)?;
    let _ = service.stop();
    service.delete()?;
    println!("{NAME} removed (the data folder is kept)");
    Ok(())
}

#[cfg(windows)]
pub fn control(action: &str) -> anyhow::Result<()> {
    use windows_service::service::ServiceAccess;
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(WINDOWS_NAME, ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::QUERY_STATUS)?;
    match action {
        "start" => service.start::<&str>(&[])?,
        "stop" => {
            service.stop()?;
        }
        _ => println!("{:?}", service.query_status()?.current_state),
    }
    Ok(())
}

/// Runs the server under the Windows service manager.
#[cfg(windows)]
pub mod windows_run {
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};

    use crate::RunArgs;

    static ARGS: OnceLock<Mutex<Option<RunArgs>>> = OnceLock::new();

    define_windows_service!(ffi_service_main, service_main);

    pub fn start(args: RunArgs) -> anyhow::Result<()> {
        let _ = ARGS.set(Mutex::new(Some(args)));
        service_dispatcher::start(super::WINDOWS_NAME, ffi_service_main)?;
        Ok(())
    }

    fn status(state: ServiceState, accept: ServiceControlAccept) -> ServiceStatus {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        }
    }

    fn service_main(_arguments: Vec<OsString>) {
        let Some(args) = ARGS.get().and_then(|m| m.lock().unwrap().take()) else { return };
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let tx = Mutex::new(Some(tx));
        let handler = move |event| match event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(handle) = service_control_handler::register(super::WINDOWS_NAME, handler) else { return };
        let _ = handle.set_service_status(status(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN));
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build();
        if let Ok(rt) = rt {
            let result = rt.block_on(crate::serve(args, async move {
                let _ = rx.await;
            }));
            if let Err(e) = result {
                tracing::error!("server stopped: {e:#}");
            }
        }
        let _ = handle.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty()));
    }
}

// ---------------------------------------------------------------- other systems

#[cfg(not(any(unix, windows)))]
pub fn install(_bind: &str, _data: &Path) -> anyhow::Result<()> {
    anyhow::bail!("services are not supported on this system")
}
#[cfg(not(any(unix, windows)))]
pub fn uninstall() -> anyhow::Result<()> {
    anyhow::bail!("services are not supported on this system")
}
#[cfg(not(any(unix, windows)))]
pub fn control(_action: &str) -> anyhow::Result<()> {
    anyhow::bail!("services are not supported on this system")
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn systemd_unit() {
        let u = unit_text(Path::new("/opt/strata/strata-network"), "0.0.0.0:8080", Path::new("/var/lib/strata-network"));
        assert!(u.contains("ExecStart=\"/opt/strata/strata-network\" run --default-bind 0.0.0.0:8080 --data-dir \"/var/lib/strata-network\""));
        assert!(u.contains("Restart=always") && u.contains("RestartSec=2") && u.contains("ReadWritePaths=/var/lib/strata-network"));
        assert!(u.contains("AmbientCapabilities=CAP_NET_BIND_SERVICE CAP_NET_RAW") && u.contains("LimitNOFILE=65536") && u.contains("ProtectSystem=full"));
    }
}
