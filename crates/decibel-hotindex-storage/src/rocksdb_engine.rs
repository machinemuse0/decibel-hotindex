use crate::engine::StorageEngine;
use crate::key;
use decibel_hotindex_core::{
    ActivityRow, BuilderAttributionRow, BuilderVolumeRow, CfChecksum, FillRow, HotIndexError,
    IngestCheckpoint, NormalizedEvent, OrderRow, PositionRow, Result, StorageStats, TimeWindow,
    TxRow,
};
use rocksdb::{ColumnFamilyDescriptor, Direction, IteratorMode, Options, DB};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

const CF_TX_BY_VERSION: &str = "cf_tx_by_version";
const CF_RAW_EVENT_BY_VERSION_IDX: &str = "cf_raw_event_by_version_idx";
const CF_FILLS_BY_MARKET_TIME: &str = "cf_fills_by_market_time";
const CF_FILLS_BY_ACCOUNT_TIME: &str = "cf_fills_by_account_time";
const CF_ORDER_BY_ID: &str = "cf_order_by_id";
const CF_POSITIONS_BY_ACCOUNT_MARKET: &str = "cf_positions_by_account_market";
const CF_BUILDER_CODE_FILLS: &str = "cf_builder_code_fills";
const CF_MARKET_RECENT_ACTIVITY: &str = "cf_market_recent_activity";
const CF_INGEST_CHECKPOINT: &str = "cf_ingest_checkpoint";

const LOGICAL_CFS: &[&str] = &[
    CF_TX_BY_VERSION,
    CF_RAW_EVENT_BY_VERSION_IDX,
    CF_FILLS_BY_MARKET_TIME,
    CF_FILLS_BY_ACCOUNT_TIME,
    CF_ORDER_BY_ID,
    CF_POSITIONS_BY_ACCOUNT_MARKET,
    CF_BUILDER_CODE_FILLS,
    CF_INGEST_CHECKPOINT,
];

#[derive(Debug)]
pub struct RocksDbEngine {
    db: DB,
}

impl RocksDbEngine {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);

        let cf_opts = Options::default();
        let descriptors = [
            CF_TX_BY_VERSION,
            CF_RAW_EVENT_BY_VERSION_IDX,
            CF_FILLS_BY_MARKET_TIME,
            CF_FILLS_BY_ACCOUNT_TIME,
            CF_ORDER_BY_ID,
            CF_POSITIONS_BY_ACCOUNT_MARKET,
            CF_BUILDER_CODE_FILLS,
            CF_MARKET_RECENT_ACTIVITY,
            CF_INGEST_CHECKPOINT,
        ]
        .into_iter()
        .map(|name| ColumnFamilyDescriptor::new(name, cf_opts.clone()));

        let db = DB::open_cf_descriptors(&db_opts, path, descriptors).map_err(rocks_error)?;
        Ok(Self { db })
    }

    fn cf(&self, name: &str) -> Result<&rocksdb::ColumnFamily> {
        self.db
            .cf_handle(name)
            .ok_or_else(|| HotIndexError::Storage(format!("missing RocksDB column family {name}")))
    }

    fn put_json<T: Serialize>(&self, cf_name: &str, key: Vec<u8>, value: &T) -> Result<()> {
        let bytes = serde_json::to_vec(value).map_err(json_error)?;
        self.db
            .put_cf(self.cf(cf_name)?, key, bytes)
            .map_err(rocks_error)
    }

    fn get_json<T: DeserializeOwned>(&self, cf_name: &str, key: Vec<u8>) -> Result<Option<T>> {
        let Some(bytes) = self
            .db
            .get_cf(self.cf(cf_name)?, key)
            .map_err(rocks_error)?
        else {
            return Ok(None);
        };
        serde_json::from_slice(&bytes).map(Some).map_err(json_error)
    }

    fn scan_prefix<T: DeserializeOwned>(
        &self,
        cf_name: &str,
        prefix: Vec<u8>,
        limit: usize,
    ) -> Result<Vec<T>> {
        let cf = self.cf(cf_name)?;
        let iter = self
            .db
            .iterator_cf(cf, IteratorMode::From(&prefix, Direction::Forward));
        let mut rows = Vec::new();
        for item in iter {
            let (key, value) = item.map_err(rocks_error)?;
            if !key.starts_with(&prefix) {
                break;
            }
            rows.push(serde_json::from_slice(&value).map_err(json_error)?);
            if rows.len() >= limit {
                break;
            }
        }
        Ok(rows)
    }

    fn cf_len(&self, cf_name: &str) -> Result<u64> {
        let cf = self.cf(cf_name)?;
        let mut count = 0_u64;
        for item in self.db.iterator_cf(cf, IteratorMode::Start) {
            item.map_err(rocks_error)?;
            count += 1;
        }
        Ok(count)
    }
}

impl StorageEngine for RocksDbEngine {
    fn put_tx(&self, tx: TxRow) -> Result<()> {
        self.put_json(CF_TX_BY_VERSION, key::tx_by_version(tx.version), &tx)
    }

    fn put_event(&self, event: NormalizedEvent) -> Result<()> {
        self.put_json(
            CF_RAW_EVENT_BY_VERSION_IDX,
            key::raw_event_by_version_idx(event.version, event.event_idx),
            &event,
        )
    }

    fn put_fill(&self, fill: FillRow) -> Result<()> {
        self.put_json(
            CF_FILLS_BY_MARKET_TIME,
            key::fills_by_market_time(
                &fill.market_id,
                fill.timestamp_us,
                fill.version,
                &fill.fill_id,
            ),
            &fill,
        )?;
        self.put_json(
            CF_FILLS_BY_ACCOUNT_TIME,
            key::fills_by_account_time(
                &fill.account,
                fill.timestamp_us,
                &fill.market_id,
                &fill.fill_id,
            ),
            &fill,
        )?;
        let activity = ActivityRow {
            market_id: fill.market_id.clone(),
            activity_type: "fill".to_string(),
            version: fill.version,
            event_idx: fill.event_idx,
            timestamp_us: fill.timestamp_us,
            summary: fill.fill_id.clone(),
        };
        self.put_activity(activity)
    }

    fn put_order(&self, order: OrderRow) -> Result<()> {
        self.put_json(CF_ORDER_BY_ID, key::order_by_id(&order.order_id), &order)
    }

    fn put_position(&self, position: PositionRow) -> Result<()> {
        self.put_json(
            CF_POSITIONS_BY_ACCOUNT_MARKET,
            key::positions_by_account_market(&position.account, &position.market_id),
            &position,
        )
    }

    fn put_builder_attribution(&self, row: BuilderAttributionRow) -> Result<()> {
        self.put_json(
            CF_BUILDER_CODE_FILLS,
            key::builder_code_fills(
                &row.builder_addr,
                row.timestamp_us,
                &row.market_id,
                &row.fill_id,
            ),
            &row,
        )
    }

    fn put_activity(&self, row: ActivityRow) -> Result<()> {
        self.put_json(
            CF_MARKET_RECENT_ACTIVITY,
            key::market_activity(
                &row.market_id,
                row.timestamp_us,
                &row.activity_type,
                row.version,
                row.event_idx,
            ),
            &row,
        )
    }

    fn put_ingest_checkpoint(&self, checkpoint: IngestCheckpoint) -> Result<()> {
        self.put_json(
            CF_INGEST_CHECKPOINT,
            key::ingest_checkpoint(checkpoint.network, &checkpoint.package_address),
            &checkpoint,
        )
    }

    fn get_tx(&self, version: u64) -> Result<Option<TxRow>> {
        self.get_json(CF_TX_BY_VERSION, key::tx_by_version(version))
    }

    fn multi_get_txs(&self, versions: &[u64]) -> Result<Vec<Option<TxRow>>> {
        versions
            .iter()
            .map(|version| self.get_tx(*version))
            .collect()
    }

    fn get_order(&self, order_id: &str) -> Result<Option<OrderRow>> {
        self.get_json(CF_ORDER_BY_ID, key::order_by_id(order_id))
    }

    fn get_positions_by_account(&self, account: &str) -> Result<Vec<PositionRow>> {
        self.scan_prefix(
            CF_POSITIONS_BY_ACCOUNT_MARKET,
            key::positions_by_account_prefix(account),
            usize::MAX,
        )
    }

    fn scan_market_fills(&self, market_id: &str, limit: usize) -> Result<Vec<FillRow>> {
        self.scan_prefix(
            CF_FILLS_BY_MARKET_TIME,
            key::fills_by_market_prefix(market_id),
            limit,
        )
    }

    fn scan_account_fills(&self, account: &str, limit: usize) -> Result<Vec<FillRow>> {
        self.scan_prefix(
            CF_FILLS_BY_ACCOUNT_TIME,
            key::fills_by_account_prefix(account),
            limit,
        )
    }

    fn scan_builder_code_fills(
        &self,
        builder_addr: &str,
        limit: usize,
    ) -> Result<Vec<BuilderAttributionRow>> {
        self.scan_prefix(
            CF_BUILDER_CODE_FILLS,
            key::builder_code_fills_prefix(builder_addr),
            limit,
        )
    }

    fn get_builder_code_volume(
        &self,
        builder_addr: &str,
        window: TimeWindow,
    ) -> Result<Option<BuilderVolumeRow>> {
        let rows = self.scan_builder_code_fills(builder_addr, usize::MAX)?;
        if rows.is_empty() {
            return Ok(None);
        }

        let max_ts = rows
            .iter()
            .map(|row| row.timestamp_us)
            .max()
            .unwrap_or_default();
        let window_start_ts_us = max_ts
            .saturating_sub(window.duration_us())
            .saturating_add(1);
        let mut trades = 0_u64;
        let mut accounts = BTreeSet::new();
        let mut notional = DecimalAccumulator::default();
        let mut fee_share = DecimalAccumulator::default();
        let mut has_fee_share = false;

        for row in rows
            .iter()
            .filter(|row| row.timestamp_us >= window_start_ts_us)
        {
            trades += 1;
            accounts.insert(row.account.clone());
            if let Some(value) = &row.notional {
                notional.add(value);
            }
            if let Some(value) = &row.estimated_fee_amount {
                fee_share.add(value);
                has_fee_share = true;
            }
        }

        Ok(Some(BuilderVolumeRow {
            builder_addr: builder_addr.to_string(),
            window,
            window_start_ts_us,
            notional_volume: notional.to_string(),
            trades,
            active_accounts: accounts.len() as u64,
            estimated_fee_share: has_fee_share.then(|| fee_share.to_string()),
            source: "parsed_decibel_events".to_string(),
            disclaimer: "analytics estimate; not official settlement statement".to_string(),
        }))
    }

    fn scan_market_activity(&self, market_id: &str, limit: usize) -> Result<Vec<ActivityRow>> {
        self.scan_prefix(
            CF_MARKET_RECENT_ACTIVITY,
            key::market_activity_prefix(market_id),
            limit,
        )
    }

    fn stats(&self) -> Result<StorageStats> {
        Ok(StorageStats {
            tx_count: self.cf_len(CF_TX_BY_VERSION)?,
            event_count: self.cf_len(CF_RAW_EVENT_BY_VERSION_IDX)?,
            fill_count: self.cf_len(CF_FILLS_BY_MARKET_TIME)?,
            order_count: self.cf_len(CF_ORDER_BY_ID)?,
            position_count: self.cf_len(CF_POSITIONS_BY_ACCOUNT_MARKET)?,
            builder_attribution_count: self.cf_len(CF_BUILDER_CODE_FILLS)?,
            checkpoint_count: self.cf_len(CF_INGEST_CHECKPOINT)?,
        })
    }

    fn checksums(&self) -> Result<Vec<CfChecksum>> {
        LOGICAL_CFS
            .iter()
            .map(|cf_name| checksum_cf(self, cf_name))
            .collect()
    }
}

fn checksum_cf(engine: &RocksDbEngine, cf_name: &str) -> Result<CfChecksum> {
    let cf = engine.cf(cf_name)?;
    let mut hash = StableHasher::default();
    let mut row_count = 0_u64;

    for item in engine.db.iterator_cf(cf, IteratorMode::Start) {
        let (key, value) = item.map_err(rocks_error)?;
        hash.update(&key);
        hash.update(&[0xff]);
        hash.update(debug_value(cf_name, &value)?.as_bytes());
        hash.update(&[0xfe]);
        row_count += 1;
    }

    Ok(CfChecksum {
        cf_name: cf_name.to_string(),
        row_count,
        hash_hex: format!("{:016x}", hash.finish()),
    })
}

fn debug_value(cf_name: &str, value: &[u8]) -> Result<String> {
    match cf_name {
        CF_TX_BY_VERSION => Ok(format!(
            "{:?}",
            serde_json::from_slice::<TxRow>(value).map_err(json_error)?
        )),
        CF_RAW_EVENT_BY_VERSION_IDX => Ok(format!(
            "{:?}",
            serde_json::from_slice::<NormalizedEvent>(value).map_err(json_error)?
        )),
        CF_FILLS_BY_MARKET_TIME | CF_FILLS_BY_ACCOUNT_TIME => Ok(format!(
            "{:?}",
            serde_json::from_slice::<FillRow>(value).map_err(json_error)?
        )),
        CF_ORDER_BY_ID => Ok(format!(
            "{:?}",
            serde_json::from_slice::<OrderRow>(value).map_err(json_error)?
        )),
        CF_POSITIONS_BY_ACCOUNT_MARKET => Ok(format!(
            "{:?}",
            serde_json::from_slice::<PositionRow>(value).map_err(json_error)?
        )),
        CF_BUILDER_CODE_FILLS => Ok(format!(
            "{:?}",
            serde_json::from_slice::<BuilderAttributionRow>(value).map_err(json_error)?
        )),
        CF_INGEST_CHECKPOINT => Ok(format!(
            "{:?}",
            serde_json::from_slice::<IngestCheckpoint>(value).map_err(json_error)?
        )),
        other => Err(HotIndexError::Storage(format!(
            "unsupported checksum column family {other}"
        ))),
    }
}

fn rocks_error(error: rocksdb::Error) -> HotIndexError {
    HotIndexError::Storage(error.to_string())
}

fn json_error(error: serde_json::Error) -> HotIndexError {
    HotIndexError::Parse(error.to_string())
}

#[derive(Debug, Clone)]
struct StableHasher {
    state: u64,
}

impl Default for StableHasher {
    fn default() -> Self {
        Self {
            state: 0xcbf2_9ce4_8422_2325,
        }
    }
}

impl StableHasher {
    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.state ^= u64::from(*byte);
            self.state = self.state.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(self) -> u64 {
        self.state
    }
}

#[derive(Debug, Default)]
struct DecimalAccumulator {
    units: i128,
    scale: u32,
}

impl DecimalAccumulator {
    fn add(&mut self, value: &str) {
        if let Some((units, scale)) = parse_decimal(value) {
            while self.scale < scale {
                self.units *= 10;
                self.scale += 1;
            }
            let mut aligned = units;
            for _ in scale..self.scale {
                aligned *= 10;
            }
            self.units += aligned;
        }
    }
}

impl std::fmt::Display for DecimalAccumulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.scale == 0 {
            return write!(f, "{}", self.units);
        }

        let sign = if self.units < 0 { "-" } else { "" };
        let digits = self.units.abs().to_string();
        let scale = self.scale as usize;
        if digits.len() <= scale {
            let zeros = "0".repeat(scale - digits.len());
            let mut fractional = format!("{zeros}{digits}");
            trim_trailing_zeros(&mut fractional);
            if fractional.is_empty() {
                write!(f, "{sign}0")
            } else {
                write!(f, "{sign}0.{fractional}")
            }
        } else {
            let split = digits.len() - scale;
            let (whole, fractional) = digits.split_at(split);
            let mut fractional = fractional.to_string();
            trim_trailing_zeros(&mut fractional);
            if fractional.is_empty() {
                write!(f, "{sign}{whole}")
            } else {
                write!(f, "{sign}{whole}.{fractional}")
            }
        }
    }
}

fn parse_decimal(value: &str) -> Option<(i128, u32)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let negative = trimmed.starts_with('-');
    let body = trimmed.strip_prefix('-').unwrap_or(trimmed);
    let mut parts = body.split('.');
    let whole = parts.next()?;
    let fractional = parts.next();
    if parts.next().is_some() {
        return None;
    }
    if whole.is_empty() && fractional.unwrap_or_default().is_empty() {
        return None;
    }
    if !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fractional
            .unwrap_or_default()
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let scale = fractional.map_or(0, |value| value.len() as u32);
    let digits = format!("{whole}{}", fractional.unwrap_or_default());
    let mut units = digits.parse::<i128>().ok()?;
    if negative {
        units = -units;
    }
    Some((units, scale))
}

fn trim_trailing_zeros(value: &mut String) {
    while value.ends_with('0') {
        value.pop();
    }
}
