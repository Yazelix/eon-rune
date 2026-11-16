#!/usr/bin/env bash
set -euo pipefail
for kind in CACHE CONFIG DATA STATE; do
    folder="$RUNE_SYNC_STATE/native-${kind,,}"
    mkdir -p "$folder"
    export "XDG_${kind}_HOME=$folder"
done
export XDG_RUNTIME_DIR="$RUNE_SYNC_STATE/xdg"
unset DISPLAY WAYLAND_DISPLAY SWAYSOCK I3SOCK DBUS_SESSION_BUS_ADDRESS AT_SPI_BUS_ADDRESS XDG_ACTIVATION_TOKEN DESKTOP_STARTUP_ID
exec dbus-run-session --config-file="$RUNE_SYNC_STATE/native-dbus.conf" -- python3 "$RUNE_SYNC_STATE/native-tools/primary.py" functional
