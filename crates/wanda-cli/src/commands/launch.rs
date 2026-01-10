//! wanda launch - Launch a game with WeMod

use clap::Args;
use console::style;
use std::path::PathBuf;
use wanda_core::{
    config::WandaConfig,
    launcher::{GameLauncher, LaunchConfig},
    prefix::PrefixManager,
    steam::{ProtonManager, SteamInstallation},
    Result, WandaError,
};

#[derive(Args)]
pub struct LaunchArgs {
    /// Game name or App ID
    game: String,

    /// Launch without WeMod
    #[arg(long)]
    no_wemod: bool,

    /// Delay in seconds before launching game (after WeMod starts)
    #[arg(long, default_value = "3")]
    delay: u64,

    /// Additional arguments to pass to the game
    #[arg(long)]
    args: Option<String>,

    /// Wait for the game to exit
    #[arg(long, short)]
    wait: bool,
}

pub async fn run(args: LaunchArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = match &config_path {
        Some(path) => WandaConfig::load_from(path)?,
        None => WandaConfig::load()?,
    };

    // Discover Steam and find the game
    let steam = SteamInstallation::discover(&config)?;

    // Try to parse as App ID first
    let game = if let Ok(app_id) = args.game.parse::<u32>() {
        steam.find_game(app_id).ok_or(WandaError::GameNotFound { app_id })?
    } else {
        // Search by name
        let matches = steam.find_game_by_name(&args.game);
        match matches.len() {
            0 => {
                return Err(WandaError::LaunchFailed {
                    reason: format!("No game found matching '{}'", args.game),
                })
            }
            1 => matches[0],
            _ => {
                println!("Multiple games found matching '{}':", args.game);
                for game in &matches {
                    println!("  {} - {}", game.app_id, game.name);
                }
                return Err(WandaError::LaunchFailed {
                    reason: "Please specify the exact App ID".to_string(),
                });
            }
        }
    };

    println!(
        "Launching {} {}",
        style(&game.name).bold(),
        style(format!("({})", game.app_id)).dim()
    );

    // Check if launching with WeMod
    let with_wemod = !args.no_wemod;

    if with_wemod {
        // Load WANDA prefix
        let mut prefix_manager = PrefixManager::new(&config);
        prefix_manager.load()?;

        let prefix = prefix_manager.get("default").ok_or_else(|| WandaError::LaunchFailed {
            reason: "WANDA not initialized. Run 'wanda init' first.".to_string(),
        })?;

        // Check if WeMod is installed
        if !prefix.wemod_installed {
            return Err(WandaError::WemodNotInstalled);
        }

        // Get Proton version
        let proton_manager = ProtonManager::discover(&steam, &config)?;
        let proton = proton_manager.get_preferred(&config)?;

        println!("  Using WeMod with {}", proton.name);

        // Create launcher
        let launcher = GameLauncher::new(&steam, prefix, proton);

        let launch_config = LaunchConfig {
            app_id: game.app_id,
            with_wemod: true,
            wemod_delay: args.delay,
            extra_args: args
                .args
                .map(|a| a.split_whitespace().map(String::from).collect())
                .unwrap_or_default(),
            ..Default::default()
        };

        let mut handle = launcher.launch(launch_config).await?;

        println!(
            "\n{} Game launched with WeMod!",
            style("SUCCESS").green().bold()
        );
        println!("\nWeMod should appear in a separate window.");
        println!("Select your game in WeMod and click Play to activate trainers.\n");

        if args.wait {
            println!("Waiting for session to end...");
            handle.wait().await?;
            println!(
                "Session ended. Play time: {:?}",
                handle.elapsed()
            );
        }
    } else {
        // Launch without WeMod (just use Steam)
        println!("  Launching via Steam (without WeMod)...");

        let steam_url = format!("steam://rungameid/{}", game.app_id);
        let status = tokio::process::Command::new("xdg-open")
            .arg(&steam_url)
            .status()
            .await
            .map_err(|e| WandaError::LaunchFailed {
                reason: format!("Failed to launch: {}", e),
            })?;

        if status.success() {
            println!("\n{} Game launched!", style("SUCCESS").green().bold());
        }
    }

    Ok(())
}
