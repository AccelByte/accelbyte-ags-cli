#!/usr/bin/env bash
# Regenerate the README demo GIF.
#
# Starts the mock server on localhost, runs VHS against demo/reel.tape,
# and kills the server on exit (including on error or interrupt).
#
# Requirements: cargo, python3, vhs (see CONTRIBUTING.md).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "==> Building ags (release)"
cargo build --release

# The tape uses AGS_HOME=/tmp/ags-demo-state and AGS_NO_KEYCHAIN=1 so the
# recording starts unauthenticated. Wipe any leftover state from prior runs.
rm -rf /tmp/ags-demo-state

# Reap any mock server still listening from a previous (possibly aborted) run
# before starting our own — otherwise we'd attach to it and fail to clean up.
pkill -f 'demo/demo-server\.py' 2>/dev/null || true

echo "==> Starting mock server on localhost:8765"
python3 demo/demo-server.py &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null || true' EXIT

# Wait for the server to accept connections rather than guessing a fixed delay.
for _ in $(seq 1 30); do
    if python3 -c "import socket; s=socket.socket(); s.settimeout(0.2); s.connect(('127.0.0.1', 8765)); s.close()" 2>/dev/null; then
        break
    fi
    sleep 0.1
done

echo "==> Pre-warming spec cache (avoids a 'Preparing specs…' flash on the first IAM call)"
AGS_HOME=/tmp/ags-demo-state AGS_NO_KEYCHAIN=1 \
    "$REPO_ROOT/target/release/ags" iam --help >/dev/null 2>&1

echo "==> Running VHS"
# Put the freshly built binary first on PATH so the tape runs it.
PATH="$REPO_ROOT/target/release:$PATH" vhs demo/reel.tape

echo "==> Done: demo/reel.gif"
