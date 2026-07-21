#!/usr/bin/env bash
# Record a VHS demo.  Usage: demos/record.sh <name>
#
# Builds ags (release), starts the mock with demos/<name>/<name>.routes.json,
# waits for the port, pre-warms the spec cache, then runs vhs on the tape.
# An optional demos/<name>/prewarm file (one service display-name per line) warms
# those services' specs so the first on-camera command shows no "Preparing specs…".
#
# Requirements: cargo, python3, vhs (see CONTRIBUTING.md).
set -euo pipefail

NAME="${1:?usage: demos/record.sh <name>}"
# Demo names are kebab-case (see SKILL.md). Validate before interpolating into
# paths / pkill so a stray value can't traverse outside demos/ or inject.
[[ "$NAME" =~ ^[a-z0-9-]+$ ]] || {
    echo "demo name must be kebab-case alphanumeric: $NAME" >&2
    exit 1
}
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DEMO_DIR="demos/$NAME"
TAPE="$DEMO_DIR/$NAME.tape"
ROUTES="$DEMO_DIR/$NAME.routes.json"
# The mock binds $AGS_DEMO_PORT (default 8765); the tapes' Env AGS_BASE_URL also
# uses 8765, so overriding the port additionally requires editing the tape.
DEMO_PORT="${AGS_DEMO_PORT:-8765}"
[ -f "$TAPE" ]   || { echo "no tape: $TAPE" >&2; exit 1; }
[ -f "$ROUTES" ] || { echo "no routes: $ROUTES" >&2; exit 1; }

echo "==> Building ags (release)"
cargo build --release

# Tapes use a throwaway AGS_HOME + AGS_NO_KEYCHAIN=1 so recording starts
# unauthenticated. Wipe leftover state from prior runs.
rm -rf /tmp/ags-demo-state

# Reap any mock still listening from an aborted run before starting our own.
pkill -f 'demos/engine/mock-server\.py' 2>/dev/null || true

echo "==> Starting mock server (routes: $ROUTES)"
AGS_DEMO_ROUTES="$ROUTES" python3 demos/engine/mock-server.py &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null || true' EXIT

ready=
for _ in $(seq 1 30); do
    if python3 -c "import socket,sys; s=socket.socket(); s.settimeout(0.2); s.connect(('127.0.0.1', int(sys.argv[1]))); s.close()" "$DEMO_PORT" 2>/dev/null; then
        ready=1
        break
    fi
    # Bail early if the mock died (port in use, Python error) rather than
    # recording connection-refused output into the GIF.
    kill -0 "$SERVER_PID" 2>/dev/null || { echo "mock server exited before becoming ready" >&2; exit 1; }
    sleep 0.1
done
[ -n "$ready" ] || { echo "mock server did not open port $DEMO_PORT in time" >&2; exit 1; }

if [ -f "$DEMO_DIR/prewarm" ]; then
    echo "==> Pre-warming spec cache"
    while read -r svc; do
        [ -n "$svc" ] || continue
        AGS_HOME=/tmp/ags-demo-state AGS_NO_KEYCHAIN=1 \
            "$REPO_ROOT/target/release/ags" "$svc" --help >/dev/null 2>&1 || true
    done < "$DEMO_DIR/prewarm"
fi

# Off-camera authentication. A demos/<name>/prelogin marker means "this demo runs
# live commands but does NOT show auth" — log in here, into the same throwaway
# AGS_HOME the tape uses, so the token is already present when recording starts
# and no login appears in the GIF. Demos where auth IS the subject omit the marker
# and show the login on-camera in their tape instead.
if [ -f "$DEMO_DIR/prelogin" ]; then
    echo "==> Pre-authenticating (off-camera)"
    # demo-only credentials: the mock token route accepts anything (see routes.json).
    AGS_HOME=/tmp/ags-demo-state AGS_NO_KEYCHAIN=1 AGS_BASE_URL="http://localhost:$DEMO_PORT" \
        "$REPO_ROOT/target/release/ags" auth login --grant client-credentials \
        --client-id demo --client-secret-stdin <<< 'demo-secret' >/dev/null 2>&1 \
        || { echo "pre-authentication failed (is the token route mocked?)" >&2; exit 1; }
fi

echo "==> Running VHS"
PATH="$REPO_ROOT/target/release:$PATH" vhs "$TAPE"

echo "==> Done: $DEMO_DIR/$NAME.gif"
