#!/usr/bin/env bash
# GTK's main thread belongs to one test process, including ignored display tests.
set -euo pipefail
cargo test --locked -p maintainer-core -p maintainer-helper --all-features -- --include-ignored
cargo test --locked -p maintainer-app --bins --all-features -- --include-ignored
# App integration targets are separate from its binary and library tests.
integration_metadata=$(cargo test --locked -p maintainer-app --tests --all-features --no-run --message-format=json)
mapfile -t integration_binaries < <(printf '%s\n' "$integration_metadata" | python3 -c '
import json, sys
for line in sys.stdin:
    event = json.loads(line)
    if event.get("reason") == "compiler-artifact" and event.get("profile", {}).get("test") and event.get("target", {}).get("kind") == ["test"] and event.get("executable"):
        print(event["executable"])
')
for integration_binary in "${integration_binaries[@]}"; do
    "$integration_binary" --include-ignored
done
# Compile once, obtain Cargo's actual test executable, then isolate every case.
metadata=$(cargo test --locked -p maintainer-app --lib --all-features --no-run --message-format=json)
binary=$(printf '%s\n' "$metadata" | python3 -c '
import json, sys
executables = [event["executable"] for line in sys.stdin if (event := json.loads(line)).get("reason") == "compiler-artifact" and event.get("profile", {}).get("test") and event.get("target", {}).get("name") == "maintainer_app" and event.get("executable")]
if len(executables) != 1:
    sys.exit("Expected exactly one app library test executable")
print(executables[0])')
listing=$("$binary" --list)
mapfile -t cases < <(printf '%s\n' "$listing" | sed -n 's/: test$//p')
[[ ${#cases[@]} -gt 0 ]] || { echo 'No app tests discovered' >&2; exit 1; }
# Each GTK case retains its own process and private bus; one headless display
# avoids restarting an X server hundreds of times without sharing GTK state.
# Independent clients must not reset the shared server between test processes.
xvfb-run -a -s '-screen 0 1280x1024x24 -noreset' bash -c '
    binary=$1
    shift
    for case in "$@"; do
        dbus-run-session -- "$binary" "$case" --exact --include-ignored || exit "$?"
    done
' test-native "$binary" "${cases[@]}"
