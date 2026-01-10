//! WeMod installation into Wine prefixes

use crate::error::{Result, WandaError};
use crate::prefix::WandaPrefix;
use crate::steam::ProtonVersion;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, error, info, warn};

/// Handles WeMod installation into Wine prefixes
pub struct WemodInstaller<'a> {
    /// The prefix to install into
    prefix: &'a WandaPrefix,
    /// Proton version for running installer
    proton: &'a ProtonVersion,
}

impl<'a> WemodInstaller<'a> {
    /// Create a new installer
    pub fn new(prefix: &'a WandaPrefix, proton: &'a ProtonVersion) -> Self {
        Self { prefix, proton }
    }

    /// Build environment variables for Wine
    fn build_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();

        env.insert(
            "WINEPREFIX".to_string(),
            self.prefix.path.to_string_lossy().to_string(),
        );
        env.insert("WINEARCH".to_string(), "win64".to_string());
        env.insert("WINEDEBUG".to_string(), "warn+all".to_string());

        // Proton compatibility flags
        env.insert("PROTON_NO_ESYNC".to_string(), "1".to_string());
        env.insert("PROTON_NO_FSYNC".to_string(), "1".to_string());

        // Set Proton lib paths for finding dependencies
        let proton_lib64 = self.proton.path.join("files/lib64");
        let proton_lib = self.proton.path.join("files/lib");
        if proton_lib64.exists() || proton_lib.exists() {
            let mut ld_path = String::new();
            if proton_lib64.exists() {
                ld_path.push_str(&proton_lib64.to_string_lossy());
            }
            if proton_lib.exists() {
                if !ld_path.is_empty() {
                    ld_path.push(':');
                }
                ld_path.push_str(&proton_lib.to_string_lossy());
            }
            if let Ok(existing) = std::env::var("LD_LIBRARY_PATH") {
                ld_path.push(':');
                ld_path.push_str(&existing);
            }
            env.insert("LD_LIBRARY_PATH".to_string(), ld_path);
        }

        debug!("Installer environment:");
        for (key, value) in &env {
            debug!("  {}={}", key, value);
        }

        env
    }

    /// Get the path to wine executable
    fn wine_exe(&self) -> String {
        let proton_wine = self.proton.wine_exe();
        debug!("Checking Proton wine at: {}", proton_wine.display());
        if proton_wine.exists() {
            debug!("Using Proton wine");
            proton_wine.to_string_lossy().to_string()
        } else {
            warn!("Proton wine not found, falling back to system wine");
            "wine".to_string()
        }
    }

    /// Get the path to wineserver executable
    fn wineserver_exe(&self) -> String {
        let proton_wineserver = self.proton.path.join("files/bin/wineserver");
        if proton_wineserver.exists() {
            proton_wineserver.to_string_lossy().to_string()
        } else {
            "wineserver".to_string()
        }
    }

    /// Install WeMod from the downloaded installer
    pub async fn install(&self, installer_path: &Path) -> Result<()> {
        if !installer_path.exists() {
            return Err(WandaError::WemodInstallFailed {
                reason: format!("Installer not found at {}", installer_path.display()),
            });
        }

        info!("Installing WeMod from {}...", installer_path.display());

        let env = self.build_env();
        let wine = self.wine_exe();

        info!("Using wine: {}", wine);
        info!("Running: {} {} /S", wine, installer_path.display());

        // WeMod installer supports silent installation with /S flag
        let output = Command::new(&wine)
            .arg(installer_path)
            .arg("/S") // Silent install
            .envs(&env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| WandaError::WemodInstallFailed {
                reason: format!("Failed to run installer: {}", e),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !stdout.is_empty() {
            debug!("Installer stdout: {}", stdout);
        }

        if !output.status.success() {
            error!("WeMod installer exited with status: {:?}", output.status.code());
            if !stderr.is_empty() {
                error!("Installer stderr: {}", stderr);
            }
            // Don't fail immediately - installer might still have worked
        } else {
            debug!("Installer completed with success status");
        }

        // Wait for wineserver using Proton's wineserver
        let wineserver = self.wineserver_exe();
        debug!("Waiting for wineserver: {}", wineserver);
        let _ = Command::new(&wineserver)
            .arg("-w")
            .envs(&env)
            .status()
            .await;

        // Verify installation
        info!("Verifying WeMod installation...");
        if self.verify()? {
            info!("WeMod installed successfully");
            Ok(())
        } else {
            error!("WeMod executable not found after installation");
            error!("Checked locations:");
            error!("  - {}", self.prefix.wemod_exe().display());
            error!("  - {}", self.prefix.user_folder().join("AppData/Local/WeMod/WeMod.exe").display());
            error!("  - {}", self.prefix.drive_c().join("Program Files/WeMod/WeMod.exe").display());
            Err(WandaError::WemodInstallFailed {
                reason: "WeMod executable not found after installation".to_string(),
            })
        }
    }

    /// Verify WeMod installation
    pub fn verify(&self) -> Result<bool> {
        // Check if WeMod.exe exists
        let wemod_exe = self.prefix.wemod_exe();
        debug!("Checking for WeMod at: {}", wemod_exe.display());

        if wemod_exe.exists() {
            info!("WeMod found at {}", wemod_exe.display());
            return Ok(true);
        }

        // Also check alternative locations
        let alt_locations = [
            self.prefix
                .user_folder()
                .join("AppData/Local/WeMod/WeMod.exe"),
            self.prefix
                .drive_c()
                .join("Program Files/WeMod/WeMod.exe"),
            self.prefix
                .drive_c()
                .join("Program Files (x86)/WeMod/WeMod.exe"),
        ];

        for loc in &alt_locations {
            debug!("Checking alternative location: {}", loc.display());
            if loc.exists() {
                info!("WeMod found at alternative location: {}", loc.display());
                return Ok(true);
            }
        }

        // List what's in the WeMod directory if it exists
        let wemod_dir = self.prefix.wemod_path();
        debug!("WeMod directory: {}", wemod_dir.display());
        if wemod_dir.exists() {
            debug!("WeMod directory exists, contents:");
            if let Ok(entries) = std::fs::read_dir(&wemod_dir) {
                for entry in entries.flatten() {
                    debug!("  - {}", entry.path().display());
                }
            }
        } else {
            debug!("WeMod directory does not exist");
        }

        // Also check what's in AppData/Local
        let local_appdata = self.prefix.user_folder().join("AppData/Local");
        if local_appdata.exists() {
            debug!("Listing AppData/Local contents:");
            if let Ok(entries) = std::fs::read_dir(&local_appdata) {
                for entry in entries.flatten() {
                    debug!("  - {}", entry.file_name().to_string_lossy());
                }
            }
        }

        debug!("WeMod not found in prefix");
        Ok(false)
    }

    /// Get the installed WeMod version (if available)
    pub fn get_version(&self) -> Option<String> {
        // Try to read version from WeMod's app data
        let version_file = self.prefix.wemod_path().join("current");
        if version_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&version_file) {
                // The 'current' file contains the version directory name
                let version = content.trim().to_string();
                if !version.is_empty() {
                    return Some(version);
                }
            }
        }

        // Try to detect from installed app directories
        let wemod_dir = self.prefix.wemod_path();
        if wemod_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&wemod_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    // Look for version directories like "app-8.12.5"
                    if name.starts_with("app-") {
                        return Some(name.strip_prefix("app-").unwrap_or(&name).to_string());
                    }
                }
            }
        }

        None
    }

    /// Uninstall WeMod from the prefix
    pub async fn uninstall(&self) -> Result<()> {
        info!("Uninstalling WeMod...");

        let wemod_dir = self.prefix.wemod_path();
        if wemod_dir.exists() {
            tokio::fs::remove_dir_all(&wemod_dir).await?;
            info!("WeMod uninstalled");
        } else {
            info!("WeMod was not installed");
        }

        Ok(())
    }

    /// Run WeMod standalone (for testing or manual use)
    pub async fn run(&self) -> Result<tokio::process::Child> {
        let wemod_exe = self.prefix.wemod_exe();

        if !wemod_exe.exists() {
            return Err(WandaError::WemodNotInstalled);
        }

        let env = self.build_env();
        let wine = self.wine_exe();

        info!("Starting WeMod...");

        let child = Command::new(&wine)
            .arg(&wemod_exe)
            .envs(&env)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| WandaError::LaunchFailed {
                reason: format!("Failed to start WeMod: {}", e),
            })?;

        Ok(child)
    }
}
