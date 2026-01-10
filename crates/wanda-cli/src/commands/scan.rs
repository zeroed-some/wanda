//! wanda scan - Scan for Steam games

use clap::Args;
use console::style;
use std::path::PathBuf;
use wanda_core::{config::WandaConfig, steam::SteamInstallation, Result};

#[derive(Args)]
pub struct ScanArgs {
    /// Filter games by name
    #[arg(short, long)]
    filter: Option<String>,

    /// Show all games, including non-Proton
    #[arg(long)]
    all: bool,

    /// Show detailed information
    #[arg(short, long)]
    long: bool,
}

pub async fn run(args: ScanArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = match &config_path {
        Some(path) => WandaConfig::load_from(path)?,
        None => WandaConfig::load()?,
    };

    let steam = SteamInstallation::discover(&config)?;
    let mut games: Vec<_> = steam.get_all_games();

    // Filter by name if specified
    if let Some(ref filter) = args.filter {
        let filter_lower = filter.to_lowercase();
        games.retain(|g| g.name.to_lowercase().contains(&filter_lower));
    }

    // Filter to Proton games unless --all
    if !args.all {
        games.retain(|g| g.uses_proton());
    }

    // Sort by name
    games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    if games.is_empty() {
        println!("No games found.");
        if !args.all {
            println!("(Use --all to show non-Proton games)");
        }
        return Ok(());
    }

    println!(
        "Found {} games{}:\n",
        games.len(),
        if !args.all {
            " (Proton/Windows)"
        } else {
            ""
        }
    );

    if args.long {
        // Detailed view
        for game in &games {
            let proton_indicator = if game.uses_proton() {
                style("PROTON").green()
            } else {
                style("NATIVE").blue()
            };

            println!(
                "{} {} {}",
                style(format!("[{}]", game.app_id)).dim(),
                style(&game.name).bold(),
                proton_indicator
            );
            println!("    Path: {}", game.install_path.display());
            println!("    Size: {}", game.size_human());
            if let Some(ref compat) = game.compat_data_path {
                println!("    Prefix: {}", compat.display());
            }
            println!();
        }
    } else {
        // Compact view
        let max_name_len = games
            .iter()
            .map(|g| g.name.len())
            .max()
            .unwrap_or(20)
            .min(50);

        println!(
            "{:>8}  {:<width$}  {:>10}  {}",
            "App ID",
            "Name",
            "Size",
            "Type",
            width = max_name_len
        );
        println!("{}", "-".repeat(8 + 2 + max_name_len + 2 + 10 + 2 + 8));

        for game in &games {
            let name = if game.name.len() > max_name_len {
                format!("{}...", &game.name[..max_name_len - 3])
            } else {
                game.name.clone()
            };

            let game_type = if game.uses_proton() {
                style("Proton").green()
            } else {
                style("Native").blue()
            };

            println!(
                "{:>8}  {:<width$}  {:>10}  {}",
                game.app_id,
                name,
                game.size_human(),
                game_type,
                width = max_name_len
            );
        }
    }

    println!();
    println!(
        "Launch a game with: {} <name or app_id>",
        style("wanda launch").cyan()
    );

    Ok(())
}
