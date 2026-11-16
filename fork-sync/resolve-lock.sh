#!/usr/bin/env bash
# Adapt the fixture graph to Rune while retaining Cargo's compatible locked versions.
set -euo pipefail
tools=$(cd "$(dirname "$0")" && pwd)
state=${1:?materialized state directory}
evidence="$state/provenance/lock"
mkdir "$evidence"
cp "$state/fixtures/Cargo.lock" "$evidence/seed.lock"
# Without --locked Cargo changes only what this candidate graph requires.
bash "$tools/cargo.sh" metadata --format-version 1 \
    --manifest-path "$state/fixtures/Cargo.toml" > "$evidence/candidate-metadata.log"
cp "$state/fixtures/Cargo.lock" "$evidence/resolved.lock"
if git diff --no-index --no-ext-diff --no-color "$evidence/seed.lock" "$evidence/resolved.lock" > "$evidence/resolution.diff"; then
    :
else
    result=$?
    test "$result" = 1
fi
# Both builds use the same resolved dependency versions; incompatibility stops qualification.
cp "$evidence/resolved.lock" "$state/baseline-fixtures/Cargo.lock"
for fixture in fixtures baseline-fixtures; do
    bash "$tools/cargo.sh" metadata --locked --format-version 1 \
        --manifest-path "$state/$fixture/Cargo.toml" > "$evidence/$fixture-locked-metadata.log"
    cmp "$evidence/resolved.lock" "$state/$fixture/Cargo.lock"
done
sha256sum "$evidence/seed.lock" "$evidence/resolved.lock"
