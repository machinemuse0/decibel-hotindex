# Local RocksDB End-to-End Flow

这份文档用于本地排查 Decibel HotIndex 的完整 RocksDB 流程：

```text
拉取 Aptos mainnet bounded raw data
  -> 本地 inspect raw chunk
  -> normalize protobuf dataset
  -> replay/import 到 RocksDB
  -> checksum
  -> serving benchmark
```

真实 protobuf 路径会导入 transaction rows，并在 bounded range 含有匹配
Decibel event type 时生成 fills/orders/positions/builder rows。没有 Decibel
events 的 range 仍然适合验证 raw/protobuf/replay/checksum 和
`get_tx_by_version`、`multi_get_tx_versions_100`，但 `mixed_market_dashboard`
等 Decibel event workload 必须使用 Decibel-active range，或临时用 fixture/synthetic
数据。

## 0. One-Time Checks

确认本地工具链和 workspace 能跑：

```bash
rtk cargo check --workspace
rtk cargo test --workspace
```

如果 RocksDB 编译失败，先补本机依赖：Rust stable、clang/libclang、cmake、
pkg-config、OpenSSL headers、zstd。

## 1. Pick A Local Dataset

不要把 API key、raw chunks、materialized DB 提交进 git。`datasets/*` 已经被
`.gitignore` 忽略，适合放本地调试数据。

```bash
export APTOS_GRPC_AUTH_TOKEN="<redacted>"

export START_VERSION=4365621793
export END_VERSION=4365622792
export DATASET_ROOT="$PWD/datasets/local-rocksdb-${START_VERSION}-${END_VERSION}"

rtk mkdir -p "$DATASET_ROOT/raw"
```

建议第一遍只拉 1k 到 10k transactions，确认 auth、raw decode、import 和 bench 都
跑通后再放大范围。

## 2. Pull Raw Data

从 Aptos mainnet Transaction Stream 拉一个有边界的 range：

```bash
rtk cargo run -p decibel-dataset -- record \
  --live \
  --network mainnet \
  --endpoint grpc.mainnet.aptoslabs.com:443 \
  --auth-token-env APTOS_GRPC_AUTH_TOKEN \
  --start-version "$START_VERSION" \
  --end-version "$END_VERSION" \
  --batch-size 100 \
  --chunk-transaction-count 100000 \
  --progress-interval-secs 30 \
  --key-sample-limit 100000 \
  --out-dir "$DATASET_ROOT/raw" \
  --raw-format protobuf-zstd
```

成功后会看到类似：

```text
recorded transaction stream: tx=1000 range=4365621793..4365622792 chunk=.../transactions_4365621793_4365622792.pb.zst
```

`record` 默认每 30 秒打印一次进度 summary，包括当前 tx 数、百分比、last/next
version、raw bytes、吞吐和 ETA。把 `--progress-interval-secs 0` 设为 0 可以关闭周期进度。

## 3. Inspect The Raw Chunk

先确认 raw chunk 可解码，且 range 没错：

```bash
rtk cargo run -p decibel-dataset -- inspect-raw \
  --input "$DATASET_ROOT/raw/transactions_${START_VERSION}_${END_VERSION}.pb.zst"
```

再看 checkpoint：

```bash
rtk read "$DATASET_ROOT/raw/record_checkpoint.json"
```

重点检查：

- `status` 是 `complete`
- `chain_id` 是 `1`
- `first_version` / `last_version` 符合预期
- `next_start_version` 等于 `END_VERSION + 1`
- 文件里没有 token 明文

## 4. Build RocksDB Binaries

本地导入和压测都用 release binary，避免 debug build 把结果拖慢太多：

```bash
rtk cargo build \
  -p decibel-dataset \
  -p decibel-admin \
  -p decibel-hotindex-bench \
  --features rocksdb \
  --release \
  --target-dir target/rocksdb
```

## 5. Import Into RocksDB

一条命令完成 normalize、RocksDB replay、checksum：

```bash
rtk ./scripts/import-real-data.sh rocksdb "$DATASET_ROOT" \
  --bin-dir target/rocksdb/release
```

产物位置：

```text
$DATASET_ROOT/manifest.json
$DATASET_ROOT/normalized/txs.ndjson
$DATASET_ROOT/materialized/rocksdb/
$DATASET_ROOT/reports/rocksdb-checksums.json
```

如果要重建同一个 RocksDB 目录：

```bash
rtk ./scripts/import-real-data.sh rocksdb "$DATASET_ROOT" \
  --bin-dir target/rocksdb/release \
  --force
```

如果只想重跑 replay，不想重新 normalize：

```bash
rtk ./scripts/import-real-data.sh rocksdb "$DATASET_ROOT" \
  --bin-dir target/rocksdb/release \
  --skip-normalize \
  --force
```

## 6. Run Serving Benchmarks

如果这个 bounded range 没有 Decibel events，先跑 tx key workloads：

```bash
rtk ./scripts/run-benchmark-suite.sh \
  --backend rocksdb \
  --dataset "$DATASET_ROOT" \
  --bin-dir target/rocksdb/release \
  --workloads get_tx_by_version,multi_get_tx_versions_100 \
  --iterations 100000 \
  --warmup 1000 \
  --access-pattern zipfian
```

输出：

```text
$DATASET_ROOT/reports/bench-rocksdb-serving-get_tx_by_version.json
$DATASET_ROOT/reports/bench-rocksdb-serving-multi_get_tx_versions_100.json
$DATASET_ROOT/reports/BENCHMARK_SUMMARY.md
```

看摘要：

```bash
rtk read "$DATASET_ROOT/reports/BENCHMARK_SUMMARY.md"
```

这些结果是本地 engineering smoke，用于排障和回归对比；不要直接当成可发布的
RocksDB 性能报告。

## 7. Scale Up Or Resume

小 range 跑通后，可以扩大 `END_VERSION`，或者用 byte cap 控制一次拉取规模：
如果要保留 1k smoke 数据，先换一个新的 `DATASET_ROOT`。

```bash
export END_VERSION=4381375638

rtk cargo run -p decibel-dataset -- record \
  --live \
  --network mainnet \
  --endpoint grpc.mainnet.aptoslabs.com:443 \
  --auth-token-env APTOS_GRPC_AUTH_TOKEN \
  --resume \
  --end-version "$END_VERSION" \
  --max-raw-bytes 10GiB \
  --max-stream-retries 10 \
  --chunk-transaction-count 100000 \
  --batch-size 500 \
  --progress-interval-secs 30 \
  --key-sample-limit 1000000 \
  --out-dir "$DATASET_ROOT/raw" \
  --raw-format protobuf-zstd
```

恢复逻辑优先读：

```text
$DATASET_ROOT/raw/record_checkpoint.json
```

如果只有 `.pb.zst.tmp`，先用允许截断模式定位下一个版本：

```bash
rtk cargo run -p decibel-dataset -- inspect-raw \
  --input "$DATASET_ROOT/raw/<tmp-file>.pb.zst.tmp" \
  --allow-truncated
```

然后用输出里的 `next_start_version` 作为新的 `--start-version`，或修好 checkpoint 后
继续 `--resume`。

## 8. Common Failures

`missing APTOS_GRPC_AUTH_TOKEN`

检查当前 shell：

```bash
test -n "$APTOS_GRPC_AUTH_TOKEN" && echo "token present"
```

`materialized DB path already exists`

第一次导入不要加 `--force`；重建同一个目录时加 `--force`。

`workload ... requires Decibel events`

当前 bounded range 没有可用的 Decibel event rows。先跑：

```text
get_tx_by_version
multi_get_tx_versions_100
```

需要 Decibel workload 时，换一个 Decibel-active bounded range，或先切到
fixture/synthetic dataset。

`missing dataset manifest`

通常是 import 前没有 normalize 成功。重跑：

```bash
rtk ./scripts/import-real-data.sh rocksdb "$DATASET_ROOT" \
  --bin-dir target/rocksdb/release \
  --force-normalize
```

## 9. Manual Equivalent

如果需要拆开查问题，可以不用 import script：

```bash
rtk target/rocksdb/release/decibel-dataset normalize \
  --input "$DATASET_ROOT/raw" \
  --out-dir "$DATASET_ROOT/normalized" \
  --format protobuf-zstd \
  --network mainnet

rtk target/rocksdb/release/decibel-dataset replay \
  --dataset "$DATASET_ROOT" \
  --engine rocksdb \
  --db-path "$DATASET_ROOT/materialized/rocksdb"

rtk target/rocksdb/release/decibel-admin checksum \
  --engine rocksdb \
  --db-path "$DATASET_ROOT/materialized/rocksdb" \
  --out "$DATASET_ROOT/reports/rocksdb-checksums.json"
```
