# Server Runbook

This runbook is for collecting Decibel HotIndex mainnet raw data and preparing the server for RocksDB/ToplingDB benchmark runs.

Current status:

- Mainnet Transaction Stream recording works locally.
- 1k transaction raw smoke passed locally:
  - range: `4365621793..4365622792`
  - decoded transactions: `1000`
  - remaining bytes after decode: `0`
- Real Aptos protobuf `normalize` is not wired yet. Server runs should first collect immutable raw archives and verify decode. Normalized replay/benchmark against real raw data comes after the protobuf parser adapter is implemented.

## 1. Server Prerequisites

Install normal Rust build dependencies:

```bash
rustup default stable
sudo apt-get update
sudo apt-get install -y build-essential clang libclang-dev cmake pkg-config libssl-dev zstd
```

If the server is not Ubuntu/Debian, install the equivalent packages:

- C/C++ build toolchain
- clang/libclang for bindgen
- cmake
- pkg-config
- OpenSSL development headers
- zstd CLI for manual inspection

Set libclang if bindgen cannot find it:

```bash
export LIBCLANG_PATH=/usr/lib/llvm-*/lib
```

## 2. Secrets

Export the Geomi/Aptos API key only as an environment variable. Do not paste it into commands, config files, shell scripts, or committed docs.

```bash
export APTOS_GRPC_AUTH_TOKEN="<redacted>"
```

Quick check:

```bash
test -n "$APTOS_GRPC_AUTH_TOKEN" && echo "token present"
```

## 3. Clone And Build

```bash
cd /data
git clone <repo-url> decibel-hotindex
cd /data/decibel-hotindex
rtk cargo check --workspace
rtk cargo test --workspace
```

If `rtk` is not available on the server, use the same commands without the `rtk` prefix.

## 4. Mainnet Raw Recording Smoke

Use a small bounded range first:

```bash
export DATASET_ROOT=/data/decibel-hotindex/datasets/mainnet-4365621793-4365622792
mkdir -p "$DATASET_ROOT/raw"

rtk cargo run -p decibel-dataset -- record \
  --live \
  --network mainnet \
  --endpoint grpc.mainnet.aptoslabs.com:443 \
  --auth-token-env APTOS_GRPC_AUTH_TOKEN \
  --start-version 4365621793 \
  --end-version 4365622792 \
  --batch-size 100 \
  --out-dir "$DATASET_ROOT/raw" \
  --raw-format protobuf-zstd
```

Expected shape:

```text
recorded transaction stream: tx=1000 range=4365621793..4365622792 chunk=.../transactions_4365621793_4365622792.pb.zst
```

Verify the raw chunk:

```bash
rtk cargo run -p decibel-dataset -- inspect-raw \
  --input "$DATASET_ROOT/raw/transactions_4365621793_4365622792.pb.zst"
```

Expected result:

```json
{
  "decoded_transactions": 1000,
  "first_version": 4365621793,
  "last_version": 4365622792,
  "remaining_bytes_after_limit": 0
}
```

Also inspect the checkpoint:

```bash
rtk read "$DATASET_ROOT/raw/record_checkpoint.json"
```

It should include:

- `status: complete`
- `chain_id: 1`
- `transaction_count: 1000`
- `last_success_version: 4365622792`
- chunk sha256
- no token material

## 5. Larger Raw Recording

After the 1k smoke passes, increase the range. Keep ranges explicit and bounded.

Example 100k transaction archive:

```bash
export START_VERSION=4365621793
export END_VERSION=4365721792
export DATASET_ROOT=/data/decibel-hotindex/datasets/mainnet-${START_VERSION}-${END_VERSION}
mkdir -p "$DATASET_ROOT/raw"

rtk cargo run -p decibel-dataset -- record \
  --live \
  --network mainnet \
  --endpoint grpc.mainnet.aptoslabs.com:443 \
  --auth-token-env APTOS_GRPC_AUTH_TOKEN \
  --start-version "$START_VERSION" \
  --end-version "$END_VERSION" \
  --batch-size 500 \
  --out-dir "$DATASET_ROOT/raw" \
  --raw-format protobuf-zstd
```

Then:

```bash
rtk cargo run -p decibel-dataset -- inspect-raw \
  --input "$DATASET_ROOT/raw/transactions_${START_VERSION}_${END_VERSION}.pb.zst"
```

Do not delete raw chunks after normalization. Raw chunks are the source of truth for RocksDB and ToplingDB replay.

Example ~20 GiB archive, based on the observed local ratio `100k tx ~= 130 MiB`:

```bash
export START_VERSION=4365621793
export END_VERSION=4381375638
export DATASET_ROOT=/data/decibel-hotindex/datasets/mainnet-${START_VERSION}-${END_VERSION}
mkdir -p "$DATASET_ROOT/raw"

rtk cargo run -p decibel-dataset -- record \
  --live \
  --network mainnet \
  --endpoint grpc.mainnet.aptoslabs.com:443 \
  --auth-token-env APTOS_GRPC_AUTH_TOKEN \
  --start-version "$START_VERSION" \
  --end-version "$END_VERSION" \
  --batch-size 500 \
  --key-sample-limit 1000000 \
  --out-dir "$DATASET_ROOT/raw" \
  --raw-format protobuf-zstd
```

This range contains `15,753,846` transactions. At `130 MiB / 100k tx`, expected local compressed raw size is roughly `20 GiB`. The Geomi billable Transaction Stream size may differ from local `.pb.zst` size, so check dashboard usage after the first large run.

The recorder also writes benchmark key artifacts while streaming:

```text
$DATASET_ROOT/raw/keys/tx_versions_<start>_<end>.u64be
$DATASET_ROOT/raw/keys/tx_versions_sample_<start>_<end>.ndjson
$DATASET_ROOT/queries/point_tx_versions.ndjson
$DATASET_ROOT/queries/multi_get_tx_versions.ndjson
$DATASET_ROOT/queries/record_keys_manifest.json
```

Notes:

- `tx_versions_*.u64be` contains every recorded transaction version as big-endian `u64`; for 15.75M tx it is about 126 MiB.
- query corpus files are sampled during record using `--key-sample-limit`.
- Decibel market/account/builder query files still require protobuf normalization and event parsing.

## 6. RocksDB Baseline

RocksDB is already wired. For fixture/synthetic datasets:

```bash
rtk cargo check -p decibel-hotindex-storage --features rocksdb
rtk cargo test -p decibel-hotindex-storage --features rocksdb
```

Replay after normalized real-data support is wired:

```bash
rtk cargo run -p decibel-dataset --features rocksdb -- replay \
  --dataset "$DATASET_ROOT" \
  --engine rocksdb \
  --db-path "$DATASET_ROOT/materialized/rocksdb"
```

Checksum:

```bash
mkdir -p "$DATASET_ROOT/reports"

rtk cargo run -p decibel-admin --features rocksdb -- checksum \
  --engine rocksdb \
  --db-path "$DATASET_ROOT/materialized/rocksdb" \
  --out "$DATASET_ROOT/reports/rocksdb-checksums.json"
```

## 7. ToplingDB Server Configuration

Decibel follows the same integration strategy as `sui-hotstore`: use the RocksDB-compatible Rust API, and patch the `rocksdb` crate to `topling/rust-toplingdb` only in the server worktree used for ToplingDB runs.

Current Decibel code support:

- `decibel-hotindex-storage --features toplingsdb`
- `decibel-dataset --features toplingsdb`
- `decibel-admin --features toplingsdb`
- `decibel-hotindex-bench --features toplingsdb`
- `TOPLINGDB_EASY_MIGRATE_CONF` is required and must point to a readable YAML config.

Create a dedicated branch/worktree for ToplingDB so the RocksDB baseline stays clean:

```bash
git switch -c codex/toplingdb-server
```

Patch root `Cargo.toml` on that branch:

```toml
[patch.crates-io]
rocksdb = { git = "https://github.com/topling/rust-toplingdb", rev = "5390ceb77bebba1bf2720b052f83f82b864d64df" }
```

Use the same revision as `sui-hotstore` unless we intentionally update both projects together.

Set the ToplingDB config:

```bash
export TOPLINGDB_EASY_MIGRATE_CONF=/path/to/sui/crates/typed-store/config/topling_sui.yaml
test -f "$TOPLINGDB_EASY_MIGRATE_CONF"
```

If the server does not have a Sui checkout with `topling_sui.yaml`, copy that config from the Sui/ToplingDB environment used by `sui-hotstore`. Do not invent a config silently; record the exact config path and sha256 in benchmark notes.

Build checks:

```bash
rtk cargo check -p decibel-hotindex-storage --features toplingsdb
rtk cargo check -p decibel-dataset --features toplingsdb
rtk cargo check -p decibel-admin --features toplingsdb
rtk cargo check -p decibel-hotindex-bench --features toplingsdb
```

ToplingDB replay after normalized real-data support is wired:

```bash
rtk cargo run -p decibel-dataset --features toplingsdb -- replay \
  --dataset "$DATASET_ROOT" \
  --engine toplingdb \
  --db-path "$DATASET_ROOT/materialized/toplingdb"
```

ToplingDB checksum:

```bash
rtk cargo run -p decibel-admin --features toplingsdb -- checksum \
  --engine toplingdb \
  --db-path "$DATASET_ROOT/materialized/toplingdb" \
  --out "$DATASET_ROOT/reports/toplingdb-checksums.json"
```

Compare:

```bash
rtk cargo run -p decibel-admin -- compare-checksum \
  --left "$DATASET_ROOT/reports/rocksdb-checksums.json" \
  --right "$DATASET_ROOT/reports/toplingdb-checksums.json"
```

## 8. Benchmark Commands

Serving benchmark:

```bash
rtk cargo run -p decibel-hotindex-bench --features rocksdb -- run \
  --dataset "$DATASET_ROOT" \
  --engine rocksdb \
  --db-path "$DATASET_ROOT/materialized/rocksdb" \
  --class serving \
  --workload mixed_market_dashboard \
  --iterations 100000 \
  --warmup 1000 \
  --expected-checksum "$DATASET_ROOT/reports/rocksdb-checksums.json" \
  --out "$DATASET_ROOT/reports/bench-rocksdb-serving.json"
```

ToplingDB serving benchmark:

```bash
rtk cargo run -p decibel-hotindex-bench --features toplingsdb -- run \
  --dataset "$DATASET_ROOT" \
  --engine toplingdb \
  --db-path "$DATASET_ROOT/materialized/toplingdb" \
  --class serving \
  --workload mixed_market_dashboard \
  --iterations 100000 \
  --warmup 1000 \
  --expected-checksum "$DATASET_ROOT/reports/rocksdb-checksums.json" \
  --out "$DATASET_ROOT/reports/bench-toplingdb-serving.json"
```

Summary:

```bash
rtk cargo run -p decibel-hotindex-bench -- summarize \
  --reports "$DATASET_ROOT/reports/bench-rocksdb-serving.json,$DATASET_ROOT/reports/bench-toplingdb-serving.json" \
  --out "$DATASET_ROOT/reports/BENCHMARK_SUMMARY.md"
```

## 9. Current Blocking Item

The live raw archive path is ready. The remaining blocker before real-data RocksDB/ToplingDB materialization is:

- implement `normalize` for Aptos Transaction protobuf `.pb.zst` chunks
- extract Decibel events from real `Transaction` messages
- generate manifest/query corpus from the normalized real rows

Until then, server work should focus on collecting and verifying raw archives, plus compiling the ToplingDB worktree.
