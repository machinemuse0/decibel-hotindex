#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/run-benchmark-suite.sh \
    --backend <memory|toplingdb> \
    --dataset <dataset-root> \
    [--bin-dir <path>] \
    [--bench-bin <path>] \
    [--class <serving|ingest|read-under-ingest>] \
    [--workload <name>] \
    [--workloads <csv>] \
    [--iterations <count>] \
    [--warmup <count>] \
    [--access-pattern <sequential|uniform|zipfian>] \
    [--seed <value>] \
    [--db-path <path>] \
    [--toplingdb-path <path>] \
    [--report-dir <path>] \
    [--out <path>] \
    [--summary-out <path>] \
    [--expected-checksum <auto|none|path>] \
    [--checksum-status <status>] \
    [--toplingdb-conf <path>] \
    [--no-summary] \
    [--dry-run]

Purpose:
  Run Decibel HotIndex benchmark reports from a prebuilt decibel-hotindex-bench
  binary, then generate a Markdown summary via the same binary.

Defaults:
  - binary lookup: --bench-bin or DECIBEL_BENCH_BIN, then --bin-dir,
    then target/release/decibel-hotindex-bench, then target/debug/decibel-hotindex-bench
  - class: serving
  - workload: get_tx_by_version
  - iterations: 100000
  - warmup: 1000
  - access pattern: zipfian
  - report dir: <dataset-root>/reports
  - summary out: <report-dir>/BENCHMARK_SUMMARY.md
  - expected checksum: auto

Examples:
  rtk ./scripts/run-benchmark-suite.sh \
    --backend toplingdb \
    --dataset "$DATASET_ROOT" \
    --bin-dir target/topingdb/release \
    --toplingdb-conf "$TOPLINGDB_EASY_MIGRATE_CONF"

Build hint:
  rtk cargo build -p decibel-hotindex-bench --features toplingsdb --release --target-dir target/topingdb

Note:
  RocksDB benchmark runs must be launched from the clean main worktree. This
  topingdb worktree intentionally accepts only memory and ToplingDB.
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

run_cmd() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  if [[ "$DRY_RUN" -eq 1 ]]; then
    return
  fi
  if command -v rtk >/dev/null 2>&1; then
    rtk "$@"
  else
    "$@"
  fi
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

BACKEND=""
DATASET_ROOT=""
BIN_DIR="${DECIBEL_BIN_DIR:-}"
BENCH_BIN="${DECIBEL_BENCH_BIN:-}"
BENCH_CLASS="serving"
WORKLOADS="get_tx_by_version"
ITERATIONS="100000"
WARMUP="1000"
ACCESS_PATTERN="zipfian"
SEED="6840346605343600653"
DB_PATH=""
TOPLINGDB_PATH=""
REPORT_DIR=""
OUT_PATH=""
SUMMARY_OUT=""
EXPECTED_CHECKSUM="auto"
CHECKSUM_STATUS="not_run"
TOPLINGDB_CONF="${TOPLINGDB_EASY_MIGRATE_CONF:-}"
NO_SUMMARY=0
DRY_RUN=0

if [[ $# -eq 0 ]]; then
  usage
  exit 2
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --backend)
      BACKEND="${2:-}"
      shift 2
      ;;
    --dataset | --dataset-root)
      DATASET_ROOT="${2:-}"
      shift 2
      ;;
    --bin-dir)
      BIN_DIR="${2:-}"
      shift 2
      ;;
    --bench-bin)
      BENCH_BIN="${2:-}"
      shift 2
      ;;
    --class)
      BENCH_CLASS="${2:-}"
      shift 2
      ;;
    --workload)
      WORKLOADS="${2:-}"
      shift 2
      ;;
    --workloads)
      WORKLOADS="${2:-}"
      shift 2
      ;;
    --iterations)
      ITERATIONS="${2:-}"
      shift 2
      ;;
    --warmup)
      WARMUP="${2:-}"
      shift 2
      ;;
    --access-pattern)
      ACCESS_PATTERN="${2:-}"
      shift 2
      ;;
    --seed)
      SEED="${2:-}"
      shift 2
      ;;
    --db-path)
      DB_PATH="${2:-}"
      shift 2
      ;;
    --toplingdb-path)
      TOPLINGDB_PATH="${2:-}"
      shift 2
      ;;
    --report-dir)
      REPORT_DIR="${2:-}"
      shift 2
      ;;
    --out)
      OUT_PATH="${2:-}"
      shift 2
      ;;
    --summary-out)
      SUMMARY_OUT="${2:-}"
      shift 2
      ;;
    --expected-checksum)
      EXPECTED_CHECKSUM="${2:-}"
      shift 2
      ;;
    --checksum-status)
      CHECKSUM_STATUS="${2:-}"
      shift 2
      ;;
    --toplingdb-conf)
      TOPLINGDB_CONF="${2:-}"
      shift 2
      ;;
    --no-summary)
      NO_SUMMARY=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      usage
      die "unknown option: $1"
      ;;
  esac
done

[[ -n "$BACKEND" ]] || die "--backend is required"
[[ -n "$DATASET_ROOT" ]] || die "--dataset is required"
if [[ "$DRY_RUN" -eq 0 ]]; then
  [[ -d "$DATASET_ROOT" ]] || die "dataset root does not exist: $DATASET_ROOT"
  [[ -f "$DATASET_ROOT/manifest.json" ]] || die "missing dataset manifest: $DATASET_ROOT/manifest.json"
fi

case "$BACKEND" in
  memory | toplingdb) ;;
  rocksdb)
    die "RocksDB benchmarks must be run from the main worktree"
    ;;
  both | all)
    die "combined backend benchmarks are forbidden; run isolated worktrees separately"
    ;;
  *) die "--backend must be memory or toplingdb" ;;
esac

case "$BENCH_CLASS" in
  serving | ingest | read-under-ingest | read_under_ingest) ;;
  *) die "--class must be serving, ingest, or read-under-ingest" ;;
esac

case "$ACCESS_PATTERN" in
  sequential | uniform | zipfian) ;;
  *) die "--access-pattern must be sequential, uniform, or zipfian" ;;
esac

if [[ -n "$OUT_PATH" && "$WORKLOADS" == *","* ]]; then
  die "--out is only valid for one workload"
fi

if [[ "$BACKEND" == "toplingdb" ]]; then
  if [[ -n "$TOPLINGDB_CONF" ]]; then
    export TOPLINGDB_EASY_MIGRATE_CONF="$TOPLINGDB_CONF"
  fi
  if [[ "$DRY_RUN" -eq 0 ]]; then
    [[ -f "${TOPLINGDB_EASY_MIGRATE_CONF:-}" ]] || die "ToplingDB requires TOPLINGDB_EASY_MIGRATE_CONF or --toplingdb-conf"
    grep -qs "rust-toplingdb" "$repo_root/Cargo.toml" "$repo_root/Cargo.lock" || die "topingdb worktree must contain the rust-toplingdb cargo patch"
  fi
fi

resolve_bench_bin() {
  if [[ -n "$BENCH_BIN" ]]; then
    if [[ "$DRY_RUN" -eq 0 ]]; then
      [[ -x "$BENCH_BIN" ]] || die "missing executable: $BENCH_BIN"
    fi
    return
  fi

  if [[ -n "$BIN_DIR" ]]; then
    BENCH_BIN="$BIN_DIR/decibel-hotindex-bench"
    if [[ "$DRY_RUN" -eq 0 ]]; then
      [[ -x "$BENCH_BIN" ]] || die "missing executable: $BENCH_BIN"
    fi
    return
  fi

  if [[ -x "$repo_root/target/release/decibel-hotindex-bench" ]]; then
    BENCH_BIN="$repo_root/target/release/decibel-hotindex-bench"
    return
  fi

  if [[ -x "$repo_root/target/debug/decibel-hotindex-bench" ]]; then
    BENCH_BIN="$repo_root/target/debug/decibel-hotindex-bench"
    return
  fi

  if [[ "$DRY_RUN" -eq 1 ]]; then
    BENCH_BIN="$repo_root/target/topingdb/release/decibel-hotindex-bench"
    return
  fi

  die "could not find decibel-hotindex-bench; pass --bin-dir or --bench-bin after building"
}

sanitize_name() {
  local value="$1"
  value="${value//\//-}"
  value="${value// /-}"
  echo "$value"
}

backend_db_path() {
  if [[ "$BACKEND" == "memory" ]]; then
    return
  fi

  if [[ -n "$DB_PATH" ]]; then
    echo "$DB_PATH"
    return
  fi

  if [[ "$BENCH_CLASS" != "serving" ]]; then
    return
  fi

  if [[ -n "$TOPLINGDB_PATH" ]]; then
    echo "$TOPLINGDB_PATH"
  else
    echo "$DATASET_ROOT/materialized/toplingdb"
  fi
}

resolve_expected_checksum() {
  local path=""

  case "$EXPECTED_CHECKSUM" in
    none | "")
      return
      ;;
    auto)
      if [[ -f "$DATASET_ROOT/reports/rocksdb-checksums.json" ]]; then
        path="$DATASET_ROOT/reports/rocksdb-checksums.json"
      elif [[ -f "$DATASET_ROOT/reports/toplingdb-checksums.json" ]]; then
        path="$DATASET_ROOT/reports/toplingdb-checksums.json"
      fi
      ;;
    *)
      path="$EXPECTED_CHECKSUM"
      if [[ "$DRY_RUN" -eq 0 ]]; then
        [[ -f "$path" ]] || die "expected checksum file does not exist: $path"
      fi
      ;;
  esac

  if [[ -n "$path" ]]; then
    echo "$path"
  fi
}

join_reports() {
  local joined=""
  local path
  for path in "$@"; do
    if [[ -z "$joined" ]]; then
      joined="$path"
    else
      joined="${joined},${path}"
    fi
  done
  echo "$joined"
}

run_one_benchmark() {
  local workload="$1"
  local report_path="$2"
  local args=(
    run
    --dataset "$DATASET_ROOT"
    --engine "$BACKEND"
    --class "$BENCH_CLASS"
    --iterations "$ITERATIONS"
    --warmup "$WARMUP"
    --out "$report_path"
  )

  if [[ "$BENCH_CLASS" == "serving" || "$BENCH_CLASS" == "read-under-ingest" || "$BENCH_CLASS" == "read_under_ingest" ]]; then
    args+=(--workload "$workload")
    args+=(--access-pattern "$ACCESS_PATTERN" --seed "$SEED")
  fi

  local db_path
  db_path="$(backend_db_path)"
  if [[ -n "$db_path" ]]; then
    if [[ "$DRY_RUN" -eq 0 && "$BENCH_CLASS" == "serving" && ! -d "$db_path" ]]; then
      die "serving DB path does not exist for $BACKEND: $db_path"
    fi
    args+=(--db-path "$db_path")
  fi

  local expected_checksum_path
  expected_checksum_path="$(resolve_expected_checksum)"
  if [[ -n "$expected_checksum_path" ]]; then
    args+=(--expected-checksum "$expected_checksum_path")
  else
    args+=(--checksum-status "$CHECKSUM_STATUS")
  fi

  echo "benchmark: backend=$BACKEND class=$BENCH_CLASS workload=$workload report=$report_path"
  run_cmd "$BENCH_BIN" "${args[@]}"
}

resolve_bench_bin

if [[ -z "$REPORT_DIR" ]]; then
  REPORT_DIR="$DATASET_ROOT/reports"
fi
if [[ -z "$SUMMARY_OUT" ]]; then
  SUMMARY_OUT="$REPORT_DIR/BENCHMARK_SUMMARY.md"
fi

if [[ "$DRY_RUN" -eq 0 ]]; then
  mkdir -p "$REPORT_DIR"
fi

workloads=()
if [[ "$BENCH_CLASS" == "ingest" ]]; then
  workloads=(normalized_replay)
else
  IFS=',' read -r -a workloads <<< "$WORKLOADS"
fi

reports=()
for workload in "${workloads[@]}"; do
  workload="${workload//[[:space:]]/}"
  [[ -n "$workload" ]] || continue

  if [[ -n "$OUT_PATH" ]]; then
    report_path="$OUT_PATH"
  else
    report_workload="$(sanitize_name "$workload")"
    report_class="$(sanitize_name "$BENCH_CLASS")"
    report_path="$REPORT_DIR/bench-${BACKEND}-${report_class}-${report_workload}.json"
  fi

  run_one_benchmark "$workload" "$report_path"
  reports+=("$report_path")
done

if [[ "${#reports[@]}" -eq 0 ]]; then
  die "no benchmark reports were produced"
fi

if [[ "$NO_SUMMARY" -eq 0 ]]; then
  reports_csv="$(join_reports "${reports[@]}")"
  echo "summary: reports=${#reports[@]} out=$SUMMARY_OUT"
  run_cmd "$BENCH_BIN" summarize --reports "$reports_csv" --out "$SUMMARY_OUT"
fi

echo "benchmark suite complete: backend=$BACKEND class=$BENCH_CLASS reports=${#reports[@]}"
if [[ "$NO_SUMMARY" -eq 0 ]]; then
  echo "summary written: $SUMMARY_OUT"
fi
