use decibel_hotindex_core::{
    BuilderAttributionRow, BuilderVolumeRow, CfChecksum, FillRow, IngestCheckpoint,
    NormalizedEvent, OrderRow, PositionRow, Result, StorageStats, TimeWindow, TxRow,
};

pub trait StorageEngine: Send + Sync + 'static {
    fn put_tx(&self, tx: TxRow) -> Result<()>;
    fn put_event(&self, event: NormalizedEvent) -> Result<()>;
    fn put_fill(&self, fill: FillRow) -> Result<()>;
    fn put_order(&self, order: OrderRow) -> Result<()>;
    fn put_position(&self, position: PositionRow) -> Result<()>;
    fn put_builder_attribution(&self, row: BuilderAttributionRow) -> Result<()>;
    fn put_activity(&self, row: decibel_hotindex_core::ActivityRow) -> Result<()>;
    fn put_ingest_checkpoint(&self, checkpoint: IngestCheckpoint) -> Result<()>;

    fn get_tx(&self, version: u64) -> Result<Option<TxRow>>;
    fn multi_get_txs(&self, versions: &[u64]) -> Result<Vec<Option<TxRow>>>;
    fn get_order(&self, order_id: &str) -> Result<Option<OrderRow>>;
    fn get_positions_by_account(&self, account: &str) -> Result<Vec<PositionRow>>;

    fn scan_market_fills(&self, market_id: &str, limit: usize) -> Result<Vec<FillRow>>;
    fn scan_account_fills(&self, account: &str, limit: usize) -> Result<Vec<FillRow>>;
    fn scan_builder_code_fills(
        &self,
        builder_addr: &str,
        limit: usize,
    ) -> Result<Vec<BuilderAttributionRow>>;
    fn get_builder_code_volume(
        &self,
        builder_addr: &str,
        window: TimeWindow,
    ) -> Result<Option<BuilderVolumeRow>>;
    fn scan_market_activity(
        &self,
        market_id: &str,
        limit: usize,
    ) -> Result<Vec<decibel_hotindex_core::ActivityRow>>;

    fn stats(&self) -> Result<StorageStats>;
    fn checksums(&self) -> Result<Vec<CfChecksum>>;
}
