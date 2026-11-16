#!/usr/bin/env bash
# Seal evidence, not the mutable environment shared by successive native phases.
set -euo pipefail
state=${1:?state directory}
find "$state" -type f \
    -not -path '*/focused/*/source/*' -not -path '*/upstream-source/*' \
    -not -path '*/fixtures/*' -not -path '*/baseline-fixtures/*' \
    -not -path '*/native-cache/*' -not -path '*/native-config/*' \
    -not -path '*/native-data/*' -not -path '*/native-state/*' \
    -not -path '*/xdg/*' -not -path '*/xdg-gpu/*' \
    -exec sha256sum '{}' +
