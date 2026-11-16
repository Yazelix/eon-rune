#!/usr/bin/env bash
# Offline Cargo graphs prove dependency drift handling without compiling Rune.
set -euo pipefail
tools=$(cd "$(dirname "$0")" && pwd)
state=$(mktemp -d "${1:?existing proof parent}/lock-proof-XXXXXX")
export CARGO_NET_OFFLINE=true
mkdir "$state/provenance"
for fixture in fixtures baseline-fixtures; do
    mkdir -p "$state/$fixture/src" "$state/$fixture/rune/src" "$state/$fixture/new-dep/src"
    touch "$state/$fixture/src/lib.rs" "$state/$fixture/rune/src/lib.rs" "$state/$fixture/new-dep/src/lib.rs"
    cat > "$state/$fixture/Cargo.toml" <<'MANIFEST'
[package]
name = "fixture-lock-check"
version = "0.1.0"
edition = "2024"
[dependencies]
rune = { path = "rune" }
MANIFEST
    cat > "$state/$fixture/rune/Cargo.toml" <<'MANIFEST'
[package]
name = "rune"
version = "0.1.0"
edition = "2024"
MANIFEST
    cat > "$state/$fixture/new-dep/Cargo.toml" <<'MANIFEST'
[package]
name = "new-rune-dependency"
version = "0.2.0"
edition = "2024"
MANIFEST
done
printf 'version = 4\n' > "$state/fixtures/Cargo.lock"
bash "$tools/cargo.sh" generate-lockfile --manifest-path "$state/fixtures/Cargo.toml" > "$state/seed.log" 2>&1
cp "$state/fixtures/Cargo.lock" "$state/baseline-fixtures/Cargo.lock"
# This is a Rune dependency change, not a change to the fixture manifest or seed.
for fixture in fixtures baseline-fixtures; do
    cat >> "$state/$fixture/rune/Cargo.toml" <<'MANIFEST'
[dependencies]
new-rune-dependency = { path = "../new-dep" }
MANIFEST
done
if bash "$tools/cargo.sh" metadata --locked --format-version 1 --manifest-path "$state/fixtures/Cargo.toml" > "$state/stale.log" 2>&1; then
    echo 'stale fixture lock unexpectedly accepted changed Rune dependencies' >&2
    exit 1
fi
bash "$tools/resolve-lock.sh" "$state" > "$state/resolve.log" 2>&1
grep -q '+name = "new-rune-dependency"' "$state/provenance/lock/resolution.diff"
cmp "$state/fixtures/Cargo.lock" "$state/baseline-fixtures/Cargo.lock"
test -s "$state/provenance/lock/fixtures-locked-metadata.log"
test -s "$state/provenance/lock/baseline-fixtures-locked-metadata.log"
if bash "$tools/resolve-lock.sh" "$state" > "$state/replay.log" 2>&1; then
    echo 'resolution evidence was overwritten' >&2
    exit 1
fi
bash "$tools/artifacts.sh" "$state" > "$state-artifacts.sha256"
sha256sum --check --status "$state-artifacts.sha256"
printf '\nchanged\n' >> "$state/provenance/lock/resolution.diff"
if sha256sum --check --status "$state-artifacts.sha256"; then
    echo 'modified lock resolution evidence was accepted' >&2
    exit 1
fi
printf 'PASS Rune-dependency advance/stale-lock rejection/shared resolved lock/immutable evidence: %s\n' "$state"
