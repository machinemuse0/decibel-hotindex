# Milestone 5 Plan

Milestone 5 turns the local dataset pipeline into reproducible benchmark artifacts. The benchmark runner remains offline-only: it consumes a local dataset directory, deterministic query corpus files, and a materialized backend path.

## Scope

- Serving benchmark for query corpus workloads.
- Ingest benchmark for normalized-row replay.
- JSON benchmark reports.
- Markdown benchmark summary.
- Dataset manifest sha256, query corpus sha256, checksum status, report gate status, and environment fingerprint in every report.

Out of first-pass scope:

- publishable release-mode hardware benchmark

Release-mode hardware publication remains an M5 follow-up after the offline
runner is stable. The runner now supports HDR histogram latency summaries and
open-loop `--rate` scheduling, query concurrency, environment fingerprinting,
and explicit compaction/cache state. Reports stay marked as engineering smoke
until a pinned release-mode hardware benchmark is produced and checksum-passed
against isolated backend materializations. `gate.status=bypassed` reports are
kept only for diagnosis and must not be used for README, grant, or outreach
performance claims.

## Implemented Commands

Memory serving:

```bash
rtk cargo run -p decibel-hotindex-bench -- run --dataset <dataset> --engine memory --class serving --workload mixed_market_dashboard --iterations 1000 --warmup 100 --rate 1000 --cache-state warm --checksum-status pass --out reports/bench-memory-serving.json
```

RocksDB serving:

```bash
rtk cargo run -p decibel-hotindex-bench --features rocksdb -- run --dataset <dataset> --engine rocksdb --db-path <dataset>/materialized/rocksdb --class serving --workload mixed_market_dashboard --iterations 1000 --warmup 100 --rate 1000 --compact-before-run --cache-state warm --checksum-status pass --out reports/bench-rocksdb-serving.json
```

Memory ingest:

```bash
rtk cargo run -p decibel-hotindex-bench -- run --dataset <dataset> --engine memory --class ingest --iterations 1000 --warmup 100 --cache-state warm --checksum-status pass --out reports/bench-memory-ingest.json
```

RocksDB ingest:

```bash
rtk cargo run -p decibel-hotindex-bench --features rocksdb -- run --dataset <dataset> --engine rocksdb --db-path <dataset>/materialized/rocksdb-bench-ingest --class ingest --iterations 1000 --warmup 100 --compact-before-run --cache-state warm --checksum-status pass --out reports/bench-rocksdb-ingest.json
```

Read-under-ingest:

```bash
rtk cargo run -p decibel-hotindex-bench -- run --dataset <dataset> --engine memory --class read-under-ingest --workload mixed_market_dashboard --iterations 1000 --warmup 100 --rate 1000 --cache-state warm --expected-checksum reports/rocksdb-checksums.json --out reports/bench-memory-rui.json
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
- query concurrency for serving/read-under-ingest
- access pattern and seed label
- checksum status and logical CF checksums. `--expected-checksum <file>` sets status to `pass` or `fail` automatically.
- OS, architecture, CPU model, kernel, parallelism, memory, filesystem, mount point, git sha/dirty bit, ulimit, and storage path
- storage state: backend options, compaction status, cache state, and cache-clear command when claiming cold-cache
- report gate status: `pass`, `fail`, or `bypassed`
- p50/p95/p99/p999/max latency from HDR histogram
- timing mode: closed-loop or open-loop `--rate`
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

- Run release-mode benchmark on a real mainnet dataset after recorder/parser coverage is implemented.
- Add a dedicated cached ToplingDB preflight lane outside the fast RocksDB CI.
  The local macOS/arm64 `topingdb` worktree has produced a full native
  `toplingdb-preflight.ok`, but publishable ToplingDB numbers still require a
  fresh full `scripts/toplingdb-preflight.sh` pass for the exact benchmark
  dataset and package gate.
