//! Storage engine contract and backend implementations for Decibel HotIndex.

pub mod engine;
pub mod key;
pub mod memory_engine;
#[cfg(feature = "rocksdb")]
pub mod rocksdb_engine;
#[cfg(feature = "toplingsdb")]
pub mod toplingsdb_engine;

pub use engine::StorageEngine;
pub use memory_engine::MemoryEngine;
#[cfg(feature = "rocksdb")]
pub use rocksdb_engine::RocksDbEngine;
#[cfg(feature = "toplingsdb")]
pub use toplingsdb_engine::ToplingDbEngine;

pub const CF_TX_BY_VERSION: &str = "cf_tx_by_version";
pub const CF_RAW_EVENT_BY_VERSION_IDX: &str = "cf_raw_event_by_version_idx";
pub const CF_FILLS_BY_MARKET_TIME: &str = "cf_fills_by_market_time";
pub const CF_FILLS_BY_ACCOUNT_TIME: &str = "cf_fills_by_account_time";
pub const CF_ORDER_BY_ID: &str = "cf_order_by_id";
pub const CF_POSITIONS_BY_ACCOUNT_MARKET: &str = "cf_positions_by_account_market";
pub const CF_BUILDER_CODE_FILLS: &str = "cf_builder_code_fills";
pub const CF_MARKET_RECENT_ACTIVITY: &str = "cf_market_recent_activity";
pub const CF_INGEST_CHECKPOINT: &str = "cf_ingest_checkpoint";

pub const LOGICAL_CFS: &[&str] = &[
    CF_TX_BY_VERSION,
    CF_RAW_EVENT_BY_VERSION_IDX,
    CF_FILLS_BY_MARKET_TIME,
    CF_FILLS_BY_ACCOUNT_TIME,
    CF_ORDER_BY_ID,
    CF_POSITIONS_BY_ACCOUNT_MARKET,
    CF_BUILDER_CODE_FILLS,
    CF_MARKET_RECENT_ACTIVITY,
    CF_INGEST_CHECKPOINT,
];

pub fn crate_status() -> &'static str {
    decibel_hotindex_core::crate_status()
}

#[cfg(test)]
mod tests;
