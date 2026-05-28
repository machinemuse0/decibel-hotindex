# Milestone 2 Tracking Plan

Last updated: 2026-05-27

Milestone 2 establishes the offline dataset pipeline. It intentionally avoids live Aptos gRPC implementation while making local synthetic datasets replayable through the shared `StorageEngine` contract.

## Scope

Included:

- `decibel-dataset synthetic`
- `decibel-dataset build-query-corpus`
- `decibel-dataset replay --engine memory`
- `decibel-dataset normalize` skeleton
- `decibel-dataset record` skeleton
- manifest generation with sha256 map
- synthetic round-trip tests

Excluded:

- real Aptos Transaction Stream recording
- protobuf decoding
- zstd read/write implementation
- RocksDB/ToplingDB replay
- parser adapter against Decibel ABI

## Task Board

### M2-01: Synthetic Dataset Command

Status: Completed

Command:

```bash
rtk cargo run -p decibel-dataset -- synthetic --out /private/tmp/decibel-hotindex-m2-smoke --events 24
```

Acceptance:

- Writes `manifest.json`.
- Writes normalized tx/event/fill/order/position/builder rows.
- Manifest includes counts and sha256 hashes.

### M2-02: Query Corpus Command

Status: Completed

Command:

```bash
rtk cargo run -p decibel-dataset -- build-query-corpus --events /private/tmp/decibel-hotindex-m2-smoke/normalized/events.ndjson --out-dir /private/tmp/decibel-hotindex-m2-smoke/queries --seed 42
```

Acceptance:

- Writes hit-capable query corpus files.
- Uses versions, markets, accounts, and builder addresses present in normalized events.
- Produces split query files plus `mixed_dashboard.ndjson`.
- Updates manifest query hashes when run against `<dataset>/queries`.

### M2-03: Memory Replay Command

Status: Completed

Command:

```bash
rtk cargo run -p decibel-dataset -- replay --dataset /private/tmp/decibel-hotindex-m2-smoke --engine memory
```

Acceptance:

- Replays txs/events/fills/orders/positions/builder rows into `MemoryEngine`.
- Prints stats and per-logical-CF checksums.
- Validates manifest sha256 entries before replay.

### M2-04: Normalize Skeleton

Status: Completed

Command:

```bash
rtk cargo run -p decibel-dataset -- normalize --input <raw.pb.zst> --out-dir <dataset>/normalized
```

Acceptance:

- Fails fast when `--input` is not readable.
- Creates output directory.
- Writes `parse_warnings.log` stating raw protobuf decoding is pending.

### M2-05: Record Skeleton

Status: Completed

Command:

```bash
rtk cargo run -p decibel-dataset -- record --live --network mainnet --endpoint grpc.mainnet.aptoslabs.com:443 --auth-token-env APTOS_GRPC_AUTH_TOKEN --start-version 4365621793 --end-version 4365622793 --raw-format protobuf-zstd --out-dir <dataset>/raw
```

Acceptance:

- Creates output directory.
- Writes `record_checkpoint.json`.
- Records only whether an auth token was supplied; never writes token material.
- Keeps real gRPC implementation deferred to the mainnet recording milestone.

## Verification

```bash
rtk cargo fmt --all
rtk cargo check --workspace
rtk cargo test --workspace
```

Latest result:

```text
cargo test: 16 passed (10 suites, 0.01s)
```

## Notes

- M2 synthetic files use plain `ndjson` because they are small smoke artifacts.
- Real mainnet raw archives still target length-delimited Aptos Transaction protobuf plus zstd chunks.
- RocksDB/ToplingDB materialization is intentionally deferred until persistent backend work.
