//! Wine/Proton prefix management
//!
//! Handles creation, validation, and repair of Wine prefixes for WeMod.

mod builder;
mod manager;

pub use builder::PrefixBuilder;
pub use manager::{PrefixHealth, PrefixIssue, PrefixManager, WandaPrefix};
