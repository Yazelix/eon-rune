#!/usr/bin/env bash
set -euo pipefail
tools=$1; shift
group=$(sed -n 's/^0:://p' /proc/self/cgroup)
case "$group" in
    */heavy-*.scope)
        test "$(cat "/sys/fs/cgroup$group/memory.max")" = 8589934592
        test "$(cat "/sys/fs/cgroup$group/memory.swap.max")" = 0
        exec bash "$tools/bounded.sh" "$@"
        ;;
esac
# Older schedulers have no job scope. Keep the same explicit cap as a fallback.
exec systemd-run --user --scope --quiet --unit="rune-sync-$(date +%s%N)" \
    -p MemoryMax=8589934592 -p MemorySwapMax=0 -p OOMPolicy=continue \
    bash "$tools/bounded.sh" "$@"
