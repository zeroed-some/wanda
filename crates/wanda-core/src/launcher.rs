//! Game launching with WeMod
//!
//! Handles launching Steam games alongside WeMod through Proton.
//!
//! Key insight: WeMod and the game MUST run in the same Wine prefix
//! for WeMod to be able to hook into the game process. We achieve this by
//! running both in a single Proton session using a batch file coordinator.

use crate::error::{Result, WandaError};
use crate::prefix::WandaPrefix;
use crate::steam::{ProtonVersion, SteamApp, SteamInstallation};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};
use tracing::{debug, info};

/// Get the log file path
fn log_file_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wanda")
        .join("wanda.log")
}

/// Wrap a shell command so its combined stdout+stderr is written to both the
/// terminal and `wanda.log`. This captures Wine/DXVK errors, crash backtraces,
/// and game/WeMod output that would otherwise only flash past in the terminal,
/// so a failed launch leaves a diagnosable trail in the log file.
fn tee_to_log(cmd: &str) -> String {
    let log_path = log_file_path();
    // Single-quote the path for the shell; escape any embedded single quotes.
    let quoted = format!("'{}'", log_path.to_string_lossy().replace('\'', r"'\''"));
    // Group the command so the pipe applies to the whole thing. `tee -a`
    // appends; stderr is merged into stdout first.
    format!("{{ {cmd} ; }} 2>&1 | tee -a {quoted}")
}

/// Log a message to file
fn log_to_file(msg: &str) {
    let log_path = log_file_path();
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "[{}] {}", timestamp, msg);
    }
}

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
    /// Standalone mode: launch WeMod only, user starts game from WeMod UI
    pub standalone: bool,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            app_id: 0,
            with_wemod: true,
            extra_args: Vec::new(),
            extra_env: HashMap::new(),
            wemod_delay: 3,
            standalone: false,
        }
    }
}

/// Handle to a launched game session
pub struct LaunchHandle {
    /// The main process (proton running the batch file)
    process: Option<Child>,
    /// Game App ID
    pub app_id: u32,
    /// Start time
    start_time: std::time::Instant,
}

impl LaunchHandle {
    /// Check if the session is still running
    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut proc) = self.process {
            match proc.try_wait() {
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
        if let Some(ref mut proc) = self.process {
            let _ = proc.kill().await;
        }
        Ok(())
    }

    /// Wait for the session to exit
    pub async fn wait(&mut self) -> Result<()> {
        if let Some(ref mut proc) = self.process {
            let _ = proc.wait().await;
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
    fn build_env(&self, app_id: Option<u32>) -> HashMap<String, String> {
        let mut env = HashMap::new();

        // Primary Proton/Wine environment
        env.insert(
            "STEAM_COMPAT_DATA_PATH".to_string(),
            self.prefix.path.to_string_lossy().to_string(),
        );
        env.insert(
            "STEAM_COMPAT_CLIENT_INSTALL_PATH".to_string(),
            self.steam.root_path.to_string_lossy().to_string(),
        );

        // Add Steam app ID for proper Steam API initialization
        if let Some(id) = app_id {
            env.insert("SteamAppId".to_string(), id.to_string());
            env.insert("SteamGameId".to_string(), id.to_string());
            env.insert("STEAM_COMPAT_APP_ID".to_string(), id.to_string());
        }

        // Pass display variables for GUI apps (Wayland/X11)
        if let Ok(disp) = std::env::var("DISPLAY") {
            debug!("Passing DISPLAY={}", disp);
            env.insert("DISPLAY".to_string(), disp);
        }
        if let Ok(wayland) = std::env::var("WAYLAND_DISPLAY") {
            debug!("Passing WAYLAND_DISPLAY={}", wayland);
            env.insert("WAYLAND_DISPLAY".to_string(), wayland);
        }
        if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
            debug!("Passing XDG_RUNTIME_DIR={}", xdg);
            env.insert("XDG_RUNTIME_DIR".to_string(), xdg);
        }
        // Pass XAUTHORITY for X11 authentication
        if let Ok(xauth) = std::env::var("XAUTHORITY") {
            debug!("Passing XAUTHORITY={}", xauth);
            env.insert("XAUTHORITY".to_string(), xauth);
        }
        // Disable NTSync to avoid "Maximum number of clients reached" errors
        env.insert("WINEESYNC".to_string(), "0".to_string());
        env.insert("WINEFSYNC".to_string(), "0".to_string());
        env.insert("PROTON_NO_ESYNC".to_string(), "1".to_string());
        env.insert("PROTON_NO_FSYNC".to_string(), "1".to_string());

        // Proton library paths for proper dependency resolution
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

        env
    }

    /// Convert a Linux path to a Wine path (Z: drive)
    /// Uses forward slashes which Wine accepts and avoids bash escaping issues
    fn to_wine_path(path: &std::path::Path) -> String {
        format!("Z:{}", path.to_string_lossy())
    }

    /// Convert a game executable's Linux path to the corresponding C: drive
    /// path through the Steam library symlink inside the prefix.
    ///
    /// WeMod detects game processes by matching `QueryFullProcessImageName`
    /// against the game's expected install directory. If we launch the game
    /// via its `Z:\...` Linux path, the imagePath won't match WeMod's
    /// `C:\Program Files (x86)\Steam\steamapps\common\...` expectation.
    /// Launching through the symlinked C: path makes the paths match.
    fn to_prefix_game_path(game: &SteamApp, game_exe: &std::path::Path) -> Option<String> {
        let common_dir = game.library_path.join("steamapps/common");
        let rel_path = game_exe.strip_prefix(&common_dir).ok()?;
        Some(format!(
            "C:\\Program Files (x86)\\Steam\\steamapps\\common\\{}",
            rel_path.to_string_lossy().replace('/', "\\")
        ))
    }

    /// Create a batch file that coordinates WeMod and game startup
    fn create_launch_batch(
        &self,
        wemod_exe: &std::path::Path,
        game: &SteamApp,
        game_exe: &std::path::Path,
        game_args: &[String],
        delay_seconds: u64,
    ) -> Result<PathBuf> {
        let batch_path = self.prefix.path.join("wanda_launch.bat");

        // For batch file internal paths, use backslashes (Windows convention)
        let wemod_win_path = format!("Z:{}", wemod_exe.to_string_lossy().replace('/', "\\"));
        // Use C: drive path through the Steam library symlink so that
        // QueryFullProcessImageName returns a path WeMod recognizes
        let game_win_path = Self::to_prefix_game_path(game, game_exe)
            .unwrap_or_else(|| format!("Z:{}", game_exe.to_string_lossy().replace('/', "\\")));
        let game_args_str = game_args.join(" ");

        // Create batch file content
        // Uses Windows batch syntax to:
        // 1. Start WeMod in the background
        // 2. Wait for WeMod to initialize (using ping for delay)
        // 3. Start the game and wait for it to exit
        // 4. Kill WeMod when game exits
        let batch_content = format!(
r#"@echo off
@title WANDA Launcher

echo WANDA Launcher starting...
echo.

REM WeMod executable path
SET wemodpath={wemod_path}
SET wemodname=WeMod.exe

echo Starting WeMod: %wemodpath%
start "" "%wemodpath%" --no-sandbox

echo Waiting {delay} seconds for WeMod to initialize...
ping localhost -n {ping_count} > NUL 2>&1

echo.
echo Starting game: {game_path}
echo Arguments: {game_args}
echo.

REM Start the game and wait for it to exit
start /wait "" "{game_path}" {game_args}

echo.
echo Game exited, closing WeMod...

REM Find and kill WeMod process
for /F "TOKENS=1,2,*" %%a in ('C:/windows/system32/tasklist /FI "IMAGENAME eq %wemodname%"') do (
    set wemodPID=%%b
)
if defined wemodPID (
    C:/windows/system32/taskkill.exe /PID %wemodPID% /F 2>NUL
    echo Closed WeMod
)

echo.
echo WANDA Launcher finished
"#,
            wemod_path = wemod_win_path,
            delay = delay_seconds,
            ping_count = delay_seconds + 1, // ping -n N waits N-1 seconds
            game_path = game_win_path,
            game_args = game_args_str,
        );

        // Windows cmd.exe requires CRLF line endings
        let batch_content = batch_content.replace('\n', "\r\n");
        std::fs::write(&batch_path, batch_content).map_err(|e| WandaError::LaunchFailed {
            reason: format!("Failed to create launch batch file: {}", e),
        })?;

        log_to_file(&format!("Created batch file at: {}", batch_path.display()));
        debug!("Created batch file at: {}", batch_path.display());

        Ok(batch_path)
    }

    /// Create a batch file for standalone mode (WeMod only, no game)
    ///
    /// Launches WeMod via the Squirrel stub (identical to normal mode) then
    /// keeps cmd.exe alive with an infinite ping loop so Wine doesn't tear
    /// down the session. The user closes WeMod from its UI and Ctrl+C's the
    /// terminal to end the Proton session.
    fn create_standalone_batch(&self, wemod_exe: &std::path::Path) -> Result<PathBuf> {
        let batch_path = self.prefix.path.join("wanda_standalone.bat");
        let wemod_win_path = format!("Z:{}", wemod_exe.to_string_lossy().replace('/', "\\"));

        let batch_content = format!(
r#"@echo off
@title WANDA Standalone Launcher

echo WANDA Standalone Launcher starting...
echo.

SET wemodpath={wemod_path}

echo Starting WeMod: %wemodpath%
start "" "%wemodpath%" --no-sandbox

echo Waiting for WeMod to initialize...
ping localhost -n 6 > NUL 2>&1

echo.
echo WeMod is running in standalone mode.
echo Launch your game from the WeMod UI.
echo.

:wait
ping localhost -n 30 > NUL 2>&1
goto wait
"#,
            wemod_path = wemod_win_path,
        );

        // Windows cmd.exe requires CRLF line endings
        let batch_content = batch_content.replace('\n', "\r\n");
        std::fs::write(&batch_path, batch_content).map_err(|e| WandaError::LaunchFailed {
            reason: format!("Failed to create standalone batch file: {}", e),
        })?;

        log_to_file(&format!("Created standalone batch at: {}", batch_path.display()));
        debug!("Created standalone batch at: {}", batch_path.display());

        Ok(batch_path)
    }

    /// Set up Steam library structure inside the prefix so WeMod can find games
    ///
    /// WeMod locates games via the Windows registry path
    /// `C:\Program Files (x86)\Steam` -> `steamapps\common\{game}\`. We create
    /// symlinks from the prefix into the real Steam library so WeMod sees the
    /// game without copying any files.
    fn setup_steam_library(&self, game: &SteamApp) -> Result<()> {
        let steamapps_dir = self.prefix.path
            .join("pfx/drive_c/Program Files (x86)/Steam/steamapps");

        // Create the steamapps directory structure
        std::fs::create_dir_all(&steamapps_dir).map_err(|e| WandaError::LaunchFailed {
            reason: format!("Failed to create Steam library structure in prefix: {}", e),
        })?;

        // Symlink common/ directory
        let common_link = steamapps_dir.join("common");
        let common_target = game.library_path.join("steamapps/common");
        if !common_link.exists() {
            info!("Symlinking Steam common: {} -> {}", common_link.display(), common_target.display());
            log_to_file(&format!("Symlinking common: {} -> {}", common_link.display(), common_target.display()));
            std::os::unix::fs::symlink(&common_target, &common_link).map_err(|e| {
                WandaError::LaunchFailed {
                    reason: format!(
                        "Failed to symlink Steam common directory: {} -> {}: {}",
                        common_link.display(), common_target.display(), e
                    ),
                }
            })?;
        }

        // Symlink the game's appmanifest
        let manifest_name = format!("appmanifest_{}.acf", game.app_id);
        let manifest_link = steamapps_dir.join(&manifest_name);
        let manifest_target = game.library_path.join("steamapps").join(&manifest_name);
        if !manifest_link.exists() && manifest_target.exists() {
            info!("Symlinking appmanifest: {} -> {}", manifest_link.display(), manifest_target.display());
            log_to_file(&format!("Symlinking appmanifest: {} -> {}", manifest_link.display(), manifest_target.display()));
            std::os::unix::fs::symlink(&manifest_target, &manifest_link).map_err(|e| {
                WandaError::LaunchFailed {
                    reason: format!(
                        "Failed to symlink appmanifest: {} -> {}: {}",
                        manifest_link.display(), manifest_target.display(), e
                    ),
                }
            })?;
        }

        // Create libraryfolders.vdf so WeMod can discover the game library
        let vdf_path = steamapps_dir.join("libraryfolders.vdf");
        let vdf_content = format!(
            r#""libraryfolders"
{{
	"0"
	{{
		"path"		"C:\\Program Files (x86)\\Steam"
		"apps"
		{{
			"{app_id}"		"0"
		}}
	}}
}}"#,
            app_id = game.app_id,
        );
        info!("Writing libraryfolders.vdf with app {}", game.app_id);
        log_to_file(&format!("Writing libraryfolders.vdf for app {}", game.app_id));
        std::fs::write(&vdf_path, &vdf_content).map_err(|e| WandaError::LaunchFailed {
            reason: format!("Failed to create libraryfolders.vdf: {}", e),
        })?;

        // Add Steam registry entries so WeMod can find the fake Steam installation
        self.setup_steam_registry(game)?;

        Ok(())
    }

    /// Write Steam registry entries into the Wine prefix so WeMod can locate Steam.
    ///
    /// WeMod checks `HKCU\Software\Valve\Steam\SteamPath` to find the Steam
    /// installation directory and then reads `steamapps/libraryfolders.vdf`.
    fn setup_steam_registry(&self, game: &SteamApp) -> Result<()> {
        let user_reg = self.prefix.path.join("pfx/user.reg");
        if !user_reg.exists() {
            debug!("user.reg not found, skipping Steam registry setup");
            return Ok(());
        }

        let content = std::fs::read_to_string(&user_reg).map_err(|e| WandaError::LaunchFailed {
            reason: format!("Failed to read user.reg: {}", e),
        })?;

        // Only add if Valve\Steam key doesn't already exist
        if content.contains("[Software\\\\Valve\\\\Steam]") {
            debug!("Steam registry entries already exist");
            return Ok(());
        }

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let steam_entry = format!(
            r#"

[Software\\Valve\\Steam] {ts}
"Language"="english"
"SteamExe"="C:\\\\Program Files (x86)\\\\Steam\\\\steam.exe"
"SteamPath"="C:\\\\Program Files (x86)\\\\Steam"

[Software\\Valve\\Steam\\Apps\\{app_id}] {ts}
"Installed"=dword:00000001
"Name"="{game_name}"
"Running"=dword:00000000
"#,
            ts = timestamp,
            app_id = game.app_id,
            game_name = game.name.replace('\\', "\\\\").replace('"', ""),
        );

        info!("Adding Steam registry entries for app {}", game.app_id);
        log_to_file(&format!("Adding Steam registry entries for app {}", game.app_id));

        let mut file = OpenOptions::new()
            .append(true)
            .open(&user_reg)
            .map_err(|e| WandaError::LaunchFailed {
                reason: format!("Failed to open user.reg for writing: {}", e),
            })?;
        file.write_all(steam_entry.as_bytes()).map_err(|e| WandaError::LaunchFailed {
            reason: format!("Failed to write Steam registry entries: {}", e),
        })?;

        Ok(())
    }

    /// Set up a Steam launch proxy so that when WeMod clicks "Play", the game
    /// launches directly in the same Wine session instead of going through the
    /// native Linux Steam client (which creates a separate Proton session that
    /// WeMod cannot hook into).
    ///
    /// This works by:
    /// 1. Writing a batch file at `C:\wanda\steam_proxy.bat` that launches
    ///    the game executable directly
    /// 2. Overriding the `steam://` protocol handler in the Wine registry
    ///    to point to this batch file instead of Proton's `steam.exe` stub
    ///
    /// When WeMod calls `steam://run/<APPID>`, Wine routes it through our
    /// proxy, which starts the game process in WeMod's Wine session.
    fn setup_steam_launch_proxy(
        &self,
        game: &SteamApp,
        game_exe: &std::path::Path,
    ) -> Result<()> {
        let pfx_path = self.prefix.path.join("pfx");

        // Create the proxy batch file at C:\wanda\steam_proxy.bat
        let wanda_dir = pfx_path.join("drive_c/wanda");
        std::fs::create_dir_all(&wanda_dir).map_err(|e| WandaError::LaunchFailed {
            reason: format!("Failed to create wanda directory in prefix: {}", e),
        })?;

        let proxy_path = wanda_dir.join("steam_proxy.bat");
        // Use C: drive path so QueryFullProcessImageName returns a path
        // that matches WeMod's expected Steam library location
        let game_win_path = Self::to_prefix_game_path(game, game_exe)
            .unwrap_or_else(|| format!("Z:{}", game_exe.to_string_lossy().replace('/', "\\")));

        let proxy_content = format!(
            r#"@echo off
REM Wanda Steam Protocol Proxy
REM Intercepts steam://run requests from WeMod and launches the game
REM directly in the same Wine session (instead of via native Steam
REM which would create a separate Proton session).
REM
REM Game: {game_name} (AppID: {app_id})

echo [WANDA] Steam launch intercepted: %~1
echo [WANDA] Launching: {game_path}

start "" "{game_path}"
"#,
            game_name = game.name,
            app_id = game.app_id,
            game_path = game_win_path,
        );

        std::fs::write(&proxy_path, &proxy_content).map_err(|e| WandaError::LaunchFailed {
            reason: format!("Failed to create Steam proxy batch: {}", e),
        })?;

        info!("Created Steam launch proxy at {}", proxy_path.display());
        log_to_file(&format!(
            "Created Steam launch proxy for {} ({})",
            game.name, game.app_id
        ));

        // Override the steam:// protocol handler in user.reg so Wine routes
        // steam://run/APPID to our proxy batch instead of Proton's steam.exe.
        // Later entries in user.reg take precedence, so we always append.
        let user_reg = self.prefix.path.join("pfx/user.reg");
        if !user_reg.exists() {
            debug!("user.reg not found, skipping steam:// protocol override");
            return Ok(());
        }

        let content = std::fs::read_to_string(&user_reg).map_err(|e| WandaError::LaunchFailed {
            reason: format!("Failed to read user.reg: {}", e),
        })?;

        // Skip if already configured (avoid growing user.reg on repeated launches)
        if content.contains("wanda\\\\steam_proxy.bat") {
            debug!("Steam protocol proxy already configured in registry");
            return Ok(());
        }

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Wine user.reg format: \\ = literal backslash, \" = literal quote
        let registry_override = format!(
            concat!(
                "\n",
                "[Software\\\\Classes\\\\steam] {ts}\n",
                "@=\"URL:Steam Protocol\"\n",
                "\"URL Protocol\"=\"\"\n",
                "\n",
                "[Software\\\\Classes\\\\steam\\\\shell\\\\open\\\\command] {ts}\n",
                "@=\"\\\"C:\\\\wanda\\\\steam_proxy.bat\\\" \\\"%1\\\"\"\n",
            ),
            ts = timestamp,
        );

        let mut file = OpenOptions::new()
            .append(true)
            .open(&user_reg)
            .map_err(|e| WandaError::LaunchFailed {
                reason: format!("Failed to open user.reg: {}", e),
            })?;
        file.write_all(registry_override.as_bytes())
            .map_err(|e| WandaError::LaunchFailed {
                reason: format!("Failed to write steam:// protocol override: {}", e),
            })?;

        info!("Overrode steam:// protocol handler to use Wanda proxy");
        log_to_file("Overrode steam:// protocol handler → C:\\wanda\\steam_proxy.bat");

        Ok(())
    }

    /// Launch a game with WeMod
    pub async fn launch(&self, config: LaunchConfig) -> Result<LaunchHandle> {
        log_to_file("=== WANDA LAUNCH START ===");

        let game = self
            .steam
            .find_game(config.app_id)
            .ok_or(WandaError::GameNotFound { app_id: config.app_id })?;

        let launch_msg = format!(
            "Launching {} (AppID: {}) {}",
            game.name,
            game.app_id,
            if config.with_wemod { "with WeMod" } else { "without WeMod" }
        );
        info!("{}", launch_msg);
        log_to_file(&launch_msg);

        info!("Using WANDA prefix: {}", self.prefix.path.display());
        log_to_file(&format!("WANDA prefix: {}", self.prefix.path.display()));
        log_to_file(&format!("Proton: {} at {}", self.proton.name, self.proton.path.display()));

        let game_exe = self.find_game_executable(&game)?;
        info!("Game executable: {}", game_exe.display());
        log_to_file(&format!("Game executable: {}", game_exe.display()));

        // Always include SteamAppId — the game needs it for Steam API
        // initialization. Without it, many games exit immediately.
        // In standalone CLI mode this is safe since Steam tracks games
        // via reaper/launch-wrapper, not env vars.
        let mut env = self.build_env(Some(config.app_id));

        // Set STEAM_COMPAT_INSTALL_PATH so ProtonFixes knows where the game is
        env.insert(
            "STEAM_COMPAT_INSTALL_PATH".to_string(),
            game.install_path.to_string_lossy().to_string(),
        );
        debug!("Setting STEAM_COMPAT_INSTALL_PATH={}", game.install_path.display());

        env.extend(config.extra_env);

        let process = if config.with_wemod {
            let wemod_stub = self.prefix.wemod_exe();
            if !wemod_stub.exists() {
                log_to_file(&format!("WeMod not found at: {}", wemod_stub.display()));
                return Err(WandaError::WemodNotInstalled);
            }

            // Prefer the real Electron exe (app-X.Y.Z/WeMod.exe) over the Squirrel stub.
            // The stub exits immediately after spawning the real app, which can cause
            // Proton to tear down the Wine session before WeMod fully starts.
            let wemod_exe = if let Some(app_exe) = self.prefix.wemod_app_exe() {
                info!("Using real WeMod app exe: {}", app_exe.display());
                log_to_file(&format!("Using real WeMod app exe: {}", app_exe.display()));
                app_exe
            } else {
                info!("Real WeMod app exe not found, falling back to stub: {}", wemod_stub.display());
                log_to_file(&format!("Real WeMod app exe not found, falling back to stub: {}", wemod_stub.display()));
                wemod_stub
            };

            info!("WeMod path: {}", wemod_exe.display());
            log_to_file(&format!("WeMod path: {}", wemod_exe.display()));

            // Set up Steam library so WeMod can find the game (both modes)
            info!("Setting up Steam library in prefix");
            log_to_file("Setting up Steam library in prefix");
            self.setup_steam_library(&game)?;

            // Override the steam:// protocol handler so WeMod launches the game
            // directly in this Wine session instead of through the native Steam
            // client (which would create a separate, unhookable Proton session)
            self.setup_steam_launch_proxy(&game, &game_exe)?;

            let cmd_str = if config.standalone {
                let batch_path = self.create_standalone_batch(&wemod_exe)?;
                let batch_win_path = Self::to_wine_path(&batch_path);

                let cmd = format!(
                    "cd \"{}\" && \"{}\" waitforexitandrun \"{}\"",
                    self.proton.path.display(),
                    self.proton.proton_exe().display(),
                    batch_win_path,
                );
                info!("Launching WeMod standalone via batch: {}", cmd);
                log_to_file(&format!("Standalone launch command: {}", cmd));
                cmd
            } else {
                // Normal mode: batch file coordinates WeMod + game startup
                let batch_path = self.create_launch_batch(
                    &wemod_exe,
                    &game,
                    &game_exe,
                    &config.extra_args,
                    config.wemod_delay,
                )?;
                let batch_win_path = Self::to_wine_path(&batch_path);

                let cmd = format!(
                    "cd \"{}\" && \"{}\" waitforexitandrun \"{}\"",
                    self.proton.path.display(),
                    self.proton.proton_exe().display(),
                    batch_win_path,
                );
                info!("Launching via batch file: {}", cmd);
                log_to_file(&format!("Launch command: {}", cmd));
                cmd
            };

            let child = Command::new("bash")
                .arg("-c")
                .arg(tee_to_log(&cmd_str))
                .envs(&env)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|e| {
                    log_to_file(&format!("Failed to launch: {}", e));
                    WandaError::LaunchFailed {
                        reason: format!("Failed to launch: {}", e),
                    }
                })?;

            log_to_file("Launch process started successfully");
            Some(child)
        } else {
            // Launch game without WeMod using direct Proton invocation
            let game_win_path = Self::to_wine_path(&game_exe);
            let cmd_str = format!(
                "cd \"{}\" && \"{}\" waitforexitandrun \"{}\" {}",
                self.proton.path.display(),
                self.proton.proton_exe().display(),
                game_win_path,
                config.extra_args.join(" ")
            );

            info!("Launching game directly: {}", cmd_str);
            log_to_file(&format!("Launch command: {}", cmd_str));

            let child = Command::new("bash")
                .arg("-c")
                .arg(tee_to_log(&cmd_str))
                .envs(&env)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|e| {
                    log_to_file(&format!("Failed to launch game: {}", e));
                    WandaError::LaunchFailed {
                        reason: format!("Failed to launch game: {}", e),
                    }
                })?;

            log_to_file("Game launched successfully");
            Some(child)
        };

        Ok(LaunchHandle {
            process,
            app_id: config.app_id,
            start_time: std::time::Instant::now(),
        })
    }

    /// Find the main executable for a game
    fn find_game_executable(&self, game: &SteamApp) -> Result<PathBuf> {
        let install_path = &game.install_path;

        if !install_path.exists() {
            return Err(WandaError::GameNotFound { app_id: game.app_id });
        }

        // Generate name patterns from install_dir (e.g., "ELDEN RING" -> "eldenring")
        let normalized_name = game.install_dir.to_lowercase().replace(' ', "");

        let patterns = [
            format!("{}.exe", game.install_dir),
            format!("{}.exe", normalized_name),  // eldenring.exe
            "game.exe".to_string(),
            "start.exe".to_string(),
            "launcher.exe".to_string(),
        ];

        // Check in root and Game/ subdirectory (common structure)
        let search_dirs = [
            install_path.clone(),
            install_path.join("Game"),
            install_path.join("game"),
            install_path.join("Bin"),
            install_path.join("bin"),
        ];

        for pattern in &patterns {
            for dir in &search_dirs {
                let exe_path = dir.join(pattern);
                if exe_path.exists() {
                    return Ok(exe_path);
                }
            }
        }

        // Exclusion list for anti-cheat, launchers, and utilities
        let exclusions = [
            "unins", "redist", "setup", "crash", "report",
            "start_protected", "easyanticheat", "battleye", "eac_",
            "dxsetup", "vcredist", "dotnet", "directx",
            "ue4prereq", "installer", "updater",
        ];

        // Walk directory looking for game executables
        for entry in walkdir::WalkDir::new(install_path)
            .max_depth(3)
            .into_iter()
            .flatten()
        {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "exe" {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    let name_lower = name.to_lowercase();

                    // Skip excluded executables
                    if exclusions.iter().any(|ex| name_lower.contains(ex)) {
                        continue;
                    }

                    // Prefer executables that match the game name
                    if name_lower.contains(&normalized_name) {
                        return Ok(path.to_path_buf());
                    }
                }
            }
        }

        // Fallback: return first non-excluded exe found
        for entry in walkdir::WalkDir::new(install_path)
            .max_depth(3)
            .into_iter()
            .flatten()
        {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "exe" {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    let name_lower = name.to_lowercase();

                    if !exclusions.iter().any(|ex| name_lower.contains(ex)) {
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
