//! wanda inject - Steam launch option wrapper for WeMod injection
//!
//! Usage: Set Steam launch options for a game to:
//!   wanda inject %command%
//!
//! This intercepts the game's launch, sets up WeMod in the game's own Proton
//! prefix, and creates a batch file that:
//! - On first run: starts WeMod and keeps the session alive
//! - On subsequent runs (from WeMod's Play button): launches the game directly
//!   into the existing Wine session so WeMod can detect and hook it

use clap::Args;
use std::fs;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use wanda_core::{
    config::WandaConfig,
    prefix::PrefixBuilder,
    steam::{ProtonCompatibility, ProtonVersion},
    Result, WandaError,
};

#[derive(Args)]
pub struct InjectArgs {
    /// The Steam launch command (%command%)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<String>,
}

/// Log a message to the wanda log file
fn log_to_file(msg: &str) {
    let log_path = WandaConfig::default_data_dir().join("wanda.log");
    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "[{}] [inject] {}", timestamp, msg);
    }
}

/// Parsed components of the Steam launch command
struct ParsedCommand {
    /// Path to the Proton installation directory
    proton_dir: PathBuf,
    /// Path to the game executable (Linux path) — Steam's original
    game_exe: PathBuf,
    /// Path to the real game executable (bypassing EAC launcher if present)
    real_game_exe: PathBuf,
    /// Index of the game exe in the original args (for replacement)
    game_exe_idx: usize,
}

/// Parse the Steam launch command by finding the Proton script and game exe.
///
/// Steam's launch pipeline uses multiple `--` separators:
/// ```text
/// steam-launch-wrapper -- reaper SteamLaunch AppId=XXXX --
///   _v2-entry-point --verb=waitforexitandrun --
///   proton waitforexitandrun /path/to/game.exe
/// ```
///
/// We find the actual `proton` script by scanning for a path whose filename
/// is `proton` and exists on disk, then take the game exe as the arg after
/// the verb.
fn parse_command(raw_args: &[String]) -> Result<ParsedCommand> {
    // Find the Proton script: last arg whose filename is "proton" and exists
    let proton_idx = raw_args
        .iter()
        .rposition(|arg| {
            let path = Path::new(arg);
            path.file_name().map_or(false, |name| name == "proton")
                && path.exists()
        })
        .ok_or_else(|| WandaError::LaunchFailed {
            reason: format!(
                "Could not find Proton script in command args: {:?}",
                raw_args
            ),
        })?;

    let proton_exe = PathBuf::from(&raw_args[proton_idx]);
    let proton_dir = proton_exe
        .parent()
        .ok_or_else(|| WandaError::LaunchFailed {
            reason: format!(
                "Cannot determine Proton directory from: {}",
                proton_exe.display()
            ),
        })?
        .to_path_buf();

    // Verb is the arg after proton (e.g., "waitforexitandrun")
    let verb_idx = proton_idx + 1;
    if verb_idx >= raw_args.len() {
        return Err(WandaError::LaunchFailed {
            reason: "No verb found after Proton script".into(),
        });
    }

    // Game exe is the arg after the verb
    let game_idx = verb_idx + 1;
    if game_idx >= raw_args.len() {
        return Err(WandaError::LaunchFailed {
            reason: "No game executable found after Proton verb".into(),
        });
    }

    let game_exe = PathBuf::from(&raw_args[game_idx]);

    // Find the real game exe, bypassing anti-cheat launchers.
    // Steam launches `start_protected_game.exe` (EAC) which internally
    // starts the real game. WeMod scans for the real exe, not the
    // launcher. Look for a sibling exe that isn't an anti-cheat launcher.
    let real_game_exe = find_real_game_exe(&game_exe).unwrap_or_else(|| game_exe.clone());

    Ok(ParsedCommand {
        proton_dir,
        game_exe,
        real_game_exe,
        game_exe_idx: game_idx,
    })
}

/// Find the real game executable, bypassing anti-cheat launchers.
///
/// When Steam launches `start_protected_game.exe` (EAC) or similar,
/// the real game exe is usually a sibling in the same directory.
/// WeMod scans for the real exe via QueryFullProcessImageName.
fn find_real_game_exe(steam_game_exe: &Path) -> Option<PathBuf> {
    let name = steam_game_exe
        .file_name()?
        .to_string_lossy()
        .to_lowercase();

    // If Steam's exe isn't an anti-cheat launcher, use it directly
    let ac_patterns = [
        "start_protected",
        "easyanticheat",
        "battleye",
        "eac_",
    ];
    if !ac_patterns.iter().any(|p| name.contains(p)) {
        return None; // Already the real exe
    }

    let dir = steam_game_exe.parent()?;
    let exclude = [
        "start_protected",
        "easyanticheat",
        "battleye",
        "eac_",
        "unins",
        "crash",
        "report",
        "redist",
        "setup",
        "dxsetup",
        "vcredist",
        "dotnet",
        "directx",
        "ue4prereq",
        "installer",
        "updater",
    ];

    // Look for a non-excluded .exe in the same directory
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "exe") && path.is_file() {
                let fname = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                if !exclude.iter().any(|ex| fname.contains(ex)) {
                    candidates.push(path);
                }
            }
        }
    }

    // Prefer the largest exe (usually the real game, not a small launcher)
    candidates.sort_by(|a, b| {
        let sa = fs::metadata(a).map(|m| m.len()).unwrap_or(0);
        let sb = fs::metadata(b).map(|m| m.len()).unwrap_or(0);
        sb.cmp(&sa)
    });

    candidates.into_iter().next()
}

/// Find the WeMod installation in wanda's default prefix
fn find_wanda_wemod(config: &WandaConfig) -> Result<PathBuf> {
    let prefix_base = config.prefix_base_path();
    let default_prefix = prefix_base.join("default");

    // Check WeMod branding (v11.x)
    let wemod_path =
        default_prefix.join("pfx/drive_c/users/steamuser/AppData/Local/WeMod");
    if wemod_path.exists() {
        return Ok(wemod_path);
    }

    // Check Wand branding (v12.x)
    let wand_path =
        default_prefix.join("pfx/drive_c/users/steamuser/AppData/Local/Wand");
    if wand_path.exists() {
        return Ok(wand_path);
    }

    Err(WandaError::WemodNotInstalled)
}

/// Find the real WeMod app exe (not the Squirrel stub) in a WeMod directory.
/// Rejects v12+ which causes black window/renderer crashes under Wine.
fn find_wemod_app_exe(wemod_dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(String, PathBuf)> = None;
    if let Ok(entries) = fs::read_dir(wemod_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("app-") && entry.path().is_dir() {
                // Skip v12+ (black screen under Wine)
                let version_part = name.trim_start_matches("app-");
                if let Some(major) = version_part.split('.').next() {
                    if major.parse::<u32>().unwrap_or(0) >= 12 {
                        debug!("Skipping incompatible WeMod version: {}", name);
                        continue;
                    }
                }
                let exe = entry.path().join("WeMod.exe");
                if exe.exists() {
                    if best.as_ref().map_or(true, |(v, _)| name > *v) {
                        best = Some((name, exe));
                    }
                }
            }
        }
    }
    best.map(|(_, path)| path)
}

/// Symlink WeMod from wanda's prefix into the game's prefix
fn sync_wemod_to_prefix(wanda_wemod: &Path, compat_data: &Path) -> Result<()> {
    let target_local =
        compat_data.join("pfx/drive_c/users/steamuser/AppData/Local");
    fs::create_dir_all(&target_local).map_err(|e| WandaError::LaunchFailed {
        reason: format!("Failed to create AppData/Local in game prefix: {}", e),
    })?;

    let dir_name = wanda_wemod
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let link_path = target_local.join(&dir_name);

    if link_path.exists() {
        if link_path.is_symlink() {
            debug!("WeMod symlink already exists at {}", link_path.display());
            return Ok(());
        }
        // Real directory exists — leave it alone, WeMod may have been installed
        // directly into this prefix
        debug!(
            "WeMod directory already exists (not a symlink) at {}",
            link_path.display()
        );
        return Ok(());
    }

    info!(
        "Symlinking WeMod: {} -> {}",
        link_path.display(),
        wanda_wemod.display()
    );
    log_to_file(&format!(
        "Symlinking WeMod: {} -> {}",
        link_path.display(),
        wanda_wemod.display()
    ));
    std::os::unix::fs::symlink(wanda_wemod, &link_path).map_err(|e| {
        WandaError::LaunchFailed {
            reason: format!("Failed to symlink WeMod: {}", e),
        }
    })?;

    Ok(())
}

/// Extract the Steam library root from a game exe path.
///
/// Game exe is like: `/path/to/steamapps/common/GAME/game.exe`
/// Library root is: `/path/to`
fn library_root_from_game_exe(game_exe: &Path) -> Option<&Path> {
    let game_str = game_exe.to_str()?;
    let pos = game_str.find("/steamapps/common/")?;
    Some(Path::new(&game_str[..pos]))
}

/// Set up Steam library structure in the game's prefix so WeMod can find games
fn setup_steam_library(compat_data: &Path, game_exe: &Path) -> Result<()> {
    let steamapps_dir =
        compat_data.join("pfx/drive_c/Program Files (x86)/Steam/steamapps");
    fs::create_dir_all(&steamapps_dir).map_err(|e| WandaError::LaunchFailed {
        reason: format!("Failed to create Steam library in prefix: {}", e),
    })?;

    let library_root = match library_root_from_game_exe(game_exe) {
        Some(root) => root,
        None => {
            warn!(
                "Could not find steamapps/common in game path: {}",
                game_exe.display()
            );
            return Ok(());
        }
    };

    // Symlink common/ → real steamapps/common/
    let common_link = steamapps_dir.join("common");
    let common_target = library_root.join("steamapps/common");
    if !common_link.exists() {
        info!(
            "Symlinking Steam common: {} -> {}",
            common_link.display(),
            common_target.display()
        );
        log_to_file(&format!(
            "Symlinking common: {} -> {}",
            common_link.display(),
            common_target.display()
        ));
        std::os::unix::fs::symlink(&common_target, &common_link).map_err(
            |e| WandaError::LaunchFailed {
                reason: format!("Failed to symlink common: {}", e),
            },
        )?;
    }

    // Symlink appmanifest and write libraryfolders.vdf using app ID from env
    if let Ok(app_id) = std::env::var("SteamAppId") {
        let manifest_name = format!("appmanifest_{}.acf", app_id);
        let manifest_target =
            library_root.join("steamapps").join(&manifest_name);
        let manifest_link = steamapps_dir.join(&manifest_name);

        if !manifest_link.exists() && manifest_target.exists() {
            info!("Symlinking appmanifest: {}", manifest_name);
            log_to_file(&format!(
                "Symlinking appmanifest: {} -> {}",
                manifest_link.display(),
                manifest_target.display()
            ));
            std::os::unix::fs::symlink(&manifest_target, &manifest_link)
                .map_err(|e| WandaError::LaunchFailed {
                    reason: format!("Failed to symlink appmanifest: {}", e),
                })?;
        }

        // Write libraryfolders.vdf so WeMod can discover the game library
        let vdf_path = steamapps_dir.join("libraryfolders.vdf");
        let vdf_content = format!(
            r#""libraryfolders"
{{
	"0"
	{{
		"path"		"C:\\Program Files (x86)\\Steam"
		"apps"
		{{
			"{}"		"0"
		}}
	}}
}}"#,
            app_id,
        );
        fs::write(&vdf_path, &vdf_content).map_err(|e| {
            WandaError::LaunchFailed {
                reason: format!("Failed to create libraryfolders.vdf: {}", e),
            }
        })?;
    }

    // Add Steam registry entries
    setup_steam_registry(compat_data)?;

    Ok(())
}

/// Write Steam registry entries into the game's Wine prefix
fn setup_steam_registry(compat_data: &Path) -> Result<()> {
    let user_reg = compat_data.join("pfx/user.reg");
    if !user_reg.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&user_reg).map_err(|e| {
        WandaError::LaunchFailed {
            reason: format!("Failed to read user.reg: {}", e),
        }
    })?;

    if content.contains("[Software\\\\Valve\\\\Steam]") {
        debug!("Steam registry entries already exist");
        return Ok(());
    }

    let app_id = std::env::var("SteamAppId").unwrap_or_default();

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
"Running"=dword:00000000
"#,
        ts = timestamp,
        app_id = app_id,
    );

    info!("Adding Steam registry entries");
    log_to_file("Adding Steam registry entries to game prefix");

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&user_reg)
        .map_err(|e| WandaError::LaunchFailed {
            reason: format!("Failed to open user.reg: {}", e),
        })?;
    file.write_all(steam_entry.as_bytes()).map_err(|e| {
        WandaError::LaunchFailed {
            reason: format!("Failed to write Steam registry: {}", e),
        }
    })?;

    Ok(())
}

/// Convert a game exe path to its C: drive equivalent through the Steam
/// library symlink set up in the prefix.
///
/// `/path/to/steamapps/common/GAME/game.exe`
///   → `C:\Program Files (x86)\Steam\steamapps\common\GAME\game.exe`
fn to_c_drive_game_path(game_exe: &Path) -> Option<String> {
    let game_str = game_exe.to_string_lossy();
    let pos = game_str.find("/steamapps/common/")?;
    let rel_path = &game_str[pos + "/steamapps/common/".len()..];
    Some(format!(
        "C:\\Program Files (x86)\\Steam\\steamapps\\common\\{}",
        rel_path.replace('/', "\\")
    ))
}

/// Compute the C: drive path for the WeMod exe inside the game's prefix.
///
/// The WeMod directory is symlinked at:
///   `<prefix>/pfx/drive_c/users/steamuser/AppData/Local/WeMod/`
/// so the Windows path is:
///   `C:\users\steamuser\AppData\Local\WeMod\app-X.Y.Z\WeMod.exe`
fn wemod_c_drive_path(
    compat_data: &Path,
    wanda_wemod_dir: &Path,
) -> Option<String> {
    let dir_name = wanda_wemod_dir.file_name()?.to_string_lossy();
    let prefix_wemod = compat_data
        .join("pfx/drive_c/users/steamuser/AppData/Local")
        .join(dir_name.as_ref());

    // Find the app-X.Y.Z subdirectory (skip v12+ — black screen under Wine)
    let mut best: Option<(String, String)> = None;
    if let Ok(entries) = fs::read_dir(&prefix_wemod) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("app-") && entry.path().is_dir() {
                let version_part = name.trim_start_matches("app-");
                if let Some(major) = version_part.split('.').next() {
                    if major.parse::<u32>().unwrap_or(0) >= 12 {
                        continue;
                    }
                }
                let exe = entry.path().join("WeMod.exe");
                if exe.exists() {
                    if best.as_ref().map_or(true, |(v, _)| name > *v) {
                        best = Some((
                            name.clone(),
                            format!(
                                "C:\\users\\steamuser\\AppData\\Local\\{}\\{}\\WeMod.exe",
                                dir_name, name
                            ),
                        ));
                    }
                }
            }
        }
    }

    best.map(|(_, path)| path).or_else(|| {
        // Fall back to root WeMod.exe
        let root = prefix_wemod.join("WeMod.exe");
        if root.exists() {
            Some(format!(
                "C:\\users\\steamuser\\AppData\\Local\\{}\\WeMod.exe",
                dir_name
            ))
        } else {
            None
        }
    })
}

/// Create the main inject batch: starts WeMod, then polls for a signal
/// file that the steam:// protocol proxy creates when WeMod clicks Play.
/// When the signal is detected, launches the game directly in the same
/// Wine session.
fn create_inject_batch(
    compat_data: &Path,
    wanda_wemod_dir: &Path,
    game_exe: &Path,
    game_args: &[String],
) -> Result<PathBuf> {
    let batch_path = compat_data.join("wanda_inject.bat");

    let wemod_win_path =
        wemod_c_drive_path(compat_data, wanda_wemod_dir).unwrap_or_else(|| {
            let fallback = find_wemod_app_exe(wanda_wemod_dir)
                .unwrap_or_else(|| wanda_wemod_dir.join("WeMod.exe"));
            format!("Z:{}", fallback.to_string_lossy().replace('/', "\\"))
        });
    let game_win_path = to_c_drive_game_path(game_exe).unwrap_or_else(|| {
        format!("Z:{}", game_exe.to_string_lossy().replace('/', "\\"))
    });
    let game_args_str = game_args.join(" ");

    // Start WeMod and keep the session alive. WeMod calls CreateProcessW
    // directly when the user clicks Play — the game starts in the same
    // wineserver. We must NOT launch the game ourselves for trainers
    // that require WeMod to create the process (suspended + DLL inject).
    let batch_content = format!(
        r#"@echo off
@title WANDA Inject
echo [WANDA] Starting WeMod...
echo [WANDA] WeMod: {wemod_path}
echo [WANDA] Game: {game_path}

REM Clear Steam app ID env vars so steamclient.dll inside WeMod does
REM not associate this session with the running game. Without this,
REM WeMod tells Steam "launch game" and Steam says "already running"
REM because our batch IS the running game from Steam's perspective.
set SteamAppId=
set SteamGameId=
set SteamOverlayGameId=
set STEAM_COMPAT_APP_ID=

start "" "{wemod_path}" --no-sandbox

echo [WANDA] Waiting for WeMod to initialize...
ping 127.0.0.1 -n 12 > NUL 2>&1

echo.
echo [WANDA] WeMod is running.
echo [WANDA] Find your game in WeMod and click Play.
echo [WANDA] WeMod will launch the game directly.
echo [WANDA] Press Stop in Steam to end the session.
echo.

:keepalive
ping 127.0.0.1 -n 30 > NUL 2>&1
goto keepalive
"#,
        wemod_path = wemod_win_path,
        game_path = game_win_path,
    );

    let batch_crlf = batch_content.replace('\n', "\r\n");
    fs::write(&batch_path, &batch_crlf).map_err(|e| {
        WandaError::LaunchFailed {
            reason: format!("Failed to create inject batch: {}", e),
        }
    })?;

    log_to_file(&format!(
        "Created inject batch at: {}",
        batch_path.display()
    ));
    log_to_file(&format!("WeMod C: path: {}", wemod_win_path));
    log_to_file(&format!("Game C: path: {}", game_win_path));

    Ok(batch_path)
}

/// Check if a wineserver is already running for the game's prefix.
/// Uses Proton's wineserver binary with signal 0 (existence check).
fn is_wineserver_running(proton_dir: &Path, compat_data: &Path) -> bool {
    let wineserver = proton_dir.join("files/bin/wineserver");
    let prefix = compat_data.join("pfx");
    if !wineserver.exists() {
        log_to_file("wineserver binary not found");
        return false;
    }

    // Proton's wineserver needs its own libraries
    let lib64 = proton_dir.join("files/lib64");
    let lib = proton_dir.join("files/lib");
    let ld_path = format!("{}:{}", lib64.display(), lib.display());

    let result = std::process::Command::new(&wineserver)
        .env("WINEPREFIX", &prefix)
        .env("LD_LIBRARY_PATH", &ld_path)
        .arg("-k")
        .arg("0")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match result {
        Ok(status) => {
            let running = status.success();
            log_to_file(&format!("wineserver check: running={}", running));
            running
        }
        Err(e) => {
            log_to_file(&format!("wineserver check failed: {}", e));
            false
        }
    }
}

/// Set up steam:// protocol proxy so WeMod's Play button launches the
/// game directly inside the Wine session (instead of going through
/// native Steam, which creates a separate Proton session).
fn setup_steam_protocol_proxy(
    compat_data: &Path,
    game_exe: &Path,
) -> Result<()> {
    let pfx = compat_data.join("pfx");

    // Create proxy batch at C:\wanda\steam_proxy.bat
    // This is triggered when WeMod calls steam://rungameid/<appid>.
    // Instead of launching the game directly, it creates a signal file
    // that the main inject batch polls for.
    let wanda_dir = pfx.join("drive_c/wanda");
    let _ = fs::create_dir_all(&wanda_dir);
    let proxy_path = wanda_dir.join("steam_proxy.bat");

    let proxy_content = r#"@echo off
REM Wanda Steam Protocol Proxy — signals the inject batch to launch the game
echo [WANDA] Steam launch intercepted: %~1
echo [WANDA] Creating launch signal...
echo launch > C:\wanda\launch_signal
"#;
    let proxy_crlf = proxy_content.replace('\n', "\r\n").to_string();
    fs::write(&proxy_path, &proxy_crlf).map_err(|e| WandaError::LaunchFailed {
        reason: format!("Failed to create steam proxy: {}", e),
    })?;

    // Override steam:// protocol handler in user.reg
    let user_reg = pfx.join("user.reg");
    if !user_reg.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(&user_reg).unwrap_or_default();
    if content.contains("wanda\\\\steam_proxy.bat") {
        return Ok(());
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let registry = format!(
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

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&user_reg)
        .map_err(|e| WandaError::LaunchFailed {
            reason: format!("Failed to open user.reg: {}", e),
        })?;
    file.write_all(registry.as_bytes()).map_err(|e| {
        WandaError::LaunchFailed {
            reason: format!("Failed to write steam:// override: {}", e),
        }
    })?;

    log_to_file("Set up steam:// protocol proxy in game prefix");
    Ok(())
}

pub async fn run(_args: InjectArgs, config_path: Option<PathBuf>) -> Result<()> {
    // Log immediately so we can tell if inject was even called
    log_to_file("=== WANDA INJECT START ===");
    log_to_file(&format!("PID: {}", std::process::id()));

    // Get raw args from environment to preserve `--` separators
    let raw_args: Vec<String> = std::env::args().collect();
    log_to_file(&format!("Raw argv: {:?}", raw_args));

    let inject_pos = raw_args
        .iter()
        .position(|a| a == "inject")
        .ok_or_else(|| WandaError::LaunchFailed {
            reason: "Could not find 'inject' in argv".into(),
        })?;
    let command_args: Vec<String> = raw_args[inject_pos + 1..].to_vec();

    log_to_file(&format!("Command args: {:?}", command_args));

    if command_args.is_empty() {
        return Err(WandaError::LaunchFailed {
            reason: "No command provided. Usage: wanda inject %command%".into(),
        });
    }

    let parsed = parse_command(&command_args)?;

    log_to_file(&format!("Proton dir: {}", parsed.proton_dir.display()));
    log_to_file(&format!("Game exe (Steam): {}", parsed.game_exe.display()));
    log_to_file(&format!("Game exe (real): {}", parsed.real_game_exe.display()));
    log_to_file(&format!(
        "Game exe at index {} in command args",
        parsed.game_exe_idx
    ));

    let compat_data = std::env::var("STEAM_COMPAT_DATA_PATH")
        .map(PathBuf::from)
        .map_err(|_| WandaError::LaunchFailed {
            reason: "STEAM_COMPAT_DATA_PATH not set. \
                     Are you running this from Steam launch options?"
                .into(),
        })?;

    log_to_file(&format!("Compat data path: {}", compat_data.display()));

    // ── Check if WeMod is already running (wineserver active) ──────────
    //
    // If `wanda launch --standalone` started WeMod in the game's prefix,
    // the wineserver is already running. In that case, just pass through
    // the original Steam command unmodified — Proton will connect to the
    // existing wineserver, and the game will run alongside WeMod.
    if is_wineserver_running(&parsed.proton_dir, &compat_data) {
        log_to_file("Wineserver active — passthrough mode (WeMod already running)");
        eprintln!("[wanda inject] WeMod session detected — launching game into existing session");

        let program = &command_args[0];
        let exec_args = &command_args[1..];

        log_to_file(&format!("Passthrough exec: {} {:?}", program, exec_args));

        let err = std::process::Command::new(program)
            .args(exec_args)
            .exec();

        return Err(WandaError::LaunchFailed {
            reason: format!("Failed to exec {}: {}", program, err),
        });
    }

    // ── No wineserver: full setup + WeMod launch ───────────────────────
    log_to_file("No active wineserver — full inject mode");

    let config = match &config_path {
        Some(path) => WandaConfig::load_from(path)?,
        None => WandaConfig::load()?,
    };

    let wanda_wemod = find_wanda_wemod(&config)?;
    log_to_file(&format!("Found WeMod at: {}", wanda_wemod.display()));

    // Setup (all idempotent)
    sync_wemod_to_prefix(&wanda_wemod, &compat_data)?;

    let wanda_roaming = {
        let prefix_base = config.prefix_base_path();
        prefix_base.join(
            "default/pfx/drive_c/users/steamuser/AppData/Roaming/WeMod",
        )
    };
    if wanda_roaming.exists() {
        let target_roaming = compat_data
            .join("pfx/drive_c/users/steamuser/AppData/Roaming");
        let _ = fs::create_dir_all(&target_roaming);
        let roaming_link = target_roaming.join("WeMod");
        if !roaming_link.exists() {
            let _ =
                std::os::unix::fs::symlink(&wanda_roaming, &roaming_link);
        }
    }

    let proton = ProtonVersion {
        name: parsed
            .proton_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        path: parsed.proton_dir.clone(),
        version: (0, 0, 0),
        is_ge: false,
        is_experimental: false,
        compatibility: ProtonCompatibility::Supported,
    };
    let builder = PrefixBuilder::new(&compat_data, &proton);
    if let Err(e) = builder.patch_mscorlib_version() {
        log_to_file(&format!("Warning: mscorlib patch failed: {}", e));
        warn!("mscorlib patch failed (may not be critical): {}", e);
    }

    setup_steam_library(&compat_data, &parsed.game_exe)?;

    let game_args = &command_args[parsed.game_exe_idx + 1..];
    let batch_path = create_inject_batch(
        &compat_data,
        &wanda_wemod,
        &parsed.real_game_exe,
        game_args,
    )?;

    let batch_wine_path = format!("Z:{}", batch_path.to_string_lossy());

    let mut exec_args: Vec<String> =
        command_args[1..parsed.game_exe_idx].to_vec();
    exec_args.push(batch_wine_path.clone());

    let program = &command_args[0];

    log_to_file(&format!("Exec: {} {:?}", program, exec_args));
    eprintln!(
        "[wanda inject] Starting WeMod session for {} — click Play in WeMod to launch",
        parsed.game_exe.file_name().unwrap_or_default().to_string_lossy()
    );

    let err = std::process::Command::new(program)
        .args(&exec_args)
        .exec();

    Err(WandaError::LaunchFailed {
        reason: format!("Failed to exec {}: {}", program, err),
    })
}
