#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  exit 0
fi

cargo_home="${CARGO_HOME:-$HOME/.cargo}"
marker="DECIBEL_HOTINDEX_MACOS_FAST_POPCOUNT_TRAIL_OVERLOAD"
queue_marker="DECIBEL_HOTINDEX_MACOS_QUEUE_BACK_CONST_CAST"
cpuid_marker="DECIBEL_HOTINDEX_MACOS_ARM64_SKIP_CPUID"
hugepage_marker="DECIBEL_HOTINDEX_MACOS_MADV_HUGEPAGE_GUARD"
zpath_marker="DECIBEL_HOTINDEX_MACOS_ZPATH_LINK_SIGNATURE"
futex_marker="DECIBEL_HOTINDEX_MACOS_FUTEX_FALLBACK"
localtime_marker="DECIBEL_HOTINDEX_MACOS_LOCALTIME_CASTS"
process_marker="DECIBEL_HOTINDEX_MACOS_PROCESS_FALLBACKS"
thread_local_marker="DECIBEL_HOTINDEX_MACOS_ALWAYS_INLINE_FALLBACK"
fiber_aio_marker="DECIBEL_HOTINDEX_MACOS_FIBER_AIO_POSIX"
vm_util_marker="DECIBEL_HOTINDEX_MACOS_MADV_POPULATE_GUARD"
vm_util_defs_marker="DECIBEL_HOTINDEX_MACOS_VM_UTIL_DARWIN_DEFS"
aioinit_marker="DECIBEL_HOTINDEX_MACOS_AIOINIT_GUARD"
buildrs_marker="DECIBEL_HOTINDEX_MACOS_BUILD_RS_DYLIB_LTO"
buildrs_lib_marker="DECIBEL_HOTINDEX_MACOS_BUILD_RS_REQUIRE_NATIVE_LIB"
rockside_prctl_marker="DECIBEL_HOTINDEX_MACOS_ROCKSIDE_PRCTL_GUARD"
topling_obj_list_marker="DECIBEL_HOTINDEX_MACOS_TOPLING_OBJ_LIST_PREREQ"
topling_obj_list_space_marker="DECIBEL_HOTINDEX_MACOS_TOPLING_OBJ_LIST_STRIP"
topling_darwin_shared_marker="DECIBEL_HOTINDEX_MACOS_TOPLING_SHARED_LINK"
topling_core_lto_marker="DECIBEL_HOTINDEX_MACOS_TOPLING_CORE_NO_LTO"
atomic_marker="DECIBEL_HOTINDEX_MACOS_NO_LIBATOMIC"
dbformat_inline_marker="DECIBEL_HOTINDEX_MACOS_DBFORMAT_ALWAYS_INLINE"
port_bswap_marker="DECIBEL_HOTINDEX_MACOS_PORT_BSWAP_FALLBACK"
mock_env_min_marker="DECIBEL_HOTINDEX_MACOS_MOCK_ENV_MIN_CAST"
fast_getcpu_marker="DECIBEL_HOTINDEX_MACOS_FAST_GETCPU_FALLBACK"
preproc_inline_marker="DECIBEL_HOTINDEX_MACOS_PREPROC_ALWAYS_INLINE"
vm_util_header_marker="DECIBEL_HOTINDEX_MACOS_VM_UTIL_HEADER_CONSTS"
topling_rocks_weak_marker="DECIBEL_HOTINDEX_MACOS_TOPLING_ROCKS_WEAK_IMPORT"
optional_weak_import_marker="DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_IMPORT"
optional_weak_stub_marker="DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_STUB"
filestream_varint_marker="DECIBEL_HOTINDEX_MACOS_FILESTREAM_VARINT_FALLBACK"

shopt -s nullglob
rocksdb_roots=(
  "$cargo_home"/git/checkouts/rust-toplingdb-*/*/librocksdb-sys/rocksdb
)
headers=(
  "$cargo_home"/git/checkouts/rust-toplingdb-*/*/librocksdb-sys/rocksdb/sideplugin/topling-zip/src/terark/bitmanip.hpp
)
shopt -u nullglob

if ((${#rocksdb_roots[@]} == 0)); then
  echo "warning: rust-toplingdb checkout not found; skipping macOS compatibility patch" >&2
  exit 0
fi

for rocksdb_root in "${rocksdb_roots[@]}"; do
  # Sideplugin builds invoke GNU time with Linux-only flags. Prefer the local
  # compatibility wrapper supplied by scripts/toplingdb-cargo.sh.
  while IFS= read -r makefile; do
    perl -0pi -e 's|TIME_CMD\s*[:?]?=\s*/usr/bin/time|TIME_CMD ?= gtime|g' "$makefile"
  done < <(find "$rocksdb_root" -type f \( -name 'Makefile*' -o -name '*.mk' \))

  rocksdb_makefile="$rocksdb_root/Makefile"
  if [[ -f "$rocksdb_makefile" ]] && ! grep -q "$topling_obj_list_marker" "$rocksdb_makefile"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_TOPLING_OBJ_LIST_PREREQ";
my $needle = "\${TOPLING_CORE_DIR}/\${TOPLING_LIB_OBJ_LIST_FILE}: \$(addprefix \${TOPLING_CORE_DIR}/, \${TOPLING_LIB_SRC_LIST_VAR})\n";
my $replacement = "# $marker: TOPLING_LIB_SRC_LIST_VAR is a multiline define; do not use it as a single prerequisite path on Darwin.\n\${TOPLING_CORE_DIR}/\${TOPLING_LIB_OBJ_LIST_FILE}:\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$rocksdb_makefile"
  fi
  if [[ -f "$rocksdb_makefile" ]] && ! grep -q "$topling_obj_list_space_marker" "$rocksdb_makefile"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_TOPLING_OBJ_LIST_STRIP";
my $needle = "TOPLING_LIB_OBJECTS = \$(addprefix \${TOPLING_CORE_DIR}/, \${TOPLING_LIB_OBJ_LIST_VAR})\n";
my $replacement = "TOPLING_LIB_OBJECTS = \$(addprefix \${TOPLING_CORE_DIR}/, \$(strip \${TOPLING_LIB_OBJ_LIST_VAR})) # $marker\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$rocksdb_makefile"
  fi

  if [[ -f "$rocksdb_makefile" ]] && ! grep -q "$topling_darwin_shared_marker" "$rocksdb_makefile"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_TOPLING_SHARED_LINK";
my $needle = "\$(SHARED4): \$(LIB_OBJECTS) \${TOPLING_CORE_DIR}/\${TOPLING_LIB_OBJ_LIST_FILE}\n\t\$(AM_V_CCLD) \$(CXX) \$(PLATFORM_SHARED_LDFLAGS)\$(SHARED3) \$(LIB_OBJECTS) \$(TOPLING_LIB_OBJECTS) \$(EXTRA_SHARED_LIB_LIB) -Wl,-rpath,'\''\$\$ORIGIN'\'' \$(LDFLAGS) -o \$@\n";
my $replacement = "ifeq (\${PLATFORM},OS_MACOSX)\nTOPLING_DARWIN_SHARED_LIBS := -L\${TOPLING_CORE_DIR}/\${BUILD_ROOT}/lib_shared -lterark-zbs-\${COMPILER}-\${BUILD_TYPE_SIG} -lterark-fsa-\${COMPILER}-\${BUILD_TYPE_SIG} -lterark-core-\${COMPILER}-\${BUILD_TYPE_SIG} -lz -lbz2\n\$(SHARED4): \$(LIB_OBJECTS) \${TOPLING_CORE_DIR}/\${TOPLING_ZBS_TARGET} \${TOPLING_CORE_DIR}/\${TOPLING_LIB_OBJ_LIST_FILE}\n\t\$(AM_V_CCLD) \$(CXX) \$(PLATFORM_SHARED_LDFLAGS)\$(SHARED3) \$(LIB_OBJECTS) \$(TOPLING_LIB_OBJECTS) \$(EXTRA_SHARED_LIB_LIB) -Wl,-rpath,\@loader_path -Wl,-rpath,\${TOPLING_CORE_DIR}/\${BUILD_ROOT}/lib_shared \$(TOPLING_DARWIN_SHARED_LIBS) \$(LDFLAGS) -o \$@ # $marker\nelse\n\$(SHARED4): \$(LIB_OBJECTS) \${TOPLING_CORE_DIR}/\${TOPLING_LIB_OBJ_LIST_FILE}\n\t\$(AM_V_CCLD) \$(CXX) \$(PLATFORM_SHARED_LDFLAGS)\$(SHARED3) \$(LIB_OBJECTS) \$(TOPLING_LIB_OBJECTS) \$(EXTRA_SHARED_LIB_LIB) -Wl,-rpath,'\''\$\$ORIGIN'\'' \$(LDFLAGS) -o \$@\nendif\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$rocksdb_makefile"
  fi
  if [[ -f "$rocksdb_makefile" ]] && grep -q "$topling_darwin_shared_marker" "$rocksdb_makefile" && grep -q -- "-Wl,-rpath, -Wl,-rpath" "$rocksdb_makefile"; then
    perl -0pi -e 's|-Wl,-rpath, -Wl,-rpath|-Wl,-rpath,\@loader_path -Wl,-rpath|g' "$rocksdb_makefile"
  fi
  if [[ -f "$rocksdb_makefile" ]] && grep -q "$topling_darwin_shared_marker" "$rocksdb_makefile" && ! grep -q "\$(TOPLING_LIB_OBJECTS).*DECIBEL_HOTINDEX_MACOS_TOPLING_SHARED_LINK" "$rocksdb_makefile"; then
    perl -0pi -e '
my $count = 0;
$count += s|\$\(SHARED4\): \$\(LIB_OBJECTS\) \$\{TOPLING_CORE_DIR\}/\$\{TOPLING_ZBS_TARGET\}\n|\$(SHARED4): \$(LIB_OBJECTS) \${TOPLING_CORE_DIR}/\${TOPLING_ZBS_TARGET} \${TOPLING_CORE_DIR}/\${TOPLING_LIB_OBJ_LIST_FILE}\n|g;
$count += s|\$\(SHARED3\) \$\(LIB_OBJECTS\) \$\(EXTRA_SHARED_LIB_LIB\) -Wl,-rpath,|\$(SHARED3) \$(LIB_OBJECTS) \$(TOPLING_LIB_OBJECTS) \$(EXTRA_SHARED_LIB_LIB) -Wl,-rpath,|g;
die "failed to migrate $ARGV\n" if $count != 2;
' "$rocksdb_makefile"
  fi

  if [[ -f "$rocksdb_makefile" ]] && ! grep -q "$topling_core_lto_marker" "$rocksdb_makefile"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_TOPLING_CORE_NO_LTO";
my $needle = "\t+make -C \${TOPLING_CORE_DIR} \${TOPLING_ZBS_TARGET}\n";
my $replacement = "\t+make -C \${TOPLING_CORE_DIR} \${TOPLING_ZBS_TARGET} \$(if \$(filter OS_MACOSX,\${PLATFORM}),USE_LTO=0) # $marker\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$rocksdb_makefile"
  fi

  if [[ -f "$rocksdb_makefile" ]] && ! grep -q "$atomic_marker" "$rocksdb_makefile"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_NO_LIBATOMIC";
my $needle = "  LDFLAGS += -latomic\n";
my $replacement = "  ifneq (\${PLATFORM},OS_MACOSX)\n    LDFLAGS += -latomic\n  endif # $marker\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$rocksdb_makefile"
  fi

  queue_header="$rocksdb_root/sideplugin/topling-zip/src/terark/util/auto_grow_circular_queue_matrix.hpp"
  if [[ -f "$queue_header" ]] && ! grep -q "$queue_marker" "$queue_header"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_QUEUE_BACK_CONST_CAST";
my $needle = "    const T& back() const { return const_cast<MyType&>(this)->back(); }\n";
my $replacement = "    // $marker: this is a pointer in const member functions.\n    const T& back() const { return const_cast<MyType*>(this)->back(); }\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$queue_header"
  fi

  cspptrie="$rocksdb_root/sideplugin/topling-zip/src/terark/fsa/cspptrie.cpp"
  if [[ -f "$cspptrie" ]] && ! grep -q "$cpuid_marker" "$cspptrie"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_ARM64_SKIP_CPUID";
my $needle = "#elif BOOST_OS_MACOS\n    #include <cpuid.h>\n#else\n";
my $replacement = "#elif BOOST_OS_MACOS\n  #if defined(__x86_64__) || defined(__i386__)\n    #include <cpuid.h>\n  #endif // $marker\n#else\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$cspptrie"
  fi

  if [[ -f "$cspptrie" ]] && ! grep -q "$hugepage_marker" "$cspptrie"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_MADV_HUGEPAGE_GUARD";
my $needle = "        if (HugePageEnum::kTransparent == use_hugepage) {\n            if (madvise(mem, maxMem, MADV_HUGEPAGE) != 0) {\n                WARN(\"madvise(MADV_HUGEPAGE, size=%zd[0x%zX]) = %s\",\n                     maxMem, maxMem, strerror(errno));\n            }\n        }\n";
my $replacement = "        if (HugePageEnum::kTransparent == use_hugepage) {\n#if defined(MADV_HUGEPAGE)\n            if (madvise(mem, maxMem, MADV_HUGEPAGE) != 0) {\n                WARN(\"madvise(MADV_HUGEPAGE, size=%zd[0x%zX]) = %s\",\n                     maxMem, maxMem, strerror(errno));\n            }\n#else\n            // $marker: Darwin has no transparent hugepage madvise flag.\n#endif\n        }\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$cspptrie"
  fi

  zpath_cpp="$rocksdb_root/sideplugin/topling-zip/src/terark/fsa/nest_louds_trie.cpp"
  if [[ -f "$zpath_cpp" ]] && ! grep -q "$zpath_marker" "$zpath_cpp"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_ZPATH_LINK_SIGNATURE";
my $needle = "matchZpath_link(size_t linkVal, const byte_t* str, size_t slen) const noexcept {\n";
my $replacement = "matchZpath_link(uint64_t linkVal, const byte_t* str, size_t slen) const noexcept { // $marker\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$zpath_cpp"
  fi

  lru_map_cpp="$rocksdb_root/sideplugin/topling-zip/src/terark/lru_map.cpp"
  if [[ -f "$lru_map_cpp" ]] && ! grep -q "$futex_marker" "$lru_map_cpp"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_FUTEX_FALLBACK";
my $needle = "#include \"lru_map.hpp\"\n#include <terark/util/atomic.hpp>\n#include <terark/thread/futex.hpp>\n";
my $replacement = "#include \"lru_map.hpp\"\n#include <terark/util/atomic.hpp>\n#if defined(__linux__)\n#include <terark/thread/futex.hpp>\n#else\n#include <climits>\n#include <ctime>\n#include <thread>\n#ifndef FUTEX_WAIT_PRIVATE\n#define FUTEX_WAIT_PRIVATE 0\n#endif\n#ifndef FUTEX_WAKE_PRIVATE\n#define FUTEX_WAKE_PRIVATE 1\n#endif\n// $marker: Darwin has no linux/futex.h; use a cooperative spin fallback.\ninline long futex(void*, uint32_t op, uint32_t, const timespec* = nullptr, void* = nullptr, uint32_t = 0) {\n  if (op == FUTEX_WAIT_PRIVATE) std::this_thread::yield();\n  return 0;\n}\ninline long futex(void*, uint32_t op, uint32_t, uint32_t, void* = nullptr, uint32_t = 0) {\n  if (op == FUTEX_WAIT_PRIVATE) std::this_thread::yield();\n  return 0;\n}\n#endif\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$lru_map_cpp"
  fi

  localtime_cpp="$rocksdb_root/sideplugin/topling-zip/src/terark/util/nolocks_localtime.cpp"
  if [[ -f "$localtime_cpp" ]] && ! grep -q "$localtime_marker" "$localtime_cpp"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_LOCALTIME_CASTS";
my $count = 0;
$count += s/auto tail = terark::fast_popcount_trail\(leap_bits\[year \/ 64\], year % 64\);/auto tail = terark::fast_popcount_trail((unsigned long long)leap_bits[year \/ 64], (unsigned long long)(year % 64)); \/\/ $marker/g;
$count += s/p_tm->tm_zone = g_tzname;/p_tm->tm_zone = const_cast<char*>(g_tzname);/g;
die "failed to patch $ARGV\n" if $count != 2;
' "$localtime_cpp"
  fi

  process_cpp="$rocksdb_root/sideplugin/topling-zip/src/terark/util/process.cpp"
  if [[ -f "$process_cpp" ]] && ! grep -q "$process_marker" "$process_cpp"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_PROCESS_FALLBACKS";
my $needle = "#else\n    #include <sys/types.h>\n    #include <sys/wait.h>\n    #include <unistd.h>\n    #include <spawn.h>\n#endif\n";
my $replacement = "#else\n    #include <sys/types.h>\n    #include <sys/wait.h>\n    #include <unistd.h>\n    #include <spawn.h>\n    #if defined(__APPLE__)\n    extern char** environ; // $marker\n    #endif\n#endif\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
my $read_needle = "bool process_mem_read(pid_t pid, void* data, size_t len, size_t r_addr) {\n  iovec local, remote;\n  local.iov_base = data;\n  local.iov_len = len;\n  remote.iov_base = (void*)r_addr;\n  remote.iov_len = len;\n  ssize_t n_read = process_vm_readv(pid, &local, 1, &remote, 1, 0);\n  \/*\n  if (size_t(n_read) != len) {\n    TERARK_DIE(\"process_read(%d, %p, %zd, %zd) = (n_read=%zd) : %m\", pid, data, len, r_addr, n_read);\n  }\n  *\/\n  return size_t(n_read) == len;\n}\n";
my $read_replacement = "bool process_mem_read(pid_t pid, void* data, size_t len, size_t r_addr) {\n#if defined(__linux__)\n  iovec local, remote;\n  local.iov_base = data;\n  local.iov_len = len;\n  remote.iov_base = (void*)r_addr;\n  remote.iov_len = len;\n  ssize_t n_read = process_vm_readv(pid, &local, 1, &remote, 1, 0);\n  return size_t(n_read) == len;\n#else\n  (void)pid; (void)data; (void)len; (void)r_addr;\n  return false;\n#endif\n}\n";
$count += ($_ =~ s/\Q$read_needle\E/$read_replacement/g);
my $write_needle = "bool process_mem_write(pid_t pid, const void* data, size_t len, size_t r_addr) {\n  iovec local, remote;\n  local.iov_base = (void*)data;\n  local.iov_len = len;\n  remote.iov_base = (void*)r_addr;\n  remote.iov_len = len;\n  ssize_t n_write = process_vm_writev(pid, &local, 1, &remote, 1, 0);\n  \/*\n  if (size_t(n_write) != len) {\n    TERARK_DIE(\"process_write(%d, %p, %zd, %zd) = (n_write=%zd) : %m\", pid, data, len, r_addr, n_write);\n  }\n  *\/\n  return size_t(n_write) == len;\n}\n";
my $write_replacement = "bool process_mem_write(pid_t pid, const void* data, size_t len, size_t r_addr) {\n#if defined(__linux__)\n  iovec local, remote;\n  local.iov_base = (void*)data;\n  local.iov_len = len;\n  remote.iov_base = (void*)r_addr;\n  remote.iov_len = len;\n  ssize_t n_write = process_vm_writev(pid, &local, 1, &remote, 1, 0);\n  return size_t(n_write) == len;\n#else\n  (void)pid; (void)data; (void)len; (void)r_addr;\n  return false;\n#endif\n}\n";
$count += ($_ =~ s/\Q$write_needle\E/$write_replacement/g);
die "failed to patch $ARGV\n" if $count != 3;
' "$process_cpp"
  fi

  thread_local_cpp="$rocksdb_root/sideplugin/topling-zip/src/terark/util/thread_local.cpp"
  if [[ -f "$thread_local_cpp" ]] && ! grep -q "$thread_local_marker" "$thread_local_cpp"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_ALWAYS_INLINE_FALLBACK";
my $needle = "#endif  // BOOST_OS_WINDOWS\n\n#if !defined(__attribute_noinline__)\n";
my $replacement = "#endif  // BOOST_OS_WINDOWS\n\n#if !defined(__always_inline)\n#define __always_inline inline __attribute__((always_inline)) // $marker\n#endif\n\n#if !defined(__attribute_noinline__)\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$thread_local_cpp"
  fi

  fiber_aio_cpp="$rocksdb_root/sideplugin/topling-zip/src/terark/thread/fiber_aio.cpp"
  if [[ -f "$fiber_aio_cpp" ]] && ! grep -q "$fiber_aio_marker" "$fiber_aio_cpp"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_FIBER_AIO_POSIX";
my $count = 0;
my $provider_needle = "  return prov;\n}();\n\nstatic std::atomic<size_t> g_ft_num;\n";
my $provider_replacement = "  return prov;\n}();\n#else\nconst static IoProvider g_io_provider = IoProvider::posix; // $marker\n#endif\n\nstatic std::atomic<size_t> g_ft_num;\n";
$count += ($_ =~ s/\Q$provider_needle\E/$provider_replacement/g);
my $aio_needle = "public:\n  inline void yield() { m_fy.unchecked_yield(); }\n};\n\nclass io_fiber_aio : public io_fiber_base {\n";
my $aio_replacement = "public:\n  inline void yield() { m_fy.unchecked_yield(); }\n};\n\n#if BOOST_OS_LINUX\nclass io_fiber_aio : public io_fiber_base {\n";
$count += ($_ =~ s/\Q$aio_needle\E/$aio_replacement/g);
die "failed to patch $ARGV\n" if $count != 2;
' "$fiber_aio_cpp"
  fi

  if [[ -f "$fiber_aio_cpp" ]] && ! grep -q "$aioinit_marker" "$fiber_aio_cpp"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_AIOINIT_GUARD";
my $needle = "    int threads = (int)getEnvLong(\"TOPLING_IO_POSIX_THREADS\", 0);\n    if (threads > 0) {\n      struct aioinit init = {};\n      init.aio_threads = threads;\n      init.aio_num = threads * 4;\n      aio_init(&init); // return is void\n    } else {\n      // do not call aio_init, use posix aio default conf\n      threads = 20; // glib aio default\n    }\n";
my $replacement = "    int threads = (int)getEnvLong(\"TOPLING_IO_POSIX_THREADS\", 0);\n#if defined(__linux__)\n    if (threads > 0) {\n      struct aioinit init = {};\n      init.aio_threads = threads;\n      init.aio_num = threads * 4;\n      aio_init(&init); // return is void\n    } else {\n      // do not call aio_init, use posix aio default conf\n      threads = 20; // glib aio default\n    }\n#else\n    // $marker: Darwin has POSIX AIO but no glibc aio_init/aioinit.\n    if (threads <= 0) threads = 20;\n#endif\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$fiber_aio_cpp"
  fi

  vm_util_cpp="$rocksdb_root/sideplugin/topling-zip/src/terark/util/vm_util.cpp"
  if [[ -f "$vm_util_cpp" ]] && ! grep -q "$vm_util_marker" "$vm_util_cpp"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_MADV_POPULATE_GUARD";
my $needle = "  #elif !defined(__CYGWIN__)\n    // check g_has_madv_populate first, only MADV_POPULATE_READ\n    if (g_has_madv_populate) {\n        madvise((void*)lo, aligned_len, MADV_POPULATE_READ);\n    }\n  #endif\n";
my $replacement = "  #elif !defined(__CYGWIN__) && defined(MADV_POPULATE_READ)\n    // check g_has_madv_populate first, only MADV_POPULATE_READ\n    if (g_has_madv_populate) {\n        madvise((void*)lo, aligned_len, MADV_POPULATE_READ);\n    }\n  #else\n    (void)lo; (void)aligned_len; // $marker\n  #endif\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$vm_util_cpp"
  fi

  vm_util_h="$rocksdb_root/sideplugin/topling-zip/src/terark/util/vm_util.hpp"
  if [[ -f "$vm_util_h" ]] && ! grep -q "$vm_util_header_marker" "$vm_util_h"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_VM_UTIL_HEADER_CONSTS";
my $needle = "#if defined(_MSC_VER)\nconstexpr bool g_has_madv_populate = true;\nconstexpr size_t g_min_prefault_pages = 1;\n#else\nTERARK_DLL_EXPORT extern const int g_linux_kernel_version;\nTERARK_DLL_EXPORT extern const bool g_has_madv_populate;\nTERARK_DLL_EXPORT extern const size_t g_min_prefault_pages;\n#endif\n";
my $replacement = "#if defined(_MSC_VER)\nconstexpr bool g_has_madv_populate = true;\nconstexpr size_t g_min_prefault_pages = 1;\n#else\nTERARK_DLL_EXPORT extern const int g_linux_kernel_version;\nTERARK_DLL_EXPORT extern const bool g_has_madv_populate;\nTERARK_DLL_EXPORT extern const size_t g_min_prefault_pages; // $marker\n#endif\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$vm_util_h"
  fi
  if [[ -f "$vm_util_h" ]] && grep -q "$vm_util_header_marker" "$vm_util_h" && grep -q "constexpr int g_linux_kernel_version = -1" "$vm_util_h"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_VM_UTIL_HEADER_CONSTS";
my $needle = "#if defined(_MSC_VER)\nconstexpr bool g_has_madv_populate = true;\nconstexpr size_t g_min_prefault_pages = 1;\n#elif !defined(__linux__)\nconstexpr int g_linux_kernel_version = -1; // $marker\nconstexpr bool g_has_madv_populate = false;\nconstexpr size_t g_min_prefault_pages = 2;\n#else\nTERARK_DLL_EXPORT extern const int g_linux_kernel_version;\nTERARK_DLL_EXPORT extern const bool g_has_madv_populate;\nTERARK_DLL_EXPORT extern const size_t g_min_prefault_pages;\n#endif\n";
my $replacement = "#if defined(_MSC_VER)\nconstexpr bool g_has_madv_populate = true;\nconstexpr size_t g_min_prefault_pages = 1;\n#else\nTERARK_DLL_EXPORT extern const int g_linux_kernel_version;\nTERARK_DLL_EXPORT extern const bool g_has_madv_populate;\nTERARK_DLL_EXPORT extern const size_t g_min_prefault_pages; // $marker\n#endif\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to migrate $ARGV\n" if $count != 1;
' "$vm_util_h"
  fi

  if [[ -f "$vm_util_cpp" ]] && ! grep -q "$vm_util_defs_marker" "$vm_util_cpp"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_VM_UTIL_DARWIN_DEFS";
my $needle = "#endif\n#endif\n\nTERARK_DLL_EXPORT\nvoid vm_prefetch";
my $replacement = "#endif\n#elif !defined(_MSC_VER)\nTERARK_DLL_EXPORT\nconst int g_linux_kernel_version = -1;\nTERARK_DLL_EXPORT\nconst bool g_has_madv_populate = false;\nTERARK_DLL_EXPORT\nconst size_t g_min_prefault_pages = 2; // $marker\n#endif\n\nTERARK_DLL_EXPORT\nvoid vm_prefetch";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$vm_util_cpp"
  fi

  dbformat_h="$rocksdb_root/db/dbformat.h"
  if [[ -f "$dbformat_h" ]] && ! grep -q "$dbformat_inline_marker" "$dbformat_h"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_DBFORMAT_ALWAYS_INLINE";
my $needle = "#pragma once\n#include <stdio.h>\n";
my $replacement = "#pragma once\n#include <stdio.h>\n\n#if !defined(__always_inline)\n#define __always_inline inline __attribute__((always_inline)) // $marker\n#endif\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$dbformat_h"
  fi

  port_posix_h="$rocksdb_root/port/port_posix.h"
  if [[ -f "$port_posix_h" ]] && ! grep -q "$port_bswap_marker" "$port_posix_h"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_PORT_BSWAP_FALLBACK";
my $needle = "#if defined(OS_MACOSX)\n#include <machine/endian.h>\n";
my $replacement = "#if defined(OS_MACOSX)\n#include <machine/endian.h>\n#ifndef __bswap_16\n#define __bswap_16(x) __builtin_bswap16(x)\n#endif\n#ifndef __bswap_32\n#define __bswap_32(x) __builtin_bswap32(x)\n#endif\n#ifndef __bswap_64\n#define __bswap_64(x) __builtin_bswap64(x)\n#endif\n// $marker\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$port_posix_h"
  fi

  mock_env_cc="$rocksdb_root/env/mock_env.cc"
  if [[ -f "$mock_env_cc" ]] && ! grep -q "$mock_env_min_marker" "$mock_env_cc"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_MOCK_ENV_MIN_CAST";
my $needle = "    const uint64_t available = data_.size() - std::min(data_.size(), offset);\n";
my $replacement = "    const uint64_t available = data_.size() - std::min(data_.size(), static_cast<size_t>(offset)); // $marker\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$mock_env_cc"
  fi

  fast_getcpu_h="$rocksdb_root/sideplugin/topling-zip/src/terark/util/fast_getcpu.hpp"
  if [[ -f "$fast_getcpu_h" ]] && ! grep -q "$fast_getcpu_marker" "$fast_getcpu_h"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_FAST_GETCPU_FALLBACK";
my $needle = "#elif !defined(_MSC_VER)\n\n#include <sched.h>\nnamespace terark {\nterark_forceinline unsigned int fast_getcpu(void) {\n    return sched_getcpu();\n}\n} // namespace terark\n\n#endif\n";
my $replacement = "#elif defined(__linux__) && !defined(_MSC_VER)\n\n#include <sched.h>\nnamespace terark {\nterark_forceinline unsigned int fast_getcpu(void) {\n    return sched_getcpu();\n}\n} // namespace terark\n\n#elif !defined(_MSC_VER)\nnamespace terark {\nterark_forceinline unsigned int fast_getcpu(void) {\n    return 0; // $marker: Darwin has no sched_getcpu.\n}\n} // namespace terark\n\n#endif\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$fast_getcpu_h"
  fi

  preproc_h="$rocksdb_root/include/rocksdb/preproc.h"
  if [[ -f "$preproc_h" ]] && ! grep -q "$preproc_inline_marker" "$preproc_h"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_PREPROC_ALWAYS_INLINE";
my $needle = "#if defined(_MSC_VER) && !defined(__always_inline)\n  #define __always_inline __forceinline\n#endif\n";
my $replacement = "#if !defined(__always_inline)\n  #if defined(_MSC_VER)\n    #define __always_inline __forceinline\n  #else\n    #define __always_inline inline __attribute__((always_inline)) // $marker\n  #endif\n#endif\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$preproc_h"
  fi

  top_zip_table_cc="$rocksdb_root/sideplugin/topling-zip_table_reader/src/table/top_zip_table.cc"
  if [[ -f "$top_zip_table_cc" ]] && ! grep -q "$topling_rocks_weak_marker" "$top_zip_table_cc" && ! grep -q "$optional_weak_stub_marker" "$top_zip_table_cc"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_TOPLING_ROCKS_WEAK_IMPORT";
my $needle = "__attribute__((weak))\nconst char* git_version_hash_info_topling_rocks();\n";
my $replacement = "#if defined(__APPLE__)\n__attribute__((weak_import)) // $marker\n#else\n__attribute__((weak))\n#endif\nconst char* git_version_hash_info_topling_rocks();\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$top_zip_table_cc"
  fi

  if [[ -f "$top_zip_table_cc" ]] && ! grep -q "$optional_weak_import_marker" "$top_zip_table_cc" && ! grep -q "$optional_weak_stub_marker" "$top_zip_table_cc"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_IMPORT";
my $count = 0;
my $git_needle = "#if defined(__APPLE__)\n__attribute__((weak_import)) // DECIBEL_HOTINDEX_MACOS_TOPLING_ROCKS_WEAK_IMPORT\n#else\n__attribute__((weak))\n#endif\nconst char* git_version_hash_info_topling_rocks();\n";
my $git_replacement = "#if defined(__APPLE__)\nextern const char* git_version_hash_info_topling_rocks() __attribute__((weak_import)); // $marker\n#else\n__attribute__((weak))\nconst char* git_version_hash_info_topling_rocks();\n#endif\n";
$count += ($_ =~ s/\Q$git_needle\E/$git_replacement/g);
my $builder_needle = "__attribute__((weak))\nextern\nTableBuilder*\ncreateToplingZipTableBuilder(const ToplingZipTableFactory*,\n                             const TableBuilderOptions&,\n                             WritableFileWriter*);\n";
my $builder_replacement = "#if defined(__APPLE__)\nextern\nTableBuilder*\ncreateToplingZipTableBuilder(const ToplingZipTableFactory*,\n                             const TableBuilderOptions&,\n                             WritableFileWriter*) __attribute__((weak_import)); // $marker\n#else\n__attribute__((weak))\nextern\nTableBuilder*\ncreateToplingZipTableBuilder(const ToplingZipTableFactory*,\n                             const TableBuilderOptions&,\n                             WritableFileWriter*);\n#endif\n";
$count += ($_ =~ s/\Q$builder_needle\E/$builder_replacement/g);
die "failed to patch $ARGV\n" if $count != 2;
' "$top_zip_table_cc"
  fi

  if [[ -f "$top_zip_table_cc" ]] && ! grep -q "$optional_weak_stub_marker" "$top_zip_table_cc"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_STUB";
my $count = 0;
my $git_needle = qr/#if defined\(__APPLE__\)\nextern const char\* git_version_hash_info_topling_rocks\(\) __attribute__\(\(weak_import\)\); \/\/ DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_IMPORT\n#else\n(?:#if defined\(__APPLE__\)\n__attribute__\(\(weak_import\)\) \/\/ DECIBEL_HOTINDEX_MACOS_TOPLING_ROCKS_WEAK_IMPORT\n#else\n)?__attribute__\(\(weak\)\)\n(?:#endif\n)?const char\* git_version_hash_info_topling_rocks\(\);\n#endif\n/;
my $git_replacement = "#if defined(__APPLE__)\n__attribute__((weak))\nconst char* git_version_hash_info_topling_rocks() { return nullptr; } // $marker\n#else\n__attribute__((weak))\nconst char* git_version_hash_info_topling_rocks();\n#endif\n";
$count += ($_ =~ s/$git_needle/$git_replacement/g);
my $print_needle = "    if (git_version_hash_info_topling_rocks)\n      INFO(info_log, \"topling-rocks %s\", git_version_hash_info_topling_rocks());\n";
my $print_replacement = "#if defined(__APPLE__)\n    if (const char* topling_rocks_git = git_version_hash_info_topling_rocks())\n      INFO(info_log, \"topling-rocks %s\", topling_rocks_git);\n#else\n    if (git_version_hash_info_topling_rocks)\n      INFO(info_log, \"topling-rocks %s\", git_version_hash_info_topling_rocks());\n#endif\n";
$count += ($_ =~ s/\Q$print_needle\E/$print_replacement/g);
my $warn_needle = "  if (git_version_hash_info_topling_rocks == nullptr) {\n";
my $warn_replacement = "#if defined(__APPLE__)\n  if (git_version_hash_info_topling_rocks() == nullptr) {\n#else\n  if (git_version_hash_info_topling_rocks == nullptr) {\n#endif\n";
$count += ($_ =~ s/\Q$warn_needle\E/$warn_replacement/g);
my $builder_needle = "#if defined(__APPLE__)\nextern\nTableBuilder*\ncreateToplingZipTableBuilder(const ToplingZipTableFactory*,\n                             const TableBuilderOptions&,\n                             WritableFileWriter*) __attribute__((weak_import)); // DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_IMPORT\n#else\n__attribute__((weak))\nextern\nTableBuilder*\ncreateToplingZipTableBuilder(const ToplingZipTableFactory*,\n                             const TableBuilderOptions&,\n                             WritableFileWriter*);\n#endif\n";
my $builder_replacement = "#if defined(__APPLE__)\n__attribute__((weak))\nTableBuilder*\ncreateToplingZipTableBuilder(const ToplingZipTableFactory*,\n                             const TableBuilderOptions&,\n                             WritableFileWriter*) { return nullptr; } // $marker\n#else\n__attribute__((weak))\nextern\nTableBuilder*\ncreateToplingZipTableBuilder(const ToplingZipTableFactory*,\n                             const TableBuilderOptions&,\n                             WritableFileWriter*);\n#endif\n";
$count += ($_ =~ s/\Q$builder_needle\E/$builder_replacement/g);
die "failed to patch $ARGV\n" if $count != 4;
' "$top_zip_table_cc"
  fi

  top_zip_table_json_plugin="$rocksdb_root/sideplugin/topling-zip_table_reader/src/table/top_zip_table_json_plugin.cc"
  if [[ -f "$top_zip_table_json_plugin" ]] && ! grep -q "$optional_weak_import_marker" "$top_zip_table_json_plugin" && ! grep -q "$optional_weak_stub_marker" "$top_zip_table_json_plugin"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_IMPORT";
my $count = 0;
my $rocks_needle = "#ifdef HAS_TOPLING_ROCKS\n__attribute__((weak))\nconst char* git_version_hash_info_topling_rocks();\n__attribute__((weak)) long toplingdb_expire_time();\n#endif\n";
my $rocks_replacement = "#ifdef HAS_TOPLING_ROCKS\n#if defined(__APPLE__)\nconst char* git_version_hash_info_topling_rocks() __attribute__((weak_import)); // $marker\nlong toplingdb_expire_time() __attribute__((weak_import));\n#else\n__attribute__((weak))\nconst char* git_version_hash_info_topling_rocks();\n__attribute__((weak)) long toplingdb_expire_time();\n#endif\n#endif\n";
$count += ($_ =~ s/\Q$rocks_needle\E/$rocks_replacement/g);
my $pid_needle = "#ifndef _MSC_VER\n__attribute__((weak))\nextern pid_t GetZipServerPID();\n#endif\n";
my $pid_replacement = "#ifndef _MSC_VER\n#if defined(__APPLE__)\nextern pid_t GetZipServerPID() __attribute__((weak_import)); // $marker\n#else\n__attribute__((weak))\nextern pid_t GetZipServerPID();\n#endif\n#endif\n";
$count += ($_ =~ s/\Q$pid_needle\E/$pid_replacement/g);
die "failed to patch $ARGV\n" if $count != 2;
' "$top_zip_table_json_plugin"
  fi

  if [[ -f "$top_zip_table_json_plugin" ]] && ! grep -q "$optional_weak_stub_marker" "$top_zip_table_json_plugin"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_STUB";
my $count = 0;
my $pid_needle = "#ifndef _MSC_VER\n#if defined(__APPLE__)\nextern pid_t GetZipServerPID() __attribute__((weak_import)); // DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_IMPORT\n#else\n__attribute__((weak))\nextern pid_t GetZipServerPID();\n#endif\n#endif\n";
my $pid_replacement = "#ifndef _MSC_VER\n#if defined(__APPLE__)\n__attribute__((weak))\npid_t GetZipServerPID() { return -1; } // $marker\n#else\n__attribute__((weak))\nextern pid_t GetZipServerPID();\n#endif\n#endif\n";
$count += ($_ =~ s/\Q$pid_needle\E/$pid_replacement/g);
my $guard_needle = "#ifndef _MSC_VER\n  if (&GetZipServerPID == nullptr)\n#endif\n";
my $guard_replacement = "#if !defined(_MSC_VER) && defined(HAS_TOPLING_ROCKS)\n  if (&GetZipServerPID == nullptr)\n#endif\n";
$count += ($_ =~ s/\Q$guard_needle\E/$guard_replacement/g);
die "failed to patch $ARGV\n" if $count != 2;
' "$top_zip_table_json_plugin"
  fi

  filestream_cpp="$rocksdb_root/sideplugin/topling-zip/src/terark/io/FileStream.cpp"
  if [[ -f "$filestream_cpp" ]] && ! grep -q "$filestream_varint_marker" "$filestream_cpp"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_FILESTREAM_VARINT_FALLBACK";
my $needle = "#elif defined(__ANDROID__)\n";
my $replacement = "#elif defined(__ANDROID__) || defined(__APPLE__) // $marker\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$filestream_cpp"
  fi

  build_rs="$(dirname "$rocksdb_root")/build.rs"
  if [[ -f "$build_rs" ]] && ! grep -q "$buildrs_marker" "$build_rs"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_BUILD_RS_DYLIB_LTO";
my $count = 0;
my $lto_needle = "        let update_repo = env::var(\"UPDATE_REPO\").unwrap_or_else(|_| \"0\".into());\n        println!(\"cargo:rerun-if-env-changed=UPDATE_REPO\");\n";
my $lto_replacement = "        let update_repo = env::var(\"UPDATE_REPO\").unwrap_or_else(|_| \"0\".into());\n        println!(\"cargo:rerun-if-env-changed=UPDATE_REPO\");\n        let use_lto = env::var(\"TOPLINGDB_USE_LTO\").unwrap_or_else(|_| {\n            let target = env::var(\"TARGET\").unwrap_or_default();\n            if target.contains(\"apple\") { \"0\".into() } else { \"1\".into() }\n        });\n        println!(\"cargo:rerun-if-env-changed=TOPLINGDB_USE_LTO\"); // $marker\n";
$count += ($_ =~ s/\Q$lto_needle\E/$lto_replacement/g);
$count += s/"USE_LTO=1",/&format!("USE_LTO={use_lto}"),/g;
my $copy_needle = "        if name_str.starts_with(\"librocksdb\") && name_str.contains(\".so\")\n";
my $copy_replacement = "        if name_str.starts_with(\"librocksdb\") && (name_str.contains(\".so\") || name_str.contains(\".dylib\"))\n";
$count += ($_ =~ s/\Q$copy_needle\E/$copy_replacement/g);
die "failed to patch $ARGV\n" if $count != 3;
' "$build_rs"
  fi

  if [[ -f "$build_rs" ]] && ! grep -q "$buildrs_lib_marker" "$build_rs"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_BUILD_RS_REQUIRE_NATIVE_LIB";
my $count = 0;
my $loop_needle = "    for entry in fs::read_dir(&rocksdb_lib_dir).unwrap() {\n";
my $loop_replacement = "    let mut copied_native_lib = false; // $marker\n    for entry in fs::read_dir(&rocksdb_lib_dir).unwrap() {\n";
$count += ($_ =~ s/\Q$loop_needle\E/$loop_replacement/g);
my $copy_needle = "            if entry.file_type().map(|t| t.is_symlink()).unwrap_or(false) {\n                let link_target = fs::read_link(entry.path()).unwrap();\n                std::os::unix::fs::symlink(link_target, &dest).unwrap();\n            } else {\n                fs::copy(entry.path(), &dest).unwrap();\n            }\n        }\n    }\n\n    println!(\n";
my $copy_replacement = "            if entry.file_type().map(|t| t.is_symlink()).unwrap_or(false) {\n                let link_target = fs::read_link(entry.path()).unwrap();\n                std::os::unix::fs::symlink(link_target, &dest).unwrap();\n            } else {\n                fs::copy(entry.path(), &dest).unwrap();\n            }\n            copied_native_lib = true;\n        }\n    }\n    assert!(\n        copied_native_lib,\n        \"native librocksdb was not produced in {}\",\n        rocksdb_lib_dir.display()\n    );\n\n    println!(\n";
$count += ($_ =~ s/\Q$copy_needle\E/$copy_replacement/g);
die "failed to patch $ARGV\n" if $count != 2;
' "$build_rs"
  fi

  builtin_plugin_basic="$rocksdb_root/sideplugin/rockside/src/topling/builtin_plugin_basic.cc"
  if [[ -f "$builtin_plugin_basic" ]] && ! grep -q "$rockside_prctl_marker" "$builtin_plugin_basic"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_ROCKSIDE_PRCTL_GUARD";
my $count = 0;
my $include_needle = "#if defined(_MSC_VER)\n#else\n  #include <sys/prctl.h>\n  #include <sys/wait.h>\n  #include <signal.h>\n  #include <unistd.h>\n#endif\n";
my $include_replacement = "#if defined(_MSC_VER)\n#else\n  #if defined(__linux__)\n    #include <sys/prctl.h>\n  #endif\n  #include <sys/wait.h>\n  #include <signal.h>\n  #include <unistd.h>\n#endif\n";
$count += ($_ =~ s/\Q$include_needle\E/$include_replacement/g);
my $prctl_needle = "      prctl(PR_SET_PDEATHSIG, SIGKILL);\n";
my $prctl_replacement = "#if defined(__linux__)\n      prctl(PR_SET_PDEATHSIG, SIGKILL);\n#else\n      (void)SIGKILL; // $marker: Darwin has no PR_SET_PDEATHSIG equivalent.\n#endif\n";
$count += ($_ =~ s/\Q$prctl_needle\E/$prctl_replacement/g);
die "failed to patch $ARGV\n" if $count != 2;
' "$builtin_plugin_basic"
  fi

  if [[ -f "$builtin_plugin_basic" ]] && ! grep -q "$optional_weak_import_marker" "$builtin_plugin_basic" && ! grep -q "$optional_weak_stub_marker" "$builtin_plugin_basic"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_IMPORT";
my $count = 0;
my $decl_needle = "__attribute__((weak)) void JS_ZipTable_AddVersion(json& djs, bool html);\n__attribute__((weak)) void JS_ToplingDB_FS_AddVersion(json& djs, bool html);\n\nvoid JS_CSPPMemTab_AddVersion(json& djs, bool html);\nvoid JS_CSPP_WBWI_AddVersion(json& djs, bool html);\nvoid JS_ToplingDcompact_AddVersion(json& djs, bool html);\n";
my $decl_replacement = "#if defined(__APPLE__)\nvoid JS_ZipTable_AddVersion(json& djs, bool html) __attribute__((weak_import)); // $marker\nvoid JS_ToplingDB_FS_AddVersion(json& djs, bool html) __attribute__((weak_import));\nvoid JS_ToplingDcompact_AddVersion(json& djs, bool html) __attribute__((weak_import));\n#else\n__attribute__((weak)) void JS_ZipTable_AddVersion(json& djs, bool html);\n__attribute__((weak)) void JS_ToplingDB_FS_AddVersion(json& djs, bool html);\nvoid JS_ToplingDcompact_AddVersion(json& djs, bool html);\n#endif\n\nvoid JS_CSPPMemTab_AddVersion(json& djs, bool html);\nvoid JS_CSPP_WBWI_AddVersion(json& djs, bool html);\n";
$count += ($_ =~ s/\Q$decl_needle\E/$decl_replacement/g);
my $call_needle = "  JS_TopTable_AddVersion(js, html);\n  JS_ToplingDcompact_AddVersion(js, html);\n";
my $call_replacement = "  JS_TopTable_AddVersion(js, html);\n  if (JS_ToplingDcompact_AddVersion)\n    JS_ToplingDcompact_AddVersion(js, html);\n";
$count += ($_ =~ s/\Q$call_needle\E/$call_replacement/g);
die "failed to patch $ARGV\n" if $count != 2;
' "$builtin_plugin_basic"
  fi

  if [[ -f "$builtin_plugin_basic" ]] && ! grep -q "$optional_weak_stub_marker" "$builtin_plugin_basic"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_STUB";
my $decl_needle = "#if defined(__APPLE__)\nvoid JS_ZipTable_AddVersion(json& djs, bool html) __attribute__((weak_import)); // DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_IMPORT\nvoid JS_ToplingDB_FS_AddVersion(json& djs, bool html) __attribute__((weak_import));\nvoid JS_ToplingDcompact_AddVersion(json& djs, bool html) __attribute__((weak_import));\n#else\n__attribute__((weak)) void JS_ZipTable_AddVersion(json& djs, bool html);\n__attribute__((weak)) void JS_ToplingDB_FS_AddVersion(json& djs, bool html);\nvoid JS_ToplingDcompact_AddVersion(json& djs, bool html);\n#endif\n\nvoid JS_CSPPMemTab_AddVersion(json& djs, bool html);\nvoid JS_CSPP_WBWI_AddVersion(json& djs, bool html);\n";
my $decl_replacement = "#if defined(__APPLE__)\nvoid JS_ZipTable_AddVersion(json& djs, bool html) __attribute__((weak_import));\n__attribute__((weak)) void JS_ToplingDB_FS_AddVersion(json&, bool) {} // $marker\n__attribute__((weak)) void JS_ToplingDcompact_AddVersion(json&, bool) {}\n#else\n__attribute__((weak)) void JS_ZipTable_AddVersion(json& djs, bool html);\n__attribute__((weak)) void JS_ToplingDB_FS_AddVersion(json& djs, bool html);\nvoid JS_ToplingDcompact_AddVersion(json& djs, bool html);\n#endif\n\nvoid JS_CSPPMemTab_AddVersion(json& djs, bool html);\nvoid JS_CSPP_WBWI_AddVersion(json& djs, bool html);\n";
my $count = ($_ =~ s/\Q$decl_needle\E/$decl_replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$builtin_plugin_basic"
  fi

  version_set_cc="$rocksdb_root/db/version_set.cc"
  if [[ -f "$version_set_cc" ]] && ! grep -q "$optional_weak_import_marker" "$version_set_cc" && ! grep -q "$optional_weak_stub_marker" "$version_set_cc"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_IMPORT";
my $needle = "__attribute__((weak)) void\nInitUdfa(LevelFilesBrief*, const Comparator* user_cmp);\n__attribute__((weak)) int\nFindFileInRangeUdfa(const LevelFilesBrief&, const Slice& key);\n";
my $replacement = "#if defined(__APPLE__)\nvoid\nInitUdfa(LevelFilesBrief*, const Comparator* user_cmp) __attribute__((weak_import)); // $marker\nint\nFindFileInRangeUdfa(const LevelFilesBrief&, const Slice& key) __attribute__((weak_import));\n#else\n__attribute__((weak)) void\nInitUdfa(LevelFilesBrief*, const Comparator* user_cmp);\n__attribute__((weak)) int\nFindFileInRangeUdfa(const LevelFilesBrief&, const Slice& key);\n#endif\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$version_set_cc"
  fi

  if [[ -f "$version_set_cc" ]] && ! grep -q "$optional_weak_stub_marker" "$version_set_cc"; then
    perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_STUB";
my $needle = "#if defined(__APPLE__)\nvoid\nInitUdfa(LevelFilesBrief*, const Comparator* user_cmp) __attribute__((weak_import)); // DECIBEL_HOTINDEX_MACOS_OPTIONAL_WEAK_IMPORT\nint\nFindFileInRangeUdfa(const LevelFilesBrief&, const Slice& key) __attribute__((weak_import));\n#else\n__attribute__((weak)) void\nInitUdfa(LevelFilesBrief*, const Comparator* user_cmp);\n__attribute__((weak)) int\nFindFileInRangeUdfa(const LevelFilesBrief&, const Slice& key);\n#endif\n";
my $replacement = "#if defined(__APPLE__)\n__attribute__((weak)) void\nInitUdfa(LevelFilesBrief*, const Comparator*) {} // $marker\n__attribute__((weak)) int\nFindFileInRangeUdfa(const LevelFilesBrief&, const Slice&) { return 0; }\n#else\n__attribute__((weak)) void\nInitUdfa(LevelFilesBrief*, const Comparator* user_cmp);\n__attribute__((weak)) int\nFindFileInRangeUdfa(const LevelFilesBrief&, const Slice& key);\n#endif\n";
my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
die "failed to patch $ARGV\n" if $count != 1;
' "$version_set_cc"
  fi
done

if ((${#headers[@]} == 0)); then
  echo "warning: rust-toplingdb bitmanip header not found; skipping fast_popcount_trail patch" >&2
  exit 0
fi

for header in "${headers[@]}"; do
  if grep -q "$marker" "$header"; then
    continue
  fi

  perl -0pi -e '
my $marker = "DECIBEL_HOTINDEX_MACOS_FAST_POPCOUNT_TRAIL_OVERLOAD";
my $needle = "#if ULONG_MAX > 0xFFFFFFFF\ninline long fast_popcount_trail(unsigned long x, unsigned long n) { return fast_popcount_trail((unsigned long long)x, (unsigned long long)n); }\n#else\n";
my $replacement = "#if ULONG_MAX > 0xFFFFFFFF\ninline long fast_popcount_trail(unsigned long x, unsigned long n) { return fast_popcount_trail((unsigned long long)x, (unsigned long long)n); }\n#if defined(__APPLE__)\n// $marker: disambiguate uint64_t,size_t calls on Darwin LP64.\ninline long long fast_popcount_trail(unsigned long long x, unsigned long n) { return fast_popcount_trail(x, (unsigned long long)n); }\n#endif\n#else\n";
if (index($_, $marker) < 0) {
  my $count = ($_ =~ s/\Q$needle\E/$replacement/g);
  die "failed to patch $ARGV\n" if $count != 1;
}
' "$header"
done
