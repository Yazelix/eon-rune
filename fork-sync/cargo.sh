#!/usr/bin/env bash
set -euo pipefail
: "${RUNE_SYNC_TOOLCHAIN:?absolute Rust 1.96.1 toolchain directory}"
: "${CARGO_TARGET_DIR:?dedicated target directory}"
export RUSTC="$RUNE_SYNC_TOOLCHAIN/bin/rustc"
test "$("$RUSTC" --version | cut -d ' ' -f 2)" = 1.96.1
test "$("$RUNE_SYNC_TOOLCHAIN/bin/cargo" --version | cut -d ' ' -f 2)" = 1.96.1
for argument in "$@"; do test "$argument" != --ignore-rust-version; done
"$RUSTC" -Vv
"$RUNE_SYNC_TOOLCHAIN/bin/cargo" -Vv
sha256sum Cargo.lock
exec "$RUNE_SYNC_TOOLCHAIN/bin/cargo" "$@"
