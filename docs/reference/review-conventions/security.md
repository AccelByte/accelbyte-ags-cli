# Security — AGS CLI Review Conventions

Part of the [AGS CLI Review Conventions](../review-conventions.md) (RULE-31, 1 rule).

### RULE-31 — Execution traces must redact the `Authorization` header and must never echo `client_secret`

**Rule:** Any code that captures or emits an execution trace (HTTP request/response
headers, workflow step trace, audit log line) must redact the value of the
`Authorization` header to `"Bearer <redacted>"`. The `client_secret` field must never
appear in any trace, log line, or structured output — not even as a placeholder. The
`base_url` and `client_id` are safe to include.

**Why:** `CONTRIBUTING.md §Security` documents the path-parameter and display-output
sanitization rules but does not explicitly require credential redaction in traces. The
codebase already redacts `Authorization` in two places; this rule makes the pattern
explicit and mandatory for all future trace code.

**House pattern:**
```rust
// crates/ags-runtime/src/runtime/facade/service.rs:131
// Production dry-run preview masks the auth token at construction time:
let headers = vec![("Authorization".to_string(), "Bearer <token>".to_string())];

// Expected serialized form asserted by round-trip tests (both inside #[cfg(test)]):
// crates/ags-protocol/src/result.rs:191  — fn test_dry_run_result_round_trip
// crates/ags-protocol/src/workflow.rs:1721 — fn test_step_dry_run_preview_serde_round_trip
```

**Self-check:**
```
grep -rn "Authorization" crates/ --include="*.rs"
```
Every `Authorization` header appearing in trace or output code must have its value set to
`"Bearer <redacted>"` or an equivalent constant — never the real token string.

```
grep -rn "client_secret" crates/ --include="*.rs"
```
The `client_secret` field must only appear in `credentials.rs` (resolution), `store.rs`
(keychain read/write), and `keys.rs` (alias lookup) — never in trace output or rendered
results.

**Provenance:** `crates/ags-runtime/src/runtime/facade/service.rs:131` (production masking site); `crates/ags-protocol/src/result.rs:191` and `crates/ags-protocol/src/workflow.rs:1721` (round-trip test assertions inside `#[cfg(test)]`); `crates/ags-runtime/src/runtime/auth/credentials.rs:1-80`; recurring review finding class #7 (security: traversal / redaction / creds)

---
