#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/toplingdb-preflight.sh [--target-dir <path>] [--skip-release-build] [--write-ok <path>]

Runs the dedicated ToplingDB native preflight for the topingdb worktree.

Checks:
  1. backend isolation guard for the topingdb branch/config
  2. Rust feature check for decibel-hotindex-storage --features toplingsdb
  3. Rust feature check for decibel-hotindex-bench --features toplingsdb
  4. release benchmark binary build/link unless --skip-release-build is set

The release build is intentionally separate from the fast RocksDB CI lane. On
macOS/arm64, upstream rust-toplingdb may compile Rust feature checks but still
fail to produce the native librocksdb dylib needed for final linking; this
script reports that as a preflight failure instead of letting the project
mistake a cargo check pass for a usable native benchmark binary.
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="$repo_root/target/toplingdb-preflight"
skip_release_build=0
write_ok_path=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target-dir)
      target_dir="${2:-}"
      shift 2
      ;;
    --skip-release-build)
      skip_release_build=1
      shift
      ;;
    --write-ok)
      write_ok_path="${2:-}"
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

if [[ -n "$write_ok_path" && "$skip_release_build" -eq 1 ]]; then
  die "--write-ok requires a full release build; remove --skip-release-build"
fi

run_cmd() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

latest_librocksdb_stderr() {
  find "$target_dir" -path '*/build/librocksdb-sys-*/stderr' -type f -print 2>/dev/null |
    sort |
    tail -1
}

run_cmd "$repo_root/scripts/check-backend-isolation.sh" toplingdb
run_cmd "$repo_root/scripts/toplingdb-cargo.sh" check -p decibel-hotindex-storage --features toplingsdb --target-dir "$target_dir"
run_cmd "$repo_root/scripts/toplingdb-cargo.sh" check -p decibel-hotindex-bench --features toplingsdb --target-dir "$target_dir"

if [[ "$skip_release_build" -eq 1 ]]; then
  echo "toplingdb preflight partial ok: release build skipped target_dir=$target_dir"
  exit 0
fi

if ! run_cmd "$repo_root/scripts/toplingdb-cargo.sh" build -p decibel-hotindex-bench --features toplingsdb --release --target-dir "$target_dir"; then
  stderr_path="$(latest_librocksdb_stderr || true)"
  if [[ -n "$stderr_path" ]]; then
    echo "ToplingDB native release build failed; librocksdb-sys stderr: $stderr_path" >&2
    echo "Last native stderr lines:" >&2
    tail -80 "$stderr_path" >&2 || true
  fi
  die "ToplingDB native release build/link failed"
fi

bench_bin="$target_dir/release/decibel-hotindex-bench"
[[ -x "$bench_bin" ]] || die "release benchmark binary was not produced: $bench_bin"

if [[ -n "$write_ok_path" ]]; then
  mkdir -p "$(dirname "$write_ok_path")"
  tmp_ok="$write_ok_path.tmp.$$"
  {
    echo "status=pass"
    echo "branch=$(git -C "$repo_root" branch --show-current)"
    echo "git_sha=$(git -C "$repo_root" rev-parse HEAD)"
    echo "target_dir=$target_dir"
    echo "bench_bin=$bench_bin"
    echo "toplingdb_config=$TOPLINGDB_EASY_MIGRATE_CONF"
    echo "timestamp_utc=$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  } >"$tmp_ok"
  mv "$tmp_ok" "$write_ok_path"
  echo "toplingdb native preflight marker written: $write_ok_path"
fi

echo "toplingdb native preflight ok: $bench_bin"
