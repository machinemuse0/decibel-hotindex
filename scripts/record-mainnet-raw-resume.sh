#!/usr/bin/env bash
set -u

usage() {
  cat <<'USAGE'
Usage:
  END_VERSION=<version> scripts/record-mainnet-raw-resume.sh [options]

Options:
  --dataset-bin <path>              Default: target/rocksdb/release/decibel-dataset
  --network <name>                  Default: mainnet
  --endpoint <host:port>            Default: grpc.mainnet.aptoslabs.com:443
  --auth-token-env <name>           Default: APTOS_GRPC_AUTH_TOKEN
  --end-version <version>           Required unless END_VERSION is set
  --out-dir <path>                  Default: /data4/decibel-hotindex/datasets/mainnet-4365621793-4381375639/raw
  --sleep-secs <seconds>            Default: 1800
  --max-attempts <count>            Default: 0 (unlimited)
  --max-raw-bytes <bytes>           Default: 7GiB
  --max-stream-retries <count>      Default: 10
  --chunk-transaction-count <count> Default: 100000
  --batch-size <count>              Default: 500
  --progress-interval-secs <count>  Default: 30
  --key-sample-limit <count>        Default: 1000000
  --raw-format <format>             Default: protobuf-zstd
  --dry-run                         Print the command and exit
  -h, --help

Extra decibel-dataset record options can be appended after --.

Examples:
  END_VERSION=4381375639 scripts/record-mainnet-raw-resume.sh
  scripts/record-mainnet-raw-resume.sh --end-version 4381375639 --sleep-secs 1800
USAGE
}

die() {
  echo "error: $*" >&2
  exit 2
}

timestamp() {
  date '+%Y-%m-%d %H:%M:%S'
}

print_cmd() {
  printf '[%s] +' "$(timestamp)"
  printf ' %q' "$@"
  printf '\n'
}

run_cmd() {
  print_cmd "$@"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    return 0
  fi

  if command -v rtk >/dev/null 2>&1; then
    rtk "$@" &
  else
    "$@" &
  fi
  child_pid=$!
  wait "$child_pid"
  local status=$?
  child_pid=""
  return "$status"
}

is_uint() {
  [[ "$1" =~ ^[0-9]+$ ]]
}

on_signal() {
  stop_requested=1
  if [[ -n "${child_pid:-}" ]]; then
    kill "$child_pid" 2>/dev/null || true
  fi
  if [[ -n "${sleep_pid:-}" ]]; then
    kill "$sleep_pid" 2>/dev/null || true
  fi
}

trap on_signal INT TERM

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

DATASET_BIN="$repo_root/target/rocksdb/release/decibel-dataset"
NETWORK="mainnet"
ENDPOINT="grpc.mainnet.aptoslabs.com:443"
AUTH_TOKEN_ENV="APTOS_GRPC_AUTH_TOKEN"
END_VERSION_VALUE="${END_VERSION:-}"
OUT_DIR="/data4/decibel-hotindex/datasets/mainnet-4365621793-4381375639/raw"
SLEEP_SECS="1800"
MAX_ATTEMPTS="0"
MAX_RAW_BYTES="7GiB"
MAX_STREAM_RETRIES="10"
CHUNK_TRANSACTION_COUNT="100000"
BATCH_SIZE="500"
PROGRESS_INTERVAL_SECS="30"
KEY_SAMPLE_LIMIT="1000000"
RAW_FORMAT="protobuf-zstd"
DRY_RUN=0
EXTRA_ARGS=()
child_pid=""
sleep_pid=""
stop_requested=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dataset-bin)
      DATASET_BIN="${2:-}"
      shift 2
      ;;
    --network)
      NETWORK="${2:-}"
      shift 2
      ;;
    --endpoint)
      ENDPOINT="${2:-}"
      shift 2
      ;;
    --auth-token-env)
      AUTH_TOKEN_ENV="${2:-}"
      shift 2
      ;;
    --end-version)
      END_VERSION_VALUE="${2:-}"
      shift 2
      ;;
    --out-dir)
      OUT_DIR="${2:-}"
      shift 2
      ;;
    --sleep-secs)
      SLEEP_SECS="${2:-}"
      shift 2
      ;;
    --max-attempts)
      MAX_ATTEMPTS="${2:-}"
      shift 2
      ;;
    --max-raw-bytes)
      MAX_RAW_BYTES="${2:-}"
      shift 2
      ;;
    --max-stream-retries)
      MAX_STREAM_RETRIES="${2:-}"
      shift 2
      ;;
    --chunk-transaction-count)
      CHUNK_TRANSACTION_COUNT="${2:-}"
      shift 2
      ;;
    --batch-size)
      BATCH_SIZE="${2:-}"
      shift 2
      ;;
    --progress-interval-secs)
      PROGRESS_INTERVAL_SECS="${2:-}"
      shift 2
      ;;
    --key-sample-limit)
      KEY_SAMPLE_LIMIT="${2:-}"
      shift 2
      ;;
    --raw-format)
      RAW_FORMAT="${2:-}"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --)
      shift
      EXTRA_ARGS+=("$@")
      break
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

[[ -n "$DATASET_BIN" ]] || die "--dataset-bin cannot be empty"
[[ -n "$NETWORK" ]] || die "--network cannot be empty"
[[ -n "$ENDPOINT" ]] || die "--endpoint cannot be empty"
[[ -n "$AUTH_TOKEN_ENV" ]] || die "--auth-token-env cannot be empty"
[[ -n "$END_VERSION_VALUE" ]] || die "--end-version is required, or set END_VERSION"
[[ -n "$OUT_DIR" ]] || die "--out-dir cannot be empty"
is_uint "$SLEEP_SECS" || die "--sleep-secs must be a non-negative integer"
is_uint "$MAX_ATTEMPTS" || die "--max-attempts must be a non-negative integer"
is_uint "$MAX_STREAM_RETRIES" || die "--max-stream-retries must be a non-negative integer"
is_uint "$CHUNK_TRANSACTION_COUNT" || die "--chunk-transaction-count must be a non-negative integer"
is_uint "$BATCH_SIZE" || die "--batch-size must be a non-negative integer"
is_uint "$PROGRESS_INTERVAL_SECS" || die "--progress-interval-secs must be a non-negative integer"
is_uint "$KEY_SAMPLE_LIMIT" || die "--key-sample-limit must be a non-negative integer"

if [[ "$DRY_RUN" -eq 0 ]]; then
  [[ -x "$DATASET_BIN" ]] || die "missing executable: $DATASET_BIN"
  [[ -n "${!AUTH_TOKEN_ENV:-}" ]] || die "environment variable is empty: $AUTH_TOKEN_ENV"
  mkdir -p "$OUT_DIR" || die "failed to create out dir: $OUT_DIR"
fi

build_args() {
  record_args=(
    "$DATASET_BIN" record
    --live
    --network "$NETWORK"
    --endpoint "$ENDPOINT"
    --auth-token-env "$AUTH_TOKEN_ENV"
    --resume
    --end-version "$END_VERSION_VALUE"
    --max-raw-bytes "$MAX_RAW_BYTES"
    --max-stream-retries "$MAX_STREAM_RETRIES"
    --chunk-transaction-count "$CHUNK_TRANSACTION_COUNT"
    --batch-size "$BATCH_SIZE"
    --progress-interval-secs "$PROGRESS_INTERVAL_SECS"
    --key-sample-limit "$KEY_SAMPLE_LIMIT"
    --out-dir "$OUT_DIR"
    --raw-format "$RAW_FORMAT"
  )

  if [[ "${#EXTRA_ARGS[@]}" -gt 0 ]]; then
    record_args+=("${EXTRA_ARGS[@]}")
  fi
}

build_args

attempt=1
while :; do
  echo "[$(timestamp)] record attempt=$attempt network=$NETWORK end_version=$END_VERSION_VALUE out_dir=$OUT_DIR"
  run_cmd "${record_args[@]}"
  status=$?

  if [[ "$status" -eq 0 ]]; then
    echo "[$(timestamp)] record completed successfully"
    exit 0
  fi

  if [[ "$stop_requested" -eq 1 ]]; then
    echo "[$(timestamp)] record stopped after signal; last exit status=$status" >&2
    exit "$status"
  fi

  if [[ "$MAX_ATTEMPTS" -gt 0 && "$attempt" -ge "$MAX_ATTEMPTS" ]]; then
    echo "[$(timestamp)] record failed after $attempt attempt(s); last exit status=$status" >&2
    exit "$status"
  fi

  echo "[$(timestamp)] record exited with status=$status; sleeping ${SLEEP_SECS}s before resume"
  sleep "$SLEEP_SECS" &
  sleep_pid=$!
  wait "$sleep_pid"
  sleep_status=$?
  sleep_pid=""

  if [[ "$stop_requested" -eq 1 ]]; then
    echo "[$(timestamp)] stopped during retry sleep" >&2
    exit 130
  fi
  if [[ "$sleep_status" -ne 0 ]]; then
    exit "$sleep_status"
  fi

  attempt=$((attempt + 1))
done
