use decibel_hotindex_core::{
    normalize_aptos_address, ActivityRow, BuilderAttributionRow, DatasetId, DecibelEventPayload,
    DecibelEventType, FillRow, HotIndexError, Network, NormalizedEvent, OrderRow, PositionRow,
    Result, TxRow,
};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone)]
pub struct ParserOptions {
    pub network: Network,
    pub dataset_id: DatasetId,
    pub package_address: String,
    pub orderbook_address: String,
    pub parser_source: String,
    pub parser_commit: Option<String>,
}

#[derive(Debug, Default)]
pub struct ParserOutput {
    pub txs: Vec<TxRow>,
    pub events: Vec<NormalizedEvent>,
    pub fills: Vec<FillRow>,
    pub orders: Vec<OrderRow>,
    pub positions: Vec<PositionRow>,
    pub builder_rows: Vec<BuilderAttributionRow>,
    pub activity_rows: Vec<ActivityRow>,
    pub unknown_events: Vec<NormalizedEvent>,
    pub warnings: Vec<String>,
    pub raw_transaction_count: u64,
}

#[derive(Debug, Clone)]
struct TxMeta {
    version: u64,
    tx_hash: String,
    block_timestamp_us: u64,
}

pub fn parse_fixture_jsonl_file(path: &Path, options: &ParserOptions) -> Result<ParserOutput> {
    let file = File::open(path)?;
    parse_fixture_jsonl_reader(BufReader::new(file), options)
}

pub fn parse_fixture_jsonl_str(input: &str, options: &ParserOptions) -> Result<ParserOutput> {
    parse_fixture_jsonl_reader(BufReader::new(input.as_bytes()), options)
}

fn parse_fixture_jsonl_reader<R: BufRead>(
    reader: R,
    options: &ParserOptions,
) -> Result<ParserOutput> {
    let mut output = ParserOutput::default();

    for (line_idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line).map_err(|error| {
            HotIndexError::Parse(format!("fixture jsonl line {}: {error}", line_idx + 1))
        })?;
        parse_fixture_line(&value, line_idx + 1, options, &mut output)?;
    }

    Ok(output)
}

fn parse_fixture_line(
    value: &Value,
    line_no: usize,
    options: &ParserOptions,
    output: &mut ParserOutput,
) -> Result<()> {
    let meta = tx_meta(value, line_no)?;
    let before = output.events.len();

    if let Some(events) = value.get("events").and_then(Value::as_array) {
        for (idx, event) in events.iter().enumerate() {
            parse_event(event, &meta, idx as u32, line_no, options, output)?;
        }
    } else {
        parse_event(value, &meta, 0, line_no, options, output)?;
    }

    let included = output.events.len() - before;
    if included > 0 {
        output.raw_transaction_count += 1;
        output.txs.push(TxRow {
            network: options.network,
            version: meta.version,
            tx_hash: meta.tx_hash,
            block_timestamp_us: meta.block_timestamp_us,
            event_count: included as u32,
            dataset_id: Some(options.dataset_id.clone()),
            raw_summary: Some(format!(
                "fixture transaction with {included} Decibel event(s)"
            )),
        });
    }
    Ok(())
}

fn parse_event(
    event: &Value,
    meta: &TxMeta,
    fallback_event_idx: u32,
    line_no: usize,
    options: &ParserOptions,
    output: &mut ParserOutput,
) -> Result<()> {
    let raw_type =
        string_field(event, &["type", "event_type", "raw_event_type"]).ok_or_else(|| {
            HotIndexError::Parse(format!("fixture jsonl line {line_no}: missing event type"))
        })?;
    let event_idx = u32_field(event, &["event_idx", "event_index"]).unwrap_or(fallback_event_idx);
    let data = event
        .get("data")
        .or_else(|| event.get("payload"))
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));

    if !matches_decibel_address(event, &raw_type, options) {
        return Ok(());
    }

    let event_type = classify_event_type(&raw_type, &data);
    let market_id = string_field(&data, &["market_id", "market", "symbol"]);
    let account = string_field(&data, &["account", "trader", "user", "owner"]);
    let order_id = string_field(&data, &["order_id", "orderId", "client_order_id"]);
    let builder_addr = string_field(
        &data,
        &["builder_addr", "builder_address", "builder_code", "builder"],
    );

    let normalized = NormalizedEvent {
        network: options.network,
        version: meta.version,
        event_idx,
        tx_hash: meta.tx_hash.clone(),
        block_timestamp_us: meta.block_timestamp_us,
        event_type: event_type.clone(),
        market_id: market_id.clone(),
        account: account.clone(),
        subaccount: string_field(&data, &["subaccount", "sub_account"]),
        order_id: order_id.clone(),
        builder_addr: builder_addr.clone(),
        payload: DecibelEventPayload::RawJson {
            value: serde_json::to_string(&data).map_err(json_error)?,
        },
    };

    if matches!(event_type, DecibelEventType::Unknown(_)) {
        output.warnings.push(format!(
            "line {line_no} version {} event {} unknown event type {raw_type}",
            meta.version, event_idx
        ));
        output.unknown_events.push(normalized.clone());
    }
    output.events.push(normalized);

    match event_type {
        DecibelEventType::OrderFilled => {
            maybe_push_fill(&raw_type, &data, meta, event_idx, output);
            maybe_push_builder_row(&data, meta, event_idx, output);
        }
        DecibelEventType::OrderPlaced | DecibelEventType::OrderCancelled => {
            maybe_push_order(&raw_type, &data, &event_type, meta, event_idx, output);
        }
        DecibelEventType::PositionUpdated => {
            maybe_push_position(&data, meta, event_idx, output);
        }
        DecibelEventType::Liquidation
        | DecibelEventType::FundingPayment
        | DecibelEventType::MarketCreated
        | DecibelEventType::BuilderFeeAttributed => {
            maybe_push_activity(&data, &event_type, meta, event_idx, output);
        }
        DecibelEventType::Unknown(_) => {}
    }

    Ok(())
}

fn tx_meta(value: &Value, line_no: usize) -> Result<TxMeta> {
    let version = u64_field(value, &["version", "transaction_version"]).ok_or_else(|| {
        HotIndexError::Parse(format!(
            "fixture jsonl line {line_no}: missing transaction version"
        ))
    })?;
    Ok(TxMeta {
        version,
        tx_hash: string_field(value, &["tx_hash", "transaction_hash"])
            .unwrap_or_else(|| format!("0x{version:064x}")),
        block_timestamp_us: u64_field(value, &["block_timestamp_us", "timestamp_us"])
            .unwrap_or_default(),
    })
}

fn maybe_push_fill(
    raw_type: &str,
    data: &Value,
    meta: &TxMeta,
    event_idx: u32,
    output: &mut ParserOutput,
) {
    let Some(market_id) = string_field(data, &["market_id", "market", "symbol"]) else {
        output.warnings.push(format!(
            "version {} event {} TradeEvent missing market_id",
            meta.version, event_idx
        ));
        return;
    };
    let Some(account) = string_field(data, &["account", "trader", "user", "owner"]) else {
        output.warnings.push(format!(
            "version {} event {} TradeEvent missing account",
            meta.version, event_idx
        ));
        return;
    };

    output.fills.push(FillRow {
        version: meta.version,
        event_idx,
        fill_id: string_field(data, &["fill_id", "fillId", "trade_id"])
            .unwrap_or_else(|| format!("fill-{}-{event_idx}", meta.version)),
        market_id,
        account,
        subaccount: string_field(data, &["subaccount", "sub_account"]),
        order_id: string_field(data, &["order_id", "orderId", "client_order_id"]),
        side: string_field(data, &["side"]),
        price: string_field(data, &["price", "limit_price"]).unwrap_or_else(|| "0".to_string()),
        size: string_field(data, &["size", "quantity", "base_size"])
            .unwrap_or_else(|| "0".to_string()),
        notional: string_field(data, &["notional", "quote_size", "quote_amount"]),
        builder_addr: string_field(
            data,
            &["builder_addr", "builder_address", "builder_code", "builder"],
        ),
        builder_fee_bps: u16_field(data, &["builder_fee_bps", "fee_bps"]),
        timestamp_us: meta.block_timestamp_us,
        raw_event_type: event_type_tail(raw_type).to_string(),
    });
}

fn maybe_push_builder_row(data: &Value, meta: &TxMeta, event_idx: u32, output: &mut ParserOutput) {
    let Some(builder_addr) = string_field(
        data,
        &["builder_addr", "builder_address", "builder_code", "builder"],
    ) else {
        return;
    };
    let Some(market_id) = string_field(data, &["market_id", "market", "symbol"]) else {
        return;
    };
    let Some(account) = string_field(data, &["account", "trader", "user", "owner"]) else {
        return;
    };

    output.builder_rows.push(BuilderAttributionRow {
        builder_addr,
        market_id,
        account,
        version: meta.version,
        event_idx,
        fill_id: string_field(data, &["fill_id", "fillId", "trade_id"])
            .unwrap_or_else(|| format!("fill-{}-{event_idx}", meta.version)),
        notional: string_field(data, &["notional", "quote_size", "quote_amount"]),
        builder_fee_bps: u16_field(data, &["builder_fee_bps", "fee_bps"]),
        estimated_fee_amount: string_field(data, &["estimated_fee_amount", "builder_fee_amount"]),
        timestamp_us: meta.block_timestamp_us,
    });
}

fn maybe_push_order(
    raw_type: &str,
    data: &Value,
    event_type: &DecibelEventType,
    meta: &TxMeta,
    event_idx: u32,
    output: &mut ParserOutput,
) {
    let Some(market_id) = string_field(data, &["market_id", "market", "symbol"]) else {
        return;
    };
    let Some(account) = string_field(data, &["account", "trader", "user", "owner"]) else {
        return;
    };
    let order_id = string_field(data, &["order_id", "orderId", "client_order_id"])
        .unwrap_or_else(|| format!("order-{}-{event_idx}", meta.version));
    let status = string_field(data, &["status"]).or_else(|| match event_type {
        DecibelEventType::OrderCancelled => Some("cancelled".to_string()),
        _ => Some("open".to_string()),
    });

    output.orders.push(OrderRow {
        order_id: order_id.clone(),
        account,
        market_id: market_id.clone(),
        side: string_field(data, &["side"]),
        price: string_field(data, &["price", "limit_price"]),
        size: string_field(data, &["size", "quantity", "base_size"]),
        status,
        version: meta.version,
        event_idx,
        timestamp_us: meta.block_timestamp_us,
        raw_event_type: event_type_tail(raw_type).to_string(),
    });

    output.activity_rows.push(ActivityRow {
        market_id,
        activity_type: "order".to_string(),
        version: meta.version,
        event_idx,
        timestamp_us: meta.block_timestamp_us,
        summary: order_id,
    });
}

fn maybe_push_position(data: &Value, meta: &TxMeta, event_idx: u32, output: &mut ParserOutput) {
    let Some(market_id) = string_field(data, &["market_id", "market", "symbol"]) else {
        return;
    };
    let Some(account) = string_field(data, &["account", "trader", "user", "owner"]) else {
        return;
    };

    output.positions.push(PositionRow {
        account: account.clone(),
        market_id: market_id.clone(),
        subaccount: string_field(data, &["subaccount", "sub_account"]),
        size: string_field(data, &["size", "position_size"]).unwrap_or_else(|| "0".to_string()),
        entry_price: string_field(data, &["entry_price", "avg_entry_price"]),
        source_version: meta.version,
        source_event_idx: event_idx,
        timestamp_us: meta.block_timestamp_us,
    });

    output.activity_rows.push(ActivityRow {
        market_id,
        activity_type: "position".to_string(),
        version: meta.version,
        event_idx,
        timestamp_us: meta.block_timestamp_us,
        summary: account,
    });
}

fn maybe_push_activity(
    data: &Value,
    event_type: &DecibelEventType,
    meta: &TxMeta,
    event_idx: u32,
    output: &mut ParserOutput,
) {
    let Some(market_id) = string_field(data, &["market_id", "market", "symbol"]) else {
        return;
    };
    output.activity_rows.push(ActivityRow {
        market_id,
        activity_type: event_type_label(event_type).to_string(),
        version: meta.version,
        event_idx,
        timestamp_us: meta.block_timestamp_us,
        summary: string_field(data, &["account", "trader", "user", "owner", "id"])
            .unwrap_or_else(|| event_type_label(event_type).to_string()),
    });
}

fn classify_event_type(raw_type: &str, data: &Value) -> DecibelEventType {
    let tail = event_type_tail(raw_type);
    let lowered = tail.to_ascii_lowercase();
    if lowered.contains("trade") || lowered.contains("fill") {
        DecibelEventType::OrderFilled
    } else if lowered.contains("liquidation") || lowered.contains("margincall") {
        DecibelEventType::Liquidation
    } else if lowered.contains("position") {
        DecibelEventType::PositionUpdated
    } else if lowered.contains("funding") {
        DecibelEventType::FundingPayment
    } else if lowered.contains("market") {
        DecibelEventType::MarketCreated
    } else if lowered.contains("builder") && lowered.contains("fee") {
        DecibelEventType::BuilderFeeAttributed
    } else if lowered.contains("order") {
        let status = string_field(data, &["status"])
            .unwrap_or_default()
            .to_ascii_lowercase();
        if lowered.contains("cancel") || status.contains("cancel") {
            DecibelEventType::OrderCancelled
        } else {
            DecibelEventType::OrderPlaced
        }
    } else {
        DecibelEventType::Unknown(tail.to_string())
    }
}

fn matches_decibel_address(event: &Value, raw_type: &str, options: &ParserOptions) -> bool {
    if is_zero_address(&options.package_address) && is_zero_address(&options.orderbook_address) {
        return true;
    }

    let mut candidates = Vec::new();
    if let Some(package) = raw_type.split("::").next() {
        candidates.push(package.to_string());
    }
    for key in [
        "package_address",
        "orderbook_address",
        "account_address",
        "emitting_contract",
    ] {
        if let Some(value) = string_field(event, &[key]) {
            candidates.push(value);
        }
    }

    candidates
        .iter()
        .filter_map(|candidate| normalize_aptos_address(candidate).ok())
        .any(|candidate| {
            candidate == options.package_address || candidate == options.orderbook_address
        })
}

fn event_type_tail(raw_type: &str) -> &str {
    raw_type.rsplit("::").next().unwrap_or(raw_type)
}

fn event_type_label(event_type: &DecibelEventType) -> &'static str {
    match event_type {
        DecibelEventType::OrderPlaced => "order_placed",
        DecibelEventType::OrderCancelled => "order_cancelled",
        DecibelEventType::OrderFilled => "order_filled",
        DecibelEventType::PositionUpdated => "position_updated",
        DecibelEventType::Liquidation => "liquidation",
        DecibelEventType::FundingPayment => "funding_payment",
        DecibelEventType::BuilderFeeAttributed => "builder_fee_attributed",
        DecibelEventType::MarketCreated => "market_created",
        DecibelEventType::Unknown(_) => "unknown",
    }
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    let field = keys.iter().find_map(|key| value.get(*key))?;
    match field {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn u64_field(value: &Value, keys: &[&str]) -> Option<u64> {
    let field = keys.iter().find_map(|key| value.get(*key))?;
    match field {
        Value::Number(value) => value.as_u64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn u32_field(value: &Value, keys: &[&str]) -> Option<u32> {
    u64_field(value, keys).and_then(|value| value.try_into().ok())
}

fn u16_field(value: &Value, keys: &[&str]) -> Option<u16> {
    u64_field(value, keys).and_then(|value| value.try_into().ok())
}

fn is_zero_address(value: &str) -> bool {
    normalize_aptos_address(value)
        .map(|normalized| normalized == ZERO_ADDRESS)
        .unwrap_or(false)
}

fn json_error(error: serde_json::Error) -> HotIndexError {
    HotIndexError::Parse(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{parse_fixture_jsonl_str, ParserOptions};
    use decibel_hotindex_core::{DatasetId, DecibelEventType, Network};

    const PACKAGE: &str = "0x50ead22afd6ffd9769e3b3d6e0e64a2a350d68e8b102c4e72e33d0b8cfdfdb06";

    #[test]
    fn parses_trade_order_position_and_unknown_events() {
        let input = serde_json::json!({
            "version": 4365621793_u64,
            "tx_hash": "0xabc",
            "block_timestamp_us": 1770000000000000_u64,
            "events": [
                {"event_idx":0,"type":format!("{PACKAGE}::orderbook::TradeEvent"),"data":{"market_id":"BTC-PERP","account":"0x1","order_id":"o1","fill_id":"f1","side":"buy","price":"100","size":"1","notional":"100","builder_addr":"0x2","builder_fee_bps":5,"estimated_fee_amount":"0.05"}},
                {"event_idx":1,"type":format!("{PACKAGE}::orderbook::OrderEvent"),"data":{"market_id":"BTC-PERP","account":"0x1","order_id":"o2","status":"open"}},
                {"event_idx":2,"type":format!("{PACKAGE}::orderbook::PositionUpdateEvent"),"data":{"market_id":"BTC-PERP","account":"0x1","size":"2","entry_price":"100"}},
                {"event_idx":3,"type":format!("{PACKAGE}::orderbook::MysteryEvent"),"data":{"market_id":"BTC-PERP","account":"0x1","raw":"kept"}},
                {"event_idx":4,"type":"0x3::other::TradeEvent","data":{"market_id":"BTC-PERP","account":"0x1"}}
            ]
        })
        .to_string();
        let output = parse_fixture_jsonl_str(&input, &options()).unwrap();

        assert_eq!(output.txs.len(), 1);
        assert_eq!(output.events.len(), 4);
        assert_eq!(output.fills.len(), 1);
        assert_eq!(output.orders.len(), 1);
        assert_eq!(output.positions.len(), 1);
        assert_eq!(output.builder_rows.len(), 1);
        assert_eq!(output.unknown_events.len(), 1);
        assert!(matches!(
            output.unknown_events[0].event_type,
            DecibelEventType::Unknown(_)
        ));
        assert!(output.unknown_events[0]
            .payload
            .clone()
            .as_raw_json()
            .contains("kept"));
    }

    fn options() -> ParserOptions {
        ParserOptions {
            network: Network::Mainnet,
            dataset_id: DatasetId("fixture_test".to_string()),
            package_address: PACKAGE.to_string(),
            orderbook_address: PACKAGE.to_string(),
            parser_source: "test".to_string(),
            parser_commit: Some("abc123".to_string()),
        }
    }

    trait PayloadExt {
        fn as_raw_json(self) -> String;
    }

    impl PayloadExt for decibel_hotindex_core::DecibelEventPayload {
        fn as_raw_json(self) -> String {
            match self {
                decibel_hotindex_core::DecibelEventPayload::RawJson { value } => value,
                _ => String::new(),
            }
        }
    }
}
