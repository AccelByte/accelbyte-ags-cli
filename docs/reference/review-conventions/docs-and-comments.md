# Docs & Comments — AGS CLI Review Conventions

Part of the [AGS CLI Review Conventions](../review-conventions.md) (RULE-28–RULE-29, 2 rules).

### RULE-28 — A new command or service update must touch three artefacts in the same change

**Rule:** When adding a new hand-written command, a new service, or any change that alters
`ags describe <service>` output, the same pull request must also update:

1. **CLAUDE.md module map** — add an entry for any new handler directory or module under
   `invocation/handlers/`.
2. **`docs/reference/cli-reference.md`** — update the service count and the auxiliary
   command listing if the surface grows.
3. **The `describe` envelope** — any new command reachable via `ags describe` must be
   representable by the strongly-typed `DescribeEnvelope` / `DescribeKind` types in
   `handlers/describe/envelope.rs`, not produce an ad-hoc JSON object.

A change that adds a command but omits these artefacts creates immediate docs drift; review
bots have flagged this class of omission in every PR that introduced a new auxiliary
command or updated the service manifest.

**Why:** `CLAUDE.md` is the first file the engineer reads for every task; stale module
entries mislead future work. `cli-reference.md` once hard-coded "24 services"; the count
drifted when the next service was bundled and the doc now delegates it to the manifest
(`manifest.rs`) — the fix for exactly this class. The `describe` envelope is the
machine-readable introspection surface; ad-hoc JSON objects break the schema-version
contract.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/invocation/handlers/describe/envelope.rs:21-43
// All describe output goes through typed structs that implement #[derive(Serialize)]:
#[derive(Serialize)]
pub struct DescribeEnvelope<T: Serialize> {
    pub schema_version: &'static str,
    pub kind: DescribeKind,      // ← add a new variant here for new entity types
    pub path: Vec<String>,
    pub generated_by: GeneratorInfo,
    pub data: T,
}
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DescribeKind {
    Catalogue, Command, Error, Workflow,
}
```

**Self-check:**
```
grep -rn "DescribeKind\|schema_version" crates/ --include="*.rs"
```
Any new `CommandOutput::Describe(…)` emitter must use one of the existing `DescribeKind`
variants or add a new typed variant — never build a raw `serde_json::json!({})` object
in the handler.

**Provenance:** `crates/accelbyte-ags-cli/src/invocation/handlers/describe/envelope.rs:1-43`; `CLAUDE.md:107-142`; recurring review finding class #14 (new-command doc completeness)

---

### RULE-29 — A comment asserting an invariant must be enforced by code or a test, or reworded

**Rule:** If a doc comment states that an invariant always holds (e.g. "DELETE always
requires confirmation", "the archive is never empty", "this path is unreachable"), the
invariant must be verifiable by: (a) a `debug_assert!` or `assert!` in the code, (b) a
unit or integration test that would catch a violation, or (c) a structural type guarantee
(e.g. a non-empty `Vec` represented as `(T, Vec<T>)`). Comments that claim invariants
without any enforcement become lies as the code evolves.

**Why:** Review findings have flagged comments whose claimed invariant was already
violated by code in the same PR. No reference doc states this rule explicitly.
`testing-reference.md §11.3` covers snapshot quality but not comment-driven invariants.

**House pattern:**
```rust
// crates/ags-runtime/src/runtime/dispatch/confirmation.rs:5-9 — comment IS backed by a test:
//
// DELETE always confirms; POST/PUT/PATCH confirm only when the operation name
// contains a risky keyword (…).
//
// Tests: test_delete_always_confirms, test_get_never_confirms, etc.
// (crates/ags-runtime/src/runtime/dispatch/confirmation.rs:33-100)
```

**Self-check:** For any new doc comment containing "always", "never", "invariant",
"guaranteed", or "unreachable": locate the test or assertion that would catch a violation.
If none exists, add one or reword the comment to describe intent rather than guarantee.

**Provenance:** `crates/ags-runtime/src/runtime/dispatch/confirmation.rs:5-9, 33-100`; recurring review finding class #12 (comment / code / test parity)

---
