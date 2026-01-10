//! Steam library and game discovery

use crate::config::WandaConfig;
use crate::error::{Result, WandaError};
use crate::steam::vdf::parse_vdf_file;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// A Steam installation with all its libraries
#[derive(Debug, Clone)]
pub struct SteamInstallation {
    /// Root Steam installation path
    pub root_path: PathBuf,
    /// All discovered library folders
    pub libraries: Vec<SteamLibrary>,
    /// Whether this is a Flatpak installation
    pub is_flatpak: bool,
}

/// A Steam library folder containing games
#[derive(Debug, Clone)]
pub struct SteamLibrary {
    /// Path to the library folder
    pub path: PathBuf,
    /// Optional label for this library
    pub label: Option<String>,
    /// Installed apps in this library (app_id -> SteamApp)
    pub apps: HashMap<u32, SteamApp>,
}

/// A Steam game/application
#[derive(Debug, Clone)]
pub struct SteamApp {
    /// Steam App ID
    pub app_id: u32,
    /// Game name
    pub name: String,
    /// Installation directory name (relative to steamapps/common/)
    pub install_dir: String,
    /// Full path to the game installation
    pub install_path: PathBuf,
    /// Library this game belongs to
    pub library_path: PathBuf,
    /// Size on disk in bytes
    pub size_on_disk: u64,
    /// Path to the Proton compatdata directory (if using Proton)
    pub compat_data_path: Option<PathBuf>,
}

impl SteamInstallation {
    /// Discover Steam installation on the system
    pub fn discover(config: &WandaConfig) -> Result<Self> {
        // Check for user-specified path first
        if let Some(ref path) = config.steam.install_path {
            if Self::is_valid_steam_path(path) {
                info!("Using configured Steam path: {}", path.display());
                return Self::from_path(path.clone(), false);
            }
            warn!(
                "Configured Steam path {} is invalid, falling back to auto-detection",
                path.display()
            );
        }

        // Try standard locations
        let candidates = Self::get_candidate_paths(config.steam.scan_flatpak);

        for (path, is_flatpak) in candidates {
            if Self::is_valid_steam_path(&path) {
                info!("Found Steam installation at: {}", path.display());
                return Self::from_path(path, is_flatpak);
            }
        }

        Err(WandaError::SteamNotFound)
    }

    /// Get candidate Steam installation paths
    fn get_candidate_paths(scan_flatpak: bool) -> Vec<(PathBuf, bool)> {
        let mut candidates = Vec::new();

        // Standard native Steam locations
        if let Some(data_dir) = dirs::data_local_dir() {
            candidates.push((data_dir.join("Steam"), false));
        }
        if let Some(home) = dirs::home_dir() {
            candidates.push((home.join(".steam/steam"), false));
            candidates.push((home.join(".steam/root"), false));
        }

        // Flatpak Steam
        if scan_flatpak {
            if let Some(home) = dirs::home_dir() {
                candidates.push((
                    home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
                    true,
                ));
            }
        }

        candidates
    }

    /// Check if a path is a valid Steam installation
    fn is_valid_steam_path(path: &Path) -> bool {
        // Steam should have a config directory with libraryfolders.vdf
        let config_vdf = path.join("config/libraryfolders.vdf");
        let steamapps_vdf = path.join("steamapps/libraryfolders.vdf");

        config_vdf.exists() || steamapps_vdf.exists()
    }

    /// Create SteamInstallation from a known valid path
    fn from_path(root_path: PathBuf, is_flatpak: bool) -> Result<Self> {
        let mut installation = Self {
            root_path: root_path.clone(),
            libraries: Vec::new(),
            is_flatpak,
        };

        // Parse library folders
        let library_folders_path = if root_path.join("config/libraryfolders.vdf").exists() {
            root_path.join("config/libraryfolders.vdf")
        } else {
            root_path.join("steamapps/libraryfolders.vdf")
        };

        installation.parse_library_folders(&library_folders_path)?;

        Ok(installation)
    }

    /// Parse libraryfolders.vdf to discover all library locations
    fn parse_library_folders(&mut self, vdf_path: &Path) -> Result<()> {
        let vdf = parse_vdf_file(vdf_path)?;

        let folders = vdf
            .get("libraryfolders")
            .ok_or_else(|| WandaError::VdfParseError {
                path: vdf_path.to_path_buf(),
                reason: "Missing 'libraryfolders' key".to_string(),
            })?;

        let folders_obj = folders.as_object().ok_or_else(|| WandaError::VdfParseError {
            path: vdf_path.to_path_buf(),
            reason: "'libraryfolders' is not an object".to_string(),
        })?;

        for (key, value) in folders_obj {
            // Library entries are numbered: "0", "1", "2", etc.
            if key.parse::<u32>().is_err() {
                continue;
            }

            if let Some(path_str) = value.get_str("path") {
                let library_path = PathBuf::from(path_str);
                let label = value.get_str("label").map(String::from);

                debug!("Found Steam library: {}", library_path.display());

                let mut library = SteamLibrary {
                    path: library_path.clone(),
                    label,
                    apps: HashMap::new(),
                };

                // Scan for installed apps
                library.scan_apps()?;
                self.libraries.push(library);
            }
        }

        info!("Discovered {} Steam libraries", self.libraries.len());
        Ok(())
    }

    /// Get all installed games across all libraries
    pub fn get_all_games(&self) -> Vec<&SteamApp> {
        self.libraries
            .iter()
            .flat_map(|lib| lib.apps.values())
            .collect()
    }

    /// Find a game by app ID
    pub fn find_game(&self, app_id: u32) -> Option<&SteamApp> {
        for library in &self.libraries {
            if let Some(app) = library.apps.get(&app_id) {
                return Some(app);
            }
        }
        None
    }

    /// Find a game by name (case-insensitive partial match)
    pub fn find_game_by_name(&self, name: &str) -> Vec<&SteamApp> {
        let name_lower = name.to_lowercase();
        self.get_all_games()
            .into_iter()
            .filter(|app| app.name.to_lowercase().contains(&name_lower))
            .collect()
    }
}

impl SteamLibrary {
    /// Scan for installed apps in this library
    fn scan_apps(&mut self) -> Result<()> {
        let steamapps_dir = self.path.join("steamapps");
        if !steamapps_dir.exists() {
            return Ok(());
        }

        // Look for appmanifest_*.acf files
        let entries = std::fs::read_dir(&steamapps_dir).map_err(|e| {
            WandaError::SteamLibraryInaccessible {
                path: self.path.clone(),
                reason: e.to_string(),
            }
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            if filename.starts_with("appmanifest_") && filename.ends_with(".acf") {
                match self.parse_app_manifest(&path) {
                    Ok(app) => {
                        debug!("Found game: {} ({})", app.name, app.app_id);
                        self.apps.insert(app.app_id, app);
                    }
                    Err(e) => {
                        warn!("Failed to parse {}: {}", path.display(), e);
                    }
                }
            }
        }

        info!(
            "Found {} games in library {}",
            self.apps.len(),
            self.path.display()
        );
        Ok(())
    }

    /// Parse an appmanifest_*.acf file
    fn parse_app_manifest(&self, acf_path: &Path) -> Result<SteamApp> {
        let vdf = parse_vdf_file(acf_path)?;

        let app_state = vdf.get("AppState").ok_or_else(|| WandaError::VdfParseError {
            path: acf_path.to_path_buf(),
            reason: "Missing 'AppState' key".to_string(),
        })?;

        let app_id: u32 = app_state
            .get_str("appid")
            .ok_or_else(|| WandaError::VdfParseError {
                path: acf_path.to_path_buf(),
                reason: "Missing 'appid'".to_string(),
            })?
            .parse()
            .map_err(|_| WandaError::VdfParseError {
                path: acf_path.to_path_buf(),
                reason: "Invalid 'appid'".to_string(),
            })?;

        let name = app_state
            .get_str("name")
            .unwrap_or("Unknown")
            .to_string();

        let install_dir = app_state
            .get_str("installdir")
            .unwrap_or("")
            .to_string();

        let size_on_disk: u64 = app_state
            .get_str("SizeOnDisk")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let install_path = self.path.join("steamapps/common").join(&install_dir);
        let compat_data_path = self.path.join("steamapps/compatdata").join(app_id.to_string());

        Ok(SteamApp {
            app_id,
            name,
            install_dir,
            install_path,
            library_path: self.path.clone(),
            size_on_disk,
            compat_data_path: if compat_data_path.exists() {
                Some(compat_data_path)
            } else {
                None
            },
        })
    }
}

impl SteamApp {
    /// Check if this game uses Proton (has compatdata)
    pub fn uses_proton(&self) -> bool {
        self.compat_data_path.is_some()
    }

    /// Get the Wine prefix path for this game (inside compatdata)
    pub fn get_prefix_path(&self) -> Option<PathBuf> {
        self.compat_data_path.as_ref().map(|p| p.join("pfx"))
    }

    /// Format size as human-readable string
    pub fn size_human(&self) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;

        if self.size_on_disk >= GB {
            format!("{:.1} GB", self.size_on_disk as f64 / GB as f64)
        } else if self.size_on_disk >= MB {
            format!("{:.1} MB", self.size_on_disk as f64 / MB as f64)
        } else if self.size_on_disk >= KB {
            format!("{:.1} KB", self.size_on_disk as f64 / KB as f64)
        } else {
            format!("{} B", self.size_on_disk)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_human() {
        let app = SteamApp {
            app_id: 1,
            name: "Test".to_string(),
            install_dir: "test".to_string(),
            install_path: PathBuf::new(),
            library_path: PathBuf::new(),
            size_on_disk: 50 * 1024 * 1024 * 1024, // 50 GB
            compat_data_path: None,
        };
        assert_eq!(app.size_human(), "50.0 GB");
    }
}
