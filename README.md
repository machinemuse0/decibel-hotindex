# Decibel HotIndex

Decibel HotIndex is a ToplingDB/RocksDB-backed local serving layer and builder analytics gateway for Decibel trading events on Aptos.

The project is Decibel-specific infrastructure, not a generic Aptos explorer and not a trading strategy. It records bounded mainnet transaction-stream data once into an immutable local raw archive, normalizes Decibel events, replays the same dataset into multiple storage backends, and benchmarks the same schema, dataset, keyset, and workload.

## Current Status

Milestone 4 is in progress:

- Rust workspace scaffold
- dataset-first planning
- core config/types/helpers
- MemoryEngine and storage conformance tests
- synthetic dataset generation
- query corpus generation
- memory replay smoke path
- manifest sha256 validation before replay
- fixture JSONL raw dataset generation
- Decibel parser adapter for fixture normalization
- unknown Decibel event payload preservation
- feature-gated RocksDB backend implementation
- feature-gated ToplingDB stub
- admin checksum / compare-checksum commands
- mainnet raw archive format: length-delimited Aptos Transaction protobuf + zstd
- RocksDB/ToplingDB replay from the same saved dataset
- benchmark runner must remain offline-only

## Workspace

```text
crates/
  decibel-hotindex-core/
  decibel-hotindex-storage/
  decibel-dataset/
  decibel-admin/
  decibel-hotindex-ingest/
  decibel-hotindex-api/
  decibel-hotindex-bench/
```

## Local Checks

```bash
rtk cargo metadata --format-version 1
rtk cargo check --workspace
rtk cargo test --workspace
```

## Dataset Smoke

```bash
rtk cargo run -p decibel-dataset -- synthetic --out /private/tmp/decibel-hotindex-smoke --events 24
rtk cargo run -p decibel-dataset -- build-query-corpus --events /private/tmp/decibel-hotindex-smoke/normalized/events.ndjson --out-dir /private/tmp/decibel-hotindex-smoke/queries --seed 42
rtk cargo run -p decibel-dataset -- replay --dataset /private/tmp/decibel-hotindex-smoke --engine memory
```

## Fixture Parser Smoke

```bash
rtk cargo run -p decibel-dataset -- fixture --out /private/tmp/decibel-hotindex-m3-smoke --events 20
rtk cargo run -p decibel-dataset -- normalize --input /private/tmp/decibel-hotindex-m3-smoke/raw/fixture_events.jsonl --out-dir /private/tmp/decibel-hotindex-m3-smoke/normalized --format fixture-jsonl --parser-commit fixture-local
rtk cargo run -p decibel-dataset -- build-query-corpus --events /private/tmp/decibel-hotindex-m3-smoke/normalized/events.ndjson --out-dir /private/tmp/decibel-hotindex-m3-smoke/queries --seed 42
rtk cargo run -p decibel-dataset -- replay --dataset /private/tmp/decibel-hotindex-m3-smoke --engine memory
```

## Data Rule

Mainnet raw data should be pulled as few times as possible, ideally once per bounded dataset. RocksDB and ToplingDB benchmarks must replay from the same saved raw/normalized dataset. Benchmark code must not call Aptos gRPC.

## Non-Goals

- No matching engine.
- No automated trading strategy.
- No wallet/private-key handling.
- No official builder-code settlement claims.
- No universal database performance claims.

Builder-code metrics are analytics estimates from parsed Decibel events, not official settlement statements.

## Planning Docs

- [Development Plan](DEVELOPMENT_PLAN.md)
- [Milestone 0 + 1 Plan](docs/MILESTONE_0_1_PLAN.md)
- [Milestone 2 Plan](docs/MILESTONE_2_PLAN.md)
- [Milestone 3 Plan](docs/MILESTONE_3_PLAN.md)
- [Milestone 4 Plan](docs/MILESTONE_4_PLAN.md)
- [Dataset Layout](docs/DATASET_LAYOUT.md)
- [Benchmark Methodology](docs/BENCHMARK_METHODOLOGY.md)
- [Spikes](docs/SPIKES.md)
