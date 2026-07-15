# Decibel HotIndex 实现层独立 Review (R2)

Last updated: 2026-06-01
Scope: 在 `docs/PLAN_REVIEW.md` (R1) 之后第二轮独立审阅，聚焦**已落地代码**而不是 plan 文档。
R2 重点回答："看看有哪些实现的有问题，或者遗漏的地方"。

Reviewer note: 文中所有问题都带 file:line 引用，便于直接对照修复。严重度按 P0/P1/P2 排序。

---

## 0. 一句话结论

工程进度走得比 R1 预期更快——M0–M5 主干都已经写下来，dataset 管线、record/normalize/replay、bench 三类、admin checksum 都有实物。但是：

```text
已有的"漂亮架构"在三个核心地方与代码不一致，会让 benchmark 报告和 grant
narrative 在第三方复核时直接被打穿。
```

三个核心地方是：

1. **ToplingDB backend 是 RocksDB 直通**。Benchmark 里的 "rocksdb vs toplingdb" 对比
   实际上是 "rocksdb vs rocksdb"。
2. **checksum 是 FNV-64 over `format!("{:?}", row)`**。既不是 SHA-256，也不
   hash 真实落盘字节。cross-backend equivalence 的硬证据其实不存在。
3. **真实 Aptos protobuf 数据进 dataset 之后 fills/orders/positions/builder 全空**。
   normalize 阶段只产出 TxRow。所以"mainnet bounded benchmark"目前实际上跑不出
   Decibel 维度的数据，能跑的只剩 fixture-synthetic 路径。

加上 `aptos-protos` 用了绝对本地路径，repo clone 下来连 dataset crate 都编不过——
grant 评审或外部用户拿到 repo 第一步就 fail。

下面按 P0–P2 展开。

---

## P0: 必须在出任何对外 benchmark/grant 文案之前修掉

### P0-1: ToplingDB backend 实际是 RocksDB 直通

文件: `crates/decibel-hotindex-storage/src/toplingsdb_engine.rs`

```rust
pub struct ToplingDbEngine {
    _config_path: PathBuf,   // 注意下划线：故意标记 unused
    inner: RocksDbEngine,    // 直接打开 RocksDB
}

impl ToplingDbEngine {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let config_path = env::var_os(TOPLINGDB_EASY_MIGRATE_CONF_ENV) ...;
        if !config_path.is_file() { return Err(...); }
        Ok(Self {
            _config_path: config_path,        // 存了但不用
            inner: RocksDbEngine::open(path)?, // 用普通 RocksDB
        })
    }
}
```

所有 trait 方法都是 `self.inner.put_*` / `self.inner.scan_*`。`TOPLINGDB_EASY_MIGRATE_CONF`
存在性被检查，但内容**从未加载进 RocksDB 选项**。

后果：

- `decibel-hotindex-bench --engine toplingdb` 与 `--engine rocksdb` 跑的是同一份
  RocksDB 代码、同一份默认 Options。两边的 p50/p95/throughput 只是噪声差异。
- 报告里"backend: toplingdb"是错误标签。
- `MILESTONE_4_PLAN.md` §M4-02 写的是 "Attempts to open/use the backend return an
  explicit unsupported error"——文档与实现不符，是引入更大风险的方向（doc 声称会拒，
  代码偷偷接受并改写实际后端）。
- `README.md` 第 3 行 "Decibel HotIndex is a ToplingDB/RocksDB-backed local serving
  layer" 在当前状态下不成立。

修复方向（任选一，但必须落地一项）：

A. 真正接 `topling/rust-toplingdb`（参考 sui-hotstore 的依赖路径），并在 open
   时实际把 `TOPLINGDB_EASY_MIGRATE_CONF` 应用到后端。这是最终目标。

B. 立即把 `ToplingDbEngine::open` 改成 `HotIndexError::Config("ToplingDB binding
   not yet integrated; use RocksDB or wait for M4 follow-up")`，并把
   `decibel-hotindex-bench` 的 toplingdb 分支也改成拒绝。这样不会让人误测。

C. 如果短期内 A 不现实，把 README/DEVELOPMENT_PLAN/M4 plan 的 "ToplingDB-backed"
   全部改成 "RocksDB-baseline, ToplingDB integration pending"，并在 admin/bench
   的 `--engine toplingdb` 命令首行打印告警 "WARNING: toplingdb engine currently
   delegates to RocksDB."

我的建议是 B + C 双管齐下，A 进 M4 follow-up。**在 B 落地之前不要出任何
"rocksdb vs toplingdb" 的对比报告。**

---

### P0-2: Checksum 是 FNV-64 over Debug，不是 SHA-256 over bytes

文件: `crates/decibel-hotindex-storage/src/rocksdb_engine.rs:347-436`
文件: `crates/decibel-hotindex-storage/src/memory_engine.rs:254-403`

两处都是这个模式：

```rust
let mut hash = StableHasher::default();        // FNV-1a, 64-bit
hash.update(&key);                              // 二进制 key OK
hash.update(&[0xff]);
hash.update(debug_value(cf_name, &value)?.as_bytes()); // !!! Debug format
hash.update(&[0xfe]);
// 最终 format!("{:016x}", hash.finish())       // 16 hex chars
```

问题：

1. **`format!("{:?}", row)`** 把 row 用 Rust Debug 派生格式打印成 String，
   再 hash。这意味着：
   - Field 顺序换一下，hash 全变。
   - serde rename / enum variant 重命名也会变。
   - 跨 Rust 版本不保证稳定（Debug 是 unstable surface）。
   - 真正落盘的 value (RocksDB 是 serde_json bytes，MemoryEngine 是 typed row)
     被绕开了；只要两边都用同一份 Rust 类型 `format!("{:?}")` 出来一样，就过——
     这只是"两边用了同一个公共表达式"的证明，不是"两边持有同一份数据"的证明。
2. **`StableHasher` 是 FNV-1a 风格的 64-bit 滚动 hash**。2⁶⁴ 空间下，
   碰撞构造在 grant 评审尺度上不至于被人玩，但作为 "same dataset, same schema"
   的硬证据是不够看的；任何外部审计都会要 SHA-256。
3. **CF 覆盖缺一块**。`rocksdb_engine.rs:24-33` 的 `LOGICAL_CFS` 不包含
   `CF_MARKET_RECENT_ACTIVITY`；`memory_engine.rs:254` 的 checksum vec 也不
   返回 activity CF。但 `put_fill` 两边都会写 activity (`rocksdb_engine.rs:158-166`,
   `memory_engine.rs:42-52`)。结果是：activity CF 上的差异**永远静默通过 compare-checksum**。

后果：

- `decibel-admin compare-checksum --left rocksdb --right rocksdb-replayed` 可能
  在数据真的不一致时也 pass。
- 任何"两个 backend 的 checksum 完全一致"对外口径，都需要补一句"基于 FNV-64
  Debug-format 等价类，未覆盖 activity CF"——这种附注会直接劝退 grant 评审。

修复方向（最小集）：

- `CfChecksum.hash_hex` 改成 SHA-256，64 hex chars。
- 用 `sha2::Sha256` 取代 `StableHasher`（依赖已经在 `decibel-hotindex-bench`
  和 `decibel-dataset` 里，只需要把它加到 storage crate）。
- Hash 的输入改成 (key bytes, value bytes)：RocksDB 直接用 `iterator_cf` 返回
  的 raw bytes；MemoryEngine 用 `serde_json::to_vec` 做 canonical encoding，
  确保两边 hash 同一种 byte 表达。
- 把 `CF_MARKET_RECENT_ACTIVITY` 加进 `LOGICAL_CFS` 和 MemoryEngine 的 checksum 列表，
  或者明确把它从 schema 等价范围里剔除（在 SCHEMA.md 里写"activity CF 不参与
  cross-backend checksum，因为它是 derived view"）。我建议前者，因为现在它已经
  在 `put_fill` 路径里。
- 加一个 R1 PLAN_REVIEW 里点过的等价测试：跑 RocksDB 上的 `checksums()`，
  然后**完全清空 DB**、再 replay 同一个 dataset、再 hash，应该得到完全一样的结果。
  现在的 `rocksdb_checksums_match_memory_for_same_rows` 测试只验"内存 == RocksDB
  一次"，不验"RocksDB 自身两次 replay 一致"。

---

### P0-3: 真实 Aptos protobuf 路径只产出 TxRow，没有 Decibel events

文件: `crates/decibel-dataset/src/main.rs:1566-1657` (`normalize_protobuf_tx_only`)

代码逻辑：

```rust
// 仅 decode tx 元数据，写出 TxRow
let row = tx_row_from_transaction(&transaction, parser_options);
serde_json::to_writer(&mut tx_writer, &row)?;

// 其余文件全部写空
write_ndjson::<NormalizedEvent>(&normalized_dir.join("events.ndjson"), &[])?;
write_ndjson::<FillRow>(&normalized_dir.join("fills.ndjson"), &[])?;
write_ndjson::<OrderRow>(&normalized_dir.join("orders.ndjson"), &[])?;
write_ndjson::<PositionRow>(&normalized_dir.join("positions.ndjson"), &[])?;
write_ndjson::<BuilderAttributionRow>(&normalized_dir.join("builder_code_rows.ndjson"), &[])?;
write_ndjson::<ActivityRow>(&normalized_dir.join("activity_rows.ndjson"), &[])?;
write_ndjson::<NormalizedEvent>(&normalized_dir.join("unknown_events.ndjson"), &[])?;
fs::write(
    normalized_dir.join("parse_warnings.log"),
    "tx-only protobuf normalization: Decibel event extraction is pending\n",
)?;
```

`docs/REAL_DATA_PREP.md` 第 158 行已经诚实承认这一点，但**没有出现在 README 顶层 Status
列表里**——README 第 25 行只写 "mainnet raw archive format: length-delimited Aptos
Transaction protobuf + zstd"，让人误以为真实数据走得通。

下游连锁：

- `decibel-dataset build-query-corpus` 读 `events.ndjson` (`main.rs:190`)，
  真实数据集里这个文件是空的 → corpus 全空 → 报错"query corpus for workload
  ... is empty" (`crates/decibel-hotindex-bench/src/main.rs:184-188`)。
- `decibel-hotindex-bench --class serving` 在真实数据集上**根本无法启动**。
- `decibel-hotindex-bench --class ingest` 可以跑（IngestRows 包括 txs），
  但只会 replay TxRow——衡量的不是 Decibel 工作负载。
- `read-under-ingest` 同理：只在 tx 维度上做并发，与"Decibel dashboard hot path"
  的卖点无关。

所以现在的真实情况是：

```text
real protobuf path:    Decibel events 缺失 -> bench 不能跑
fixture JSONL path:    有 Decibel events，但 schema 是项目自造的 fake JSON
                        -> 跑得通 bench，但不是真实 Decibel 数据
```

中间没有 overlap。Benchmark 报告即便能产出，也是 fixture 上的数据。

修复方向：

- 在 `crates/decibel-hotindex-ingest` 里加一条 `parse_protobuf_transaction(&Transaction)`
  路径，按 `aptos-labs/decibel-indexer-example` 的 event_router 把
  `events[i].type_str` 与 Decibel package address 匹配，从 `events[i].data` JSON
  payload 抽 `NormalizedEvent`/`FillRow`/...。
- 把 `normalize_protobuf_tx_only` 改名为 `normalize_protobuf` 并把空文件兜底换成
  真实 events 提取。
- 实在做不完的情况下，至少在 README Status 加一条 "Real mainnet Decibel event
  extraction: pending"，并在 `decibel-hotindex-bench --class serving` 检测到
  events.ndjson 为空时打印明确的引导 "this dataset has no Decibel events;
  use fixture dataset, or wait for M3 protobuf event extraction"。

---

### P0-4: aptos-protos 用绝对本地路径，repo 不可复现

文件: `crates/decibel-dataset/Cargo.toml:12`

```toml
aptos-protos = { path = "/Users/ssyuan/work/project/aptos/protos/rust" }
```

任何 clone 这个 repo 的人 `cargo check --workspace` 会立刻 fail：

```text
error: failed to load manifest for workspace member `crates/decibel-dataset`
  failed to read `/Users/ssyuan/work/project/aptos/protos/rust/Cargo.toml`
```

`Cargo.lock` 也提交了，但它依赖这个本地路径，所以 lockfile 对其他人没意义。

修复：把它改成 git dep，例如：

```toml
aptos-protos = { git = "https://github.com/aptos-labs/aptos-core", rev = "<pinned-sha>" }
```

或者用官方发布的 crate（如果有合适版本）。同时检查
`tonic`/`prost`/`tokio` 是否能与 aptos-protos 的版本约束兼容。

P0-4 不修，grant 评审第一步 build 就失败。

---

### P0-5: Benchmark runner 是单线程闭环，方法学硬约束全部没落地

文件: `crates/decibel-hotindex-bench/src/main.rs`

`docs/BENCHMARK_METHODOLOGY.md` (M5 时写下) 明确要求：

- HDR histogram
- open-loop `--rate` mode
- access pattern `{sequential, uniform, zipfian}` + `--seed`
- 环境指纹包含 CPU model / RAM / kernel / FS / RocksDB version / git sha
- 错误按类计数
- compaction/cache state 记录

实际代码：

- `measure_queries` 是单 worker `for idx in 0..iterations`，固定
  `corpus[idx % corpus.len()]`，sequential only (line 738-760)。
- 没有 `--concurrency` 参数，没有 `std::thread` / `rayon`。
- `access_pattern` 字段在 `run_command` 里被读，但只是塞进 `BenchmarkReport`
  作为 metadata（line 1019），**从未影响迭代顺序**。
- `seed` 同样是 metadata only，永远是字符串 `"query-corpus-order"`，
  没有 `SmallRng::seed_from_u64` 之类。
- Latencies 用 `Vec::with_capacity(iterations)` 收集后 `sort_unstable()`
  (line 842-867)。没有 HDR histogram，p999 在小样本下噪声大、不可 merge。
- `EnvironmentReport::capture` 只采集 4 个字段：os / arch / cpu_parallelism /
  storage_path，并把 `rust_profile = "dev"` 硬编码 (line 1120-1134)，即使是
  release build 也写 "dev"。
- 错误处理：`if execute_query(...).is_err() { errors += 1; }` —— 不分类，不打印
  首条 stderr (line 754-756)。
- 没有 compaction 控制。`measure_read_under_ingest` 在 `std::thread::scope` 里
  起 ingest 线程，但用的是同一个 materialized 目录，重跑会双重写入
  (line 825-840 + 692)。
- "ingest lag" 概念在 read-under-ingest 里完全没体现——背景 ingest 跑完就 join，
  没有滞后窗口度量。
- multi_get 走 `multi_get_txs` trait，但 RocksDB 实现内部就是 N 次单 get
  (`rocksdb_engine.rs:220-225`)。这是 R1 PLAN_REVIEW.md §3 R-A 第 1 项点出来的
  sui-hotstore 老坑，**这版没修**。

结论：现在 bench 报告只能用作 smoke/CI verification。**不能上 README，
不能上 grant**。如果在这种状态下出 BENCHMARK_SUMMARY.md，第三方拿一个有
经验的 reviewer 一行行扒，问题清单足够把整个对外 narrative 否掉。

最小修复集（要么这些全做完，要么 BENCHMARK_PLAN/README 里明确标 "engineering
smoke, not publishable"）：

1. RocksDB 接 `db.batched_multi_get_cf(&handle, keys, false)`；trait 不动。
2. 加 `--concurrency`、按 worker 把 corpus 切片；下限 `iterations/concurrency >= 1000`。
3. 用 `hdrhistogram` crate；worker 各自 record，最后 merge。
4. 加 `--rate <qps>` open-loop 模式，latency 用 `now - expected_start_time`。
5. `EnvironmentReport` 补 cpu model（`/proc/cpuinfo` 第一条 / sysctl macOS）、
   总内存、kernel、`available_parallelism`、FS、挂载选项、RocksDB 编译版本
   （`librocksdb-sys::ROCKSDB_VERSION_*` 或类似常量）、bench binary git sha
   （build.rs 注入 `option_env!("CARGO_PKG_VERSION")` 不够，要 `git rev-parse HEAD`）、
   `rust_profile` 从 `cfg!(debug_assertions)` 推断。
6. 加 `--access-pattern` 实际执行：sequential 走当前路径，uniform 用
   `SmallRng::from_seed` per worker 取 `gen_range(0..corpus.len())`，
   zipfian 用 zipf 系数 theta=0.99。
7. Errors 用 `BTreeMap<String, ErrorBucket>`，首条打印到 stderr，限制总量。
8. `read-under-ingest` 真正测 ingest lag：
   ingest 线程每 N 条记 `last_ingested_version`；query worker 比较 corpus
   命中的 version 与 `last_ingested_version` 的差，得到 lag distribution。
9. bench 启动时先调用一次 `db.compact_range_cf(handle, None, None)` 让 LSM 收敛，
   并把"compaction performed"写入报告。

---

## P1: 影响正确性 / 复现性，但不一定堵 grant

### P1-1: `BuilderVolume` 的 24h 窗口锚在数据 max_ts，不是 wall clock

文件: `crates/decibel-hotindex-storage/src/rocksdb_engine.rs:267-317`
文件: `crates/decibel-hotindex-storage/src/memory_engine.rs:164-221`

```rust
let max_ts = rows.iter().map(|row| row.timestamp_us).max().unwrap_or_default();
let window_start_ts_us = max_ts.saturating_sub(window.duration_us()).saturating_add(1);
```

如果 builder 已经停手 30 天，`get_builder_code_volume("...", H24)` 返回的是
30 天前那 24 小时的 volume，但报告字段叫 "24h"。API 消费者会以为是当下 24h。

修复：

- 给 trait 加显式 `as_of_ts_us: u64` 参数，让上层（API/bench）决定锚点；
  默认 wall clock。
- 或者从 IngestCheckpoint.last_processed_timestamp_us 取锚。
- API 响应里把 `window_end_ts_us` 一并暴露，避免歧义。

### P1-2: 真实 protobuf 路径直接用 fixture parser，没用 decibel-indexer-example

文件: `crates/decibel-hotindex-ingest/src/decibel_parser.rs:374-401`

```rust
fn classify_event_type(raw_type: &str, data: &Value) -> DecibelEventType {
    let tail = event_type_tail(raw_type);
    let lowered = tail.to_ascii_lowercase();
    if lowered.contains("trade") || lowered.contains("fill") { ... }
    else if lowered.contains("liquidation") || lowered.contains("margincall") { ... }
    ...
}
```

这是字符串子串启发式。Decibel 官方 example 给的是显式 31 类事件名映射，
应该走 allowlist 而不是 substring 命中。

`maybe_push_fill` 等里的 field 备选名：

```rust
string_field(data, &["market_id", "market", "symbol"])
string_field(data, &["account", "trader", "user", "owner"])
```

这些备选名是工程师猜的，不是 decibel-indexer-example 真实字段。fixture 自造
JSON 配合一下能跑通；真 protobuf 上一旦字段名不在 fallback list 里就静默丢。

修复方向：

- 把 `aptos-labs/decibel-indexer-example` 的 `events/*.rs` 列表 vendor 进来
  （或者跟它的 Cargo 依赖），把 event-type allowlist 固定下来。
- `parser_commit` 字段已经在 `ParserOptions` 里——把 vendored 版本的 git sha
  写进去，确保 manifest 里的 `parser_commit` 是有意义的。
- 完成 P0-3 时一并处理。

### P1-3: workspace deps 没有集中，版本可能漂

每个 crate 自己写 `serde = { version = "1", features = ["derive"] }`、
`serde_json = "1"`、`sha2 = "0.10"`。M3 之后再加 crate 时很容易写不同的版本号。

修复：在 root `Cargo.toml` 加 `[workspace.dependencies]`，所有 crate 用
`serde = { workspace = true }`。同时把 rocksdb 版本也 hoist 上去。

### P1-4: 写文件路径没有 atomic rename + fsync

文件: `crates/decibel-dataset/src/main.rs:1372-1430` 等

`write_synthetic_dataset` / `write_normalized_fixture_dataset` 都是
`fs::create_dir_all` + `File::create` + `serde_json::to_writer` + `flush`。
没有 `tmp + rename`，没有 `file.sync_all()`。

后果：写到一半 crash / OOM / kill，目录里会留下：

- manifest.json 写好了，但 normalized/events.ndjson 是部分写入
- 或反之

下次 replay 时 `validate_manifest_hashes` 会发现 sha256 不一致，但用户已经损失
全部上游数据。

`record` 路径反而做对了，用 `TransactionChunkWriter` + `fs::rename` 写入 `.tmp`
再 rename（见 `main.rs:919-998`）。把同一套模式推广到 synthetic / normalize /
query_corpus 输出。

### P1-5: RocksDB `stats()` 是 O(N) 全表扫

文件: `crates/decibel-hotindex-storage/src/rocksdb_engine.rs:113-121, 327-337`

```rust
fn cf_len(&self, cf_name: &str) -> Result<u64> {
    let cf = self.cf(cf_name)?;
    let mut count = 0_u64;
    for item in self.db.iterator_cf(cf, IteratorMode::Start) {
        item.map_err(rocks_error)?;
        count += 1;
    }
    Ok(count)
}
```

对 1M+ 行的 mainnet bounded dataset，每次 `GET /stats` 都是分钟级。
- 改用 `db.property_value_cf(cf, "rocksdb.estimate-num-keys")`（毫秒级，
  不精确但足够 stats）；
- 如果上层确实需要精确计数，再提供 `decibel-admin exact-count` 单独命令。

### P1-6: `order_by_id` 没含 `market_id`

文件: `crates/decibel-hotindex-storage/src/key.rs:76-78`

```rust
pub fn order_by_id(order_id: &str) -> Vec<u8> {
    order_id.as_bytes().to_vec()
}
```

如果两个 market 用了同一个 client_order_id（用户/bot 给同一个值），后写的会覆盖
先写的。Decibel 内部 order_id 是不是全局唯一需要查一下；如果不能保证，加
`market_id` 前缀更稳。

### P1-7: SEP=0 分隔符没强制约束 segment 内不出现 0x00

文件: `crates/decibel-hotindex-storage/src/key.rs:3-4, 134-150`

`market_id`/`account`/`fill_id`/`builder_addr` 是用户/链上数据，理论上 ASCII 安全，
但代码层没有 assert。如果将来增加任意 bytes segment（例如真 binary tx hash 不
hex-encode 就塞进 key），prefix scan 会被 0x00 截断或者出现假阳性 prefix 命中。

最小修复：在 `join_segments` 里 debug_assert 每段不包含 0x00；或者改成 length-prefix
编码（`be_u32(len)` + payload）。length-prefix 是更稳的工程选择，sui-hotstore 也
是这样。

### P1-8: API crate 是 6 行 println

文件: `crates/decibel-hotindex-api/src/main.rs`

```rust
fn main() {
    println!("decibel-hotindex-api {}", decibel_hotindex_core::crate_status());
}
```

这本身 OK（API 在 M6）。但 workspace 列出来 + README 提到 / planning docs
默认存在，会让 grant 评审误以为有 HTTP 端点能查。建议在 README "Current Status"
里显式写一行 "REST API: not started (M6)"。

### P1-9: dataset crate 是 2947 行单 main.rs

`crates/decibel-dataset/src/main.rs` 把 7 个 command（synthetic / fixture /
normalize / build-query-corpus / replay / record / inspect-raw）+ live gRPC
recorder + checkpoint resume + chunk writer + manifest writer 全塞在一个文件。

不是 P0，但任何后续接 protobuf 事件解析 / topling backend / open-loop bench 都
要回到这里改，会迅速变成不可维护。建议拆成：

```
src/
  main.rs            (只做 CLI dispatch)
  manifest.rs
  synthetic.rs
  fixture.rs
  normalize/
    mod.rs
    fixture_jsonl.rs
    protobuf.rs
  query_corpus.rs
  replay.rs
  record/
    mod.rs
    live.rs
    chunk.rs
    checkpoint.rs
```

---

## P2: 卫生 / 长期维护性

### P2-1: README "Current Status" 与 docs 真相不一致

README 第 25 行 "mainnet raw archive format: length-delimited Aptos Transaction
protobuf + zstd" 让人以为 mainnet pipeline 完整可用。docs/REAL_DATA_PREP.md
最后一行才承认 tx-only。建议 README Status 改成：

```text
- mainnet raw archive recording: working (tx-only protobuf+zstd)
- mainnet Decibel event extraction: pending
- ToplingDB backend: stub (delegates to RocksDB, pending native binding)
- REST API: not started
- Benchmark runner: smoke only, methodology hardening pending (--rate, HDR,
  concurrency, env fingerprint)
```

### P2-2: 没有 CI

`.github/workflows/` 没有。grant 评审会看 CI badge。最小集合：

```yaml
# .github/workflows/ci.yml
- cargo fmt --all --check
- cargo check --workspace
- cargo test --workspace
- cargo check -p decibel-hotindex-storage --features rocksdb
- cargo test  -p decibel-hotindex-storage --features rocksdb
```

P0-4 修了之后这套才有意义。

### P2-3: `key::checksum_logical_cf` 定义了从未被使用

`crates/decibel-hotindex-storage/src/key.rs:130-132` 是 dead code。删掉或者
接进 checksum 路径。

### P2-4: `ARCHITECTURE.md` 没有保留之前已经画过的 Mermaid 图

sui-hotstore README 的 mermaid 图在 grant pitch 时复用度很高。本仓库的
ARCHITECTURE.md 只有文本块状图。建议加 mermaid 版本（M0-03 plan 提过
"architecture diagram" 但没落到 docs）。

### P2-5: `BUILDER_FEE_BPS` 用 `u16`，但 Decibel 实际 fee 表达精度未确认

`FillRow.builder_fee_bps: Option<u16>`：上限 65535 bps = 655%。Decibel 实际
不会到那么高，但 16-bit 上限来自工程师选择不是 ABI 文档。等接 P1-2 的官方
parser 时一起确认。

### P2-6: `IngestCheckpoint.last_processed_timestamp_us = 0` 在 replay 里被硬写

文件: `crates/decibel-dataset/src/main.rs:1469-1478`

```rust
engine.put_ingest_checkpoint(IngestCheckpoint {
    ...
    last_processed_timestamp_us: 0,    // 硬写 0
    ...
})?;
```

这会让 API `/ingest/status` 输出 `last_processed_timestamp_us: 0`。应该用
manifest 里 last tx 的 block_timestamp_us，或者 dataset 里 normalized events
的 max timestamp。

---

## 1. 整体修复优先级建议

按 "对外可信度 / 修复成本" 排序：

1. **P0-4** aptos-protos 路径 (10 分钟改 Cargo.toml + 重新 lock，最便宜)
2. **P0-1** ToplingDB backend 立刻改成显式 unsupported (30 分钟，挽回 narrative)
3. **P0-2** checksum 改 SHA-256 over bytes + 补 activity CF (1 天)
4. **P1-9** 拆 decibel-dataset main.rs，否则后续没法继续往里塞东西 (半天)
5. **P0-3** real protobuf 路径接 Decibel event 提取 (2-4 天，依赖 P1-2 同步做)
6. **P0-5** bench methodology 硬约束落地 (2-3 天)
7. **P1-1** builder window 锚点修正 (半天)
8. 其余 P1/P2 按节奏来

修完 1–3 之后，DEVELOPMENT_PLAN.md / README.md 的对外口径就和实际能力对齐了，
不会出现"宣传跑了 ToplingDB benchmark 实际是 RocksDB"这类无法回头的事故。

修完 4–6 之后，第一份对外 BENCHMARK_SUMMARY 可以发；在那之前任何报告都建议
打 `engineering_preview = true` 标签，且不进 grant 材料。

---

## 2. 与 R1 PLAN_REVIEW.md 的关系

R1 列了 6 类风险 (R-A 到 R-F)，R2 是它们在代码里的实际落地状态：

| R1 风险 | R2 实际状态 | 严重度 |
|---|---|---|
| R-A: benchmark 报告不安全 | 完全没落 (P0-5) | P0 |
| R-B: dataset 不可复现 | 部分落 (manifest + sha256 OK; atomic write 缺) | P1 (P1-4) |
| R-C: 跨 backend checksum 无证据 | 假落 (P0-2: 用 FNV-Debug 假装等价) | P0 |
| R-D: ToplingDB binding 不确定 | 退化为 RocksDB 直通 (P0-1) | P0 |
| R-E: Aptos Transaction Stream auth | 通过 spike，recorder 走通 | OK |
| R-F: Decibel ABI 解析覆盖不完整 | 真实路径完全没解析 (P0-3, P1-2) | P0 |

R1 的 §4 milestone 重排（dataset crate 前置、checksum gate、bench methodology
最低必须项）方向正确，团队也基本按这个顺序走了。但 §3 R-A 的 10 条最低必须项
**一条都没进代码**，R-C 的 `compare-checksum` 接口虽然写出来了但底层 hash
算法弱到不能作证据。所以 R2 是把 R1 的 doc 级约束**翻译成代码级 PR clip list**。

---

## 3. 最后给团队的一句话

> 现阶段项目"看起来已经做了很多"，但只要外部 reviewer 实际跑一遍代码或者审一遍
> CF 列表、checksum 算法、ToplingDB engine 实现，会立刻发现 narrative 与实现
> 不一致。修复成本最低、收益最高的优先级是 P0-1 / P0-4——它们都不到 1 天就能改完，
> 改完之后至少可以安全地宣传"RocksDB baseline + dataset pipeline + bench
> smoke"。在 P0-2 / P0-3 / P0-5 全部落地之前，不要让任何 benchmark 数字流出仓库。
