#!/usr/bin/env python3
"""Minimal stand-in for the AGS platform + AMS upload API + S3 storage.

Lets `ags ams upload` be exercised end to end with no tenant. Logs every call
so the request sequence is visible, and verifies the bytes it received
reassemble into the archive the CLI said it sent.

    python3 scripts/ams-upload-mock.py 8799

Then point the CLI at it:

    AGS_BASE_URL=http://127.0.0.1:8799 AGS_ACCESS_TOKEN=fake-token \
        ags ams upload --path ./build --executable server --image-name test-image

This is a manual-testing aid, not part of the test suite — the automated
coverage lives in `crates/ags-runtime/src/runtime/ams_upload/tests.rs` and
`crates/accelbyte-ags-cli/tests/integration/ams_upload.rs`.
"""
import hashlib
import json
import re
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8799
BASE = f"http://127.0.0.1:{PORT}"

GZIP_MAGIC = b"\x1f\x8b"
received = {}  # part label -> bytes


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass  # we print our own, tidier log

    def _body(self):
        length = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(length) if length else b""

    def _send(self, code, payload=None, headers=None):
        data = b"" if payload is None else (
            payload if isinstance(payload, bytes) else json.dumps(payload).encode()
        )
        self.send_response(code)
        if payload is not None and not isinstance(payload, bytes):
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(data)

    def _note(self, label, extra=""):
        print(f"  {label:<34} {extra}", flush=True)

    def do_GET(self):
        if self.path.startswith("/ams/v1/upload-url"):
            self._note("GET /ams/v1/upload-url", f"-> {BASE}")
            self._send(200, BASE.encode())
            return
        self._send(404)

    def do_POST(self):
        body = self._body()
        parsed = json.loads(body) if body else {}
        if self.path == "/upload/v1/images":
            self._note(
                "POST /upload/v1/images",
                f"name={parsed.get('name')} arch={parsed.get('targetArchitecture')} "
                f"format={parsed.get('format')} src={self.headers.get('ams-source-environment')} "
                f"cli={self.headers.get('ams-cli-version')}",
            )
            self._send(201, {"id": "img-mock-1"})
            return
        if self.path == "/upload/v1/pre-sign-url":
            self._note("POST /upload/v1/pre-sign-url", f"filePath={parsed.get('filePath')}")
            self._send(200, {"url": f"{BASE}/storage?part=whole"})
            return
        if self.path == "/upload/v1/multi-part":
            self._note("POST /upload/v1/multi-part", f"filePath={parsed.get('filePath')}")
            self._send(200, {"uploadId": "up-mock-1"})
            return
        self._send(404)

    def do_PUT(self):
        body = self._body()
        if self.path.startswith("/storage"):
            label = re.search(r"part=([^&]+)", self.path).group(1)
            received[label] = body
            self._note(f"PUT /storage part={label}", f"{len(body)} bytes")
            self._send(200, b"", {"ETag": f'"etag-{label}"'})
            return

        parsed = json.loads(body) if body else {}
        if re.fullmatch(r"/upload/v1/multi-part/[^/]+", self.path):
            part_no = parsed.get("partNo")
            self._note("PUT /upload/v1/multi-part/{id}", f"partNo={part_no}")
            self._send(200, {"url": f"{BASE}/storage?part={part_no}"})
            return
        if re.fullmatch(r"/upload/v1/multi-part/[^/]+/finalize", self.path):
            parts = parsed.get("parts", [])
            expected = [f'"etag-{i + 1}"' for i in range(len(parts))]
            ordered = "OK" if parts == expected else f"WRONG ORDER: {parts}"
            self._note("PUT .../finalize", f"{len(parts)} etags, order={ordered}")
            self._summarise()
            self._send(200)
            return
        if self.path == "/upload/v1/complete":
            self._note(
                "PUT /upload/v1/complete",
                f"imageId={parsed.get('imageId')} bytes={parsed.get('imageSizeBytes')} "
                f"command={parsed.get('command')}",
            )
            if "whole" in received:
                self._summarise()
            print("  ✔ upload sequence complete\n", flush=True)
            received.clear()
            self._send(200)
            return
        self._send(404)

    def _summarise(self):
        if "whole" in received:
            blob = received["whole"]
        else:
            blob = b"".join(received[str(i + 1)] for i in range(len(received)))
        is_gzip = "OK" if blob[:2] == GZIP_MAGIC else "BAD"
        digest = hashlib.sha256(blob).hexdigest()[:16]
        print(
            f"  reassembled {len(blob)} bytes, sha256={digest}, gzip magic={is_gzip}",
            flush=True,
        )


print(f"AMS mock listening on {BASE}\n", flush=True)
HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
