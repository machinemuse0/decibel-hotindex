# Milestone 4 Tracking Plan

Milestone 4 makes persistent backends comparable under the same `StorageEngine` contract. The first pass adds a feature-gated RocksDB implementation and a ToplingDB stub while keeping the default workspace build independent of native RocksDB toolchain setup.

## Scope

Included:

- `RocksDbEngine` behind `--features rocksdb`
- RocksDB column family initialization
- Put/get/scan/stats/checksum implementation for the current logical schema
- RocksDB conformance tests gated behind the `rocksdb` feature
- Memory-vs-RocksDB checksum equivalence test gated behind the `rocksdb` feature
- `decibel-dataset replay --engine rocksdb --db-path <path>` behind `--features rocksdb`
- `decibel-admin checksum`
- `decibel-admin compare-checksum`
- `toplingsdb` feature-gated explicit stub

Deferred:

- Real ToplingDB binding integration
- RocksDB/ToplingDB benchmark reports

## M4-01: RocksDB Backend

Status: Completed

Acceptance:

- Opens with required column families.
- Implements point lookup, multi-get, prefix scans, activity scans, stats, and checksums.
- Uses the same key encoding as MemoryEngine.

Verified:

```text
rtk cargo check -p decibel-hotindex-storage --features rocksdb
rtk cargo test -p decibel-hotindex-storage --features rocksdb
```

RocksDB build requires LLVM/libclang on macOS. The local environment is now configured with `LIBCLANG_PATH=/opt/homebrew/opt/llvm/lib`.

## M4-02: ToplingDB Gate

Status: Completed

Acceptance:

- `toplingsdb` feature compiles.
- Attempts to open/use the backend return an explicit unsupported error.
- Default build does not require ToplingDB.

## M4-03: Admin Checksum Commands

Status: Completed

Commands:

```bash
rtk cargo run -p decibel-admin --features rocksdb -- checksum --engine rocksdb --db-path <path> --out <checksums.json>
rtk cargo run -p decibel-admin -- compare-checksum --left <checksums-a.json> --right <checksums-b.json>
```

Acceptance:

- Checksum command emits JSON checksum arrays.
- Compare command fails on any row/hash mismatch.

## M4-04: Dataset Replay to RocksDB

Status: Completed

Command:

```bash
rtk cargo run -p decibel-dataset --features rocksdb -- replay --dataset <dataset> --engine rocksdb --db-path <dataset>/materialized/rocksdb
```

Acceptance:

- Reuses the same manifest hash validation as memory replay.
- Writes normalized rows into RocksDB.
- Prints stats and logical CF checksums.
- Matches MemoryEngine logical CF checksums on the same fixture dataset.

## Current Verified Commands

```bash
rtk cargo fmt --all
rtk cargo check --workspace
rtk cargo test --workspace
rtk cargo check -p decibel-hotindex-storage --features rocksdb
rtk cargo test -p decibel-hotindex-storage --features rocksdb
rtk cargo check -p decibel-hotindex-storage --features toplingsdb
rtk cargo run -p decibel-dataset --features rocksdb -- replay --dataset /private/tmp/decibel-hotindex-m4-smoke --engine rocksdb --db-path /private/tmp/decibel-hotindex-m4-smoke/materialized/rocksdb
rtk cargo run -p decibel-admin --features rocksdb -- checksum --engine rocksdb --db-path /private/tmp/decibel-hotindex-m4-smoke/materialized/rocksdb --out /private/tmp/decibel-hotindex-m4-smoke/reports/rocksdb-checksums.json
rtk cargo run -p decibel-admin -- compare-checksum --left /private/tmp/decibel-hotindex-m4-smoke/reports/rocksdb-checksums.json --right /private/tmp/decibel-hotindex-m4-smoke/reports/rocksdb-checksums.json
```

Results:

- Default workspace: passing.
- ToplingDB stub feature: passing.
- RocksDB feature: passing.
- Memory and RocksDB checksums match on the M4 fixture smoke dataset.
