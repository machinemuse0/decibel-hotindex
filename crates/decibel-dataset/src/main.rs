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
use std::io::{BufRead, BufReader, BufWriter, ErrorKind, Read, Write};
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
const DEFAULT_RECORD_CHUNK_TRANSACTION_COUNT: u64 = 100_000;

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
    let allow_truncated = opts.has_flag("--allow-truncated");
    let mut decoder = zstd::stream::read::Decoder::new(File::open(&input)?)?;
    let mut count = 0_u64;
    let mut first_version = None;
    let mut last_version = None;
    let mut truncated_error = None;
    while count < limit {
        match read_next_len_delimited_transaction(&mut decoder) {
            Ok(Some(tx)) => {
                first_version.get_or_insert(tx.version);
                last_version = Some(tx.version);
                count += 1;
            }
            Ok(None) => break,
            Err(error) if allow_truncated => {
                truncated_error = Some(error.to_string());
                break;
            }
            Err(error) => return Err(error),
        }
    }
    let report = serde_json::json!({
        "input": input.display().to_string(),
        "decoded_transactions": count,
        "first_version": first_version,
        "last_version": last_version,
        "next_start_version": last_version.and_then(|version| version.checked_add(1)),
        "truncated_error": truncated_error,
        "stopped_at_limit": count == limit
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
            "no normalized Decibel events found in {}; real Aptos protobuf normalization is currently tx-only, so use fixture/synthetic data for Decibel serving workloads until event extraction lands",
            events.display(),
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
    if raw_format == "protobuf-zstd" || raw_format == "aptos-transaction-protobuf-zstd" {
        let dataset_root = dataset_root_for_normalized_dir(&out_dir)?;
        let parser_options = parser_options_from_args(&opts, &dataset_root)?;
        let raw_inputs = resolve_protobuf_raw_inputs(&input)?;
        let raw_input_count = raw_inputs.len();
        normalize_protobuf_tx_only(&dataset_root, &out_dir, &raw_inputs, &parser_options)?;
        println!(
            "normalized Aptos protobuf tx-only dataset written at {} from {} raw chunk(s)",
            out_dir.display(),
            raw_input_count
        );
        return Ok(());
    }

    if raw_format != "fixture-jsonl" {
        return Err(HotIndexError::Config(format!(
            "unsupported normalize raw format: {raw_format}"
        )));
    }
    if input.is_dir() {
        return Err(HotIndexError::Config(format!(
            "fixture-jsonl normalize requires a file input, got directory {}",
            input.display()
        )));
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
    let out_dir = opts.required_path("--out-dir")?;
    let resume = opts.has_flag("--resume");
    let resume_state = load_record_resume_state(&out_dir)?;
    let requested_start_version = opts.optional_u64("--start-version")?;
    let start_version =
        resolve_record_start_version(requested_start_version, resume, &resume_state)?;
    let requested_end_version = opts.optional_u64("--end-version")?;
    let transactions_count = opts.optional_u64("--transactions-count")?;
    let end_version =
        resolve_record_end_version(start_version, requested_end_version, transactions_count)?;
    let max_raw_bytes = opts.optional_bytes("--max-raw-bytes")?;
    let max_stream_retries = opts.optional_u64("--max-stream-retries")?.unwrap_or(10);
    let retry_backoff_ms = opts.optional_u64("--retry-backoff-ms")?.unwrap_or(2_000);
    let record_chunk_transaction_count = opts
        .optional_u64("--chunk-transaction-count")?
        .unwrap_or(DEFAULT_RECORD_CHUNK_TRANSACTION_COUNT);
    if record_chunk_transaction_count == 0 {
        return Err(HotIndexError::Config(
            "--chunk-transaction-count must be greater than zero".to_string(),
        ));
    }
    let raw_format = opts
        .optional_value("--raw-format")
        .unwrap_or("protobuf-zstd");
    let package_address = resolve_package_address(&opts, config.as_ref(), network)?;
    let orderbook_address = resolve_orderbook_address(&opts, config.as_ref(), network)?;
    let auth_token_present = auth_token_present(&opts);
    fs::create_dir_all(&out_dir)?;

    if opts.has_flag("--live") {
        let token = auth_token(&opts)?;
        if network == Network::Mainnet
            && start_version < 1_000_000_000
            && end_version.unwrap_or(0) >= 1_000_000_000
            && !opts.has_flag("--allow-low-mainnet-start")
        {
            return Err(HotIndexError::Config(format!(
                "suspicious mainnet record range: start_version={start_version}, end_version={}. Did an environment variable or placeholder expand incorrectly? Pass --allow-low-mainnet-start to override.",
                end_version.unwrap_or(0)
            )));
        }
        let request = LiveRecordRequest {
            network,
            endpoint: endpoint.to_string(),
            start_version,
            end_version: end_version.ok_or_else(|| {
                HotIndexError::Config(
                    "--live record requires --end-version or --transactions-count".to_string(),
                )
            })?,
            requested_start_version,
            requested_end_version,
            transactions_count,
            resume,
            batch_size: opts.optional_u64("--batch-size")?.unwrap_or(100).min(1000),
            max_decoding_message_size: opts
                .optional_u64("--max-message-mb")?
                .unwrap_or(128)
                .saturating_mul(1024 * 1024) as usize,
            max_raw_bytes,
            chunk_transaction_count: record_chunk_transaction_count,
            key_sample_limit: opts
                .optional_u64("--key-sample-limit")?
                .unwrap_or(1_000_000),
            out_dir: out_dir.clone(),
            raw_format: raw_format.to_string(),
            package_address,
            orderbook_address,
            auth_token: token,
            previous_chunks: if resume {
                resume_state.chunks.clone()
            } else {
                Vec::new()
            },
        };
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|error| HotIndexError::Config(error.to_string()))?;
        return runtime.block_on(record_live_transaction_stream_with_retries(
            request,
            max_stream_retries,
            Duration::from_millis(retry_backoff_ms),
        ));
    }

    let resume_transaction_count = resume_state.chunks.iter().try_fold(0_u64, |acc, chunk| {
        acc.checked_add(chunk_transaction_count(chunk))
            .ok_or_else(|| {
                HotIndexError::Config("checkpoint transaction count overflow".to_string())
            })
    })?;
    let checkpoint = serde_json::json!({
        "status": "planned",
        "message": "record skeleton only; Aptos gRPC recording is implemented in a later milestone",
        "network": network.as_str(),
        "endpoint": endpoint,
        "package_address": package_address,
        "orderbook_address": orderbook_address,
        "start_version": start_version,
        "end_version": end_version,
        "requested_start_version": requested_start_version,
        "requested_end_version": requested_end_version,
        "transactions_count": transactions_count,
        "resume": resume,
        "raw_format": raw_format,
        "auth_token_present": auth_token_present,
        "last_success_version": resume_state.last_success_version,
        "next_start_version": resume_state.next_start_version.unwrap_or(start_version),
        "max_raw_bytes": max_raw_bytes,
        "max_stream_retries": max_stream_retries,
        "retry_backoff_ms": retry_backoff_ms,
        "chunk_transaction_count": record_chunk_transaction_count,
        "transaction_count": resume_transaction_count,
        "chunks": resume_state.chunks
    });
    write_json_pretty(&out_dir.join("record_checkpoint.json"), &checkpoint)?;
    println!("record skeleton wrote checkpoint at {}", out_dir.display());
    Ok(())
}

#[derive(Clone)]
struct LiveRecordRequest {
    network: Network,
    endpoint: String,
    start_version: u64,
    end_version: u64,
    requested_start_version: Option<u64>,
    requested_end_version: Option<u64>,
    transactions_count: Option<u64>,
    resume: bool,
    batch_size: u64,
    max_decoding_message_size: usize,
    max_raw_bytes: Option<u64>,
    chunk_transaction_count: u64,
    key_sample_limit: u64,
    out_dir: PathBuf,
    raw_format: String,
    package_address: String,
    orderbook_address: String,
    auth_token: String,
    previous_chunks: Vec<serde_json::Value>,
}

enum RecordRunOutcome {
    Complete {
        next_start_version: u64,
        raw_bytes: u64,
        stopped_at_byte_limit: bool,
    },
    Interrupted {
        next_start_version: u64,
        raw_bytes: u64,
        stream_error: String,
    },
}

#[derive(Default)]
struct RecordResumeState {
    last_success_version: Option<u64>,
    next_start_version: Option<u64>,
    chunks: Vec<serde_json::Value>,
    resume_hint: Option<String>,
}

fn load_record_resume_state(out_dir: &Path) -> Result<RecordResumeState> {
    let checkpoint_path = out_dir.join("record_checkpoint.json");
    let mut state = if checkpoint_path.exists() {
        let value = read_json::<serde_json::Value>(&checkpoint_path)?;
        let last_success_version = value
            .get("last_success_version")
            .and_then(serde_json::Value::as_u64);
        let next_start_version = value
            .get("next_start_version")
            .and_then(serde_json::Value::as_u64)
            .or_else(|| last_success_version.and_then(|version| version.checked_add(1)));
        let chunks = value
            .get("chunks")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();

        RecordResumeState {
            last_success_version,
            next_start_version,
            chunks,
            resume_hint: None,
        }
    } else {
        RecordResumeState::default()
    };

    if state.next_start_version.is_none() {
        if let Some(inferred) = infer_record_resume_state_from_chunks(out_dir)? {
            state.last_success_version = inferred.last_success_version;
            state.next_start_version = inferred.next_start_version;
            if state.chunks.is_empty() {
                state.chunks = inferred.chunks;
            }
        }
    }

    if state.next_start_version.is_none() {
        state.resume_hint = resume_hint_from_tmp_chunks(out_dir)?;
    }

    Ok(state)
}

fn infer_record_resume_state_from_chunks(out_dir: &Path) -> Result<Option<RecordResumeState>> {
    if !out_dir.exists() {
        return Ok(None);
    }

    let mut ranges = Vec::new();
    for entry in fs::read_dir(out_dir)? {
        let path = entry?.path();
        if let Some((first_version, last_version)) = raw_chunk_version_range(&path) {
            ranges.push((first_version, last_version, path));
        }
    }
    if ranges.is_empty() {
        return Ok(None);
    }

    ranges.sort_by_key(|(first_version, last_version, _)| (*first_version, *last_version));
    let last_success_version = ranges
        .iter()
        .map(|(_, last_version, _)| *last_version)
        .max()
        .ok_or_else(|| HotIndexError::Config("no raw chunk ranges found".to_string()))?;
    let mut chunks = Vec::with_capacity(ranges.len());
    for (first_version, last_version, path) in ranges {
        let transaction_count = last_version
            .checked_sub(first_version)
            .and_then(|span| span.checked_add(1))
            .ok_or_else(|| {
                HotIndexError::Config(format!(
                    "invalid raw chunk version range in {}",
                    path.display()
                ))
            })?;
        let chunk_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("transactions.pb.zst");
        chunks.push(serde_json::json!({
            "path": chunk_name,
            "first_version": first_version,
            "last_version": last_version,
            "transaction_count": transaction_count,
            "sha256": null,
            "resume_inferred_from_filename": true
        }));
    }

    Ok(Some(RecordResumeState {
        last_success_version: Some(last_success_version),
        next_start_version: Some(checked_next_version(last_success_version)?),
        chunks,
        resume_hint: None,
    }))
}

fn resume_hint_from_tmp_chunks(out_dir: &Path) -> Result<Option<String>> {
    if !out_dir.exists() {
        return Ok(None);
    }

    let mut tmp_chunks = Vec::new();
    for entry in fs::read_dir(out_dir)? {
        let path = entry?.path();
        if is_transaction_tmp_chunk(&path) {
            tmp_chunks.push(path);
        }
    }
    tmp_chunks.sort();
    let Some(path) = tmp_chunks.last() else {
        return Ok(None);
    };

    Ok(Some(format!(
        "No completed transactions_*.pb.zst chunk or checkpoint cursor was found. A tmp chunk exists at {}; run `decibel-dataset inspect-raw --input {} --allow-truncated`, then retry record with `--start-version <next_start_version>`.",
        path.display(),
        path.display()
    )))
}

fn resolve_record_start_version(
    requested_start_version: Option<u64>,
    resume: bool,
    resume_state: &RecordResumeState,
) -> Result<u64> {
    if resume {
        if let Some(next_start_version) = resume_state.next_start_version {
            return Ok(next_start_version);
        }
        return requested_start_version.ok_or_else(|| {
            let mut message =
                "--resume requires record_checkpoint.json with next_start_version, an existing completed transactions_*.pb.zst chunk, or an explicit --start-version"
                    .to_string();
            if let Some(hint) = &resume_state.resume_hint {
                message.push_str(". ");
                message.push_str(hint);
            }
            HotIndexError::Config(message)
        });
    }

    Ok(requested_start_version.unwrap_or(0))
}

fn resolve_record_end_version(
    start_version: u64,
    requested_end_version: Option<u64>,
    transactions_count: Option<u64>,
) -> Result<Option<u64>> {
    match (requested_end_version, transactions_count) {
        (Some(_), Some(_)) => Err(HotIndexError::Config(
            "--end-version and --transactions-count cannot be used together".to_string(),
        )),
        (Some(end_version), None) => Ok(Some(end_version)),
        (None, Some(0)) => Err(HotIndexError::Config(
            "--transactions-count must be greater than zero".to_string(),
        )),
        (None, Some(count)) => start_version
            .checked_add(count - 1)
            .map(Some)
            .ok_or_else(|| {
                HotIndexError::Config(
                    "--transactions-count overflows u64 version range".to_string(),
                )
            }),
        (None, None) => Ok(None),
    }
}

async fn record_live_transaction_stream_with_retries(
    mut request: LiveRecordRequest,
    max_stream_retries: u64,
    retry_backoff: Duration,
) -> Result<()> {
    let initial_max_raw_bytes = request.max_raw_bytes;
    let mut total_raw_bytes = 0_u64;
    let mut retries = 0_u64;

    loop {
        let outcome = record_live_transaction_stream(request.clone()).await?;
        match outcome {
            RecordRunOutcome::Complete {
                next_start_version,
                raw_bytes,
                stopped_at_byte_limit,
            } => {
                total_raw_bytes = total_raw_bytes.checked_add(raw_bytes).ok_or_else(|| {
                    HotIndexError::Config("recorded raw byte count overflowed u64".to_string())
                })?;
                if stopped_at_byte_limit {
                    eprintln!(
                        "record stopped at byte limit after {total_raw_bytes} raw bytes; next_start_version={next_start_version}"
                    );
                }
                return Ok(());
            }
            RecordRunOutcome::Interrupted {
                next_start_version,
                raw_bytes,
                stream_error,
            } => {
                total_raw_bytes = total_raw_bytes.checked_add(raw_bytes).ok_or_else(|| {
                    HotIndexError::Config("recorded raw byte count overflowed u64".to_string())
                })?;
                if next_start_version > request.end_version {
                    return Ok(());
                }
                if let Some(limit) = initial_max_raw_bytes {
                    if total_raw_bytes >= limit {
                        eprintln!(
                            "record stopped after interruption because byte limit was reached; next_start_version={next_start_version}"
                        );
                        return Ok(());
                    }
                    request.max_raw_bytes = Some(limit - total_raw_bytes);
                }
                if retries >= max_stream_retries {
                    return Err(HotIndexError::Config(format!(
                        "Transaction Stream interrupted after retry limit; cursor saved at next_start_version={next_start_version}; last error: {stream_error}"
                    )));
                }
                retries += 1;
                eprintln!(
                    "Transaction Stream interrupted; retry {retries}/{max_stream_retries} from version {next_start_version} after {} ms",
                    retry_backoff.as_millis()
                );
                std::thread::sleep(retry_backoff);

                let resume_state = load_record_resume_state(&request.out_dir)?;
                request.start_version = next_start_version;
                request.requested_start_version = Some(next_start_version);
                request.resume = true;
                request.previous_chunks = resume_state.chunks;
            }
        }
    }
}

async fn record_live_transaction_stream(request: LiveRecordRequest) -> Result<RecordRunOutcome> {
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

    let mut key_writer = RecordKeyWriter::create(&request, expected_count)?;
    let mut stream = client
        .get_transactions(grpc_request)
        .await
        .map_err(|error| HotIndexError::Config(error.to_string()))?
        .into_inner();

    let mut count = 0_u64;
    let mut last_version = None;
    let mut chain_id = None;
    let mut uncompressed_raw_bytes = 0_u64;
    let mut stopped_at_byte_limit = false;
    let mut chunks = request.previous_chunks.clone();
    let mut active_chunk = None;

    'stream: loop {
        let response = match stream.message().await {
            Ok(Some(response)) => response,
            Ok(None) => break,
            Err(error) => {
                let stream_error = error.to_string();
                if count > 0 {
                    finish_active_chunk(&request, &mut active_chunk, &mut chunks)?;
                    let last_success_version = last_version.unwrap_or(request.start_version);
                    let key_report = key_writer.finish(&request, last_success_version)?;
                    write_record_checkpoint(
                        &request,
                        "partial",
                        "recorded partial Aptos Transaction Stream raw protobuf chunks before stream interruption",
                        chain_id,
                        count,
                        uncompressed_raw_bytes,
                        last_success_version,
                        &chunks,
                        Some(key_report),
                        stopped_at_byte_limit,
                        Some(stream_error.as_str()),
                    )?;
                    return Ok(RecordRunOutcome::Interrupted {
                        next_start_version: checked_next_version(last_success_version)?,
                        raw_bytes: uncompressed_raw_bytes,
                        stream_error,
                    });
                }
                return Err(HotIndexError::Config(stream_error));
            }
        };

        chain_id = response.chain_id.or(chain_id);
        for transaction in response.transactions {
            if request
                .max_raw_bytes
                .map(|limit| uncompressed_raw_bytes >= limit)
                .unwrap_or(false)
            {
                stopped_at_byte_limit = true;
                break 'stream;
            }

            if active_chunk.is_none() {
                active_chunk = Some(ActiveTransactionChunkWriter::create(
                    &request.out_dir,
                    transaction.version,
                )?);
            }
            let writer = active_chunk.as_mut().ok_or_else(|| {
                HotIndexError::Config("transaction chunk writer was not initialized".to_string())
            })?;
            let bytes_written = writer.write_transaction(&transaction)?;
            uncompressed_raw_bytes = uncompressed_raw_bytes
                .checked_add(bytes_written)
                .ok_or_else(|| {
                    HotIndexError::Config("recorded raw byte count overflowed u64".to_string())
                })?;
            last_version = Some(transaction.version);
            key_writer.write_version(transaction.version, count)?;
            count += 1;

            if writer.transaction_count() >= request.chunk_transaction_count {
                finish_active_chunk(&request, &mut active_chunk, &mut chunks)?;
                let last_success_version = last_version.unwrap_or(request.start_version);
                write_record_checkpoint(
                    &request,
                    "recording",
                    "recorded Aptos Transaction Stream raw protobuf chunk; recording still in progress",
                    chain_id,
                    count,
                    uncompressed_raw_bytes,
                    last_success_version,
                    &chunks,
                    None,
                    stopped_at_byte_limit,
                    None,
                )?;
            }

            if request
                .max_raw_bytes
                .map(|limit| uncompressed_raw_bytes >= limit)
                .unwrap_or(false)
            {
                stopped_at_byte_limit = true;
                break 'stream;
            }
        }
    }
    finish_active_chunk(&request, &mut active_chunk, &mut chunks)?;

    if count == 0 {
        return Err(HotIndexError::Config(
            "Transaction Stream returned zero transactions".to_string(),
        ));
    }
    let last_success_version = last_version.unwrap_or(request.start_version);
    let key_report = key_writer.finish(&request, last_success_version)?;
    let message = if stopped_at_byte_limit {
        "recorded Aptos Transaction Stream raw protobuf chunks; stopped at max raw byte limit"
    } else {
        "recorded Aptos Transaction Stream raw protobuf chunks"
    };
    write_record_checkpoint(
        &request,
        "complete",
        message,
        chain_id,
        count,
        uncompressed_raw_bytes,
        last_success_version,
        &chunks,
        Some(key_report),
        stopped_at_byte_limit,
        None,
    )?;
    println!(
        "recorded transaction stream: tx={} range={}..{} raw_bytes={} next_start_version={}",
        count,
        request.start_version,
        last_success_version,
        uncompressed_raw_bytes,
        checked_next_version(last_success_version)?
    );
    Ok(RecordRunOutcome::Complete {
        next_start_version: checked_next_version(last_success_version)?,
        raw_bytes: uncompressed_raw_bytes,
        stopped_at_byte_limit,
    })
}

fn finish_active_chunk(
    request: &LiveRecordRequest,
    active_chunk: &mut Option<ActiveTransactionChunkWriter>,
    chunks: &mut Vec<serde_json::Value>,
) -> Result<()> {
    let Some(chunk) = active_chunk.take() else {
        return Ok(());
    };
    chunks.push(chunk.finish(&request.out_dir)?);
    Ok(())
}

struct ActiveTransactionChunkWriter {
    first_version: u64,
    last_version: Option<u64>,
    transaction_count: u64,
    tmp_path: PathBuf,
    writer: TransactionChunkWriter,
}

impl ActiveTransactionChunkWriter {
    fn create(out_dir: &Path, first_version: u64) -> Result<Self> {
        let tmp_path = out_dir.join(format!(
            "transactions_{}_{}.pb.zst.tmp",
            first_version,
            std::process::id()
        ));
        let writer = TransactionChunkWriter::create(&tmp_path)?;
        Ok(Self {
            first_version,
            last_version: None,
            transaction_count: 0,
            tmp_path,
            writer,
        })
    }

    fn write_transaction(&mut self, transaction: &Transaction) -> Result<u64> {
        let bytes_written = self.writer.write_transaction(transaction)?;
        self.last_version = Some(transaction.version);
        self.transaction_count += 1;
        Ok(bytes_written)
    }

    fn transaction_count(&self) -> u64 {
        self.transaction_count
    }

    fn finish(self, out_dir: &Path) -> Result<serde_json::Value> {
        let last_version = self.last_version.ok_or_else(|| {
            HotIndexError::Config("cannot finish empty transaction chunk".to_string())
        })?;
        let final_path = out_dir.join(format!(
            "transactions_{}_{}.pb.zst",
            self.first_version, last_version
        ));
        if final_path.exists() {
            return Err(HotIndexError::Config(format!(
                "refusing to overwrite existing raw chunk {}",
                final_path.display()
            )));
        }

        self.writer.finish()?;
        fs::rename(&self.tmp_path, &final_path)?;
        let chunk_sha256 = sha256_file(&final_path)?;
        let chunk_name = final_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("transactions.pb.zst")
            .to_string();
        Ok(serde_json::json!({
            "path": chunk_name,
            "first_version": self.first_version,
            "last_version": last_version,
            "transaction_count": self.transaction_count,
            "sha256": chunk_sha256
        }))
    }
}

fn write_record_checkpoint(
    request: &LiveRecordRequest,
    status: &str,
    message: &str,
    chain_id: Option<u64>,
    run_transaction_count: u64,
    uncompressed_raw_bytes: u64,
    last_success_version: u64,
    chunks: &[serde_json::Value],
    key_report: Option<serde_json::Value>,
    stopped_at_byte_limit: bool,
    stream_error: Option<&str>,
) -> Result<()> {
    let total_transaction_count = chunks.iter().try_fold(0_u64, |acc, chunk| {
        acc.checked_add(chunk_transaction_count(chunk))
            .ok_or_else(|| {
                HotIndexError::Config("checkpoint transaction count overflow".to_string())
            })
    })?;
    let mut checkpoint = serde_json::json!({
        "status": status,
        "message": message,
        "network": request.network.as_str(),
        "endpoint": request.endpoint.as_str(),
        "package_address": request.package_address.as_str(),
        "orderbook_address": request.orderbook_address.as_str(),
        "start_version": request.start_version,
        "end_version": request.end_version,
        "requested_start_version": request.requested_start_version,
        "requested_end_version": request.requested_end_version,
        "transactions_count": request.transactions_count,
        "resume": request.resume,
        "raw_format": request.raw_format.as_str(),
        "auth_token_present": true,
        "chain_id": chain_id,
        "run_transaction_count": run_transaction_count,
        "transaction_count": total_transaction_count,
        "uncompressed_raw_bytes": uncompressed_raw_bytes,
        "max_raw_bytes": request.max_raw_bytes,
        "chunk_transaction_count": request.chunk_transaction_count,
        "last_success_version": last_success_version,
        "next_start_version": checked_next_version(last_success_version)?,
        "stopped_at_byte_limit": stopped_at_byte_limit,
        "chunks": chunks,
    });

    if let serde_json::Value::Object(object) = &mut checkpoint {
        if let Some(key_report) = key_report {
            object.insert("key_files".to_string(), key_report);
        }
        if let Some(stream_error) = stream_error {
            object.insert(
                "stream_error".to_string(),
                serde_json::Value::String(stream_error.to_string()),
            );
        }
    }

    write_json_pretty(&request.out_dir.join("record_checkpoint.json"), &checkpoint)
}

fn chunk_transaction_count(chunk: &serde_json::Value) -> u64 {
    chunk
        .get("transaction_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

fn checked_next_version(version: u64) -> Result<u64> {
    version
        .checked_add(1)
        .ok_or_else(|| HotIndexError::Config("last_success_version cannot advance".to_string()))
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

    fn write_transaction(&mut self, transaction: &Transaction) -> Result<u64> {
        let Some(encoder) = self.encoder.as_mut() else {
            return Err(HotIndexError::Config(
                "transaction chunk writer is already finished".to_string(),
            ));
        };
        let mut buffer = Vec::new();
        transaction
            .encode_length_delimited(&mut buffer)
            .map_err(|error| HotIndexError::Parse(error.to_string()))?;
        let bytes_written = buffer.len().try_into().map_err(|_| {
            HotIndexError::Config("encoded transaction length overflowed u64".to_string())
        })?;
        encoder.write_all(&buffer)?;
        Ok(bytes_written)
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

fn read_next_len_delimited_transaction<R: Read>(reader: &mut R) -> Result<Option<Transaction>> {
    let Some(len) = read_varint_len(reader)? else {
        return Ok(None);
    };
    let len: usize = len
        .try_into()
        .map_err(|_| HotIndexError::Parse(format!("protobuf message too large: {len}")))?;
    let mut buffer = vec![0_u8; len];
    reader.read_exact(&mut buffer)?;
    Transaction::decode(buffer.as_slice())
        .map(Some)
        .map_err(|error| HotIndexError::Parse(error.to_string()))
}

fn read_varint_len<R: Read>(reader: &mut R) -> Result<Option<u64>> {
    let mut len = 0_u64;
    for idx in 0..10 {
        let mut byte = [0_u8; 1];
        match reader.read_exact(&mut byte) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::UnexpectedEof && idx == 0 => return Ok(None),
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => {
                return Err(HotIndexError::Parse(
                    "truncated protobuf length prefix".to_string(),
                ));
            }
            Err(error) => return Err(error.into()),
        }
        len |= u64::from(byte[0] & 0x7f) << (idx * 7);
        if byte[0] & 0x80 == 0 {
            return Ok(Some(len));
        }
    }
    Err(HotIndexError::Parse(
        "protobuf length prefix exceeds 10 bytes".to_string(),
    ))
}

struct RecordKeyWriter {
    tx_versions_tmp: PathBuf,
    keys_dir: PathBuf,
    tx_versions: BufWriter<File>,
    sampled_versions: Vec<u64>,
    sample_stride: u64,
    sample_limit: u64,
}

impl RecordKeyWriter {
    fn create(request: &LiveRecordRequest, expected_count: u64) -> Result<Self> {
        let keys_dir = request.out_dir.join("keys");
        fs::create_dir_all(&keys_dir)?;
        let tx_versions_tmp = keys_dir.join(format!(
            "tx_versions_{}_{}.u64be.tmp",
            request.start_version,
            std::process::id()
        ));
        let tx_versions = BufWriter::new(File::create(&tx_versions_tmp)?);
        let sample_limit = request.key_sample_limit.max(1);
        let sample_stride = expected_count.div_ceil(sample_limit).max(1);
        Ok(Self {
            tx_versions_tmp,
            keys_dir,
            tx_versions,
            sampled_versions: Vec::new(),
            sample_stride,
            sample_limit,
        })
    }

    fn write_version(&mut self, version: u64, zero_based_idx: u64) -> Result<()> {
        self.tx_versions.write_all(&version.to_be_bytes())?;
        if zero_based_idx % self.sample_stride == 0
            && self.sampled_versions.len() < self.sample_limit as usize
        {
            self.sampled_versions.push(version);
        }
        Ok(())
    }

    fn finish(
        mut self,
        request: &LiveRecordRequest,
        last_success_version: u64,
    ) -> Result<serde_json::Value> {
        self.tx_versions.flush()?;
        self.tx_versions.get_ref().sync_all()?;
        drop(self.tx_versions);
        let tx_versions_final = self.keys_dir.join(format!(
            "tx_versions_{}_{}.u64be",
            request.start_version, last_success_version
        ));
        if tx_versions_final.exists() {
            return Err(HotIndexError::Config(format!(
                "refusing to overwrite existing key file {}",
                tx_versions_final.display()
            )));
        }
        fs::rename(&self.tx_versions_tmp, &tx_versions_final)?;

        let dataset_root = dataset_root_for_raw_dir(&request.out_dir)?;
        let queries_dir = dataset_root.join("queries");
        fs::create_dir_all(&queries_dir)?;

        let sample_file = request.out_dir.join("keys").join(format!(
            "tx_versions_sample_{}_{}.ndjson",
            request.start_version, last_success_version
        ));
        write_version_sample_file(&sample_file, &self.sampled_versions)?;

        let point_file = queries_dir.join("point_tx_versions.ndjson");
        write_point_tx_query_file(&point_file, &self.sampled_versions)?;

        let multi_file = queries_dir.join("multi_get_tx_versions.ndjson");
        write_multi_get_tx_query_file(&multi_file, &self.sampled_versions, 100)?;

        let manifest_path = queries_dir.join("record_keys_manifest.json");
        let key_manifest = serde_json::json!({
            "source": "aptos_transaction_stream_record",
            "start_version": request.start_version,
            "last_success_version": last_success_version,
            "tx_versions_file": artifact_key(&dataset_root, &tx_versions_final),
            "tx_versions_sha256": sha256_file(&tx_versions_final)?,
            "sample_stride": self.sample_stride,
            "sample_limit": self.sample_limit,
            "sample_count": self.sampled_versions.len(),
            "sample_file": artifact_key(&dataset_root, &sample_file),
            "sample_sha256": sha256_file(&sample_file)?,
            "point_query_file": artifact_key(&dataset_root, &point_file),
            "point_query_sha256": sha256_file(&point_file)?,
            "multi_get_query_file": artifact_key(&dataset_root, &multi_file),
            "multi_get_query_sha256": sha256_file(&multi_file)?,
        });
        write_json_pretty(&manifest_path, &key_manifest)?;
        Ok(key_manifest)
    }
}

fn write_version_sample_file(path: &Path, versions: &[u64]) -> Result<()> {
    atomic_write(path, |writer| {
        for version in versions {
            serde_json::to_writer(&mut *writer, &serde_json::json!({ "version": version }))
                .map_err(json_error)?;
            writer.write_all(b"\n")?;
        }
        Ok(())
    })
}

fn write_point_tx_query_file(path: &Path, versions: &[u64]) -> Result<()> {
    atomic_write(path, |writer| {
        for version in versions {
            let record = QueryCorpusRecord {
                query_kind: QueryKind::GetTxByVersion,
                tx_version: Some(*version),
                tx_versions: Vec::new(),
                market_id: None,
                account: None,
                builder_addr: None,
                limit: None,
            };
            serde_json::to_writer(&mut *writer, &record).map_err(json_error)?;
            writer.write_all(b"\n")?;
        }
        Ok(())
    })
}

fn write_multi_get_tx_query_file(path: &Path, versions: &[u64], batch_size: usize) -> Result<()> {
    atomic_write(path, |writer| {
        for chunk in versions
            .chunks(batch_size)
            .filter(|chunk| !chunk.is_empty())
        {
            let record = QueryCorpusRecord {
                query_kind: QueryKind::MultiGetTxVersions,
                tx_version: None,
                tx_versions: chunk.to_vec(),
                market_id: None,
                account: None,
                builder_addr: None,
                limit: None,
            };
            serde_json::to_writer(&mut *writer, &record).map_err(json_error)?;
            writer.write_all(b"\n")?;
        }
        Ok(())
    })
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
    write_text(&normalized.join("parse_warnings.log"), "")?;

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

    replay_ndjson_rows(&normalized.join("txs.ndjson"), |tx: TxRow| {
        engine.put_tx(tx)
    })?;
    replay_ndjson_rows(
        &normalized.join("events.ndjson"),
        |event: NormalizedEvent| engine.put_event(event),
    )?;
    replay_ndjson_rows(&normalized.join("fills.ndjson"), |fill: FillRow| {
        engine.put_fill(fill)
    })?;
    replay_ndjson_rows(&normalized.join("orders.ndjson"), |order: OrderRow| {
        engine.put_order(order)
    })?;
    replay_ndjson_rows(
        &normalized.join("positions.ndjson"),
        |position: PositionRow| engine.put_position(position),
    )?;
    replay_ndjson_rows(
        &normalized.join("builder_code_rows.ndjson"),
        |row: BuilderAttributionRow| engine.put_builder_attribution(row),
    )?;
    replay_ndjson_rows(
        &normalized.join("activity_rows.ndjson"),
        |row: ActivityRow| engine.put_activity(row),
    )?;

    engine.put_ingest_checkpoint(IngestCheckpoint {
        network: manifest.network,
        package_address: manifest.package_address,
        dataset_id: Some(manifest.dataset_id),
        last_processed_version: manifest.end_version.unwrap_or(manifest.start_version),
        last_processed_timestamp_us: dataset_last_processed_timestamp_us(root)?,
        events_indexed: manifest.decibel_event_count,
        fills_indexed: manifest.fill_count,
        builder_attributions_indexed: manifest.builder_code_row_count,
    })?;

    Ok(())
}

fn dataset_last_processed_timestamp_us(root: &Path) -> Result<u64> {
    let normalized = root.join("normalized");
    let mut max_ts = 0_u64;

    for row in read_ndjson::<TxRow>(&normalized.join("txs.ndjson"))? {
        max_ts = max_ts.max(row.block_timestamp_us);
    }
    for row in read_ndjson::<NormalizedEvent>(&normalized.join("events.ndjson"))? {
        max_ts = max_ts.max(row.block_timestamp_us);
    }
    for row in read_ndjson::<FillRow>(&normalized.join("fills.ndjson"))? {
        max_ts = max_ts.max(row.timestamp_us);
    }
    for row in read_ndjson::<OrderRow>(&normalized.join("orders.ndjson"))? {
        max_ts = max_ts.max(row.timestamp_us);
    }
    for row in read_ndjson::<PositionRow>(&normalized.join("positions.ndjson"))? {
        max_ts = max_ts.max(row.timestamp_us);
    }
    for row in read_ndjson::<BuilderAttributionRow>(&normalized.join("builder_code_rows.ndjson"))? {
        max_ts = max_ts.max(row.timestamp_us);
    }
    for row in read_ndjson::<ActivityRow>(&normalized.join("activity_rows.ndjson"))? {
        max_ts = max_ts.max(row.timestamp_us);
    }

    Ok(max_ts)
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
    write_text(
        &normalized_dir.join("parse_warnings.log"),
        &rows.warnings.join("\n"),
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

fn normalize_protobuf_tx_only(
    dataset_root: &Path,
    normalized_dir: &Path,
    raw_inputs: &[PathBuf],
    parser_options: &ParserOptions,
) -> Result<()> {
    if raw_inputs.is_empty() {
        return Err(HotIndexError::Config(
            "no protobuf raw inputs were provided".to_string(),
        ));
    }

    fs::create_dir_all(normalized_dir)?;
    let txs_path = normalized_dir.join("txs.ndjson");

    let mut first_version = None;
    let mut last_version = None;
    let mut tx_count = 0_u64;
    atomic_write(&txs_path, |tx_writer| {
        for raw_input in raw_inputs {
            let mut decoder = zstd::stream::read::Decoder::new(File::open(raw_input)?)?;
            while let Some(transaction) = read_next_len_delimited_transaction(&mut decoder)? {
                if let Some(previous_version) = last_version {
                    if transaction.version <= previous_version {
                        return Err(HotIndexError::Config(format!(
                            "raw chunks are not strictly increasing: {} has version {} after {}",
                            raw_input.display(),
                            transaction.version,
                            previous_version
                        )));
                    }
                }
                first_version.get_or_insert(transaction.version);
                last_version = Some(transaction.version);
                let row = tx_row_from_transaction(&transaction, parser_options);
                serde_json::to_writer(&mut *tx_writer, &row).map_err(json_error)?;
                tx_writer.write_all(b"\n")?;
                tx_count += 1;
            }
        }
        Ok(())
    })?;

    write_ndjson::<NormalizedEvent>(&normalized_dir.join("events.ndjson"), &[])?;
    write_ndjson::<FillRow>(&normalized_dir.join("fills.ndjson"), &[])?;
    write_ndjson::<OrderRow>(&normalized_dir.join("orders.ndjson"), &[])?;
    write_ndjson::<PositionRow>(&normalized_dir.join("positions.ndjson"), &[])?;
    write_ndjson::<BuilderAttributionRow>(&normalized_dir.join("builder_code_rows.ndjson"), &[])?;
    write_ndjson::<ActivityRow>(&normalized_dir.join("activity_rows.ndjson"), &[])?;
    write_ndjson::<NormalizedEvent>(&normalized_dir.join("unknown_events.ndjson"), &[])?;
    write_text(
        &normalized_dir.join("parse_warnings.log"),
        "tx-only protobuf normalization: Decibel event extraction is pending\n",
    )?;

    let mut hashes = BTreeMap::new();
    for raw_input in raw_inputs {
        insert_hash(&mut hashes, dataset_root, raw_input)?;
    }
    for relative in NORMALIZED_ARTIFACTS {
        insert_hash(&mut hashes, dataset_root, &dataset_root.join(relative))?;
    }
    for relative in QUERY_CORPUS_ARTIFACTS {
        let path = dataset_root.join(relative);
        if path.exists() {
            insert_hash(&mut hashes, dataset_root, &path)?;
        }
    }

    let manifest = DatasetManifest {
        dataset_id: parser_options.dataset_id.clone(),
        network: parser_options.network,
        source: "aptos_transaction_stream".to_string(),
        transaction_stream_endpoint: None,
        raw_encoding: DatasetEncoding::AptosTransactionProtobufLenDelimitedZstd,
        normalized_encoding: DatasetEncoding::Ndjson,
        start_version: first_version.unwrap_or_default(),
        end_version: last_version,
        package_address: parser_options.package_address.clone(),
        orderbook_address: parser_options.orderbook_address.clone(),
        parser_source: Some("decibel-dataset tx-only protobuf normalizer".to_string()),
        parser_commit: parser_options.parser_commit.clone(),
        captured_at: Some(current_epoch_string()),
        raw_transaction_count: tx_count,
        decibel_event_count: 0,
        fill_count: 0,
        order_count: 0,
        position_count: 0,
        builder_code_row_count: 0,
        hashes: DatasetFileHashes { sha256: hashes },
    };
    write_json_pretty(&dataset_root.join("manifest.json"), &manifest)
}

fn tx_row_from_transaction(transaction: &Transaction, parser_options: &ParserOptions) -> TxRow {
    TxRow {
        network: parser_options.network,
        version: transaction.version,
        tx_hash: transaction_hash_hex(transaction),
        block_timestamp_us: transaction_timestamp_us(transaction),
        event_count: transaction_event_count(transaction),
        dataset_id: Some(parser_options.dataset_id.clone()),
        raw_summary: Some(format!(
            "aptos_transaction_type={} epoch={} block_height={}",
            transaction.r#type, transaction.epoch, transaction.block_height
        )),
    }
}

fn transaction_hash_hex(transaction: &Transaction) -> String {
    transaction
        .info
        .as_ref()
        .filter(|info| !info.hash.is_empty())
        .map(|info| format!("0x{}", bytes_to_hex(&info.hash)))
        .unwrap_or_else(|| format!("0x{:064x}", transaction.version))
}

fn transaction_timestamp_us(transaction: &Transaction) -> u64 {
    let Some(timestamp) = transaction.timestamp else {
        return 0;
    };
    if timestamp.seconds < 0 || timestamp.nanos < 0 {
        return 0;
    }
    timestamp.seconds as u64 * 1_000_000 + timestamp.nanos as u64 / 1_000
}

fn transaction_event_count(transaction: &Transaction) -> u32 {
    use aptos_protos::transaction::v1::transaction::TxnData;

    let count = match transaction.txn_data.as_ref() {
        Some(TxnData::BlockMetadata(txn)) => txn.events.len(),
        Some(TxnData::Genesis(txn)) => txn.events.len(),
        Some(TxnData::User(txn)) => txn.events.len(),
        Some(TxnData::Validator(txn)) => txn.events.len(),
        Some(TxnData::StateCheckpoint(_)) | Some(TxnData::BlockEpilogue(_)) | None => 0,
    };
    count.try_into().unwrap_or(u32::MAX)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
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

fn resolve_protobuf_raw_inputs(input: &Path) -> Result<Vec<PathBuf>> {
    if input.is_file() {
        return Ok(vec![input.to_path_buf()]);
    }
    if !input.is_dir() {
        return Err(HotIndexError::Config(format!(
            "raw input {} is neither a file nor a directory",
            input.display()
        )));
    }

    let mut chunks = Vec::new();
    for entry in fs::read_dir(input)? {
        let path = entry?.path();
        if is_transaction_chunk(&path) {
            chunks.push(path);
        }
    }
    chunks.sort_by(|left, right| {
        raw_chunk_start_version(left)
            .cmp(&raw_chunk_start_version(right))
            .then_with(|| left.file_name().cmp(&right.file_name()))
    });

    if chunks.is_empty() {
        return Err(HotIndexError::Config(format!(
            "no transactions_*.pb.zst chunks found in {}",
            input.display()
        )));
    }
    Ok(chunks)
}

fn is_transaction_chunk(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.starts_with("transactions_") && name.ends_with(".pb.zst")
}

fn raw_chunk_start_version(path: &Path) -> Option<u64> {
    raw_chunk_version_range(path).map(|(start_version, _)| start_version)
}

fn raw_chunk_version_range(path: &Path) -> Option<(u64, u64)> {
    let name = path.file_name()?.to_str()?;
    let range = name
        .strip_prefix("transactions_")?
        .strip_suffix(".pb.zst")?;
    let mut parts = range.split('_');
    let first_version = parts.next()?.parse().ok()?;
    let last_version = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((first_version, last_version))
}

fn is_transaction_tmp_chunk(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.starts_with("transactions_") && name.ends_with(".pb.zst.tmp")
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

fn dataset_root_for_raw_dir(raw_dir: &Path) -> Result<PathBuf> {
    if raw_dir.file_name().and_then(|name| name.to_str()) == Some("raw") {
        raw_dir.parent().map(Path::to_path_buf).ok_or_else(|| {
            HotIndexError::Config(format!(
                "raw output {} has no dataset root parent",
                raw_dir.display()
            ))
        })
    } else {
        Ok(raw_dir.to_path_buf())
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

    fn optional_bytes(&self, name: &str) -> Result<Option<u64>> {
        self.optional_value(name)
            .map(|value| parse_byte_size(value, name))
            .transpose()
    }
}

fn parse_byte_size(value: &str, name: &str) -> Result<u64> {
    let value = value.trim();
    if value.is_empty() {
        return Err(HotIndexError::Config(format!(
            "invalid byte size for {name}: empty value"
        )));
    }

    let split_at = value
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(value.len());
    let (number, suffix) = value.split_at(split_at);
    if number.is_empty() {
        return Err(HotIndexError::Config(format!(
            "invalid byte size for {name}: {value}"
        )));
    }
    let units = number
        .parse::<u64>()
        .map_err(|_| HotIndexError::Config(format!("invalid byte size for {name}: {value}")))?;
    let multiplier = match suffix.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024_u64.pow(2),
        "g" | "gb" | "gib" => 1024_u64.pow(3),
        "t" | "tb" | "tib" => 1024_u64.pow(4),
        other => {
            return Err(HotIndexError::Config(format!(
                "unsupported byte size suffix for {name}: {other}"
            )));
        }
    };

    units.checked_mul(multiplier).ok_or_else(|| {
        HotIndexError::Config(format!("byte size for {name} overflows u64: {value}"))
    })
}

fn write_ndjson<T: Serialize>(path: &Path, rows: &[T]) -> Result<()> {
    atomic_write(path, |writer| {
        for row in rows {
            serde_json::to_writer(&mut *writer, row).map_err(json_error)?;
            writer.write_all(b"\n")?;
        }
        Ok(())
    })
}

fn write_jsonl_values(path: &Path, rows: &[serde_json::Value]) -> Result<()> {
    atomic_write(path, |writer| {
        for row in rows {
            serde_json::to_writer(&mut *writer, row).map_err(json_error)?;
            writer.write_all(b"\n")?;
        }
        Ok(())
    })
}

fn write_text(path: &Path, value: &str) -> Result<()> {
    atomic_write(path, |writer| {
        writer.write_all(value.as_bytes())?;
        Ok(())
    })
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

fn replay_ndjson_rows<T, F>(path: &Path, mut put: F) -> Result<()>
where
    T: DeserializeOwned,
    F: FnMut(T) -> Result<()>,
{
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let row = serde_json::from_str(&line).map_err(|error| {
            HotIndexError::Parse(format!("{}:{}: {error}", path.display(), idx + 1))
        })?;
        put(row)?;
    }
    Ok(())
}

fn write_json_pretty<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    atomic_write(path, |writer| {
        serde_json::to_writer_pretty(&mut *writer, value).map_err(json_error)
    })
}

fn atomic_write<F>(path: &Path, write: F) -> Result<()>
where
    F: FnOnce(&mut BufWriter<File>) -> Result<()>,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = tmp_path_for(path)?;
    let file = File::create(&tmp_path)?;
    let mut writer = BufWriter::new(file);
    write(&mut writer)?;
    writer.flush()?;
    let file = writer
        .into_inner()
        .map_err(|error| HotIndexError::Storage(error.to_string()))?;
    file.sync_all()?;
    fs::rename(&tmp_path, path)?;
    if let Some(parent) = path.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

fn tmp_path_for(path: &Path) -> Result<PathBuf> {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Err(HotIndexError::Config(format!(
            "cannot build temp path for {}",
            path.display()
        )));
    };
    Ok(path.with_file_name(format!("{file_name}.tmp")))
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

fn current_epoch_string() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => format!("unix_epoch_seconds:{}", duration.as_secs()),
        Err(_) => "unix_epoch_seconds:0".to_string(),
    }
}

fn print_usage() {
    eprintln!(
        "usage:
  decibel-dataset synthetic --out <dataset-dir> [--events <n>]
  decibel-dataset fixture --out <dataset-dir> [--events <n>] [--config <config.yaml>]
  decibel-dataset build-query-corpus --events <events.ndjson> --out-dir <queries-dir> [--seed <n>]
  decibel-dataset replay --dataset <dataset-dir> [--engine memory]
  decibel-dataset normalize --input <raw.pb.zst|raw-dir|fixture.jsonl> --out-dir <normalized-dir> [--format fixture-jsonl|protobuf-zstd] [--config <config.yaml>]
  decibel-dataset inspect-raw --input <transactions.pb.zst> [--limit <n>] [--allow-truncated]
  decibel-dataset record --live --network mainnet --endpoint <url> [--auth-token <token>|--auth-token-env <env>] [--resume] [--allow-low-mainnet-start] [--batch-size <n>] [--chunk-transaction-count <n>] [--max-raw-bytes <10GiB>] [--max-stream-retries <n>] [--retry-backoff-ms <n>] [--key-sample-limit <n>] [--package-address <addr>] [--orderbook-address <addr>] (--start-version <n> (--end-version <n>|--transactions-count <n>)|--resume --end-version <n>) --out-dir <raw-dir> --raw-format protobuf-zstd"
    );
}

#[cfg(test)]
mod tests {
    use super::{
        build_query_corpus, build_query_corpus_command, fixture_command, normalize_command,
        parse_byte_size, read_json, record_command, replay_into_memory, resolve_record_end_version,
        write_json_pretty, write_synthetic_dataset, DatasetManifest, HotIndexError,
        NormalizedEvent, SyntheticDataset, NORMALIZED_ARTIFACTS, QUERY_CORPUS_ARTIFACTS,
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
    fn protobuf_raw_directory_normalizes_all_chunks() {
        let root = temp_root("protobuf-raw-dir");
        let _ = std::fs::remove_dir_all(&root);
        let raw_dir = root.join("raw");
        std::fs::create_dir_all(&raw_dir).unwrap();
        write_test_transaction_chunk(&raw_dir.join("transactions_100_101.pb.zst"), 100..=101);
        write_test_transaction_chunk(&raw_dir.join("transactions_102_103.pb.zst"), 102..=103);

        normalize_command(&[
            "--input".to_string(),
            raw_dir.display().to_string(),
            "--out-dir".to_string(),
            root.join("normalized").display().to_string(),
            "--format".to_string(),
            "protobuf-zstd".to_string(),
            "--dataset-id".to_string(),
            "protobuf_raw_dir".to_string(),
            "--parser-commit".to_string(),
            "test-commit".to_string(),
        ])
        .unwrap();

        let manifest = read_json::<DatasetManifest>(&root.join("manifest.json")).unwrap();
        assert_eq!(manifest.dataset_id.0, "protobuf_raw_dir");
        assert_eq!(manifest.start_version, 100);
        assert_eq!(manifest.end_version, Some(103));
        assert_eq!(manifest.raw_transaction_count, 4);
        assert_eq!(manifest.decibel_event_count, 0);
        assert!(manifest
            .hashes
            .sha256
            .contains_key("raw/transactions_100_101.pb.zst"));
        assert!(manifest
            .hashes
            .sha256
            .contains_key("raw/transactions_102_103.pb.zst"));

        let engine = replay_into_memory(&root).unwrap();
        let stats = engine.stats().unwrap();
        assert_eq!(stats.tx_count, 4);
        assert_eq!(stats.event_count, 0);

        std::fs::remove_dir_all(root).unwrap();
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
        let env_name = "DECIBEL_DATASET_TEST_AUTH_TOKEN_ENV_NAME";
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

    #[test]
    fn record_resume_uses_checkpoint_cursor() {
        let root = temp_root("record-resume");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        write_json_pretty(
            &root.join("record_checkpoint.json"),
            &serde_json::json!({
                "status": "complete",
                "last_success_version": 4365622792_u64,
                "next_start_version": 4365622793_u64,
                "chunks": [{
                    "path": "transactions_4365621793_4365622792.pb.zst",
                    "first_version": 4365621793_u64,
                    "last_version": 4365622792_u64,
                    "transaction_count": 1000_u64,
                    "sha256": "abc"
                }]
            }),
        )
        .unwrap();

        record_command(&[
            "--network".to_string(),
            "mainnet".to_string(),
            "--resume".to_string(),
            "--transactions-count".to_string(),
            "10".to_string(),
            "--out-dir".to_string(),
            root.display().to_string(),
            "--raw-format".to_string(),
            "protobuf-zstd".to_string(),
        ])
        .unwrap();

        let value = read_json::<serde_json::Value>(&root.join("record_checkpoint.json")).unwrap();
        assert_eq!(value["start_version"], 4365622793_u64);
        assert_eq!(value["end_version"], 4365622802_u64);
        assert_eq!(value["next_start_version"], 4365622793_u64);
        assert_eq!(value["transaction_count"], 1000_u64);
        assert_eq!(value["chunks"].as_array().unwrap().len(), 1);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn record_resume_infers_cursor_from_completed_chunk_name() {
        let root = temp_root("record-resume-chunk");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("transactions_4365621793_4365622792.pb.zst"), "").unwrap();

        record_command(&[
            "--network".to_string(),
            "mainnet".to_string(),
            "--resume".to_string(),
            "--transactions-count".to_string(),
            "10".to_string(),
            "--out-dir".to_string(),
            root.display().to_string(),
            "--raw-format".to_string(),
            "protobuf-zstd".to_string(),
        ])
        .unwrap();

        let value = read_json::<serde_json::Value>(&root.join("record_checkpoint.json")).unwrap();
        assert_eq!(value["start_version"], 4365622793_u64);
        assert_eq!(value["end_version"], 4365622802_u64);
        assert_eq!(value["transaction_count"], 1000_u64);
        assert_eq!(value["chunks"][0]["resume_inferred_from_filename"], true);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn record_resume_error_points_at_tmp_chunk_recovery() {
        let root = temp_root("record-resume-tmp");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("transactions_4365621793_4381375638.pb.zst.tmp"),
            "",
        )
        .unwrap();

        let err = record_command(&[
            "--network".to_string(),
            "mainnet".to_string(),
            "--resume".to_string(),
            "--transactions-count".to_string(),
            "10".to_string(),
            "--out-dir".to_string(),
            root.display().to_string(),
            "--raw-format".to_string(),
            "protobuf-zstd".to_string(),
        ])
        .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("inspect-raw"));
        assert!(message.contains("--allow-truncated"));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn live_record_rejects_suspicious_low_mainnet_start() {
        let root = temp_root("record-low-mainnet-start");
        let _ = std::fs::remove_dir_all(&root);
        std::env::set_var("DECIBEL_DATASET_TEST_AUTH_TOKEN", "token");

        let err = record_command(&[
            "--live".to_string(),
            "--network".to_string(),
            "mainnet".to_string(),
            "--endpoint".to_string(),
            "grpc.mainnet.aptoslabs.com:443".to_string(),
            "--auth-token-env".to_string(),
            "DECIBEL_DATASET_TEST_AUTH_TOKEN".to_string(),
            "--start-version".to_string(),
            "237432".to_string(),
            "--end-version".to_string(),
            "4381375638".to_string(),
            "--out-dir".to_string(),
            root.display().to_string(),
            "--raw-format".to_string(),
            "protobuf-zstd".to_string(),
        ])
        .unwrap_err();

        assert!(err.to_string().contains("suspicious mainnet record range"));

        std::env::remove_var("DECIBEL_DATASET_TEST_AUTH_TOKEN");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn record_end_version_can_be_derived_from_transaction_count() {
        assert_eq!(
            resolve_record_end_version(4365621793, None, Some(10)).unwrap(),
            Some(4365621802)
        );
        let err = resolve_record_end_version(10, Some(20), Some(1)).unwrap_err();
        assert!(err
            .to_string()
            .contains("--end-version and --transactions-count"));
    }

    #[test]
    fn byte_size_parser_accepts_gib_style_limits() {
        assert_eq!(
            parse_byte_size("10G", "--max-raw-bytes").unwrap(),
            10 * 1024_u64.pow(3)
        );
        assert_eq!(
            parse_byte_size("10240MiB", "--max-raw-bytes").unwrap(),
            10 * 1024_u64.pow(3)
        );
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("decibel-dataset-{name}-{}", std::process::id()))
    }

    fn write_test_transaction_chunk(
        path: &std::path::Path,
        versions: std::ops::RangeInclusive<u64>,
    ) {
        let mut writer = super::TransactionChunkWriter::create(path).unwrap();
        for version in versions {
            let mut transaction = aptos_protos::transaction::v1::Transaction::default();
            transaction.version = version;
            writer.write_transaction(&transaction).unwrap();
        }
        writer.finish().unwrap();
    }
}
