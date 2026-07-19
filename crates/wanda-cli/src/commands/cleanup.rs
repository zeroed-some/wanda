//! Cleanup command - kill stale Wine/WeMod processes

use clap::Args;
use std::path::PathBuf;
use tokio::process::Command;
use tracing::{debug, info};
use wanda_core::{config::WandaConfig, error::Result};

#[derive(Args)]
pub struct CleanupArgs {
    /// Force kill without confirmation
    #[arg(short, long)]
    pub force: bool,
}

pub async fn run(_args: CleanupArgs, config_path: Option<PathBuf>) -> Result<()> {
    info!("Cleaning up Wine/WeMod processes...");

    let config = match &config_path {
        Some(path) => WandaConfig::load_from(path).unwrap_or_default(),
        None => WandaConfig::load().unwrap_or_default(),
    };

    // 1. Kill every process whose WINEPREFIX points at a wanda-managed prefix
    //    or a Steam compatdata prefix. This tears down the whole Wine session
    //    — the game, Ubisoft Connect / other launchers, WeMod, and the
    //    explorer.exe virtual desktop that leaves a stray black window —
    //    including processes already orphaned by a dead wineserver, which a
    //    plain `pkill wineserver` would leave behind.
    let killed = kill_by_wineprefix(&config);
    if killed.is_empty() {
        debug!("No processes matched a wanda/compatdata WINEPREFIX");
    } else {
        info!("Killed {} process(es) by WINEPREFIX match", killed.len());
        for (pid, prefix) in &killed {
            debug!("  killed pid {} (WINEPREFIX={})", pid, prefix);
        }
    }

    // 2. Name-based safety net for Wine infrastructure that may linger. These
    //    are Windows-only executable names, so matching them on a Linux host
    //    can't hit a native process. Covers orphans whose environ was
    //    unreadable and the black-window culprit (explorer.exe).
    let patterns = [
        "WeMod",
        "WeModAuxiliary",
        "wineserver",
        "winedevice.exe",
        "services.exe",
        "plugplay.exe",
        "svchost.exe",
        "rpcss.exe",
        "tabtip.exe",
        "explorer.exe",
        "xalia.exe",
        "wineboot.exe",
        "conhost.exe",
        "start.exe",
        "steam.exe",
    ];

    for pattern in &patterns {
        let output = Command::new("pkill")
            .args(["-9", "-f", pattern])
            .output()
            .await;

        if let Ok(o) = output {
            if o.status.success() {
                info!("Killed processes matching '{}'", pattern);
            }
        }
    }

    // 3. Verify
    let check = Command::new("sh")
        .args([
            "-c",
            "ps -eo comm,args 2>/dev/null | grep -iE 'wemod|wineserver|winedevice|explorer\\.exe|services\\.exe' | grep -v grep | wc -l",
        ])
        .output()
        .await;

    if let Ok(output) = check {
        let count: i32 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap_or(0);

        if count == 0 {
            println!("All Wine/WeMod processes cleaned up");
        } else {
            println!(
                "{} Wine process(es) may still be running (possibly zombies awaiting reaping, or an unrelated Proton game)",
                count
            );
        }
    }

    Ok(())
}

/// Kill every process whose `WINEPREFIX` environment variable points at a
/// wanda-managed prefix or a Steam `compatdata` prefix.
///
/// Reads `/proc/<pid>/environ` (only our own processes are readable, which is
/// exactly the set we care about). Returns the `(pid, wineprefix)` pairs killed.
fn kill_by_wineprefix(config: &WandaConfig) -> Vec<(u32, String)> {
    let self_pid = std::process::id();

    // The wanda prefix base, e.g. ~/.local/share/wanda/prefix
    let wanda_base = config
        .prefix_base_path()
        .to_string_lossy()
        .to_string();

    let proc = match std::fs::read_dir("/proc") {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let mut victims: Vec<(u32, String)> = Vec::new();

    for entry in proc.flatten() {
        let pid: u32 = match entry.file_name().to_string_lossy().parse() {
            Ok(p) => p,
            Err(_) => continue, // non-numeric /proc entry
        };
        if pid == self_pid {
            continue;
        }

        let data = match std::fs::read(format!("/proc/{}/environ", pid)) {
            Ok(d) => d,
            Err(_) => continue, // process gone or not readable
        };

        // environ is a NUL-separated list of KEY=VALUE entries
        for var in data.split(|&b| b == 0) {
            let var = String::from_utf8_lossy(var);
            let value = match var.strip_prefix("WINEPREFIX=") {
                Some(v) => v,
                None => continue,
            };

            let is_compatdata = value.contains("/steamapps/compatdata/");
            let is_wanda = !wanda_base.is_empty() && value.contains(&wanda_base);
            if is_compatdata || is_wanda {
                victims.push((pid, value.to_string()));
            }
            break; // WINEPREFIX seen; no need to scan the rest of environ
        }
    }

    // SIGKILL each victim in one batch via `kill -9`.
    if !victims.is_empty() {
        let mut args: Vec<String> = vec!["-9".to_string()];
        args.extend(victims.iter().map(|(pid, _)| pid.to_string()));
        let _ = std::process::Command::new("kill").args(&args).output();
    }

    victims
}
