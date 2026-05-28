# Decibel HotIndex Development Plan

本文档基于 `/Users/ssyuan/Downloads/decibel_hotindex_codex_handoff.md`，并结合当前官方资料核对后整理。目标是把 handoff 收敛成可以执行、可以验收、可以逐步交付的开发计划。

## 1. 项目目标

Decibel HotIndex 是一个面向 Aptos Decibel 交易事件的本地高速索引与分析服务层：

- 从 Aptos mainnet Transaction Stream 或 fixture 读取交易事件。
- 先录制、规范化、校验本地 dataset，再让 replay/API/benchmark 消费同一份 dataset。
- 解析 Decibel-specific events，不做 generic Aptos explorer。
- 将 market/account/builder-code 等热路径索引落到统一 `StorageEngine` trait。
- 使用 RocksDB 作为 baseline，ToplingDB 作为 feature-gated optimized backend。
- 暴露 REST API、benchmark runner、grant/demo 文档。

第一阶段交付重点是“可跑、可测、可展示”，而不是全量历史回补或官方结算级对账。

数据导入优先级提前：benchmark runner 不允许在线下载链上数据；主网链上数据只通过 dataset pipeline 尽量录制一次，多次 RocksDB/ToplingDB replay 和 benchmark 都消费同一份本地可复现 raw/normalized dataset。

真实 raw archive 不使用未压缩 JSON。默认保存为 Aptos Transaction protobuf length-delimited stream，再用 zstd 压缩并按 version range 分片。Normalized rows 初期可用 `ndjson.zst` 便于审计；大规模数据再增加二进制 replay 格式。

## 2. 已核对事实

- `aptos-labs/decibel-indexer-example` 是官方 Decibel 事件解析参考，实现为 Rust indexer，并列出了 package/orderbook 地址、starting_version、31 类事件目录。
- Decibel mainnet package/orderbook 地址：`0x50ead22afd6ffd9769e3b3d6e0e64a2a350d68e8b102c4e72e33d0b8cfdfdb06`。
- Decibel testnet package/orderbook 地址：`0xe7da2794b1d8af76532ed95f38bfdf1136abfd8ea3a240189971988a83101b7f`。
- 官方示例 starting versions：testnet `8106081556`，mainnet `4365621793`。
- Aptos Hosted Transaction Stream endpoints：mainnet `grpc.mainnet.aptoslabs.com:443`，testnet `grpc.testnet.aptoslabs.com:443`，devnet `grpc.devnet.aptoslabs.com:443`。
- Decibel Builder Codes 需要用户先 approve max builder fee，builder fee 以 bps 表示，builder address 需要标准化为 64-character hex address。

Reference links:

- https://github.com/aptos-labs/decibel-indexer-example
- https://aptos.dev/build/indexer/txn-stream/aptos-hosted-txn-stream
- https://docs.decibel.trade/quickstart/builder-codes
- https://docs.decibel.trade/llms.txt

## 3. 需要先澄清或特别注意的问题

### P0: Transaction Stream 授权口径

Handoff 中写的是 Aptos Developer Portal token；当前 Aptos Hosted Transaction Stream 文档写的是通过 Geomi 获取 API key，并用 `Authorization: Bearer ...`。实现上不要把 token 来源写死，只保留 `auth_token` / `authorization_bearer` 配置字段，并在 docs 中标注“以官方当前文档为准”。

### P0: ToplingDB Rust binding 可用性

当前计划不应阻塞在 ToplingDB binding。第一阶段必须先完成 MemoryEngine + RocksDbEngine，ToplingDB 放在 `toplingsdb` feature 下。若本机已有 rust-toplingdb，再接真实 backend；否则提供明确 stub 和启用说明。

### P0: Decibel parser 不要从零猜 ABI

必须参考 `aptos-labs/decibel-indexer-example` 的 `event_router.rs`、`events/*`、地址匹配方式和 Move JSON 序列化约定。第一阶段可以只把关键事件映射到 normalized rows，但 raw event 必须保留，Unknown 必须可落盘。

### P1: Builder-code volume 只能是 analytics estimate

Builder code fee/volume 只能表达为 parsed-event analytics，不应称为 official settlement。API response 和 docs 都要包含 source/indexed_range/disclaimer。

### P1: Dashboard 不应拖慢主线

MVP 可以先用 static HTML 或极简 Vite React。只要能展示 markets、builder codes、accounts、bench summary 即可。

### P1: Benchmark 口径必须保守

所有 benchmark 必须使用 same schema、same dataset、same keyset、same workload。不能写 ToplingDB universally faster。

### P0: Dataset pipeline 是前置依赖

不要等 storage/API/dashboard 完成后才准备数据。必须先建立 `record -> normalize -> build-query-corpus -> replay` 的本地 dataset pipeline，并让 benchmark 只消费本地 dataset。任何对外 benchmark report 都必须引用 `dataset_id`、manifest sha256 和 checksum 结果。

## 4. 技术边界

### Crate 边界

- `decibel-hotindex-core`: config、errors、types、time/address helpers。
- `decibel-hotindex-ingest`: fixture ingest、Transaction Stream adapter、Decibel parser adapter、checkpoint progression。
- `decibel-hotindex-storage`: `StorageEngine` trait、key encoding、MemoryEngine、RocksDbEngine、ToplingDbEngine feature gate。
- `decibel-dataset`: dataset manifest、record、normalize、query corpus、replay。
- `decibel-admin`: checksum、compare-checksum、dataset/backend validation utilities。
- `decibel-hotindex-api`: axum REST API、DTO、state、error mapping。
- `decibel-hotindex-bench`: workload generator、latency histogram、report JSON/Markdown。
- `decibel-hotindex-dashboard`: optional minimal dashboard。

### Data flow

```text
Aptos Transaction Stream / synthetic fixture
  -> decibel-dataset record mainnet once
  -> immutable raw local archive (.pb.zst chunks)
  -> normalize Decibel events
  -> build deterministic query corpus
  -> replay into StorageEngine
  -> REST API / benchmark runner / dashboard
```

### Storage rule

All engines must implement the same trait and use the same logical key schema. Benchmark code must call only the trait API.

### Dataset rule

Benchmark code must not call Aptos gRPC or any online data source. It must receive a dataset directory, query corpus, and materialized database path. Data recording, normalization, corpus generation, and replay are separate commands owned by `decibel-dataset`.

RocksDB and ToplingDB must both be materialized from the same saved raw/normalized dataset. A backend comparison is invalid if either side pulled chain data separately.

## 5. Milestone Plan

### Milestone 0: Repo scaffold, docs, and dependency spikes

Goal: repository becomes buildable and self-explanatory.

Tasks:

- Create Rust workspace and crate layout.
- Add `README.md`, `ARCHITECTURE.md`, `SCHEMA.md`, `BENCHMARK_PLAN.md`, `GRANT_PROPOSAL_DRAFT.md`.
- Add `docs/SPIKES.md`, `docs/DATASET_LAYOUT.md`, `docs/BENCHMARK_METHODOLOGY.md`.
- Add `config/example.local.yaml`, `config/example.testnet.yaml`, `config/example.mainnet.yaml`.
- Add `datasets/`, `fixtures/`, `reports/`, `scripts/`.
- Add `.gitignore`.
- Define two early spikes:
  - rust-toplingdb compile/link smoke test.
  - Aptos mainnet Transaction Stream auth/range-recording smoke test.

Acceptance:

- `rtk cargo metadata` works.
- Docs explain scope, non-goals, and local run path.
- Spike goals, exit criteria, and fallback plans are documented before backend/network implementation begins.

### Milestone 1: Core model, MemoryEngine, manifest, and checksum

Goal: define stable types and testable storage contract before touching external streams.

Tasks:

- Implement config parser with environment variable expansion for secrets.
- Implement `Network`, `NormalizedEvent`, `TxRow`, `FillRow`, `OrderRow`, `PositionRow`, `BuilderAttributionRow`, `IngestCheckpoint`.
- Implement `DatasetManifest`, `DatasetId`, `DatasetEncoding`, `DatasetArtifactKind`, `QueryCorpusRecord`, `CfChecksum`, `StorageChecksum`.
- Implement `StorageEngine` trait.
- Add checksum API to storage contract so later backends can prove equivalence.
- Implement key helpers: big-endian integers, reverse timestamp, address normalization.
- Implement `MemoryEngine`.
- Add shared storage conformance tests for put/get/scan/checksum paths.

Acceptance:

- `rtk cargo test --workspace` passes.
- MemoryEngine supports tx, event, market fills, account fills, builder-code fills, checkpoint, stats.
- MemoryEngine checksum is deterministic for identical synthetic data.

### Milestone 2: Dataset pipeline and synthetic fixture

Goal: produce local datasets that can be replayed and benchmarked without network access.

Trackable detail: `docs/MILESTONE_2_PLAN.md`.

Tasks:

- Add `crates/decibel-dataset`.
- Implement `synthetic` command for P0 synthetic smoke dataset.
- Implement `manifest` read/write and sha256 file accounting.
- Implement `normalize` command skeleton for raw/parsed input to normalized rows.
- Implement `build-query-corpus` command that samples only keys present in normalized rows.
- Implement `replay` command that writes normalized rows into `StorageEngine`.
- Implement `record` CLI/checkpoint/retry framework, with mainnet gRPC gated by auth spike result.

Acceptance:

- Synthetic dataset round-trips through `synthetic -> replay(memory)`.
- Every dataset directory has a valid `manifest.json` with dataset_id, counts, and sha256 map.
- Query corpus contains only hit-capable tx versions, market ids, accounts, and builder addresses.
- Benchmark runner remains offline-only by design.

### Milestone 3: Decibel parser adapter and mainnet bounded recording

Goal: turn real or recorded Aptos Decibel events into normalized local datasets.

Tasks:

- Reference or adapt `aptos-labs/decibel-indexer-example` event routing and Move JSON conventions.
- Record `parser_source` and `parser_commit` in `DatasetManifest`.
- Filter events by package/orderbook address from config.
- Map initial key events:
  - `TradeEvent` -> fill + builder attribution when fields exist.
  - `OrderEvent` / bulk order events -> order/activity rows.
  - `PositionUpdateEvent` -> latest observed position row.
  - `MarginCallLog` / `LiquidationEvent` -> liquidation/activity rows.
  - unknown recognized/unrecognized events -> raw normalized event and `unknown_events.ndjson.zst`.
- Output `parse_warnings.log`.
- If Transaction Stream auth spike succeeds, record a bounded mainnet dataset once into raw `.pb.zst` chunks; otherwise keep fixture-only and document blocker.

Acceptance:

- Bounded mainnet or fixture dataset runs `record/fixture -> normalize -> query corpus -> replay(memory)`.
- Unknown payload preservation is tested.
- Manifest captures parser commit and indexed version range.

### Milestone 4: RocksDB baseline, ToplingDB gate, and checksum equivalence

Goal: make persistent backends comparable under the same dataset.

Tasks:

- Add RocksDB dependency and column family initialization.
- Implement all key encoding and prefix scan APIs.
- Add storage conformance tests for RocksDB using temp dirs.
- Add `toplingsdb` feature with either real binding or explicit compile-time stub based on spike result.
- Add `decibel-admin checksum`.
- Add `decibel-admin compare-checksum`.
- Replay the same dataset into Memory/RocksDB/ToplingDB where available and compare per-CF checksums.

Acceptance:

- RocksDB backend passes the same conformance tests as MemoryEngine.
- ToplingDB absence does not break default build.
- Checksum comparison passes for Memory vs RocksDB on the same synthetic dataset.
- Any ToplingDB benchmark requires checksum pass against RocksDB first.

### Milestone 5: Benchmark runner

Goal: produce reproducible, offline, methodology-safe JSON and Markdown reports.

Tasks:

- Consume only local dataset directories and query corpus files.
- Implement workloads:
  - `get_tx_by_version`
  - `scan_market_recent_fills_100`
  - `scan_account_recent_fills_100`
  - `scan_builder_code_fills_100`
  - `get_builder_code_volume_24h`
  - `multi_get_tx_versions_100`
  - `mixed_market_dashboard`
- Implement three benchmark classes:
  - ingest benchmark: replay events -> storage backend.
  - serving benchmark: materialized DB point lookup / prefix scan / multi-get / mixed dashboard.
  - read-under-ingest benchmark: background replay plus foreground query workload.
- Add warmup, duration, concurrency, fixed keyset seed, access pattern, open-loop rate mode.
- Use HDR histogram, environment fingerprint, error classification, and checksum status.
- Generate `reports/bench-*.json` and `reports/BENCHMARK_SUMMARY.md`.

Acceptance:

- Benchmark never calls Aptos gRPC.
- Report explicitly states same schema/dataset/keyset/workload.
- Report includes dataset_id, manifest sha256, checksum pass/fail, and environment fingerprint.

### Milestone 6: REST API

Goal: expose MVP query surface after storage/dataset/bench are stable.

Tasks:

- Implement `GET /health`.
- Implement `GET /stats`.
- Implement `GET /ingest/status`.
- Implement `GET /tx/{version}`.
- Implement `POST /multi-get/txs`.
- Implement `GET /market/{market_id}/fills`.
- Implement `GET /account/{account}/fills`.
- Implement `GET /builder-code/{builder_addr}/fills`.
- Implement `GET /builder-code/{builder_addr}/volume`.
- Implement `GET /bench/summary`.
- Clamp limits by config and map bad params to 400.

Acceptance:

- `rtk cargo run -p decibel-hotindex-api -- --config config/example.local.yaml` starts.
- API smoke tests cover empty-result behavior and fixture/dataset-backed results.
- `/ingest/status` returns dataset_id and indexed range from manifest.
- Builder-code volume response includes analytics-estimate disclaimer.

### Milestone 7: Demo and grant package

Goal: make it submit-ready.

Tasks:

- Add minimal dashboard or static API demo.
- Polish README local flow.
- Add `docs/OUTREACH_MESSAGE.md`.
- Add 3-minute demo script.
- Finalize grant proposal draft.
- Add benchmark reproduction steps: obtain dataset, replay, compare checksum, run bench.

Acceptance:

- A new reviewer can run dataset replay, compare checksum, start API, query sample endpoints, run benchmark, and read grant proposal.

## 6. Recommended First Implementation Slice

Start with Milestones 0 and 1 only, but include dataset primitives in core so Milestone 2 can start immediately. This gives a firm foundation and avoids early network/auth/binding delays.

Trackable detail: `docs/MILESTONE_0_1_PLAN.md`.

Concrete first slice:

1. Create workspace and crates.
2. Add config examples.
3. Add dataset layout, benchmark methodology, and spike docs.
4. Add core data types, including dataset manifest and query corpus records.
5. Add `StorageEngine` and checksum contract.
6. Add `MemoryEngine`.
7. Add key encoding helpers.
8. Add storage conformance and checksum tests.
9. Add initial docs.

Do not implement dashboard or ToplingDB real backend in the first slice. Do run/document ToplingDB and Aptos Transaction Stream spikes early enough to avoid passive waiting.

## 7. Definition of Done for 14-day MVP

- `rtk cargo test --workspace` passes.
- Dataset pipeline can generate P0 synthetic dataset, build query corpus, and replay into storage.
- Bounded fixture or mainnet Decibel events can be normalized into local dataset format.
- RocksDB backend can put/get/scan all MVP key paths.
- ToplingDB is either working or feature-gated with clear docs.
- Cross-backend checksum comparison passes before any benchmark result is treated as publishable.
- API starts from config and returns fixture-backed data.
- Benchmark runner consumes only local dataset/corpus and produces JSON and Markdown summary.
- README includes local run steps.
- Docs include architecture, schema, dataset layout, benchmark methodology, grant draft, outreach message.
- No trading-profit claims.
- No official settlement claims for builder-code analytics.

## 8. Benchmark Methodology Constraints

These constraints are mandatory before any benchmark is used in README, grant material, or outreach:

1. Benchmark runner must not download chain data or call Aptos gRPC.
2. Query keys must come from dataset query corpus, not random non-hit keys.
3. Warmup samples must be discarded before measuring.
4. Access pattern must be explicit: sequential, uniform, or zipfian, with seed recorded.
5. Open-loop `--rate` mode must be available for latency-sensitive reports; closed-loop is only max-throughput mode.
6. HDR histogram or equivalent mergeable histogram must be used for p50/p95/p99/p999.
7. Environment fingerprint must include CPU, core count, memory, filesystem, OS/kernel, backend versions, git sha, ulimit, and dataset_id.
8. Errors must be classified and counted; first representative error should be reported.
9. Compaction/cache state must be controlled or explicitly marked.
10. Multi-get must use backend-native batched multi-get where available and avoid cloning measured keys after timing starts.

## 9. Dataset Reproducibility Constraints

Every dataset directory must include `manifest.json` with:

- `dataset_id`
- `network`
- `source`
- `transaction_stream_endpoint`
- `start_version`
- `end_version`
- `package_address`
- `orderbook_address`
- `parser_source`
- `parser_commit`
- `captured_at`
- event/row counts
- sha256 map for raw, normalized, query, and report files

Raw chain data should use length-delimited Aptos Transaction protobuf plus zstd and version-range chunking. Normalized rows may use `ndjson.zst` for early MVP auditability; larger datasets can add a binary replay encoding recorded in the manifest. Recording should flush incrementally and support resume from `last_success_version`. Synthetic amplification must be labeled as synthetic and must not be mixed into real mainnet benchmark claims.

## 10. Cross-Backend Equivalence

`decibel-admin checksum` and `decibel-admin compare-checksum` are required before benchmark publication.

Rules:

- Checksums are computed per logical column family/index.
- Checksums must be deterministic for the same dataset.
- Benchmark reports must include checksum pass/fail status.
- ToplingDB vs RocksDB performance comparisons are non-publishable unless checksum equivalence passes on the same dataset.
