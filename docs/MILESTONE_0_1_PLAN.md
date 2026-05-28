# Milestone 0 + 1 Tracking Plan

Last updated: 2026-05-27

This document expands the first implementation slice from `DEVELOPMENT_PLAN.md` into trackable engineering work. Scope is intentionally narrow: create a buildable Rust workspace, define core data and dataset contracts, implement `StorageEngine`, implement `MemoryEngine`, and prove the contract with tests.

## Scope

Milestone 0 establishes the repository shape and docs. Milestone 1 establishes the data model, dataset manifest, checksum contract, and in-memory storage contract that later dataset replay, RocksDB, ToplingDB, ingest, API, and benchmark code must follow.

Included:

- Rust workspace scaffold.
- Core crates and placeholder binary crates.
- Dataset/admin placeholder crates so data preparation is first-class from day one.
- Config examples.
- Architecture/schema/benchmark/grant docs.
- Dataset layout, benchmark methodology, and spike docs.
- Core data types.
- Dataset manifest and query corpus types.
- Config parser with environment variable expansion.
- Key encoding helpers.
- `StorageEngine` trait.
- Storage checksum contract.
- `MemoryEngine`.
- Shared storage conformance tests.

Excluded for this slice:

- Aptos Transaction Stream connection.
- Decibel event ABI parser.
- Full dataset record/normalize/query-corpus/replay implementation.
- RocksDB implementation.
- ToplingDB implementation.
- REST API endpoints beyond compile-only crate stubs.
- Dashboard.
- Benchmark runner execution.

## Implementation Decisions

- Rust edition: `2021` for compatibility with Aptos-adjacent tooling.
- Workspace crate names:
  - `decibel-hotindex-core`
  - `decibel-hotindex-ingest`
  - `decibel-hotindex-storage`
  - `decibel-dataset`
  - `decibel-admin`
  - `decibel-hotindex-api`
  - `decibel-hotindex-bench`
- Dataset primitives live in `decibel-hotindex-core` so dataset, storage, admin, API, and bench crates share one schema.
- Benchmark runner is offline-only by contract; chain recording belongs to `decibel-dataset`.
- Real chain data is mainnet-first and recorded once into immutable raw `.pb.zst` chunks, then replayed locally into RocksDB and ToplingDB.
- Uncompressed JSON is not allowed for real raw transaction archives; `ndjson` is only for P0 synthetic data or small audit samples.
- Checksum equivalence is a storage contract, not a later benchmark afterthought.
- Core error style: use `anyhow` at application boundaries and keep typed validation helpers where useful.
- Storage values: use strongly typed Rust structs serialized through `serde` later; `MemoryEngine` stores typed rows directly.
- Numeric key ordering: all integers use big-endian encoding.
- Recent-first scan ordering: use `reverse_ts_us = u64::MAX - timestamp_us`.
- Address normalization: return lowercase `0x` + 64 hex chars for Aptos addresses, reject non-hex and overlong addresses.
- ToplingDB: no implementation in this slice; only mention future feature gate in docs.

## File Deliverables

### Root and docs

- [x] `Cargo.toml`
- [x] `.gitignore`
- [x] `README.md`
- [x] `ARCHITECTURE.md`
- [x] `SCHEMA.md`
- [x] `BENCHMARK_PLAN.md`
- [x] `GRANT_PROPOSAL_DRAFT.md`
- [x] `DEVELOPMENT_PLAN.md`
- [x] `docs/PLAN_REVIEW.md`
- [x] `docs/MILESTONE_0_1_PLAN.md`
- [x] `docs/SPIKES.md`
- [x] `docs/DATASET_LAYOUT.md`
- [x] `docs/BENCHMARK_METHODOLOGY.md`

### Config, fixtures, reports, scripts

- [x] `config/example.local.yaml`
- [x] `config/example.testnet.yaml`
- [x] `config/example.mainnet.yaml`
- [x] `fixtures/.gitkeep`
- [x] `datasets/.gitkeep`
- [x] `reports/.gitkeep`
- [x] `scripts/.gitkeep`

### Crates

- [x] `crates/decibel-hotindex-core/Cargo.toml`
- [x] `crates/decibel-hotindex-core/src/lib.rs`
- [x] `crates/decibel-hotindex-core/src/config.rs`
- [x] `crates/decibel-hotindex-core/src/error.rs`
- [x] `crates/decibel-hotindex-core/src/types.rs`
- [x] `crates/decibel-hotindex-core/src/time.rs`
- [x] `crates/decibel-hotindex-core/src/address.rs`
- [x] `crates/decibel-hotindex-storage/Cargo.toml`
- [x] `crates/decibel-hotindex-storage/src/lib.rs`
- [x] `crates/decibel-hotindex-storage/src/engine.rs`
- [x] `crates/decibel-hotindex-storage/src/key.rs`
- [x] `crates/decibel-hotindex-storage/src/memory_engine.rs`
- [x] `crates/decibel-hotindex-storage/src/tests.rs`
- [x] `crates/decibel-dataset/Cargo.toml`
- [x] `crates/decibel-dataset/src/main.rs`
- [x] `crates/decibel-admin/Cargo.toml`
- [x] `crates/decibel-admin/src/main.rs`
- [x] `crates/decibel-hotindex-ingest/Cargo.toml`
- [x] `crates/decibel-hotindex-ingest/src/lib.rs`
- [x] `crates/decibel-hotindex-api/Cargo.toml`
- [x] `crates/decibel-hotindex-api/src/main.rs`
- [x] `crates/decibel-hotindex-bench/Cargo.toml`
- [x] `crates/decibel-hotindex-bench/src/main.rs`

## Task Board

### M0-01: Workspace scaffold

Status: Completed

Goal: make the repo recognizable as a Rust workspace.

Tasks:

- Create root `Cargo.toml` with all seven workspace members.
- Add workspace package metadata and dependency versions.
- Create crate directories and minimal `lib.rs` / `main.rs`.
- Add `.gitignore` for Rust build output, local data, reports, editor files, and secrets.

Acceptance:

- `rtk cargo metadata --format-version 1` succeeds.
- `rtk cargo check --workspace` succeeds with empty stubs.
- `decibel-dataset` and `decibel-admin` compile as placeholder binaries.

### M0-02: Config examples

Status: Completed

Goal: make local/testnet/mainnet run modes concrete before code uses them.

Tasks:

- Add `config/example.local.yaml` using `network: local`, `storage.engine: memory`, and fixture-oriented defaults.
- Add `config/example.testnet.yaml` with Decibel testnet package/orderbook address and Aptos testnet gRPC endpoint.
- Add `config/example.mainnet.yaml` with Decibel mainnet package/orderbook address and Aptos mainnet gRPC endpoint.
- Use `${APTOS_GRPC_AUTH_TOKEN}` placeholder without hardcoding token source.

Acceptance:

- Config files parse as YAML.
- No secret values are committed.

### M0-03: Documentation scaffold

Status: Completed

Goal: make the project legible to grants reviewers and future agents.

Tasks:

- Add README with one-line positioning, non-goals, local run placeholder, and current milestone status.
- Add architecture diagram and boundaries.
- Add schema doc with column families and key rules.
- Add benchmark plan with fairness language.
- Add grant proposal draft with placeholders.
- Add dataset layout doc with manifest and directory schema.
- Add benchmark methodology doc with offline dataset, corpus, warmup, HDR histogram, environment fingerprint, and checksum requirements.
- Add spike doc for ToplingDB binding and Aptos Transaction Stream auth.

Acceptance:

- Docs state this is Decibel-specific infrastructure, not a trading strategy.
- Docs state builder-code metrics are analytics estimates, not official settlement statements.
- Docs state benchmark claims require same schema, dataset, keyset, and workload.
- Docs state benchmark runner must not call Aptos gRPC or download chain data.
- Docs state checksum equivalence is required before publishable cross-backend benchmark claims.

### M0-04: Dependency and data-source spikes

Status: Completed

Goal: avoid passively discovering critical dependency blockers after implementation is already coupled to them.

Tasks:

- Define rust-toplingdb spike command, expected success output, failure modes, and fallback to feature-gated stub.
- Define Aptos mainnet Transaction Stream auth/range-recording spike, expected success output, failure modes, and fallback to fixture-only or testnet-debug path.
- Record spike results in `docs/SPIKES.md` once executed.

Acceptance:

- Both spikes have documented objective, command plan, exit criteria, and fallback before M2 starts.
- A missing token or missing ToplingDB binding does not block M0/M1 completion.

### M1-01: Core config parser

Status: Completed

Goal: load project config consistently across future ingest/API/bench binaries.

Tasks:

- Define `AppConfig`, `AptosConfig`, `DecibelConfig`, `StorageConfig`, `ApiConfig`, and `BenchConfig`.
- Implement `AppConfig::from_path`.
- Expand `${VAR_NAME}` placeholders from environment variables.
- Keep missing required environment values as explicit config errors.
- Add tests for successful load and missing env var behavior.

Acceptance:

- Config tests pass.
- Local config can parse without external secrets.

### M1-02: Core domain types

Status: Completed

Goal: define stable row contracts before storage and parser work.

Tasks:

- Define `Network`.
- Define `DatasetId`.
- Define `DatasetManifest`.
- Define `DatasetFileHashes`.
- Define `DatasetEncoding`.
- Define `DatasetArtifactKind`.
- Define `QueryCorpusRecord`.
- Define `QueryKind`.
- Define `DecibelEventType`.
- Define `DecibelEventPayload`.
- Define `NormalizedEvent`.
- Define `TxRow`.
- Define `FillRow`.
- Define `OrderRow`.
- Define `PositionRow`.
- Define `BuilderAttributionRow`.
- Define `BuilderVolumeRow`.
- Define `ActivityRow`.
- Define `IngestCheckpoint`.
- Define `StorageStats`.
- Define `CfChecksum`.
- Define `TimeWindow`.

Acceptance:

- All public structs derive `Debug`, `Clone`, `Serialize`, and `Deserialize` where appropriate.
- Types avoid floating-point money values; use strings for exact decimal quantities.
- Comments clarify latest-observed semantics where relevant.
- Dataset manifest includes network, source, endpoint, raw/normalized encoding, version range, package/orderbook addresses, parser source/commit, captured_at, counts, and sha256 map.
- Query corpus records are explicit about workload/query kind and are designed to contain hit-capable keys.

### M1-03: Address and time helpers

Status: Completed

Goal: centralize small correctness-sensitive helpers.

Tasks:

- Implement `normalize_aptos_address`.
- Implement `reverse_ts_us`.
- Implement `TimeWindow` parsing for `24h` and `7d`.
- Add unit tests for short, full, uppercase, invalid, and overlong addresses.
- Add unit tests for reverse timestamp ordering.

Acceptance:

- Address normalization returns lowercase `0x` + 64 hex chars.
- Invalid addresses fail deterministically.

### M1-04: Key encoding helpers

Status: Completed

Goal: make future RocksDB/ToplingDB keys deterministic and testable.

Tasks:

- Implement `be_u64`, `be_u32`, and reverse timestamp key helpers.
- Implement key builders for MVP paths:
  - `tx_by_version`
  - `raw_event_by_version_idx`
  - `event_by_type_version`
  - `fills_by_market_time`
  - `fills_by_account_time`
  - `order_by_id`
  - `positions_by_account_market`
  - `builder_code_fills`
  - `ingest_checkpoint`
  - `checksum_logical_cf`
- Add tests proving lexicographic order matches expected numeric/time order.

Acceptance:

- Key tests pass.
- Key helpers are backend-independent.

### M1-05: StorageEngine trait

Status: Completed

Goal: lock the contract all storage backends must implement.

Tasks:

- Define put APIs for tx, normalized event, fill, order, position, builder attribution, and checkpoint.
- Define read APIs for tx, multi-get txs, order by id, positions by account.
- Define scan APIs for market fills, account fills, builder-code fills, market activity.
- Define builder-code volume API.
- Define stats API.
- Define checksum API returning deterministic per-logical-CF hash summaries.
- Keep limit clamping outside storage, but make storage scan methods accept a limit.

Acceptance:

- Trait is `Send + Sync + 'static`.
- Trait returns a shared project result type.
- No backend-specific query is exposed.
- `multi_get_txs(&self, versions: &[u64])` preserves input order; backend-specific batched multi-get optimization remains internal.
- Checksum contract is backend-neutral and can be used by `decibel-admin compare-checksum`.

### M1-06: MemoryEngine implementation

Status: Completed

Goal: provide deterministic in-memory backend for tests and fixture demos.

Tasks:

- Implement `MemoryEngine` with `RwLock`-protected maps.
- Store typed rows directly.
- Maintain secondary indexes needed for scan methods.
- Return recent-first fill scans.
- Compute `BuilderVolumeRow` from stored attribution rows for requested windows.
- Track simple stats counts.
- Implement checksum by serializing logical key/value pairs in deterministic ordering and hashing them.

Acceptance:

- MemoryEngine implements all trait methods.
- Empty scans return empty vectors, not errors.
- Missing tx/order returns `Ok(None)`.
- Checksum output is stable across repeated calls for unchanged data.

### M1-07: Storage conformance tests

Status: Completed

Goal: make future RocksDB/ToplingDB implementations inherit the same behavior tests.

Tasks:

- Create test helper that accepts a `StorageEngine` factory.
- Test tx put/get and multi-get order preservation.
- Test event/fill insertion.
- Test market fill recent-first scan.
- Test account fill recent-first scan.
- Test builder-code fill scan and volume aggregation.
- Test position latest observed replacement.
- Test checkpoint replacement.
- Test stats counts.
- Test deterministic checksum on identical synthetic rows.
- Test checksum changes when data changes.

Acceptance:

- MemoryEngine passes the shared conformance suite.
- Tests use deterministic synthetic rows and timestamps.

### M1-08: Verification pass

Status: Completed

Goal: ensure the first slice is actually runnable.

Tasks:

- Run `rtk cargo fmt --all`.
- Run `rtk cargo check --workspace`.
- Run `rtk cargo test --workspace`.
- Update this tracking document statuses.
- Update `docs/SPIKES.md` with spike status if any spike has been executed.
- Summarize limitations for Milestone 2.

Acceptance:

- All commands pass or failures are documented with exact next fix.

## Execution Order

1. M0-01 workspace scaffold.
2. M0-02 config examples.
3. M0-03 documentation scaffold.
4. M0-04 dependency and data-source spikes.
5. M1-01 config parser.
6. M1-02 core domain types.
7. M1-03 address and time helpers.
8. M1-04 key encoding helpers.
9. M1-05 storage trait.
10. M1-06 MemoryEngine.
11. M1-07 storage conformance tests.
12. M1-08 verification pass.

## Verification Commands

```bash
rtk cargo metadata --format-version 1
rtk cargo fmt --all
rtk cargo check --workspace
rtk cargo test --workspace
```

## Risks

### R1: Over-modeling before parser reality

Mitigation: keep raw event payload and `Unknown(String)` in the model. Avoid exhaustive Decibel ABI modeling in Milestone 1.

### R2: Storage trait becomes too broad

Mitigation: expose only MVP API/benchmark paths. Add future methods only when a Milestone 2+ feature needs them.

### R3: Address normalization mismatch

Mitigation: use one helper across config, parser, storage, and API. Tests must cover short and full Aptos addresses.

### R4: Money precision loss

Mitigation: store quantities as strings in core rows until Decibel-specific decimal scale is confirmed from official parser/reference code.

### R5: Local config accidentally requires network auth

Mitigation: local config defaults to fixture/memory mode and must parse without `APTOS_GRPC_AUTH_TOKEN`.

### R6: Dataset work slips behind storage/API work

Mitigation: create `decibel-dataset` crate stub, dataset manifest types, and dataset layout doc in M0/M1. Treat dataset pipeline as M2, before REST API.

### R7: Cross-backend benchmark lacks equivalence proof

Mitigation: add checksum types and storage checksum contract in M1. `decibel-admin compare-checksum` in M4 will build on this.

### R8: Raw JSON archive becomes too large

Mitigation: real raw chain data uses length-delimited Aptos protobuf plus zstd and version-range chunking. JSON is limited to synthetic smoke data or small audit samples.

## Open Questions

- Should `StorageEngine` own builder volume aggregation, or should a later analytics layer materialize volume rows during ingest? For Milestone 1, `MemoryEngine` can compute from attribution rows; Milestone 3 can decide whether persistent engines compute or store materialized windows.
- Should the API crate be a compile-only stub in Milestone 0, or should `/health` be implemented immediately? Current plan keeps API as compile-only stub until Milestone 4.
- Should `OrderRow` and `PositionRow` be minimal placeholders until Decibel parser fields are confirmed? Current plan uses conservative optional fields plus raw/source metadata.
- Should normalized rows stay as `ndjson.zst` for P1/P2 or move directly to binary replay encoding? Current plan uses `ndjson.zst` first for auditability and records encoding in the manifest so a binary format can be added without changing raw archives.

## Milestone 0 + 1 Done Criteria

- [x] Workspace exists and `rtk cargo metadata --format-version 1` works.
- [x] Core, storage, dataset, admin, ingest, API, and bench crates compile.
- [x] Config examples exist and contain no secrets.
- [x] README, architecture, schema, dataset layout, benchmark methodology, benchmark plan, spike, and grant docs exist.
- [x] Core config parser loads local config.
- [x] Core types include dataset manifest/query corpus records and avoid floating-point money values.
- [x] Address/time helpers are unit tested.
- [x] Key encoding helpers are unit tested.
- [x] `StorageEngine` trait is backend-neutral.
- [x] Storage checksum contract exists.
- [x] `MemoryEngine` implements all trait methods.
- [x] Storage conformance and checksum tests pass for `MemoryEngine`.
- [x] `rtk cargo fmt --all` passes.
- [x] `rtk cargo check --workspace` passes.
- [x] `rtk cargo test --workspace` passes.
