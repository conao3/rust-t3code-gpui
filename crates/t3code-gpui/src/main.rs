mod app;
mod appearance;
mod attachments;
mod client_settings;
mod completion;
mod drafts;
mod explorer;
mod keymap;
mod markdown_directives;
mod models;
mod navigation;
mod permissions;
mod projects;
mod row_view;
mod sidebar;
mod tables;
mod terminal;
mod terminal_paint;
mod terminal_view;
mod timeline;
mod timestamp;
mod workspace;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use t3_client::config::Config;
use t3_client::session::Connection;

const EXIT_USAGE: u8 = 252;
const EXIT_TARGET: u8 = 254;
const EXIT_OTHER: u8 = 255;

#[derive(Parser)]
#[command(
    version,
    about = "T3 Code GPUI (Unofficial) — independent native frontend"
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Desktop {
        #[command(subcommand)]
        command: DesktopCommand,
    },
    Environment {
        #[command(subcommand)]
        command: EnvironmentCommand,
    },
}

#[derive(Subcommand)]
enum DesktopCommand {
    StartApp,
}

#[derive(Subcommand)]
enum EnvironmentCommand {
    ListEnvironments,
    CheckEnvironment {
        #[arg(long)]
        label: String,
    },
}

enum Failure {
    Target(anyhow::Error),
    Other(anyhow::Error),
}

fn config_path(path: Option<PathBuf>) -> Result<PathBuf, Failure> {
    match path {
        Some(path) => Ok(path),
        None => Config::default_path().map_err(Failure::Other),
    }
}

fn load(path: Option<PathBuf>) -> Result<Config, Failure> {
    Config::load(&config_path(path)?).map_err(Failure::Other)
}

fn list_environments(config: &Config) -> Value {
    json!({
        "Environments": config.environments.iter().map(|e| json!({
            "Label": e.label,
            "HttpBaseUrl": e.http_base_url,
        })).collect::<Vec<_>>()
    })
}

fn check_environment(config: &Config, label: &str) -> Result<Value, Failure> {
    let environment = config
        .find(label)
        .ok_or_else(|| Failure::Other(anyhow::anyhow!("no environment is labeled {label}")))?;
    let runtime = tokio::runtime::Runtime::new().map_err(|e| Failure::Other(e.into()))?;
    let (sender, _receiver) = async_channel::unbounded();
    let connection = Connection::new(environment, sender);
    runtime.block_on(async {
        let descriptor = connection.describe().await.map_err(Failure::Target)?;
        let session = connection.auth_session().await.map_err(Failure::Target)?;
        Ok(json!({
            "Label": environment.label,
            "EnvironmentId": descriptor.get("environmentId"),
            "ServerVersion": descriptor.get("serverVersion"),
            "OrchestrationProtocolVersion": descriptor.get("orchestrationProtocolVersion"),
            "Authenticated": session.get("authenticated"),
        }))
    })
}

fn run(cli: Cli) -> Result<(), Failure> {
    match cli.command {
        None
        | Some(Command::Desktop {
            command: DesktopCommand::StartApp,
        }) => {
            let settings_path = client_settings::default_path().map_err(Failure::Other)?;
            let settings =
                client_settings::ClientSettings::load(&settings_path).map_err(Failure::Other)?;
            let path = config_path(cli.config)?;
            let config = Config::load(&path).map_err(Failure::Other)?;
            app::run(config, path, (settings, settings_path)).map_err(Failure::Other)
        }
        Some(Command::Environment { command }) => {
            let config = load(cli.config)?;
            let output = match command {
                EnvironmentCommand::ListEnvironments => list_environments(&config),
                EnvironmentCommand::CheckEnvironment { label } => {
                    check_environment(&config, &label)?
                }
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&output).expect("JSON output")
            );
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = if error.use_stderr() { EXIT_USAGE } else { 0 };
            let _ = error.print();
            return ExitCode::from(code);
        }
    };
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Target(error)) => {
            eprintln!("{error:#}");
            ExitCode::from(EXIT_TARGET)
        }
        Err(Failure::Other(error)) => {
            eprintln!("{error:#}");
            ExitCode::from(EXIT_OTHER)
        }
    }
}
