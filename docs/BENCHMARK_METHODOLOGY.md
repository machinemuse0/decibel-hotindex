# Benchmark Methodology

Last updated: 2026-05-27

This document defines the minimum methodology required before Decibel HotIndex benchmark results can be used in README, grant material, outreach, or comparison claims.

## Core Rule

Serving benchmarks are offline. They consume:

- a dataset directory with `manifest.json`
- a deterministic query corpus
- a materialized database path
- a selected storage backend

They must not record Aptos data, call Aptos gRPC, normalize raw transactions, or generate random non-hit keys during the measured benchmark.

## Benchmark Classes

### Ingest Benchmark

Measures replay of normalized events into a storage backend.

Includes:

- write throughput
- write latency
- ingest lag where applicable
- disk growth
- errors by class

Excludes:

- gRPC download time
- raw transaction recording
- event normalization time, unless explicitly running a parser benchmark

### Serving Benchmark

Measures query performance against an already materialized database.

Workloads:

- `get_tx_by_version`
- `scan_market_recent_fills_100`
- `scan_account_recent_fills_100`
- `scan_builder_code_fills_100`
- `get_builder_code_volume_24h`
- `multi_get_tx_versions_100`
- `mixed_market_dashboard`

### Read-Under-Ingest Benchmark

Measures foreground serving workload while background replay appends new normalized rows.

Reports:

- p50/p95/p99/p999 latency
- throughput
- ingest lag
- query error rate
- write/read interference notes

## Query Corpus

All measured queries must come from corpus files produced by `decibel-dataset build-query-corpus`.

The corpus must be derived from normalized dataset rows so query keys are hit-capable. Synthetic negative-key workloads are allowed only when separately named and reported.

Default mixed workload:

```text
35% scan_market_recent_fills_100
20% scan_account_recent_fills_100
15% get_tx_by_version
15% get_builder_code_volume_24h
10% multi_get_tx_versions_100
5% scan_liquidations_100
```

## Timing Rules

- Warmup is required.
- Warmup samples are discarded.
- Histograms are reset after warmup.
- Key cloning, request construction, JSON parsing, and corpus loading must happen before measured timing starts.
- Multi-get should use backend-native batched APIs where available.
- Requests per worker must have a sane lower bound to avoid tiny-sample throughput artifacts.

## Access Patterns

Benchmark config must record:

- `sequential`
- `uniform`
- `zipfian`
- seed

Dashboard-style mixed workloads should default to zipfian unless the report explicitly says otherwise.

## Latency Measurement

Use HDR histogram or an equivalent mergeable histogram.

Required percentiles:

- p50
- p95
- p99
- p999

Closed-loop mode measures max throughput behavior. Open-loop `--rate` mode is required for latency-sensitive reports and should measure latency against scheduled start time to reduce coordinated omission.

## Environment Fingerprint

Reports must include:

- dataset_id
- dataset manifest sha256 summary
- benchmark binary git sha
- CPU model
- core count
- memory
- OS/kernel
- filesystem
- ulimit
- RocksDB version/options
- ToplingDB git revision/options, when used
- storage path
- compaction/cache state

## Compaction and Cache State

Benchmark reports must say whether the database was compacted before the run.

Cold-cache claims are allowed only if page cache was actually cleared and the command/permission is documented. Otherwise report as warm-cache or unspecified-cache.

## Error Reporting

Errors must be counted by class. The report should include the first representative error per class and a bounded sample of additional errors.

Empty results are not automatically errors if the workload is explicitly negative-query. For normal Decibel serving workloads, query corpus keys should hit.

## Cross-Backend Equivalence

Any RocksDB vs ToplingDB comparison must include checksum status:

```text
checksum: pass | fail | not_run
```

Publishable comparisons require `pass` on the same dataset and logical schema. Failed or missing checksum results may be used only as engineering notes.

## Report Header Requirements

Every benchmark summary starts with:

- dataset_id
- network
- version range
- backend
- workload
- query corpus id/hash
- checksum status
- environment fingerprint summary
- disclaimer: same schema, same dataset, same keyset, same workload

