#!/usr/bin/env python3
"""Emit an example response body for an ags operation, read from the bundled
OpenAPI spec.

Usage:
    python3 demos/engine/spec-reader.py <service> <x-operation-id>
    python3 demos/engine/spec-reader.py --selftest

`ags describe` does not expose response schemas (only kind + description), so the
vhs-demo skill reads them straight from crates/ags-runtime/specs/<service>.json.gz.
Prints a pretty JSON example of the operation's 200/201 response to stdout, or
exits non-zero (so the caller falls back to a minimal payload) if the operation or
its schema can't be resolved.
"""
from __future__ import annotations

import gzip
import json
import os
import sys

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
SPECS_DIR = os.path.join(REPO_ROOT, "crates", "ags-runtime", "specs")
MAX_DEPTH = 8


def load_spec(service: str) -> dict:
    path = os.path.join(SPECS_DIR, f"{service}.json.gz")
    with gzip.open(path, "rt", encoding="utf-8") as f:
        return json.load(f)


def find_operation(spec: dict, operation_id: str) -> dict | None:
    """Return the operation object whose x-operationId (or operationId) matches."""
    for _path, methods in spec.get("paths", {}).items():
        for _method, op in methods.items():
            if not isinstance(op, dict):
                continue
            if op.get("x-operationId") == operation_id or op.get("operationId") == operation_id:
                return op
    return None


def resolve_ref(spec: dict, ref: str) -> dict | None:
    """Resolve a local '#/definitions/Name' reference to its schema object."""
    if not ref.startswith("#/"):
        return None
    node: object = spec
    for part in ref[2:].split("/"):
        if not isinstance(node, dict) or part not in node:
            return None
        node = node[part]
    return node if isinstance(node, dict) else None


def example(spec: dict, schema: dict | None, depth: int, seen: set[str]) -> object:
    """Walk a JSON-schema fragment and produce a representative example value.

    Handles `$ref`, object/properties, array, and scalar types. `allOf`/`anyOf`/
    `oneOf` are not composed — such a schema yields `{}`, and the caller falls
    back to a minimal hand-written payload (rare in the bundled specs). Each `{}`
    substitution (depth limit, circular `$ref`, uncomposed schema) warns on stderr
    so the author can spot an incomplete body during authoring; exit stays 0."""
    if schema is None:
        return {}
    if depth > MAX_DEPTH:
        sys.stderr.write(
            f"spec-reader: warn: max depth {MAX_DEPTH} exceeded, substituting {{}}\n"
        )
        return {}
    if "$ref" in schema:
        ref = schema["$ref"]
        if ref in seen:
            sys.stderr.write(
                f"spec-reader: warn: circular $ref {ref}, substituting {{}}\n"
            )
            return {}
        target = resolve_ref(spec, ref)
        return example(spec, target, depth + 1, seen | {ref})
    if (
        any(k in schema for k in ("allOf", "anyOf", "oneOf"))
        and "type" not in schema
        and "properties" not in schema
    ):
        sys.stderr.write(
            "spec-reader: warn: allOf/anyOf/oneOf not composed, substituting {}\n"
        )
        return {}
    t = schema.get("type")
    if t == "object" or "properties" in schema:
        return {
            name: example(spec, prop, depth + 1, seen)
            for name, prop in schema.get("properties", {}).items()
        }
    if t == "array":
        return [example(spec, schema.get("items"), depth + 1, seen)]
    if t == "integer":
        return 0
    if t == "number":
        return 0.0
    if t == "boolean":
        return False
    return "string"


def response_example(service: str, operation_id: str) -> object:
    spec = load_spec(service)
    op = find_operation(spec, operation_id)
    if op is None:
        raise KeyError(f"operation '{operation_id}' not found in {service} spec")
    responses = op.get("responses", {})
    resp = responses.get("200") or responses.get("201")
    schema = (resp or {}).get("schema")
    if schema is None:
        raise KeyError(f"operation '{operation_id}' has no 200/201 response schema")
    return example(spec, schema, 0, set())


def selftest() -> int:
    # achievement/admin/achievements/v1/list returns a paginated object.
    body = response_example("achievement", "achievement/admin/achievements/v1/list")
    assert isinstance(body, dict) and body, f"expected non-empty object, got {body!r}"
    # Unknown ids raise, so the caller can fall back.
    try:
        response_example("achievement", "does/not/exist")
    except KeyError:
        pass
    else:
        raise AssertionError("expected KeyError for unknown operation id")
    print("spec-reader selftest: OK")
    return 0


def main(argv: list[str]) -> int:
    if "--selftest" in argv:
        return selftest()
    if len(argv) != 2:
        sys.stderr.write("usage: spec-reader.py <service> <x-operation-id>\n")
        return 2
    service, operation_id = argv
    try:
        body = response_example(service, operation_id)
    except (OSError, KeyError, ValueError) as e:
        sys.stderr.write(f"spec-reader: {e}\n")
        return 1
    print(json.dumps(body, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
