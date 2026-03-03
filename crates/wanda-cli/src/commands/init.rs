//! wanda init - Initialize WANDA

use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use wanda_core::{
    config::WandaConfig,
    prefix::{PrefixBuilder, PrefixManager},
    steam::{ProtonCompatibility, ProtonManager, SteamInstallation},
    wemod::{WemodDownloader, WemodInstaller},
    Result, WandaError,
};

#[derive(Args)]
pub struct InitArgs {
    /// Override Steam installation path
    #[arg(long)]
    steam_path: Option<PathBuf>,

    /// Specify Proton version to use
    #[arg(long)]
    proton: Option<String>,

    /// Skip WeMod installation
    #[arg(long)]
    skip_wemod: bool,

    /// Skip .NET Framework installation (WeMod needs it for trainers)
    #[arg(long)]
    skip_dotnet: bool,

    /// Force reinitialization even if already set up
    #[arg(long, short)]
    force: bool,
}

pub async fn run(args: InitArgs, config_path: Option<PathBuf>) -> Result<()> {
    println!("Initializing WANDA...\n");

    // Load or create config
    let mut config = match &config_path {
        Some(path) => WandaConfig::load_from(path)?,
        None => WandaConfig::load()?,
    };

    // Override steam path if provided
    if let Some(ref path) = args.steam_path {
        config.steam.install_path = Some(path.clone());
    }

    // Discover Steam
    println!("Looking for Steam installation...");
    let steam = SteamInstallation::discover(&config)?;
    println!(
        "  Found Steam at: {}{}",
        steam.root_path.display(),
        if steam.is_flatpak { " (Flatpak)" } else { "" }
    );
    println!("  Libraries: {}", steam.libraries.len());

    let total_games: usize = steam.libraries.iter().map(|l| l.apps.len()).sum();
    println!("  Games: {}", total_games);

    // Discover Proton
    println!("\nLooking for Proton versions...");
    let proton_manager = ProtonManager::discover(&steam, &config)?;

    if proton_manager.versions.is_empty() {
        return Err(WandaError::ProtonNotFound);
    }

    // Select Proton version
    // Priority: 1) --proton arg, 2) config preferred_version, 3) get_recommended()
    // Experimental/Unsupported versions from config are auto-overridden unless --proton is explicit
    let proton = if let Some(ref name) = args.proton {
        let v = proton_manager.find_by_name(name).ok_or_else(|| {
            eprintln!("Available Proton versions:");
            for v in &proton_manager.versions {
                eprintln!("  - {} ({})", v.name, v.compatibility);
            }
            WandaError::ProtonNotFound
        })?;
        if matches!(v.compatibility, ProtonCompatibility::Experimental | ProtonCompatibility::Unsupported) {
            eprintln!(
                "  Warning: '{}' is {} — this may cause WeMod to fail",
                v.name, v.compatibility
            );
        }
        v
    } else if let Some(ref preferred) = config.proton.preferred_version {
        match proton_manager.find_by_name(preferred) {
            Some(v) if matches!(v.compatibility, ProtonCompatibility::Experimental | ProtonCompatibility::Unsupported) => {
                let recommended = proton_manager.get_recommended().ok_or(WandaError::ProtonNotFound)?;
                eprintln!(
                    "  Configured Proton '{}' is {} — auto-switching to '{}' ({})",
                    v.name, v.compatibility, recommended.name, recommended.compatibility
                );
                recommended
            }
            Some(v) => v,
            None => {
                eprintln!("  Configured Proton '{}' not found, using recommended", preferred);
                proton_manager.get_recommended().ok_or(WandaError::ProtonNotFound)?
            }
        }
    } else {
        proton_manager.get_recommended().ok_or(WandaError::ProtonNotFound)?
    };

    println!("  Using: {} ({})", proton.name, proton.compatibility);

    // Set up prefix
    println!("\nSetting up Wine prefix...");
    let mut prefix_manager = PrefixManager::new(&config);
    prefix_manager.load()?;

    let existing = prefix_manager.get("default");
    if existing.is_some() && !args.force {
        println!("  Prefix already exists. Use --force to reinitialize.");
    } else {
        if existing.is_some() && args.force {
            println!("  Removing existing prefix...");
            prefix_manager.delete("default")?;
        }

        println!("  Creating prefix (this may take several minutes)...");
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        pb.set_message("Initializing Wine prefix...");
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        prefix_manager.create("default", proton).await?;

        pb.finish_with_message("Prefix created successfully");
    }

    // Install .NET Framework 4.8 (required by WeMod's trainer engine)
    if !args.skip_dotnet {
        println!("\nInstalling .NET Framework 4.8...");
        println!("  This may take 10-15 minutes. Use --skip-dotnet to skip.");

        let prefix_path = prefix_manager.base_path.join("default");
        let dotnet_builder = PrefixBuilder::new(&prefix_path, proton);

        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        pb.set_message("Installing .NET Framework 4.8 via winetricks...");
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        match dotnet_builder.install_dotnet().await {
            Ok(_) => {
                pb.finish_with_message(".NET Framework installed successfully");
            }
            Err(e) => {
                pb.finish_with_message(".NET Framework installation failed");
                eprintln!("  Warning: .NET install failed: {}", e);
                eprintln!("  WeMod may show a '.NET framework' error on startup.");
                eprintln!("  You can retry manually: winetricks -q dotnet48");
            }
        }
    }

    // Reload prefix after creation
    prefix_manager.load()?;
    let prefix = prefix_manager.get("default").unwrap();

    // Install WeMod
    if !args.skip_wemod {
        println!("\nInstalling WeMod...");

        let downloader = WemodDownloader::new(&config);

        // Get latest release info
        let release = downloader.get_latest().await?;
        if let Some(ref version) = release.version {
            println!("  Latest version: {}", version);
        }

        // Download with progress
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

        // Install
        println!("  Installing WeMod...");
        let installer = WemodInstaller::new(prefix, proton);
        installer.install(&installer_path).await?;

        // Update prefix metadata
        let version = release.version.clone();
        prefix_manager.update_metadata("default", |p| {
            p.wemod_installed = true;
            p.wemod_version = version;
        })?;

        println!("  WeMod installed successfully");
    }

    // Save config
    config.proton.preferred_version = Some(proton.name.clone());
    match &config_path {
        Some(path) => config.save_to(path)?,
        None => config.save()?,
    }

    println!("\nWANDA initialization complete!");
    println!("\nNext steps:");
    println!("  wanda scan          - View available games");
    println!("  wanda launch <game> - Launch a game with WeMod");
    println!("  wanda doctor        - Check for issues");

    Ok(())
}
