//! Game launching with WeMod
//!
//! Handles launching Steam games alongside WeMod through Proton.
//!
//! Key insight: WeMod and the game MUST run in the same Wine prefix
//! for WeMod to be able to hook into the game process. We achieve this by
//! running both from wanda's prefix (which has .NET and WeMod installed).

use crate::error::{Result, WandaError};
use crate::prefix::WandaPrefix;
use crate::steam::{ProtonVersion, SteamApp, SteamInstallation};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};
use tracing::info;

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
                Ok(Some(_)) => return false,
                Ok(None) => return true,
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
    steam: &'a SteamInstallation,
    prefix: &'a WandaPrefix,
    proton: &'a ProtonVersion,
}

impl<'a> GameLauncher<'a> {
    /// Create a new game launcher
    pub fn new(
        steam: &'a SteamInstallation,
        prefix: &'a WandaPrefix,
        proton: &'a ProtonVersion,
    ) -> Self {
        Self { steam, prefix, proton }
    }

    /// Build environment variables for launching
    fn build_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();

        env.insert(
            "WINEPREFIX".to_string(),
            self.prefix.pfx_path().to_string_lossy().to_string(),
        );
        env.insert(
            "STEAM_COMPAT_DATA_PATH".to_string(),
            self.prefix.path.to_string_lossy().to_string(),
        );
        env.insert(
            "STEAM_COMPAT_CLIENT_INSTALL_PATH".to_string(),
            self.steam.root_path.to_string_lossy().to_string(),
        );
        env.insert("PROTON_NO_ESYNC".to_string(), "1".to_string());
        env.insert("PROTON_NO_FSYNC".to_string(), "1".to_string());
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
            .ok_or(WandaError::GameNotFound { app_id: config.app_id })?;

        info!(
            "Launching {} (AppID: {}) {}",
            game.name,
            game.app_id,
            if config.with_wemod { "with WeMod" } else { "without WeMod" }
        );

        info!("Using WANDA prefix: {}", self.prefix.path.display());

        // Kill stale wineservers to avoid conflicts
        info!("Cleaning up stale wineservers...");
        let _ = Command::new("pkill").args(["-9", "wineserver"]).output().await;
        tokio::time::sleep(Duration::from_secs(1)).await;

        let mut env = self.build_env();
        env.insert("SteamAppId".to_string(), config.app_id.to_string());
        env.insert("SteamGameId".to_string(), config.app_id.to_string());
        env.extend(config.extra_env);

        let wine = self.wine_exe();
        let mut wemod_process = None;

        if config.with_wemod {
            let wemod_exe = self.prefix.wemod_exe();
            if !wemod_exe.exists() {
                return Err(WandaError::WemodNotInstalled);
            }

            info!("Starting WeMod...");
            info!("WeMod path: {}", wemod_exe.display());

            let child = Command::new(&wine)
                .arg(&wemod_exe)
                .arg("--no-sandbox")
                .envs(&env)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|e| WandaError::LaunchFailed {
                    reason: format!("Failed to start WeMod: {}", e),
                })?;

            wemod_process = Some(child);

            info!("Waiting {} seconds for WeMod to initialize...", config.wemod_delay);
            tokio::time::sleep(Duration::from_secs(config.wemod_delay)).await;
        }

        info!("Launching game via Proton...");
        self.launch_game_via_proton(&game, &config.extra_args, &env).await?;

        Ok(LaunchHandle {
            wemod_process,
            app_id: config.app_id,
            start_time: std::time::Instant::now(),
        })
    }

    /// Launch game via Proton
    async fn launch_game_via_proton(
        &self,
        game: &SteamApp,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<()> {
        let proton_exe = self.proton.proton_exe();
        if !proton_exe.exists() {
            return Err(WandaError::ProtonNotFound);
        }

        let game_exe = self.find_game_executable(game)?;
        info!("Game executable: {}", game_exe.display());

        let mut cmd = Command::new(&proton_exe);
        cmd.arg("run").arg(&game_exe).args(args).envs(env);
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());

        info!("Running: {} run {}", proton_exe.display(), game_exe.display());

        cmd.spawn().map_err(|e| WandaError::LaunchFailed {
            reason: format!("Failed to start game: {}", e),
        })?;

        Ok(())
    }

    /// Find the main executable for a game
    fn find_game_executable(&self, game: &SteamApp) -> Result<PathBuf> {
        let install_path = &game.install_path;

        if !install_path.exists() {
            return Err(WandaError::GameNotFound { app_id: game.app_id });
        }

        let patterns = [
            format!("{}.exe", game.install_dir),
            "game.exe".to_string(),
            "start.exe".to_string(),
            "launcher.exe".to_string(),
        ];

        for pattern in &patterns {
            let exe_path = install_path.join(pattern);
            if exe_path.exists() {
                return Ok(exe_path);
            }
        }

        for entry in walkdir::WalkDir::new(install_path)
            .max_depth(2)
            .into_iter()
            .flatten()
        {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "exe" {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
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
