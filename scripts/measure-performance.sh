#!/usr/bin/env bash
# Unprivileged diagnostic measurements; never drops host caches.
set -euo pipefail
[[ $EUID -ne 0 ]] || { printf '{"category":"environment-error","reason":"run measurements as an unprivileged user"}\n'; exit 2; }
ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
command -v python3 >/dev/null || { printf '{"category":"environment-error","reason":"python3 is required"}\n'; exit 2; }
if [[ ${PERF_SKIP_XVFB:-0} == 1 ]]; then
    exec python3 "$ROOT/scripts/measure-performance.py" "$@"
fi
command -v xvfb-run >/dev/null || { printf '{"category":"environment-error","reason":"xvfb-run is required"}\n'; exit 2; }
export PERF_HARNESS_XVFB=1
exec xvfb-run -a python3 "$ROOT/scripts/measure-performance.py" "$@"
