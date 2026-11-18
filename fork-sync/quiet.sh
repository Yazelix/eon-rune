#!/usr/bin/env bash
set -euo pipefail
observer=
finish() {
    result=$?
    if test -n "$observer"; then
        kill -TERM "$observer" 2>/dev/null || true
        wait "$observer" 2>/dev/null || true
    fi
    cp "$RUNE_SYNC_HEAVY_LEDGER" "$RUNE_SYNC_STATE/logs/quiet-heavy-after.log" || result=1
    printf 'QUIET DONE %s exit=%s\n' "$(date -u +%FT%TZ)" "$result"
    if test -n "${RUNE_SYNC_STATUS:-}"; then
        printf 'QUIET DONE %s exit=%s\n' "$(date -u +%FT%TZ)" "$result" >> "$RUNE_SYNC_STATUS" || result=1
    fi
    exit "$result"
}
trap finish EXIT
test "$(date +%s)" -ge "$RUNE_SYNC_QUIET_START"
test "$(date +%s)" -lt "$((RUNE_SYNC_QUIET_END - 100))"
cp "$RUNE_SYNC_HEAVY_LEDGER" "$RUNE_SYNC_STATE/logs/quiet-heavy-before.log"
pidstat -h -u 5 > "$RUNE_SYNC_STATE/logs/quiet-cpu.log" 2>&1 &
observer=$!
python3 -u "$RUNE_SYNC_STATE/native-tools/motion.py" acceptance --quiet
test "$(date +%s)" -lt "$RUNE_SYNC_QUIET_END"
