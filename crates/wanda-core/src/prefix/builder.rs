//! Prefix building and initialization

use crate::error::{Result, WandaError};
use crate::steam::ProtonVersion;
use futures::StreamExt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

/// Microsoft .NET Framework 4.8 offline installer (stable URL)
const DOTNET48_URL: &str = "https://download.visualstudio.microsoft.com/download/pr/2d6bb6b2-226a-4baa-bdec-798822606ff1/8494001c276a4b96804cde7829c04d7f/ndp48-x86-x64-allos-enu.exe";
const DOTNET48_FILENAME: &str = "ndp48-x86-x64-allos-enu.exe";
/// Timeout for the .NET installer (5 minutes)
const DOTNET_INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

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

        // Tell winetricks which wineserver to use (must match wine version)
        let wineserver_path = self.proton.path.join("files/bin/wineserver");
        if wineserver_path.exists() {
            debug!("Using Proton wineserver: {}", wineserver_path.display());
            env.insert("WINESERVER".to_string(), wineserver_path.to_string_lossy().to_string());
        }

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

        // Pass display variables for GUI installers (required for .NET, vcrun, etc.)
        if let Ok(disp) = std::env::var("DISPLAY") {
            info!("Captured DISPLAY={}", disp);
            env.insert("DISPLAY".to_string(), disp);
        } else {
            warn!("DISPLAY not set in environment!");
        }
        if let Ok(wayland_disp) = std::env::var("WAYLAND_DISPLAY") {
            info!("Captured WAYLAND_DISPLAY={}", wayland_disp);
            env.insert("WAYLAND_DISPLAY".to_string(), wayland_disp);
        }
        if let Ok(xdg_dir) = std::env::var("XDG_RUNTIME_DIR") {
            info!("Captured XDG_RUNTIME_DIR={}", xdg_dir);
            env.insert("XDG_RUNTIME_DIR".to_string(), xdg_dir);
        }

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

        // Patch mscorlib.dll version so WeMod's .NET check passes.
        // WeMod reads the PE FileVersion from mscorlib.dll and requires >= 4.7.2556.0.
        // Wine Mono ships 4.0.30319.1 which fails the check. We patch the version
        // resource to report 4.8.9232.0 (.NET 4.8) — Wine Mono's runtime still works
        // fine, this is purely metadata in the PE resource section.
        self.patch_mscorlib_version()?;

        info!("Prefix built successfully");
        Ok(())
    }

    /// Initialize the prefix with wineboot via Proton's script
    ///
    /// We must run through `./proton run wineboot` rather than calling wine64
    /// directly so that Proton sets up its own prefix layout: steam.exe stubs,
    /// DLL symlinks into the Proton distribution, lsteamclient, etc. Without
    /// this, `proton waitforexitandrun` fails with STATUS_DLL_NOT_FOUND
    /// (c0000135) because the symlinks point nowhere.
    async fn init_wineboot(&self) -> Result<()> {
        info!("Initializing Wine prefix via Proton script...");

        let proton_script = self.proton.path.join("proton");
        let env = self.build_proton_env();

        info!(
            "Running: ./proton run wineboot --init (from {})",
            self.proton.path.display()
        );

        let output = Command::new(&proton_script)
            .arg("run")
            .arg("wineboot")
            .arg("--init")
            .current_dir(&self.proton.path)
            .envs(&env)
            .output()
            .await
            .map_err(|e| WandaError::PrefixCreationFailed {
                path: self.base_path.clone(),
                reason: format!("Proton wineboot failed to start: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            error!("proton wineboot stdout: {}", stdout);
            error!("proton wineboot stderr: {}", stderr);
            return Err(WandaError::PrefixCreationFailed {
                path: self.base_path.clone(),
                reason: format!(
                    "Proton wineboot failed with exit code: {:?}",
                    output.status.code()
                ),
            });
        }

        debug!("Proton wineboot completed successfully");

        // Wait for wineserver to finish
        self.wait_wineserver().await;

        Ok(())
    }

    /// Build environment for running through the Proton script.
    ///
    /// Proton uses STEAM_COMPAT_DATA_PATH (not WINEPREFIX) and creates
    /// the actual Wine prefix at `$STEAM_COMPAT_DATA_PATH/pfx/`.
    fn build_proton_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();

        // Proton manages WINEPREFIX internally from this path
        env.insert(
            "STEAM_COMPAT_DATA_PATH".to_string(),
            self.base_path.to_string_lossy().to_string(),
        );

        // Try to find Steam root from Proton's location for Steam integration
        if let Some(steam_root) = self.find_steam_root() {
            debug!("Derived Steam root: {}", steam_root.display());
            env.insert(
                "STEAM_COMPAT_CLIENT_INSTALL_PATH".to_string(),
                steam_root.to_string_lossy().to_string(),
            );
        }

        env.insert("PROTON_NO_ESYNC".to_string(), "1".to_string());
        env.insert("PROTON_NO_FSYNC".to_string(), "1".to_string());
        env.insert("WINEDEBUG".to_string(), "-all".to_string());

        // Display variables for GUI initialization
        if let Ok(disp) = std::env::var("DISPLAY") {
            env.insert("DISPLAY".to_string(), disp);
        }
        if let Ok(wayland) = std::env::var("WAYLAND_DISPLAY") {
            env.insert("WAYLAND_DISPLAY".to_string(), wayland);
        }
        if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
            env.insert("XDG_RUNTIME_DIR".to_string(), xdg);
        }

        env
    }

    /// Walk up from Proton's path to find the Steam root directory.
    ///
    /// Proton lives under either:
    ///   `<steam>/compatibilitytools.d/<version>/`  (GE-Proton, custom)
    ///   `<steam>/steamapps/common/<version>/`      (official Proton)
    fn find_steam_root(&self) -> Option<PathBuf> {
        let mut current = self.proton.path.as_path();
        for _ in 0..5 {
            current = current.parent()?;
            // Steam root has steam.sh (native) or ubuntu12_32 (runtime)
            if current.join("steam.sh").exists()
                || current.join("ubuntu12_32").exists()
                || current.join("steamapps").exists()
            {
                return Some(current.to_path_buf());
            }
        }
        None
    }

    /// Patch mscorlib.dll PE version info so WeMod's .NET check passes.
    ///
    /// WeMod parses the VS_FIXEDFILEINFO from mscorlib.dll and requires
    /// FileVersion >= 4.7.2556.0. Wine Mono ships 4.0.30319.1 which fails.
    /// We patch dwFileVersionMS/LS and dwProductVersionMS/LS to report
    /// 4.8.9232.0 (.NET Framework 4.8). This is safe because the version
    /// resource is metadata only — Wine Mono's actual runtime is unaffected.
    pub fn patch_mscorlib_version(&self) -> Result<()> {
        // .NET 4.8 version: 4.8.9232.0
        let new_file_version_ms: u32 = (4 << 16) | 8;      // major=4, minor=8
        let new_file_version_ls: u32 = (9232 << 16) | 0;    // build=9232, revision=0

        let paths = [
            self.prefix_path.join("drive_c/windows/Microsoft.NET/Framework64/v4.0.30319/mscorlib.dll"),
            self.prefix_path.join("drive_c/windows/Microsoft.NET/Framework/v4.0.30319/mscorlib.dll"),
        ];

        let signature: [u8; 4] = 0xFEEF04BDu32.to_le_bytes();

        for path in &paths {
            if !path.exists() {
                debug!("mscorlib.dll not found at {}, skipping", path.display());
                continue;
            }

            let mut data = std::fs::read(path).map_err(|e| WandaError::PrefixCreationFailed {
                path: path.clone(),
                reason: format!("Failed to read mscorlib.dll: {}", e),
            })?;

            // Find VS_FIXEDFILEINFO signature
            let pos = data
                .windows(4)
                .position(|w| w == signature);

            let pos = match pos {
                Some(p) => p,
                None => {
                    warn!("VS_FIXEDFILEINFO signature not found in {}", path.display());
                    continue;
                }
            };

            // Verify we have enough bytes after the signature
            // Layout: signature(4) + struc_version(4) + file_ver_ms(4) + file_ver_ls(4)
            //       + product_ver_ms(4) + product_ver_ls(4)
            if pos + 24 > data.len() {
                warn!("VS_FIXEDFILEINFO too close to end of file in {}", path.display());
                continue;
            }

            // Read current version for logging
            let old_ms = u32::from_le_bytes(data[pos + 8..pos + 12].try_into().unwrap());
            let old_ls = u32::from_le_bytes(data[pos + 12..pos + 16].try_into().unwrap());
            info!(
                "Patching {} version {}.{}.{}.{} -> 4.8.9232.0",
                path.display(),
                old_ms >> 16, old_ms & 0xFFFF,
                old_ls >> 16, old_ls & 0xFFFF,
            );

            // Patch FileVersion (offset +8, +12 from signature)
            data[pos + 8..pos + 12].copy_from_slice(&new_file_version_ms.to_le_bytes());
            data[pos + 12..pos + 16].copy_from_slice(&new_file_version_ls.to_le_bytes());

            // Patch ProductVersion (offset +16, +20 from signature)
            data[pos + 16..pos + 20].copy_from_slice(&new_file_version_ms.to_le_bytes());
            data[pos + 20..pos + 24].copy_from_slice(&new_file_version_ls.to_le_bytes());

            std::fs::write(path, &data).map_err(|e| WandaError::PrefixCreationFailed {
                path: path.clone(),
                reason: format!("Failed to write patched mscorlib.dll: {}", e),
            })?;
        }

        info!("mscorlib.dll version patched for WeMod compatibility");
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

    /// Install .NET Framework 4.8 (required for WeMod's trainer engine)
    ///
    /// Downloads the official Microsoft offline installer and runs it directly
    /// via Proton — avoids winetricks which corrupts 64-bit Proton prefixes
    /// by changing the Windows version and removing Wine Mono.
    pub async fn install_dotnet(&self) -> Result<()> {
        info!("Installing .NET Framework 4.8 via direct Proton install...");

        // Download the offline installer to cache
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("wanda/downloads");
        std::fs::create_dir_all(&cache_dir)?;

        let installer_path = cache_dir.join(DOTNET48_FILENAME);

        if installer_path.exists() {
            info!("Using cached .NET installer: {}", installer_path.display());
        } else {
            info!("Downloading .NET 4.8 offline installer...");
            self.download_file(DOTNET48_URL, &installer_path).await?;
            info!("Downloaded to: {}", installer_path.display());
        }

        // Run the installer via Proton's runinprefix verb.
        // This uses Proton's wine with proper DLL overrides and library paths
        // WITHOUT changing Windows version or removing Wine Mono.
        let proton_script = self.proton.path.join("proton");
        let env = self.build_proton_env();

        info!("Running .NET installer via Proton (timeout: {:?})...", DOTNET_INSTALL_TIMEOUT);

        let install_future = Command::new(&proton_script)
            .arg("runinprefix")
            .arg(&installer_path)
            .arg("/q")        // Quiet mode
            .arg("/norestart") // Don't try to restart
            .current_dir(&self.proton.path)
            .envs(&env)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status();

        match timeout(DOTNET_INSTALL_TIMEOUT, install_future).await {
            Ok(Ok(status)) => {
                if status.success() {
                    info!(".NET 4.8 installed successfully");
                } else {
                    let code = status.code();
                    // .NET installer exit codes:
                    // 0 = success, 1602 = user cancelled, 1603 = fatal error,
                    // 1641/3010 = success (reboot needed), 5100 = prerequisites
                    match code {
                        Some(1641) | Some(3010) => {
                            info!(".NET 4.8 installed (reboot requested but not needed under Wine)");
                        }
                        _ => {
                            warn!(".NET installer exited with code: {:?}", code);
                            return Err(WandaError::WinetricksFailed {
                                reason: format!(".NET 4.8 installer failed with exit code {:?}", code),
                            });
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                return Err(WandaError::WinetricksFailed {
                    reason: format!("Failed to run .NET installer: {}", e),
                });
            }
            Err(_) => {
                warn!(".NET installer timed out after {:?}", DOTNET_INSTALL_TIMEOUT);
                return Err(WandaError::WinetricksFailed {
                    reason: format!(".NET installer timed out after {:?}", DOTNET_INSTALL_TIMEOUT),
                });
            }
        }

        // Wait for wineserver to finish
        self.wait_wineserver().await;

        Ok(())
    }

    /// Download a file from a URL to a local path
    async fn download_file(&self, url: &str, dest: &Path) -> Result<()> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| WandaError::WinetricksFailed {
                reason: format!("Failed to create HTTP client: {}", e),
            })?;

        let response = client.get(url).send().await.map_err(|e| {
            WandaError::WinetricksFailed {
                reason: format!("Failed to download {}: {}", url, e),
            }
        })?;

        if !response.status().is_success() {
            return Err(WandaError::WinetricksFailed {
                reason: format!("Download failed with HTTP {}", response.status()),
            });
        }

        let total_size = response.content_length().unwrap_or(0);
        info!("Downloading {} ({:.1} MB)...", DOTNET48_FILENAME, total_size as f64 / 1_048_576.0);

        let tmp_path = dest.with_extension("tmp");
        let mut file = tokio::fs::File::create(&tmp_path).await.map_err(|e| {
            WandaError::WinetricksFailed {
                reason: format!("Failed to create file: {}", e),
            }
        })?;

        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| WandaError::WinetricksFailed {
                reason: format!("Download interrupted: {}", e),
            })?;
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                .await
                .map_err(|e| WandaError::WinetricksFailed {
                    reason: format!("Failed to write file: {}", e),
                })?;
        }

        // Atomic rename
        tokio::fs::rename(&tmp_path, dest).await.map_err(|e| {
            WandaError::WinetricksFailed {
                reason: format!("Failed to move downloaded file: {}", e),
            }
        })?;

        Ok(())
    }

    /// Install additional dependencies
    async fn install_dependencies(&self) -> Result<()> {
        info!("Installing additional dependencies...");

        // Install common dependencies WeMod needs
        // - vcrun2019: Visual C++ runtime
        // - corefonts: Windows fonts
        // - winhttp/wininet: Network components for WeMod authentication
        let deps = ["vcrun2019", "corefonts", "winhttp", "wininet"];

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
                .arg("-q")      // Unattended mode - no GUI dialogs
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

        assert_eq!(env.get("WINEPREFIX"), Some(&"/tmp/test/pfx".to_string()));
        assert_eq!(env.get("WINEARCH"), Some(&"win64".to_string()));
    }
}
