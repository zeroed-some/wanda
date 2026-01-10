//! wanda wemod - Manage WeMod installation

use clap::Subcommand;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use wanda_core::{
    config::WandaConfig,
    prefix::PrefixManager,
    steam::{ProtonManager, SteamInstallation},
    wemod::{WemodDownloader, WemodInstaller},
    Result, WandaError,
};

#[derive(Subcommand)]
pub enum WemodCommands {
    /// Show WeMod installation status
    Status,

    /// Update WeMod to the latest version
    Update,

    /// Install WeMod into a prefix
    Install,

    /// Uninstall WeMod from a prefix
    Uninstall,

    /// Run WeMod standalone (for testing)
    Run,
}

pub async fn run(cmd: WemodCommands, config_path: Option<PathBuf>) -> Result<()> {
    let config = match &config_path {
        Some(path) => WandaConfig::load_from(path)?,
        None => WandaConfig::load()?,
    };

    match cmd {
        WemodCommands::Status => status(&config).await,
        WemodCommands::Update => update(&config).await,
        WemodCommands::Install => install(&config).await,
        WemodCommands::Uninstall => uninstall(&config).await,
        WemodCommands::Run => run_wemod(&config).await,
    }
}

async fn status(config: &WandaConfig) -> Result<()> {
    let mut prefix_manager = PrefixManager::new(config);
    prefix_manager.load()?;

    let prefix = prefix_manager.get("default");

    println!("WeMod Status:\n");

    match prefix {
        Some(p) if p.wemod_installed => {
            println!(
                "  Status:  {} Installed",
                style("OK").green().bold()
            );
            if let Some(ref version) = p.wemod_version {
                println!("  Version: {}", version);
            }
            println!("  Path:    {}", p.wemod_path().display());

            // Check for updates
            println!("\nChecking for updates...");
            let downloader = WemodDownloader::new(config);
            match downloader.get_latest().await {
                Ok(release) => {
                    if let Some(ref latest) = release.version {
                        let current = p.wemod_version.as_deref().unwrap_or("unknown");
                        if current != latest {
                            println!(
                                "  {} Update available: {} -> {}",
                                style("UPDATE").yellow(),
                                current,
                                latest
                            );
                            println!("\n  Run 'wanda wemod update' to update.");
                        } else {
                            println!("  {} Up to date", style("OK").green());
                        }
                    }
                }
                Err(e) => {
                    println!("  Could not check for updates: {}", e);
                }
            }
        }
        Some(_) => {
            println!(
                "  Status: {} Not installed",
                style("WARNING").yellow().bold()
            );
            println!("\n  Run 'wanda wemod install' to install WeMod.");
        }
        None => {
            println!(
                "  Status: {} WANDA not initialized",
                style("ERROR").red().bold()
            );
            println!("\n  Run 'wanda init' first.");
        }
    }

    Ok(())
}

async fn update(config: &WandaConfig) -> Result<()> {
    let mut prefix_manager = PrefixManager::new(config);
    prefix_manager.load()?;

    let prefix = prefix_manager.get("default").ok_or_else(|| WandaError::LaunchFailed {
        reason: "WANDA not initialized. Run 'wanda init' first.".to_string(),
    })?;

    println!("Checking for WeMod updates...\n");

    let downloader = WemodDownloader::new(config);
    let release = downloader.get_latest().await?;

    let current = prefix.wemod_version.as_deref().unwrap_or("unknown");
    let latest = release.version.as_deref().unwrap_or("unknown");

    if current == latest {
        println!("WeMod is already up to date (version {}).", current);
        return Ok(());
    }

    println!("Current version: {}", current);
    println!("Latest version:  {}", latest);
    println!("\nDownloading update...");

    let pb = ProgressBar::new(release.size.unwrap_or(0));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    let installer_path = downloader
        .download(&release, |downloaded, total| {
            pb.set_length(total);
            pb.set_position(downloaded);
        })
        .await?;

    pb.finish_with_message("Download complete");

    println!("\nInstalling update...");

    let steam = SteamInstallation::discover(config)?;
    let proton_manager = ProtonManager::discover(&steam, config)?;
    let proton = proton_manager.get_preferred(config)?;

    let installer = WemodInstaller::new(prefix, proton);
    installer.install(&installer_path).await?;

    // Update metadata
    let version = release.version.clone();
    prefix_manager.update_metadata("default", |p| {
        p.wemod_version = version;
    })?;

    println!(
        "\n{} WeMod updated to version {}!",
        style("SUCCESS").green().bold(),
        latest
    );

    Ok(())
}

async fn install(config: &WandaConfig) -> Result<()> {
    let mut prefix_manager = PrefixManager::new(config);
    prefix_manager.load()?;

    let prefix = prefix_manager.get("default").ok_or_else(|| WandaError::LaunchFailed {
        reason: "WANDA not initialized. Run 'wanda init' first.".to_string(),
    })?;

    if prefix.wemod_installed {
        println!("WeMod is already installed.");
        println!("Use 'wanda wemod update' to update to the latest version.");
        return Ok(());
    }

    println!("Installing WeMod...\n");

    let downloader = WemodDownloader::new(config);
    let release = downloader.get_latest().await?;

    if let Some(ref version) = release.version {
        println!("Version: {}", version);
    }

    let pb = ProgressBar::new(release.size.unwrap_or(0));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    let installer_path = downloader
        .download(&release, |downloaded, total| {
            pb.set_length(total);
            pb.set_position(downloaded);
        })
        .await?;

    pb.finish_with_message("Download complete");

    println!("\nInstalling...");

    let steam = SteamInstallation::discover(config)?;
    let proton_manager = ProtonManager::discover(&steam, config)?;
    let proton = proton_manager.get_preferred(config)?;

    let installer = WemodInstaller::new(prefix, proton);
    installer.install(&installer_path).await?;

    let version = release.version.clone();
    prefix_manager.update_metadata("default", |p| {
        p.wemod_installed = true;
        p.wemod_version = version;
    })?;

    println!(
        "\n{} WeMod installed successfully!",
        style("SUCCESS").green().bold()
    );

    Ok(())
}

async fn uninstall(config: &WandaConfig) -> Result<()> {
    let mut prefix_manager = PrefixManager::new(config);
    prefix_manager.load()?;

    let prefix = prefix_manager.get("default").ok_or_else(|| WandaError::LaunchFailed {
        reason: "WANDA not initialized.".to_string(),
    })?;

    if !prefix.wemod_installed {
        println!("WeMod is not installed.");
        return Ok(());
    }

    let steam = SteamInstallation::discover(config)?;
    let proton_manager = ProtonManager::discover(&steam, config)?;
    let proton = proton_manager.get_preferred(config)?;

    println!("Uninstalling WeMod...");

    let installer = WemodInstaller::new(prefix, proton);
    installer.uninstall().await?;

    prefix_manager.update_metadata("default", |p| {
        p.wemod_installed = false;
        p.wemod_version = None;
    })?;

    println!("WeMod uninstalled.");

    Ok(())
}

async fn run_wemod(config: &WandaConfig) -> Result<()> {
    let mut prefix_manager = PrefixManager::new(config);
    prefix_manager.load()?;

    let prefix = prefix_manager.get("default").ok_or(WandaError::WemodNotInstalled)?;

    if !prefix.wemod_installed {
        return Err(WandaError::WemodNotInstalled);
    }

    let steam = SteamInstallation::discover(config)?;
    let proton_manager = ProtonManager::discover(&steam, config)?;
    let proton = proton_manager.get_preferred(config)?;

    println!("Starting WeMod...");

    let installer = WemodInstaller::new(prefix, proton);
    let mut child = installer.run().await?;

    println!("WeMod is running. Press Ctrl+C to stop.");

    // Wait for WeMod to exit
    let _ = child.wait().await;

    println!("WeMod closed.");

    Ok(())
}
