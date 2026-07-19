//! Prefix lifecycle management

use crate::config::WandaConfig;
use crate::error::{Result, WandaError};
use crate::prefix::PrefixBuilder;
use crate::steam::ProtonVersion;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Health status of a Wine prefix
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefixHealth {
    /// Prefix is healthy and ready for use
    Healthy,
    /// Prefix has issues that may be repairable
    NeedsRepair(Vec<PrefixIssue>),
    /// Prefix is corrupted beyond repair
    Corrupted(String),
    /// Prefix doesn't exist yet
    NotCreated,
}

/// Specific issues that can affect a prefix
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefixIssue {
    /// .NET Framework is missing
    MissingDotNet,
    /// Required dependency is missing
    MissingDependency(String),
    /// Registry appears corrupted
    CorruptedRegistry,
    /// WeMod is not installed
    WemodNotInstalled,
    /// WeMod version is outdated
    WemodOutdated,
    /// Wine prefix structure is incomplete
    IncompletePrefix,
}

impl std::fmt::Display for PrefixIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingDotNet => write!(f, ".NET Framework 4.8 not installed"),
            Self::MissingDependency(dep) => write!(f, "Missing dependency: {}", dep),
            Self::CorruptedRegistry => write!(f, "Windows registry is corrupted"),
            Self::WemodNotInstalled => write!(f, "WeMod is not installed"),
            Self::WemodOutdated => write!(f, "WeMod is outdated"),
            Self::IncompletePrefix => write!(f, "Wine prefix structure is incomplete"),
        }
    }
}

/// Metadata about a WANDA prefix
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WandaPrefix {
    /// Name of this prefix
    pub name: String,
    /// Path to the prefix
    pub path: PathBuf,
    /// Whether WeMod is installed in this prefix
    pub wemod_installed: bool,
    /// Installed WeMod version
    pub wemod_version: Option<String>,
    /// Proton version used to create this prefix
    pub proton_version: Option<String>,
    /// When the prefix was created
    pub created_at: Option<String>,
    /// When the prefix was last used
    pub last_used: Option<String>,
}

impl WandaPrefix {
    /// Get the path to the actual Wine prefix (pfx subdirectory)
    /// Proton stores the Wine prefix in a `pfx` subdirectory
    pub fn pfx_path(&self) -> PathBuf {
        self.path.join("pfx")
    }

    /// Get the path to drive_c (inside the pfx directory)
    pub fn drive_c(&self) -> PathBuf {
        self.pfx_path().join("drive_c")
    }

    /// Get the path to the Windows user folder
    pub fn user_folder(&self) -> PathBuf {
        self.drive_c().join("users/steamuser")
    }

    /// Get the WeMod installation path.
    ///
    /// Version 12.x uses "Wand" branding, version 11.x uses "WeMod". We can't
    /// just return the first branded directory that exists: WeMod 11.6 creates
    /// an empty `AppData/Local/Wand` directory for logs while installing the
    /// actual app under `AppData/Local/WeMod`. So prefer whichever directory
    /// actually contains a WeMod install (root stub or `app-*/WeMod.exe`), and
    /// only fall back to a bare-existing directory if neither looks installed.
    pub fn wemod_path(&self) -> PathBuf {
        let wand_path = self.user_folder().join("AppData/Local/Wand");
        let wemod_path = self.user_folder().join("AppData/Local/WeMod");

        if Self::is_wemod_install_dir(&wand_path) {
            return wand_path;
        }
        if Self::is_wemod_install_dir(&wemod_path) {
            return wemod_path;
        }

        // Neither holds a real install — return whichever exists (Wand first
        // for v12 branding), else default to the WeMod path for fresh installs.
        if wand_path.exists() {
            return wand_path;
        }
        if wemod_path.exists() {
            return wemod_path;
        }
        wemod_path
    }

    /// Whether a directory holds a WeMod install: a root `WeMod.exe` (Squirrel
    /// stub) or a versioned `app-*/WeMod.exe`.
    fn is_wemod_install_dir(dir: &Path) -> bool {
        if dir.join("WeMod.exe").exists() {
            return true;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("app-") && entry.path().join("WeMod.exe").exists() {
                    return true;
                }
            }
        }
        false
    }

    /// Get the WeMod executable path (Squirrel stub)
    pub fn wemod_exe(&self) -> PathBuf {
        self.wemod_path().join("WeMod.exe")
    }

    /// Get the real WeMod executable inside the versioned app directory
    ///
    /// WeMod uses Squirrel for updates. The root WeMod.exe is a small stub
    /// that spawns the real Electron app from `app-{version}/WeMod.exe` and
    /// exits. For standalone mode we need the real exe so Proton keeps the
    /// Wine session alive.
    pub fn wemod_app_exe(&self) -> Option<PathBuf> {
        let wemod_dir = self.wemod_path();
        let mut best: Option<(String, PathBuf)> = None;

        if let Ok(entries) = std::fs::read_dir(&wemod_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("app-") && entry.path().is_dir() {
                    let exe = entry.path().join("WeMod.exe");
                    if exe.exists() {
                        // Pick the highest version directory
                        if best.as_ref().map_or(true, |(v, _)| name > *v) {
                            best = Some((name, exe));
                        }
                    }
                }
            }
        }

        best.map(|(_, path)| path)
    }

    /// Get alternative WeMod paths for checking installation
    pub fn wemod_paths_all(&self) -> Vec<PathBuf> {
        vec![
            self.user_folder().join("AppData/Local/Wand"),
            self.user_folder().join("AppData/Local/WeMod"),
        ]
    }

    /// Get the system32 directory
    pub fn system32(&self) -> PathBuf {
        self.drive_c().join("windows/system32")
    }
}

/// Manages WANDA Wine prefixes
pub struct PrefixManager {
    /// Base directory for prefixes
    pub base_path: PathBuf,
    /// Currently loaded prefixes
    prefixes: Vec<WandaPrefix>,
}

impl PrefixManager {
    /// Create a new prefix manager
    pub fn new(config: &WandaConfig) -> Self {
        Self {
            base_path: config.prefix_base_path(),
            prefixes: Vec::new(),
        }
    }

    /// Load all existing prefixes
    pub fn load(&mut self) -> Result<()> {
        self.prefixes.clear();

        if !self.base_path.exists() {
            debug!("Prefix base path doesn't exist yet: {}", self.base_path.display());
            return Ok(());
        }

        // Look for prefix metadata files
        let entries = std::fs::read_dir(&self.base_path)?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let meta_path = path.join("wanda.json");
                if meta_path.exists() {
                    match self.load_prefix_metadata(&meta_path) {
                        Ok(prefix) => {
                            debug!("Loaded prefix: {}", prefix.name);
                            self.prefixes.push(prefix);
                        }
                        Err(e) => {
                            warn!("Failed to load prefix at {}: {}", path.display(), e);
                        }
                    }
                }
            }
        }

        info!("Loaded {} prefixes", self.prefixes.len());
        Ok(())
    }

    /// Load prefix metadata from file
    fn load_prefix_metadata(&self, path: &Path) -> Result<WandaPrefix> {
        let content = std::fs::read_to_string(path)?;
        let prefix: WandaPrefix = serde_json::from_str(&content)?;
        Ok(prefix)
    }

    /// Save prefix metadata to file
    fn save_prefix_metadata(prefix: &WandaPrefix) -> Result<()> {
        let meta_path = prefix.path.join("wanda.json");
        let content = serde_json::to_string_pretty(prefix)?;
        std::fs::write(meta_path, content)?;
        Ok(())
    }

    /// Get all prefixes
    pub fn list(&self) -> &[WandaPrefix] {
        &self.prefixes
    }

    /// Get a prefix by name
    pub fn get(&self, name: &str) -> Option<&WandaPrefix> {
        self.prefixes.iter().find(|p| p.name == name)
    }

    /// Get the default prefix (creates one if it doesn't exist)
    pub async fn get_or_create_default(
        &mut self,
        proton: &ProtonVersion,
    ) -> Result<&WandaPrefix> {
        const DEFAULT_NAME: &str = "default";

        // Check if default exists
        if self.get(DEFAULT_NAME).is_some() {
            // Validate health
            let health = self.validate(DEFAULT_NAME)?;
            if health != PrefixHealth::Healthy {
                warn!("Default prefix needs attention: {:?}", health);
            }
            return Ok(self.get(DEFAULT_NAME).unwrap());
        }

        // Create the default prefix
        self.create(DEFAULT_NAME, proton).await?;
        Ok(self.get(DEFAULT_NAME).unwrap())
    }

    /// Create a new prefix
    pub async fn create(&mut self, name: &str, proton: &ProtonVersion) -> Result<WandaPrefix> {
        let prefix_path = self.base_path.join(name);

        if prefix_path.exists() {
            return Err(WandaError::PrefixCreationFailed {
                path: prefix_path,
                reason: "Prefix already exists".to_string(),
            });
        }

        info!("Creating prefix '{}' at {}", name, prefix_path.display());

        // Use PrefixBuilder to set up the prefix
        let builder = PrefixBuilder::new(&prefix_path, proton);
        builder.build().await?;

        // Create metadata
        let prefix = WandaPrefix {
            name: name.to_string(),
            path: prefix_path,
            wemod_installed: false,
            wemod_version: None,
            proton_version: Some(proton.name.clone()),
            created_at: Some(chrono_lite_now()),
            last_used: None,
        };

        Self::save_prefix_metadata(&prefix)?;
        self.prefixes.push(prefix.clone());

        info!("Prefix '{}' created successfully", name);
        Ok(prefix)
    }

    /// Validate a prefix's health
    pub fn validate(&self, name: &str) -> Result<PrefixHealth> {
        let prefix = self.get(name).ok_or_else(|| WandaError::PrefixNotFound {
            path: self.base_path.join(name),
        })?;

        let mut issues = Vec::new();

        // Check basic structure
        if !prefix.drive_c().exists() {
            return Ok(PrefixHealth::NotCreated);
        }

        if !prefix.system32().exists() {
            issues.push(PrefixIssue::IncompletePrefix);
        }

        // Check for registry files (in the pfx directory)
        let system_reg = prefix.pfx_path().join("system.reg");
        let user_reg = prefix.pfx_path().join("user.reg");
        if !system_reg.exists() || !user_reg.exists() {
            issues.push(PrefixIssue::CorruptedRegistry);
        }

        // Check .NET (look for mscorlib.dll as indicator)
        let dotnet_indicator = prefix.drive_c().join("windows/Microsoft.NET");
        if !dotnet_indicator.exists() {
            issues.push(PrefixIssue::MissingDotNet);
        }

        // Check WeMod
        if !prefix.wemod_exe().exists() {
            issues.push(PrefixIssue::WemodNotInstalled);
        }

        if issues.is_empty() {
            Ok(PrefixHealth::Healthy)
        } else if issues.contains(&PrefixIssue::CorruptedRegistry) {
            Ok(PrefixHealth::Corrupted("Registry files are missing or corrupted".to_string()))
        } else {
            Ok(PrefixHealth::NeedsRepair(issues))
        }
    }

    /// Attempt to repair a prefix
    pub async fn repair(&mut self, name: &str, proton: &ProtonVersion) -> Result<()> {
        let health = self.validate(name)?;

        match health {
            PrefixHealth::Healthy => {
                info!("Prefix '{}' is already healthy", name);
                return Ok(());
            }
            PrefixHealth::Corrupted(reason) => {
                return Err(WandaError::PrefixCorrupted {
                    path: self.base_path.join(name),
                    reason,
                });
            }
            PrefixHealth::NotCreated => {
                info!("Prefix doesn't exist, creating it");
                self.create(name, proton).await?;
                return Ok(());
            }
            PrefixHealth::NeedsRepair(issues) => {
                info!("Repairing prefix '{}': {:?}", name, issues);
                // Get prefix path before mutable borrow
                let prefix_path = self.base_path.join(name);
                let builder = PrefixBuilder::new(&prefix_path, proton);

                for issue in issues {
                    match issue {
                        PrefixIssue::MissingDotNet => {
                            info!("Installing .NET Framework...");
                            builder.install_dotnet().await?;
                        }
                        PrefixIssue::MissingDependency(dep) => {
                            info!("Installing dependency: {}", dep);
                            builder.install_winetricks(&[&dep]).await?;
                        }
                        PrefixIssue::WemodNotInstalled | PrefixIssue::WemodOutdated => {
                            // WeMod installation is handled separately
                            info!("WeMod needs to be installed/updated");
                        }
                        _ => {
                            warn!("Cannot automatically repair: {:?}", issue);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Delete a prefix
    pub fn delete(&mut self, name: &str) -> Result<()> {
        let prefix_path = self.base_path.join(name);

        if !prefix_path.exists() {
            return Err(WandaError::PrefixNotFound { path: prefix_path });
        }

        info!("Deleting prefix '{}' at {}", name, prefix_path.display());
        std::fs::remove_dir_all(&prefix_path)?;

        self.prefixes.retain(|p| p.name != name);

        Ok(())
    }

    /// Update prefix metadata (e.g., after WeMod installation)
    pub fn update_metadata(&mut self, name: &str, updater: impl FnOnce(&mut WandaPrefix)) -> Result<()> {
        if let Some(prefix) = self.prefixes.iter_mut().find(|p| p.name == name) {
            updater(prefix);
            Self::save_prefix_metadata(prefix)?;
        }
        Ok(())
    }
}

/// Get current timestamp as ISO string (simple implementation)
fn chrono_lite_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", duration.as_secs())
}
