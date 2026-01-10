//! WANDA Core Library
//!
//! Core functionality for running WeMod on Linux via Wine/Proton.

pub mod config;
pub mod error;
pub mod launcher;
pub mod prefix;
pub mod steam;
pub mod wemod;

pub use config::WandaConfig;
pub use error::{Result, WandaError};
