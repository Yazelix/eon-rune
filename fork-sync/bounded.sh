#!/usr/bin/env bash
set -euo pipefail
group=$(sed -n 's/^0:://p' /proc/self/cgroup)
scope=/sys/fs/cgroup$group
test "$(cat "$scope/memory.max")" = 6442450944
test "$(cat "$scope/memory.swap.max")" = 0
printf 'cgroup=%s memory.max=6442450944 memory.swap.max=0\n' "$group"
set +e
/usr/bin/time -v "$@"
result=$?
set -e
cat "$scope/memory.peak" "$scope/memory.events"
exit "$result"
