#!/usr/bin/env bash
set -euo pipefail
tools=$1; shift
exec systemd-run --user --scope --quiet --unit="rune-sync-$(date +%s%N)" \
    -p MemoryMax=6442450944 -p MemorySwapMax=0 -p OOMPolicy=continue \
    bash "$tools/bounded.sh" "$@"
