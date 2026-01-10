//! WeMod download functionality

use crate::config::WandaConfig;
use crate::error::{Result, WandaError};
use futures::StreamExt;
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

/// WeMod download API endpoint (latest version - may not work on Linux)
const WEMOD_DOWNLOAD_URL: &str = "https://api.wemod.com/client/download";

/// Pinned WeMod version known to work on Linux with Wine/Proton
/// Version 12.x has known compatibility issues (black window, renderer crashes)
/// See: https://community.wemod.com/t/wand-wemod-version-12-0-3-on-linux-proton-just-displays-a-black-window/373133
const WEMOD_LINUX_COMPATIBLE_VERSION: &str = "11.5.0";
const WEMOD_LINUX_COMPATIBLE_URL: &str =
    "https://storage-cdn.wemod.com/app/releases/stable/WeMod-11.5.0.exe";

/// Information about a WeMod release
#[derive(Debug, Clone)]
pub struct WemodRelease {
    /// Download URL
    pub url: String,
    /// Version string (if known)
    pub version: Option<String>,
    /// File size in bytes (if known)
    pub size: Option<u64>,
}

/// Handles downloading WeMod
pub struct WemodDownloader {
    /// HTTP client
    client: reqwest::Client,
    /// Cache directory for downloads
    cache_dir: PathBuf,
}

impl WemodDownloader {
    /// Create a new downloader
    pub fn new(_config: &WandaConfig) -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("WANDA/0.1.0")
                .build()
                .expect("Failed to create HTTP client"),
            cache_dir: WandaConfig::default_cache_dir().join("downloads"),
        }
    }

    /// Get the Linux-compatible WeMod release info
    ///
    /// This returns a pinned version (11.5.0) known to work with Wine/Proton.
    /// Version 12.x has compatibility issues on Linux (black window, renderer crashes).
    pub async fn get_latest(&self) -> Result<WemodRelease> {
        info!("Using WeMod {} (pinned for Linux compatibility)", WEMOD_LINUX_COMPATIBLE_VERSION);
        warn!("WeMod 12.x has known Linux compatibility issues - using 11.5.0 instead");

        // Use the pinned Linux-compatible version instead of fetching latest
        // The latest 12.x versions have renderer crashes under Wine/Proton
        let url = WEMOD_LINUX_COMPATIBLE_URL.to_string();
        let version = Some(WEMOD_LINUX_COMPATIBLE_VERSION.to_string());

        // Get the file size via HEAD request
        let content_length = self
            .client
            .head(&url)
            .send()
            .await
            .ok()
            .and_then(|r| r.headers().get(reqwest::header::CONTENT_LENGTH).cloned())
            .and_then(|v| v.to_str().ok().and_then(|s| s.parse().ok()));

        debug!("WeMod download URL: {}", url);
        debug!("Using pinned version: {}", WEMOD_LINUX_COMPATIBLE_VERSION);

        Ok(WemodRelease {
            url,
            version,
            size: content_length,
        })
    }

    /// Get the actual latest WeMod release (may not work on Linux)
    ///
    /// WARNING: Version 12.x has known compatibility issues with Wine/Proton
    /// Use `get_latest()` for Linux-compatible version
    #[allow(dead_code)]
    pub async fn get_latest_upstream(&self) -> Result<WemodRelease> {
        info!("Fetching upstream WeMod download info...");
        warn!("Latest WeMod may have Linux compatibility issues!");

        // The WeMod API redirects to the actual download URL
        // We need to follow the redirect to get the final URL
        let response = self
            .client
            .head(WEMOD_DOWNLOAD_URL)
            .send()
            .await
            .map_err(|e| WandaError::WemodDownloadFailed {
                reason: format!("Failed to fetch download info: {}", e),
            })?;

        let final_url = response.url().to_string();
        let content_length = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());

        // Try to extract version from URL
        // URL format is typically: https://storage.wemod.com/app/releases/stable/WeMod-X.Y.Z.exe
        let version = final_url
            .split('/')
            .last()
            .and_then(|filename| {
                filename
                    .strip_prefix("WeMod-")
                    .and_then(|v| v.strip_suffix(".exe"))
                    .map(String::from)
            });

        debug!("WeMod download URL: {}", final_url);
        if let Some(ref v) = version {
            debug!("Detected version: {}", v);
        }

        Ok(WemodRelease {
            url: final_url,
            version,
            size: content_length,
        })
    }

    /// Download WeMod installer to cache
    pub async fn download<F>(&self, release: &WemodRelease, progress_callback: F) -> Result<PathBuf>
    where
        F: Fn(u64, u64),
    {
        // Ensure cache directory exists
        tokio::fs::create_dir_all(&self.cache_dir).await?;

        let filename = release
            .version
            .as_ref()
            .map(|v| format!("WeMod-{}.exe", v))
            .unwrap_or_else(|| "WeMod-latest.exe".to_string());

        let dest_path = self.cache_dir.join(&filename);

        // Check if we already have this version cached
        if dest_path.exists() {
            if let Some(expected_size) = release.size {
                let actual_size = tokio::fs::metadata(&dest_path).await?.len();
                if actual_size == expected_size {
                    info!("Using cached WeMod installer: {}", dest_path.display());
                    return Ok(dest_path);
                }
            }
        }

        info!("Downloading WeMod from {}...", release.url);

        let response = self
            .client
            .get(&release.url)
            .send()
            .await
            .map_err(|e| WandaError::WemodDownloadFailed {
                reason: format!("Download request failed: {}", e),
            })?;

        if !response.status().is_success() {
            return Err(WandaError::WemodDownloadFailed {
                reason: format!("HTTP error: {}", response.status()),
            });
        }

        let total_size = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        let mut file = File::create(&dest_path).await?;
        let mut stream = response.bytes_stream();

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| WandaError::WemodDownloadFailed {
                reason: format!("Error reading download stream: {}", e),
            })?;

            file.write_all(&chunk).await?;

            downloaded += chunk.len() as u64;
            progress_callback(downloaded, total_size);
        }

        file.flush().await?;

        info!("Downloaded WeMod to {}", dest_path.display());
        Ok(dest_path)
    }

    /// Check if we have a cached version matching the release
    pub async fn get_cached(&self, release: &WemodRelease) -> Option<PathBuf> {
        if let Some(ref version) = release.version {
            let path = self.cache_dir.join(format!("WeMod-{}.exe", version));
            if path.exists() {
                return Some(path);
            }
        }
        None
    }

    /// Clear the download cache
    pub async fn clear_cache(&self) -> Result<()> {
        if self.cache_dir.exists() {
            tokio::fs::remove_dir_all(&self.cache_dir).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_extraction() {
        let url = "https://storage.wemod.com/app/releases/stable/WeMod-8.12.5.exe";
        let version = url
            .split('/')
            .last()
            .and_then(|f| f.strip_prefix("WeMod-"))
            .and_then(|v| v.strip_suffix(".exe"));

        assert_eq!(version, Some("8.12.5"));
    }
}
