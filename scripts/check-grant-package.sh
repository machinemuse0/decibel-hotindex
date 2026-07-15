#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/check-grant-package.sh --dataset <dataset-root> [options]

Options:
  --require-toplingdb          Require ToplingDB checksum and benchmark reports.
  --toplingdb-preflight-ok <path>
                                Require a full native ToplingDB preflight marker.
  --admin-bin <path>           Run decibel-admin compare-checksum when both checksum files exist.
  --allow-engineering-smoke    Permit reports marked engineering_smoke_not_publishable.

This is the M7 release gate for README/grant/outreach benchmark packages.
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

require_file() {
  local path="$1"
  [[ -f "$path" ]] || die "missing required file: $path"
}

require_glob() {
  local pattern="$1"
  compgen -G "$pattern" >/dev/null || die "missing required artifact matching: $pattern"
}

require_report_field() {
  local report="$1"
  local pattern="$2"
  local field="$3"
  grep -Eq "$pattern" "$report" || die "benchmark report missing required field $field: $report"
}

require_gate_pass() {
  local report="$1"
  awk '
    /"gate"[[:space:]]*:/ {
      in_gate = 1
      next
    }
    in_gate && /"status"[[:space:]]*:[[:space:]]*"pass"/ {
      status_pass = 1
    }
    in_gate && /"allow_failures"[[:space:]]*:[[:space:]]*false/ {
      allow_false = 1
    }
    in_gate && /"failures"[[:space:]]*:[[:space:]]*\[/ {
      has_failures = 1
    }
    in_gate && /^  }[,]*$/ {
      in_gate = 0
    }
    END {
      exit (status_pass && allow_false && has_failures) ? 0 : 1
    }
  ' "$report" || die "benchmark report gate must be pass with allow_failures=false: $report"
}

require_toplingdb_preflight_marker() {
  local marker="$1"
  require_file "$marker"
  grep -qx 'status=pass' "$marker" || die "ToplingDB preflight marker must contain status=pass: $marker"
  grep -Eq '^branch=topingdb$' "$marker" || die "ToplingDB preflight marker must come from topingdb branch: $marker"
  grep -Eq '^git_sha=[0-9a-f]{40}$' "$marker" || die "ToplingDB preflight marker missing git_sha: $marker"
  grep -Eq '^bench_bin=.+' "$marker" || die "ToplingDB preflight marker missing bench_bin: $marker"
  grep -Eq '^toplingdb_config=.+' "$marker" || die "ToplingDB preflight marker missing toplingdb_config: $marker"
  grep -Eq '^timestamp_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' "$marker" || \
    die "ToplingDB preflight marker missing timestamp_utc: $marker"
}

check_report_evidence() {
  local report="$1"

  require_report_field "$report" '"gate"[[:space:]]*:' "gate"
  require_report_field "$report" '"allow_failures"[[:space:]]*:' "gate.allow_failures"
  require_report_field "$report" '"failures"[[:space:]]*:' "gate.failures"
  require_gate_pass "$report"
  require_report_field "$report" '"environment"[[:space:]]*:' "environment"
  require_report_field "$report" '"cpu_model"[[:space:]]*:' "environment.cpu_model"
  require_report_field "$report" '"total_memory_bytes"[[:space:]]*:' "environment.total_memory_bytes"
  require_report_field "$report" '"kernel"[[:space:]]*:' "environment.kernel"
  require_report_field "$report" '"filesystem"[[:space:]]*:' "environment.filesystem"
  require_report_field "$report" '"mount_point"[[:space:]]*:' "environment.mount_point"
  require_report_field "$report" '"git_sha"[[:space:]]*:' "environment.git_sha"
  require_report_field "$report" '"git_dirty"[[:space:]]*:' "environment.git_dirty"
  require_report_field "$report" '"ulimit_open_files"[[:space:]]*:' "environment.ulimit_open_files"
  require_report_field "$report" '"storage"[[:space:]]*:' "storage"
  require_report_field "$report" '"compaction"[[:space:]]*:' "storage.compaction"
  require_report_field "$report" '"cache"[[:space:]]*:' "storage.cache"
  require_report_field "$report" '"backend_options"[[:space:]]*:' "storage.backend_options"
  require_report_field "$report" '"performed"[[:space:]]*:' "storage.compaction.performed"
  require_report_field "$report" '"state"[[:space:]]*:' "storage.cache.state"

  if [[ "$allow_engineering_smoke" -eq 0 ]]; then
    if grep -Eq '"(cpu_model|kernel|filesystem|mount_point|git_sha|ulimit_open_files)"[[:space:]]*:[[:space:]]*"unknown"' "$report"; then
      die "benchmark report has unknown environment fields: $report"
    fi
    if grep -Eq '"total_memory_bytes"[[:space:]]*:[[:space:]]*null' "$report"; then
      die "benchmark report has null environment.total_memory_bytes: $report"
    fi
    if grep -Eq '"path"[[:space:]]*:[[:space:]]*""' "$report"; then
      die "benchmark report has empty storage path: $report"
    fi
  fi
}

dataset_root=""
require_toplingdb=0
toplingdb_preflight_ok=""
admin_bin=""
allow_engineering_smoke=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dataset | --dataset-root)
      dataset_root="${2:-}"
      shift 2
      ;;
    --require-toplingdb)
      require_toplingdb=1
      shift
      ;;
    --toplingdb-preflight-ok)
      toplingdb_preflight_ok="${2:-}"
      shift 2
      ;;
    --admin-bin)
      admin_bin="${2:-}"
      shift 2
      ;;
    --allow-engineering-smoke)
      allow_engineering_smoke=1
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

[[ -n "$dataset_root" ]] || die "--dataset is required"
[[ -d "$dataset_root" ]] || die "dataset root does not exist: $dataset_root"

require_file "$dataset_root/manifest.json"
require_file "$dataset_root/raw/record_checkpoint.json"
require_glob "$dataset_root/raw/transactions_*.pb.zst"
require_file "$dataset_root/reports/rocksdb-checksums.json"
require_glob "$dataset_root/reports/bench-rocksdb-*.json"
require_file "$dataset_root/reports/BENCHMARK_SUMMARY.md"
grep -q "same schema, same dataset, same keyset, same workload" "$dataset_root/reports/BENCHMARK_SUMMARY.md" || \
  die "BENCHMARK_SUMMARY.md must state same schema, same dataset, same keyset, same workload"

if [[ -d "$dataset_root/queries" ]]; then
  require_glob "$dataset_root/queries/*.ndjson"
else
  die "missing query corpus directory: $dataset_root/queries"
fi

if [[ "$require_toplingdb" -eq 1 ]]; then
  require_file "$dataset_root/reports/toplingdb-checksums.json"
  require_glob "$dataset_root/reports/bench-toplingdb-*.json"
  if [[ -z "$toplingdb_preflight_ok" ]]; then
    toplingdb_preflight_ok="$dataset_root/reports/toplingdb-preflight.ok"
  fi
  require_toplingdb_preflight_marker "$toplingdb_preflight_ok"
fi

if grep -R '"errors"[[:space:]]*:[[:space:]]*[1-9]' "$dataset_root"/reports/bench-*.json >/dev/null; then
  die "one or more benchmark reports contain non-zero errors"
fi

if grep -R '"status"[[:space:]]*:[[:space:]]*"fail"' "$dataset_root"/reports/bench-*.json >/dev/null; then
  die "one or more benchmark reports contain checksum fail status"
fi

if [[ "$allow_engineering_smoke" -eq 0 ]]; then
  if grep -R 'engineering_smoke_not_publishable\|engineering smoke benchmark only' "$dataset_root"/reports/bench-*.json "$dataset_root/reports/BENCHMARK_SUMMARY.md" >/dev/null; then
    die "reports are marked engineering smoke; pass --allow-engineering-smoke only for internal notes"
  fi
fi

for report in "$dataset_root"/reports/bench-*.json; do
  [[ -f "$report" ]] || continue
  check_report_evidence "$report"
done

if [[ -n "$admin_bin" && "$require_toplingdb" -eq 1 ]]; then
  [[ -x "$admin_bin" ]] || die "missing executable: $admin_bin"
  "$admin_bin" compare-checksum \
    --left "$dataset_root/reports/rocksdb-checksums.json" \
    --right "$dataset_root/reports/toplingdb-checksums.json"
fi

echo "grant package gate ok: dataset=$dataset_root require_toplingdb=$require_toplingdb"
