//! Error types for WANDA

use std::path::PathBuf;
use thiserror::Error;

/// Result type alias using WandaError
pub type Result<T> = std::result::Result<T, WandaError>;

/// Main error type for WANDA operations
#[derive(Error, Debug)]
pub enum WandaError {
    // Steam errors
    #[error("Steam installation not found")]
    SteamNotFound,

    #[error("Steam library at {path} is inaccessible: {reason}")]
    SteamLibraryInaccessible { path: PathBuf, reason: String },

    #[error("Failed to parse VDF file {path}: {reason}")]
    VdfParseError { path: PathBuf, reason: String },

    #[error("Game with app ID {app_id} not found")]
    GameNotFound { app_id: u32 },

    // Proton errors
    #[error("No compatible Proton version found")]
    ProtonNotFound,

    #[error("Proton version {version} is incompatible: {reason}")]
    ProtonIncompatible { version: String, reason: String },

    // Prefix errors
    #[error("Prefix at {path} is corrupted: {reason}")]
    PrefixCorrupted { path: PathBuf, reason: String },

    #[error("Failed to create prefix at {path}: {reason}")]
    PrefixCreationFailed { path: PathBuf, reason: String },

    #[error("Prefix at {path} not found")]
    PrefixNotFound { path: PathBuf },

    #[error("Winetricks failed: {reason}")]
    WinetricksFailed { reason: String },

    // WeMod errors
    #[error("Failed to download WeMod: {reason}")]
    WemodDownloadFailed { reason: String },

    #[error("WeMod installation failed: {reason}")]
    WemodInstallFailed { reason: String },

    #[error("WeMod not installed")]
    WemodNotInstalled,

    // Launch errors
    #[error("Failed to launch game: {reason}")]
    LaunchFailed { reason: String },

    #[error("Process terminated unexpectedly: {reason}")]
    ProcessCrashed { reason: String },

    // Configuration errors
    #[error("Configuration error: {reason}")]
    ConfigError { reason: String },

    #[error("Configuration file not found at {path}")]
    ConfigNotFound { path: PathBuf },

    // Generic errors
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
}

impl WandaError {
    /// Get a user-friendly message with suggestions
    pub fn user_message(&self) -> String {
        match self {
            Self::SteamNotFound => {
                "Steam installation not found. Please ensure Steam is installed, \
                or specify the path manually in the config."
                    .into()
            }
            Self::ProtonNotFound => {
                "No compatible Proton version found. Please install GE-Proton \
                or Proton Experimental from Steam."
                    .into()
            }
            Self::PrefixCorrupted { path, reason } => {
                format!(
                    "The Wine prefix at {} appears to be corrupted: {}\n\
                    Try running 'wanda prefix repair' to fix this issue.",
                    path.display(),
                    reason
                )
            }
            Self::WemodNotInstalled => {
                "WeMod is not installed. Run 'wanda init' to set up WANDA.".into()
            }
            _ => self.to_string(),
        }
    }
}
