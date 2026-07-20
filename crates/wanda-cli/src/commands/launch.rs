//! wanda launch - Launch a game with WeMod

use clap::Args;
use console::style;
use std::path::PathBuf;
use wanda_core::{
    config::WandaConfig,
    launcher::{GameLauncher, LaunchConfig},
    prefix::{PrefixBuilder, PrefixManager},
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

    /// Standalone mode: launch WeMod only, start the game from WeMod's UI
    #[arg(long)]
    standalone: bool,

    /// Specify Proton version to use (overrides config)
    #[arg(long)]
    proton: Option<String>,
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

        // Prefer the GAME's own compatdata prefix over wanda's default
        // prefix whenever it exists. That prefix is what a normal Steam
        // launch uses, so it already contains the game's dependencies —
        // Ubisoft Connect, EA/Epic launchers, redists, etc. Running the
        // game there means it launches "as if wanda wasn't involved",
        // while WeMod (symlinked in from wanda's prefix) shares the same
        // wineserver so it can still hook the game.
        //
        // Only fall back to wanda's default prefix for games that have
        // never been launched through Proton (no compatdata yet).
        // Standalone mode requires the game prefix and errors without it.
        let game_prefix;
        let launch_prefix = match game.compat_data_path.as_ref() {
            Some(compat_data) => {
                println!(
                    "  Using game prefix: {}",
                    style(compat_data.display()).dim()
                );

                // Symlink WeMod into the game's prefix
                let wemod_src = prefix.wemod_path();
                let target_local = compat_data.join("pfx/drive_c/users/steamuser/AppData/Local");
                let _ = std::fs::create_dir_all(&target_local);
                let wemod_dir_name = wemod_src.file_name().unwrap_or_default();
                let link_path = target_local.join(wemod_dir_name);
                if !link_path.exists() {
                    let _ = std::os::unix::fs::symlink(&wemod_src, &link_path);
                }

                // Symlink WeMod roaming data
                let roaming_src = prefix.path.join("pfx/drive_c/users/steamuser/AppData/Roaming/WeMod");
                if roaming_src.exists() {
                    let target_roaming = compat_data.join("pfx/drive_c/users/steamuser/AppData/Roaming");
                    let _ = std::fs::create_dir_all(&target_roaming);
                    let roaming_link = target_roaming.join("WeMod");
                    if !roaming_link.exists() {
                        let _ = std::os::unix::fs::symlink(&roaming_src, &roaming_link);
                    }
                }

                // Create a WandaPrefix pointing to the game's compat data
                game_prefix = wanda_core::prefix::WandaPrefix {
                    name: "game".to_string(),
                    path: compat_data.clone(),
                    wemod_installed: true,
                    wemod_version: prefix.wemod_version.clone(),
                    proton_version: prefix.proton_version.clone(),
                    created_at: None,
                    last_used: None,
                };
                &game_prefix
            }
            None => {
                if args.standalone {
                    return Err(WandaError::LaunchFailed {
                        reason: format!(
                            "Game '{}' has no Proton compat data. Launch it via Steam once first.",
                            game.name
                        ),
                    });
                }
                println!("  Game has no Proton prefix yet — using WANDA's default prefix");
                prefix
            }
        };

        // Get Proton version
        let proton_manager = ProtonManager::discover(&steam, &config)?;
        let proton = if let Some(ref name) = args.proton {
            proton_manager.find_by_name(name).ok_or_else(|| {
                eprintln!("Available Proton versions:");
                for v in &proton_manager.versions {
                    eprintln!("  - {} ({})", v.name, v.compatibility);
                }
                WandaError::LaunchFailed {
                    reason: format!("Proton version '{}' not found", name),
                }
            })?
        } else {
            proton_manager.get_preferred(&config)?
        };

        println!("  Using WeMod with {}", proton.name);

        // Patch mscorlib in the prefix we're launching in so WeMod's .NET
        // version check passes. The default prefix is patched at creation, but
        // a game's own compatdata prefix (Wine Mono, reports .NET 4.0) is not —
        // without this WeMod fails its ">= 4.7" check when activating trainers.
        // Idempotent: re-patching an already-patched mscorlib is a no-op.
        let builder = PrefixBuilder::new(&launch_prefix.path, proton);
        if let Err(e) = builder.patch_mscorlib_version() {
            eprintln!("  Warning: mscorlib patch failed (WeMod may show a .NET error): {}", e);
        }

        // Create launcher
        let launcher = GameLauncher::new(&steam, launch_prefix, proton);

        let standalone = args.standalone;
        let launch_config = LaunchConfig {
            app_id: game.app_id,
            with_wemod: true,
            standalone,
            wemod_delay: args.delay,
            extra_args: args
                .args
                .map(|a| a.split_whitespace().map(String::from).collect())
                .unwrap_or_default(),
            ..Default::default()
        };

        let mut handle = launcher.launch(launch_config).await?;

        if standalone {
            println!(
                "\n{} WeMod launched in standalone mode!",
                style("SUCCESS").green().bold()
            );
            println!("\nWeMod is running in the game's prefix.");
            println!("Find your game in WeMod and click Play.");
            println!("WeMod will launch the game directly.\n");
        } else {
            println!(
                "\n{} Game launched with WeMod!",
                style("SUCCESS").green().bold()
            );
            println!("\nWeMod should appear in a separate window.");
            println!("Select your game in WeMod and click Play to activate trainers.\n");
        }

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
