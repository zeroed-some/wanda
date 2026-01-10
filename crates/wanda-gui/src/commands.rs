//! Tauri IPC commands

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;
use wanda_core::{
    config::WandaConfig,
    launcher::{GameLauncher, LaunchConfig},
    prefix::{PrefixHealth, PrefixIssue, PrefixManager},
    steam::{ProtonCompatibility, ProtonManager, SteamInstallation},
    wemod::{WemodDownloader, WemodInstaller},
};

// ============================================================================
// Data Transfer Objects
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct GameInfo {
    pub app_id: u32,
    pub name: String,
    pub size: String,
    pub uses_proton: bool,
    pub install_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PrefixInfo {
    pub name: String,
    pub path: String,
    pub wemod_installed: bool,
    pub wemod_version: Option<String>,
    pub proton_version: Option<String>,
    pub health: String,
    pub issues: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProtonInfo {
    pub name: String,
    pub path: String,
    pub compatibility: String,
    pub is_ge: bool,
    pub is_recommended: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WemodStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub update_available: bool,
    pub latest_version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InitStatus {
    pub initialized: bool,
    pub steam_found: bool,
    pub proton_found: bool,
    pub prefix_exists: bool,
    pub wemod_installed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DoctorReport {
    pub steam_ok: bool,
    pub steam_path: Option<String>,
    pub proton_ok: bool,
    pub proton_count: usize,
    pub prefix_ok: bool,
    pub wemod_ok: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigDto {
    pub steam_path: Option<String>,
    pub scan_flatpak: bool,
    pub preferred_proton: Option<String>,
    pub auto_update_wemod: bool,
}

// ============================================================================
// Game Commands
// ============================================================================

#[tauri::command]
pub async fn get_games(state: State<'_, Arc<Mutex<AppState>>>) -> Result<Vec<GameInfo>, String> {
    let mut state = state.lock().await;
    state.ensure_loaded()?;

    let steam = state.steam.as_ref().ok_or("Steam not loaded")?;

    let games: Vec<GameInfo> = steam
        .get_all_games()
        .iter()
        .filter(|g| g.uses_proton())
        .map(|g| GameInfo {
            app_id: g.app_id,
            name: g.name.clone(),
            size: g.size_human(),
            uses_proton: g.uses_proton(),
            install_path: g.install_path.to_string_lossy().to_string(),
        })
        .collect();

    Ok(games)
}

#[tauri::command]
pub async fn get_game(
    app_id: u32,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<GameInfo, String> {
    let mut state = state.lock().await;
    state.ensure_loaded()?;

    let steam = state.steam.as_ref().ok_or("Steam not loaded")?;
    let game = steam
        .find_game(app_id)
        .ok_or_else(|| format!("Game {} not found", app_id))?;

    Ok(GameInfo {
        app_id: game.app_id,
        name: game.name.clone(),
        size: game.size_human(),
        uses_proton: game.uses_proton(),
        install_path: game.install_path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub async fn launch_game(
    app_id: u32,
    with_wemod: bool,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state = state.lock().await;
    state.ensure_loaded()?;

    let steam = state.steam.as_ref().ok_or("Steam not loaded")?;
    let config = state.config.as_ref().ok_or("Config not loaded")?;
    let proton_mgr = state.proton.as_ref().ok_or("Proton not loaded")?;
    let prefix_mgr = state.prefix_manager.as_ref().ok_or("Prefix manager not loaded")?;

    let proton = proton_mgr
        .get_preferred(config)
        .map_err(|e| e.to_string())?;

    let prefix = prefix_mgr
        .get("default")
        .ok_or("WANDA not initialized")?;

    let launcher = GameLauncher::new(steam, prefix, proton);

    let launch_config = LaunchConfig {
        app_id,
        with_wemod,
        wemod_delay: 3,
        ..Default::default()
    };

    launcher.launch(launch_config).await.map_err(|e| e.to_string())?;

    Ok(())
}

// ============================================================================
// Prefix Commands
// ============================================================================

#[tauri::command]
pub async fn get_prefixes(state: State<'_, Arc<Mutex<AppState>>>) -> Result<Vec<PrefixInfo>, String> {
    let mut state = state.lock().await;
    state.ensure_loaded()?;

    let prefix_mgr = state.prefix_manager.as_ref().ok_or("Prefix manager not loaded")?;

    let prefixes: Vec<PrefixInfo> = prefix_mgr
        .list()
        .iter()
        .map(|p| {
            let health = prefix_mgr.validate(&p.name).unwrap_or(PrefixHealth::NotCreated);
            let (health_str, issues) = match &health {
                PrefixHealth::Healthy => ("healthy".to_string(), vec![]),
                PrefixHealth::NeedsRepair(issues) => (
                    "needs_repair".to_string(),
                    issues.iter().map(|i| i.to_string()).collect(),
                ),
                PrefixHealth::Corrupted(reason) => ("corrupted".to_string(), vec![reason.clone()]),
                PrefixHealth::NotCreated => ("not_created".to_string(), vec![]),
            };

            PrefixInfo {
                name: p.name.clone(),
                path: p.path.to_string_lossy().to_string(),
                wemod_installed: p.wemod_installed,
                wemod_version: p.wemod_version.clone(),
                proton_version: p.proton_version.clone(),
                health: health_str,
                issues,
            }
        })
        .collect();

    Ok(prefixes)
}

#[tauri::command]
pub async fn get_prefix_health(
    name: String,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<PrefixInfo, String> {
    let mut state = state.lock().await;
    state.ensure_loaded()?;

    let prefix_mgr = state.prefix_manager.as_ref().ok_or("Prefix manager not loaded")?;

    let prefix = prefix_mgr.get(&name).ok_or("Prefix not found")?;
    let health = prefix_mgr.validate(&name).map_err(|e| e.to_string())?;

    let (health_str, issues) = match &health {
        PrefixHealth::Healthy => ("healthy".to_string(), vec![]),
        PrefixHealth::NeedsRepair(issues) => (
            "needs_repair".to_string(),
            issues.iter().map(|i| i.to_string()).collect(),
        ),
        PrefixHealth::Corrupted(reason) => ("corrupted".to_string(), vec![reason.clone()]),
        PrefixHealth::NotCreated => ("not_created".to_string(), vec![]),
    };

    Ok(PrefixInfo {
        name: prefix.name.clone(),
        path: prefix.path.to_string_lossy().to_string(),
        wemod_installed: prefix.wemod_installed,
        wemod_version: prefix.wemod_version.clone(),
        proton_version: prefix.proton_version.clone(),
        health: health_str,
        issues,
    })
}

#[tauri::command]
pub async fn repair_prefix(
    name: String,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state = state.lock().await;
    state.ensure_loaded()?;

    let config = state.config.as_ref().ok_or("Config not loaded")?;
    let proton_mgr = state.proton.as_ref().ok_or("Proton not loaded")?;
    let prefix_mgr = state.prefix_manager.as_mut().ok_or("Prefix manager not loaded")?;

    let proton = proton_mgr
        .get_preferred(config)
        .map_err(|e| e.to_string())?;

    prefix_mgr.repair(&name, proton).await.map_err(|e| e.to_string())?;

    Ok(())
}

// ============================================================================
// Initialization Commands
// ============================================================================

#[tauri::command]
pub async fn get_init_status(state: State<'_, Arc<Mutex<AppState>>>) -> Result<InitStatus, String> {
    let mut state = state.lock().await;

    // Try to load, but don't fail if we can't
    let _ = state.load();

    let steam_found = state.steam.is_some();
    let proton_found = state.proton.as_ref().map(|p| !p.versions.is_empty()).unwrap_or(false);
    let prefix_exists = state
        .prefix_manager
        .as_ref()
        .map(|pm| pm.get("default").is_some())
        .unwrap_or(false);
    let wemod_installed = state
        .prefix_manager
        .as_ref()
        .and_then(|pm| pm.get("default"))
        .map(|p| p.wemod_installed)
        .unwrap_or(false);

    Ok(InitStatus {
        initialized: wemod_installed,
        steam_found,
        proton_found,
        prefix_exists,
        wemod_installed,
    })
}

#[tauri::command]
pub async fn init_wanda(state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let mut state = state.lock().await;

    // Load config
    let config = WandaConfig::load().map_err(|e| e.to_string())?;

    // Discover Steam
    let steam = SteamInstallation::discover(&config).map_err(|e| e.to_string())?;

    // Discover Proton
    let proton_mgr = ProtonManager::discover(&steam, &config).map_err(|e| e.to_string())?;
    let proton = proton_mgr.get_preferred(&config).map_err(|e| e.to_string())?;

    // Create prefix manager and default prefix
    let mut prefix_mgr = PrefixManager::new(&config);
    prefix_mgr.load().map_err(|e| e.to_string())?;

    if prefix_mgr.get("default").is_none() {
        prefix_mgr.create("default", proton).await.map_err(|e| e.to_string())?;
    }

    // Download and install WeMod
    let downloader = WemodDownloader::new(&config);
    let release = downloader.get_latest().await.map_err(|e| e.to_string())?;
    let installer_path = downloader
        .download(&release, |_, _| {})
        .await
        .map_err(|e| e.to_string())?;

    let prefix = prefix_mgr.get("default").ok_or("Prefix not created")?;
    let installer = WemodInstaller::new(prefix, proton);
    installer.install(&installer_path).await.map_err(|e| e.to_string())?;

    // Update prefix metadata
    let version = release.version.clone();
    prefix_mgr.update_metadata("default", |p| {
        p.wemod_installed = true;
        p.wemod_version = version;
    }).map_err(|e| e.to_string())?;

    // Save config
    let mut config = config;
    config.proton.preferred_version = Some(proton.name.clone());
    config.save().map_err(|e| e.to_string())?;

    // Reload state
    state.load()?;

    Ok(())
}

// ============================================================================
// Config Commands
// ============================================================================

#[tauri::command]
pub async fn get_config(state: State<'_, Arc<Mutex<AppState>>>) -> Result<ConfigDto, String> {
    let mut state = state.lock().await;
    state.ensure_loaded()?;

    let config = state.config.as_ref().ok_or("Config not loaded")?;

    Ok(ConfigDto {
        steam_path: config.steam.install_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        scan_flatpak: config.steam.scan_flatpak,
        preferred_proton: config.proton.preferred_version.clone(),
        auto_update_wemod: config.wemod.auto_update,
    })
}

#[tauri::command]
pub async fn update_config(
    config_dto: ConfigDto,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state = state.lock().await;
    state.ensure_loaded()?;

    let config = state.config.as_mut().ok_or("Config not loaded")?;

    config.steam.install_path = config_dto.steam_path.map(std::path::PathBuf::from);
    config.steam.scan_flatpak = config_dto.scan_flatpak;
    config.proton.preferred_version = config_dto.preferred_proton;
    config.wemod.auto_update = config_dto.auto_update_wemod;

    config.save().map_err(|e| e.to_string())?;

    // Reload state to reflect changes
    state.load()?;

    Ok(())
}

// ============================================================================
// WeMod Commands
// ============================================================================

#[tauri::command]
pub async fn get_wemod_status(state: State<'_, Arc<Mutex<AppState>>>) -> Result<WemodStatus, String> {
    let mut state = state.lock().await;
    state.ensure_loaded()?;

    let config = state.config.as_ref().ok_or("Config not loaded")?;
    let prefix_mgr = state.prefix_manager.as_ref().ok_or("Prefix manager not loaded")?;

    let prefix = prefix_mgr.get("default");
    let installed = prefix.map(|p| p.wemod_installed).unwrap_or(false);
    let version = prefix.and_then(|p| p.wemod_version.clone());

    // Check for updates
    let downloader = WemodDownloader::new(config);
    let (update_available, latest_version) = match downloader.get_latest().await {
        Ok(release) => {
            let latest = release.version.clone();
            let update = match (&version, &latest) {
                (Some(current), Some(new)) => current != new,
                _ => false,
            };
            (update, latest)
        }
        Err(_) => (false, None),
    };

    Ok(WemodStatus {
        installed,
        version,
        update_available,
        latest_version,
    })
}

#[tauri::command]
pub async fn update_wemod(state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let mut state = state.lock().await;
    state.ensure_loaded()?;

    let config = state.config.as_ref().ok_or("Config not loaded")?;
    let proton_mgr = state.proton.as_ref().ok_or("Proton not loaded")?;
    let prefix_mgr = state.prefix_manager.as_mut().ok_or("Prefix manager not loaded")?;

    let proton = proton_mgr.get_preferred(config).map_err(|e| e.to_string())?;
    let prefix = prefix_mgr.get("default").ok_or("WANDA not initialized")?;

    let downloader = WemodDownloader::new(config);
    let release = downloader.get_latest().await.map_err(|e| e.to_string())?;
    let installer_path = downloader
        .download(&release, |_, _| {})
        .await
        .map_err(|e| e.to_string())?;

    let installer = WemodInstaller::new(prefix, proton);
    installer.install(&installer_path).await.map_err(|e| e.to_string())?;

    let version = release.version.clone();
    prefix_mgr.update_metadata("default", |p| {
        p.wemod_version = version;
    }).map_err(|e| e.to_string())?;

    Ok(())
}

// ============================================================================
// Proton Commands
// ============================================================================

#[tauri::command]
pub async fn get_proton_versions(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<ProtonInfo>, String> {
    let mut state = state.lock().await;
    state.ensure_loaded()?;

    let config = state.config.as_ref().ok_or("Config not loaded")?;
    let proton_mgr = state.proton.as_ref().ok_or("Proton not loaded")?;

    let recommended = proton_mgr.get_preferred(config).ok();

    let versions: Vec<ProtonInfo> = proton_mgr
        .versions
        .iter()
        .map(|v| ProtonInfo {
            name: v.name.clone(),
            path: v.path.to_string_lossy().to_string(),
            compatibility: match v.compatibility {
                ProtonCompatibility::Recommended => "recommended".to_string(),
                ProtonCompatibility::Supported => "supported".to_string(),
                ProtonCompatibility::Experimental => "experimental".to_string(),
                ProtonCompatibility::Unsupported => "unsupported".to_string(),
            },
            is_ge: v.is_ge,
            is_recommended: recommended.map(|r| r.name == v.name).unwrap_or(false),
        })
        .collect();

    Ok(versions)
}

// ============================================================================
// Doctor Commands
// ============================================================================

#[tauri::command]
pub async fn run_doctor(state: State<'_, Arc<Mutex<AppState>>>) -> Result<DoctorReport, String> {
    let mut state = state.lock().await;

    let mut issues = Vec::new();

    // Try to load config
    let config = match WandaConfig::load() {
        Ok(c) => Some(c),
        Err(e) => {
            issues.push(format!("Config error: {}", e));
            None
        }
    };

    // Check Steam
    let (steam_ok, steam_path) = if let Some(ref cfg) = config {
        match SteamInstallation::discover(cfg) {
            Ok(steam) => (true, Some(steam.root_path.to_string_lossy().to_string())),
            Err(e) => {
                issues.push(format!("Steam not found: {}", e));
                (false, None)
            }
        }
    } else {
        (false, None)
    };

    // Check Proton
    let (proton_ok, proton_count) = if let (Some(ref cfg), true) = (&config, steam_ok) {
        let steam = SteamInstallation::discover(cfg).unwrap();
        match ProtonManager::discover(&steam, cfg) {
            Ok(pm) if !pm.versions.is_empty() => (true, pm.versions.len()),
            Ok(_) => {
                issues.push("No Proton versions found".to_string());
                (false, 0)
            }
            Err(e) => {
                issues.push(format!("Proton error: {}", e));
                (false, 0)
            }
        }
    } else {
        (false, 0)
    };

    // Check prefix
    let (prefix_ok, wemod_ok) = if let Some(ref cfg) = config {
        let mut pm = PrefixManager::new(cfg);
        let _ = pm.load();
        if let Some(prefix) = pm.get("default") {
            let health_ok = matches!(pm.validate("default"), Ok(PrefixHealth::Healthy));
            if !health_ok {
                issues.push("Prefix needs repair".to_string());
            }
            (true, prefix.wemod_installed)
        } else {
            issues.push("WANDA not initialized".to_string());
            (false, false)
        }
    } else {
        (false, false)
    };

    if !wemod_ok && prefix_ok {
        issues.push("WeMod not installed".to_string());
    }

    // Update state if load was successful
    if config.is_some() {
        let _ = state.load();
    }

    Ok(DoctorReport {
        steam_ok,
        steam_path,
        proton_ok,
        proton_count,
        prefix_ok,
        wemod_ok,
        issues,
    })
}
