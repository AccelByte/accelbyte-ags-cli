#!/usr/bin/env python3
"""Generate the CLI command catalogue from AccelByte OpenAPI specs.

Reads each gzipped spec in ../crates/ags-runtime/specs and groups operations by their
`x-operationId` segments (service/scope/resource/version/method). Renders a
Markdown catalogue listing every operation the CLI actually dispatches —
i.e. non-deprecated, non-internal, with a parseable `x-operationId`.

The same walk also drives `--emit-baselines DIR`, which writes per-service
input-contract baselines used by the Rust parser tests.
"""

from __future__ import annotations

import argparse
import gzip
import json
import os
import sys
from collections import defaultdict
from dataclasses import dataclass

SPEC_DIR = os.path.join(
    os.path.dirname(__file__), "..", "crates", "ags-runtime", "specs"
)
DEFAULT_OUTPUT = os.path.join(
    os.path.dirname(__file__), "..", "docs", "reference", "cli-command-catalogue.md"
)

SERVICES = [
    "achievement", "ams", "basic", "challenge", "chat", "cloudsave",
    "csm", "ehs", "gametelemetry", "gdpr", "group", "iam", "inventory",
    "leaderboard", "legal", "lobby", "loginqueue", "match2",
    "platform", "reporting", "seasonpass", "session", "sessionhistory",
    "social", "ugc",
]

DISPLAY_NAMES = {
    "cloudsave": "cloud-save",
    "gametelemetry": "game-telemetry",
    "loginqueue": "login-queue",
    "match2": "matchmaking",
    "seasonpass": "season-pass",
    "sessionhistory": "session-history",
}


@dataclass
class Operation:
    resource: str
    method: str
    api_version: int
    path: str
    http_method: str
    x_operation_id: str
    summary: str
    description: str
    permissions: list[str]
    parameters: list[dict]
    has_request_body: bool = False
    scope: str = ""


def load_spec(service: str) -> dict:
    path = os.path.join(SPEC_DIR, f"{service}.json.gz")
    with gzip.open(path, "rt", encoding="utf-8") as f:
        return json.load(f)


def split_x_operation_id(value: str) -> tuple[str, str, str, int, str] | None:
    """service/scope/resource/version/method → tuple, or None if malformed.

    Mirrors the Rust parser: an id must have exactly 5 segments (both underflow
    and overflow are spec bugs the parser skips), and a numeric version. Version
    stripping removes exactly one leading "v" to match `ApiVersion::from_str`
    (`s.strip_prefix('v')`), so "vv1" is rejected rather than coerced to 1.
    A version with no leading "v" (e.g. "1") is also accepted, matching the unwrap_or(s) fallback in ApiVersion::from_str.
    """
    parts = value.split("/")
    if len(parts) != 5:
        return None
    service, scope, resource, version, method = parts
    v = version[1:] if version.startswith("v") else version
    if not v.isdigit():
        return None
    api_version = int(v)
    return service, scope, resource, api_version, method


def extract_permissions(x_security) -> list[str]:
    if not isinstance(x_security, list):
        return []
    perms: list[str] = []
    for entry in x_security:
        if isinstance(entry, dict):
            for p in entry.get("permissions", []) or []:
                resource = p.get("resource", "")
                action = p.get("action", "")
                if resource:
                    perms.append(f"{action}:{resource}".strip(":"))
    return sorted(set(perms))


_FLAG_LOCATIONS = {"path": "path", "query": "query", "header": "header", "formData": "form_data"}


def extract_flag_parameters(operation: dict) -> list[dict]:
    """Mirror the Rust parser: operation-level parameters, excluding body.

    Returns name + normalized location + required, sorted deterministically.
    Path-level parameters are intentionally ignored — the Rust parser only
    reads operation-level parameters, and no bundled spec uses path-level today.
    """
    extracted: list[dict] = []
    for parameter in operation.get("parameters") or []:
        if not isinstance(parameter, dict):
            continue
        location = _FLAG_LOCATIONS.get(parameter.get("in"))
        if location is None:
            continue
        name = parameter.get("name")
        if not name:
            continue
        extracted.append({
            "name": name,
            "location": location,
            "required": bool(parameter.get("required", False)),
        })
    extracted.sort(key=lambda entry: (entry["location"], entry["name"]))
    return extracted


_SCALAR_ITEM_TYPES = {"string", "integer", "number", "boolean"}


def _definition_has_fields(spec: dict, ref: str) -> bool:
    """True if a `#/definitions/X` ref points at a definition with properties."""
    name = ref.rsplit("/", 1)[-1]
    definition = spec.get("definitions", {}).get(name)
    return isinstance(definition, dict) and bool(definition.get("properties"))


def has_request_body(spec: dict, operation: dict) -> bool:
    """Mirror the Rust parser's `request_body.is_some()` (resolve_body_schema)."""
    # Locating the body parameter IS the body-presence check: it subsumes the
    # parser's separate `has_body` pre-check (parser.rs) — both mean "no body
    # parameter -> bodyless". resolve_body_schema likewise re-finds it and
    # returns None when absent, so a single find here stays faithful.
    body = next(
        (p for p in (operation.get("parameters") or [])
         if isinstance(p, dict) and p.get("in") == "body"),
        None,
    )
    if body is None:
        return False
    schema = body.get("schema")
    if not isinstance(schema, dict):
        return False
    # Object body: top-level $ref.
    if schema.get("$ref"):
        return _definition_has_fields(spec, schema["$ref"])
    # Otherwise must be an explicit array.
    if schema.get("type") != "array":
        return False
    items = schema.get("items")
    if not isinstance(items, dict):
        return False
    # Object array: items is a $ref.
    if items.get("$ref"):
        return _definition_has_fields(spec, items["$ref"])
    # Scalar array: a recognised primitive item type. Items with neither $ref
    # nor type (or an unrecognised type) are bodyless — mirror the parser
    # explicitly rather than leaning on `None in set` happening to be False.
    item_type = items.get("type")
    if item_type is None:
        return False
    return item_type in _SCALAR_ITEM_TYPES


def _build_operation(
    spec: dict, path: str, http_method: str, op: dict, parsed: tuple[str, str, str, int, str]
) -> Operation:
    _service, scope, resource, version, method = parsed
    return Operation(
        resource=resource,
        method=method,
        api_version=version,
        path=path,
        http_method=http_method.upper(),
        x_operation_id=op.get("x-operationId", ""),
        summary=(op.get("summary") or "").strip(),
        description=(op.get("description") or "").strip(),
        permissions=extract_permissions(op.get("x-security")),
        parameters=extract_flag_parameters(op),
        scope=scope,
        has_request_body=has_request_body(spec, op),
    )


def collect_parser_contract(spec: dict) -> dict[str, list[Operation]]:
    """Mirror the Rust parser contract: every operation the CLI dispatches.

    Skips deprecated operations, the `internal` resource, and malformed
    `x-operationId` entries. Keeps every (scope, version) combination because
    the CLI exposes all of them via `--api-scope` / `--api-version`.
    """
    by_resource: dict[str, list[Operation]] = defaultdict(list)
    for path, methods in (spec.get("paths") or {}).items():
        for http_method in ("get", "post", "put", "patch", "delete"):
            op = methods.get(http_method)
            if not op:
                continue
            if op.get("deprecated"):
                continue
            x_op_id = op.get("x-operationId")
            if not x_op_id:
                continue
            parsed = split_x_operation_id(x_op_id)
            if not parsed:
                continue
            _service, _scope, resource, _version, _method = parsed
            if resource == "internal":
                continue
            by_resource[resource].append(_build_operation(spec, path, http_method, op, parsed))
    for ops in by_resource.values():
        ops.sort(key=lambda o: (o.method, o.scope, o.api_version, o.path))
    return dict(sorted(by_resource.items()))


def baseline_for_service(resources: dict[str, list[Operation]]) -> dict[str, list[dict]]:
    """Build the per-service contract baseline payload."""
    out: dict[str, list[dict]] = {}
    for resource_name, ops in resources.items():
        out[resource_name] = [
            {
                "method": op.method,
                "summary": op.summary,
                "http_method": op.http_method,
                "path": op.path,
                "x_operation_id": op.x_operation_id,
                "permissions": op.permissions,
                "parameters": op.parameters,
                "has_request_body": op.has_request_body,
            }
            for op in ops
        ]
    return out


def render_catalogue(per_service: dict[str, dict[str, list[Operation]]]) -> str:
    lines: list[str] = []
    lines.append("# AGS CLI Command Catalogue")
    lines.append("")
    lines.append(
        "Auto-generated from `specs/*.json.gz` via "
        "`scripts/generate_cli_command_catalogue.py`. Do not hand-edit."
    )
    lines.append("")
    lines.append(
        "Lists every operation the CLI dispatches. Deprecated operations and "
        "the `internal` resource are excluded. Each row corresponds to a "
        "concrete `--api-scope` / `--api-version` combination."
    )
    lines.append("")
    lines.append(f"**Services:** {len(per_service)}")
    total_ops = sum(
        sum(len(ops) for ops in service.values()) for service in per_service.values()
    )
    lines.append(f"**Operations:** {total_ops}")
    lines.append("")

    for service_name in sorted(per_service.keys()):
        display = DISPLAY_NAMES.get(service_name, service_name)
        resources = per_service[service_name]
        op_count = sum(len(ops) for ops in resources.values())
        lines.append(f"## {display}")
        lines.append("")
        lines.append(f"- Spec name: `{service_name}`")
        lines.append(f"- Resources: {len(resources)}")
        lines.append(f"- Operations: {op_count}")
        lines.append("")
        lines.append("| Resource | Method | Scope | Version | HTTP | Path | Summary |")
        lines.append("|----------|--------|-------|---------|------|------|---------|")
        for resource_name in sorted(resources.keys()):
            for op in resources[resource_name]:
                summary = op.summary.replace("|", "\\|")
                version_cell = f"v{op.api_version}" if op.api_version else ""
                lines.append(
                    f"| `{resource_name}` | `{op.method}` | {op.scope} | "
                    f"{version_cell} | {op.http_method} | `{op.path}` | {summary} |"
                )
        lines.append("")

    return "\n".join(lines) + "\n"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output", default=DEFAULT_OUTPUT, help="catalogue Markdown output path"
    )
    parser.add_argument(
        "--emit-baselines",
        help="also write per-service contract baselines into this directory",
        metavar="DIR",
    )
    args = parser.parse_args(argv)

    per_service: dict[str, dict[str, list[Operation]]] = {}
    for service in SERVICES:
        spec = load_spec(service)
        per_service[service] = collect_parser_contract(spec)

    catalogue_md = render_catalogue(per_service)
    os.makedirs(os.path.dirname(args.output), exist_ok=True)
    with open(args.output, "w", encoding="utf-8") as f:
        f.write(catalogue_md)
    print(f"wrote {args.output}")

    if args.emit_baselines:
        os.makedirs(args.emit_baselines, exist_ok=True)
        for service, resources in per_service.items():
            baseline = baseline_for_service(resources)
            baseline_path = os.path.join(
                args.emit_baselines, f"{service}_input_contract.json"
            )
            with open(baseline_path, "w", encoding="utf-8") as f:
                json.dump(baseline, f, indent=2, sort_keys=True)
                f.write("\n")
            print(f"wrote {baseline_path}")

    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
