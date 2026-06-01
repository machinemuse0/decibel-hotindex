use decibel_hotindex_core::{
    ActivityRow, BuilderAttributionRow, CfChecksum, DatasetManifest, FillRow, HotIndexError,
    IngestCheckpoint, NormalizedEvent, OrderRow, PositionRow, QueryCorpusRecord, QueryKind, Result,
    TimeWindow, TxRow,
};
#[cfg(feature = "rocksdb")]
use decibel_hotindex_storage::RocksDbEngine;
#[cfg(feature = "toplingsdb")]
use decibel_hotindex_storage::ToplingDbEngine;
use decibel_hotindex_storage::{MemoryEngine, StorageEngine};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
        "run" => run_command(&args[1..]),
        "summarize" => summarize_command(&args[1..]),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        other => Err(HotIndexError::Config(format!("unknown command: {other}"))),
    }
}

fn run_command(args: &[String]) -> Result<()> {
    let opts = Args::new(args);
    let dataset = opts.required_path("--dataset")?;
    let backend = opts
        .optional_value("--engine")
        .unwrap_or("memory")
        .to_string();
    let bench_class = opts.optional_value("--class").unwrap_or("serving");
    let workload = opts
        .optional_value("--workload")
        .unwrap_or("mixed_market_dashboard");
    let iterations = opts.optional_usize("--iterations")?.unwrap_or(1_000);
    let warmup = opts.optional_usize("--warmup")?.unwrap_or(100);
    let out = opts
        .optional_value("--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("reports/bench-local.json"));
    let expected_checksum = opts
        .optional_value("--expected-checksum")
        .map(PathBuf::from);
    let manifest = read_json::<DatasetManifest>(&dataset.join("manifest.json"))?;
    let started_at = iso_like_now();

    let report = match bench_class {
        "serving" => run_serving_bench(
            &dataset,
            opts.optional_value("--db-path").map(PathBuf::from),
            &backend,
            workload,
            iterations,
            warmup,
            opts.optional_value("--access-pattern")
                .unwrap_or("sequential"),
            opts.optional_value("--seed")
                .unwrap_or("query-corpus-order"),
            opts.optional_value("--checksum-status")
                .unwrap_or("not_run"),
            expected_checksum.as_deref(),
            &manifest,
            &started_at,
        )?,
        "ingest" => run_ingest_bench(
            &dataset,
            opts.optional_value("--db-path").map(PathBuf::from),
            &backend,
            iterations,
            warmup,
            opts.optional_value("--checksum-status")
                .unwrap_or("not_run"),
            expected_checksum.as_deref(),
            &manifest,
            &started_at,
        )?,
        "read-under-ingest" | "read_under_ingest" => run_read_under_ingest_bench(
            &dataset,
            opts.optional_value("--db-path").map(PathBuf::from),
            &backend,
            workload,
            iterations,
            warmup,
            opts.optional_value("--access-pattern")
                .unwrap_or("sequential"),
            opts.optional_value("--seed")
                .unwrap_or("query-corpus-order"),
            opts.optional_value("--checksum-status")
                .unwrap_or("not_run"),
            expected_checksum.as_deref(),
            &manifest,
            &started_at,
        )?,
        other => {
            return Err(HotIndexError::Config(format!(
                "unsupported benchmark class: {other}"
            )));
        }
    };

    write_json_pretty(&out, &report)?;
    println!(
        "benchmark report written: class={} workload={} backend={} out={}",
        report.benchmark_class,
        report.workload,
        report.backend,
        out.display()
    );
    println!(
        "summary: ops={} errors={} throughput_qps={:.2} p50_us={} p95_us={} p99_us={} p999_us={}",
        report.result.operations,
        report.result.errors,
        report.result.throughput_qps,
        report.result.latency_us.p50,
        report.result.latency_us.p95,
        report.result.latency_us.p99,
        report.result.latency_us.p999
    );
    Ok(())
}

fn summarize_command(args: &[String]) -> Result<()> {
    let opts = Args::new(args);
    let reports = opts.required_value("--reports")?;
    let out = opts
        .optional_value("--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("reports/BENCHMARK_SUMMARY.md"));
    let mut parsed = Vec::new();
    for report_path in reports.split(',').filter(|value| !value.trim().is_empty()) {
        parsed.push(read_json::<BenchmarkReport>(Path::new(report_path.trim()))?);
    }
    if parsed.is_empty() {
        return Err(HotIndexError::Config(
            "summarize requires at least one report path".to_string(),
        ));
    }
    write_markdown_summary(&out, &parsed)?;
    println!(
        "benchmark summary written: reports={} out={}",
        parsed.len(),
        out.display()
    );
    Ok(())
}

fn run_serving_bench(
    dataset: &Path,
    db_path: Option<PathBuf>,
    backend: &str,
    workload: &str,
    iterations: usize,
    warmup: usize,
    access_pattern: &str,
    seed: &str,
    checksum_status: &str,
    expected_checksum: Option<&Path>,
    manifest: &DatasetManifest,
    started_at: &str,
) -> Result<BenchmarkReport> {
    ensure_serving_workload_supported(manifest, workload)?;
    let query_corpus = query_corpus_report(dataset, workload)?;
    let corpus = read_ndjson(&query_corpus.path)?;
    if corpus.is_empty() {
        return Err(empty_corpus_error(manifest, workload));
    }

    match backend {
        "memory" => {
            let engine = MemoryEngine::default();
            replay_into_engine(dataset, &engine)?;
            let result =
                measure_queries(&engine, &corpus, iterations, warmup, access_pattern, seed)?;
            build_report(
                manifest,
                dataset,
                backend,
                "serving",
                workload,
                iterations,
                warmup,
                access_pattern,
                seed,
                Some(query_corpus),
                started_at,
                result,
                engine.checksums().ok(),
                "pass",
                expected_checksum,
            )
        }
        "rocksdb" => run_rocksdb_serving(
            dataset,
            db_path,
            workload,
            iterations,
            warmup,
            access_pattern,
            seed,
            checksum_status,
            expected_checksum,
            manifest,
            started_at,
            query_corpus,
            corpus,
        ),
        "toplingdb" => run_toplingdb_serving(
            dataset,
            db_path,
            workload,
            iterations,
            warmup,
            access_pattern,
            seed,
            checksum_status,
            expected_checksum,
            manifest,
            started_at,
            query_corpus,
            corpus,
        ),
        other => Err(HotIndexError::Config(format!(
            "unsupported serving backend: {other}"
        ))),
    }
}

fn ensure_serving_workload_supported(manifest: &DatasetManifest, workload: &str) -> Result<()> {
    if manifest.decibel_event_count == 0 && requires_decibel_events(workload) {
        return Err(empty_corpus_error(manifest, workload));
    }
    Ok(())
}

fn requires_decibel_events(workload: &str) -> bool {
    !matches!(workload, "get_tx_by_version" | "multi_get_tx_versions_100")
}

fn empty_corpus_error(manifest: &DatasetManifest, workload: &str) -> HotIndexError {
    if manifest.decibel_event_count == 0 && requires_decibel_events(workload) {
        HotIndexError::Config(format!(
            "workload {workload} requires Decibel events, but this dataset has decibel_event_count=0; real Aptos protobuf normalization is currently tx-only, so use fixture/synthetic data or run tx point/multi-get workloads until event extraction lands"
        ))
    } else {
        HotIndexError::Config(format!("query corpus for workload {workload} is empty"))
    }
}

#[cfg(feature = "rocksdb")]
fn run_rocksdb_serving(
    dataset: &Path,
    db_path: Option<PathBuf>,
    workload: &str,
    iterations: usize,
    warmup: usize,
    access_pattern: &str,
    seed: &str,
    checksum_status: &str,
    expected_checksum: Option<&Path>,
    manifest: &DatasetManifest,
    started_at: &str,
    query_corpus: QueryCorpusReport,
    corpus: Vec<QueryCorpusRecord>,
) -> Result<BenchmarkReport> {
    let path = db_path.unwrap_or_else(|| dataset.join("materialized/rocksdb"));
    let engine = RocksDbEngine::open(&path)?;
    let result = measure_queries(&engine, &corpus, iterations, warmup, access_pattern, seed)?;
    build_report(
        manifest,
        dataset,
        "rocksdb",
        "serving",
        workload,
        iterations,
        warmup,
        access_pattern,
        seed,
        Some(query_corpus),
        started_at,
        result,
        engine.checksums().ok(),
        checksum_status,
        expected_checksum,
    )
}

#[cfg(not(feature = "rocksdb"))]
fn run_rocksdb_serving(
    _dataset: &Path,
    _db_path: Option<PathBuf>,
    _workload: &str,
    _iterations: usize,
    _warmup: usize,
    _access_pattern: &str,
    _seed: &str,
    _checksum_status: &str,
    _expected_checksum: Option<&Path>,
    _manifest: &DatasetManifest,
    _started_at: &str,
    _query_corpus: QueryCorpusReport,
    _corpus: Vec<QueryCorpusRecord>,
) -> Result<BenchmarkReport> {
    Err(HotIndexError::Config(
        "RocksDB benchmark requires `--features rocksdb`".to_string(),
    ))
}

#[cfg(feature = "toplingsdb")]
fn run_toplingdb_serving(
    dataset: &Path,
    db_path: Option<PathBuf>,
    workload: &str,
    iterations: usize,
    warmup: usize,
    access_pattern: &str,
    seed: &str,
    checksum_status: &str,
    expected_checksum: Option<&Path>,
    manifest: &DatasetManifest,
    started_at: &str,
    query_corpus: QueryCorpusReport,
    corpus: Vec<QueryCorpusRecord>,
) -> Result<BenchmarkReport> {
    let path = db_path.unwrap_or_else(|| dataset.join("materialized/toplingdb"));
    let engine = ToplingDbEngine::open(&path)?;
    let result = measure_queries(&engine, &corpus, iterations, warmup, access_pattern, seed)?;
    build_report(
        manifest,
        dataset,
        "toplingdb",
        "serving",
        workload,
        iterations,
        warmup,
        access_pattern,
        seed,
        Some(query_corpus),
        started_at,
        result,
        engine.checksums().ok(),
        checksum_status,
        expected_checksum,
    )
}

#[cfg(not(feature = "toplingsdb"))]
#[allow(clippy::too_many_arguments)]
fn run_toplingdb_serving(
    _dataset: &Path,
    _db_path: Option<PathBuf>,
    _workload: &str,
    _iterations: usize,
    _warmup: usize,
    _access_pattern: &str,
    _seed: &str,
    _checksum_status: &str,
    _expected_checksum: Option<&Path>,
    _manifest: &DatasetManifest,
    _started_at: &str,
    _query_corpus: QueryCorpusReport,
    _corpus: Vec<QueryCorpusRecord>,
) -> Result<BenchmarkReport> {
    Err(HotIndexError::Config(
        "ToplingDB benchmark requires `--features toplingsdb`".to_string(),
    ))
}

fn run_ingest_bench(
    dataset: &Path,
    db_path: Option<PathBuf>,
    backend: &str,
    iterations: usize,
    warmup: usize,
    checksum_status: &str,
    expected_checksum: Option<&Path>,
    manifest: &DatasetManifest,
    started_at: &str,
) -> Result<BenchmarkReport> {
    match backend {
        "memory" => {
            let rows = IngestRows::load(dataset)?;
            let engine = MemoryEngine::default();
            warmup_ingest(&engine, &rows, warmup)?;
            let result = measure_ingest(&engine, &rows, iterations)?;
            put_checkpoint_from_manifest(dataset, &engine)?;
            build_report(
                manifest,
                dataset,
                backend,
                "ingest",
                "normalized_replay",
                iterations,
                warmup,
                "sequential",
                "normalized-row-order",
                None,
                started_at,
                result,
                engine.checksums().ok(),
                "pass",
                expected_checksum,
            )
        }
        "rocksdb" => run_rocksdb_ingest(
            dataset,
            db_path,
            iterations,
            warmup,
            checksum_status,
            expected_checksum,
            manifest,
            started_at,
        ),
        "toplingdb" => run_toplingdb_ingest(
            dataset,
            db_path,
            iterations,
            warmup,
            checksum_status,
            expected_checksum,
            manifest,
            started_at,
        ),
        other => Err(HotIndexError::Config(format!(
            "unsupported ingest backend: {other}"
        ))),
    }
}

#[cfg(feature = "rocksdb")]
fn run_rocksdb_ingest(
    dataset: &Path,
    db_path: Option<PathBuf>,
    iterations: usize,
    warmup: usize,
    checksum_status: &str,
    expected_checksum: Option<&Path>,
    manifest: &DatasetManifest,
    started_at: &str,
) -> Result<BenchmarkReport> {
    let rows = IngestRows::load(dataset)?;
    let path = db_path.unwrap_or_else(|| dataset.join("materialized/rocksdb-bench-ingest"));
    let engine = RocksDbEngine::open(path)?;
    warmup_ingest(&engine, &rows, warmup)?;
    let result = measure_ingest(&engine, &rows, iterations)?;
    put_checkpoint_from_manifest(dataset, &engine)?;
    build_report(
        manifest,
        dataset,
        "rocksdb",
        "ingest",
        "normalized_replay",
        iterations,
        warmup,
        "sequential",
        "normalized-row-order",
        None,
        started_at,
        result,
        engine.checksums().ok(),
        checksum_status,
        expected_checksum,
    )
}

#[cfg(not(feature = "rocksdb"))]
fn run_rocksdb_ingest(
    _dataset: &Path,
    _db_path: Option<PathBuf>,
    _iterations: usize,
    _warmup: usize,
    _checksum_status: &str,
    _expected_checksum: Option<&Path>,
    _manifest: &DatasetManifest,
    _started_at: &str,
) -> Result<BenchmarkReport> {
    Err(HotIndexError::Config(
        "RocksDB benchmark requires `--features rocksdb`".to_string(),
    ))
}

#[cfg(feature = "toplingsdb")]
fn run_toplingdb_ingest(
    dataset: &Path,
    db_path: Option<PathBuf>,
    iterations: usize,
    warmup: usize,
    checksum_status: &str,
    expected_checksum: Option<&Path>,
    manifest: &DatasetManifest,
    started_at: &str,
) -> Result<BenchmarkReport> {
    let rows = IngestRows::load(dataset)?;
    let path = db_path.unwrap_or_else(|| dataset.join("materialized/toplingdb-bench-ingest"));
    let engine = ToplingDbEngine::open(path)?;
    warmup_ingest(&engine, &rows, warmup)?;
    let result = measure_ingest(&engine, &rows, iterations)?;
    put_checkpoint_from_manifest(dataset, &engine)?;
    build_report(
        manifest,
        dataset,
        "toplingdb",
        "ingest",
        "normalized_replay",
        iterations,
        warmup,
        "sequential",
        "normalized-row-order",
        None,
        started_at,
        result,
        engine.checksums().ok(),
        checksum_status,
        expected_checksum,
    )
}

#[cfg(not(feature = "toplingsdb"))]
fn run_toplingdb_ingest(
    _dataset: &Path,
    _db_path: Option<PathBuf>,
    _iterations: usize,
    _warmup: usize,
    _checksum_status: &str,
    _expected_checksum: Option<&Path>,
    _manifest: &DatasetManifest,
    _started_at: &str,
) -> Result<BenchmarkReport> {
    Err(HotIndexError::Config(
        "ToplingDB benchmark requires `--features toplingsdb`".to_string(),
    ))
}

fn run_read_under_ingest_bench(
    dataset: &Path,
    db_path: Option<PathBuf>,
    backend: &str,
    workload: &str,
    iterations: usize,
    warmup: usize,
    access_pattern: &str,
    seed: &str,
    checksum_status: &str,
    expected_checksum: Option<&Path>,
    manifest: &DatasetManifest,
    started_at: &str,
) -> Result<BenchmarkReport> {
    let query_corpus = query_corpus_report(dataset, workload)?;
    let corpus = read_ndjson(&query_corpus.path)?;
    let rows = IngestRows::load(dataset)?;
    match backend {
        "memory" => {
            let engine = MemoryEngine::default();
            let result = measure_read_under_ingest(
                &engine,
                &rows,
                &corpus,
                iterations,
                warmup,
                access_pattern,
                seed,
            )?;
            put_checkpoint_from_manifest(dataset, &engine)?;
            build_report(
                manifest,
                dataset,
                backend,
                "read-under-ingest",
                workload,
                iterations,
                warmup,
                access_pattern,
                seed,
                Some(query_corpus),
                started_at,
                result,
                engine.checksums().ok(),
                checksum_status,
                expected_checksum,
            )
        }
        "rocksdb" => run_rocksdb_read_under_ingest(
            dataset,
            db_path,
            workload,
            iterations,
            warmup,
            access_pattern,
            seed,
            checksum_status,
            expected_checksum,
            manifest,
            started_at,
            query_corpus,
            corpus,
            rows,
        ),
        "toplingdb" => run_toplingdb_read_under_ingest(
            dataset,
            db_path,
            workload,
            iterations,
            warmup,
            access_pattern,
            seed,
            checksum_status,
            expected_checksum,
            manifest,
            started_at,
            query_corpus,
            corpus,
            rows,
        ),
        other => Err(HotIndexError::Config(format!(
            "unsupported read-under-ingest backend: {other}"
        ))),
    }
}

#[cfg(feature = "toplingsdb")]
fn run_toplingdb_read_under_ingest(
    dataset: &Path,
    db_path: Option<PathBuf>,
    workload: &str,
    iterations: usize,
    warmup: usize,
    access_pattern: &str,
    seed: &str,
    checksum_status: &str,
    expected_checksum: Option<&Path>,
    manifest: &DatasetManifest,
    started_at: &str,
    query_corpus: QueryCorpusReport,
    corpus: Vec<QueryCorpusRecord>,
    rows: IngestRows,
) -> Result<BenchmarkReport> {
    let path = db_path.unwrap_or_else(|| dataset.join("materialized/toplingdb-bench-read-ingest"));
    let engine = ToplingDbEngine::open(path)?;
    let result = measure_read_under_ingest(
        &engine,
        &rows,
        &corpus,
        iterations,
        warmup,
        access_pattern,
        seed,
    )?;
    put_checkpoint_from_manifest(dataset, &engine)?;
    build_report(
        manifest,
        dataset,
        "toplingdb",
        "read-under-ingest",
        workload,
        iterations,
        warmup,
        access_pattern,
        seed,
        Some(query_corpus),
        started_at,
        result,
        engine.checksums().ok(),
        checksum_status,
        expected_checksum,
    )
}

#[cfg(not(feature = "toplingsdb"))]
#[allow(clippy::too_many_arguments)]
fn run_toplingdb_read_under_ingest(
    _dataset: &Path,
    _db_path: Option<PathBuf>,
    _workload: &str,
    _iterations: usize,
    _warmup: usize,
    _access_pattern: &str,
    _seed: &str,
    _checksum_status: &str,
    _expected_checksum: Option<&Path>,
    _manifest: &DatasetManifest,
    _started_at: &str,
    _query_corpus: QueryCorpusReport,
    _corpus: Vec<QueryCorpusRecord>,
    _rows: IngestRows,
) -> Result<BenchmarkReport> {
    Err(HotIndexError::Config(
        "ToplingDB benchmark requires `--features toplingsdb`".to_string(),
    ))
}

#[cfg(feature = "rocksdb")]
fn run_rocksdb_read_under_ingest(
    dataset: &Path,
    db_path: Option<PathBuf>,
    workload: &str,
    iterations: usize,
    warmup: usize,
    access_pattern: &str,
    seed: &str,
    checksum_status: &str,
    expected_checksum: Option<&Path>,
    manifest: &DatasetManifest,
    started_at: &str,
    query_corpus: QueryCorpusReport,
    corpus: Vec<QueryCorpusRecord>,
    rows: IngestRows,
) -> Result<BenchmarkReport> {
    let path = db_path.unwrap_or_else(|| dataset.join("materialized/rocksdb-bench-read-ingest"));
    let engine = RocksDbEngine::open(path)?;
    let result = measure_read_under_ingest(
        &engine,
        &rows,
        &corpus,
        iterations,
        warmup,
        access_pattern,
        seed,
    )?;
    put_checkpoint_from_manifest(dataset, &engine)?;
    build_report(
        manifest,
        dataset,
        "rocksdb",
        "read-under-ingest",
        workload,
        iterations,
        warmup,
        access_pattern,
        seed,
        Some(query_corpus),
        started_at,
        result,
        engine.checksums().ok(),
        checksum_status,
        expected_checksum,
    )
}

#[cfg(not(feature = "rocksdb"))]
#[allow(clippy::too_many_arguments)]
fn run_rocksdb_read_under_ingest(
    _dataset: &Path,
    _db_path: Option<PathBuf>,
    _workload: &str,
    _iterations: usize,
    _warmup: usize,
    _access_pattern: &str,
    _seed: &str,
    _checksum_status: &str,
    _expected_checksum: Option<&Path>,
    _manifest: &DatasetManifest,
    _started_at: &str,
    _query_corpus: QueryCorpusReport,
    _corpus: Vec<QueryCorpusRecord>,
    _rows: IngestRows,
) -> Result<BenchmarkReport> {
    Err(HotIndexError::Config(
        "RocksDB benchmark requires `--features rocksdb`".to_string(),
    ))
}

fn measure_queries<E: StorageEngine>(
    engine: &E,
    corpus: &[QueryCorpusRecord],
    iterations: usize,
    warmup: usize,
    access_pattern: &str,
    seed: &str,
) -> Result<BenchResult> {
    validate_access_pattern(access_pattern)?;
    let mut rng = DeterministicRng::new(seed_from_string(seed));
    for idx in 0..warmup {
        let record_idx = query_index(access_pattern, idx, corpus.len(), &mut rng);
        let _ = execute_query(engine, &corpus[record_idx]);
    }

    let mut latencies = Vec::with_capacity(iterations);
    let mut errors = 0_u64;
    let started = Instant::now();
    for idx in 0..iterations {
        let record_idx = query_index(access_pattern, idx, corpus.len(), &mut rng);
        let record = &corpus[record_idx];
        let op_started = Instant::now();
        if execute_query(engine, record).is_err() {
            errors += 1;
        }
        latencies.push(op_started.elapsed().as_micros() as u64);
    }
    Ok(finish_result(
        iterations as u64,
        errors,
        started.elapsed(),
        latencies,
    ))
}

fn validate_access_pattern(access_pattern: &str) -> Result<()> {
    match access_pattern {
        "sequential" | "uniform" | "zipfian" => Ok(()),
        other => Err(HotIndexError::Config(format!(
            "unsupported access pattern: {other}; expected sequential, uniform, or zipfian"
        ))),
    }
}

fn query_index(
    access_pattern: &str,
    sequential_idx: usize,
    corpus_len: usize,
    rng: &mut DeterministicRng,
) -> usize {
    match access_pattern {
        "uniform" => rng.next_usize(corpus_len),
        "zipfian" => rng.next_zipf_like(corpus_len),
        _ => sequential_idx % corpus_len,
    }
}

#[derive(Debug, Clone)]
struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.state
    }

    fn next_usize(&mut self, upper: usize) -> usize {
        if upper <= 1 {
            return 0;
        }
        (self.next_u64() as usize) % upper
    }

    fn next_zipf_like(&mut self, upper: usize) -> usize {
        if upper <= 1 {
            return 0;
        }
        let unit = ((self.next_u64() >> 11) as f64) / ((1_u64 << 53) as f64);
        let skewed = unit * unit * unit;
        ((skewed * upper as f64) as usize).min(upper - 1)
    }
}

fn seed_from_string(seed: &str) -> u64 {
    seed.as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |state, byte| {
            (state ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

fn execute_query<E: StorageEngine>(engine: &E, record: &QueryCorpusRecord) -> Result<()> {
    match record.query_kind {
        QueryKind::GetTxByVersion => {
            let version = record.tx_version.ok_or_else(|| {
                HotIndexError::Config("get_tx_by_version missing tx_version".to_string())
            })?;
            engine.get_tx(version)?;
        }
        QueryKind::MultiGetTxVersions => {
            engine.multi_get_txs(&record.tx_versions)?;
        }
        QueryKind::MarketFillScan => {
            let market_id = required(record.market_id.as_deref(), "market_id")?;
            engine.scan_market_fills(market_id, record.limit.unwrap_or(100))?;
        }
        QueryKind::AccountFillScan => {
            let account = required(record.account.as_deref(), "account")?;
            engine.scan_account_fills(account, record.limit.unwrap_or(100))?;
        }
        QueryKind::BuilderCodeFillScan => {
            let builder_addr = required(record.builder_addr.as_deref(), "builder_addr")?;
            engine.scan_builder_code_fills(builder_addr, record.limit.unwrap_or(100))?;
        }
        QueryKind::BuilderCodeVolume => {
            let builder_addr = required(record.builder_addr.as_deref(), "builder_addr")?;
            engine.get_builder_code_volume(builder_addr, TimeWindow::H24)?;
        }
        QueryKind::MixedDashboard => {}
    }
    Ok(())
}

fn warmup_ingest<E: StorageEngine>(engine: &E, rows: &IngestRows, warmup: usize) -> Result<()> {
    let limit = warmup.min(rows.total_rows());
    rows.replay_limit(engine, limit)?;
    Ok(())
}

fn measure_ingest<E: StorageEngine>(
    engine: &E,
    rows: &IngestRows,
    requested_iterations: usize,
) -> Result<BenchResult> {
    let limit = requested_iterations.min(rows.total_rows());
    let mut latencies = Vec::with_capacity(limit);
    let mut errors = 0_u64;
    let started = Instant::now();
    rows.replay_each(limit, |op| {
        let op_started = Instant::now();
        if op(engine).is_err() {
            errors += 1;
        }
        latencies.push(op_started.elapsed().as_micros() as u64);
        Ok(())
    })?;
    Ok(finish_result(
        limit as u64,
        errors,
        started.elapsed(),
        latencies,
    ))
}

fn measure_read_under_ingest<E: StorageEngine>(
    engine: &E,
    rows: &IngestRows,
    corpus: &[QueryCorpusRecord],
    iterations: usize,
    warmup: usize,
    access_pattern: &str,
    seed: &str,
) -> Result<BenchResult> {
    std::thread::scope(|scope| {
        let ingest = scope.spawn(|| rows.replay_limit(engine, rows.total_rows()));
        let result = measure_queries(engine, corpus, iterations, warmup, access_pattern, seed)?;
        ingest.join().map_err(|_| {
            HotIndexError::Storage("background ingest thread panicked".to_string())
        })??;
        Ok(result)
    })
}

fn finish_result(
    operations: u64,
    errors: u64,
    elapsed: Duration,
    mut latencies: Vec<u64>,
) -> BenchResult {
    latencies.sort_unstable();
    let elapsed_seconds = elapsed.as_secs_f64();
    BenchResult {
        operations,
        errors,
        elapsed_seconds,
        throughput_qps: if elapsed_seconds > 0.0 {
            operations as f64 / elapsed_seconds
        } else {
            0.0
        },
        latency_us: LatencySummary {
            p50: percentile(&latencies, 0.50),
            p95: percentile(&latencies, 0.95),
            p99: percentile(&latencies, 0.99),
            p999: percentile(&latencies, 0.999),
            max: latencies.last().copied().unwrap_or_default(),
        },
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).ceil() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn replay_into_engine<E: StorageEngine>(root: &Path, engine: &E) -> Result<()> {
    let rows = IngestRows::load(root)?;
    rows.replay_limit(engine, rows.total_rows())?;
    put_checkpoint_from_manifest(root, engine)
}

fn put_checkpoint_from_manifest<E: StorageEngine>(root: &Path, engine: &E) -> Result<()> {
    let manifest = read_json::<DatasetManifest>(&root.join("manifest.json"))?;
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

struct IngestRows {
    txs: Vec<TxRow>,
    events: Vec<NormalizedEvent>,
    fills: Vec<FillRow>,
    orders: Vec<OrderRow>,
    positions: Vec<PositionRow>,
    builder_rows: Vec<BuilderAttributionRow>,
    activity_rows: Vec<ActivityRow>,
}

impl IngestRows {
    fn load(root: &Path) -> Result<Self> {
        let normalized = root.join("normalized");
        Ok(Self {
            txs: read_ndjson(&normalized.join("txs.ndjson"))?,
            events: read_ndjson(&normalized.join("events.ndjson"))?,
            fills: read_ndjson(&normalized.join("fills.ndjson"))?,
            orders: read_ndjson(&normalized.join("orders.ndjson"))?,
            positions: read_ndjson(&normalized.join("positions.ndjson"))?,
            builder_rows: read_ndjson(&normalized.join("builder_code_rows.ndjson"))?,
            activity_rows: read_ndjson(&normalized.join("activity_rows.ndjson"))?,
        })
    }

    fn total_rows(&self) -> usize {
        self.txs.len()
            + self.events.len()
            + self.fills.len()
            + self.orders.len()
            + self.positions.len()
            + self.builder_rows.len()
            + self.activity_rows.len()
    }

    fn replay_limit<E: StorageEngine>(&self, engine: &E, limit: usize) -> Result<()> {
        self.replay_each(limit, |op| {
            op(engine)?;
            Ok(())
        })
    }

    fn replay_each<E, F>(&self, limit: usize, mut observe: F) -> Result<()>
    where
        E: StorageEngine,
        F: FnMut(&dyn Fn(&E) -> Result<()>) -> Result<()>,
    {
        let mut seen = 0_usize;
        macro_rules! replay_rows {
            ($rows:expr, $put:ident) => {
                for row in &$rows {
                    if seen >= limit {
                        return Ok(());
                    }
                    observe(&|engine: &E| engine.$put(row.clone()))?;
                    seen += 1;
                }
            };
        }

        replay_rows!(self.txs, put_tx);
        replay_rows!(self.events, put_event);
        replay_rows!(self.fills, put_fill);
        replay_rows!(self.orders, put_order);
        replay_rows!(self.positions, put_position);
        replay_rows!(self.builder_rows, put_builder_attribution);
        replay_rows!(self.activity_rows, put_activity);
        Ok(())
    }
}

fn query_corpus_report(root: &Path, workload: &str) -> Result<QueryCorpusReport> {
    let queries = root.join("queries");
    let file = match workload {
        "get_tx_by_version" => "point_tx_versions.ndjson",
        "multi_get_tx_versions_100" => "multi_get_tx_versions.ndjson",
        "scan_market_recent_fills_100" => "market_fill_scans.ndjson",
        "scan_account_recent_fills_100" => "account_fill_scans.ndjson",
        "scan_builder_code_fills_100" => "builder_code_scans.ndjson",
        "get_builder_code_volume_24h" => "builder_code_volumes.ndjson",
        "mixed_market_dashboard" | "mixed_dashboard" => "mixed_dashboard.ndjson",
        other => {
            return Err(HotIndexError::Config(format!(
                "unsupported workload: {other}"
            )));
        }
    };
    let path = queries.join(file);
    Ok(QueryCorpusReport {
        workload: workload.to_string(),
        relative_path: format!("queries/{file}"),
        sha256: sha256_file(&path)?,
        path,
    })
}

fn build_report(
    manifest: &DatasetManifest,
    dataset: &Path,
    backend: &str,
    benchmark_class: &str,
    workload: &str,
    iterations: usize,
    warmup: usize,
    access_pattern: &str,
    seed: &str,
    query_corpus: Option<QueryCorpusReport>,
    started_at: &str,
    result: BenchResult,
    checksums: Option<Vec<CfChecksum>>,
    checksum_status: &str,
    expected_checksum: Option<&Path>,
) -> Result<BenchmarkReport> {
    let checksum = build_checksum_report(checksum_status, checksums, expected_checksum)?;
    Ok(BenchmarkReport {
        report_version: 1,
        started_at: started_at.to_string(),
        methodology_status: "engineering_smoke_not_publishable".to_string(),
        benchmark_class: benchmark_class.to_string(),
        backend: backend.to_string(),
        workload: workload.to_string(),
        iterations,
        warmup,
        access_pattern: access_pattern.to_string(),
        seed: seed.to_string(),
        query_corpus,
        dataset: DatasetReport {
            dataset_id: manifest.dataset_id.0.clone(),
            network: manifest.network.as_str().to_string(),
            start_version: manifest.start_version,
            end_version: manifest.end_version,
            raw_transaction_count: manifest.raw_transaction_count,
            decibel_event_count: manifest.decibel_event_count,
            fill_count: manifest.fill_count,
            builder_code_row_count: manifest.builder_code_row_count,
            manifest_sha256: sha256_file(&dataset.join("manifest.json"))?,
        },
        checksum,
        environment: EnvironmentReport::capture(dataset),
        result,
        disclaimer: "engineering smoke benchmark only; not publishable methodology until HDR/open-loop/concurrency/env fingerprint hardening lands".to_string(),
    })
}

fn build_checksum_report(
    fallback_status: &str,
    checksums: Option<Vec<CfChecksum>>,
    expected_checksum: Option<&Path>,
) -> Result<ChecksumReport> {
    let actual = checksums.unwrap_or_default();
    let status = if let Some(expected_path) = expected_checksum {
        let expected = read_json::<Vec<CfChecksum>>(expected_path)?;
        if expected == actual {
            "pass".to_string()
        } else {
            "fail".to_string()
        }
    } else {
        fallback_status.to_string()
    };
    Ok(ChecksumReport {
        status,
        logical_cfs: actual,
    })
}

#[derive(Debug, Serialize, Deserialize)]
struct BenchmarkReport {
    report_version: u32,
    started_at: String,
    #[serde(default)]
    methodology_status: String,
    benchmark_class: String,
    backend: String,
    workload: String,
    iterations: usize,
    warmup: usize,
    access_pattern: String,
    seed: String,
    query_corpus: Option<QueryCorpusReport>,
    dataset: DatasetReport,
    checksum: ChecksumReport,
    environment: EnvironmentReport,
    result: BenchResult,
    disclaimer: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct QueryCorpusReport {
    workload: String,
    relative_path: String,
    sha256: String,
    #[serde(skip)]
    path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct DatasetReport {
    dataset_id: String,
    network: String,
    start_version: u64,
    end_version: Option<u64>,
    raw_transaction_count: u64,
    decibel_event_count: u64,
    fill_count: u64,
    builder_code_row_count: u64,
    manifest_sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChecksumReport {
    status: String,
    logical_cfs: Vec<CfChecksum>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EnvironmentReport {
    os: String,
    arch: String,
    cpu_parallelism: usize,
    storage_path: String,
    env: BTreeMap<String, String>,
}

impl EnvironmentReport {
    fn capture(dataset: &Path) -> Self {
        let mut env = BTreeMap::new();
        env.insert(
            "rust_profile".to_string(),
            if cfg!(debug_assertions) {
                "debug".to_string()
            } else {
                "release".to_string()
            },
        );
        env.insert(
            "bench_crate_version".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        );
        Self {
            os: env::consts::OS.to_string(),
            arch: env::consts::ARCH.to_string(),
            cpu_parallelism: std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or_default(),
            storage_path: dataset.display().to_string(),
            env,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct BenchResult {
    operations: u64,
    errors: u64,
    elapsed_seconds: f64,
    throughput_qps: f64,
    latency_us: LatencySummary,
}

#[derive(Debug, Serialize, Deserialize)]
struct LatencySummary {
    p50: u64,
    p95: u64,
    p99: u64,
    p999: u64,
    max: u64,
}

fn required<'a>(value: Option<&'a str>, name: &str) -> Result<&'a str> {
    value.ok_or_else(|| HotIndexError::Config(format!("missing query field: {name}")))
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

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = File::open(path)?;
    serde_json::from_reader(BufReader::new(file)).map_err(json_error)
}

fn write_json_pretty<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    serde_json::to_writer_pretty(BufWriter::new(file), value).map_err(json_error)
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

fn iso_like_now() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => format!(
            "unix_epoch_seconds:{}.{:09}",
            duration.as_secs(),
            duration.subsec_nanos()
        ),
        Err(_) => "unix_epoch_seconds:0.000000000".to_string(),
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

    fn required_value(&self, name: &str) -> Result<&'a str> {
        self.optional_value(name)
            .ok_or_else(|| HotIndexError::Config(format!("missing required argument: {name}")))
    }

    fn optional_value(&self, name: &str) -> Option<&'a str> {
        self.args
            .windows(2)
            .find(|window| window[0] == name)
            .map(|window| window[1].as_str())
    }

    fn optional_usize(&self, name: &str) -> Result<Option<usize>> {
        self.optional_value(name)
            .map(|value| {
                value.parse::<usize>().map_err(|_| {
                    HotIndexError::Config(format!("invalid integer for {name}: {value}"))
                })
            })
            .transpose()
    }
}

fn print_usage() {
    eprintln!(
        "usage:
  decibel-hotindex-bench run --dataset <dataset-dir> --engine memory --class serving --workload mixed_market_dashboard --iterations <n> --warmup <n> --access-pattern sequential|uniform|zipfian --seed <seed> --out <report.json>
  decibel-hotindex-bench run --features rocksdb --dataset <dataset-dir> --engine rocksdb --db-path <rocksdb-path> --class serving --workload mixed_market_dashboard --iterations <n> --warmup <n> --access-pattern sequential|uniform|zipfian --seed <seed> --out <report.json>
  decibel-hotindex-bench run --dataset <dataset-dir> --engine memory --class ingest --iterations <n> --warmup <n> --out <report.json>
  decibel-hotindex-bench summarize --reports <report-a.json,report-b.json> --out reports/BENCHMARK_SUMMARY.md"
    );
}

fn write_markdown_summary(path: &Path, reports: &[BenchmarkReport]) -> Result<()> {
    let first = &reports[0];
    let query_corpus = first
        .query_corpus
        .as_ref()
        .map(|corpus| format!("{} ({})", corpus.relative_path, corpus.sha256))
        .unwrap_or_else(|| "n/a".to_string());
    let mut text = String::new();
    text.push_str("# Benchmark Summary\n\n");
    text.push_str(&format!("- dataset_id: {}\n", first.dataset.dataset_id));
    text.push_str(&format!("- network: {}\n", first.dataset.network));
    text.push_str(&format!(
        "- version_range: {}..{}\n",
        first.dataset.start_version,
        first
            .dataset
            .end_version
            .map(|value| value.to_string())
            .unwrap_or_else(|| "open".to_string())
    ));
    text.push_str(&format!(
        "- manifest_sha256: {}\n",
        first.dataset.manifest_sha256
    ));
    text.push_str(&format!("- query_corpus: {query_corpus}\n"));
    text.push_str(&format!(
        "- methodology_status: {}\n",
        non_empty(
            &first.methodology_status,
            "engineering_smoke_not_publishable"
        )
    ));
    text.push_str(&format!(
        "- checksum_status: {}\n",
        checksum_status_summary(reports)
    ));
    text.push_str(&format!(
        "- environment: {} {} cores={} storage={}\n",
        first.environment.os,
        first.environment.arch,
        first.environment.cpu_parallelism,
        first.environment.storage_path
    ));
    text.push_str(
        "- disclaimer: engineering smoke benchmark only; not publishable methodology\n\n",
    );
    text.push_str("| class | backend | workload | ops | errors | qps | p50_us | p95_us | p99_us | p999_us | checksum |\n");
    text.push_str("| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n");
    for report in reports {
        text.push_str(&format!(
            "| {} | {} | {} | {} | {} | {:.2} | {} | {} | {} | {} | {} |\n",
            report.benchmark_class,
            report.backend,
            report.workload,
            report.result.operations,
            report.result.errors,
            report.result.throughput_qps,
            report.result.latency_us.p50,
            report.result.latency_us.p95,
            report.result.latency_us.p99,
            report.result.latency_us.p999,
            report.checksum.status
        ));
    }
    text.push('\n');
    text.push_str("Notes:\n\n");
    text.push_str(
        "- Serving benchmarks are offline and consume only local dataset/query corpus files.\n",
    );
    text.push_str("- Reported numbers are engineering smoke results unless produced from a pinned release build and checksum-passed backend comparison.\n");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)?;
    Ok(())
}

fn non_empty<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() {
        fallback
    } else {
        value
    }
}

fn checksum_status_summary(reports: &[BenchmarkReport]) -> String {
    let mut counts = BTreeMap::<String, usize>::new();
    for report in reports {
        *counts.entry(report.checksum.status.clone()).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(status, count)| format!("{status}={count}"))
        .collect::<Vec<_>>()
        .join(", ")
}
