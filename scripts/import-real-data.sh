#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/import-real-data.sh <rocksdb|toplingdb|both> <dataset-root> [options]

Options:
  --raw-input <path>       Raw .pb.zst chunk or raw directory. Default: <dataset-root>/raw
  --skip-normalize         Reuse existing <dataset-root>/normalized and manifest.json
  --force-normalize        Rebuild normalized tx-only artifacts even if they already exist
  --force                  Remove existing materialized backend DB path before replay
  --network <name>         Dataset network metadata. Default: mainnet
  --dataset-id <id>        Dataset id metadata. Default: dataset root basename
  --parser-commit <sha>    Parser commit metadata
  --config <path>          Decibel config for addresses/network defaults
  --toplingdb-conf <path>  Sets TOPLINGDB_EASY_MIGRATE_CONF for ToplingDB

Examples:
  scripts/import-real-data.sh rocksdb /data/decibel-hotindex/datasets/mainnet-4365621793-4381375638
  scripts/import-real-data.sh both /data/decibel-hotindex/datasets/mainnet-4365621793-4381375638 --force
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

if [[ $# -lt 2 ]]; then
  usage
  exit 2
fi

backend="$1"
dataset_root="$2"
shift 2

case "$backend" in
  rocksdb | toplingdb | both) ;;
  *)
    usage
    die "unsupported backend: $backend"
    ;;
esac

raw_input=""
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
    cargo run -p decibel-dataset -- normalize
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

import_backend() {
  local target_backend="$1"
  local feature="$target_backend"
  local db_path="$dataset_root/materialized/$target_backend"
  local checksum_path="$dataset_root/reports/${target_backend}-checksums.json"

  if [[ "$target_backend" == "toplingdb" ]]; then
    feature="toplingsdb"
    if [[ -n "$toplingdb_conf" ]]; then
      export TOPLINGDB_EASY_MIGRATE_CONF="$toplingdb_conf"
    fi
    [[ -f "${TOPLINGDB_EASY_MIGRATE_CONF:-}" ]] || die "ToplingDB requires TOPLINGDB_EASY_MIGRATE_CONF or --toplingdb-conf"
  fi

  if [[ -e "$db_path" ]]; then
    if [[ "$force" -eq 1 ]]; then
      rm -rf "$db_path"
    else
      die "materialized DB path already exists: $db_path (pass --force to rebuild)"
    fi
  fi

  mkdir -p "$dataset_root/materialized" "$dataset_root/reports"

  run_cmd cargo run -p decibel-dataset --features "$feature" -- replay \
    --dataset "$dataset_root" \
    --engine "$target_backend" \
    --db-path "$db_path"

  run_cmd cargo run -p decibel-admin --features "$feature" -- checksum \
    --engine "$target_backend" \
    --db-path "$db_path" \
    --out "$checksum_path"
}

normalize_dataset

case "$backend" in
  rocksdb)
    import_backend rocksdb
    ;;
  toplingdb)
    import_backend toplingdb
    ;;
  both)
    import_backend rocksdb
    import_backend toplingdb
    run_cmd cargo run -p decibel-admin -- compare-checksum \
      --left "$dataset_root/reports/rocksdb-checksums.json" \
      --right "$dataset_root/reports/toplingdb-checksums.json"
    ;;
esac

echo "import complete: backend=$backend dataset=$dataset_root"
