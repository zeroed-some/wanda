//! Prefix building and initialization

use crate::error::{Result, WandaError};
use crate::steam::ProtonVersion;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::{debug, error, info, warn};

/// Builds and initializes Wine prefixes for WeMod
pub struct PrefixBuilder<'a> {
    /// Base path for the wanda prefix (STEAM_COMPAT_DATA_PATH equivalent)
    base_path: PathBuf,
    /// Actual Wine prefix path (pfx subdirectory, used by Proton)
    prefix_path: PathBuf,
    /// Proton version to use
    proton: &'a ProtonVersion,
}

impl<'a> PrefixBuilder<'a> {
    /// Create a new prefix builder
    ///
    /// Note: Proton creates a `pfx` subdirectory inside the base path for the actual
    /// Wine prefix. We install dependencies (like .NET) into the pfx directory so
    /// they're available when running apps through Proton.
    pub fn new(base_path: &Path, proton: &'a ProtonVersion) -> Self {
        Self {
            base_path: base_path.to_path_buf(),
            prefix_path: base_path.join("pfx"),
            proton,
        }
    }

    /// Get the base path (STEAM_COMPAT_DATA_PATH equivalent)
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    /// Get the actual Wine prefix path (pfx subdirectory)
    pub fn wine_prefix_path(&self) -> &Path {
        &self.prefix_path
    }

    /// Get path to Wine executable (prefer Proton's bundled wine)
    fn get_wine_path(&self) -> PathBuf {
        // Try Proton's wine64 first
        let proton_wine = self.proton.path.join("files/bin/wine64");
        debug!("Checking for Proton wine64 at: {}", proton_wine.display());
        if proton_wine.exists() {
            debug!("Found Proton wine64");
            return proton_wine;
        }

        // Try Proton's wine
        let proton_wine = self.proton.path.join("files/bin/wine");
        debug!("Checking for Proton wine at: {}", proton_wine.display());
        if proton_wine.exists() {
            debug!("Found Proton wine");
            return proton_wine;
        }

        // Fall back to system wine
        warn!("No Proton wine found, falling back to system wine");
        PathBuf::from("wine")
    }

    /// Build environment variables for Wine/Proton commands
    fn build_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();

        let wine_path = self.get_wine_path();
        debug!("Building environment for Wine at: {}", wine_path.display());

        // Wine prefix location
        env.insert(
            "WINEPREFIX".to_string(),
            self.prefix_path.to_string_lossy().to_string(),
        );

        // Use 64-bit Windows
        env.insert("WINEARCH".to_string(), "win64".to_string());

        // Tell winetricks which wine to use
        env.insert("WINE".to_string(), wine_path.to_string_lossy().to_string());

        // Proton lib paths for finding dependencies
        let proton_lib64 = self.proton.path.join("files/lib64");
        let proton_lib = self.proton.path.join("files/lib");
        debug!("Checking Proton lib64: {} (exists: {})", proton_lib64.display(), proton_lib64.exists());
        debug!("Checking Proton lib: {} (exists: {})", proton_lib.display(), proton_lib.exists());

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
            // Append existing LD_LIBRARY_PATH
            if let Ok(existing) = std::env::var("LD_LIBRARY_PATH") {
                ld_path.push(':');
                ld_path.push_str(&existing);
            }
            debug!("Setting LD_LIBRARY_PATH: {}", ld_path);
            env.insert("LD_LIBRARY_PATH".to_string(), ld_path);
        } else {
            warn!("No Proton lib directories found - Wine may have trouble finding libraries");
        }

        // Proton-specific variables
        env.insert("PROTON_NO_ESYNC".to_string(), "1".to_string());
        env.insert("PROTON_NO_FSYNC".to_string(), "1".to_string());

        // Disable Wine debug output to prevent OOM from massive log accumulation
        env.insert("WINEDEBUG".to_string(), "-all".to_string());

        // Log all environment variables at trace level
        for (key, value) in &env {
            debug!("ENV {}={}", key, value);
        }

        env
    }

    /// Build the prefix from scratch
    pub async fn build(&self) -> Result<()> {
        info!("Building prefix at {}", self.base_path.display());
        info!("Wine prefix (pfx): {}", self.prefix_path.display());
        info!("Using Proton: {}", self.proton.name);
        info!("Wine path: {}", self.get_wine_path().display());

        // Create directory structure (both base and pfx)
        std::fs::create_dir_all(&self.base_path)?;
        std::fs::create_dir_all(&self.prefix_path)?;

        // Initialize with wineboot
        self.init_wineboot().await?;

        // Try to install .NET Framework (but don't fail if it doesn't work)
        match self.install_dotnet().await {
            Ok(_) => info!(".NET Framework installed successfully"),
            Err(e) => {
                warn!(".NET Framework installation failed: {}", e);
                warn!("WeMod may still work - continuing with installation");
            }
        }

        // Install additional dependencies (optional)
        self.install_dependencies().await?;

        info!("Prefix built successfully");
        Ok(())
    }

    /// Initialize the prefix with wineboot
    async fn init_wineboot(&self) -> Result<()> {
        info!("Initializing Wine prefix with wineboot...");

        let wine_path = self.get_wine_path();
        let env = self.build_env();

        info!("Running: {} wineboot --init", wine_path.display());

        let output = Command::new(&wine_path)
            .arg("wineboot")
            .arg("--init")
            .envs(&env)
            .output()
            .await
            .map_err(|e| WandaError::PrefixCreationFailed {
                path: self.prefix_path.clone(),
                reason: format!("wineboot failed to start: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            error!("wineboot stdout: {}", stdout);
            error!("wineboot stderr: {}", stderr);
            return Err(WandaError::PrefixCreationFailed {
                path: self.prefix_path.clone(),
                reason: format!("wineboot failed with exit code: {:?}", output.status.code()),
            });
        }

        debug!("wineboot completed successfully");

        // Wait for wineserver to finish
        self.wait_wineserver().await;

        Ok(())
    }

    /// Wait for wineserver to finish
    async fn wait_wineserver(&self) {
        let env = self.build_env();

        // Try Proton's wineserver first
        let proton_wineserver = self.proton.path.join("files/bin/wineserver");
        let wineserver = if proton_wineserver.exists() {
            proton_wineserver.to_string_lossy().to_string()
        } else {
            "wineserver".to_string()
        };

        let _ = Command::new(&wineserver)
            .arg("-w")
            .envs(&env)
            .status()
            .await;
    }

    /// Install .NET Framework 4.8 (required for WeMod)
    pub async fn install_dotnet(&self) -> Result<()> {
        info!("Installing .NET Framework 4.8 via winetricks...");
        info!("This may take 10-15 minutes and requires internet connection");

        // Try dotnet48 first, fall back to dotnet40 if it fails
        match self.install_winetricks_verbose(&["dotnet48"]).await {
            Ok(_) => return Ok(()),
            Err(e) => {
                warn!("dotnet48 failed: {}", e);
                warn!("Trying dotnet40 as fallback...");
            }
        }

        // Try dotnet40 as fallback
        self.install_winetricks_verbose(&["dotnet40"]).await
    }

    /// Install additional dependencies
    async fn install_dependencies(&self) -> Result<()> {
        info!("Installing additional dependencies...");

        // Install common dependencies WeMod might need
        let deps = ["vcrun2019", "corefonts"];

        for dep in deps {
            info!("Installing {}...", dep);
            match self.install_winetricks_verbose(&[dep]).await {
                Ok(_) => info!("{} installed successfully", dep),
                Err(e) => warn!("Failed to install {}: {} (may not be critical)", dep, e),
            }
        }

        Ok(())
    }

    /// Install components via winetricks with verbose output
    pub async fn install_winetricks_verbose(&self, components: &[&str]) -> Result<()> {
        let env = self.build_env();

        // Check if winetricks is available
        let winetricks_check = Command::new("which").arg("winetricks").output().await;

        if winetricks_check.is_err() || !winetricks_check.unwrap().status.success() {
            return Err(WandaError::WinetricksFailed {
                reason: "winetricks not found. Please install winetricks.".to_string(),
            });
        }

        for component in components {
            info!("Installing {} via winetricks...", component);
            debug!("Environment: WINE={}", env.get("WINE").unwrap_or(&"".to_string()));
            debug!("Environment: WINEPREFIX={}", env.get("WINEPREFIX").unwrap_or(&"".to_string()));

            // Run winetricks with output going directly to terminal (not collected in memory)
            // This prevents OOM when installing large components like dotnet48
            let status = Command::new("winetricks")
                .arg("--force") // Force installation even if already installed
                .arg(component)
                .envs(&env)
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .status()
                .await
                .map_err(|e| WandaError::WinetricksFailed {
                    reason: format!("Failed to run winetricks: {}", e),
                })?;

            if !status.success() {
                error!("winetricks {} failed!", component);
                return Err(WandaError::WinetricksFailed {
                    reason: format!(
                        "winetricks {} failed with exit code {:?}",
                        component,
                        status.code()
                    ),
                });
            }
        }

        // Wait for wineserver to finish
        self.wait_wineserver().await;

        Ok(())
    }

    /// Install components via winetricks (quiet mode)
    pub async fn install_winetricks(&self, components: &[&str]) -> Result<()> {
        self.install_winetricks_verbose(components).await
    }

    /// Run a command in the prefix using Wine
    pub async fn run_wine_command(&self, args: &[&str]) -> Result<std::process::Output> {
        let wine_path = self.get_wine_path();
        let env = self.build_env();

        info!("Running: {} {:?}", wine_path.display(), args);

        let output = Command::new(&wine_path)
            .args(args)
            .envs(&env)
            .output()
            .await
            .map_err(|e| WandaError::PrefixCreationFailed {
                path: self.prefix_path.clone(),
                reason: format!("Wine command failed: {}", e),
            })?;

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_env() {
        let proton = ProtonVersion {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            version: (9, 0, 0),
            is_ge: true,
            is_experimental: false,
            compatibility: crate::steam::ProtonCompatibility::Recommended,
        };

        let builder = PrefixBuilder::new(Path::new("/tmp/test"), &proton);
        let env = builder.build_env();

        assert_eq!(env.get("WINEPREFIX"), Some(&"/tmp/test".to_string()));
        assert_eq!(env.get("WINEARCH"), Some(&"win64".to_string()));
    }
}
