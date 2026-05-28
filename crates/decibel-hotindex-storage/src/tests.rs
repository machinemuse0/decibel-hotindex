use crate::{MemoryEngine, StorageEngine};
use decibel_hotindex_core::{
    ActivityRow, BuilderAttributionRow, DecibelEventPayload, DecibelEventType, FillRow,
    IngestCheckpoint, Network, NormalizedEvent, OrderRow, PositionRow, TimeWindow, TxRow,
};

#[test]
fn memory_engine_passes_conformance_suite() {
    run_storage_conformance(MemoryEngine::default());
}

#[cfg(feature = "rocksdb")]
#[test]
fn rocksdb_engine_passes_conformance_suite() {
    let path = temp_db_path("conformance");
    let _ = std::fs::remove_dir_all(&path);
    let engine = crate::RocksDbEngine::open(&path).unwrap();
    run_storage_conformance(engine);
    let _ = std::fs::remove_dir_all(&path);
}

#[cfg(feature = "rocksdb")]
#[test]
fn rocksdb_checksums_match_memory_for_same_rows() {
    let path = temp_db_path("checksum");
    let _ = std::fs::remove_dir_all(&path);
    let memory = MemoryEngine::default();
    let rocksdb = crate::RocksDbEngine::open(&path).unwrap();

    populate_checksum_fixture(&memory);
    populate_checksum_fixture(&rocksdb);
    assert_eq!(memory.checksums().unwrap(), rocksdb.checksums().unwrap());

    drop(rocksdb);
    let _ = std::fs::remove_dir_all(&path);
}

fn run_storage_conformance<E: StorageEngine>(engine: E) {
    engine.put_tx(tx_row(10, "tx10")).unwrap();
    engine.put_tx(tx_row(11, "tx11")).unwrap();

    assert_eq!(engine.get_tx(10).unwrap().unwrap().tx_hash, "tx10");
    assert!(engine.get_tx(99).unwrap().is_none());

    let txs = engine.multi_get_txs(&[11, 99, 10]).unwrap();
    assert_eq!(txs[0].as_ref().unwrap().tx_hash, "tx11");
    assert!(txs[1].is_none());
    assert_eq!(txs[2].as_ref().unwrap().tx_hash, "tx10");

    engine.put_event(event_row(10, 0)).unwrap();
    engine
        .put_fill(fill_row("fill-old", 100, "acct-a"))
        .unwrap();
    engine
        .put_fill(fill_row("fill-new", 200, "acct-a"))
        .unwrap();
    engine
        .put_fill(fill_row("fill-other", 300, "acct-b"))
        .unwrap();

    let market_fills = engine.scan_market_fills("BTC-PERP", 10).unwrap();
    assert_eq!(market_fills[0].fill_id, "fill-other");
    assert_eq!(market_fills[1].fill_id, "fill-new");
    assert_eq!(market_fills[2].fill_id, "fill-old");

    let account_fills = engine.scan_account_fills("acct-a", 10).unwrap();
    assert_eq!(account_fills.len(), 2);
    assert_eq!(account_fills[0].fill_id, "fill-new");

    engine.put_order(order_row("order-1", "open")).unwrap();
    engine.put_order(order_row("order-1", "filled")).unwrap();
    assert_eq!(
        engine
            .get_order("order-1")
            .unwrap()
            .unwrap()
            .status
            .as_deref(),
        Some("filled")
    );

    engine
        .put_position(position_row("acct-a", "BTC-PERP", "1"))
        .unwrap();
    engine
        .put_position(position_row("acct-a", "BTC-PERP", "2"))
        .unwrap();
    let positions = engine.get_positions_by_account("acct-a").unwrap();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].size, "2");

    engine
        .put_builder_attribution(builder_row("fill-old", 100, "acct-a", "10.50"))
        .unwrap();
    engine
        .put_builder_attribution(builder_row("fill-new", 200, "acct-a", "3.25"))
        .unwrap();
    engine
        .put_builder_attribution(builder_row("fill-other", 300, "acct-b", "1.25"))
        .unwrap();

    let builder_fills = engine.scan_builder_code_fills("builder-1", 2).unwrap();
    assert_eq!(builder_fills.len(), 2);
    assert_eq!(builder_fills[0].fill_id, "fill-other");
    assert_eq!(builder_fills[1].fill_id, "fill-new");

    let volume = engine
        .get_builder_code_volume("builder-1", TimeWindow::H24)
        .unwrap()
        .unwrap();
    assert_eq!(volume.notional_volume, "15");
    assert_eq!(volume.trades, 3);
    assert_eq!(volume.active_accounts, 2);
    assert_eq!(
        volume.disclaimer,
        "analytics estimate; not official settlement statement"
    );

    engine.put_ingest_checkpoint(checkpoint(11)).unwrap();
    engine.put_ingest_checkpoint(checkpoint(12)).unwrap();
    engine.put_activity(activity_row("order-1", 350)).unwrap();

    let activity = engine.scan_market_activity("BTC-PERP", 2).unwrap();
    assert_eq!(activity.len(), 2);
    assert_eq!(activity[0].summary, "order-1");
    assert_eq!(activity[1].summary, "fill-other");

    let stats = engine.stats().unwrap();
    assert_eq!(stats.tx_count, 2);
    assert_eq!(stats.event_count, 1);
    assert_eq!(stats.fill_count, 3);
    assert_eq!(stats.order_count, 1);
    assert_eq!(stats.position_count, 1);
    assert_eq!(stats.builder_attribution_count, 3);
    assert_eq!(stats.checkpoint_count, 1);

    let checksums = engine.checksums().unwrap();
    assert!(checksums.iter().any(|cf| cf.cf_name == "cf_tx_by_version"));
    assert!(checksums.iter().any(|cf| cf.row_count > 0));
    assert_eq!(checksums, engine.checksums().unwrap());

    let before = checksums;
    engine.put_tx(tx_row(12, "tx12")).unwrap();
    assert_ne!(before, engine.checksums().unwrap());
}

#[cfg(feature = "rocksdb")]
fn populate_checksum_fixture<E: StorageEngine>(engine: &E) {
    engine.put_tx(tx_row(10, "tx10")).unwrap();
    engine.put_event(event_row(10, 0)).unwrap();
    engine.put_fill(fill_row("fill-1", 100, "acct-a")).unwrap();
    engine.put_order(order_row("order-1", "open")).unwrap();
    engine
        .put_position(position_row("acct-a", "BTC-PERP", "1"))
        .unwrap();
    engine
        .put_builder_attribution(builder_row("fill-1", 100, "acct-a", "10.50"))
        .unwrap();
    engine.put_ingest_checkpoint(checkpoint(10)).unwrap();
}

#[cfg(feature = "rocksdb")]
fn temp_db_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "decibel-hotindex-rocksdb-{name}-{}",
        std::process::id()
    ))
}

fn tx_row(version: u64, tx_hash: &str) -> TxRow {
    TxRow {
        network: Network::Mainnet,
        version,
        tx_hash: tx_hash.to_string(),
        block_timestamp_us: version * 10,
        event_count: 1,
        dataset_id: None,
        raw_summary: None,
    }
}

fn event_row(version: u64, event_idx: u32) -> NormalizedEvent {
    NormalizedEvent {
        network: Network::Mainnet,
        version,
        event_idx,
        tx_hash: format!("tx{version}"),
        block_timestamp_us: 100,
        event_type: DecibelEventType::OrderFilled,
        market_id: Some("BTC-PERP".to_string()),
        account: Some("acct-a".to_string()),
        subaccount: None,
        order_id: Some("order-1".to_string()),
        builder_addr: Some("builder-1".to_string()),
        payload: DecibelEventPayload::Empty,
    }
}

fn fill_row(fill_id: &str, timestamp_us: u64, account: &str) -> FillRow {
    FillRow {
        version: timestamp_us,
        event_idx: 0,
        fill_id: fill_id.to_string(),
        market_id: "BTC-PERP".to_string(),
        account: account.to_string(),
        subaccount: None,
        order_id: Some("order-1".to_string()),
        side: Some("buy".to_string()),
        price: "100".to_string(),
        size: "1".to_string(),
        notional: Some("100".to_string()),
        builder_addr: Some("builder-1".to_string()),
        builder_fee_bps: Some(5),
        timestamp_us,
        raw_event_type: "TradeEvent".to_string(),
    }
}

fn order_row(order_id: &str, status: &str) -> OrderRow {
    OrderRow {
        order_id: order_id.to_string(),
        account: "acct-a".to_string(),
        market_id: "BTC-PERP".to_string(),
        side: Some("buy".to_string()),
        price: Some("100".to_string()),
        size: Some("1".to_string()),
        status: Some(status.to_string()),
        version: 10,
        event_idx: 0,
        timestamp_us: 100,
        raw_event_type: "OrderEvent".to_string(),
    }
}

fn position_row(account: &str, market_id: &str, size: &str) -> PositionRow {
    PositionRow {
        account: account.to_string(),
        market_id: market_id.to_string(),
        subaccount: None,
        size: size.to_string(),
        entry_price: Some("100".to_string()),
        source_version: 10,
        source_event_idx: 0,
        timestamp_us: 100,
    }
}

fn builder_row(
    fill_id: &str,
    timestamp_us: u64,
    account: &str,
    notional: &str,
) -> BuilderAttributionRow {
    BuilderAttributionRow {
        builder_addr: "builder-1".to_string(),
        market_id: "BTC-PERP".to_string(),
        account: account.to_string(),
        version: timestamp_us,
        event_idx: 0,
        fill_id: fill_id.to_string(),
        notional: Some(notional.to_string()),
        builder_fee_bps: Some(5),
        estimated_fee_amount: Some("0.01".to_string()),
        timestamp_us,
    }
}

fn activity_row(summary: &str, timestamp_us: u64) -> ActivityRow {
    ActivityRow {
        market_id: "BTC-PERP".to_string(),
        activity_type: "order".to_string(),
        version: timestamp_us,
        event_idx: 0,
        timestamp_us,
        summary: summary.to_string(),
    }
}

fn checkpoint(last_processed_version: u64) -> IngestCheckpoint {
    IngestCheckpoint {
        network: Network::Mainnet,
        package_address: "0x50".to_string(),
        dataset_id: None,
        last_processed_version,
        last_processed_timestamp_us: last_processed_version * 10,
        events_indexed: 1,
        fills_indexed: 1,
        builder_attributions_indexed: 1,
    }
}
