//! WANDA CLI - WeMod launcher for Linux

mod commands;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser)]
#[command(name = "wanda")]
#[command(author = "WANDA Contributors")]
#[command(version)]
#[command(about = "WeMod launcher for Linux", long_about = None)]
struct Cli {
    /// Increase verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Suppress non-essential output
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Output in JSON format
    #[arg(long, global = true)]
    json: bool,

    /// Use a custom config file
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize WANDA and set up WeMod prefix
    Init(commands::init::InitArgs),

    /// Scan for Steam games
    Scan(commands::scan::ScanArgs),

    /// Launch a game with WeMod
    Launch(commands::launch::LaunchArgs),

    /// Manage Wine/Proton prefixes
    #[command(subcommand)]
    Prefix(commands::prefix::PrefixCommands),

    /// Manage WeMod installation
    #[command(subcommand)]
    Wemod(commands::wemod::WemodCommands),

    /// Manage configuration
    #[command(subcommand)]
    Config(commands::config::ConfigCommands),

    /// Diagnose issues and generate reports
    Doctor(commands::doctor::DoctorArgs),
}

fn setup_logging(verbose: u8, quiet: bool) {
    let filter = if quiet {
        "warn"
    } else {
        match verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
    };

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter));

    tracing_subscriber::registry()
        .with(fmt::layer().with_target(false).without_time())
        .with(filter)
        .init();
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    setup_logging(cli.verbose, cli.quiet);

    let result = match cli.command {
        Commands::Init(args) => commands::init::run(args, cli.config).await,
        Commands::Scan(args) => commands::scan::run(args, cli.config).await,
        Commands::Launch(args) => commands::launch::run(args, cli.config).await,
        Commands::Prefix(cmd) => commands::prefix::run(cmd, cli.config).await,
        Commands::Wemod(cmd) => commands::wemod::run(cmd, cli.config).await,
        Commands::Config(cmd) => commands::config::run(cmd, cli.config).await,
        Commands::Doctor(args) => commands::doctor::run(args, cli.config).await,
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e.user_message());
        std::process::exit(1);
    }
}
