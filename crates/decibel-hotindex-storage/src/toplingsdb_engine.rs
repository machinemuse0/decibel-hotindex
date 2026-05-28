use crate::engine::StorageEngine;
use crate::RocksDbEngine;
use decibel_hotindex_core::{
    ActivityRow, BuilderAttributionRow, BuilderVolumeRow, CfChecksum, FillRow, HotIndexError,
    IngestCheckpoint, NormalizedEvent, OrderRow, PositionRow, Result, StorageStats, TimeWindow,
    TxRow,
};
use std::env;
use std::path::{Path, PathBuf};

pub const TOPLINGDB_EASY_MIGRATE_CONF_ENV: &str = "TOPLINGDB_EASY_MIGRATE_CONF";

#[derive(Debug)]
pub struct ToplingDbEngine {
    _config_path: PathBuf,
    inner: RocksDbEngine,
}

impl ToplingDbEngine {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let config_path = env::var_os(TOPLINGDB_EASY_MIGRATE_CONF_ENV)
            .map(PathBuf::from)
            .ok_or_else(|| {
                HotIndexError::Config(format!(
                    "ToplingDB backend requires {TOPLINGDB_EASY_MIGRATE_CONF_ENV} to point at a ToplingDB YAML config"
                ))
            })?;
        if !config_path.is_file() {
            return Err(HotIndexError::Config(format!(
                "{TOPLINGDB_EASY_MIGRATE_CONF_ENV} does not point to a readable file: {}",
                config_path.display()
            )));
        }

        Ok(Self {
            _config_path: config_path,
            inner: RocksDbEngine::open(path)?,
        })
    }
}

impl StorageEngine for ToplingDbEngine {
    fn put_tx(&self, tx: TxRow) -> Result<()> {
        self.inner.put_tx(tx)
    }

    fn put_event(&self, event: NormalizedEvent) -> Result<()> {
        self.inner.put_event(event)
    }

    fn put_fill(&self, fill: FillRow) -> Result<()> {
        self.inner.put_fill(fill)
    }

    fn put_order(&self, order: OrderRow) -> Result<()> {
        self.inner.put_order(order)
    }

    fn put_position(&self, position: PositionRow) -> Result<()> {
        self.inner.put_position(position)
    }

    fn put_builder_attribution(&self, row: BuilderAttributionRow) -> Result<()> {
        self.inner.put_builder_attribution(row)
    }

    fn put_activity(&self, row: ActivityRow) -> Result<()> {
        self.inner.put_activity(row)
    }

    fn put_ingest_checkpoint(&self, checkpoint: IngestCheckpoint) -> Result<()> {
        self.inner.put_ingest_checkpoint(checkpoint)
    }

    fn get_tx(&self, version: u64) -> Result<Option<TxRow>> {
        self.inner.get_tx(version)
    }

    fn multi_get_txs(&self, versions: &[u64]) -> Result<Vec<Option<TxRow>>> {
        self.inner.multi_get_txs(versions)
    }

    fn get_order(&self, order_id: &str) -> Result<Option<OrderRow>> {
        self.inner.get_order(order_id)
    }

    fn get_positions_by_account(&self, account: &str) -> Result<Vec<PositionRow>> {
        self.inner.get_positions_by_account(account)
    }

    fn scan_market_fills(&self, market_id: &str, limit: usize) -> Result<Vec<FillRow>> {
        self.inner.scan_market_fills(market_id, limit)
    }

    fn scan_account_fills(&self, account: &str, limit: usize) -> Result<Vec<FillRow>> {
        self.inner.scan_account_fills(account, limit)
    }

    fn scan_builder_code_fills(
        &self,
        builder_addr: &str,
        limit: usize,
    ) -> Result<Vec<BuilderAttributionRow>> {
        self.inner.scan_builder_code_fills(builder_addr, limit)
    }

    fn get_builder_code_volume(
        &self,
        builder_addr: &str,
        window: TimeWindow,
    ) -> Result<Option<BuilderVolumeRow>> {
        self.inner.get_builder_code_volume(builder_addr, window)
    }

    fn scan_market_activity(&self, market_id: &str, limit: usize) -> Result<Vec<ActivityRow>> {
        self.inner.scan_market_activity(market_id, limit)
    }

    fn stats(&self) -> Result<StorageStats> {
        self.inner.stats()
    }

    fn checksums(&self) -> Result<Vec<CfChecksum>> {
        self.inner.checksums()
    }
}
