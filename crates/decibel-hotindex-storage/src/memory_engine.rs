use crate::engine::StorageEngine;
use crate::key;
use decibel_hotindex_core::{
    ActivityRow, BuilderAttributionRow, BuilderVolumeRow, CfChecksum, FillRow, HotIndexError,
    IngestCheckpoint, NormalizedEvent, OrderRow, PositionRow, Result, StorageStats, TimeWindow,
    TxRow,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

#[derive(Debug, Default)]
pub struct MemoryEngine {
    inner: RwLock<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    tx_by_version: BTreeMap<u64, TxRow>,
    events_by_version_idx: BTreeMap<(u64, u32), NormalizedEvent>,
    fills_by_id: BTreeMap<String, FillRow>,
    orders_by_id: BTreeMap<String, OrderRow>,
    positions_by_account_market: BTreeMap<(String, String), PositionRow>,
    builder_attributions_by_fill: BTreeMap<(String, String), BuilderAttributionRow>,
    checkpoints: BTreeMap<String, IngestCheckpoint>,
    activities: Vec<ActivityRow>,
}

impl StorageEngine for MemoryEngine {
    fn put_tx(&self, tx: TxRow) -> Result<()> {
        self.write()?.tx_by_version.insert(tx.version, tx);
        Ok(())
    }

    fn put_event(&self, event: NormalizedEvent) -> Result<()> {
        self.write()?
            .events_by_version_idx
            .insert((event.version, event.event_idx), event);
        Ok(())
    }

    fn put_fill(&self, fill: FillRow) -> Result<()> {
        let activity = ActivityRow {
            market_id: fill.market_id.clone(),
            activity_type: "fill".to_string(),
            version: fill.version,
            event_idx: fill.event_idx,
            timestamp_us: fill.timestamp_us,
            summary: fill.fill_id.clone(),
        };
        let mut inner = self.write()?;
        inner.fills_by_id.insert(fill.fill_id.clone(), fill);
        inner.activities.push(activity);
        Ok(())
    }

    fn put_order(&self, order: OrderRow) -> Result<()> {
        self.write()?
            .orders_by_id
            .insert(order.order_id.clone(), order);
        Ok(())
    }

    fn put_position(&self, position: PositionRow) -> Result<()> {
        self.write()?.positions_by_account_market.insert(
            (position.account.clone(), position.market_id.clone()),
            position,
        );
        Ok(())
    }

    fn put_builder_attribution(&self, row: BuilderAttributionRow) -> Result<()> {
        self.write()?
            .builder_attributions_by_fill
            .insert((row.builder_addr.clone(), row.fill_id.clone()), row);
        Ok(())
    }

    fn put_activity(&self, row: ActivityRow) -> Result<()> {
        self.write()?.activities.push(row);
        Ok(())
    }

    fn put_ingest_checkpoint(&self, checkpoint: IngestCheckpoint) -> Result<()> {
        let key = checkpoint_key(&checkpoint);
        self.write()?.checkpoints.insert(key, checkpoint);
        Ok(())
    }

    fn get_tx(&self, version: u64) -> Result<Option<TxRow>> {
        Ok(self.read()?.tx_by_version.get(&version).cloned())
    }

    fn multi_get_txs(&self, versions: &[u64]) -> Result<Vec<Option<TxRow>>> {
        let inner = self.read()?;
        Ok(versions
            .iter()
            .map(|version| inner.tx_by_version.get(version).cloned())
            .collect())
    }

    fn get_order(&self, order_id: &str) -> Result<Option<OrderRow>> {
        Ok(self.read()?.orders_by_id.get(order_id).cloned())
    }

    fn get_positions_by_account(&self, account: &str) -> Result<Vec<PositionRow>> {
        let mut rows: Vec<_> = self
            .read()?
            .positions_by_account_market
            .values()
            .filter(|position| position.account == account)
            .cloned()
            .collect();
        rows.sort_by(|a, b| a.market_id.cmp(&b.market_id));
        Ok(rows)
    }

    fn scan_market_fills(&self, market_id: &str, limit: usize) -> Result<Vec<FillRow>> {
        let mut rows: Vec<_> = self
            .read()?
            .fills_by_id
            .values()
            .filter(|fill| fill.market_id == market_id)
            .cloned()
            .collect();
        sort_fills_recent_first(&mut rows);
        rows.truncate(limit);
        Ok(rows)
    }

    fn scan_account_fills(&self, account: &str, limit: usize) -> Result<Vec<FillRow>> {
        let mut rows: Vec<_> = self
            .read()?
            .fills_by_id
            .values()
            .filter(|fill| fill.account == account)
            .cloned()
            .collect();
        sort_fills_recent_first(&mut rows);
        rows.truncate(limit);
        Ok(rows)
    }

    fn scan_builder_code_fills(
        &self,
        builder_addr: &str,
        limit: usize,
    ) -> Result<Vec<BuilderAttributionRow>> {
        let mut rows: Vec<_> = self
            .read()?
            .builder_attributions_by_fill
            .values()
            .filter(|row| row.builder_addr == builder_addr)
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            b.timestamp_us
                .cmp(&a.timestamp_us)
                .then_with(|| a.fill_id.cmp(&b.fill_id))
        });
        rows.truncate(limit);
        Ok(rows)
    }

    fn get_builder_code_volume(
        &self,
        builder_addr: &str,
        window: TimeWindow,
    ) -> Result<Option<BuilderVolumeRow>> {
        let inner = self.read()?;
        let rows: Vec<_> = inner
            .builder_attributions_by_fill
            .values()
            .filter(|row| row.builder_addr == builder_addr)
            .cloned()
            .collect();
        let checkpoint_ts = inner
            .checkpoints
            .values()
            .map(|checkpoint| checkpoint.last_processed_timestamp_us)
            .max()
            .unwrap_or_default();
        drop(inner);

        if rows.is_empty() {
            return Ok(None);
        }

        let max_ts = rows
            .iter()
            .map(|row| row.timestamp_us)
            .max()
            .unwrap_or_default();
        let window_end_ts_us = checkpoint_ts.max(max_ts);
        let window_start_ts_us = max_ts
            .max(window_end_ts_us)
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
            window_end_ts_us,
            notional_volume: notional.to_string(),
            trades,
            active_accounts: accounts.len() as u64,
            estimated_fee_share: has_fee_share.then(|| fee_share.to_string()),
            source: "parsed_decibel_events".to_string(),
            disclaimer: "analytics estimate; not official settlement statement".to_string(),
        }))
    }

    fn scan_market_activity(&self, market_id: &str, limit: usize) -> Result<Vec<ActivityRow>> {
        let mut rows: Vec<_> = self
            .read()?
            .activities
            .iter()
            .filter(|activity| activity.market_id == market_id)
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            b.timestamp_us
                .cmp(&a.timestamp_us)
                .then_with(|| a.version.cmp(&b.version))
                .then_with(|| a.event_idx.cmp(&b.event_idx))
        });
        rows.truncate(limit);
        Ok(rows)
    }

    fn stats(&self) -> Result<StorageStats> {
        let inner = self.read()?;
        Ok(StorageStats {
            tx_count: inner.tx_by_version.len() as u64,
            event_count: inner.events_by_version_idx.len() as u64,
            fill_count: inner.fills_by_id.len() as u64,
            order_count: inner.orders_by_id.len() as u64,
            position_count: inner.positions_by_account_market.len() as u64,
            builder_attribution_count: inner.builder_attributions_by_fill.len() as u64,
            checkpoint_count: inner.checkpoints.len() as u64,
        })
    }

    fn checksums(&self) -> Result<Vec<CfChecksum>> {
        let inner = self.read()?;
        let checksums = vec![
            checksum_cf(
                "cf_tx_by_version",
                inner
                    .tx_by_version
                    .iter()
                    .map(|(version, row)| checksum_pair(key::tx_by_version(*version), row)),
            )?,
            checksum_cf(
                "cf_raw_event_by_version_idx",
                inner
                    .events_by_version_idx
                    .iter()
                    .map(|((version, idx), row)| {
                        checksum_pair(key::raw_event_by_version_idx(*version, *idx), row)
                    }),
            )?,
            checksum_cf(
                "cf_fills_by_market_time",
                inner.fills_by_id.values().map(|row| {
                    checksum_pair(
                        key::fills_by_market_time(
                            &row.market_id,
                            row.timestamp_us,
                            row.version,
                            &row.fill_id,
                        ),
                        row,
                    )
                }),
            )?,
            checksum_cf(
                "cf_fills_by_account_time",
                inner.fills_by_id.values().map(|row| {
                    checksum_pair(
                        key::fills_by_account_time(
                            &row.account,
                            row.timestamp_us,
                            &row.market_id,
                            &row.fill_id,
                        ),
                        row,
                    )
                }),
            )?,
            checksum_cf(
                "cf_order_by_id",
                inner
                    .orders_by_id
                    .iter()
                    .map(|(order_id, row)| checksum_pair(key::order_by_id(order_id), row)),
            )?,
            checksum_cf(
                "cf_positions_by_account_market",
                inner
                    .positions_by_account_market
                    .iter()
                    .map(|((account, market_id), row)| {
                        checksum_pair(key::positions_by_account_market(account, market_id), row)
                    }),
            )?,
            checksum_cf(
                "cf_builder_code_fills",
                inner.builder_attributions_by_fill.values().map(|row| {
                    checksum_pair(
                        key::builder_code_fills(
                            &row.builder_addr,
                            row.timestamp_us,
                            &row.market_id,
                            &row.fill_id,
                        ),
                        row,
                    )
                }),
            )?,
            checksum_cf(
                "cf_market_recent_activity",
                inner.activities.iter().map(|row| {
                    checksum_pair(
                        key::market_activity(
                            &row.market_id,
                            row.timestamp_us,
                            &row.activity_type,
                            row.version,
                            row.event_idx,
                        ),
                        row,
                    )
                }),
            )?,
            checksum_cf(
                "cf_ingest_checkpoint",
                inner.checkpoints.values().map(|row| {
                    checksum_pair(
                        key::ingest_checkpoint(row.network, &row.package_address),
                        row,
                    )
                }),
            )?,
        ];
        Ok(checksums)
    }
}

impl MemoryEngine {
    fn read(&self) -> Result<std::sync::RwLockReadGuard<'_, Inner>> {
        self.inner
            .read()
            .map_err(|_| HotIndexError::Storage("memory engine read lock poisoned".to_string()))
    }

    fn write(&self) -> Result<std::sync::RwLockWriteGuard<'_, Inner>> {
        self.inner
            .write()
            .map_err(|_| HotIndexError::Storage("memory engine write lock poisoned".to_string()))
    }
}

fn sort_fills_recent_first(rows: &mut [FillRow]) {
    rows.sort_by(|a, b| {
        b.timestamp_us
            .cmp(&a.timestamp_us)
            .then_with(|| a.version.cmp(&b.version))
            .then_with(|| a.event_idx.cmp(&b.event_idx))
            .then_with(|| a.fill_id.cmp(&b.fill_id))
    });
}

fn checkpoint_key(checkpoint: &IngestCheckpoint) -> String {
    format!(
        "{}:{}",
        checkpoint.network.as_str(),
        checkpoint.package_address
    )
}

fn checksum_pair<T: Serialize>(key: Vec<u8>, row: &T) -> Result<(Vec<u8>, Vec<u8>)> {
    Ok((key, serde_json::to_vec(row).map_err(json_error)?))
}

fn json_error(error: serde_json::Error) -> HotIndexError {
    HotIndexError::Parse(error.to_string())
}

fn checksum_cf<I>(cf_name: &str, rows: I) -> Result<CfChecksum>
where
    I: IntoIterator<Item = Result<(Vec<u8>, Vec<u8>)>>,
{
    let mut pairs: Vec<_> = rows.into_iter().collect::<Result<Vec<_>>>()?;
    pairs.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hash = Sha256::new();
    for (key, value) in &pairs {
        update_checksum(&mut hash, key, value);
    }

    Ok(CfChecksum {
        cf_name: cf_name.to_string(),
        row_count: pairs.len() as u64,
        hash_hex: hex_lower(&hash.finalize()),
    })
}

fn update_checksum(hash: &mut Sha256, key: &[u8], value: &[u8]) {
    hash.update((key.len() as u64).to_be_bytes());
    hash.update(key);
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value);
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
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
