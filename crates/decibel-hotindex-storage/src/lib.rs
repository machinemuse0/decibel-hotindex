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

pub fn crate_status() -> &'static str {
    decibel_hotindex_core::crate_status()
}

#[cfg(test)]
mod tests;
