//! FluxBar command-line interface.

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

mod client;
mod commands;
mod daemon;
mod default_config;
mod handler;
mod plugin_manager;

/// Blazing fast. Secure. Absolutely customizable.
#[derive(Debug, Parser)]
#[command(name = "fluxbar", version, about, long_about = None)]
struct Cli {
    /// Path to the FluxBar JSON-RPC socket.
    #[arg(long, global = true, env = "FLUXBAR_SOCKET")]
    socket: Option<PathBuf>,

    /// Path to the configuration file.
    #[arg(long, global = true, env = "FLUXBAR_CONFIG")]
    config: Option<PathBuf>,

    /// Increase logging verbosity.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the FluxBar daemon and the bar UI.
    Start {
        /// Do not start the native UI (useful for testing IPC).
        #[arg(long)]
        headless: bool,
    },
    /// Stop a running daemon.
    Stop,
    /// Add a new item.
    Add(AddArgs),
    /// Update properties of an existing item.
    Set(SetArgs),
    /// Remove an item.
    Remove { name: String },
    /// List configured items.
    List {
        /// List plugins instead of items.
        #[arg(long)]
        plugins: bool,
    },
    /// Trigger a custom event.
    Trigger {
        name: String,
        /// JSON payload.
        #[arg(long, default_value = "{}")]
        payload: String,
    },
    /// Show the daemon status.
    Status,
    /// Manage the configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Manage plugins.
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
    /// Show the socket path and configuration location.
    Paths,
}

#[derive(Debug, Args)]
struct AddArgs {
    name: String,
    #[arg(long, default_value = "text")]
    kind: String,
    #[arg(long, default_value = "left")]
    position: String,
    #[arg(long)]
    label: Option<String>,
    #[arg(long)]
    icon: Option<String>,
}

#[derive(Debug, Args)]
struct SetArgs {
    name: String,
    /// One or more `key=value` assignments.
    #[arg(required = true)]
    assignments: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Print the config file path.
    Path,
    /// Write a commented default configuration.
    Init {
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },
    /// Validate the configuration.
    Validate,
    /// Print the effective configuration.
    Show,
}

#[derive(Debug, Subcommand)]
enum PluginCommand {
    /// List running plugins.
    List,
    /// Install a plugin from git (`github:user/repo`) or a local path.
    Install(InstallArgs),
    /// Remove a plugin.
    Remove { name: String },
    /// Restart a plugin.
    Restart { name: String },
    /// Update a plugin (git pull, rebuild, re-sync copied sources).
    Update { name: String },
}

#[derive(Debug, Args)]
struct InstallArgs {
    /// Source: `github:user/repo`, a git URL, or a local path.
    source: String,
    /// Plugin name override.
    #[arg(long)]
    name: Option<String>,
    /// Skip running the build command.
    #[arg(long)]
    no_build: bool,
    /// Register a local plugin in place instead of copying it into the
    /// FluxBar configuration directory. Useful during development.
    #[arg(long)]
    link: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let socket = cli.socket.clone().unwrap_or_else(client::default_socket);
    let config = cli.config.clone();

    let result = match cli.command {
        Command::Start { headless } => {
            return match daemon::run(config, socket, headless) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("fluxbar: {error:#}");
                    ExitCode::FAILURE
                }
            };
        }
        Command::Stop => commands::stop(&socket),
        Command::Add(args) => commands::add(&socket, args),
        Command::Set(args) => commands::set(&socket, args),
        Command::Remove { name } => commands::remove(&socket, &name),
        Command::List { plugins } => commands::list(&socket, plugins),
        Command::Trigger { name, payload } => commands::trigger(&socket, &name, &payload),
        Command::Status => commands::status(&socket),
        Command::Config { command } => commands::config(config, command),
        Command::Plugin { command } => commands::plugin(&socket, config, command),
        Command::Paths => commands::paths(config, &socket),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fluxbar: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing(verbosity: u8) {
    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(format!("fluxbar={level},fluxbar_core={level}"))
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}
