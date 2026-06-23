#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/import-real-data.sh toplingdb <dataset-root> [options]

Options:
  --bin-dir <path>         Directory containing decibel-dataset and decibel-admin binaries.
                           Default: DECIBEL_BIN_DIR, then target/release, then target/debug.
  --raw-input <path>       Raw .pb.zst chunk or raw directory. Default: <dataset-root>/raw
  --skip-normalize         Reuse existing <dataset-root>/normalized and manifest.json
  --force-normalize        Rebuild normalized tx-only artifacts even if they already exist
  --force                  Remove existing materialized ToplingDB path before replay
  --network <name>         Dataset network metadata. Default: mainnet
  --dataset-id <id>        Dataset id metadata. Default: dataset root basename
  --parser-commit <sha>    Parser commit metadata
  --config <path>          Decibel config for addresses/network defaults
  --toplingdb-conf <path>  Sets TOPLINGDB_EASY_MIGRATE_CONF for ToplingDB

Examples:
  rtk cargo build -p decibel-dataset -p decibel-admin --features toplingsdb --release --target-dir target/topingdb
  rtk ./scripts/import-real-data.sh toplingdb /data/decibel-hotindex/datasets/mainnet-4365621793-4381375638 --bin-dir target/topingdb/release --toplingdb-conf /path/to/topling_sui.yaml

Note:
  RocksDB imports must be run from the clean main worktree. This topingdb
  worktree intentionally accepts only ToplingDB so the rust-toplingdb patch
  cannot be mislabeled as a RocksDB baseline.
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
  if command -v rtk >/dev/null 2>&1; then
    rtk "$@"
  else
    "$@"
  fi
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

default_bin_dir() {
  if [[ -x "$repo_root/target/release/decibel-dataset" && -x "$repo_root/target/release/decibel-admin" ]]; then
    echo "$repo_root/target/release"
    return
  fi
  if [[ -x "$repo_root/target/debug/decibel-dataset" && -x "$repo_root/target/debug/decibel-admin" ]]; then
    echo "$repo_root/target/debug"
    return
  fi
  return 1
}

build_hint() {
  echo "rtk cargo build -p decibel-dataset -p decibel-admin --features toplingsdb --release --target-dir target/topingdb"
}

resolve_bin_dir() {
  local selected="$1"
  if [[ -z "$selected" ]]; then
    selected="$bin_dir"
  fi
  if [[ -z "$selected" ]]; then
    selected="$(default_bin_dir)" || die "could not find compiled decibel binaries; build first with: $(build_hint)"
  fi
  echo "$selected"
}

resolve_binaries() {
  local selected_bin_dir="$1"
  dataset_bin="$selected_bin_dir/decibel-dataset"
  admin_bin="$selected_bin_dir/decibel-admin"
  [[ -x "$dataset_bin" ]] || die "missing executable: $dataset_bin; build first with: $(build_hint)"
  [[ -x "$admin_bin" ]] || die "missing executable: $admin_bin; build first with: $(build_hint)"
}

ensure_toplingdb_isolation() {
  if ! grep -qs "rust-toplingdb" "$repo_root/Cargo.toml" "$repo_root/Cargo.lock"; then
    die "topingdb worktree must contain the rust-toplingdb cargo patch"
  fi
  if [[ -n "$toplingdb_conf" ]]; then
    export TOPLINGDB_EASY_MIGRATE_CONF="$toplingdb_conf"
  fi
  [[ -f "${TOPLINGDB_EASY_MIGRATE_CONF:-}" ]] || die "ToplingDB requires TOPLINGDB_EASY_MIGRATE_CONF or --toplingdb-conf"
}

if [[ $# -lt 2 ]]; then
  usage
  exit 2
fi

backend="$1"
dataset_root="$2"
shift 2

case "$backend" in
  toplingdb) ;;
  rocksdb)
    die "RocksDB imports must be run from the main worktree"
    ;;
  both | all)
    die "combined backend imports are forbidden; run isolated worktrees separately"
    ;;
  *)
    usage
    die "unsupported backend: $backend"
    ;;
esac

raw_input=""
bin_dir="${DECIBEL_BIN_DIR:-}"
dataset_bin=""
admin_bin=""
skip_normalize=0
force_normalize=0
force=0
network="mainnet"
dataset_id=""
parser_commit=""
config_path=""
toplingdb_conf="${TOPLINGDB_EASY_MIGRATE_CONF:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin-dir)
      bin_dir="${2:-}"
      shift 2
      ;;
    --raw-input)
      raw_input="${2:-}"
      shift 2
      ;;
    --skip-normalize)
      skip_normalize=1
      shift
      ;;
    --force-normalize)
      force_normalize=1
      shift
      ;;
    --force)
      force=1
      shift
      ;;
    --network)
      network="${2:-}"
      shift 2
      ;;
    --dataset-id)
      dataset_id="${2:-}"
      shift 2
      ;;
    --parser-commit)
      parser_commit="${2:-}"
      shift 2
      ;;
    --config)
      config_path="${2:-}"
      shift 2
      ;;
    --toplingdb-conf)
      toplingdb_conf="${2:-}"
      shift 2
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

[[ -d "$dataset_root" ]] || die "dataset root does not exist: $dataset_root"

if [[ -z "$raw_input" ]]; then
  raw_input="$dataset_root/raw"
fi

ensure_toplingdb_isolation

selected_bin_dir="$(resolve_bin_dir "$bin_dir")"
resolve_binaries "$selected_bin_dir"

normalize_dataset() {
  if [[ "$skip_normalize" -eq 1 ]]; then
    [[ -f "$dataset_root/manifest.json" ]] || die "--skip-normalize requires $dataset_root/manifest.json"
    [[ -f "$dataset_root/normalized/txs.ndjson" ]] || die "--skip-normalize requires $dataset_root/normalized/txs.ndjson"
    return
  fi

  if [[ "$force_normalize" -eq 0 && -f "$dataset_root/manifest.json" && -f "$dataset_root/normalized/txs.ndjson" ]]; then
    echo "normalized dataset already exists; reuse it or pass --force-normalize to rebuild"
    return
  fi

  [[ -e "$raw_input" ]] || die "raw input does not exist: $raw_input"

  args=(
    "$dataset_bin" normalize
    --input "$raw_input"
    --out-dir "$dataset_root/normalized"
    --format protobuf-zstd
    --network "$network"
  )
  if [[ -n "$dataset_id" ]]; then
    args+=(--dataset-id "$dataset_id")
  fi
  if [[ -n "$parser_commit" ]]; then
    args+=(--parser-commit "$parser_commit")
  fi
  if [[ -n "$config_path" ]]; then
    args+=(--config "$config_path")
  fi

  run_cmd "${args[@]}"
}

import_toplingdb() {
  local db_path="$dataset_root/materialized/toplingdb"
  local checksum_path="$dataset_root/reports/toplingdb-checksums.json"

  if [[ -e "$db_path" ]]; then
    if [[ "$force" -eq 1 ]]; then
      rm -rf "$db_path"
    else
      die "materialized DB path already exists: $db_path (pass --force to rebuild)"
    fi
  fi

  mkdir -p "$dataset_root/materialized" "$dataset_root/reports"

  run_cmd "$dataset_bin" replay \
    --dataset "$dataset_root" \
    --engine toplingdb \
    --db-path "$db_path"

  run_cmd "$admin_bin" checksum \
    --engine toplingdb \
    --db-path "$db_path" \
    --out "$checksum_path"
}

normalize_dataset
import_toplingdb

echo "import complete: backend=toplingdb dataset=$dataset_root"
