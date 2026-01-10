//! Configuration management for WANDA

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, info};

/// Main WANDA configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WandaConfig {
    /// Config schema version for future migrations
    pub version: u32,
    /// Steam-related configuration
    pub steam: SteamConfig,
    /// Proton-related configuration
    pub proton: ProtonConfig,
    /// WeMod-related configuration
    pub wemod: WemodConfig,
    /// Prefix-related configuration
    pub prefix: PrefixConfig,
}

impl Default for WandaConfig {
    fn default() -> Self {
        Self {
            version: 1,
            steam: SteamConfig::default(),
            proton: ProtonConfig::default(),
            wemod: WemodConfig::default(),
            prefix: PrefixConfig::default(),
        }
    }
}

/// Steam configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SteamConfig {
    /// Override auto-detected Steam installation path
    pub install_path: Option<PathBuf>,
    /// Additional Steam library folders to scan
    pub additional_libraries: Vec<PathBuf>,
    /// Whether to scan Flatpak Steam installation
    pub scan_flatpak: bool,
}

/// Proton configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProtonConfig {
    /// Preferred Proton version (e.g., "GE-Proton9-5")
    pub preferred_version: Option<String>,
    /// Additional paths to search for Proton installations
    pub search_paths: Vec<PathBuf>,
}

/// WeMod configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WemodConfig {
    /// Whether to automatically update WeMod
    pub auto_update: bool,
    /// Pin to a specific WeMod version
    pub version: Option<String>,
}

impl Default for WemodConfig {
    fn default() -> Self {
        Self {
            auto_update: true,
            version: None,
        }
    }
}

/// Prefix configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrefixConfig {
    /// Custom base path for WANDA prefixes
    pub base_path: Option<PathBuf>,
    /// Whether to attempt auto-repair of corrupted prefixes
    pub auto_repair: bool,
}

impl Default for PrefixConfig {
    fn default() -> Self {
        Self {
            base_path: None,
            auto_repair: true,
        }
    }
}

impl WandaConfig {
    /// Get the default config file path
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("wanda")
            .join("config.toml")
    }

    /// Get the default data directory
    pub fn default_data_dir() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("wanda")
    }

    /// Get the default cache directory
    pub fn default_cache_dir() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("wanda")
    }

    /// Load configuration from the default path
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::default_path())
    }

    /// Load configuration from a specific path
    pub fn load_from(path: &PathBuf) -> Result<Self> {
        if !path.exists() {
            debug!("Config file not found at {}, using defaults", path.display());
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)?;
        let config: WandaConfig = toml::from_str(&content)?;
        debug!("Loaded config from {}", path.display());
        Ok(config)
    }

    /// Save configuration to the default path
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::default_path())
    }

    /// Save configuration to a specific path
    pub fn save_to(&self, path: &PathBuf) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        info!("Saved config to {}", path.display());
        Ok(())
    }

    /// Get the prefix base path (custom or default)
    pub fn prefix_base_path(&self) -> PathBuf {
        self.prefix
            .base_path
            .clone()
            .unwrap_or_else(|| Self::default_data_dir().join("prefix"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = WandaConfig::default();
        assert_eq!(config.version, 1);
        assert!(config.wemod.auto_update);
    }

    #[test]
    fn test_serialize_deserialize() {
        let config = WandaConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: WandaConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.version, config.version);
    }
}
