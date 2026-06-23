# Real Data Preparation

Real benchmark data should be recorded from Aptos mainnet once, stored as immutable raw artifacts, then replayed locally into every backend. Benchmark commands must never call Aptos gRPC.

RocksDB and ToplingDB runs must use separate worktrees. See [Backend Worktrees](BACKEND_WORKTREES.md) for the canonical build/import/benchmark flow.

## Auth

Use a local environment variable for the Geomi/Aptos API key:

```bash
export APTOS_GRPC_AUTH_TOKEN="<redacted>"
```

Do not commit API keys, shell history exports containing the key, generated config with token material, or raw command transcripts that include token values.

The current Aptos-hosted Transaction Stream endpoint set remains:

- mainnet: `grpc.mainnet.aptoslabs.com:443`
- testnet: `grpc.testnet.aptoslabs.com:443`
- devnet: `grpc.devnet.aptoslabs.com:443`

The auth header should remain configurable as bearer auth:

```text
Authorization: Bearer $APTOS_GRPC_AUTH_TOKEN
```

## Target Flow

```text
decibel-dataset record mainnet once
  -> raw length-delimited Aptos Transaction protobuf + zstd chunks
  -> manifest sha256 + record checkpoint
  -> decibel-dataset normalize
  -> decibel-dataset build-query-corpus
  -> decibel-dataset replay memory/rocksdb from main
  -> decibel-dataset replay memory/toplingdb from topingdb
  -> decibel-admin checksum
  -> decibel-hotindex-bench run
```

## Raw Archive Rules

- Raw mainnet data is the source of truth.
- Raw chunks are immutable after successful flush.
- Chunk names include version ranges.
- Each chunk has sha256 recorded in `manifest.json`.
- `record_checkpoint.json` tracks `last_success_version` and `next_start_version`.
- Recorder can resume without overwriting completed chunks.
- Normalized rows can be regenerated from raw data.
- RocksDB and ToplingDB tests must replay from the same normalized/raw-derived dataset.

## Storage Format

Default real raw format:

```text
aptos_transaction_protobuf_len_delimited_zstd
```

This avoids JSON bloat for large version ranges while preserving exact transaction payloads for future parser fixes. Early normalized artifacts may stay as `ndjson` for auditability; larger P3/P4 datasets can add `ndjson.zst` or a binary replay encoding, recorded explicitly in the manifest.

## Recorder Status

Current `decibel-dataset record` is safe by default and only opens the live stream when `--live` is provided:

- accepts network, endpoint, auth-token presence, version range, and raw format
- writes a checkpoint
- never writes token material
- uses Aptos `aptos-protos` generated client from the official `aptos-core` repository
- writes length-delimited `Transaction` protobuf messages into `.pb.zst` chunks
- supports `--resume`, `--transactions-count`, `--max-raw-bytes`, and bounded transaction chunks
- supports tx-only protobuf normalization from a single chunk or a raw directory
- supports RocksDB import in `main` and ToplingDB import in the dedicated `topingdb` worktree

Next implementation steps:

- extract Decibel events from real protobuf `Transaction` messages
- generate market/account/builder query corpora from parsed real events

## First Mainnet Smoke

Once `APTOS_GRPC_AUTH_TOKEN` is exported:

```bash
rtk cargo run -p decibel-dataset -- record --live --network mainnet --endpoint grpc.mainnet.aptoslabs.com:443 --auth-token-env APTOS_GRPC_AUTH_TOKEN --start-version <start> --end-version <end> --batch-size 10 --out-dir <dataset>/raw --raw-format protobuf-zstd
```

Use a small bounded range first. If the range has no Decibel events, keep the raw smoke as an auth/protobuf validation and choose a Decibel-active range for the benchmark dataset.

Local smoke result:

```text
recorded transaction stream: tx=2 range=4365621793..4365621794 chunk=/private/tmp/decibel-hotindex-live-smoke/raw/transactions_4365621793_4365621794.pb.zst
```

## Resume And Bounded Pulls

For quota-bounded runs, prefer a byte cap and let the recorder write the next cursor:

```bash
rtk cargo run -p decibel-dataset -- record --live \
  --network mainnet \
  --endpoint grpc.mainnet.aptoslabs.com:443 \
  --auth-token-env APTOS_GRPC_AUTH_TOKEN \
  --resume \
  --end-version 4381375638 \
  --max-raw-bytes 10GiB \
  --max-stream-retries 10 \
  --chunk-transaction-count 100000 \
  --batch-size 500 \
  --out-dir <dataset>/raw \
  --raw-format protobuf-zstd
```

If no checkpoint exists yet, replace `--resume` with `--start-version <version>`. After each completed chunk, `record_checkpoint.json` is updated. The next run can use `--resume` and will start from `next_start_version`. Transient stream interruptions retry in-process by default; tune with `--max-stream-retries` and `--retry-backoff-ms`.

For mainnet, the recorder rejects suspicious ranges such as a tiny `--start-version` with a multi-billion `--end-version`, because that usually means a cursor variable did not expand correctly.

Raw decode check:

```bash
rtk cargo run -p decibel-dataset -- inspect-raw --input /private/tmp/decibel-hotindex-live-smoke/raw/transactions_4365621793_4365621794.pb.zst
```

Expected result:

```json
{
  "decoded_transactions": 2,
  "first_version": 4365621793,
  "last_version": 4365621794,
  "next_start_version": 4365621795,
  "truncated_error": null,
  "stopped_at_limit": false
}
```

## Import Into RocksDB/ToplingDB Worktrees

After raw chunks are downloaded under `<dataset>/raw`, import the transaction rows into RocksDB from the clean `main` worktree:

```bash
rtk ./scripts/check-backend-isolation.sh rocksdb
rtk cargo build -p decibel-dataset -p decibel-admin --features rocksdb --release --target-dir target/rocksdb
rtk ./scripts/import-real-data.sh rocksdb <dataset> --bin-dir target/rocksdb/release
```

Import ToplingDB from the separate `topingdb` worktree:

```bash
cd /Users/ssyuan/work/project/decibel-hotindex-topingdb
export TOPLINGDB_EASY_MIGRATE_CONF=/path/to/topling_sui.yaml
rtk ./scripts/check-backend-isolation.sh toplingdb
rtk cargo build -p decibel-dataset -p decibel-admin --features toplingsdb --release --target-dir target/topingdb
rtk ./scripts/import-real-data.sh toplingdb <dataset> \
  --bin-dir target/topingdb/release \
  --toplingdb-conf "$TOPLINGDB_EASY_MIGRATE_CONF"
```

For RocksDB vs ToplingDB comparison, use the checksum files produced by those isolated imports:

```bash
rtk cargo run -p decibel-admin -- compare-checksum \
  --left <dataset>/reports/rocksdb-checksums.json \
  --right <dataset>/reports/toplingdb-checksums.json
```

The import scripts intentionally have no `both` mode. Keep RocksDB and ToplingDB imports separate so `TOPLINGDB_EASY_MIGRATE_CONF` and patched `rocksdb` crates cannot leak across backend runs.

This is currently tx-only for real protobuf data: transaction rows are imported, while Decibel fills/orders/positions/builder rows stay empty until event extraction lands.
