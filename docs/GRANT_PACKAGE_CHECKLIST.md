# Grant Package Checklist

This checklist is the M7 release gate for any README benchmark number, grant
update, outreach message, or RocksDB vs ToplingDB comparison.

## Hard Gate

Do not publish benchmark claims unless all of these are true:

- the dataset has a committed or shared `manifest.json` with `dataset_id`,
  network, bounded version range, parser source/commit, counts, and sha256 map
- the raw archive is immutable and can be re-normalized from saved
  `transactions_*.pb.zst` chunks
- manifest sha256 entries validate for raw, normalized, and query corpus
  artifacts before any benchmark report is generated
- RocksDB and ToplingDB were imported from separate worktrees
- materialized backend directories were promoted from successful staging output,
  not reused after a failed or interrupted replay
- both backend checksum JSON files exist and `decibel-admin compare-checksum`
  passes for the same logical schema version
- every benchmark report references the same dataset id and query corpus hash
- `BENCHMARK_SUMMARY.md` states same schema, same dataset, same keyset, and same
  workload
- every benchmark report has `gate.status=pass` and `allow_failures=false`
- every benchmark report has `methodology_status=publishable_candidate`
- every benchmark report was produced by a release binary, not a debug build
- the benchmark environment is recorded: CPU, memory, OS/kernel, filesystem,
  storage path, backend options, and git sha
- ToplingDB native build used the dedicated `topingdb` worktree and its pinned
  `rust-toplingdb` revision
- when `--require-toplingdb` is used, `$DATASET_ROOT/reports/toplingdb-preflight.ok`
  exists and was produced by a successful full `scripts/toplingdb-preflight.sh`
  run from the `topingdb` worktree

If any item is missing, the material may still be used as an engineering smoke
note, but it must not be presented as publishable performance evidence.

## Minimum Artifacts

For a benchmark dataset rooted at `$DATASET_ROOT`, keep these files:

```text
$DATASET_ROOT/manifest.json
$DATASET_ROOT/raw/record_checkpoint.json
$DATASET_ROOT/raw/transactions_<start>_<end>.pb.zst
$DATASET_ROOT/queries/record_keys_manifest.json
$DATASET_ROOT/reports/rocksdb-checksums.json
$DATASET_ROOT/reports/toplingdb-checksums.json
$DATASET_ROOT/reports/toplingdb-preflight.ok
$DATASET_ROOT/reports/bench-rocksdb-*.json
$DATASET_ROOT/reports/bench-toplingdb-*.json
$DATASET_ROOT/reports/BENCHMARK_SUMMARY.md
```

The benchmark summary is not a substitute for the raw artifacts. It is only the
human-readable index over the dataset, checksum, and report evidence.

Run the executable gate before sharing a package:

```bash
rtk ./scripts/toplingdb-preflight.sh \
  --write-ok "$DATASET_ROOT/reports/toplingdb-preflight.ok"

rtk ./scripts/check-grant-package.sh \
  --dataset "$DATASET_ROOT" \
  --require-toplingdb \
  --admin-bin target/rocksdb/release/decibel-admin
```

When `--require-toplingdb` is used, the script always runs
`decibel-admin compare-checksum`. `--admin-bin` is an override; if omitted, the
gate tries `DECIBEL_ADMIN_BIN`, `PATH`, and local target directories before
failing closed.

For internal smoke notes, pass `--allow-engineering-smoke`; do not use that flag
for grant, README, or outreach claims. This flag does not permit
`gate.status=bypassed`; bypassed reports are diagnostic only.

Benchmark reports are smoke by default. Use `--publishable-candidate` on
`scripts/run-benchmark-suite.sh` only for release-built runs that are intended to
enter this package gate.

## Demo Script Narrative

Use this order in demos:

1. Show `manifest.json` and call out `dataset_id`, network, and version range.
2. Show raw chunks and explain that normalization can be reproduced from them.
3. Import RocksDB from `main` and ToplingDB from `topingdb`.
4. Run `compare-checksum` and show `pass` before any performance numbers.
5. Run the same workload list against both materialized backends.
6. Open `BENCHMARK_SUMMARY.md` and read numbers only after the gate evidence is
   visible.

The core message is:

```text
same schema, same dataset, same keyset, same workload, checksum-passed
```

Avoid broader claims such as "ToplingDB is universally faster" or "official
Decibel settlement data". Builder-code metrics remain analytics estimates from
parsed Decibel events.

## Residual Risks To Disclose

- ToplingDB native builds are long-running and belong in a separate cached
  preflight, not the fast RocksDB CI lane. On macOS/arm64, the local
  `topingdb` worktree has produced a full native preflight marker, but that
  marker is only branch-local enablement evidence. Do not publish ToplingDB
  performance claims until the exact benchmark dataset has its own successful
  full `scripts/toplingdb-preflight.sh` marker in `reports/` and passes this
  package gate.
- Full Decibel serving workloads require a bounded range with matching Decibel
  events; tx-only ranges are valid only for raw/protobuf/replay and tx key smoke
  benchmarks.
- Public performance claims still require inspecting each report's environment
  fingerprint and storage state. Missing or `unknown` environment/storage fields
  downgrade the result to engineering smoke.
- Reports produced with `--allow-failures` carry `gate.status=bypassed` and are
  rejected by the package gate.
