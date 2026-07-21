#!/usr/bin/env python3
"""Config-driven mock server for AGS CLI VHS demos.

Loads a routing table (JSON) from $AGS_DEMO_ROUTES and serves it on
127.0.0.1:$AGS_DEMO_PORT. Each route matches METHOD + a path pattern whose
{placeholder} segments match any single path segment; the query string is
ignored. Misses return 404 and are logged to stderr so a demo author sees which
endpoints still need a route.

Run the route-matcher self-check (no server, no recording):
    python3 demos/engine/mock-server.py --selftest
"""
from __future__ import annotations

import json
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = int(os.environ.get("AGS_DEMO_PORT", "8765"))
DEFAULT_ROUTES = os.path.join(
    os.path.dirname(__file__), "..", "onboarding", "onboarding.routes.json"
)
ROUTES_PATH = os.environ.get("AGS_DEMO_ROUTES", DEFAULT_ROUTES)

# Per-route delays make progress spinners legible; override to 0 for fast
# iteration. A route's own "delay" wins; these are the fallbacks.
RESPONSE_DELAY = float(os.environ.get("AGS_DEMO_RESPONSE_DELAY", "0.6"))
TOKEN_DELAY = float(os.environ.get("AGS_DEMO_TOKEN_DELAY", "1.2"))


def load_routes(path: str) -> list[dict]:
    with open(path, encoding="utf-8") as f:
        routes = json.load(f)
    if not isinstance(routes, list):
        raise ValueError(f"{path}: routes file must be a JSON array")
    return routes


def path_matches(pattern: str, actual: str) -> bool:
    """True when `actual` matches `pattern`, treating `{...}` segments as
    single-segment wildcards. Caller strips the query string first."""
    p = pattern.strip("/").split("/")
    a = actual.strip("/").split("/")
    if len(p) != len(a):
        return False
    return all(
        (seg.startswith("{") and seg.endswith("}")) or seg == act
        for seg, act in zip(p, a)
    )


def match_route(routes: list[dict], method: str, path: str) -> dict | None:
    """First route whose method and path pattern match, else None."""
    for route in routes:
        if route.get("method", "GET").upper() == method.upper() and path_matches(
            route["path"], path
        ):
            return route
    return None


class Handler(BaseHTTPRequestHandler):
    routes: list[dict] = []

    def _delay_for(self, route: dict) -> float:
        if "delay" in route:
            return float(route["delay"])
        # Only the OAuth token endpoint gets the longer fallback delay; match the
        # specific path so unrelated routes that merely contain "token" don't.
        return TOKEN_DELAY if "/oauth/token" in route["path"] else RESPONSE_DELAY

    def _dispatch(self, method: str) -> None:
        path = self.path.split("?", 1)[0]
        length = int(self.headers.get("Content-Length", "0"))
        if length:
            self.rfile.read(length)
        route = match_route(self.routes, method, path)
        if route is None:
            self._send(404, {"error": f"no mock for {method} {path}"})
            return
        delay = self._delay_for(route)
        if delay > 0:
            time.sleep(delay)
        self._send(int(route.get("status", 200)), route.get("body", {}))

    def _send(self, status: int, body: dict) -> None:
        payload = json.dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self) -> None:
        self._dispatch("GET")

    def do_POST(self) -> None:
        self._dispatch("POST")

    def do_PUT(self) -> None:
        self._dispatch("PUT")

    def do_DELETE(self) -> None:
        self._dispatch("DELETE")

    def do_PATCH(self) -> None:
        self._dispatch("PATCH")

    def log_message(self, fmt: str, *args) -> None:
        sys.stderr.write("mock-server: " + (fmt % args) + "\n")

    def log_error(self, fmt: str, *args) -> None:
        sys.stderr.write("mock-server: ERROR " + (fmt % args) + "\n")


def selftest() -> int:
    routes = [
        {"method": "POST", "path": "/iam/v3/oauth/token", "body": {}},
        {"method": "GET", "path": "/social/v1/admin/namespaces/{namespace}/stats", "body": {}},
    ]
    assert match_route(routes, "POST", "/iam/v3/oauth/token") is routes[0]
    assert match_route(routes, "GET", "/social/v1/admin/namespaces/dev/stats") is routes[1]
    assert match_route(routes, "POST", "/social/v1/admin/namespaces/dev/stats") is None
    assert match_route(routes, "GET", "/iam/v3/oauth/token") is None
    assert match_route(routes, "GET", "/social/v1/admin/namespaces/dev/stats/x") is None
    assert match_route(routes, "GET", "/nope") is None
    print("mock-server selftest: OK")
    return 0


def main() -> int:
    if "--selftest" in sys.argv[1:]:
        return selftest()
    Handler.routes = load_routes(ROUTES_PATH)
    server = HTTPServer(("127.0.0.1", PORT), Handler)
    sys.stderr.write(f"mock-server: listening on 127.0.0.1:{PORT}, routes={ROUTES_PATH}\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
