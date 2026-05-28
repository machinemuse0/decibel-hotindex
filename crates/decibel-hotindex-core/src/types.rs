use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Local,
    Devnet,
    Testnet,
    Mainnet,
}

impl Network {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Devnet => "devnet",
            Self::Testnet => "testnet",
            Self::Mainnet => "mainnet",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DatasetId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetEncoding {
    Synthetic,
    Ndjson,
    NdjsonZstd,
    ProtobufZstd,
    AptosTransactionProtobufLenDelimitedZstd,
    BinaryZstd,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetArtifactKind {
    RawTransactions,
    NormalizedEvents,
    Fills,
    Orders,
    Positions,
    BuilderCodeRows,
    QueryCorpus,
    Report,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DatasetFileHashes {
    pub sha256: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetManifest {
    pub dataset_id: DatasetId,
    pub network: Network,
    pub source: String,
    pub transaction_stream_endpoint: Option<String>,
    pub raw_encoding: DatasetEncoding,
    pub normalized_encoding: DatasetEncoding,
    pub start_version: u64,
    pub end_version: Option<u64>,
    pub package_address: String,
    pub orderbook_address: String,
    pub parser_source: Option<String>,
    pub parser_commit: Option<String>,
    pub captured_at: Option<String>,
    pub raw_transaction_count: u64,
    pub decibel_event_count: u64,
    pub fill_count: u64,
    pub order_count: u64,
    pub position_count: u64,
    pub builder_code_row_count: u64,
    #[serde(rename = "sha256")]
    pub hashes: DatasetFileHashes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryKind {
    GetTxByVersion,
    MultiGetTxVersions,
    MarketFillScan,
    AccountFillScan,
    BuilderCodeFillScan,
    BuilderCodeVolume,
    MixedDashboard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryCorpusRecord {
    pub query_kind: QueryKind,
    pub tx_version: Option<u64>,
    pub tx_versions: Vec<u64>,
    pub market_id: Option<String>,
    pub account: Option<String>,
    pub builder_addr: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecibelEventType {
    OrderPlaced,
    OrderCancelled,
    OrderFilled,
    PositionUpdated,
    Liquidation,
    FundingPayment,
    BuilderFeeAttributed,
    MarketCreated,
    Unknown(String),
}

impl DecibelEventType {
    pub fn as_key(&self) -> String {
        match self {
            Self::OrderPlaced => "order_placed".to_string(),
            Self::OrderCancelled => "order_cancelled".to_string(),
            Self::OrderFilled => "order_filled".to_string(),
            Self::PositionUpdated => "position_updated".to_string(),
            Self::Liquidation => "liquidation".to_string(),
            Self::FundingPayment => "funding_payment".to_string(),
            Self::BuilderFeeAttributed => "builder_fee_attributed".to_string(),
            Self::MarketCreated => "market_created".to_string(),
            Self::Unknown(raw) => format!("unknown:{raw}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecibelEventPayload {
    RawJson { value: String },
    Unknown { raw_type: String, value: String },
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedEvent {
    pub network: Network,
    pub version: u64,
    pub event_idx: u32,
    pub tx_hash: String,
    pub block_timestamp_us: u64,
    pub event_type: DecibelEventType,
    pub market_id: Option<String>,
    pub account: Option<String>,
    pub subaccount: Option<String>,
    pub order_id: Option<String>,
    pub builder_addr: Option<String>,
    pub payload: DecibelEventPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxRow {
    pub network: Network,
    pub version: u64,
    pub tx_hash: String,
    pub block_timestamp_us: u64,
    pub event_count: u32,
    pub dataset_id: Option<DatasetId>,
    pub raw_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillRow {
    pub version: u64,
    pub event_idx: u32,
    pub fill_id: String,
    pub market_id: String,
    pub account: String,
    pub subaccount: Option<String>,
    pub order_id: Option<String>,
    pub side: Option<String>,
    pub price: String,
    pub size: String,
    pub notional: Option<String>,
    pub builder_addr: Option<String>,
    pub builder_fee_bps: Option<u16>,
    pub timestamp_us: u64,
    pub raw_event_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderRow {
    pub order_id: String,
    pub account: String,
    pub market_id: String,
    pub side: Option<String>,
    pub price: Option<String>,
    pub size: Option<String>,
    pub status: Option<String>,
    pub version: u64,
    pub event_idx: u32,
    pub timestamp_us: u64,
    pub raw_event_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionRow {
    pub account: String,
    pub market_id: String,
    pub subaccount: Option<String>,
    pub size: String,
    pub entry_price: Option<String>,
    pub source_version: u64,
    pub source_event_idx: u32,
    pub timestamp_us: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuilderAttributionRow {
    pub builder_addr: String,
    pub market_id: String,
    pub account: String,
    pub version: u64,
    pub event_idx: u32,
    pub fill_id: String,
    pub notional: Option<String>,
    pub builder_fee_bps: Option<u16>,
    pub estimated_fee_amount: Option<String>,
    pub timestamp_us: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuilderVolumeRow {
    pub builder_addr: String,
    pub window: TimeWindow,
    pub window_start_ts_us: u64,
    pub notional_volume: String,
    pub trades: u64,
    pub active_accounts: u64,
    pub estimated_fee_share: Option<String>,
    pub source: String,
    pub disclaimer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityRow {
    pub market_id: String,
    pub activity_type: String,
    pub version: u64,
    pub event_idx: u32,
    pub timestamp_us: u64,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestCheckpoint {
    pub network: Network,
    pub package_address: String,
    pub dataset_id: Option<DatasetId>,
    pub last_processed_version: u64,
    pub last_processed_timestamp_us: u64,
    pub events_indexed: u64,
    pub fills_indexed: u64,
    pub builder_attributions_indexed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StorageStats {
    pub tx_count: u64,
    pub event_count: u64,
    pub fill_count: u64,
    pub order_count: u64,
    pub position_count: u64,
    pub builder_attribution_count: u64,
    pub checkpoint_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CfChecksum {
    pub cf_name: String,
    pub row_count: u64,
    pub hash_hex: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeWindow {
    #[serde(rename = "24h")]
    H24,
    #[serde(rename = "7d")]
    D7,
}

impl TimeWindow {
    pub fn label(self) -> &'static str {
        match self {
            Self::H24 => "24h",
            Self::D7 => "7d",
        }
    }

    pub fn duration_us(self) -> u64 {
        match self {
            Self::H24 => 24 * 60 * 60 * 1_000_000,
            Self::D7 => 7 * 24 * 60 * 60 * 1_000_000,
        }
    }
}
