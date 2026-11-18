#!/usr/bin/env bash
set -euo pipefail
tools=$(cd "$(dirname "$0")" && pwd)
exec python3 "$tools/bounded.py" "$@"
