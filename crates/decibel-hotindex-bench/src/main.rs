use decibel_hotindex_core::{
    ActivityRow, BuilderAttributionRow, CfChecksum, DatasetEncoding, DatasetManifest, FillRow,
    HotIndexError, IngestCheckpoint, NormalizedEvent, OrderRow, PositionRow, QueryCorpusRecord,
    QueryKind, Result, TimeWindow, TxRow, LOGICAL_SCHEMA_VERSION,
};
#[cfg(feature = "rocksdb")]
use decibel_hotindex_storage::RocksDbEngine;
use decibel_hotindex_storage::{MemoryEngine, StorageEngine};
use hdrhistogram::Histogram;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
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
    let concurrency = opts.optional_usize("--concurrency")?.unwrap_or(1);
    if concurrency == 0 {
        return Err(HotIndexError::Config(
            "--concurrency must be greater than zero".to_string(),
        ));
    }
    let out = opts
        .optional_value("--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("reports/bench-local.json"));
    let expected_checksum = opts
        .optional_value("--expected-checksum")
        .map(PathBuf::from);
    let allow_failures = opts.has_flag("--allow-failures");
    let publishable_candidate = opts.has_flag("--publishable-candidate");
    if publishable_candidate && allow_failures {
        return Err(HotIndexError::Config(
            "--publishable-candidate cannot be combined with --allow-failures".to_string(),
        ));
    }
    #[cfg(feature = "rocksdb")]
    let db_path = opts.optional_value("--db-path").map(PathBuf::from);
    let access_pattern = opts
        .optional_value("--access-pattern")
        .unwrap_or("sequential");
    let seed = opts
        .optional_value("--seed")
        .unwrap_or("query-corpus-order");
    let rate_qps = opts.optional_f64("--rate")?;
    if let Some(rate_qps) = rate_qps {
        if !rate_qps.is_finite() || rate_qps <= 0.0 {
            return Err(HotIndexError::Config(format!(
                "--rate must be a positive finite qps value, got {rate_qps}"
            )));
        }
    }
    let compact_before_run = opts.has_flag("--compact-before-run");
    let cache_state = opts
        .optional_value("--cache-state")
        .unwrap_or("unspecified");
    validate_cache_state(cache_state)?;
    let cache_clear_command = opts.optional_value("--cache-clear-command");
    if cache_state == "cold-cleared" && cache_clear_command.is_none() {
        return Err(HotIndexError::Config(
            "--cache-state cold-cleared requires --cache-clear-command".to_string(),
        ));
    }
    let checksum_status = opts
        .optional_value("--checksum-status")
        .unwrap_or("not_run");
    let manifest = read_json::<DatasetManifest>(&dataset.join("manifest.json"))?;
    validate_benchmark_dataset(&dataset, &manifest)?;
    let started_at = iso_like_now();
    let params = BenchParams {
        dataset: &dataset,
        #[cfg(feature = "rocksdb")]
        db_path: db_path.as_deref(),
        backend: &backend,
        workload,
        iterations,
        warmup,
        concurrency,
        access_pattern,
        seed,
        rate_qps,
        compact_before_run,
        cache_state,
        cache_clear_command,
        checksum_status,
        expected_checksum: expected_checksum.as_deref(),
        publishable_candidate,
        manifest: &manifest,
        started_at: &started_at,
    };

    let mut report = match bench_class {
        "serving" => run_serving_bench(&params)?,
        "ingest" => run_ingest_bench(&params)?,
        "read-under-ingest" | "read_under_ingest" => run_read_under_ingest_bench(&params)?,
        other => {
            return Err(HotIndexError::Config(format!(
                "unsupported benchmark class: {other}"
            )));
        }
    };
    let gate_failures = report_gate_failures(&report);
    report.gate = ReportGate::from_failures(&gate_failures, allow_failures);

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
    enforce_report_gate(&gate_failures, allow_failures)?;
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

#[derive(Debug, Clone, Copy)]
struct BenchParams<'a> {
    dataset: &'a Path,
    #[cfg(feature = "rocksdb")]
    db_path: Option<&'a Path>,
    backend: &'a str,
    workload: &'a str,
    iterations: usize,
    warmup: usize,
    concurrency: usize,
    access_pattern: &'a str,
    seed: &'a str,
    rate_qps: Option<f64>,
    compact_before_run: bool,
    cache_state: &'a str,
    cache_clear_command: Option<&'a str>,
    checksum_status: &'a str,
    expected_checksum: Option<&'a Path>,
    publishable_candidate: bool,
    manifest: &'a DatasetManifest,
    started_at: &'a str,
}

impl<'a> BenchParams<'a> {
    fn query_measure_config(&self) -> QueryMeasureConfig<'a> {
        QueryMeasureConfig {
            iterations: self.iterations,
            warmup: self.warmup,
            concurrency: self.concurrency,
            access_pattern: self.access_pattern,
            seed: self.seed,
            rate_qps: self.rate_qps,
        }
    }
}

fn run_serving_bench(params: &BenchParams<'_>) -> Result<BenchmarkReport> {
    ensure_serving_workload_supported(params.manifest, params.workload)?;
    let query_corpus = query_corpus_report(params.dataset, params.workload)?;
    validate_query_corpus_hash(params.manifest, &query_corpus)?;
    let corpus = read_ndjson(&query_corpus.path)?;
    if corpus.is_empty() {
        return Err(empty_corpus_error(params.manifest, params.workload));
    }

    match params.backend {
        "memory" => {
            let engine = MemoryEngine::default();
            replay_into_engine(params.dataset, &engine)?;
            let result = measure_queries(&engine, &corpus, params.query_measure_config())?;
            build_report(ReportInput {
                manifest: params.manifest,
                dataset: params.dataset,
                backend: params.backend,
                benchmark_class: "serving",
                workload: params.workload,
                iterations: params.iterations,
                warmup: params.warmup,
                concurrency: params.concurrency,
                access_pattern: params.access_pattern,
                seed: params.seed,
                rate_qps: params.rate_qps,
                query_corpus: Some(query_corpus),
                started_at: params.started_at,
                result,
                checksums: engine.checksums().ok(),
                checksum_status: "pass",
                storage_state: memory_storage_state(
                    params.dataset,
                    params.compact_before_run,
                    params.cache_state,
                    params.cache_clear_command,
                ),
                expected_checksum: params.expected_checksum,
                publishable_candidate: params.publishable_candidate,
            })
        }
        "rocksdb" => run_rocksdb_serving(params, query_corpus, corpus),
        other => Err(HotIndexError::Config(format!(
            "unsupported serving backend: {other}"
        ))),
    }
}

fn ensure_serving_workload_supported(manifest: &DatasetManifest, workload: &str) -> Result<()> {
    if manifest.decibel_event_count == 0 && requires_decibel_events(workload) {
        return Err(empty_corpus_error(manifest, workload));
    }
    if requires_fills(workload) && manifest.fill_count == 0 {
        return Err(HotIndexError::Config(format!(
            "workload {workload} requires normalized fills, but this dataset has fill_count=0; choose a Decibel-active range with TradeEvent/Fill rows"
        )));
    }
    if requires_builder_rows(workload) && manifest.builder_code_row_count == 0 {
        return Err(HotIndexError::Config(format!(
            "workload {workload} requires builder-code attribution rows, but this dataset has builder_code_row_count=0; choose a range with builder attribution data or run a narrower workload"
        )));
    }
    Ok(())
}

fn requires_decibel_events(workload: &str) -> bool {
    !matches!(workload, "get_tx_by_version" | "multi_get_tx_versions_100")
}

fn requires_fills(workload: &str) -> bool {
    matches!(
        workload,
        "scan_market_recent_fills_100"
            | "scan_account_recent_fills_100"
            | "mixed_market_dashboard"
            | "mixed_dashboard"
    )
}

fn requires_builder_rows(workload: &str) -> bool {
    matches!(
        workload,
        "scan_builder_code_fills_100"
            | "get_builder_code_volume_24h"
            | "mixed_market_dashboard"
            | "mixed_dashboard"
    )
}

fn empty_corpus_error(manifest: &DatasetManifest, workload: &str) -> HotIndexError {
    if manifest.decibel_event_count == 0 && requires_decibel_events(workload) {
        HotIndexError::Config(format!(
            "workload {workload} requires Decibel events, but this dataset has decibel_event_count=0; choose a Decibel-active range or run tx point/multi-get workloads"
        ))
    } else {
        HotIndexError::Config(format!("query corpus for workload {workload} is empty"))
    }
}

#[cfg(feature = "rocksdb")]
fn run_rocksdb_serving(
    params: &BenchParams<'_>,
    query_corpus: QueryCorpusReport,
    corpus: Vec<QueryCorpusRecord>,
) -> Result<BenchmarkReport> {
    let path = params
        .db_path
        .map(PathBuf::from)
        .unwrap_or_else(|| params.dataset.join("materialized/rocksdb"));
    let engine = RocksDbEngine::open(&path)?;
    let compaction_performed = maybe_compact_rocksdb(&engine, params.compact_before_run)?;
    let result = measure_queries(&engine, &corpus, params.query_measure_config())?;
    build_report(ReportInput {
        manifest: params.manifest,
        dataset: params.dataset,
        backend: "rocksdb",
        benchmark_class: "serving",
        workload: params.workload,
        iterations: params.iterations,
        warmup: params.warmup,
        concurrency: params.concurrency,
        access_pattern: params.access_pattern,
        seed: params.seed,
        rate_qps: params.rate_qps,
        query_corpus: Some(query_corpus),
        started_at: params.started_at,
        result,
        checksums: engine.checksums().ok(),
        checksum_status: params.checksum_status,
        storage_state: rocksdb_storage_state(
            &path,
            params.compact_before_run,
            compaction_performed,
            params.cache_state,
            params.cache_clear_command,
        ),
        expected_checksum: params.expected_checksum,
        publishable_candidate: params.publishable_candidate,
    })
}

#[cfg(not(feature = "rocksdb"))]
fn run_rocksdb_serving(
    _params: &BenchParams<'_>,
    _query_corpus: QueryCorpusReport,
    _corpus: Vec<QueryCorpusRecord>,
) -> Result<BenchmarkReport> {
    Err(HotIndexError::Config(
        "RocksDB benchmark requires `--features rocksdb`".to_string(),
    ))
}

fn run_ingest_bench(params: &BenchParams<'_>) -> Result<BenchmarkReport> {
    match params.backend {
        "memory" => {
            let rows = IngestRows::load(params.dataset)?;
            let engine = MemoryEngine::default();
            warmup_ingest(&engine, &rows, params.warmup)?;
            let result = measure_ingest(&engine, &rows, params.iterations)?;
            put_checkpoint_from_manifest(params.dataset, &engine)?;
            build_report(ReportInput {
                manifest: params.manifest,
                dataset: params.dataset,
                backend: params.backend,
                benchmark_class: "ingest",
                workload: "normalized_replay",
                iterations: params.iterations,
                warmup: params.warmup,
                concurrency: 1,
                access_pattern: "sequential",
                seed: "normalized-row-order",
                rate_qps: None,
                query_corpus: None,
                started_at: params.started_at,
                result,
                checksums: engine.checksums().ok(),
                checksum_status: "pass",
                storage_state: memory_storage_state(
                    params.dataset,
                    params.compact_before_run,
                    params.cache_state,
                    params.cache_clear_command,
                ),
                expected_checksum: params.expected_checksum,
                publishable_candidate: params.publishable_candidate,
            })
        }
        "rocksdb" => run_rocksdb_ingest(params),
        other => Err(HotIndexError::Config(format!(
            "unsupported ingest backend: {other}"
        ))),
    }
}

#[cfg(feature = "rocksdb")]
fn run_rocksdb_ingest(params: &BenchParams<'_>) -> Result<BenchmarkReport> {
    let rows = IngestRows::load(params.dataset)?;
    let path = params
        .db_path
        .map(PathBuf::from)
        .unwrap_or_else(|| params.dataset.join("materialized/rocksdb-bench-ingest"));
    let engine = RocksDbEngine::open(&path)?;
    warmup_ingest(&engine, &rows, params.warmup)?;
    let compaction_performed = maybe_compact_rocksdb(&engine, params.compact_before_run)?;
    let result = measure_ingest(&engine, &rows, params.iterations)?;
    put_checkpoint_from_manifest(params.dataset, &engine)?;
    build_report(ReportInput {
        manifest: params.manifest,
        dataset: params.dataset,
        backend: "rocksdb",
        benchmark_class: "ingest",
        workload: "normalized_replay",
        iterations: params.iterations,
        warmup: params.warmup,
        concurrency: 1,
        access_pattern: "sequential",
        seed: "normalized-row-order",
        rate_qps: None,
        query_corpus: None,
        started_at: params.started_at,
        result,
        checksums: engine.checksums().ok(),
        checksum_status: params.checksum_status,
        storage_state: rocksdb_storage_state(
            &path,
            params.compact_before_run,
            compaction_performed,
            params.cache_state,
            params.cache_clear_command,
        ),
        expected_checksum: params.expected_checksum,
        publishable_candidate: params.publishable_candidate,
    })
}

#[cfg(not(feature = "rocksdb"))]
fn run_rocksdb_ingest(_params: &BenchParams<'_>) -> Result<BenchmarkReport> {
    Err(HotIndexError::Config(
        "RocksDB benchmark requires `--features rocksdb`".to_string(),
    ))
}

fn run_read_under_ingest_bench(params: &BenchParams<'_>) -> Result<BenchmarkReport> {
    let query_corpus = query_corpus_report(params.dataset, params.workload)?;
    validate_query_corpus_hash(params.manifest, &query_corpus)?;
    let corpus = read_ndjson(&query_corpus.path)?;
    if corpus.is_empty() {
        return Err(empty_corpus_error(params.manifest, params.workload));
    }
    let rows = IngestRows::load(params.dataset)?;
    match params.backend {
        "memory" => {
            let engine = MemoryEngine::default();
            let result =
                measure_read_under_ingest(&engine, &rows, &corpus, params.query_measure_config())?;
            put_checkpoint_from_manifest(params.dataset, &engine)?;
            build_report(ReportInput {
                manifest: params.manifest,
                dataset: params.dataset,
                backend: params.backend,
                benchmark_class: "read-under-ingest",
                workload: params.workload,
                iterations: params.iterations,
                warmup: params.warmup,
                concurrency: params.concurrency,
                access_pattern: params.access_pattern,
                seed: params.seed,
                rate_qps: params.rate_qps,
                query_corpus: Some(query_corpus),
                started_at: params.started_at,
                result,
                checksums: engine.checksums().ok(),
                checksum_status: params.checksum_status,
                storage_state: memory_storage_state(
                    params.dataset,
                    params.compact_before_run,
                    params.cache_state,
                    params.cache_clear_command,
                ),
                expected_checksum: params.expected_checksum,
                publishable_candidate: params.publishable_candidate,
            })
        }
        "rocksdb" => run_rocksdb_read_under_ingest(params, query_corpus, corpus, rows),
        other => Err(HotIndexError::Config(format!(
            "unsupported read-under-ingest backend: {other}"
        ))),
    }
}

#[cfg(feature = "rocksdb")]
fn run_rocksdb_read_under_ingest(
    params: &BenchParams<'_>,
    query_corpus: QueryCorpusReport,
    corpus: Vec<QueryCorpusRecord>,
    rows: IngestRows,
) -> Result<BenchmarkReport> {
    let path = params.db_path.map(PathBuf::from).unwrap_or_else(|| {
        params
            .dataset
            .join("materialized/rocksdb-bench-read-ingest")
    });
    let engine = RocksDbEngine::open(&path)?;
    let compaction_performed = maybe_compact_rocksdb(&engine, params.compact_before_run)?;
    let result = measure_read_under_ingest(&engine, &rows, &corpus, params.query_measure_config())?;
    put_checkpoint_from_manifest(params.dataset, &engine)?;
    build_report(ReportInput {
        manifest: params.manifest,
        dataset: params.dataset,
        backend: "rocksdb",
        benchmark_class: "read-under-ingest",
        workload: params.workload,
        iterations: params.iterations,
        warmup: params.warmup,
        concurrency: params.concurrency,
        access_pattern: params.access_pattern,
        seed: params.seed,
        rate_qps: params.rate_qps,
        query_corpus: Some(query_corpus),
        started_at: params.started_at,
        result,
        checksums: engine.checksums().ok(),
        checksum_status: params.checksum_status,
        storage_state: rocksdb_storage_state(
            &path,
            params.compact_before_run,
            compaction_performed,
            params.cache_state,
            params.cache_clear_command,
        ),
        expected_checksum: params.expected_checksum,
        publishable_candidate: params.publishable_candidate,
    })
}

#[cfg(not(feature = "rocksdb"))]
fn run_rocksdb_read_under_ingest(
    _params: &BenchParams<'_>,
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
    config: QueryMeasureConfig<'_>,
) -> Result<BenchResult> {
    validate_access_pattern(config.access_pattern)?;
    validate_concurrency(config.iterations, config.concurrency)?;
    let mut rng = DeterministicRng::new(seed_from_string(config.seed));
    for idx in 0..config.warmup {
        let record_idx = query_index(config.access_pattern, idx, corpus.len(), &mut rng);
        execute_query(engine, &corpus[record_idx])?;
    }

    let started = Instant::now();
    let worker_results = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(config.concurrency);
        for worker_idx in 0..config.concurrency {
            handles.push(scope.spawn(move || {
                measure_query_worker(QueryWorkerInput {
                    engine,
                    corpus,
                    iterations: config.iterations,
                    concurrency: config.concurrency,
                    worker_idx,
                    access_pattern: config.access_pattern,
                    seed: config.seed,
                    rate_qps: config.rate_qps,
                    started,
                })
            }));
        }

        let mut results = Vec::with_capacity(config.concurrency);
        for handle in handles {
            let result = handle.join().map_err(|_| {
                HotIndexError::Storage("query worker thread panicked".to_string())
            })??;
            results.push(result);
        }
        Ok::<_, HotIndexError>(results)
    })?;
    finish_query_results(worker_results, started.elapsed())
}

#[derive(Debug, Clone, Copy)]
struct QueryMeasureConfig<'a> {
    iterations: usize,
    warmup: usize,
    concurrency: usize,
    access_pattern: &'a str,
    seed: &'a str,
    rate_qps: Option<f64>,
}

fn validate_access_pattern(access_pattern: &str) -> Result<()> {
    match access_pattern {
        "sequential" | "uniform" | "zipfian" => Ok(()),
        other => Err(HotIndexError::Config(format!(
            "unsupported access pattern: {other}; expected sequential, uniform, or zipfian"
        ))),
    }
}

fn validate_concurrency(iterations: usize, concurrency: usize) -> Result<()> {
    if concurrency == 0 {
        return Err(HotIndexError::Config(
            "--concurrency must be greater than zero".to_string(),
        ));
    }
    if iterations > 0 && concurrency > iterations {
        return Err(HotIndexError::Config(format!(
            "--concurrency ({concurrency}) must be <= --iterations ({iterations})"
        )));
    }
    Ok(())
}

struct QueryWorkerInput<'a, E: StorageEngine> {
    engine: &'a E,
    corpus: &'a [QueryCorpusRecord],
    iterations: usize,
    concurrency: usize,
    worker_idx: usize,
    access_pattern: &'a str,
    seed: &'a str,
    rate_qps: Option<f64>,
    started: Instant,
}

struct QueryWorkerResult {
    operations: u64,
    errors: u64,
    histogram: Histogram<u64>,
}

fn measure_query_worker<E: StorageEngine>(
    input: QueryWorkerInput<'_, E>,
) -> Result<QueryWorkerResult> {
    let mut rng = DeterministicRng::new(seed_from_string(&format!(
        "{}:{}",
        input.seed, input.worker_idx
    )));
    let mut histogram = new_latency_histogram()?;
    let mut errors = 0_u64;
    let mut operations = 0_u64;

    let mut idx = input.worker_idx;
    while idx < input.iterations {
        let scheduled_start = input
            .rate_qps
            .map(|rate| input.started + Duration::from_secs_f64(idx as f64 / rate));
        if let Some(scheduled_start) = scheduled_start {
            let now = Instant::now();
            if scheduled_start > now {
                std::thread::sleep(scheduled_start - now);
            }
        }

        let record_idx = query_index(input.access_pattern, idx, input.corpus.len(), &mut rng);
        let record = &input.corpus[record_idx];
        let op_started = Instant::now();
        if execute_query(input.engine, record).is_err() {
            errors += 1;
        }
        let op_finished = Instant::now();
        let latency = if let Some(scheduled_start) = scheduled_start {
            op_finished.duration_since(scheduled_start)
        } else {
            op_finished.duration_since(op_started)
        };
        record_latency(&mut histogram, latency)?;
        operations += 1;
        idx += input.concurrency;
    }

    Ok(QueryWorkerResult {
        operations,
        errors,
        histogram,
    })
}

fn finish_query_results(results: Vec<QueryWorkerResult>, elapsed: Duration) -> Result<BenchResult> {
    let mut merged = new_latency_histogram()?;
    let mut operations = 0_u64;
    let mut errors = 0_u64;

    for result in results {
        operations += result.operations;
        errors += result.errors;
        merged.add(&result.histogram).map_err(|error| {
            HotIndexError::Config(format!("failed to merge latency histogram: {error}"))
        })?;
    }

    Ok(finish_result(operations, errors, elapsed, merged))
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
            if engine.get_tx(version)?.is_none() {
                return Err(HotIndexError::Config(format!(
                    "get_tx_by_version returned no row for version {version}"
                )));
            }
        }
        QueryKind::MultiGetTxVersions => {
            if record.tx_versions.is_empty() {
                return Err(HotIndexError::Config(
                    "multi_get_tx_versions missing tx_versions".to_string(),
                ));
            }
            let rows = engine.multi_get_txs(&record.tx_versions)?;
            let missing = record
                .tx_versions
                .iter()
                .zip(rows.iter())
                .filter_map(|(version, row)| row.is_none().then_some(version.to_string()))
                .take(5)
                .collect::<Vec<_>>();
            if !missing.is_empty() || rows.len() != record.tx_versions.len() {
                return Err(HotIndexError::Config(format!(
                    "multi_get_tx_versions returned missing rows for versions {}",
                    missing.join(",")
                )));
            }
        }
        QueryKind::MarketFillScan => {
            let market_id = required(record.market_id.as_deref(), "market_id")?;
            let rows = engine.scan_market_fills(market_id, positive_limit(record)?)?;
            if rows.is_empty() {
                return Err(HotIndexError::Config(format!(
                    "scan_market_fills returned no rows for market_id {market_id}"
                )));
            }
        }
        QueryKind::AccountFillScan => {
            let account = required(record.account.as_deref(), "account")?;
            let rows = engine.scan_account_fills(account, positive_limit(record)?)?;
            if rows.is_empty() {
                return Err(HotIndexError::Config(format!(
                    "scan_account_fills returned no rows for account {account}"
                )));
            }
        }
        QueryKind::BuilderCodeFillScan => {
            let builder_addr = required(record.builder_addr.as_deref(), "builder_addr")?;
            let rows = engine.scan_builder_code_fills(builder_addr, positive_limit(record)?)?;
            if rows.is_empty() {
                return Err(HotIndexError::Config(format!(
                    "scan_builder_code_fills returned no rows for builder_addr {builder_addr}"
                )));
            }
        }
        QueryKind::BuilderCodeVolume => {
            let builder_addr = required(record.builder_addr.as_deref(), "builder_addr")?;
            if engine
                .get_builder_code_volume(builder_addr, TimeWindow::H24)?
                .is_none()
            {
                return Err(HotIndexError::Config(format!(
                    "get_builder_code_volume returned no row for builder_addr {builder_addr}"
                )));
            }
        }
        QueryKind::MixedDashboard => {
            return Err(HotIndexError::Config(
                "mixed_dashboard query_kind is not directly executable; expand it into concrete query records".to_string(),
            ));
        }
    }
    Ok(())
}

fn positive_limit(record: &QueryCorpusRecord) -> Result<usize> {
    let limit = record.limit.unwrap_or(100);
    if limit == 0 {
        return Err(HotIndexError::Config(
            "query corpus record limit must be greater than zero".to_string(),
        ));
    }
    Ok(limit)
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
    let mut histogram = new_latency_histogram()?;
    let mut errors = 0_u64;
    let started = Instant::now();
    rows.replay_each(limit, |op| {
        let op_started = Instant::now();
        if op(engine).is_err() {
            errors += 1;
        }
        record_latency(&mut histogram, op_started.elapsed())?;
        Ok(())
    })?;
    Ok(finish_result(
        limit as u64,
        errors,
        started.elapsed(),
        histogram,
    ))
}

fn measure_read_under_ingest<E: StorageEngine>(
    engine: &E,
    rows: &IngestRows,
    corpus: &[QueryCorpusRecord],
    config: QueryMeasureConfig<'_>,
) -> Result<BenchResult> {
    std::thread::scope(|scope| {
        let ingest = scope.spawn(|| rows.replay_limit(engine, rows.total_rows()));
        let result = measure_queries(engine, corpus, config)?;
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
    histogram: Histogram<u64>,
) -> BenchResult {
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
            method: "hdr_histogram".to_string(),
            significant_figures: 3,
            p50: histogram.value_at_quantile(0.50),
            p95: histogram.value_at_quantile(0.95),
            p99: histogram.value_at_quantile(0.99),
            p999: histogram.value_at_quantile(0.999),
            max: histogram.max(),
        },
    }
}

fn new_latency_histogram() -> Result<Histogram<u64>> {
    Histogram::<u64>::new(3).map_err(|error| {
        HotIndexError::Config(format!("failed to create latency histogram: {error}"))
    })
}

fn record_latency(histogram: &mut Histogram<u64>, duration: Duration) -> Result<()> {
    let micros = duration.as_micros().min(u128::from(u64::MAX)) as u64;
    histogram
        .record(micros.max(1))
        .map_err(|error| HotIndexError::Config(format!("failed to record latency: {error}")))
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

struct ReportInput<'a> {
    manifest: &'a DatasetManifest,
    dataset: &'a Path,
    backend: &'a str,
    benchmark_class: &'a str,
    workload: &'a str,
    iterations: usize,
    warmup: usize,
    concurrency: usize,
    access_pattern: &'a str,
    seed: &'a str,
    rate_qps: Option<f64>,
    query_corpus: Option<QueryCorpusReport>,
    started_at: &'a str,
    result: BenchResult,
    checksums: Option<Vec<CfChecksum>>,
    checksum_status: &'a str,
    storage_state: StorageStateReport,
    expected_checksum: Option<&'a Path>,
    publishable_candidate: bool,
}

fn build_report(input: ReportInput<'_>) -> Result<BenchmarkReport> {
    let checksum = build_checksum_report(
        input.checksum_status,
        input.checksums,
        input.expected_checksum,
    )?;
    Ok(BenchmarkReport {
        report_version: 1,
        started_at: input.started_at.to_string(),
        methodology_status: if input.publishable_candidate {
            "publishable_candidate"
        } else {
            "engineering_smoke_not_publishable"
        }
        .to_string(),
        benchmark_class: input.benchmark_class.to_string(),
        backend: input.backend.to_string(),
        workload: input.workload.to_string(),
        iterations: input.iterations,
        warmup: input.warmup,
        concurrency: input.concurrency,
        access_pattern: input.access_pattern.to_string(),
        seed: input.seed.to_string(),
        timing_mode: if input.rate_qps.is_some() {
            "open_loop_rate".to_string()
        } else {
            "closed_loop".to_string()
        },
        rate_qps: input.rate_qps,
        query_corpus: input.query_corpus,
        dataset: DatasetReport {
            dataset_id: input.manifest.dataset_id.0.clone(),
            schema_version: input.manifest.schema_version,
            network: input.manifest.network.as_str().to_string(),
            start_version: input.manifest.start_version,
            end_version: input.manifest.end_version,
            raw_transaction_count: input.manifest.raw_transaction_count,
            decibel_event_count: input.manifest.decibel_event_count,
            fill_count: input.manifest.fill_count,
            builder_code_row_count: input.manifest.builder_code_row_count,
            manifest_sha256: sha256_file(&input.dataset.join("manifest.json"))?,
        },
        checksum,
        environment: EnvironmentReport::capture(input.dataset),
        storage: input.storage_state,
        gate: ReportGate::default(),
        result: input.result,
        disclaimer: if input.publishable_candidate {
            "publishable candidate; release use still requires package gate, checksum-passed backend comparison, and archived dataset evidence"
        } else {
            "engineering smoke benchmark only; publishable claims require a pinned release build, checksum-passed backend comparison, and documented environment/storage state"
        }
        .to_string(),
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

fn validate_manifest_schema_version(manifest: &DatasetManifest) -> Result<()> {
    if manifest.schema_version != LOGICAL_SCHEMA_VERSION {
        return Err(HotIndexError::Config(format!(
            "dataset schema_version {} is unsupported; expected {}. Rebuild normalized artifacts from raw data.",
            manifest.schema_version, LOGICAL_SCHEMA_VERSION
        )));
    }
    Ok(())
}

const REQUIRED_NORMALIZED_ARTIFACTS: &[&str] = &[
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

fn validate_benchmark_dataset(root: &Path, manifest: &DatasetManifest) -> Result<()> {
    validate_manifest_schema_version(manifest)?;
    validate_manifest_range(manifest)?;
    validate_required_manifest_artifacts(manifest)?;
    validate_manifest_hashes(root, manifest)
}

fn validate_manifest_range(manifest: &DatasetManifest) -> Result<()> {
    let end_version = manifest.end_version.ok_or_else(|| {
        HotIndexError::Config(
            "benchmark dataset end_version is open; use a bounded recorded range".to_string(),
        )
    })?;
    if end_version < manifest.start_version {
        return Err(HotIndexError::Config(format!(
            "benchmark dataset end_version {} is before start_version {}",
            end_version, manifest.start_version
        )));
    }
    if manifest.raw_transaction_count == 0 {
        return Err(HotIndexError::Config(
            "benchmark dataset raw_transaction_count is zero".to_string(),
        ));
    }
    let expected_count = end_version
        .checked_sub(manifest.start_version)
        .and_then(|span| span.checked_add(1))
        .ok_or_else(|| {
            HotIndexError::Config("benchmark dataset version range overflows u64".to_string())
        })?;
    if manifest.raw_transaction_count != expected_count {
        return Err(HotIndexError::Config(format!(
            "benchmark dataset raw_transaction_count {} does not match bounded version range {}..{} (expected {})",
            manifest.raw_transaction_count, manifest.start_version, end_version, expected_count
        )));
    }
    Ok(())
}

fn validate_required_manifest_artifacts(manifest: &DatasetManifest) -> Result<()> {
    if manifest.hashes.sha256.is_empty() {
        return Err(HotIndexError::Config(
            "dataset manifest has no sha256 artifact map".to_string(),
        ));
    }
    for relative in REQUIRED_NORMALIZED_ARTIFACTS {
        if !manifest.hashes.sha256.contains_key(*relative) {
            return Err(HotIndexError::Config(format!(
                "dataset manifest is missing sha256 for required artifact {relative}"
            )));
        }
    }
    if !matches!(manifest.raw_encoding, DatasetEncoding::Synthetic)
        && !manifest
            .hashes
            .sha256
            .keys()
            .any(|relative| relative.starts_with("raw/"))
    {
        return Err(HotIndexError::Config(
            "dataset manifest is missing sha256 for raw artifacts".to_string(),
        ));
    }
    Ok(())
}

fn validate_manifest_hashes(root: &Path, manifest: &DatasetManifest) -> Result<()> {
    for (relative, expected) in &manifest.hashes.sha256 {
        validate_manifest_artifact_key(relative)?;
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

fn validate_manifest_artifact_key(relative: &str) -> Result<()> {
    let path = Path::new(relative);
    if path.is_absolute()
        || relative.is_empty()
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(HotIndexError::Config(format!(
            "dataset manifest contains invalid artifact path: {relative}"
        )));
    }
    Ok(())
}

fn validate_query_corpus_hash(
    manifest: &DatasetManifest,
    query_corpus: &QueryCorpusReport,
) -> Result<()> {
    match manifest.hashes.sha256.get(&query_corpus.relative_path) {
        Some(expected) if expected == &query_corpus.sha256 => Ok(()),
        Some(expected) => Err(HotIndexError::Config(format!(
            "query corpus sha256 mismatch for {}: manifest expected {}, got {}",
            query_corpus.relative_path, expected, query_corpus.sha256
        ))),
        None => Err(HotIndexError::Config(format!(
            "dataset manifest is missing sha256 for query corpus {}",
            query_corpus.relative_path
        ))),
    }
}

fn enforce_report_gate(failures: &[String], allow_failures: bool) -> Result<()> {
    if failures.is_empty() {
        return Ok(());
    }
    if allow_failures {
        eprintln!(
            "benchmark gate bypassed by --allow-failures: {}",
            failures.join("; ")
        );
        return Ok(());
    }
    Err(HotIndexError::Config(format!(
        "benchmark gate failed: {}; report was written for inspection",
        failures.join("; ")
    )))
}

fn report_gate_failures(report: &BenchmarkReport) -> Vec<String> {
    let mut failures = Vec::new();
    if report.result.operations == 0 {
        failures.push("result.operations is zero".to_string());
    }
    if report.result.errors > 0 {
        failures.push(format!("result.errors is {}", report.result.errors));
    }
    if !report.checksum.status.eq_ignore_ascii_case("pass") {
        failures.push(format!("checksum.status is {}", report.checksum.status));
    }
    if needs_query_corpus(report) && report.query_corpus.is_none() {
        failures.push("query_corpus is missing".to_string());
    }
    if needs_open_loop(report)
        && (!report.timing_mode.eq_ignore_ascii_case("open_loop_rate") || report.rate_qps.is_none())
    {
        failures.push("serving/read-under-ingest benchmark must use open-loop --rate".to_string());
    }
    if report.storage.path.is_empty() {
        failures.push("storage.path is empty".to_string());
    }
    if report.storage.cache.state == "unspecified" {
        failures.push("storage.cache.state is unspecified".to_string());
    }
    if needs_compaction(report) && !report.storage.compaction.performed {
        failures.push(format!(
            "{} benchmark must pass --compact-before-run",
            report.backend
        ));
    }
    if report
        .storage
        .backend_options
        .get("backend")
        .map(String::as_str)
        != Some(report.backend.as_str())
    {
        failures.push("storage.backend_options.backend does not match report backend".to_string());
    }
    if report.environment.cpu_model.is_empty() || report.environment.cpu_model == "unknown" {
        failures.push("environment.cpu_model is unknown".to_string());
    }
    if report.environment.kernel.is_empty() || report.environment.kernel == "unknown" {
        failures.push("environment.kernel is unknown".to_string());
    }
    if report.environment.filesystem.is_empty() || report.environment.filesystem == "unknown" {
        failures.push("environment.filesystem is unknown".to_string());
    }
    if report.environment.mount_point.is_empty() || report.environment.mount_point == "unknown" {
        failures.push("environment.mount_point is unknown".to_string());
    }
    if report.environment.git_sha.is_empty() || report.environment.git_sha == "unknown" {
        failures.push("environment.git_sha is unknown".to_string());
    }
    if report.environment.ulimit_open_files.is_empty()
        || report.environment.ulimit_open_files == "unknown"
    {
        failures.push("environment.ulimit_open_files is unknown".to_string());
    }
    if report.environment.total_memory_bytes.is_none() {
        failures.push("environment.total_memory_bytes is null".to_string());
    }
    match report
        .environment
        .env
        .get("rust_profile")
        .map(String::as_str)
    {
        Some("release") => {}
        Some(profile) => failures.push(format!(
            "environment.env.rust_profile is {profile}; use a release binary"
        )),
        None => failures.push("environment.env.rust_profile is missing".to_string()),
    }
    failures
}

fn needs_query_corpus(report: &BenchmarkReport) -> bool {
    matches!(
        report.benchmark_class.as_str(),
        "serving" | "read-under-ingest"
    )
}

fn needs_open_loop(report: &BenchmarkReport) -> bool {
    matches!(
        report.benchmark_class.as_str(),
        "serving" | "read-under-ingest"
    )
}

fn needs_compaction(report: &BenchmarkReport) -> bool {
    !matches!(report.backend.as_str(), "memory")
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
    #[serde(default = "default_concurrency")]
    concurrency: usize,
    access_pattern: String,
    seed: String,
    #[serde(default)]
    timing_mode: String,
    #[serde(default)]
    rate_qps: Option<f64>,
    query_corpus: Option<QueryCorpusReport>,
    dataset: DatasetReport,
    checksum: ChecksumReport,
    environment: EnvironmentReport,
    #[serde(default)]
    storage: StorageStateReport,
    #[serde(default)]
    gate: ReportGate,
    result: BenchResult,
    disclaimer: String,
}

fn default_concurrency() -> usize {
    1
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
    schema_version: u32,
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
    #[serde(default)]
    cpu_model: String,
    #[serde(default)]
    total_memory_bytes: Option<u64>,
    #[serde(default)]
    kernel: String,
    #[serde(default)]
    filesystem: String,
    #[serde(default)]
    mount_point: String,
    #[serde(default)]
    git_sha: String,
    #[serde(default)]
    git_dirty: bool,
    #[serde(default)]
    ulimit_open_files: String,
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
            cpu_model: cpu_model(),
            total_memory_bytes: total_memory_bytes(),
            kernel: kernel_release(),
            filesystem: filesystem_type(dataset),
            mount_point: mount_point(dataset),
            git_sha: git_sha(),
            git_dirty: git_dirty(),
            ulimit_open_files: ulimit_open_files(),
            storage_path: dataset.display().to_string(),
            env,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StorageStateReport {
    path: String,
    compaction: CompactionReport,
    cache: CacheReport,
    backend_options: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CompactionReport {
    requested: bool,
    performed: bool,
    status: String,
    note: String,
}

impl Default for CompactionReport {
    fn default() -> Self {
        Self {
            requested: false,
            performed: false,
            status: "unspecified".to_string(),
            note: String::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheReport {
    state: String,
    clear_command: Option<String>,
    note: String,
}

impl Default for CacheReport {
    fn default() -> Self {
        Self {
            state: "unspecified".to_string(),
            clear_command: None,
            note: "page cache state was not controlled by the benchmark runner".to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ReportGate {
    status: String,
    allow_failures: bool,
    failures: Vec<String>,
}

impl ReportGate {
    fn from_failures(failures: &[String], allow_failures: bool) -> Self {
        let status = if failures.is_empty() {
            "pass"
        } else if allow_failures {
            "bypassed"
        } else {
            "fail"
        };
        Self {
            status: status.to_string(),
            allow_failures,
            failures: failures.to_vec(),
        }
    }
}

impl Default for ReportGate {
    fn default() -> Self {
        Self {
            status: "unknown".to_string(),
            allow_failures: false,
            failures: Vec::new(),
        }
    }
}

fn memory_storage_state(
    path: &Path,
    compaction_requested: bool,
    cache_state: &str,
    cache_clear_command: Option<&str>,
) -> StorageStateReport {
    let mut backend_options = BTreeMap::new();
    backend_options.insert("backend".to_string(), "memory".to_string());
    backend_options.insert("durability".to_string(), "in_memory_only".to_string());
    StorageStateReport {
        path: path.display().to_string(),
        compaction: CompactionReport {
            requested: compaction_requested,
            performed: false,
            status: "not_applicable".to_string(),
            note: "memory backend has no LSM compaction".to_string(),
        },
        cache: cache_report(cache_state, cache_clear_command),
        backend_options,
    }
}

#[cfg(feature = "rocksdb")]
fn rocksdb_storage_state(
    path: &Path,
    compaction_requested: bool,
    compaction_performed: bool,
    cache_state: &str,
    cache_clear_command: Option<&str>,
) -> StorageStateReport {
    let mut backend_options = BTreeMap::new();
    backend_options.insert("backend".to_string(), "rocksdb".to_string());
    backend_options.insert("create_if_missing".to_string(), "true".to_string());
    backend_options.insert(
        "create_missing_column_families".to_string(),
        "true".to_string(),
    );
    backend_options.insert("cargo_feature".to_string(), "rocksdb".to_string());
    backend_options.insert("compression".to_string(), "lz4".to_string());
    StorageStateReport {
        path: path.display().to_string(),
        compaction: CompactionReport {
            requested: compaction_requested,
            performed: compaction_performed,
            status: if compaction_performed {
                "performed_before_measurement".to_string()
            } else {
                "not_requested".to_string()
            },
            note: if compaction_performed {
                "compact_range was called for every logical column family before measured operations"
                    .to_string()
            } else {
                "database was measured without benchmark-triggered compaction".to_string()
            },
        },
        cache: cache_report(cache_state, cache_clear_command),
        backend_options,
    }
}

fn cache_report(cache_state: &str, cache_clear_command: Option<&str>) -> CacheReport {
    CacheReport {
        state: cache_state.to_string(),
        clear_command: cache_clear_command.map(str::to_string),
        note: match cache_state {
            "warm" => "page cache was intentionally left warm or prewarmed outside the runner",
            "cold-cleared" => "caller reported that OS page cache was cleared before the benchmark",
            _ => "page cache state was not controlled by the benchmark runner",
        }
        .to_string(),
    }
}

#[cfg(feature = "rocksdb")]
fn maybe_compact_rocksdb(engine: &RocksDbEngine, compact_before_run: bool) -> Result<bool> {
    if compact_before_run {
        engine.compact_all()?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn validate_cache_state(cache_state: &str) -> Result<()> {
    match cache_state {
        "unspecified" | "warm" | "cold-cleared" => Ok(()),
        other => Err(HotIndexError::Config(format!(
            "unsupported --cache-state {other}; expected unspecified, warm, or cold-cleared"
        ))),
    }
}

fn cpu_model() -> String {
    if let Some(value) = command_stdout("sysctl", &["-n", "machdep.cpu.brand_string"]) {
        return value;
    }
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        for line in cpuinfo.lines() {
            if let Some((_, value)) = line.split_once(':') {
                if line.starts_with("model name") {
                    let trimmed = value.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_string();
                    }
                }
            }
        }
    }
    "unknown".to_string()
}

fn total_memory_bytes() -> Option<u64> {
    if let Some(value) = command_stdout("sysctl", &["-n", "hw.memsize"]) {
        if let Ok(bytes) = value.parse::<u64>() {
            return Some(bytes);
        }
    }
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in meminfo.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
            return Some(kb.saturating_mul(1024));
        }
    }
    None
}

fn kernel_release() -> String {
    command_stdout("uname", &["-sr"]).unwrap_or_else(|| "unknown".to_string())
}

fn filesystem_type(path: &Path) -> String {
    if let Some(value) = filesystem_type_from_mount(path) {
        return value;
    }
    let path = path.to_string_lossy();
    command_stdout("stat", &["-f", "-c", "%T", &path]).unwrap_or_else(|| "unknown".to_string())
}

fn mount_point(path: &Path) -> String {
    let path = path.to_string_lossy();
    let Some(output) = command_stdout("df", &["-P", &path]) else {
        return "unknown".to_string();
    };
    output
        .lines()
        .last()
        .and_then(|line| line.split_whitespace().last())
        .unwrap_or("unknown")
        .to_string()
}

fn filesystem_type_from_mount(path: &Path) -> Option<String> {
    let mount = mount_point(path);
    if mount == "unknown" {
        return None;
    }
    let output = command_stdout("mount", &[])?;
    let needle = format!(" on {mount} (");
    for line in output.lines() {
        let Some(rest) = line.split_once(&needle).map(|(_, rest)| rest) else {
            continue;
        };
        let fs_type = rest.split([',', ')']).next()?.trim();
        if !fs_type.is_empty() {
            return Some(fs_type.to_string());
        }
    }
    None
}

fn git_sha() -> String {
    env::var("GITHUB_SHA")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| command_stdout("git", &["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string())
}

fn git_dirty() -> bool {
    command_stdout("git", &["status", "--porcelain", "--untracked-files=no"])
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn ulimit_open_files() -> String {
    command_stdout("sh", &["-c", "ulimit -n"]).unwrap_or_else(|| "unknown".to_string())
}

fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        None
    } else {
        Some(stdout)
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
    #[serde(default)]
    method: String,
    #[serde(default)]
    significant_figures: u8,
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

    fn has_flag(&self, name: &str) -> bool {
        self.args.iter().any(|arg| arg == name)
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

    fn optional_f64(&self, name: &str) -> Result<Option<f64>> {
        self.optional_value(name)
            .map(|value| {
                value.parse::<f64>().map_err(|_| {
                    HotIndexError::Config(format!("invalid float for {name}: {value}"))
                })
            })
            .transpose()
    }
}

fn print_usage() {
    eprintln!(
        "usage:
  decibel-hotindex-bench run --dataset <dataset-dir> --engine memory --class serving --workload mixed_market_dashboard --iterations <n> --warmup <n> --concurrency <workers> --access-pattern sequential|uniform|zipfian --seed <seed> --rate <qps> --cache-state unspecified|warm|cold-cleared --out <report.json> [--allow-failures] [--publishable-candidate]
  decibel-hotindex-bench run --features rocksdb --dataset <dataset-dir> --engine rocksdb --db-path <rocksdb-path> --class serving --workload mixed_market_dashboard --iterations <n> --warmup <n> --concurrency <workers> --access-pattern sequential|uniform|zipfian --seed <seed> --rate <qps> --compact-before-run --cache-state unspecified|warm|cold-cleared --cache-clear-command <cmd> --out <report.json> [--allow-failures] [--publishable-candidate]
  decibel-hotindex-bench run --dataset <dataset-dir> --engine memory --class ingest --iterations <n> --warmup <n> --out <report.json> [--allow-failures]
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
    text.push_str(&format!(
        "- schema_version: {}\n",
        first.dataset.schema_version
    ));
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
    text.push_str(&format!("- concurrency: {}\n", first.concurrency));
    text.push_str(&format!(
        "- timing_mode: {}{}\n",
        non_empty(&first.timing_mode, "closed_loop"),
        first
            .rate_qps
            .map(|rate| format!(" rate_qps={rate:.2}"))
            .unwrap_or_default()
    ));
    text.push_str(&format!(
        "- latency_method: {} significant_figures={}\n",
        non_empty(&first.result.latency_us.method, "hdr_histogram"),
        first.result.latency_us.significant_figures
    ));
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
        "- environment: {} {} kernel={} cpu=\"{}\" cores={} memory_bytes={} fs={} mount={} git_sha={} git_dirty={} ulimit_open_files={}\n",
        first.environment.os,
        first.environment.arch,
        non_empty(&first.environment.kernel, "unknown"),
        non_empty(&first.environment.cpu_model, "unknown"),
        first.environment.cpu_parallelism,
        first
            .environment
            .total_memory_bytes
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        non_empty(&first.environment.filesystem, "unknown"),
        non_empty(&first.environment.mount_point, "unknown"),
        non_empty(&first.environment.git_sha, "unknown"),
        first.environment.git_dirty,
        non_empty(&first.environment.ulimit_open_files, "unknown")
    ));
    text.push_str(&format!(
        "- storage_state: path={} compaction={} requested={} performed={} cache={}\n",
        first.storage.path,
        first.storage.compaction.status,
        first.storage.compaction.requested,
        first.storage.compaction.performed,
        first.storage.cache.state
    ));
    text.push_str(&format!(
        "- gate_status: {} failures={}\n",
        first.gate.status,
        first.gate.failures.len()
    ));
    text.push_str("- comparison_scope: same schema, same dataset, same keyset, same workload\n");
    text.push_str(&format!("- disclaimer: {}\n\n", first.disclaimer));
    text.push_str("| class | backend | workload | gate | concurrency | timing | ops | errors | qps | p50_us | p95_us | p99_us | p999_us | checksum |\n");
    text.push_str(
        "| --- | --- | --- | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n",
    );
    for report in reports {
        let timing = if let Some(rate_qps) = report.rate_qps {
            format!(
                "{}@{rate_qps:.2}qps",
                non_empty(&report.timing_mode, "open_loop_rate")
            )
        } else {
            non_empty(&report.timing_mode, "closed_loop").to_string()
        };
        text.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {:.2} | {} | {} | {} | {} | {} |\n",
            report.benchmark_class,
            report.backend,
            report.workload,
            non_empty(&report.gate.status, "unknown"),
            report.concurrency,
            timing,
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
    if first.methodology_status == "publishable_candidate" {
        text.push_str("- Reported numbers are publishable candidates and still require the grant/package gate before external use.\n");
    } else {
        text.push_str("- Reported numbers are engineering smoke results unless produced from a pinned release build and checksum-passed backend comparison.\n");
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use decibel_hotindex_core::{DatasetFileHashes, DatasetId, Network};

    #[test]
    fn execute_query_requires_point_lookup_hits() {
        let engine = MemoryEngine::default();
        let record = tx_query(7);
        let error = execute_query(&engine, &record).unwrap_err();
        assert!(error.to_string().contains("returned no row"));
    }

    #[test]
    fn execute_query_accepts_point_lookup_hit() {
        let engine = MemoryEngine::default();
        engine.put_tx(tx_row(7)).unwrap();

        execute_query(&engine, &tx_query(7)).unwrap();
    }

    #[test]
    fn benchmark_gate_rejects_query_errors() {
        let mut report = passing_report();
        report.result.errors = 2;
        let failures = report_gate_failures(&report);

        assert!(enforce_report_gate(&failures, false).is_err());
        assert!(enforce_report_gate(&failures, true).is_ok());
    }

    #[test]
    fn benchmark_gate_rejects_checksum_fail_and_zero_ops() {
        let mut report = passing_report();
        report.result.operations = 0;
        report.checksum.status = "fail".to_string();

        let failures = report_gate_failures(&report);
        let error = enforce_report_gate(&failures, false).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("result.operations is zero"));
        assert!(message.contains("checksum.status is fail"));
    }

    #[test]
    fn benchmark_gate_rejects_checksum_not_run() {
        let mut report = passing_report();
        report.checksum.status = "not_run".to_string();

        let failures = report_gate_failures(&report);

        assert_eq!(failures, vec!["checksum.status is not_run"]);
    }

    #[test]
    fn benchmark_gate_rejects_closed_loop_serving() {
        let mut report = passing_report();
        report.timing_mode = "closed_loop".to_string();
        report.rate_qps = None;

        let failures = report_gate_failures(&report);

        assert!(failures
            .iter()
            .any(|failure| failure
                == "serving/read-under-ingest benchmark must use open-loop --rate"));
    }

    #[test]
    fn benchmark_gate_rejects_debug_binary() {
        let mut report = passing_report();
        report
            .environment
            .env
            .insert("rust_profile".to_string(), "debug".to_string());

        let failures = report_gate_failures(&report);

        assert!(failures
            .iter()
            .any(|failure| failure.contains("rust_profile is debug")));
    }

    #[test]
    fn benchmark_gate_records_bypassed_status() {
        let failures = vec!["checksum.status is not_run".to_string()];

        let gate = ReportGate::from_failures(&failures, true);

        assert_eq!(gate.status, "bypassed");
        assert!(gate.allow_failures);
        assert_eq!(gate.failures, failures);
    }

    #[test]
    fn finish_result_uses_hdr_histogram_summary() {
        let mut histogram = new_latency_histogram().unwrap();
        record_latency(&mut histogram, Duration::from_micros(10)).unwrap();
        record_latency(&mut histogram, Duration::from_micros(20)).unwrap();

        let result = finish_result(2, 0, Duration::from_millis(1), histogram);

        assert_eq!(result.latency_us.method, "hdr_histogram");
        assert_eq!(result.latency_us.significant_figures, 3);
        assert!(result.latency_us.p95 >= 10);
    }

    #[test]
    fn measure_queries_merges_worker_histograms() {
        let engine = MemoryEngine::default();
        let corpus = (1..=4)
            .map(|version| {
                engine.put_tx(tx_row(version)).unwrap();
                tx_query(version)
            })
            .collect::<Vec<_>>();

        let result = measure_queries(
            &engine,
            &corpus,
            QueryMeasureConfig {
                iterations: 4,
                warmup: 0,
                concurrency: 2,
                access_pattern: "sequential",
                seed: "test",
                rate_qps: None,
            },
        )
        .unwrap();

        assert_eq!(result.operations, 4);
        assert_eq!(result.errors, 0);
        assert_eq!(result.latency_us.method, "hdr_histogram");
    }

    #[test]
    fn benchmark_dataset_rejects_open_range() {
        let mut manifest = base_manifest();
        manifest.end_version = None;

        let error = validate_manifest_range(&manifest).unwrap_err();

        assert!(error.to_string().contains("end_version is open"));
    }

    #[test]
    fn benchmark_dataset_rejects_manifest_hash_mismatch() {
        let root = temp_root("manifest-hash-mismatch");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("normalized")).unwrap();
        std::fs::write(root.join("normalized/txs.ndjson"), "before\n").unwrap();
        let mut manifest = base_manifest();
        manifest.hashes.sha256.insert(
            "normalized/txs.ndjson".to_string(),
            sha256_file(&root.join("normalized/txs.ndjson")).unwrap(),
        );
        std::fs::write(root.join("normalized/txs.ndjson"), "after\n").unwrap();

        let error = validate_manifest_hashes(&root, &manifest).unwrap_err();

        assert!(error.to_string().contains("sha256 mismatch"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn benchmark_dataset_rejects_missing_query_hash() {
        let manifest = base_manifest();
        let query_corpus = QueryCorpusReport {
            workload: "get_tx_by_version".to_string(),
            relative_path: "queries/point_tx_versions.ndjson".to_string(),
            sha256: "0".repeat(64),
            path: PathBuf::from("/tmp/test/queries/point_tx_versions.ndjson"),
        };

        let error = validate_query_corpus_hash(&manifest, &query_corpus).unwrap_err();

        assert!(error
            .to_string()
            .contains("missing sha256 for query corpus"));
    }

    #[test]
    fn mixed_dashboard_requires_real_decibel_rows() {
        let mut manifest = base_manifest();
        manifest.decibel_event_count = 1;
        manifest.fill_count = 0;
        manifest.builder_code_row_count = 1;

        let error =
            ensure_serving_workload_supported(&manifest, "mixed_market_dashboard").unwrap_err();

        assert!(error.to_string().contains("fill_count=0"));

        manifest.fill_count = 1;
        manifest.builder_code_row_count = 0;
        let error =
            ensure_serving_workload_supported(&manifest, "mixed_market_dashboard").unwrap_err();

        assert!(error.to_string().contains("builder_code_row_count=0"));
    }

    fn tx_query(version: u64) -> QueryCorpusRecord {
        QueryCorpusRecord {
            query_kind: QueryKind::GetTxByVersion,
            tx_version: Some(version),
            tx_versions: Vec::new(),
            market_id: None,
            account: None,
            builder_addr: None,
            limit: None,
        }
    }

    fn tx_row(version: u64) -> TxRow {
        TxRow {
            network: Network::Mainnet,
            version,
            tx_hash: format!("tx{version}"),
            block_timestamp_us: 1,
            event_count: 0,
            dataset_id: Some(DatasetId("bench-test".to_string())),
            raw_summary: None,
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "decibel-hotindex-bench-{name}-{}",
            std::process::id()
        ))
    }

    fn base_manifest() -> DatasetManifest {
        DatasetManifest {
            dataset_id: DatasetId("bench-test".to_string()),
            schema_version: LOGICAL_SCHEMA_VERSION,
            network: Network::Mainnet,
            source: "test".to_string(),
            transaction_stream_endpoint: None,
            raw_encoding: DatasetEncoding::Synthetic,
            normalized_encoding: DatasetEncoding::Ndjson,
            start_version: 1,
            end_version: Some(1),
            package_address: "0x0".to_string(),
            orderbook_address: "0x0".to_string(),
            parser_source: Some("test".to_string()),
            parser_commit: Some("test".to_string()),
            captured_at: Some("1970-01-01T00:00:00Z".to_string()),
            raw_transaction_count: 1,
            decibel_event_count: 0,
            fill_count: 0,
            order_count: 0,
            position_count: 0,
            builder_code_row_count: 0,
            hashes: DatasetFileHashes::default(),
        }
    }

    fn passing_report() -> BenchmarkReport {
        let mut env = BTreeMap::new();
        env.insert("rust_profile".to_string(), "release".to_string());

        BenchmarkReport {
            report_version: 1,
            started_at: "1970-01-01T00:00:00Z".to_string(),
            methodology_status: "engineering_smoke_not_publishable".to_string(),
            benchmark_class: "serving".to_string(),
            backend: "memory".to_string(),
            workload: "get_tx_by_version".to_string(),
            iterations: 1,
            warmup: 0,
            concurrency: 1,
            access_pattern: "sequential".to_string(),
            seed: "test".to_string(),
            timing_mode: "open_loop_rate".to_string(),
            rate_qps: Some(1.0),
            query_corpus: Some(QueryCorpusReport {
                workload: "get_tx_by_version".to_string(),
                relative_path: "queries/point_tx_versions.ndjson".to_string(),
                sha256: "0".repeat(64),
                path: PathBuf::from("/tmp/test/queries/point_tx_versions.ndjson"),
            }),
            dataset: DatasetReport {
                dataset_id: "bench-test".to_string(),
                schema_version: LOGICAL_SCHEMA_VERSION,
                network: "mainnet".to_string(),
                start_version: 1,
                end_version: Some(1),
                raw_transaction_count: 1,
                decibel_event_count: 0,
                fill_count: 0,
                builder_code_row_count: 0,
                manifest_sha256: "0".repeat(64),
            },
            checksum: ChecksumReport {
                status: "pass".to_string(),
                logical_cfs: Vec::new(),
            },
            environment: EnvironmentReport {
                os: "test".to_string(),
                arch: "test".to_string(),
                cpu_parallelism: 1,
                cpu_model: "test-cpu".to_string(),
                total_memory_bytes: Some(1),
                kernel: "test-kernel".to_string(),
                filesystem: "test-fs".to_string(),
                mount_point: "/tmp".to_string(),
                git_sha: "0".repeat(40),
                git_dirty: false,
                ulimit_open_files: "1024".to_string(),
                storage_path: "/tmp/test".to_string(),
                env,
            },
            storage: memory_storage_state(Path::new("/tmp/test"), false, "warm", None),
            gate: ReportGate::default(),
            result: BenchResult {
                operations: 1,
                errors: 0,
                elapsed_seconds: 1.0,
                throughput_qps: 1.0,
                latency_us: LatencySummary {
                    method: "hdr_histogram".to_string(),
                    significant_figures: 3,
                    p50: 1,
                    p95: 1,
                    p99: 1,
                    p999: 1,
                    max: 1,
                },
            },
            disclaimer: "test".to_string(),
        }
    }
}
