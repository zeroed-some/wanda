//! Proton version detection and management

use crate::config::WandaConfig;
use crate::error::{Result, WandaError};
use crate::steam::SteamInstallation;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Proton compatibility level with WeMod
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtonCompatibility {
    /// Known to work well with WeMod (GE-Proton 8+)
    Recommended,
    /// Should work with WeMod
    Supported,
    /// May work but not tested
    Experimental,
    /// Known to have issues
    Unsupported,
}

/// A detected Proton installation
#[derive(Debug, Clone)]
pub struct ProtonVersion {
    /// Display name (e.g., "GE-Proton9-5", "Proton 8.0")
    pub name: String,
    /// Path to the Proton installation
    pub path: PathBuf,
    /// Parsed version numbers (major, minor, patch)
    pub version: (u32, u32, u32),
    /// Whether this is a GE-Proton build
    pub is_ge: bool,
    /// Whether this is Proton Experimental
    pub is_experimental: bool,
    /// Compatibility level with WeMod
    pub compatibility: ProtonCompatibility,
}

impl ProtonVersion {
    /// Get the path to the proton executable
    pub fn proton_exe(&self) -> PathBuf {
        self.path.join("proton")
    }

    /// Get the path to the Wine binary
    pub fn wine_exe(&self) -> PathBuf {
        self.path.join("files/bin/wine64")
    }
}

impl PartialOrd for ProtonVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ProtonVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        // Prefer GE-Proton over standard
        match (self.is_ge, other.is_ge) {
            (true, false) => return Ordering::Greater,
            (false, true) => return Ordering::Less,
            _ => {}
        }
        // Then compare by version
        self.version.cmp(&other.version)
    }
}

impl PartialEq for ProtonVersion {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.path == other.path
    }
}

impl Eq for ProtonVersion {}

/// Manages Proton version discovery and selection
pub struct ProtonManager {
    /// All discovered Proton versions
    pub versions: Vec<ProtonVersion>,
}

impl ProtonManager {
    /// Discover all Proton versions on the system
    pub fn discover(steam: &SteamInstallation, config: &WandaConfig) -> Result<Self> {
        let mut versions = Vec::new();

        // Search in Steam's common folder (official Proton)
        let common_dir = steam.root_path.join("steamapps/common");
        Self::scan_directory(&common_dir, &mut versions);

        // Search in compatibility tools directory (GE-Proton, custom builds)
        let compat_tools_dir = steam.root_path.join("compatibilitytools.d");
        Self::scan_directory(&compat_tools_dir, &mut versions);

        // System-wide compatibility tools
        let system_compat = PathBuf::from("/usr/share/steam/compatibilitytools.d");
        if system_compat.exists() {
            Self::scan_directory(&system_compat, &mut versions);
        }

        // User-configured additional paths
        for path in &config.proton.search_paths {
            if path.exists() {
                Self::scan_directory(path, &mut versions);
            }
        }

        // Sort by preference (GE-Proton first, then by version descending)
        versions.sort_by(|a, b| b.cmp(a));

        info!("Discovered {} Proton versions", versions.len());
        for v in &versions {
            debug!(
                "  {} ({:?}) at {}",
                v.name,
                v.compatibility,
                v.path.display()
            );
        }

        Ok(Self { versions })
    }

    /// Scan a directory for Proton installations
    fn scan_directory(dir: &Path, versions: &mut Vec<ProtonVersion>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(version) = Self::detect_proton(&path) {
                    versions.push(version);
                }
            }
        }
    }

    /// Detect if a directory contains a Proton installation
    fn detect_proton(path: &Path) -> Option<ProtonVersion> {
        // Proton directories should have a 'proton' script
        let proton_script = path.join("proton");
        if !proton_script.exists() {
            return None;
        }

        let name = path.file_name()?.to_str()?.to_string();

        // Parse version from name
        let (version, is_ge, is_experimental) = Self::parse_version_from_name(&name);
        let compatibility = Self::determine_compatibility(&name, version, is_ge, is_experimental);

        Some(ProtonVersion {
            name,
            path: path.to_path_buf(),
            version,
            is_ge,
            is_experimental,
            compatibility,
        })
    }

    /// Parse version numbers from Proton directory name
    fn parse_version_from_name(name: &str) -> ((u32, u32, u32), bool, bool) {
        let is_ge = name.contains("GE-Proton") || name.contains("Proton-GE");
        let is_experimental = name.contains("Experimental");

        // Try to extract version numbers
        // GE-Proton format: "GE-Proton9-5" -> (9, 5, 0)
        // Standard format: "Proton 8.0-5" -> (8, 0, 5)
        // Experimental: "Proton - Experimental" -> (999, 0, 0) to sort high

        if is_experimental {
            return ((999, 0, 0), is_ge, is_experimental);
        }

        let version = Self::extract_version_numbers(name);
        (version, is_ge, is_experimental)
    }

    /// Extract version numbers from a string
    fn extract_version_numbers(s: &str) -> (u32, u32, u32) {
        let numbers: Vec<u32> = s
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
            .collect::<String>()
            .split(|c| c == '.' || c == '-')
            .filter_map(|n| n.parse().ok())
            .collect();

        match numbers.as_slice() {
            [major, minor, patch, ..] => (*major, *minor, *patch),
            [major, minor] => (*major, *minor, 0),
            [major] => (*major, 0, 0),
            [] => (0, 0, 0),
        }
    }

    /// Determine compatibility level based on version info
    fn determine_compatibility(
        name: &str,
        version: (u32, u32, u32),
        is_ge: bool,
        is_experimental: bool,
    ) -> ProtonCompatibility {
        // Proton Experimental is recommended for WeMod
        // GE-Proton 10.x has wow64 mode issues that cause WeMod to crash
        if is_experimental {
            return ProtonCompatibility::Recommended;
        }

        // GE-Proton 9.x and below should work (before wow64 mode)
        if is_ge && version.0 >= 8 && version.0 <= 9 {
            return ProtonCompatibility::Supported;
        }

        // GE-Proton 10.x has known wow64 issues with WeMod
        if is_ge && version.0 >= 10 {
            return ProtonCompatibility::Experimental;
        }

        // GE-Proton 7.x might work
        if is_ge && version.0 >= 7 {
            return ProtonCompatibility::Supported;
        }

        // Standard Proton 8+ should work
        if version.0 >= 8 {
            return ProtonCompatibility::Supported;
        }

        // Wine-GE has known issues
        if name.contains("Wine-GE") {
            return ProtonCompatibility::Unsupported;
        }

        // Older versions might have issues
        if version.0 < 7 {
            return ProtonCompatibility::Unsupported;
        }

        ProtonCompatibility::Experimental
    }

    /// Get the recommended Proton version for WeMod
    pub fn get_recommended(&self) -> Option<&ProtonVersion> {
        // First try to find a Recommended version
        if let Some(v) = self
            .versions
            .iter()
            .find(|v| v.compatibility == ProtonCompatibility::Recommended)
        {
            return Some(v);
        }

        // Fall back to Supported
        if let Some(v) = self
            .versions
            .iter()
            .find(|v| v.compatibility == ProtonCompatibility::Supported)
        {
            return Some(v);
        }

        // Fall back to Experimental
        self.versions
            .iter()
            .find(|v| v.compatibility == ProtonCompatibility::Experimental)
    }

    /// Find a Proton version by name
    pub fn find_by_name(&self, name: &str) -> Option<&ProtonVersion> {
        let name_lower = name.to_lowercase();
        self.versions
            .iter()
            .find(|v| v.name.to_lowercase().contains(&name_lower))
    }

    /// Get a version by preference: user-specified, then recommended
    pub fn get_preferred(&self, config: &WandaConfig) -> Result<&ProtonVersion> {
        // Check user preference first
        if let Some(ref preferred) = config.proton.preferred_version {
            if let Some(v) = self.find_by_name(preferred) {
                if v.compatibility == ProtonCompatibility::Unsupported {
                    warn!(
                        "Preferred Proton version {} has known compatibility issues",
                        v.name
                    );
                }
                return Ok(v);
            }
            warn!(
                "Preferred Proton version '{}' not found, using recommended",
                preferred
            );
        }

        self.get_recommended().ok_or(WandaError::ProtonNotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parsing() {
        let (v, is_ge, _) = ProtonManager::parse_version_from_name("GE-Proton9-5");
        assert_eq!(v, (9, 5, 0));
        assert!(is_ge);

        let (v, is_ge, _) = ProtonManager::parse_version_from_name("Proton 8.0-5");
        assert_eq!(v, (8, 0, 5));
        assert!(!is_ge);
    }

    #[test]
    fn test_compatibility() {
        // Proton Experimental is now recommended (GE-Proton 10.x has wow64 issues)
        let compat =
            ProtonManager::determine_compatibility("Proton - Experimental", (0, 0, 0), false, true);
        assert_eq!(compat, ProtonCompatibility::Recommended);

        // GE-Proton 9.x is supported (before wow64 issues)
        let compat =
            ProtonManager::determine_compatibility("GE-Proton9-5", (9, 5, 0), true, false);
        assert_eq!(compat, ProtonCompatibility::Supported);

        // GE-Proton 10.x is experimental (wow64 issues)
        let compat =
            ProtonManager::determine_compatibility("GE-Proton10-28", (10, 28, 0), true, false);
        assert_eq!(compat, ProtonCompatibility::Experimental);

        let compat =
            ProtonManager::determine_compatibility("Proton 6.0", (6, 0, 0), false, false);
        assert_eq!(compat, ProtonCompatibility::Unsupported);
    }
}
