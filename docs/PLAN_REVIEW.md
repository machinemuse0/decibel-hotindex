# Decibel HotIndex 开发计划独立 Review

Last updated: 2026-05-27
Reviewer scope: 对 `DEVELOPMENT_PLAN.md`、`docs/MILESTONE_0_1_PLAN.md` 做独立审阅，结合
`/Users/ssyuan/Downloads/decibel_hotindex_codex_handoff.md`、
`/Users/ssyuan/Downloads/decibel_hotindex_data_prep_addendum.md` 以及姊妹项目
`/Users/ssyuan/work/project/sui-hotstore` 的实际落地经验。

本文是独立建议，不替代现有 plan；最后给出建议的 milestone 重排和落地补丁清单。

---

## 1. 总体判断

现有 `DEVELOPMENT_PLAN.md` 的目标边界、非目标、acceptance 语言（保守 benchmark 口径、
builder-code analytics-estimate 免责声明、ToplingDB feature gate）是合格的，可以作为
对外口径直接复用。

但存在三类结构性问题，按严重度排序：

1. **数据准备的优先级被低估了。** addendum 已经明确，data pipeline 必须独立先做；
   sui-hotstore 的实际教训也印证这一点（200 GB formal snapshot 路线因 pruning/compaction
   时间窗过长，最终只能退回到 "bounded route 1 latest 10000 checkpoints" 的折中方案）。
   当前 plan 把 fixture/真实数据混在 `decibel-hotindex-ingest` 里，并且 benchmark runner
   隐含会触及链上数据；这跟 addendum §7 "benchmark runner 不应下载链上数据"是直接冲突的。
2. **Benchmark 设计没有吸收 sui-hotstore 已经付过学费的 methodology 教训。**
   `docs/hotstore-bench-review.md` 罗列的 12 类问题（假 multi-get、无 warmup、线性访问、
   key clone 污染计时、closed-loop coordinated omission、缺 compaction 控制、缺
   environment fingerprint、缺 HDR histogram、错误静默吞掉、未暴露调优开关 等）在
   decibel-hotindex 当前 plan 里全部没有约束。第一版如果照抄默认实现，benchmark 报告
   出对外不安全。
3. **缺少跨 backend 等价性验证。** sui-hotstore 有 `hotstore-admin compare-checksum`，
   是 "same schema, same dataset" 论断的唯一硬证据。decibel-hotindex 的 plan 里
   `StorageEngine` conformance tests 只覆盖 MemoryEngine 的行为契约，没有
   "RocksDB 与 ToplingDB 在同一数据集上 column-family-level checksum 一致" 这种
   端到端等价证明。

剩下的问题是可以延后优化的，不阻塞 M0/M1。

---

## 2. 与 handoff / addendum 的对照清单

### 2.1 handoff 中已被 plan 正确吸收的部分

- 5 个 crate 边界（core / ingest / storage / api / bench）。
- StorageEngine trait + MemoryEngine + RocksDB baseline + ToplingDB feature gate 的纵深顺序。
- big-endian numeric + `reverse_ts_us = u64::MAX - ts` 的 key 编码约定。
- 把 builder-code analytics 明确标注为 analytics estimate，不是 settlement。
- benchmark 口径用 "same schema, same dataset, same keyset, same workload" 而不是
  "ToplingDB universally faster"。
- 保留 raw event + `Unknown(String)` 兜底 ABI 漂移。

### 2.2 addendum 中尚未被 plan 反映的部分

- **独立的 `decibel-dataset` crate。** addendum §3 明确要求把数据准备拆出来，
  当前 plan 把它放在 `decibel-hotindex-ingest` 里，并把 fixture 处理散在 M2。
- **DatasetManifest（含 sha256 / counts / parser_commit / endpoint / network /
  start_version / end_version）。** plan 完全未提。
- **record / normalize / build-query-corpus / replay 四个独立子命令。**
  plan 只有 "fixture ingest"，没有 record（带 resume/backoff/retry）、
  normalize（独立步骤）、query corpus（从真实数据抽样）三步。
- **dataset 分层 P0–P4（synthetic smoke / testnet historical / mainnet bounded /
  mainnet rolling / synthetic amplification）。** plan 没有显式分层，容易把
  synthetic 数据混进对外 benchmark report。
- **benchmark runner 不下载链上数据。** plan 没有强制 boundary，benchmark crate
  目前在边界上可能会污染数据获取路径。
- **三类 benchmark：ingest / serving / read-under-ingest。** plan 只列了
  serving workloads，缺 ingest throughput 和 read-under-ingest 这两个对
  Decibel "实时" 叙事更关键的项目。
- **query corpus 必须从真实 normalized events 抽样。** plan 没有这条约束，
  benchmark crate 容易随机生成不命中的 key，结果失真。

### 2.3 plan 中存在但 handoff/addendum 都没强约束、需要拍板的事项

- `OrderRow` / `PositionRow` 字段在 Decibel ABI 没有完全锁死之前是否要落 schema？
  当前选择是"保留 optional + raw metadata"，建议保留并加 TODO 锚点到 parser_commit。
- 是否在 ingest 时物化 builder volume window？建议第一版**不物化**，由 MemoryEngine 和
  RocksDB 都从 attribution rows 现算；待 benchmark 暴露热点后再决定是否预聚合。
- API crate 在 M0 是 compile-only stub，还是直接放 `/health`？建议 compile-only stub，
  和 plan 一致。

---

## 3. 关键风险（按"现在不处理后期就贵"排序）

### R-A：benchmark 报告对外不安全

来源：sui-hotstore `docs/hotstore-bench-review.md` 的 12 类问题。
若 decibel-hotindex 不在 M1 trait/接口阶段就把这些约束做出来，等 bench runner
写完再回头改，会同时撼动 trait、所有 backend 实现、key 编码、报告 schema。

最低必须项（建议 trait 与 bench schema 一起锁）：

1. `multi_get` 走 RocksDB `batched_multi_get_cf`，trait 签名用 `&[&[u8]]`，
   避免每次请求 `Vec<Vec<u8>>` clone。
2. WorkloadData 用 arena `Vec<u8>` + `Vec<Range<u32>>`，请求时取 `&[u8]`，
   `Instant::now()` 后**不再**有 clone。
3. `--warmup` 参数和 warmup pass，丢弃 warmup 样本，再重置 histogram。
4. Access pattern 支持 `{sequential, uniform, zipfian}` + `--seed`；
   默认对 dashboard mixed workload 用 zipfian。
5. 用 `hdrhistogram` 取代 `Vec<u64> + sort_unstable`，支持 merge 与压缩 dump。
6. 报告里强制写入 environment fingerprint：CPU model、核数、内存、文件系统、
   kernel、RocksDB 版本、ToplingDB git rev、dataset_id（sha256 来自 manifest）、
   bench 二进制 git sha、ulimit、`vm.swappiness`。
7. Open-loop（`--rate`）模式：用预定 schedule 喂 channel，latency 用
   `now - expected_start_time`；closed-loop 保留为 "max throughput" 模式
   并在报告里标注。
8. Compaction 控制：bench 前 `compact_range_cf(None, None)`；可选 `--drop-caches`
   为冷基线，但只有真的清了 page cache 才允许声明 cold。
9. Error 分类计数 + 第一条 stderr 打印 + 总量上限。
10. Requests / concurrency 下限校验，避免每 worker 只跑几条样本污染 throughput。

### R-B：dataset 不可复现

来源：addendum §10。
对外 benchmark / grant 的复现性，关键不是 RocksDB vs ToplingDB 跑了多少 qps，
而是别人能不能拉到**同一份** dataset。所有 dataset 文件必须：

- 写入 `manifest.json`，含 network、start_version、end_version、endpoint、
  package_address、orderbook_address、parser_source、parser_commit、
  captured_at、counts、sha256 map、dataset_id。
- 落盘单位用 ndjson + zstd，单文件先 fsync 再 atomic rename。
- recorder 支持 resume：last_success_version + 每 N 行 flush。
- benchmark report 引用 dataset_id（不是路径），路径只在 manifest 里。

### R-C：跨 backend 等价性无证据

来源：sui-hotstore admin tooling。
没有 `compare-checksum`，"ToplingDB 在 same schema 上跑出 X qps" 是个无法被
第三方校验的论断。第一版必须给：

- `decibel-admin checksum --db-path ... --backend ...` 输出 per-CF rolling hash。
- `decibel-admin compare-checksum --left ... --right ...` 输出差异列表 + summary。
- 任何 benchmark report 都必须先附 checksum 一致性证明（pass/fail），
  否则视为非正式数据。

### R-D：ToplingDB rust binding 可用性不确定

来源：handoff §9 + sui-hotstore 已实际使用 `topling/rust-toplingdb`。
建议在 M0 阶段就跑一次 "rust-toplingdb 能不能在本机编出来 + 跑个 hello world"
的探针实验（一个独立 spike，不进主线代码），决定是否在 M3 真的接 backend
还是只留 stub。这是 14 天 MVP 里风险最高、回退成本最大的依赖。

### R-E：Aptos Transaction Stream auth / rate limit

来源：addendum §10 + handoff §3 P0。
recorder 写之前先在 spike 里跑通 gRPC 鉴权（Geomi API key + `Authorization: Bearer`），
确认能稳定拉到 bounded range，再写正式 record command。如果第一周 token 拿不到，
全流程会卡死。

### R-F：Decibel ABI 解析覆盖不完整

来源：handoff §8 + decibel-indexer-example 31 类事件。
第一版只确认 fill / order / position / liquidation / builder-code 五类的
normalized 映射；其余事件**必须**走 Unknown 路径，且 `unknown_events.ndjson.zst`
要单独落盘以便后续补类型。不要为了"看起来完整"硬塞猜测字段。

---

## 4. 建议的 Milestone 重排

下面是相对当前 `DEVELOPMENT_PLAN.md` §5 的最小重排。命名沿用 M0–M7，
把 dataset pipeline 提前到 M2，REST API 后移到 M6，原因：dataset pipeline
是后续所有 milestone 的前置依赖（包括 RocksDB 接入、bench、API smoke），
而 REST API 只是把已成型的数据再包一层 HTTP，可以等 storage + bench 稳定后再做。
这跟 sui-hotstore 里 "API serving 仍是 scaffold，benchmark focus 是 DB-level workloads"
的实际优先级一致。

### M0：Repo scaffold + docs（保留）

- 维持现状，但 `docs/PLAN_REVIEW.md`（本文件）也纳入 docs scaffold。
- 在 M0 末尾追加一次 ToplingDB / Transaction Stream auth 的 spike，
  spike 结果记到 `docs/SPIKES.md`，不进主线代码。

### M1：Core model + MemoryEngine（保留，微调）

- 类型层加入 `DatasetManifest`、`DatasetId`、`QueryCorpusRecord`。
  这样 dataset crate 在 M2 可以直接吃 core 类型，避免循环依赖。
- StorageEngine trait 此时就把 `multi_get_*` 改成接 `&[&[u8]]` 的签名，
  不要等 bench runner 才回头改。
- 加入 `StorageChecksum` trait method：返回 per-CF rolling hash，
  MemoryEngine 实现 deterministic ordering。后续 RocksDB / ToplingDB 必须实现。

### M2（新）：Dataset pipeline + synthetic fixture

替代原 M2 "Fixture ingest"。新增 crate：

```
crates/decibel-dataset/
  src/
    lib.rs
    manifest.rs        // DatasetManifest, sha256, dataset_id
    synthetic.rs       // P0 synthetic fixture generator
    normalize.rs       // raw txn -> normalized events
    query_corpus.rs    // 从真实 normalized events 抽样生成 corpus
    replay.rs          // 把 normalized events 灌进 StorageEngine
    record/
      mod.rs
      cli.rs           // record CLI 框架（spike 通过后再接真 gRPC）
      checkpoint.rs    // resume / fsync / last_success_version
      retry.rs         // backoff
```

子命令：

```
cargo run -p decibel-dataset -- synthetic   --out datasets/fixtures/synthetic_smoke
cargo run -p decibel-dataset -- normalize   --input ... --out-dir ...
cargo run -p decibel-dataset -- build-query-corpus --events ... --out-dir ... --seed 42
cargo run -p decibel-dataset -- replay      --engine rocksdb --events ... --db-path ...
cargo run -p decibel-dataset -- record      --network ... --endpoint ... --start-version ... --end-version ...
```

Acceptance：

- synthetic dataset 能 round-trip：synthetic -> replay(memory) -> 同样 events 出来。
- query_corpus 输出的所有 key 在 dataset 里都能命中（不允许出现不存在的 tx version /
  market_id / account / builder_addr）。
- 任何 dataset 目录都必带合法 `manifest.json`，包含 sha256。

### M3：Decibel parser adapter

替代原 M2 后半段。

- 直接 vendor / 引用 `aptos-labs/decibel-indexer-example` 的事件路由与
  Move JSON 序列化约定，记录 `parser_commit` 到 manifest。
- 在 normalize 阶段输出 `parse_warnings.log` 和 `unknown_events.ndjson.zst`。
- 真正接 gRPC 的 record 命令只在 R-E spike 验证通过后启用，否则保持 fixture-only。

Acceptance：

- 给一份 testnet bounded range（来自 R-E spike 拉下来的小样本），
  能完整跑 record -> normalize -> replay -> checksum 一致。

### M4：RocksDB backend + ToplingDB feature gate + 跨 backend checksum

合并原 M3 + 新增 `decibel-admin` 工具：

- RocksDB backend 通过 M1 conformance suite。
- ToplingDB 视 R-D spike 结果，要么接真 backend 并通过同一套 conformance suite，
  要么 explicit feature-gated stub 且 README 有启用指引。
- `decibel-admin checksum` / `decibel-admin compare-checksum` 至少能对
  Memory vs RocksDB、RocksDB vs ToplingDB 做 per-CF 比较，差异 dump 前 N 条 key。
- 在两个 backend 上各 replay 同一个 dataset，compare-checksum 必须 pass。
  这一项进入 M4 acceptance，**没过不允许进 M5**。

### M5：Benchmark runner

替代原 M5，但必须把 §3 R-A 的 10 条最低必须项作为 acceptance 子项：

- `--warmup`、`--access-pattern`、`--seed`、`--rate`（open-loop）、
  `--access-keys <corpus_dir>`（来自 dataset crate 的 query corpus，禁止随机生成）。
- HDR histogram、environment fingerprint、error 分类。
- 三类 benchmark：
  1. **ingest benchmark**：replay events -> storage 的写入吞吐 / 延迟。
  2. **serving benchmark**：已 materialized 数据库的 point lookup / prefix scan /
     真 batched multi-get / dashboard mixed workload。
  3. **read-under-ingest benchmark**：后台 replay 新事件，前台 serving workload，
     报告 p95/p99/p999、ingest lag、query error rate。
- `reports/BENCHMARK_SUMMARY.md` 强制头部包含 dataset_id、checksum pass/fail、
  environment fingerprint。

### M6：REST API

原 M4 后移。理由：到此为止 storage + dataset + bench 已经稳定，API 只是
JSON envelope；提前做没收益。沿用原 M4 的端点清单与 disclaimer。

API 额外约束：

- `/ingest/status` 返回 dataset_id（不是裸路径）。
- 所有 list 类响应附 `indexed_range: {start_version, end_version, captured_at}`，
  来源是当前 dataset manifest。
- `/builder-code/.../volume` 响应里 `source` 固定为 `parsed_decibel_events`，
  `disclaimer` 固定为 `analytics estimate; not official settlement statement`。

### M7：Demo + grant package

合并原 M6。新增要求：

- demo script 必须强调 "dataset_id 公开 + manifest sha256 可校验 + 跨 backend
  checksum 一致 + bench 同 corpus 复现"，这四点是 grant 评审能拍板的关键。
- `docs/OUTREACH_MESSAGE.md` 沿用 handoff §17 模板。
- README 顶层加 "如何复现 benchmark" 三步：拉 dataset、replay、跑 bench。

---

## 5. 对 `docs/MILESTONE_0_1_PLAN.md` 的具体修改建议

M0/M1 的整体结构是合格的，下面是建议增量：

1. M1-02 加：`DatasetManifest`、`DatasetId`、`QueryCorpusRecord`、`StorageChecksum`
   trait 的字段先在 core 定义。原因：dataset crate 不应反向依赖 storage。
2. M1-05 trait：
   - `multi_get_txs` 签名直接定义为 `&[u64]` -> `Vec<Option<TxRow>>`，
     底层 RocksDB 用 `batched_multi_get_cf`。如果未来要给 generic blob multi-get，
     新加一个 `multi_get_blobs(&self, cf: ColumnFamily, keys: &[&[u8]])`，
     而不是把 `Vec<Vec<u8>>` 留在签名里。
   - 加 `checksum(&self, cf: ColumnFamily) -> Result<CfChecksum>`。
3. M1-06 MemoryEngine：实现 `checksum` 时按 (key bytes, value bytes) 排序后做
   rolling SHA-256，给后续 RocksDB / ToplingDB 立标准。
4. M1-07 conformance tests：加 "对同一份 synthetic 数据，replay 完后
   `checksum(cf)` 必须稳定可重复" 的测试，作为后续多 backend 比对的金标准。
5. M1-08 verification pass 之后追加一个 "M0/M1 spike report"，记录 R-D 与 R-E
   的 spike 结果（rust-toplingdb 编译/链接情况、Aptos gRPC 鉴权流程）。

---

## 6. 落地补丁清单（按可立即执行排序）

下面这些是建议在动 M0 代码之前先把 docs 改齐的轻量改动，全部不写 Rust 代码。

- [ ] 更新 `DEVELOPMENT_PLAN.md` §5，把 dataset pipeline 抽到 M2，REST API 后移到 M6，
      并显式声明 "benchmark runner 禁止访问 gRPC"。
- [ ] 在 `DEVELOPMENT_PLAN.md` 增加 §8 "Benchmark Methodology Constraints"，
      把 §3 R-A 的 10 条最低必须项落到 acceptance。
- [ ] 在 `DEVELOPMENT_PLAN.md` 增加 §9 "Dataset Reproducibility Constraints"，
      明确 manifest 字段、sha256、dataset_id、不允许随机生成 corpus。
- [ ] 在 `DEVELOPMENT_PLAN.md` 增加 §10 "Cross-Backend Equivalence"，
      要求 `decibel-admin compare-checksum` 是 benchmark 报告的前置条件。
- [ ] 在 `docs/MILESTONE_0_1_PLAN.md` 按 §5 补 trait 与 manifest 字段。
- [ ] 新建 `docs/SPIKES.md`，列出 R-D（rust-toplingdb 可用性）和
      R-E（Aptos Transaction Stream auth）两个 spike 的目标、退出条件、
      回退方案。
- [ ] 新建 `docs/DATASET_LAYOUT.md`，把 addendum §5 的目录布局与 manifest schema
      固化为项目内规范，供 dataset crate 直接遵循。
- [ ] 新建 `docs/BENCHMARK_METHODOLOGY.md`，整合 sui-hotstore review 的教训，
      作为 bench runner 实现时的硬约束清单。

---

## 7. 与当前 14 天 MVP 的可行性判断

按建议重排后，14 天 MVP 仍然成立，但需要承认：

- M2（dataset pipeline）会从原 plan 的 "fixture 写写就行" 扩成 1.5–2 天工作量，
  含 manifest / synthetic / query_corpus / replay。
- R-D / R-E 两个 spike 占用 M0 的半天到一天。
- REST API 只保证 MVP endpoints（health / ingest/status / 两三个数据查询），
  WebSocket / x402 全部留给后续。
- Benchmark 报告里 "三类 benchmark" 中 read-under-ingest 可以在第一版以
  "best-effort, marked preliminary" 出，但 ingest 和 serving 必须正式出。

对外 grant 叙事路径不变：

```
Decibel-specific KV serving layer
  + reproducible dataset (manifest + sha256 + bounded range)
  + cross-backend checksum equivalence
  + same-schema/same-corpus benchmark
  + builder-code analytics (with explicit disclaimer)
```

这条线相比"单纯 ToplingDB 性能宣传"更稳，也更贴近 Aptos / Decibel 生态此刻
真实关心的 builder / bot / dashboard 客户。
