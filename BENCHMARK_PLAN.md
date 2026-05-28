# Benchmark Plan

Benchmarking is dataset-first and offline.

## Required Inputs

- dataset directory with `manifest.json`
- deterministic query corpus
- materialized backend database
- checksum result for the backend comparison

## Rule

Benchmark runner must not call Aptos gRPC, normalize raw transactions, or generate random non-hit keys during measured runs.

## Workloads

```text
get_tx_by_version
scan_market_recent_fills_100
scan_account_recent_fills_100
scan_builder_code_fills_100
get_builder_code_volume_24h
multi_get_tx_versions_100
mixed_market_dashboard
```

## Benchmark Classes

- ingest benchmark: replay normalized rows into a backend
- serving benchmark: query an already materialized DB
- read-under-ingest benchmark: query while replay continues in the background

## Fairness Requirements

Reports must state:

- same schema
- same dataset
- same keyset
- same workload
- checksum pass/fail
- environment fingerprint

No result should claim ToplingDB is universally faster than RocksDB.
