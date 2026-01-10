//! Game launching with WeMod
//!
//! Handles launching Steam games alongside WeMod through Proton.

use crate::error::{Result, WandaError};
use crate::prefix::WandaPrefix;
use crate::steam::{ProtonVersion, SteamApp, SteamInstallation};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

/// Configuration for launching a game
#[derive(Debug, Clone)]
pub struct LaunchConfig {
    /// The game to launch
    pub app_id: u32,
    /// Whether to launch with WeMod
    pub with_wemod: bool,
    /// Additional command-line arguments for the game
    pub extra_args: Vec<String>,
    /// Additional environment variables
    pub extra_env: HashMap<String, String>,
    /// Delay between starting WeMod and the game (in seconds)
    pub wemod_delay: u64,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            app_id: 0,
            with_wemod: true,
            extra_args: Vec::new(),
            extra_env: HashMap::new(),
            wemod_delay: 3,
        }
    }
}

/// Handle to a launched game session
pub struct LaunchHandle {
    /// WeMod process (if launched with WeMod)
    wemod_process: Option<Child>,
    /// Game App ID
    pub app_id: u32,
    /// Start time
    start_time: std::time::Instant,
}

impl LaunchHandle {
    /// Check if the session is still running
    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut wemod) = self.wemod_process {
            match wemod.try_wait() {
                Ok(Some(_)) => return false, // WeMod exited
                Ok(None) => return true,     // Still running
                Err(_) => return false,
            }
        }
        false
    }

    /// Get elapsed time since launch
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Terminate the session
    pub async fn terminate(&mut self) -> Result<()> {
        if let Some(ref mut wemod) = self.wemod_process {
            let _ = wemod.kill().await;
        }
        Ok(())
    }

    /// Wait for WeMod to exit
    pub async fn wait(&mut self) -> Result<()> {
        if let Some(ref mut wemod) = self.wemod_process {
            let _ = wemod.wait().await;
        }
        Ok(())
    }
}

/// Game launcher that coordinates WeMod and game startup
pub struct GameLauncher<'a> {
    /// Steam installation
    steam: &'a SteamInstallation,
    /// WANDA prefix with WeMod
    prefix: &'a WandaPrefix,
    /// Proton version to use
    proton: &'a ProtonVersion,
}

impl<'a> GameLauncher<'a> {
    /// Create a new game launcher
    pub fn new(
        steam: &'a SteamInstallation,
        prefix: &'a WandaPrefix,
        proton: &'a ProtonVersion,
    ) -> Self {
        Self {
            steam,
            prefix,
            proton,
        }
    }

    /// Build environment variables for launching
    fn build_env(&self, game: &SteamApp) -> HashMap<String, String> {
        let mut env = HashMap::new();

        // Wine prefix (WANDA's prefix for WeMod)
        env.insert(
            "WINEPREFIX".to_string(),
            self.prefix.path.to_string_lossy().to_string(),
        );

        // Steam compatibility data path (for the game's own prefix if needed)
        if let Some(ref compat_path) = game.compat_data_path {
            env.insert(
                "STEAM_COMPAT_DATA_PATH".to_string(),
                compat_path.to_string_lossy().to_string(),
            );
        }

        // Steam client install path
        env.insert(
            "STEAM_COMPAT_CLIENT_INSTALL_PATH".to_string(),
            self.steam.root_path.to_string_lossy().to_string(),
        );

        // Proton flags that may help stability
        env.insert("PROTON_NO_ESYNC".to_string(), "1".to_string());
        env.insert("PROTON_NO_FSYNC".to_string(), "1".to_string());

        // Reduce Wine debug noise
        env.insert("WINEDEBUG".to_string(), "-all".to_string());

        env
    }

    /// Get the Wine executable path
    fn wine_exe(&self) -> PathBuf {
        let proton_wine = self.proton.wine_exe();
        if proton_wine.exists() {
            proton_wine
        } else {
            PathBuf::from("wine")
        }
    }

    /// Launch a game with WeMod
    pub async fn launch(&self, config: LaunchConfig) -> Result<LaunchHandle> {
        let game = self
            .steam
            .find_game(config.app_id)
            .ok_or(WandaError::GameNotFound {
                app_id: config.app_id,
            })?;

        info!(
            "Launching {} (AppID: {}) {}",
            game.name,
            game.app_id,
            if config.with_wemod {
                "with WeMod"
            } else {
                "without WeMod"
            }
        );

        let mut env = self.build_env(game);
        env.extend(config.extra_env);

        let wine = self.wine_exe();
        let mut wemod_process = None;

        // Start WeMod first if requested
        if config.with_wemod {
            let wemod_exe = self.prefix.wemod_exe();
            if !wemod_exe.exists() {
                return Err(WandaError::WemodNotInstalled);
            }

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

            wemod_process = Some(child);

            // Wait for WeMod to initialize
            info!(
                "Waiting {} seconds for WeMod to initialize...",
                config.wemod_delay
            );
            tokio::time::sleep(Duration::from_secs(config.wemod_delay)).await;
        }

        // Launch the game via Steam
        // Using steam:// URL protocol ensures Steam handles Proton setup correctly
        info!("Launching game via Steam...");
        self.launch_via_steam(config.app_id).await?;

        Ok(LaunchHandle {
            wemod_process,
            app_id: config.app_id,
            start_time: std::time::Instant::now(),
        })
    }

    /// Launch a game via Steam's URL protocol
    async fn launch_via_steam(&self, app_id: u32) -> Result<()> {
        // Use xdg-open to launch via steam:// protocol
        // This ensures Steam handles all the Proton setup correctly
        let steam_url = format!("steam://rungameid/{}", app_id);

        let status = Command::new("xdg-open")
            .arg(&steam_url)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|e| WandaError::LaunchFailed {
                reason: format!("Failed to launch Steam URL: {}", e),
            })?;

        if !status.success() {
            // Try alternative: steam command directly
            let status = Command::new("steam")
                .arg(&steam_url)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map_err(|e| WandaError::LaunchFailed {
                    reason: format!("Failed to launch via steam command: {}", e),
                })?;

            if !status.success() {
                warn!("Steam launch returned non-zero, game may still start");
            }
        }

        debug!("Game launch initiated via Steam");
        Ok(())
    }

    /// Launch a game directly via Proton (without Steam)
    /// This is an alternative method that gives more control
    #[allow(dead_code)]
    async fn launch_directly(&self, game: &SteamApp, args: &[String]) -> Result<Child> {
        let proton_exe = self.proton.proton_exe();

        if !proton_exe.exists() {
            return Err(WandaError::ProtonNotFound);
        }

        // Find the game executable
        let game_exe = self.find_game_executable(game)?;

        let mut env = self.build_env(game);

        // Set up Proton environment
        env.insert("STEAM_COMPAT_DATA_PATH".to_string(),
            game.compat_data_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| {
                    self.steam.root_path
                        .join("steamapps/compatdata")
                        .join(game.app_id.to_string())
                        .to_string_lossy()
                        .to_string()
                })
        );

        let mut cmd = Command::new(&proton_exe);
        cmd.arg("run").arg(&game_exe);
        cmd.args(args);
        cmd.envs(&env);
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        let child = cmd.spawn().map_err(|e| WandaError::LaunchFailed {
            reason: format!("Failed to start game: {}", e),
        })?;

        Ok(child)
    }

    /// Try to find the main executable for a game
    fn find_game_executable(&self, game: &SteamApp) -> Result<PathBuf> {
        let install_path = &game.install_path;

        if !install_path.exists() {
            return Err(WandaError::GameNotFound {
                app_id: game.app_id,
            });
        }

        // Common executable patterns
        let patterns = [
            format!("{}.exe", game.install_dir),
            "game.exe".to_string(),
            "start.exe".to_string(),
            "launcher.exe".to_string(),
        ];

        // Try to find a matching executable
        for pattern in &patterns {
            let exe_path = install_path.join(pattern);
            if exe_path.exists() {
                return Ok(exe_path);
            }
        }

        // Walk the directory looking for .exe files
        for entry in walkdir::WalkDir::new(install_path)
            .max_depth(2)
            .into_iter()
            .flatten()
        {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "exe" {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    // Skip common non-game executables
                    if !name.to_lowercase().contains("unins")
                        && !name.to_lowercase().contains("redist")
                        && !name.to_lowercase().contains("setup")
                    {
                        return Ok(path.to_path_buf());
                    }
                }
            }
        }

        Err(WandaError::LaunchFailed {
            reason: format!("Could not find executable for {}", game.name),
        })
    }
}
