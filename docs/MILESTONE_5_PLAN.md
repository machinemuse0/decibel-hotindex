# Milestone 5 Plan

Milestone 5 turns the local dataset pipeline into reproducible benchmark artifacts. The benchmark runner remains offline-only: it consumes a local dataset directory, deterministic query corpus files, and a materialized backend path.

## Scope

- Serving benchmark for query corpus workloads.
- Ingest benchmark for normalized-row replay.
- JSON benchmark reports.
- Markdown benchmark summary.
- Dataset manifest sha256, query corpus sha256, checksum status, and environment fingerprint in every report.

Out of first-pass scope:

- open-loop `--rate` mode
- HDR histogram dependency
- publishable release-mode hardware benchmark

Those remain M5 follow-up tasks after the first offline runner is stable.

## Implemented Commands

Memory serving:

```bash
rtk cargo run -p decibel-hotindex-bench -- run --dataset <dataset> --engine memory --class serving --workload mixed_market_dashboard --iterations 1000 --warmup 100 --checksum-status pass --out reports/bench-memory-serving.json
```

RocksDB serving:

```bash
rtk cargo run -p decibel-hotindex-bench --features rocksdb -- run --dataset <dataset> --engine rocksdb --db-path <dataset>/materialized/rocksdb --class serving --workload mixed_market_dashboard --iterations 1000 --warmup 100 --checksum-status not_run --out reports/bench-rocksdb-serving.json
```

Memory ingest:

```bash
rtk cargo run -p decibel-hotindex-bench -- run --dataset <dataset> --engine memory --class ingest --iterations 1000 --warmup 100 --checksum-status pass --out reports/bench-memory-ingest.json
```

RocksDB ingest:

```bash
rtk cargo run -p decibel-hotindex-bench --features rocksdb -- run --dataset <dataset> --engine rocksdb --db-path <dataset>/materialized/rocksdb-bench-ingest --class ingest --iterations 1000 --warmup 100 --checksum-status not_run --out reports/bench-rocksdb-ingest.json
```

Read-under-ingest:

```bash
rtk cargo run -p decibel-hotindex-bench -- run --dataset <dataset> --engine memory --class read-under-ingest --workload mixed_market_dashboard --iterations 1000 --warmup 100 --expected-checksum reports/rocksdb-checksums.json --out reports/bench-memory-rui.json
```

Summary:

```bash
rtk cargo run -p decibel-hotindex-bench -- summarize --reports reports/bench-memory-serving.json,reports/bench-rocksdb-serving.json,reports/bench-memory-ingest.json,reports/bench-rocksdb-ingest.json --out reports/BENCHMARK_SUMMARY.md
```

## Report Fields

Each JSON report includes:

- `dataset.dataset_id`
- dataset network and version range
- dataset manifest sha256
- query corpus path and sha256 for serving benchmarks
- benchmark class, backend, workload, iterations, warmup
- access pattern and seed label
- checksum status and logical CF checksums. `--expected-checksum <file>` sets status to `pass` or `fail` automatically.
- OS, architecture, parallelism, and storage path
- p50/p95/p99/p999/max latency
- throughput, operation count, and error count
- methodology disclaimer

## Smoke Verification

Fixture dataset:

- dataset root: `/private/tmp/decibel-hotindex-m5-smoke`
- raw events: 64
- query corpus records: 92
- RocksDB replay: 64 tx, 64 events, 13 fills, 13 builder rows

Benchmark smoke:

- memory serving: 200 ops, 0 errors
- RocksDB serving: 200 ops, 0 errors
- memory ingest: 200 ops, 0 errors
- RocksDB ingest: 200 ops, 0 errors
- memory read-under-ingest: 200 ops, 0 errors, checksum pass against RocksDB replay checksum
- memory ingest with `--expected-checksum`: 200 ops, 0 errors, checksum pass

Verification commands:

```bash
rtk cargo fmt --all
rtk cargo check --workspace
rtk cargo test --workspace
rtk cargo check -p decibel-hotindex-bench --features rocksdb
```

## Remaining M5 Work

- Add open-loop `--rate` mode.
- Replace exact latency vector with HDR histogram or equivalent mergeable histogram.
- Capture richer environment fingerprint: git sha, CPU model, memory, filesystem, ulimit, backend options, compaction/cache state.
- Run release-mode benchmark on a real mainnet dataset after recorder is implemented.
