//! Steam integration module
//!
//! Handles Steam library discovery, game detection, and Proton version management.

mod library;
mod proton;
mod vdf;

pub use library::{SteamApp, SteamInstallation, SteamLibrary};
pub use proton::{ProtonCompatibility, ProtonManager, ProtonVersion};
pub use vdf::parse_vdf_file;
