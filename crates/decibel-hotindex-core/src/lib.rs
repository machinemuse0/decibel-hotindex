//! Core types and helpers for Decibel HotIndex.

pub mod address;
pub mod config;
pub mod error;
pub mod time;
pub mod types;

pub use address::normalize_aptos_address;
pub use config::{ApiConfig, AppConfig, AptosConfig, BenchConfig, DecibelConfig, StorageConfig};
pub use error::{HotIndexError, Result};
pub use time::reverse_ts_us;
pub use types::*;

pub const CRATE_STATUS: &str = "milestone-1-core";

pub fn crate_status() -> &'static str {
    CRATE_STATUS
}
