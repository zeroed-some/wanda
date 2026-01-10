//! Application state management

use wanda_core::{
    config::WandaConfig,
    prefix::PrefixManager,
    steam::{ProtonManager, SteamInstallation},
};

/// Shared application state
pub struct AppState {
    /// Loaded configuration
    pub config: Option<WandaConfig>,
    /// Steam installation (cached)
    pub steam: Option<SteamInstallation>,
    /// Proton manager (cached)
    pub proton: Option<ProtonManager>,
    /// Prefix manager
    pub prefix_manager: Option<PrefixManager>,
    /// Whether WANDA is initialized
    pub initialized: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            config: None,
            steam: None,
            proton: None,
            prefix_manager: None,
            initialized: false,
        }
    }

    /// Load or reload state from disk
    pub fn load(&mut self) -> Result<(), String> {
        // Load config
        let config = WandaConfig::load().map_err(|e| e.to_string())?;

        // Discover Steam
        let steam = SteamInstallation::discover(&config).map_err(|e| e.to_string())?;

        // Discover Proton
        let proton = ProtonManager::discover(&steam, &config).map_err(|e| e.to_string())?;

        // Load prefix manager
        let mut prefix_manager = PrefixManager::new(&config);
        prefix_manager.load().map_err(|e| e.to_string())?;

        // Check if initialized (has default prefix with WeMod)
        let initialized = prefix_manager
            .get("default")
            .map(|p| p.wemod_installed)
            .unwrap_or(false);

        self.config = Some(config);
        self.steam = Some(steam);
        self.proton = Some(proton);
        self.prefix_manager = Some(prefix_manager);
        self.initialized = initialized;

        Ok(())
    }

    /// Ensure state is loaded
    pub fn ensure_loaded(&mut self) -> Result<(), String> {
        if self.config.is_none() {
            self.load()?;
        }
        Ok(())
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
