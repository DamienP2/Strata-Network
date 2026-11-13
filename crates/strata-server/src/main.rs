//! `strata-network`: the command line of the Strata Network server.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use strata_server::{auth, paths, prefs, service, RunArgs, DEFAULT_BIND};

#[derive(Parser)]
#[command(name = "strata-network", version, about = "Strata Network server", args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    #[command(flatten)]
    run: RunArgs,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the server (the default)
    Run(RunArgs),
    /// Install or control the system service
    Service {
        #[command(subcommand)]
        action: ServiceCmd,
    },
    /// Manage the accounts
    User {
        #[command(subcommand)]
        action: UserCmd,
    },
}

#[derive(Subcommand)]
enum ServiceCmd {
    /// Install and start the service
    Install {
        #[arg(long, default_value = DEFAULT_BIND)]
        bind: SocketAddr,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Stop and remove the service (the data is kept)
    Uninstall,
    Start,
    Stop,
    Status,
}

#[derive(Subcommand)]
enum UserCmd {
    /// Create an account (asks the password twice)
    Add {
        name: String,
        /// admin | operator | viewer (default: admin for the first account, else viewer)
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// List the accounts
    List {
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Change a password
    Passwd {
        name: String,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Delete an account
    Delete {
        name: String,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.cmd {
        None => strata_server::run(cli.run),
        Some(Cmd::Run(a)) => strata_server::run(a),
        Some(Cmd::Service { action }) => service_cmd(action),
        Some(Cmd::User { action }) => user_cmd(action),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn service_cmd(action: ServiceCmd) -> anyhow::Result<()> {
    match action {
        ServiceCmd::Install { bind, data_dir } => service::install(&bind.to_string(), &data_dir.unwrap_or_else(paths::service_data_dir)),
        ServiceCmd::Uninstall => service::uninstall(),
        ServiceCmd::Start => service::control("start"),
        ServiceCmd::Stop => service::control("stop"),
        ServiceCmd::Status => service::control("status"),
    }
}

fn ask_password() -> anyhow::Result<String> {
    let a = rpassword::prompt_password("Password: ")?;
    let b = rpassword::prompt_password("Password again: ")?;
    if a != b {
        anyhow::bail!("the passwords do not match");
    }
    auth::check_password(&a).map_err(|e| anyhow::anyhow!(e))?;
    Ok(a)
}

fn user_cmd(action: UserCmd) -> anyhow::Result<()> {
    let open = |d: Option<PathBuf>| -> anyhow::Result<(auth::Auth, PathBuf)> {
        let dir = paths::cli_data_dir(d);
        std::fs::create_dir_all(&dir)?;
        Ok((auth::Auth::load(&dir), dir))
    };
    match action {
        UserCmd::Add { name, role, data_dir } => {
            let (a, dir) = open(data_dir)?;
            let role = match role {
                Some(r) => auth::Role::parse(&r).ok_or_else(|| anyhow::anyhow!("unknown role {r} (admin, operator, viewer)"))?,
                None if a.needs_setup() => auth::Role::Admin,
                None => auth::Role::Viewer,
            };
            auth::check_username(name.trim()).map_err(|e| anyhow::anyhow!(e))?;
            let password = ask_password()?;
            let u = a.create(auth::UserInput { username: name, password, role: Some(role), ..Default::default() }, false).map_err(|e| anyhow::anyhow!(e))?;
            println!("{} ({}) created in {}", u.username, u.role.as_str(), dir.display());
        }
        UserCmd::List { data_dir } => {
            let (a, dir) = open(data_dir)?;
            println!("{}", dir.display());
            for u in a.list() {
                println!(
                    "  {:<24} {:<9} {}{}",
                    u.username,
                    u.role.as_str(),
                    if u.disabled { "disabled " } else { "" },
                    if u.sso.is_empty() { "" } else { "sso" }
                );
            }
        }
        UserCmd::Passwd { name, data_dir } => {
            let (a, _) = open(data_dir)?;
            if a.find_by_name(&name).is_none() {
                anyhow::bail!("no such user");
            }
            let password = ask_password()?;
            a.set_password(&name, &password).map_err(|e| anyhow::anyhow!(e))?;
            println!("password of {name} changed");
        }
        UserCmd::Delete { name, data_dir } => {
            let (a, dir) = open(data_dir)?;
            let u = a.find_by_name(&name).ok_or_else(|| anyhow::anyhow!("no such user"))?;
            a.delete(&u.id, "").map_err(|e| anyhow::anyhow!(e))?;
            prefs::forget(&dir, &u.id);
            println!("{} deleted", u.username);
        }
    }
    Ok(())
}
