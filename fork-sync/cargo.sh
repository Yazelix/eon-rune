#!/usr/bin/env bash
set -euo pipefail
: "${RUNE_SYNC_TOOLCHAIN:?absolute Rust 1.99.1 toolchain directory}"
: "${CARGO_TARGET_DIR:?dedicated target directory}"
tools=$(cd "$(dirname "$0")" && pwd)
if test "${RUNE_SYNC_BOUNDED:-0}" != 1; then
    exec python3 "$tools/launch.py" --build bash "$tools/cargo.sh" "$@"
fi
: "${RUNE_SYNC_PRIVATE_ROOT:?private run output root}"
test "$CARGO_HOME" = "$RUNE_SYNC_PRIVATE_ROOT/cargo"
test "${RUNE_SYNC_BUILD_HELD:-0}" = 1
python3 "$tools/native/owned.py" guard
for name in config config.toml credentials credentials.toml; do
    test ! -e "$CARGO_HOME/$name"
done
if test -n "${RUNE_SYNC_CARGO_SEED:-}"; then
    python3 "$tools/cache.py" "$RUNE_SYNC_CARGO_SEED" "$CARGO_HOME"
fi
export CARGO_NET_OFFLINE=true
export CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=1
group=$(sed -n 's/^0:://p' /proc/self/cgroup)
test "$(cat "/sys/fs/cgroup$group/memory.max")" = 8589934592
test "$(cat "/sys/fs/cgroup$group/memory.swap.max")" = 0
arguments=("$@")
manifest=Cargo.toml
for ((index=0; index<${#arguments[@]}; index++)); do
    if test "${arguments[$index]}" = --manifest-path; then
        index=$((index + 1))
        arguments[$index]=$(realpath -e "${arguments[$index]}")
        manifest=${arguments[$index]}
    fi
done
directory=$(python3 "$tools/native/owned.py" cargo-dir "$manifest")
cd "$directory"
export RUSTC="$RUNE_SYNC_TOOLCHAIN/bin/rustc"
test "$("$RUSTC" --version | cut -d ' ' -f 2)" = 1.99.1
test "$("$RUNE_SYNC_TOOLCHAIN/bin/cargo" --version | cut -d ' ' -f 2)" = 1.99.1
for argument in "$@"; do test "$argument" != --ignore-rust-version; done
"$RUSTC" -Vv
"$RUNE_SYNC_TOOLCHAIN/bin/cargo" -Vv
sha256sum Cargo.lock
python3 "$tools/native/owned.py" guard
exec "$RUNE_SYNC_TOOLCHAIN/bin/cargo" --offline "${arguments[@]}"
