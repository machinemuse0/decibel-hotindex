use aptos_protos::{
    indexer::v1::{raw_data_client::RawDataClient, GetTransactionsRequest},
    transaction::v1::Transaction,
};
use decibel_hotindex_core::{
    normalize_aptos_address, ActivityRow, AppConfig, BuilderAttributionRow, DatasetEncoding,
    DatasetFileHashes, DatasetId, DatasetManifest, DecibelEventPayload, DecibelEventType, FillRow,
    HotIndexError, IngestCheckpoint, Network, NormalizedEvent, OrderRow, PositionRow,
    QueryCorpusRecord, QueryKind, Result, TxRow,
};
use decibel_hotindex_ingest::{parse_fixture_jsonl_file, ParserOptions, ParserOutput};
#[cfg(feature = "rocksdb")]
use decibel_hotindex_storage::RocksDbEngine;
#[cfg(feature = "toplingsdb")]
use decibel_hotindex_storage::ToplingDbEngine;
use decibel_hotindex_storage::{MemoryEngine, StorageEngine};
use prost::Message;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tonic::transport::{Channel, ClientTlsConfig};

const NORMALIZED_ARTIFACTS: &[&str] = &[
    "normalized/txs.ndjson",
    "normalized/events.ndjson",
    "normalized/fills.ndjson",
    "normalized/orders.ndjson",
    "normalized/positions.ndjson",
    "normalized/builder_code_rows.ndjson",
    "normalized/activity_rows.ndjson",
    "normalized/unknown_events.ndjson",
    "normalized/parse_warnings.log",
];

const QUERY_CORPUS_ARTIFACTS: &[&str] = &[
    "queries/mixed_dashboard.ndjson",
    "queries/point_tx_versions.ndjson",
    "queries/multi_get_tx_versions.ndjson",
    "queries/market_fill_scans.ndjson",
    "queries/account_fill_scans.ndjson",
    "queries/builder_code_scans.ndjson",
    "queries/builder_code_volumes.ndjson",
];

const MAINNET_DECIBEL_ADDRESS: &str =
    "0x50ead22afd6ffd9769e3b3d6e0e64a2a350d68e8b102c4e72e33d0b8cfdfdb06";
const TESTNET_DECIBEL_ADDRESS: &str =
    "0xe7da2794b1d8af76532ed95f38bfdf1136abfd8ea3a240189971988a83101b7f";

fn main() {
    if let Err(error) = run(env::args().skip(1).collect()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        print_usage();
        return Ok(());
    };

    match command {
        "synthetic" => synthetic_command(&args[1..]),
        "fixture" => fixture_command(&args[1..]),
        "build-query-corpus" => build_query_corpus_command(&args[1..]),
        "replay" => replay_command(&args[1..]),
        "normalize" => normalize_command(&args[1..]),
        "record" => record_command(&args[1..]),
        "inspect-raw" => inspect_raw_command(&args[1..]),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        other => Err(HotIndexError::Config(format!("unknown command: {other}"))),
    }
}

fn inspect_raw_command(args: &[String]) -> Result<()> {
    let opts = Args::new(args);
    let input = opts.required_path("--input")?;
    let limit = opts.optional_u64("--limit")?.unwrap_or(u64::MAX);
    let mut decoder = zstd::stream::read::Decoder::new(File::open(&input)?)?;
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes)?;
    let mut slice = bytes.as_slice();
    let mut count = 0_u64;
    let mut first_version = None;
    let mut last_version = None;
    while !slice.is_empty() && count < limit {
        let tx = Transaction::decode_length_delimited(&mut slice)
            .map_err(|error| HotIndexError::Parse(error.to_string()))?;
        first_version.get_or_insert(tx.version);
        last_version = Some(tx.version);
        count += 1;
    }
    let report = serde_json::json!({
        "input": input.display().to_string(),
        "decoded_transactions": count,
        "first_version": first_version,
        "last_version": last_version,
        "remaining_bytes_after_limit": slice.len()
    });
    serde_json::to_writer_pretty(std::io::stdout(), &report).map_err(json_error)?;
    println!();
    Ok(())
}

fn synthetic_command(args: &[String]) -> Result<()> {
    let opts = Args::new(args);
    let out = opts.required_path("--out")?;
    let count = opts
        .optional_u64("--events")?
        .unwrap_or(128)
        .try_into()
        .map_err(|_| HotIndexError::Config("--events is too large".to_string()))?;

    let dataset = SyntheticDataset::generate(count);
    write_synthetic_dataset(&out, &dataset)?;
    println!(
        "synthetic dataset written: {} events at {}",
        count,
        out.display()
    );
    Ok(())
}

fn fixture_command(args: &[String]) -> Result<()> {
    let opts = Args::new(args);
    let out = opts.required_path("--out")?;
    let config = load_optional_config(&opts)?;
    let count = opts
        .optional_u64("--events")?
        .unwrap_or(12)
        .try_into()
        .map_err(|_| HotIndexError::Config("--events is too large".to_string()))?;
    let network = resolve_network(&opts, config.as_ref())?;
    let package_address = resolve_package_address(&opts, config.as_ref(), network)?;
    let orderbook_address = resolve_orderbook_address(&opts, config.as_ref(), network)?;
    let raw_dir = out.join("raw");
    fs::create_dir_all(&raw_dir)?;

    let raw_path = raw_dir.join("fixture_events.jsonl");
    let rows = build_fixture_raw_events(count, network, &package_address, &orderbook_address);
    write_jsonl_values(&raw_path, &rows)?;

    let checkpoint = serde_json::json!({
        "status": "fixture",
        "message": "fixture JSONL written locally; no Aptos gRPC call was made",
        "network": network.as_str(),
        "package_address": package_address,
        "orderbook_address": orderbook_address,
        "raw_format": "fixture-jsonl",
        "event_records": count,
        "raw_path": "raw/fixture_events.jsonl"
    });
    write_json_pretty(&raw_dir.join("record_checkpoint.json"), &checkpoint)?;

    println!(
        "fixture raw dataset written: {} event records at {}",
        count,
        raw_path.display()
    );
    Ok(())
}

fn build_query_corpus_command(args: &[String]) -> Result<()> {
    let opts = Args::new(args);
    let events = opts.required_path("--events")?;
    let out_dir = opts.required_path("--out-dir")?;
    let _seed = opts.optional_u64("--seed")?.unwrap_or(42);

    fs::create_dir_all(&out_dir)?;
    let records = read_ndjson::<NormalizedEvent>(&events)?;
    if records.is_empty() {
        return Err(HotIndexError::Config(format!(
            "no normalized events found in {}",
            events.display()
        )));
    }

    let corpus = build_query_corpus(&records);
    write_query_corpus_files(&out_dir, &corpus)?;
    update_manifest_query_hashes_if_present(&out_dir)?;

    println!(
        "query corpus written: {} records at {}",
        corpus.len(),
        out_dir.display()
    );
    Ok(())
}

fn replay_command(args: &[String]) -> Result<()> {
    let opts = Args::new(args);
    let dataset = opts.required_path("--dataset")?;
    let engine = opts.optional_value("--engine").unwrap_or("memory");
    match engine {
        "memory" => {
            let engine = replay_into_memory(&dataset)?;
            print_replay_result(&engine)?;
        }
        #[cfg(feature = "rocksdb")]
        "rocksdb" => {
            let db_path = opts
                .optional_value("--db-path")
                .map(PathBuf::from)
                .unwrap_or_else(|| dataset.join("materialized/rocksdb"));
            let engine = RocksDbEngine::open(&db_path)?;
            replay_into_engine(&dataset, &engine)?;
            print_replay_result(&engine)?;
            println!("rocksdb path={}", db_path.display());
        }
        #[cfg(not(feature = "rocksdb"))]
        "rocksdb" => {
            return Err(HotIndexError::Config(
                "RocksDB replay requires `--features rocksdb`".to_string(),
            ));
        }
        #[cfg(feature = "toplingsdb")]
        "toplingdb" => {
            let db_path = opts
                .optional_value("--db-path")
                .map(PathBuf::from)
                .unwrap_or_else(|| dataset.join("materialized/toplingdb"));
            let engine = ToplingDbEngine::open(&db_path)?;
            replay_into_engine(&dataset, &engine)?;
            print_replay_result(&engine)?;
            println!("toplingdb path={}", db_path.display());
        }
        #[cfg(not(feature = "toplingsdb"))]
        "toplingdb" => {
            return Err(HotIndexError::Config(
                "ToplingDB replay requires `--features toplingsdb`".to_string(),
            ));
        }
        other => {
            return Err(HotIndexError::Config(format!(
                "unsupported replay engine: {other}"
            )));
        }
    }
    Ok(())
}

fn normalize_command(args: &[String]) -> Result<()> {
    let opts = Args::new(args);
    let input = opts.required_path("--input")?;
    let out_dir = opts.required_path("--out-dir")?;
    fs::metadata(&input).map_err(|error| {
        HotIndexError::Config(format!(
            "raw input {} is not readable: {error}",
            input.display()
        ))
    })?;

    let raw_format = opts
        .optional_value("--format")
        .or_else(|| opts.optional_value("--raw-format"))
        .map(str::to_string)
        .unwrap_or_else(|| infer_raw_format(&input).to_string());
    if raw_format != "fixture-jsonl" {
        fs::create_dir_all(&out_dir)?;
        let warning = format!(
            "normalize skeleton only: raw Aptos protobuf decoding is pending; input={}\n",
            input.display()
        );
        fs::write(out_dir.join("parse_warnings.log"), warning)?;
        println!(
            "normalize skeleton wrote parse_warnings.log at {}",
            out_dir.display()
        );
        return Ok(());
    }

    let dataset_root = dataset_root_for_normalized_dir(&out_dir)?;
    let parser_options = parser_options_from_args(&opts, &dataset_root)?;
    let rows = parse_fixture_jsonl_file(&input, &parser_options)?;
    write_normalized_fixture_dataset(&dataset_root, &out_dir, &input, &parser_options, rows)?;
    println!(
        "normalized fixture dataset written at {}",
        out_dir.display()
    );
    Ok(())
}

fn record_command(args: &[String]) -> Result<()> {
    let opts = Args::new(args);
    let config = load_optional_config(&opts)?;
    let network = resolve_network(&opts, config.as_ref())?;
    let endpoint = opts.optional_value("--endpoint").unwrap_or("");
    let start_version = opts.optional_u64("--start-version")?.unwrap_or(0);
    let end_version = opts.optional_u64("--end-version")?;
    let out_dir = opts.required_path("--out-dir")?;
    let raw_format = opts
        .optional_value("--raw-format")
        .unwrap_or("protobuf-zstd");
    let package_address = resolve_package_address(&opts, config.as_ref(), network)?;
    let orderbook_address = resolve_orderbook_address(&opts, config.as_ref(), network)?;
    let auth_token_present = auth_token_present(&opts);
    fs::create_dir_all(&out_dir)?;

    if opts.has_flag("--live") {
        let token = auth_token(&opts)?;
        let request = LiveRecordRequest {
            network,
            endpoint: endpoint.to_string(),
            start_version,
            end_version: end_version.ok_or_else(|| {
                HotIndexError::Config("--live record requires --end-version".to_string())
            })?,
            batch_size: opts.optional_u64("--batch-size")?.unwrap_or(100).min(1000),
            max_decoding_message_size: opts
                .optional_u64("--max-message-mb")?
                .unwrap_or(128)
                .saturating_mul(1024 * 1024) as usize,
            out_dir: out_dir.clone(),
            raw_format: raw_format.to_string(),
            package_address,
            orderbook_address,
            auth_token: token,
        };
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|error| HotIndexError::Config(error.to_string()))?;
        return runtime.block_on(record_live_transaction_stream(request));
    }

    let checkpoint = serde_json::json!({
        "status": "planned",
        "message": "record skeleton only; Aptos gRPC recording is implemented in a later milestone",
        "network": network.as_str(),
        "endpoint": endpoint,
        "package_address": package_address,
        "orderbook_address": orderbook_address,
        "start_version": start_version,
        "end_version": end_version,
        "raw_format": raw_format,
        "auth_token_present": auth_token_present,
        "last_success_version": null
    });
    write_json_pretty(&out_dir.join("record_checkpoint.json"), &checkpoint)?;
    println!("record skeleton wrote checkpoint at {}", out_dir.display());
    Ok(())
}

struct LiveRecordRequest {
    network: Network,
    endpoint: String,
    start_version: u64,
    end_version: u64,
    batch_size: u64,
    max_decoding_message_size: usize,
    out_dir: PathBuf,
    raw_format: String,
    package_address: String,
    orderbook_address: String,
    auth_token: String,
}

async fn record_live_transaction_stream(request: LiveRecordRequest) -> Result<()> {
    if request.end_version < request.start_version {
        return Err(HotIndexError::Config(format!(
            "end-version {} is before start-version {}",
            request.end_version, request.start_version
        )));
    }
    let expected_count = request.end_version - request.start_version + 1;
    let endpoint_url = normalize_grpc_endpoint(&request.endpoint)?;
    let mut endpoint = Channel::from_shared(endpoint_url.clone())
        .map_err(|error| HotIndexError::Config(error.to_string()))?
        .http2_keep_alive_interval(Duration::from_secs(30))
        .keep_alive_timeout(Duration::from_secs(10));
    if endpoint_url.starts_with("https://") {
        endpoint = endpoint
            .tls_config(ClientTlsConfig::new().with_enabled_roots())
            .map_err(|error| HotIndexError::Config(error.to_string()))?;
    }
    let channel = endpoint.connect().await.map_err(|error| {
        HotIndexError::Config(format!(
            "failed to connect Aptos Transaction Stream endpoint {endpoint_url}: {error:?}"
        ))
    })?;
    let mut client = RawDataClient::new(channel)
        .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
        .max_decoding_message_size(request.max_decoding_message_size);

    let mut grpc_request = tonic::Request::new(GetTransactionsRequest {
        starting_version: Some(request.start_version),
        transactions_count: Some(expected_count),
        batch_size: Some(request.batch_size),
        ..GetTransactionsRequest::default()
    });
    let auth_header = format!("Bearer {}", request.auth_token);
    let auth_value = auth_header
        .parse()
        .map_err(|_| HotIndexError::Config("invalid auth token metadata".to_string()))?;
    grpc_request
        .metadata_mut()
        .insert("authorization", auth_value);
    grpc_request.metadata_mut().insert(
        "x-aptos-request-name",
        "decibel-hotindex-record"
            .parse()
            .map_err(|_| HotIndexError::Config("invalid request-name metadata".to_string()))?,
    );

    let tmp_path = request.out_dir.join(format!(
        "transactions_{}_{}.pb.zst.tmp",
        request.start_version, request.end_version
    ));
    let mut writer = TransactionChunkWriter::create(&tmp_path)?;
    let mut stream = client
        .get_transactions(grpc_request)
        .await
        .map_err(|error| HotIndexError::Config(error.to_string()))?
        .into_inner();

    let mut count = 0_u64;
    let mut last_version = None;
    let mut chain_id = None;
    while let Some(response) = stream
        .message()
        .await
        .map_err(|error| HotIndexError::Config(error.to_string()))?
    {
        chain_id = response.chain_id.or(chain_id);
        for transaction in response.transactions {
            last_version = Some(transaction.version);
            writer.write_transaction(&transaction)?;
            count += 1;
        }
    }
    writer.finish()?;

    if count == 0 {
        return Err(HotIndexError::Config(
            "Transaction Stream returned zero transactions".to_string(),
        ));
    }
    let last_success_version = last_version.unwrap_or(request.start_version);
    let final_path = request.out_dir.join(format!(
        "transactions_{}_{}.pb.zst",
        request.start_version, last_success_version
    ));
    fs::rename(&tmp_path, &final_path)?;
    let chunk_sha256 = sha256_file(&final_path)?;
    let chunk_name = final_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("transactions.pb.zst")
        .to_string();
    let checkpoint = serde_json::json!({
        "status": "complete",
        "message": "recorded Aptos Transaction Stream raw protobuf chunk",
        "network": request.network.as_str(),
        "endpoint": request.endpoint,
        "package_address": request.package_address,
        "orderbook_address": request.orderbook_address,
        "start_version": request.start_version,
        "end_version": request.end_version,
        "raw_format": request.raw_format,
        "auth_token_present": true,
        "chain_id": chain_id,
        "transaction_count": count,
        "last_success_version": last_success_version,
        "chunks": [{
            "path": chunk_name,
            "first_version": request.start_version,
            "last_version": last_success_version,
            "transaction_count": count,
            "sha256": chunk_sha256
        }]
    });
    write_json_pretty(&request.out_dir.join("record_checkpoint.json"), &checkpoint)?;
    println!(
        "recorded transaction stream: tx={} range={}..{} chunk={}",
        count,
        request.start_version,
        last_success_version,
        final_path.display()
    );
    Ok(())
}

struct TransactionChunkWriter {
    encoder: Option<zstd::stream::write::Encoder<'static, BufWriter<File>>>,
}

impl TransactionChunkWriter {
    fn create(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = File::create(path)?;
        let writer = BufWriter::new(file);
        let encoder = zstd::stream::write::Encoder::new(writer, 3)?;
        Ok(Self {
            encoder: Some(encoder),
        })
    }

    fn write_transaction(&mut self, transaction: &Transaction) -> Result<()> {
        let Some(encoder) = self.encoder.as_mut() else {
            return Err(HotIndexError::Config(
                "transaction chunk writer is already finished".to_string(),
            ));
        };
        let mut buffer = Vec::new();
        transaction
            .encode_length_delimited(&mut buffer)
            .map_err(|error| HotIndexError::Parse(error.to_string()))?;
        encoder.write_all(&buffer)?;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        let Some(encoder) = self.encoder.take() else {
            return Ok(());
        };
        let mut writer = encoder.finish()?;
        writer.flush()?;
        let file = writer
            .into_inner()
            .map_err(|error| HotIndexError::Config(error.to_string()))?;
        file.sync_all()?;
        Ok(())
    }
}

fn auth_token_present(opts: &Args<'_>) -> bool {
    if opts
        .optional_value("--auth-token")
        .map(|value| !value.is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    opts.optional_value("--auth-token-env")
        .and_then(|name| env::var(name).ok())
        .map(|value| !value.is_empty())
        .unwrap_or(false)
}

fn auth_token(opts: &Args<'_>) -> Result<String> {
    if let Some(value) = opts.optional_value("--auth-token") {
        if !value.is_empty() {
            return Ok(value.to_string());
        }
    }
    let env_name = opts
        .optional_value("--auth-token-env")
        .unwrap_or("APTOS_GRPC_AUTH_TOKEN");
    let value = env::var(env_name).map_err(|_| {
        HotIndexError::Config(format!(
            "missing auth token; set {env_name} or pass --auth-token-env <env>"
        ))
    })?;
    if value.is_empty() {
        return Err(HotIndexError::Config(format!(
            "auth token env var {env_name} is empty"
        )));
    }
    Ok(value)
}

fn normalize_grpc_endpoint(endpoint: &str) -> Result<String> {
    if endpoint.is_empty() {
        return Err(HotIndexError::Config(
            "missing required argument for --live: --endpoint".to_string(),
        ));
    }
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        Ok(endpoint.to_string())
    } else {
        Ok(format!("https://{endpoint}"))
    }
}

fn write_synthetic_dataset(root: &Path, dataset: &SyntheticDataset) -> Result<()> {
    let normalized = root.join("normalized");
    let queries = root.join("queries");
    fs::create_dir_all(&normalized)?;
    fs::create_dir_all(&queries)?;

    write_ndjson(&normalized.join("txs.ndjson"), &dataset.txs)?;
    write_ndjson(&normalized.join("events.ndjson"), &dataset.events)?;
    write_ndjson(&normalized.join("fills.ndjson"), &dataset.fills)?;
    write_ndjson(&normalized.join("orders.ndjson"), &dataset.orders)?;
    write_ndjson(&normalized.join("positions.ndjson"), &dataset.positions)?;
    write_ndjson(
        &normalized.join("builder_code_rows.ndjson"),
        &dataset.builder_rows,
    )?;
    write_ndjson::<ActivityRow>(&normalized.join("activity_rows.ndjson"), &[])?;
    write_ndjson::<NormalizedEvent>(&normalized.join("unknown_events.ndjson"), &[])?;
    fs::write(normalized.join("parse_warnings.log"), "")?;

    let corpus = build_query_corpus(&dataset.events);
    write_query_corpus_files(&queries, &corpus)?;

    let mut hashes = BTreeMap::new();
    for relative in NORMALIZED_ARTIFACTS
        .iter()
        .chain(QUERY_CORPUS_ARTIFACTS.iter())
    {
        hashes.insert(relative.to_string(), sha256_file(&root.join(relative))?);
    }

    let manifest = DatasetManifest {
        dataset_id: DatasetId("synthetic_smoke".to_string()),
        network: Network::Local,
        source: "synthetic".to_string(),
        transaction_stream_endpoint: None,
        raw_encoding: DatasetEncoding::Synthetic,
        normalized_encoding: DatasetEncoding::Ndjson,
        start_version: dataset
            .events
            .first()
            .map(|event| event.version)
            .unwrap_or_default(),
        end_version: dataset.events.last().map(|event| event.version),
        package_address: zero_address(),
        orderbook_address: zero_address(),
        parser_source: Some("decibel-dataset synthetic".to_string()),
        parser_commit: None,
        captured_at: Some("1970-01-01T00:00:00Z".to_string()),
        raw_transaction_count: dataset.txs.len() as u64,
        decibel_event_count: dataset.events.len() as u64,
        fill_count: dataset.fills.len() as u64,
        order_count: dataset.orders.len() as u64,
        position_count: dataset.positions.len() as u64,
        builder_code_row_count: dataset.builder_rows.len() as u64,
        hashes: DatasetFileHashes { sha256: hashes },
    };
    write_json_pretty(&root.join("manifest.json"), &manifest)?;
    Ok(())
}

fn replay_into_memory(root: &Path) -> Result<MemoryEngine> {
    let engine = MemoryEngine::default();
    replay_into_engine(root, &engine)?;
    Ok(engine)
}

fn replay_into_engine<E: StorageEngine>(root: &Path, engine: &E) -> Result<()> {
    let normalized = root.join("normalized");
    let manifest = read_json::<DatasetManifest>(&root.join("manifest.json"))?;
    validate_manifest_hashes(root, &manifest)?;

    for tx in read_ndjson::<TxRow>(&normalized.join("txs.ndjson"))? {
        engine.put_tx(tx)?;
    }
    for event in read_ndjson::<NormalizedEvent>(&normalized.join("events.ndjson"))? {
        engine.put_event(event)?;
    }
    for fill in read_ndjson::<FillRow>(&normalized.join("fills.ndjson"))? {
        engine.put_fill(fill)?;
    }
    for order in read_ndjson::<OrderRow>(&normalized.join("orders.ndjson"))? {
        engine.put_order(order)?;
    }
    for position in read_ndjson::<PositionRow>(&normalized.join("positions.ndjson"))? {
        engine.put_position(position)?;
    }
    for row in read_ndjson::<BuilderAttributionRow>(&normalized.join("builder_code_rows.ndjson"))? {
        engine.put_builder_attribution(row)?;
    }
    for row in read_ndjson::<ActivityRow>(&normalized.join("activity_rows.ndjson"))? {
        engine.put_activity(row)?;
    }

    engine.put_ingest_checkpoint(IngestCheckpoint {
        network: manifest.network,
        package_address: manifest.package_address,
        dataset_id: Some(manifest.dataset_id),
        last_processed_version: manifest.end_version.unwrap_or(manifest.start_version),
        last_processed_timestamp_us: 0,
        events_indexed: manifest.decibel_event_count,
        fills_indexed: manifest.fill_count,
        builder_attributions_indexed: manifest.builder_code_row_count,
    })?;

    Ok(())
}

fn print_replay_result<E: StorageEngine>(engine: &E) -> Result<()> {
    let stats = engine.stats()?;
    println!(
        "replay complete: tx={} events={} fills={} builder_rows={}",
        stats.tx_count, stats.event_count, stats.fill_count, stats.builder_attribution_count
    );
    for checksum in engine.checksums()? {
        println!(
            "checksum {} rows={} hash={}",
            checksum.cf_name, checksum.row_count, checksum.hash_hex
        );
    }
    Ok(())
}

fn write_normalized_fixture_dataset(
    dataset_root: &Path,
    normalized_dir: &Path,
    raw_input: &Path,
    parser_options: &ParserOptions,
    rows: ParserOutput,
) -> Result<()> {
    fs::create_dir_all(normalized_dir)?;
    write_ndjson(&normalized_dir.join("txs.ndjson"), &rows.txs)?;
    write_ndjson(&normalized_dir.join("events.ndjson"), &rows.events)?;
    write_ndjson(&normalized_dir.join("fills.ndjson"), &rows.fills)?;
    write_ndjson(&normalized_dir.join("orders.ndjson"), &rows.orders)?;
    write_ndjson(&normalized_dir.join("positions.ndjson"), &rows.positions)?;
    write_ndjson(
        &normalized_dir.join("builder_code_rows.ndjson"),
        &rows.builder_rows,
    )?;
    write_ndjson(
        &normalized_dir.join("activity_rows.ndjson"),
        &rows.activity_rows,
    )?;
    write_ndjson(
        &normalized_dir.join("unknown_events.ndjson"),
        &rows.unknown_events,
    )?;
    fs::write(
        normalized_dir.join("parse_warnings.log"),
        rows.warnings.join("\n"),
    )?;

    let mut hashes = BTreeMap::new();
    insert_hash(&mut hashes, dataset_root, raw_input)?;
    for relative in NORMALIZED_ARTIFACTS {
        insert_hash(&mut hashes, dataset_root, &dataset_root.join(relative))?;
    }

    let start_version = rows
        .txs
        .iter()
        .map(|tx| tx.version)
        .min()
        .unwrap_or_default();
    let end_version = rows.txs.iter().map(|tx| tx.version).max();
    let manifest = DatasetManifest {
        dataset_id: parser_options.dataset_id.clone(),
        network: parser_options.network,
        source: "fixture_jsonl".to_string(),
        transaction_stream_endpoint: None,
        raw_encoding: DatasetEncoding::Ndjson,
        normalized_encoding: DatasetEncoding::Ndjson,
        start_version,
        end_version,
        package_address: parser_options.package_address.clone(),
        orderbook_address: parser_options.orderbook_address.clone(),
        parser_source: Some(parser_options.parser_source.clone()),
        parser_commit: parser_options.parser_commit.clone(),
        captured_at: Some("1970-01-01T00:00:00Z".to_string()),
        raw_transaction_count: rows.raw_transaction_count,
        decibel_event_count: rows.events.len() as u64,
        fill_count: rows.fills.len() as u64,
        order_count: rows.orders.len() as u64,
        position_count: rows.positions.len() as u64,
        builder_code_row_count: rows.builder_rows.len() as u64,
        hashes: DatasetFileHashes { sha256: hashes },
    };
    write_json_pretty(&dataset_root.join("manifest.json"), &manifest)
}

fn write_query_corpus_files(out_dir: &Path, corpus: &[QueryCorpusRecord]) -> Result<()> {
    fs::create_dir_all(out_dir)?;
    write_ndjson(&out_dir.join("mixed_dashboard.ndjson"), corpus)?;
    write_query_kind_file(
        out_dir,
        "point_tx_versions.ndjson",
        corpus,
        QueryKind::GetTxByVersion,
    )?;
    write_query_kind_file(
        out_dir,
        "multi_get_tx_versions.ndjson",
        corpus,
        QueryKind::MultiGetTxVersions,
    )?;
    write_query_kind_file(
        out_dir,
        "market_fill_scans.ndjson",
        corpus,
        QueryKind::MarketFillScan,
    )?;
    write_query_kind_file(
        out_dir,
        "account_fill_scans.ndjson",
        corpus,
        QueryKind::AccountFillScan,
    )?;
    write_query_kind_file(
        out_dir,
        "builder_code_scans.ndjson",
        corpus,
        QueryKind::BuilderCodeFillScan,
    )?;
    write_query_kind_file(
        out_dir,
        "builder_code_volumes.ndjson",
        corpus,
        QueryKind::BuilderCodeVolume,
    )?;
    Ok(())
}

fn write_query_kind_file(
    out_dir: &Path,
    file_name: &str,
    corpus: &[QueryCorpusRecord],
    query_kind: QueryKind,
) -> Result<()> {
    let rows = corpus
        .iter()
        .filter(|record| record.query_kind == query_kind)
        .cloned()
        .collect::<Vec<_>>();
    write_ndjson(&out_dir.join(file_name), &rows)
}

fn update_manifest_query_hashes_if_present(query_dir: &Path) -> Result<()> {
    if query_dir.file_name().and_then(|name| name.to_str()) != Some("queries") {
        return Ok(());
    }
    let Some(dataset_root) = query_dir.parent() else {
        return Ok(());
    };
    let manifest_path = dataset_root.join("manifest.json");
    if !manifest_path.exists() {
        return Ok(());
    }

    let mut manifest = read_json::<DatasetManifest>(&manifest_path)?;
    for relative in QUERY_CORPUS_ARTIFACTS {
        manifest.hashes.sha256.insert(
            relative.to_string(),
            sha256_file(&dataset_root.join(relative))?,
        );
    }
    write_json_pretty(&manifest_path, &manifest)
}

fn validate_manifest_hashes(root: &Path, manifest: &DatasetManifest) -> Result<()> {
    for (relative, expected) in &manifest.hashes.sha256 {
        let path = root.join(relative);
        let actual = sha256_file(&path).map_err(|error| {
            HotIndexError::Config(format!(
                "manifest artifact {} is not readable: {error}",
                path.display()
            ))
        })?;
        if actual != *expected {
            return Err(HotIndexError::Config(format!(
                "sha256 mismatch for {relative}: expected {expected}, got {actual}"
            )));
        }
    }
    Ok(())
}

fn insert_hash(hashes: &mut BTreeMap<String, String>, root: &Path, path: &Path) -> Result<()> {
    let key = artifact_key(root, path);
    hashes.insert(key, sha256_file(path)?);
    Ok(())
}

fn artifact_key(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

fn build_query_corpus(events: &[NormalizedEvent]) -> Vec<QueryCorpusRecord> {
    let mut corpus = Vec::new();
    let mut seen_versions = BTreeSet::new();
    let mut seen_markets = BTreeSet::new();
    let mut seen_accounts = BTreeSet::new();
    let mut seen_builders = BTreeSet::new();

    for event in events {
        if seen_versions.insert(event.version) {
            corpus.push(QueryCorpusRecord {
                query_kind: QueryKind::GetTxByVersion,
                tx_version: Some(event.version),
                tx_versions: Vec::new(),
                market_id: None,
                account: None,
                builder_addr: None,
                limit: None,
            });
        }
        if let Some(market_id) = &event.market_id {
            if seen_markets.insert(market_id.clone()) {
                corpus.push(QueryCorpusRecord {
                    query_kind: QueryKind::MarketFillScan,
                    tx_version: None,
                    tx_versions: Vec::new(),
                    market_id: Some(market_id.clone()),
                    account: None,
                    builder_addr: None,
                    limit: Some(100),
                });
            }
        }
        if let Some(account) = &event.account {
            if seen_accounts.insert(account.clone()) {
                corpus.push(QueryCorpusRecord {
                    query_kind: QueryKind::AccountFillScan,
                    tx_version: None,
                    tx_versions: Vec::new(),
                    market_id: None,
                    account: Some(account.clone()),
                    builder_addr: None,
                    limit: Some(100),
                });
            }
        }
        if let Some(builder_addr) = &event.builder_addr {
            if seen_builders.insert(builder_addr.clone()) {
                corpus.push(QueryCorpusRecord {
                    query_kind: QueryKind::BuilderCodeFillScan,
                    tx_version: None,
                    tx_versions: Vec::new(),
                    market_id: None,
                    account: None,
                    builder_addr: Some(builder_addr.clone()),
                    limit: Some(100),
                });
                corpus.push(QueryCorpusRecord {
                    query_kind: QueryKind::BuilderCodeVolume,
                    tx_version: None,
                    tx_versions: Vec::new(),
                    market_id: None,
                    account: None,
                    builder_addr: Some(builder_addr.clone()),
                    limit: None,
                });
            }
        }
    }

    let versions: Vec<_> = seen_versions.iter().copied().take(100).collect();
    if !versions.is_empty() {
        corpus.push(QueryCorpusRecord {
            query_kind: QueryKind::MultiGetTxVersions,
            tx_version: None,
            tx_versions: versions,
            market_id: None,
            account: None,
            builder_addr: None,
            limit: None,
        });
    }

    corpus
}

#[derive(Debug, Default)]
struct SyntheticDataset {
    txs: Vec<TxRow>,
    events: Vec<NormalizedEvent>,
    fills: Vec<FillRow>,
    orders: Vec<OrderRow>,
    positions: Vec<PositionRow>,
    builder_rows: Vec<BuilderAttributionRow>,
}

impl SyntheticDataset {
    fn generate(count: usize) -> Self {
        let mut dataset = Self::default();
        let markets = ["BTC-PERP", "ETH-PERP", "APT-PERP"];
        let base_version = 4_365_621_793_u64;
        let base_ts = 1_770_000_000_000_000_u64;

        for idx in 0..count {
            let version = base_version + idx as u64;
            let timestamp_us = base_ts + idx as u64 * 1_000_000;
            let market_id = markets[idx % markets.len()].to_string();
            let account = format!("0x{:064x}", 1 + idx % 32);
            let builder_addr = format!("0x{:064x}", 10_000 + idx % 4);
            let fill_id = format!("fill-{idx:08}");
            let order_id = format!("order-{idx:08}");
            let notional = format!("{}", 100 + idx % 50);

            dataset.txs.push(TxRow {
                network: Network::Mainnet,
                version,
                tx_hash: format!("0x{:064x}", version),
                block_timestamp_us: timestamp_us,
                event_count: 1,
                dataset_id: Some(DatasetId("synthetic_smoke".to_string())),
                raw_summary: Some("synthetic Decibel-like transaction".to_string()),
            });
            dataset.events.push(NormalizedEvent {
                network: Network::Mainnet,
                version,
                event_idx: 0,
                tx_hash: format!("0x{:064x}", version),
                block_timestamp_us: timestamp_us,
                event_type: DecibelEventType::OrderFilled,
                market_id: Some(market_id.clone()),
                account: Some(account.clone()),
                subaccount: None,
                order_id: Some(order_id.clone()),
                builder_addr: Some(builder_addr.clone()),
                payload: DecibelEventPayload::RawJson {
                    value: format!(
                        "{{\"synthetic\":true,\"market_id\":\"{}\",\"fill_id\":\"{}\"}}",
                        market_id, fill_id
                    ),
                },
            });
            dataset.fills.push(FillRow {
                version,
                event_idx: 0,
                fill_id: fill_id.clone(),
                market_id: market_id.clone(),
                account: account.clone(),
                subaccount: None,
                order_id: Some(order_id.clone()),
                side: Some(if idx % 2 == 0 { "buy" } else { "sell" }.to_string()),
                price: format!("{}", 1000 + idx % 100),
                size: "1".to_string(),
                notional: Some(notional.clone()),
                builder_addr: Some(builder_addr.clone()),
                builder_fee_bps: Some(5),
                timestamp_us,
                raw_event_type: "TradeEvent".to_string(),
            });
            dataset.orders.push(OrderRow {
                order_id,
                account: account.clone(),
                market_id: market_id.clone(),
                side: Some(if idx % 2 == 0 { "buy" } else { "sell" }.to_string()),
                price: Some(format!("{}", 1000 + idx % 100)),
                size: Some("1".to_string()),
                status: Some("filled".to_string()),
                version,
                event_idx: 0,
                timestamp_us,
                raw_event_type: "OrderEvent".to_string(),
            });
            dataset.positions.push(PositionRow {
                account: account.clone(),
                market_id: market_id.clone(),
                subaccount: None,
                size: if idx % 2 == 0 {
                    "1".to_string()
                } else {
                    "-1".to_string()
                },
                entry_price: Some(format!("{}", 1000 + idx % 100)),
                source_version: version,
                source_event_idx: 0,
                timestamp_us,
            });
            dataset.builder_rows.push(BuilderAttributionRow {
                builder_addr,
                market_id,
                account,
                version,
                event_idx: 0,
                fill_id,
                notional: Some(notional),
                builder_fee_bps: Some(5),
                estimated_fee_amount: Some("0.01".to_string()),
                timestamp_us,
            });
        }

        dataset
    }
}

fn build_fixture_raw_events(
    count: usize,
    network: Network,
    package_address: &str,
    _orderbook_address: &str,
) -> Vec<serde_json::Value> {
    let markets = ["BTC-PERP", "ETH-PERP", "APT-PERP"];
    let base_version = 4_365_621_793_u64;
    let base_ts = 1_770_000_000_000_000_u64;

    (0..count)
        .map(|idx| {
            let version = base_version + idx as u64;
            let timestamp_us = base_ts + idx as u64 * 1_000_000;
            let market_id = markets[idx % markets.len()];
            let account = format!("0x{:064x}", 1 + idx % 16);
            let builder_addr = format!("0x{:064x}", 10_000 + idx % 4);
            let event_type = match idx % 5 {
                0 => "TradeEvent",
                1 => "OrderEvent",
                2 => "PositionUpdateEvent",
                3 => "LiquidationEvent",
                _ => "MysteryEvent",
            };
            let data = match idx % 5 {
                0 => serde_json::json!({
                    "market_id": market_id,
                    "account": account,
                    "order_id": format!("order-{idx:08}"),
                    "fill_id": format!("fill-{idx:08}"),
                    "side": if idx % 2 == 0 { "buy" } else { "sell" },
                    "price": format!("{}", 1000 + idx % 100),
                    "size": "1",
                    "notional": format!("{}", 100 + idx % 50),
                    "builder_addr": builder_addr,
                    "builder_fee_bps": 5,
                    "estimated_fee_amount": "0.01"
                }),
                1 => serde_json::json!({
                    "market_id": market_id,
                    "account": account,
                    "order_id": format!("order-{idx:08}"),
                    "side": if idx % 2 == 0 { "buy" } else { "sell" },
                    "price": format!("{}", 1000 + idx % 100),
                    "size": "1",
                    "status": "open"
                }),
                2 => serde_json::json!({
                    "market_id": market_id,
                    "account": account,
                    "size": if idx % 2 == 0 { "1" } else { "-1" },
                    "entry_price": format!("{}", 1000 + idx % 100)
                }),
                3 => serde_json::json!({
                    "market_id": market_id,
                    "account": account,
                    "liquidation_id": format!("liq-{idx:08}")
                }),
                _ => serde_json::json!({
                    "market_id": market_id,
                    "account": account,
                    "raw": "preserve me"
                }),
            };

            serde_json::json!({
                "network": network.as_str(),
                "version": version,
                "tx_hash": format!("0x{version:064x}"),
                "block_timestamp_us": timestamp_us,
                "events": [{
                    "event_idx": 0,
                    "type": format!("{package_address}::orderbook::{event_type}"),
                    "package_address": package_address,
                    "data": data
                }]
            })
        })
        .collect()
}

fn parser_options_from_args(opts: &Args<'_>, dataset_root: &Path) -> Result<ParserOptions> {
    let config = load_optional_config(opts)?;
    let network = resolve_network(opts, config.as_ref())?;
    let dataset_id = opts
        .optional_value("--dataset-id")
        .map(str::to_string)
        .or_else(|| config.as_ref().map(|config| config.dataset.id.clone()))
        .unwrap_or_else(|| {
            dataset_root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("fixture_decibel")
                .to_string()
        });
    Ok(ParserOptions {
        network,
        dataset_id: DatasetId(dataset_id),
        package_address: resolve_package_address(opts, config.as_ref(), network)?,
        orderbook_address: resolve_orderbook_address(opts, config.as_ref(), network)?,
        parser_source: opts
            .optional_value("--parser-source")
            .unwrap_or("decibel-hotindex-ingest fixture-jsonl parser")
            .to_string(),
        parser_commit: opts.optional_value("--parser-commit").map(str::to_string),
    })
}

fn load_optional_config(opts: &Args<'_>) -> Result<Option<AppConfig>> {
    opts.optional_value("--config")
        .map(AppConfig::from_path)
        .transpose()
}

fn resolve_network(opts: &Args<'_>, config: Option<&AppConfig>) -> Result<Network> {
    if let Some(network) = opts.optional_value("--network") {
        return parse_network(network);
    }
    Ok(config
        .map(|config| config.network)
        .unwrap_or(Network::Mainnet))
}

fn resolve_package_address(
    opts: &Args<'_>,
    config: Option<&AppConfig>,
    network: Network,
) -> Result<String> {
    let raw = opts
        .optional_value("--package-address")
        .map(str::to_string)
        .or_else(|| config.map(|config| config.decibel.package_address.clone()))
        .unwrap_or_else(|| default_decibel_address(network).to_string());
    normalize_aptos_address(&raw)
}

fn resolve_orderbook_address(
    opts: &Args<'_>,
    config: Option<&AppConfig>,
    network: Network,
) -> Result<String> {
    let raw = opts
        .optional_value("--orderbook-address")
        .map(str::to_string)
        .or_else(|| config.map(|config| config.decibel.orderbook_address.clone()))
        .unwrap_or_else(|| default_decibel_address(network).to_string());
    normalize_aptos_address(&raw)
}

fn parse_network(value: &str) -> Result<Network> {
    match value {
        "local" => Ok(Network::Local),
        "devnet" => Ok(Network::Devnet),
        "testnet" => Ok(Network::Testnet),
        "mainnet" => Ok(Network::Mainnet),
        other => Err(HotIndexError::Config(format!(
            "unsupported network: {other}"
        ))),
    }
}

fn default_decibel_address(network: Network) -> &'static str {
    match network {
        Network::Mainnet => MAINNET_DECIBEL_ADDRESS,
        Network::Testnet => TESTNET_DECIBEL_ADDRESS,
        Network::Local | Network::Devnet => {
            "0x0000000000000000000000000000000000000000000000000000000000000000"
        }
    }
}

fn infer_raw_format(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("jsonl" | "ndjson") => "fixture-jsonl",
        _ => "protobuf-zstd",
    }
}

fn dataset_root_for_normalized_dir(normalized_dir: &Path) -> Result<PathBuf> {
    if normalized_dir.file_name().and_then(|name| name.to_str()) == Some("normalized") {
        normalized_dir
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| {
                HotIndexError::Config(format!(
                    "normalized output {} has no dataset root parent",
                    normalized_dir.display()
                ))
            })
    } else {
        Ok(normalized_dir.to_path_buf())
    }
}

struct Args<'a> {
    args: &'a [String],
}

impl<'a> Args<'a> {
    fn new(args: &'a [String]) -> Self {
        Self { args }
    }

    fn required_path(&self, name: &str) -> Result<PathBuf> {
        self.optional_value(name)
            .map(PathBuf::from)
            .ok_or_else(|| HotIndexError::Config(format!("missing required argument: {name}")))
    }

    fn optional_value(&self, name: &str) -> Option<&'a str> {
        self.args
            .windows(2)
            .find(|window| window[0] == name)
            .map(|window| window[1].as_str())
    }

    fn has_flag(&self, name: &str) -> bool {
        self.args.iter().any(|arg| arg == name)
    }

    fn optional_u64(&self, name: &str) -> Result<Option<u64>> {
        self.optional_value(name)
            .map(|value| {
                value.parse::<u64>().map_err(|_| {
                    HotIndexError::Config(format!("invalid integer for {name}: {value}"))
                })
            })
            .transpose()
    }
}

fn write_ndjson<T: Serialize>(path: &Path, rows: &[T]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    for row in rows {
        serde_json::to_writer(&mut writer, row).map_err(json_error)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn write_jsonl_values(path: &Path, rows: &[serde_json::Value]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    for row in rows {
        serde_json::to_writer(&mut writer, row).map_err(json_error)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn read_ndjson<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut rows = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let row = serde_json::from_str(&line).map_err(|error| {
            HotIndexError::Parse(format!("{}:{}: {error}", path.display(), idx + 1))
        })?;
        rows.push(row);
    }
    Ok(rows)
}

fn write_json_pretty<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    serde_json::to_writer_pretty(BufWriter::new(file), value).map_err(json_error)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = File::open(path)?;
    serde_json::from_reader(BufReader::new(file)).map_err(json_error)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn json_error(error: serde_json::Error) -> HotIndexError {
    HotIndexError::Parse(error.to_string())
}

fn zero_address() -> String {
    "0x0000000000000000000000000000000000000000000000000000000000000000".to_string()
}

fn print_usage() {
    eprintln!(
        "usage:
  decibel-dataset synthetic --out <dataset-dir> [--events <n>]
  decibel-dataset fixture --out <dataset-dir> [--events <n>] [--config <config.yaml>]
  decibel-dataset build-query-corpus --events <events.ndjson> --out-dir <queries-dir> [--seed <n>]
  decibel-dataset replay --dataset <dataset-dir> [--engine memory]
  decibel-dataset normalize --input <raw> --out-dir <normalized-dir> [--format fixture-jsonl] [--config <config.yaml>]
  decibel-dataset inspect-raw --input <transactions.pb.zst> [--limit <n>]
  decibel-dataset record --live --network mainnet --endpoint <url> [--auth-token <token>|--auth-token-env <env>] [--batch-size <n>] [--package-address <addr>] [--orderbook-address <addr>] --start-version <n> --end-version <n> --out-dir <raw-dir> --raw-format protobuf-zstd"
    );
}

#[cfg(test)]
mod tests {
    use super::{
        build_query_corpus, build_query_corpus_command, fixture_command, normalize_command,
        read_json, record_command, replay_into_memory, write_synthetic_dataset, DatasetManifest,
        HotIndexError, NormalizedEvent, SyntheticDataset, NORMALIZED_ARTIFACTS,
        QUERY_CORPUS_ARTIFACTS,
    };
    use decibel_hotindex_storage::StorageEngine;
    use std::path::PathBuf;

    #[test]
    fn synthetic_dataset_round_trips_into_memory() {
        let root = temp_root("round-trip");
        let _ = std::fs::remove_dir_all(&root);
        let dataset = SyntheticDataset::generate(12);
        write_synthetic_dataset(&root, &dataset).unwrap();

        let engine = replay_into_memory(&root).unwrap();
        let stats = engine.stats().unwrap();
        assert_eq!(stats.tx_count, 12);
        assert_eq!(stats.event_count, 12);
        assert_eq!(stats.fill_count, 12);
        assert_eq!(stats.builder_attribution_count, 12);

        let checksums = engine.checksums().unwrap();
        assert!(checksums.iter().any(|checksum| checksum.row_count > 0));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn synthetic_dataset_hashes_normalized_and_query_artifacts() {
        let root = temp_root("manifest");
        let _ = std::fs::remove_dir_all(&root);
        let dataset = SyntheticDataset::generate(8);
        write_synthetic_dataset(&root, &dataset).unwrap();

        let manifest = read_json::<DatasetManifest>(&root.join("manifest.json")).unwrap();
        for relative in NORMALIZED_ARTIFACTS
            .iter()
            .chain(QUERY_CORPUS_ARTIFACTS.iter())
        {
            assert!(
                root.join(relative).exists(),
                "missing expected artifact {relative}"
            );
            assert!(
                manifest.hashes.sha256.contains_key(*relative),
                "manifest did not hash {relative}"
            );
        }

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replay_rejects_manifest_hash_mismatch() {
        let root = temp_root("hash-mismatch");
        let _ = std::fs::remove_dir_all(&root);
        let dataset = SyntheticDataset::generate(4);
        write_synthetic_dataset(&root, &dataset).unwrap();
        std::fs::write(root.join("normalized/events.ndjson"), "{}\n").unwrap();

        let err = replay_into_memory(&root).unwrap_err();
        assert!(matches!(
            err,
            HotIndexError::Config(message) if message.contains("sha256 mismatch")
        ));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fixture_jsonl_normalizes_builds_queries_and_replays() {
        let root = temp_root("fixture-pipeline");
        let _ = std::fs::remove_dir_all(&root);
        fixture_command(&[
            "--out".to_string(),
            root.display().to_string(),
            "--events".to_string(),
            "10".to_string(),
        ])
        .unwrap();
        normalize_command(&[
            "--input".to_string(),
            root.join("raw/fixture_events.jsonl").display().to_string(),
            "--out-dir".to_string(),
            root.join("normalized").display().to_string(),
            "--format".to_string(),
            "fixture-jsonl".to_string(),
            "--dataset-id".to_string(),
            "fixture_pipeline".to_string(),
            "--parser-commit".to_string(),
            "test-commit".to_string(),
        ])
        .unwrap();
        build_query_corpus_command(&[
            "--events".to_string(),
            root.join("normalized/events.ndjson").display().to_string(),
            "--out-dir".to_string(),
            root.join("queries").display().to_string(),
        ])
        .unwrap();

        let unknown_events =
            super::read_ndjson::<NormalizedEvent>(&root.join("normalized/unknown_events.ndjson"))
                .unwrap();
        assert_eq!(unknown_events.len(), 2);

        let manifest = read_json::<DatasetManifest>(&root.join("manifest.json")).unwrap();
        assert_eq!(manifest.dataset_id.0, "fixture_pipeline");
        assert_eq!(manifest.parser_commit.as_deref(), Some("test-commit"));
        assert!(manifest
            .hashes
            .sha256
            .contains_key("raw/fixture_events.jsonl"));
        assert!(manifest
            .hashes
            .sha256
            .contains_key("queries/mixed_dashboard.ndjson"));

        let engine = replay_into_memory(&root).unwrap();
        let stats = engine.stats().unwrap();
        assert_eq!(stats.tx_count, 10);
        assert_eq!(stats.event_count, 10);
        assert_eq!(stats.fill_count, 2);
        assert_eq!(stats.builder_attribution_count, 2);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn query_corpus_uses_hit_capable_keys() {
        let dataset = SyntheticDataset::generate(8);
        let corpus = build_query_corpus(&dataset.events);
        assert!(corpus.iter().any(|record| record.tx_version.is_some()));
        assert!(corpus.iter().any(|record| record.market_id.is_some()));
        assert!(corpus.iter().any(|record| record.account.is_some()));
        assert!(corpus.iter().any(|record| record.builder_addr.is_some()));
    }

    #[test]
    fn normalize_requires_existing_input_path() {
        let root = temp_root("normalize-missing");
        let _ = std::fs::remove_dir_all(&root);
        let args = vec![
            "--input".to_string(),
            root.join("missing.pb.zst").display().to_string(),
            "--out-dir".to_string(),
            root.join("normalized").display().to_string(),
        ];

        let err = normalize_command(&args).unwrap_err();
        assert!(err.to_string().contains("not readable"));
    }

    #[test]
    fn record_checkpoint_tracks_token_presence_without_secret() {
        let root = temp_root("record-token");
        let _ = std::fs::remove_dir_all(&root);
        let args = vec![
            "--network".to_string(),
            "mainnet".to_string(),
            "--endpoint".to_string(),
            "https://grpc.mainnet.aptoslabs.com:443".to_string(),
            "--auth-token".to_string(),
            "secret-token".to_string(),
            "--start-version".to_string(),
            "10".to_string(),
            "--end-version".to_string(),
            "20".to_string(),
            "--out-dir".to_string(),
            root.display().to_string(),
            "--raw-format".to_string(),
            "protobuf-zstd".to_string(),
        ];

        record_command(&args).unwrap();
        let checkpoint = std::fs::read_to_string(root.join("record_checkpoint.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&checkpoint).unwrap();
        assert_eq!(value["auth_token_present"], true);
        assert!(!checkpoint.contains("secret-token"));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn record_checkpoint_accepts_token_from_env_name() {
        let root = temp_root("record-token-env");
        let _ = std::fs::remove_dir_all(&root);
        let env_name = "DECIBEL_DATASET_TEST_AUTH_TOKEN";
        std::env::set_var(env_name, "secret-token-from-env");
        let args = vec![
            "--network".to_string(),
            "mainnet".to_string(),
            "--endpoint".to_string(),
            "grpc.mainnet.aptoslabs.com:443".to_string(),
            "--auth-token-env".to_string(),
            env_name.to_string(),
            "--start-version".to_string(),
            "10".to_string(),
            "--end-version".to_string(),
            "20".to_string(),
            "--out-dir".to_string(),
            root.display().to_string(),
            "--raw-format".to_string(),
            "protobuf-zstd".to_string(),
        ];

        record_command(&args).unwrap();
        let checkpoint = std::fs::read_to_string(root.join("record_checkpoint.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&checkpoint).unwrap();
        assert_eq!(value["auth_token_present"], true);
        assert!(!checkpoint.contains("secret-token-from-env"));

        std::env::remove_var(env_name);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("decibel-dataset-{name}-{}", std::process::id()))
    }
}
