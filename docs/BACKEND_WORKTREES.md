# Backend Worktrees

Decibel HotIndex keeps RocksDB and ToplingDB in separate Git worktrees so a
patched `rocksdb` crate, `TOPLINGDB_EASY_MIGRATE_CONF`, and compiled binaries
cannot leak into the baseline run.

## Layout

```text
/Users/ssyuan/work/project/decibel-hotindex
  branch: main
  backend: memory + rocksdb

/Users/ssyuan/work/project/decibel-hotindex-topingdb
  branch: topingdb
  backend: memory + toplingdb
```

The sync direction is `main -> topingdb`. Do not merge the ToplingDB cargo patch
back into `main`.

## Create The ToplingDB Worktree

From the main worktree:

```bash
rtk git worktree add -b topingdb ../decibel-hotindex-topingdb main
```

The ToplingDB branch must carry this root `Cargo.toml` patch:

```toml
[patch.crates-io]
rocksdb = { git = "https://github.com/topling/rust-toplingdb", rev = "5390ceb77bebba1bf2720b052f83f82b864d64df" }
```

After changing the patch, refresh the lockfile from the ToplingDB worktree:

```bash
rtk cargo update -p rocksdb
```

## Isolation Checks

Run this before building either backend:

```bash
# main worktree
rtk ./scripts/check-backend-isolation.sh rocksdb

# topingdb worktree
export TOPLINGDB_EASY_MIGRATE_CONF=/path/to/topling_sui.yaml
rtk ./scripts/check-backend-isolation.sh toplingdb
```

`main` must not mention `rust-toplingdb` in `Cargo.toml` or `Cargo.lock`.
`topingdb` must mention it and must have a readable ToplingDB config.

## Build And Import

RocksDB baseline from `main`:

```bash
rtk cargo build -p decibel-dataset -p decibel-admin --features rocksdb --release --target-dir target/rocksdb
rtk ./scripts/import-real-data.sh rocksdb "$DATASET_ROOT" --bin-dir target/rocksdb/release
```

ToplingDB from `topingdb`:

```bash
export TOPLINGDB_EASY_MIGRATE_CONF=/path/to/topling_sui.yaml
rtk ./scripts/toplingdb-cargo.sh build -p decibel-dataset -p decibel-admin --features toplingsdb --release --target-dir target/topingdb
rtk ./scripts/import-real-data.sh toplingdb "$DATASET_ROOT" \
  --bin-dir target/topingdb/release \
  --toplingdb-conf "$TOPLINGDB_EASY_MIGRATE_CONF"
```

On macOS, always use `scripts/toplingdb-cargo.sh` for ToplingDB feature builds.
The wrapper provides local compatibility for the pinned `rust-toplingdb`
checkout:

- adds a local `endian.h` shim for Darwin
- disables the Linux-only Topling dcompact target with `WITH_TOPLING_DCOMPACT=0`
- applies an idempotent `fast_popcount_trail` overload patch to the Cargo git
  checkout after `cargo fetch --locked`

The patch is deliberately narrow and lives in `scripts/patch-rust-toplingdb-macos.sh`.
Remove it once the pinned upstream `rust-toplingdb` revision builds cleanly on
Darwin without local compatibility.

ToplingDB native builds are long-running because the upstream `librocksdb-sys`
build invokes `make shared_lib` with LTO. Do not put this build in the fast
RocksDB CI lane; run it as a separate cached job or a release/benchmark
preflight.

Use the executable preflight from the `topingdb` worktree:

```bash
rtk ./scripts/toplingdb-preflight.sh --skip-release-build
rtk ./scripts/toplingdb-preflight.sh
rtk ./scripts/toplingdb-preflight.sh --write-ok "$DATASET_ROOT/reports/toplingdb-preflight.ok"
```

The partial mode verifies backend isolation and Rust `toplingsdb` feature checks
for storage and benchmark crates. The full mode additionally requires the
release benchmark binary to link against the native ToplingDB/RocksDB dylib.
Use `--write-ok` only for full preflight runs; it writes the pass marker consumed
by `scripts/check-grant-package.sh --require-toplingdb`.

Current macOS/arm64 status: partial preflight passes, but full release preflight
still does not produce `librocksdb.dylib`. The local compatibility patch now
covers the earlier GNU `time`, x86-only `cpuid.h`, Linux-only `madvise`/futex,
`process_vm_*`, `aioinit`, and fiber-AIO Darwin compile failures, but the full
preflight has not completed with a native library. Treat ToplingDB performance
numbers as unavailable until the full preflight succeeds and writes
`toplingdb-preflight.ok`.

## Checksum Comparison

The checksum JSON shape is backend-neutral. Compare after both imports complete:

```bash
rtk cargo run -p decibel-admin -- compare-checksum \
  --left "$DATASET_ROOT/reports/rocksdb-checksums.json" \
  --right "$DATASET_ROOT/reports/toplingdb-checksums.json"
```

Do not use a combined backend mode. Build and import each backend from its own
worktree.

## Benchmarks

RocksDB from `main`:

```bash
rtk cargo build -p decibel-hotindex-bench --features rocksdb --release --target-dir target/rocksdb
rtk ./scripts/run-benchmark-suite.sh \
  --backend rocksdb \
  --dataset "$DATASET_ROOT" \
  --bin-dir target/rocksdb/release \
  --workloads get_tx_by_version,multi_get_tx_versions_100
```

ToplingDB from `topingdb`:

```bash
rtk ./scripts/toplingdb-cargo.sh build -p decibel-hotindex-bench --features toplingsdb --release --target-dir target/topingdb
rtk ./scripts/run-benchmark-suite.sh \
  --backend toplingdb \
  --dataset "$DATASET_ROOT" \
  --bin-dir target/topingdb/release \
  --workloads get_tx_by_version,multi_get_tx_versions_100 \
  --expected-checksum "$DATASET_ROOT/reports/rocksdb-checksums.json" \
  --toplingdb-conf "$TOPLINGDB_EASY_MIGRATE_CONF"
```

Real protobuf normalization is currently tx-only, so use tx point and multi-get
workloads for real-data smoke runs until Decibel event extraction lands.

## Updating ToplingDB From Main

After main receives shared schema, ingest, dataset, or benchmark changes:

```bash
cd /Users/ssyuan/work/project/decibel-hotindex-topingdb
rtk git fetch
rtk git merge main
```

Resolve conflicts by keeping shared code from `main` and preserving only the
ToplingDB branch-specific cargo patch, backend feature, and CLI/script
ToplingDB entrypoints.
