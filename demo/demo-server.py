#!/usr/bin/env python3
"""Minimal mock server for the AGS CLI demo reel.

Listens on 127.0.0.1:$AGS_DEMO_PORT (default 8765) and serves just enough
of the IAM API for the reel: a token endpoint and a roles list.
Exits cleanly on SIGTERM/SIGINT so record.sh can tear it down.
"""
from __future__ import annotations

import json
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = int(os.environ.get("AGS_DEMO_PORT", "8765"))

# Slow every response so progress spinners are visible in the demo recording.
# The token endpoint gets a longer delay because the "Authenticating…" line
# would otherwise flash past too quickly to read.
# Override either with AGS_DEMO_*_DELAY=0 for instant replies.
RESPONSE_DELAY_SECS = float(os.environ.get("AGS_DEMO_RESPONSE_DELAY", "0.6"))
TOKEN_DELAY_SECS = float(os.environ.get("AGS_DEMO_TOKEN_DELAY", "1.2"))

TOKEN_RESPONSE = {
    "access_token": "demo-token",
    "token_type": "Bearer",
    "expires_in": 3600,
}

ROLES_RESPONSE = {
    "data": [
        {
            "roleId": "11111111-1111-1111-1111-111111111111",
            "roleName": "Game Admin",
            "adminRole": True,
            "isWildcard": False,
            "permissions": [],
        },
        {
            "roleId": "22222222-2222-2222-2222-222222222222",
            "roleName": "Support Agent",
            "adminRole": False,
            "isWildcard": False,
            "permissions": [],
        },
        {
            "roleId": "33333333-3333-3333-3333-333333333333",
            "roleName": "Read-Only Viewer",
            "adminRole": False,
            "isWildcard": False,
            "permissions": [],
        },
    ],
    "paging": {"first": "", "last": "", "next": "", "previous": ""},
}


class Handler(BaseHTTPRequestHandler):
    def _send_json(self, status: int, body: dict, delay: float = RESPONSE_DELAY_SECS) -> None:
        if delay > 0:
            time.sleep(delay)
        payload = json.dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_POST(self) -> None:
        if self.path == "/iam/v3/oauth/token":
            length = int(self.headers.get("Content-Length", "0"))
            if length:
                self.rfile.read(length)
            self._send_json(200, TOKEN_RESPONSE, delay=TOKEN_DELAY_SECS)
            return
        self._send_json(404, {"error": f"no mock for POST {self.path}"})

    def do_GET(self) -> None:
        route = self.path.split("?", 1)[0]
        if route == "/iam/v4/admin/roles":
            self._send_json(200, ROLES_RESPONSE)
            return
        self._send_json(404, {"error": f"no mock for GET {route}"})

    def log_message(self, fmt: str, *args) -> None:
        sys.stderr.write("demo-server: " + (fmt % args) + "\n")


def main() -> None:
    # HTTPServer sets allow_reuse_address=True, so rapid re-runs after a
    # SIGTERM-kill from record.sh don't collide on the port.
    server = HTTPServer(("127.0.0.1", PORT), Handler)
    sys.stderr.write(f"demo-server: listening on 127.0.0.1:{PORT}\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
