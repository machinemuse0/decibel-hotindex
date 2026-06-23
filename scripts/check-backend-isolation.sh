#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/check-backend-isolation.sh <rocksdb|toplingdb>

Checks:
  rocksdb
    - current branch is main
    - Cargo.toml/Cargo.lock do not mention rust-toplingdb

  toplingdb
    - current branch is topingdb
    - Cargo.toml or Cargo.lock mentions rust-toplingdb
    - TOPLINGDB_EASY_MIGRATE_CONF points to a readable file
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

git_cmd() {
  if command -v rtk >/dev/null 2>&1; then
    rtk git "$@"
  else
    git "$@"
  fi
}

if [[ $# -ne 1 ]]; then
  usage
  exit 2
fi

backend="$1"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
branch="$(git_cmd -C "$repo_root" branch --show-current)"

has_toplingdb_patch() {
  grep -qs "rust-toplingdb" "$repo_root/Cargo.toml" "$repo_root/Cargo.lock"
}

case "$backend" in
  rocksdb)
    [[ "$branch" == "main" ]] || die "RocksDB baseline must run from branch main; current branch is ${branch:-detached}"
    if has_toplingdb_patch; then
      die "RocksDB baseline worktree must not contain rust-toplingdb patch references"
    fi
    echo "backend isolation ok: rocksdb branch=$branch"
    ;;
  toplingdb)
    [[ "$branch" == "topingdb" ]] || die "ToplingDB backend must run from branch topingdb; current branch is ${branch:-detached}"
    if ! has_toplingdb_patch; then
      die "ToplingDB worktree must contain the rust-toplingdb cargo patch"
    fi
    [[ -n "${TOPLINGDB_EASY_MIGRATE_CONF:-}" ]] || die "TOPLINGDB_EASY_MIGRATE_CONF is required for ToplingDB"
    [[ -f "$TOPLINGDB_EASY_MIGRATE_CONF" ]] || die "TOPLINGDB_EASY_MIGRATE_CONF is not a readable file: $TOPLINGDB_EASY_MIGRATE_CONF"
    echo "backend isolation ok: toplingdb branch=$branch config=$TOPLINGDB_EASY_MIGRATE_CONF"
    ;;
  *)
    usage
    die "unsupported backend: $backend"
    ;;
esac
