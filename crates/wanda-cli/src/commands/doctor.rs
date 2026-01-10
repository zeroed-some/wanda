//! wanda doctor - Diagnose issues

use clap::Args;
use console::style;
use std::path::PathBuf;
use wanda_core::{
    config::WandaConfig,
    prefix::{PrefixHealth, PrefixManager},
    steam::{ProtonCompatibility, ProtonManager, SteamInstallation},
    Result,
};

#[derive(Args)]
pub struct DoctorArgs {
    /// Generate full diagnostic report
    #[arg(long)]
    full: bool,

    /// Export report to file
    #[arg(long)]
    export: Option<PathBuf>,

    /// Attempt to fix detected issues
    #[arg(long)]
    fix: bool,
}

pub async fn run(args: DoctorArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = match &config_path {
        Some(path) => WandaConfig::load_from(path)?,
        None => WandaConfig::load()?,
    };

    println!("WANDA Diagnostic Report\n");
    println!("{}\n", "=".repeat(50));

    let mut issues = Vec::new();

    // Check Steam
    print_section("Steam Installation");
    match SteamInstallation::discover(&config) {
        Ok(steam) => {
            println!(
                "  {} Found at {}{}",
                style("OK").green(),
                steam.root_path.display(),
                if steam.is_flatpak { " (Flatpak)" } else { "" }
            );
            println!("  {} {} libraries", style("OK").green(), steam.libraries.len());

            let total_games: usize = steam.libraries.iter().map(|l| l.apps.len()).sum();
            let proton_games = steam
                .get_all_games()
                .iter()
                .filter(|g| g.uses_proton())
                .count();
            println!("  {} {} games ({} using Proton)", style("OK").green(), total_games, proton_games);

            // Check Proton
            print_section("Proton");
            match ProtonManager::discover(&steam, &config) {
                Ok(proton_mgr) => {
                    if proton_mgr.versions.is_empty() {
                        println!("  {} No Proton versions found", style("ERROR").red());
                        issues.push("No Proton versions installed".to_string());
                    } else {
                        println!(
                            "  {} {} versions found",
                            style("OK").green(),
                            proton_mgr.versions.len()
                        );

                        // List versions in full mode
                        if args.full {
                            for v in &proton_mgr.versions {
                                let compat = match v.compatibility {
                                    ProtonCompatibility::Recommended => {
                                        style("RECOMMENDED").green()
                                    }
                                    ProtonCompatibility::Supported => style("SUPPORTED").blue(),
                                    ProtonCompatibility::Experimental => {
                                        style("EXPERIMENTAL").yellow()
                                    }
                                    ProtonCompatibility::Unsupported => style("UNSUPPORTED").red(),
                                };
                                println!("      {} {}", v.name, compat);
                            }
                        }

                        if let Some(recommended) = proton_mgr.get_recommended() {
                            println!(
                                "  {} Recommended: {}",
                                style("OK").green(),
                                recommended.name
                            );
                        } else {
                            println!(
                                "  {} No recommended Proton version",
                                style("WARN").yellow()
                            );
                            issues.push("No recommended Proton version found".to_string());
                        }
                    }
                }
                Err(e) => {
                    println!("  {} Error: {}", style("ERROR").red(), e);
                    issues.push(format!("Proton detection failed: {}", e));
                }
            }
        }
        Err(e) => {
            println!("  {} Not found: {}", style("ERROR").red(), e);
            issues.push("Steam installation not found".to_string());
        }
    }

    // Check WANDA prefix
    print_section("WANDA Prefix");
    let mut prefix_manager = PrefixManager::new(&config);
    prefix_manager.load()?;

    if let Some(prefix) = prefix_manager.get("default") {
        println!("  {} Found at {}", style("OK").green(), prefix.path.display());

        let health = prefix_manager.validate("default")?;
        match health {
            PrefixHealth::Healthy => {
                println!("  {} Prefix is healthy", style("OK").green());
            }
            PrefixHealth::NeedsRepair(ref issue_list) => {
                println!("  {} Prefix needs repair:", style("WARN").yellow());
                for issue in issue_list {
                    println!("      - {}", issue);
                    issues.push(format!("Prefix issue: {}", issue));
                }
            }
            PrefixHealth::Corrupted(reason) => {
                println!("  {} Prefix is corrupted: {}", style("ERROR").red(), reason);
                issues.push(format!("Prefix corrupted: {}", reason));
            }
            PrefixHealth::NotCreated => {
                println!("  {} Prefix not initialized", style("WARN").yellow());
                issues.push("WANDA prefix not created".to_string());
            }
        }

        // Check WeMod
        print_section("WeMod");
        if prefix.wemod_installed {
            println!(
                "  {} Installed (version {})",
                style("OK").green(),
                prefix.wemod_version.as_deref().unwrap_or("unknown")
            );

            if prefix.wemod_exe().exists() {
                println!("  {} Executable found", style("OK").green());
            } else {
                println!("  {} Executable missing", style("ERROR").red());
                issues.push("WeMod executable not found".to_string());
            }
        } else {
            println!("  {} Not installed", style("WARN").yellow());
            issues.push("WeMod not installed".to_string());
        }
    } else {
        println!("  {} Not initialized", style("WARN").yellow());
        issues.push("WANDA not initialized".to_string());
    }

    // Check system dependencies
    print_section("System Dependencies");
    check_command("wine", "Wine", &mut issues).await;
    check_command("winetricks", "Winetricks", &mut issues).await;
    check_command("steam", "Steam CLI", &mut issues).await;
    check_command("xdg-open", "XDG utilities", &mut issues).await;

    // Summary
    println!("\n{}", "=".repeat(50));
    if issues.is_empty() {
        println!(
            "\n{} All checks passed! WANDA is ready to use.",
            style("SUCCESS").green().bold()
        );
    } else {
        println!(
            "\n{} Found {} issue(s):\n",
            style("ISSUES").yellow().bold(),
            issues.len()
        );
        for (i, issue) in issues.iter().enumerate() {
            println!("  {}. {}", i + 1, issue);
        }

        println!("\nSuggestions:");
        if issues.iter().any(|i| i.contains("not initialized")) {
            println!("  - Run 'wanda init' to initialize WANDA");
        }
        if issues.iter().any(|i| i.contains("WeMod")) {
            println!("  - Run 'wanda wemod install' to install WeMod");
        }
        if issues.iter().any(|i| i.contains("Prefix")) {
            println!("  - Run 'wanda prefix repair' to fix prefix issues");
        }
        if issues.iter().any(|i| i.contains("Proton")) {
            println!("  - Install GE-Proton via ProtonUp-Qt or Steam");
        }
    }

    // Export if requested
    if let Some(export_path) = args.export {
        let report = format!(
            "WANDA Diagnostic Report\n\nIssues:\n{}\n",
            issues.join("\n")
        );
        std::fs::write(&export_path, report)?;
        println!("\nReport exported to: {}", export_path.display());
    }

    Ok(())
}

fn print_section(name: &str) {
    println!("\n{}", style(format!("[{}]", name)).bold());
}

async fn check_command(cmd: &str, name: &str, issues: &mut Vec<String>) {
    let result = tokio::process::Command::new("which")
        .arg(cmd)
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {
            let path = String::from_utf8_lossy(&output.stdout);
            println!("  {} {} ({})", style("OK").green(), name, path.trim());
        }
        _ => {
            println!("  {} {} not found", style("WARN").yellow(), name);
            issues.push(format!("{} not found in PATH", name));
        }
    }
}
