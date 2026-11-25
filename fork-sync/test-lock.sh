#!/usr/bin/env bash
# Offline Cargo graphs prove reviewed lock admission without compiling Rune.
set -euo pipefail
if test "${RUNE_SYNC_BOUNDED:-0}" != 1; then
    export RUNE_SYNC_PRIVATE_ROOT
    RUNE_SYNC_PRIVATE_ROOT=$(mktemp -d "${1:?existing build-disk proof parent}/lock-env-XXXXXX")
    exec python3 "$(dirname "$0")/launch.py" --build bash "$0" "$@"
fi
test "$HOME" = "$RUNE_SYNC_PRIVATE_ROOT/home"
test "$GIT_CONFIG_GLOBAL" = /dev/null
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
cat > "$state/fixtures/Cargo.lock" <<'LOCK'
version = 4
[[package]]
name = "fixture-lock-check"
version = "0.1.0"
dependencies = ["rune"]
[[package]]
name = "rune"
version = "0.1.0"
LOCK
cp "$state/fixtures/Cargo.lock" "$state/baseline-fixtures/Cargo.lock"
# Only the candidate adds a local edge; the pair cannot use a literal shared lock.
cat >> "$state/fixtures/rune/Cargo.toml" <<'MANIFEST'
[dependencies]
new-rune-dependency = { path = "../new-dep" }
MANIFEST
if bash "$tools/cargo.sh" metadata --locked --format-version 1 --manifest-path "$state/fixtures/Cargo.toml" > "$state/stale.log" 2>&1; then
    echo 'stale fixture lock unexpectedly accepted changed Rune dependencies' >&2
    exit 1
fi
mkdir "$state/inputs"
cp "$state/fixtures/Cargo.lock" "$state/inputs/baseline.lock"
cat > "$state/inputs/candidate.lock" <<'LOCK'
version = 4
[[package]]
name = "fixture-lock-check"
version = "0.1.0"
dependencies = ["rune"]
[[package]]
name = "new-rune-dependency"
version = "0.2.0"
[[package]]
name = "rune"
version = "0.1.0"
dependencies = ["new-rune-dependency"]
LOCK
export RUNE_SYNC_LOCKS="$state/inputs/locks.json"
python3 - "$state/inputs" <<'INPUTS'
import hashlib,json,sys
from pathlib import Path
root=Path(sys.argv[1])
entries={}
for key, filename in [('fixtures','candidate.lock'),('baseline-fixtures','baseline.lock')]:
    path=root/filename
    entries[key]=dict(path=str(path),sha256=hashlib.sha256(path.read_bytes()).hexdigest())
(root/'locks.json').write_text(json.dumps(entries))
# Correct file with a wrong declared checksum must be rejected before evidence creation.
entries['fixtures']['sha256']='0'*64
(root/'bad-locks.json').write_text(json.dumps(entries))
INPUTS
if RUNE_SYNC_LOCKS="$state/inputs/bad-locks.json" python3 "$tools/lock-inputs.py" "$state" > "$state/tampered-input.log" 2>&1; then
    echo 'modified lock input incorrectly accepted' >&2
    exit 1
fi
test ! -e "$state/provenance/lock"
python3 "$tools/lock-inputs.py" --self-check
bash "$tools/resolve-lock.sh" "$state" > "$state/resolve.log" 2>&1
grep -q '+name = "new-rune-dependency"' "$state/provenance/lock/fixtures-input.diff"
cmp "$state/inputs/candidate.lock" "$state/fixtures/Cargo.lock"
cmp "$state/inputs/baseline.lock" "$state/baseline-fixtures/Cargo.lock"
test -s "$state/provenance/lock/fixtures-locked-metadata.log"
test -s "$state/provenance/lock/baseline-fixtures-locked-metadata.log"
if bash "$tools/resolve-lock.sh" "$state" > "$state/replay.log" 2>&1; then
    echo 'resolution evidence was overwritten' >&2
    exit 1
fi
bash "$tools/artifacts.sh" "$state" > "$state-artifacts.sha256"
sha256sum --check --status "$state-artifacts.sha256"
printf '\nchanged\n' >> "$state/provenance/lock/fixtures-input.diff"
if sha256sum --check --status "$state-artifacts.sha256"; then
    echo 'modified lock resolution evidence was accepted' >&2
    exit 1
fi
printf 'PASS reviewed locks/local edge difference/stale lock/hash and version-source-checksum rejection/immutable evidence: %s\n' "$state"
