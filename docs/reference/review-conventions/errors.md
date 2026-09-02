# Errors — AGS CLI Review Conventions

Part of the [AGS CLI Review Conventions](../review-conventions.md) (RULE-08–RULE-11, 4 rules).

### RULE-08 — New error paths classify into one of five `CliError` variants; map to the correct variant for the actual failure

**Rule:** Every error returned from a command handler must be one of the five `CliError`
variants: `Usage` (exit 1), `Auth` (exit 2), `Api` (exit 3), `Network` (exit 4),
`Internal` (exit 5). Do not add a sixth variant or return a raw string. **Map failures to
the correct variant for the actual failure type** — a network timeout is `Network`, not
`Internal`; a missing required flag is `Usage`, not `Api`. Never silently swallow a
persistence failure: a failed config write must surface as a `CliError`, not be discarded
with `let _ =`.

**Why:** `testing-reference.md §5.4` requires asserting exit codes in tests, but the
five-variant → exit-code matrix is not documented anywhere. A new variant would silently
break exit-code contracts for any script that branches on `$?`. Mismapped variants produce
wrong exit codes, which break CI pipelines that branch on `$?` semantically (e.g.
distinguishing auth failure from network failure). `Internal` drops `ErrorMetadata` on
conversion from `RuntimeError` (see `errors.rs:156-163`); attach context in the anyhow
chain, not in metadata.

**Variant selection guide:**

| Situation | Correct variant | Exit code |
|-----------|----------------|-----------|
| Missing flag, invalid value, wrong arg count, unknown key | `Usage` | 1 |
| Missing/expired credentials, permission denied | `Auth` | 2 |
| HTTP 4xx/5xx from an AccelByte service endpoint | `Api` | 3 |
| DNS failure, TCP reset, TLS error, connection timeout | `Network` | 4 |
| Unhandled invariant violation (a bug), failed file write | `Internal` | 5 |

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/errors.rs:7-74
pub enum CliError {
    Usage   { message: String, metadata: Option<Box<ErrorMetadata>> }, // exit 1
    Auth    { message: String, metadata: Option<Box<ErrorMetadata>> }, // exit 2
    Api     { message: String, metadata: Option<Box<ErrorMetadata>> }, // exit 3
    Network { message: String, metadata: Option<Box<ErrorMetadata>> }, // exit 4
    Internal(anyhow::Error),                                           // exit 5
}
```

**Self-check:**
```
cargo test -- errors::tests
```
Also scan any new error-returning function: confirm the variant matches the failure domain,
and that no `let _ = some_write_result` discards a persistence error.

**Provenance:** `crates/accelbyte-ags-cli/src/errors.rs:7-74`; recurring review finding class #10 (error classification)

---

### RULE-09 — Enrich errors with `ErrorMetadata` using one of two established patterns; never hand-roll multi-line strings

**Rule:** To add user-facing context to a `CliError`, attach `Box<ErrorMetadata>` via
either the suggestion-only constructor (`ErrorMetadata::with_suggestion("…")`) or the full
struct literal (`ErrorMetadata { reason, detail, suggestion, suggestion_kind, tip, .. }`).
Do not build ad-hoc multi-line error strings that embed `Reason:` / `Fix:` formatting
manually.

**Why:** The rendering of `reason`, `detail`, and `suggestion` is owned by the frontend.
Hand-rolled strings with embedded `Reason: …` labels duplicate rendering logic, break JSON
output (where these fields are separate keys), and drift from the `output-reference.md
§8–§9` layout rules.

**House pattern:**
```rust
// crates/ags-protocol/src/error.rs:90-101
impl ErrorMetadata {
    /// Pattern A — suggestion only (most common):
    pub fn with_suggestion(suggestion: impl Into<String>) -> Self {
        Self {
            suggestion: Some(suggestion.into()),
            suggestion_kind: SuggestionKind::Fix,
            ..Default::default()
        }
    }
}

// Pattern B — full context (when reason/detail are also known):
metadata: Some(Box::new(ErrorMetadata {
    reason: Some("Access token expired.".into()),
    detail: Some("HTTP 401 Unauthorized.".into()),
    suggestion: Some("Run 'ags auth login' to refresh credentials.".into()),
    suggestion_kind: SuggestionKind::Fix,
    tip: None,
    ..Default::default()
}))
```

**Self-check:**
```
grep -rn "ErrorMetadata" crates/ --include="*.rs" -l
```
Scan each file: every construction should use `with_suggestion(…)` or a struct literal
with `..Default::default()`.

**Provenance:** `crates/ags-protocol/src/error.rs:90-101`; `crates/accelbyte-ags-cli/src/errors.rs:3, 13`

---

### RULE-10 — Use `SuggestionKind::Fix` for corrective action, `SuggestionKind::Next` for advisory follow-up

**Rule:** Set `ErrorMetadata::suggestion_kind` to `SuggestionKind::Fix` when the
suggestion directly fixes the error (renders as `→ Fix:`). Set it to
`SuggestionKind::Next` for optional advisory follow-up steps (renders as `→ Next:`).
`Fix` is the default.

**Why:** `output-reference.md §15` documents the visual labels `Fix:` and `Next:` and
`§4.1` says "`Fix` is a rendering role"; it does not name the Rust type `SuggestionKind`
or describe when to choose each value. Without the type, new authors produce all
suggestions as `Fix:` even when `Next:` is appropriate (e.g. warnings that completed
successfully).

**House pattern:**
```rust
// crates/ags-protocol/src/error.rs:61-69
pub enum SuggestionKind {
    /// "Fix:" — the suggestion directly resolves the error.
    #[default]
    Fix,
    /// "Next:" — a next-step hint rather than a direct fix.
    Next,
}
```

**Self-check:**
```
grep -rn "SuggestionKind" crates/ --include="*.rs"
```
Review each usage: errors that completed (warnings, degraded-but-succeeded paths) should
prefer `Next`; errors that require corrective user action should use `Fix`.

**Provenance:** `crates/ags-protocol/src/error.rs:61-69`

---

### RULE-11 — `panic!` and `.unwrap()` are banned on user-reachable production paths; use `CliError::Internal`

**Rule:** Do not call `panic!` or `.unwrap()` in production code paths that a user can
trigger. Unexpected invariant violations must be returned as
`CliError::Internal(anyhow::anyhow!("…"))` or `RuntimeError::internal("…")` from the
runtime.

**Why:** `CONTRIBUTING.md` says "Handle errors at the appropriate level" but does not
prohibit `panic!`/`.unwrap()` explicitly. A panic in a CLI command produces a Rust
backtrace rather than a structured error, leaks internal detail to the user, exits without
the correct exit code, and cannot be caught by the frontend's JSON error envelope.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/errors.rs:36-38
// Correct pattern for an invariant violation:
return Err(CliError::Internal(anyhow::anyhow!(
    "expected at least one step in compiled workflow"
)));
```

**Self-check:**
```
grep -rn "\.unwrap()\|panic!" crates/ags-runtime/src/ crates/accelbyte-ags-cli/src/ --include="*.rs"
```
Every hit must be: (a) inside a `#[cfg(test)]` block — the runtime crate's hits are
almost entirely its in-file test modules; (b) a compile-time invariant assertion in
`build.rs`; or (c) an invariant-guarded accessor unwrap immediately preceded by the
coercion that makes it infallible (two such sites in `runtime/workflows/resolve.rs` at
the time of writing). Any other hit on a runtime path reachable by user input is a
violation.

**Provenance:** `crates/accelbyte-ags-cli/src/errors.rs:36-38`

---
