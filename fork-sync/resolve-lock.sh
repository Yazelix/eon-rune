#!/usr/bin/env bash
# Qualification consumes approved private lock inputs without resolving versions.
set -euo pipefail
tools=$(cd "$(dirname "$0")" && pwd)
state=${1:?materialized state directory}
evidence="$state/provenance/lock"
python3 "$tools/lock-inputs.py" "$state"
for fixture in fixtures baseline-fixtures; do
    bash "$tools/cargo.sh" metadata --offline --locked --format-version 1 \
        --manifest-path "$state/$fixture/Cargo.toml" > "$evidence/$fixture-locked-metadata.log"
    recorded=resolved.lock
    test "$fixture" != baseline-fixtures || recorded=baseline-resolved.lock
    cmp "$evidence/$recorded" "$state/$fixture/Cargo.lock"
done
sha256sum "$evidence/resolved.lock" "$evidence/baseline-resolved.lock"
