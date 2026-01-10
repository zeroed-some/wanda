//! WeMod download and installation management

mod downloader;
mod installer;

pub use downloader::{WemodDownloader, WemodRelease};
pub use installer::WemodInstaller;
