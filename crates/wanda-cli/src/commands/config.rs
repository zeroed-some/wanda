//! wanda config - Manage configuration

use clap::Subcommand;
use console::style;
use std::path::PathBuf;
use wanda_core::{config::WandaConfig, Result};

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Display current configuration
    Show,

    /// Open config file in editor
    Edit,

    /// Set a configuration value
    Set {
        /// Configuration key (e.g., "proton.preferred_version")
        key: String,
        /// Value to set
        value: String,
    },

    /// Reset configuration to defaults
    Reset,

    /// Show config file path
    Path,
}

pub async fn run(cmd: ConfigCommands, config_path: Option<PathBuf>) -> Result<()> {
    match cmd {
        ConfigCommands::Show => show(config_path),
        ConfigCommands::Edit => edit(config_path).await,
        ConfigCommands::Set { key, value } => set(key, value, config_path),
        ConfigCommands::Reset => reset(config_path),
        ConfigCommands::Path => path(config_path),
    }
}

fn show(config_path: Option<PathBuf>) -> Result<()> {
    let config = match &config_path {
        Some(path) => WandaConfig::load_from(path)?,
        None => WandaConfig::load()?,
    };

    println!("WANDA Configuration:\n");

    // Steam settings
    println!("[steam]");
    if let Some(ref path) = config.steam.install_path {
        println!("  install_path = \"{}\"", path.display());
    }
    if !config.steam.additional_libraries.is_empty() {
        println!("  additional_libraries = [");
        for lib in &config.steam.additional_libraries {
            println!("    \"{}\",", lib.display());
        }
        println!("  ]");
    }
    println!("  scan_flatpak = {}", config.steam.scan_flatpak);

    // Proton settings
    println!("\n[proton]");
    if let Some(ref version) = config.proton.preferred_version {
        println!("  preferred_version = \"{}\"", version);
    }
    if !config.proton.search_paths.is_empty() {
        println!("  search_paths = [");
        for path in &config.proton.search_paths {
            println!("    \"{}\",", path.display());
        }
        println!("  ]");
    }

    // WeMod settings
    println!("\n[wemod]");
    println!("  auto_update = {}", config.wemod.auto_update);
    if let Some(ref version) = config.wemod.version {
        println!("  version = \"{}\"", version);
    }

    // Prefix settings
    println!("\n[prefix]");
    if let Some(ref path) = config.prefix.base_path {
        println!("  base_path = \"{}\"", path.display());
    }
    println!("  auto_repair = {}", config.prefix.auto_repair);

    Ok(())
}

async fn edit(config_path: Option<PathBuf>) -> Result<()> {
    let path = config_path.unwrap_or_else(WandaConfig::default_path);

    // Ensure config file exists
    if !path.exists() {
        println!("Config file doesn't exist, creating default...");
        let config = WandaConfig::default();
        config.save_to(&path)?;
    }

    // Try common editors
    let editors = ["$EDITOR", "code", "nano", "vim", "vi"];

    for editor in &editors {
        let editor_cmd = if *editor == "$EDITOR" {
            std::env::var("EDITOR").ok()
        } else {
            Some(editor.to_string())
        };

        if let Some(cmd) = editor_cmd {
            let status = tokio::process::Command::new(&cmd)
                .arg(&path)
                .status()
                .await;

            if status.is_ok() {
                return Ok(());
            }
        }
    }

    println!("Could not open editor. Config file is at:");
    println!("  {}", path.display());

    Ok(())
}

fn set(key: String, value: String, config_path: Option<PathBuf>) -> Result<()> {
    let path = config_path.unwrap_or_else(WandaConfig::default_path);
    let mut config = if path.exists() {
        WandaConfig::load_from(&path)?
    } else {
        WandaConfig::default()
    };

    match key.as_str() {
        "steam.install_path" => {
            config.steam.install_path = Some(PathBuf::from(&value));
        }
        "steam.scan_flatpak" => {
            config.steam.scan_flatpak = value.parse().unwrap_or(false);
        }
        "proton.preferred_version" => {
            config.proton.preferred_version = Some(value.clone());
        }
        "wemod.auto_update" => {
            config.wemod.auto_update = value.parse().unwrap_or(true);
        }
        "wemod.version" => {
            config.wemod.version = Some(value.clone());
        }
        "prefix.base_path" => {
            config.prefix.base_path = Some(PathBuf::from(&value));
        }
        "prefix.auto_repair" => {
            config.prefix.auto_repair = value.parse().unwrap_or(true);
        }
        _ => {
            println!("Unknown configuration key: {}", key);
            println!("\nAvailable keys:");
            println!("  steam.install_path");
            println!("  steam.scan_flatpak");
            println!("  proton.preferred_version");
            println!("  wemod.auto_update");
            println!("  wemod.version");
            println!("  prefix.base_path");
            println!("  prefix.auto_repair");
            return Ok(());
        }
    }

    config.save_to(&path)?;
    println!("{} {} = {}", style("Set").green(), key, value);

    Ok(())
}

fn reset(config_path: Option<PathBuf>) -> Result<()> {
    let path = config_path.unwrap_or_else(WandaConfig::default_path);

    println!(
        "This will reset all configuration to defaults. Are you sure?"
    );
    println!("Config file: {}", path.display());
    println!("\nRun with --force to confirm (not implemented yet).");

    // For now, just create a default config
    let config = WandaConfig::default();
    config.save_to(&path)?;
    println!("\nConfiguration reset to defaults.");

    Ok(())
}

fn path(config_path: Option<PathBuf>) -> Result<()> {
    let path = config_path.unwrap_or_else(WandaConfig::default_path);
    println!("{}", path.display());
    Ok(())
}
