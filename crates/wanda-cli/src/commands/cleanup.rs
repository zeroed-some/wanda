//! Cleanup command - kill stale Wine/WeMod processes

use clap::Args;
use std::path::PathBuf;
use tokio::process::Command;
use tracing::info;
use wanda_core::error::Result;

#[derive(Args)]
pub struct CleanupArgs {
    /// Force kill without confirmation
    #[arg(short, long)]
    pub force: bool,
}

pub async fn run(_args: CleanupArgs, _config_path: Option<PathBuf>) -> Result<()> {
    info!("Cleaning up Wine/WeMod processes...");

    let patterns = [
        "WeMod",
        "WeModAuxiliary",
        "wineserver",
        "winedevice",
        "steam.exe",
    ];

    for pattern in &patterns {
        let output = Command::new("pkill")
            .args(["-9", "-f", pattern])
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => {
                info!("Killed processes matching '{}'", pattern);
            }
            _ => {}
        }
    }

    // Verify cleanup
    let check = Command::new("sh")
        .args(["-c", "ps aux | grep -iE 'wemod|wineserver|winedevice' | grep -v grep | wc -l"])
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
            println!("{} processes may still be running", count);
        }
    }

    Ok(())
}
