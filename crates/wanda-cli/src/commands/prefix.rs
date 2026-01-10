//! wanda prefix - Manage Wine/Proton prefixes

use clap::{Args, Subcommand};
use console::style;
use std::path::PathBuf;
use wanda_core::{
    config::WandaConfig,
    prefix::{PrefixHealth, PrefixManager},
    steam::{ProtonManager, SteamInstallation},
    Result, WandaError,
};

#[derive(Subcommand)]
pub enum PrefixCommands {
    /// List all WANDA prefixes
    List,

    /// Create a new prefix
    Create(CreateArgs),

    /// Delete a prefix
    Delete(DeleteArgs),

    /// Repair a prefix
    Repair(RepairArgs),

    /// Validate prefix health
    Validate(ValidateArgs),

    /// Show prefix details
    Info(InfoArgs),
}

#[derive(Args)]
pub struct CreateArgs {
    /// Name for the new prefix
    name: String,
}

#[derive(Args)]
pub struct DeleteArgs {
    /// Name of the prefix to delete
    name: String,

    /// Skip confirmation
    #[arg(long, short)]
    force: bool,
}

#[derive(Args)]
pub struct RepairArgs {
    /// Name of the prefix to repair (default: "default")
    #[arg(default_value = "default")]
    name: String,
}

#[derive(Args)]
pub struct ValidateArgs {
    /// Name of the prefix to validate (default: "default")
    #[arg(default_value = "default")]
    name: String,
}

#[derive(Args)]
pub struct InfoArgs {
    /// Name of the prefix (default: "default")
    #[arg(default_value = "default")]
    name: String,
}

pub async fn run(cmd: PrefixCommands, config_path: Option<PathBuf>) -> Result<()> {
    let config = match &config_path {
        Some(path) => WandaConfig::load_from(path)?,
        None => WandaConfig::load()?,
    };

    let mut prefix_manager = PrefixManager::new(&config);
    prefix_manager.load()?;

    match cmd {
        PrefixCommands::List => list(&prefix_manager),
        PrefixCommands::Create(args) => create(&mut prefix_manager, &config, args).await,
        PrefixCommands::Delete(args) => delete(&mut prefix_manager, args),
        PrefixCommands::Repair(args) => repair(&mut prefix_manager, &config, args).await,
        PrefixCommands::Validate(args) => validate(&prefix_manager, args),
        PrefixCommands::Info(args) => info(&prefix_manager, args),
    }
}

fn list(manager: &PrefixManager) -> Result<()> {
    let prefixes = manager.list();

    if prefixes.is_empty() {
        println!("No prefixes found.");
        println!("Run 'wanda init' to create the default prefix.");
        return Ok(());
    }

    println!("WANDA Prefixes:\n");

    for prefix in prefixes {
        let health = manager.validate(&prefix.name)?;
        let health_indicator = match health {
            PrefixHealth::Healthy => style("HEALTHY").green(),
            PrefixHealth::NeedsRepair(_) => style("NEEDS REPAIR").yellow(),
            PrefixHealth::Corrupted(_) => style("CORRUPTED").red(),
            PrefixHealth::NotCreated => style("NOT CREATED").dim(),
        };

        let wemod_status = if prefix.wemod_installed {
            format!(
                "WeMod {}",
                prefix.wemod_version.as_deref().unwrap_or("(unknown version)")
            )
        } else {
            "WeMod not installed".to_string()
        };

        println!(
            "  {} {}",
            style(&prefix.name).bold(),
            health_indicator
        );
        println!("    Path: {}", prefix.path.display());
        println!("    {}", wemod_status);
        if let Some(ref proton) = prefix.proton_version {
            println!("    Proton: {}", proton);
        }
        println!();
    }

    Ok(())
}

async fn create(
    manager: &mut PrefixManager,
    config: &WandaConfig,
    args: CreateArgs,
) -> Result<()> {
    if manager.get(&args.name).is_some() {
        return Err(WandaError::PrefixCreationFailed {
            path: manager.base_path.join(&args.name),
            reason: "Prefix already exists".to_string(),
        });
    }

    println!("Creating prefix '{}'...", args.name);

    // Get Proton version
    let steam = SteamInstallation::discover(config)?;
    let proton_manager = ProtonManager::discover(&steam, config)?;
    let proton = proton_manager.get_preferred(config)?;

    println!("  Using Proton: {}", proton.name);
    println!("  This may take several minutes...");

    manager.create(&args.name, proton).await?;

    println!("\n{} Prefix '{}' created successfully!",
        style("SUCCESS").green().bold(),
        args.name
    );

    Ok(())
}

fn delete(manager: &mut PrefixManager, args: DeleteArgs) -> Result<()> {
    let prefix = manager.get(&args.name).ok_or_else(|| WandaError::PrefixNotFound {
        path: manager.base_path.join(&args.name),
    })?;

    if !args.force {
        println!(
            "Are you sure you want to delete prefix '{}'?",
            args.name
        );
        println!("  Path: {}", prefix.path.display());
        println!("\nThis action cannot be undone.");
        println!("Use --force to skip this confirmation.");
        return Ok(());
    }

    manager.delete(&args.name)?;
    println!("Prefix '{}' deleted.", args.name);

    Ok(())
}

async fn repair(
    manager: &mut PrefixManager,
    config: &WandaConfig,
    args: RepairArgs,
) -> Result<()> {
    let health = manager.validate(&args.name)?;

    match health {
        PrefixHealth::Healthy => {
            println!("Prefix '{}' is healthy, no repair needed.", args.name);
            return Ok(());
        }
        PrefixHealth::NotCreated => {
            println!("Prefix '{}' doesn't exist.", args.name);
            return Ok(());
        }
        PrefixHealth::Corrupted(reason) => {
            println!(
                "Prefix '{}' is corrupted: {}",
                args.name, reason
            );
            println!("Consider deleting and recreating the prefix.");
            return Err(WandaError::PrefixCorrupted {
                path: manager.base_path.join(&args.name),
                reason,
            });
        }
        PrefixHealth::NeedsRepair(issues) => {
            println!("Repairing prefix '{}'...", args.name);
            for issue in &issues {
                println!("  - {}", issue);
            }
        }
    }

    let steam = SteamInstallation::discover(config)?;
    let proton_manager = ProtonManager::discover(&steam, config)?;
    let proton = proton_manager.get_preferred(config)?;

    manager.repair(&args.name, proton).await?;

    println!("\n{} Prefix repaired!", style("SUCCESS").green().bold());

    Ok(())
}

fn validate(manager: &PrefixManager, args: ValidateArgs) -> Result<()> {
    let health = manager.validate(&args.name)?;

    println!("Prefix '{}' health check:\n", args.name);

    match health {
        PrefixHealth::Healthy => {
            println!("  {} All checks passed!", style("HEALTHY").green().bold());
        }
        PrefixHealth::NeedsRepair(issues) => {
            println!("  {} Issues found:", style("NEEDS REPAIR").yellow().bold());
            for issue in issues {
                println!("    - {}", issue);
            }
            println!("\nRun 'wanda prefix repair {}' to fix.", args.name);
        }
        PrefixHealth::Corrupted(reason) => {
            println!("  {} {}", style("CORRUPTED").red().bold(), reason);
            println!("\nConsider deleting and recreating the prefix.");
        }
        PrefixHealth::NotCreated => {
            println!("  {} Prefix has not been created yet.", style("NOT CREATED").dim());
            println!("\nRun 'wanda init' to create the default prefix.");
        }
    }

    Ok(())
}

fn info(manager: &PrefixManager, args: InfoArgs) -> Result<()> {
    let prefix = manager.get(&args.name).ok_or_else(|| WandaError::PrefixNotFound {
        path: manager.base_path.join(&args.name),
    })?;

    let health = manager.validate(&args.name)?;

    println!("Prefix: {}\n", style(&args.name).bold());
    println!("  Path:          {}", prefix.path.display());
    println!(
        "  Health:        {:?}",
        match health {
            PrefixHealth::Healthy => style("Healthy").green(),
            PrefixHealth::NeedsRepair(_) => style("Needs Repair").yellow(),
            PrefixHealth::Corrupted(_) => style("Corrupted").red(),
            PrefixHealth::NotCreated => style("Not Created").dim(),
        }
    );
    println!(
        "  WeMod:         {}",
        if prefix.wemod_installed {
            format!(
                "Installed ({})",
                prefix.wemod_version.as_deref().unwrap_or("unknown")
            )
        } else {
            "Not installed".to_string()
        }
    );
    if let Some(ref proton) = prefix.proton_version {
        println!("  Proton:        {}", proton);
    }
    if let Some(ref created) = prefix.created_at {
        println!("  Created:       {}", created);
    }

    Ok(())
}
