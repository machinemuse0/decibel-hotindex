#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
compat_root="$repo_root/build-support/toplingdb-macos"

export PATH="$compat_root/bin:$PATH"

if [[ "$(uname -s)" == "Darwin" ]]; then
  export ROCKSDB_DISABLE_GFLAGS="${ROCKSDB_DISABLE_GFLAGS:-1}"
  export ROCKSDB_DISABLE_ZLIB="${ROCKSDB_DISABLE_ZLIB:-1}"
  export WITH_TOPLING_DCOMPACT="${WITH_TOPLING_DCOMPACT:-0}"
  export WITH_TOPLING_ROCKS="${WITH_TOPLING_ROCKS:-0}"
  libcxx_removed_function_flag="-D_LIBCPP_ENABLE_CXX17_REMOVED_UNARY_BINARY_FUNCTION"
  if [[ " ${EXTRA_CXXFLAGS:-} " != *" $libcxx_removed_function_flag "* ]]; then
    export EXTRA_CXXFLAGS="${EXTRA_CXXFLAGS:-} $libcxx_removed_function_flag"
  fi

  case " $* " in
    *"toplingsdb"*|*"--all-features"*)
      cargo fetch --locked >/dev/null
      "$repo_root/scripts/patch-rust-toplingdb-macos.sh"
      ;;
  esac
fi

if [[ -n "${CPATH:-}" ]]; then
  export CPATH="$compat_root/include:$CPATH"
else
  export CPATH="$compat_root/include"
fi

exec cargo "$@"
