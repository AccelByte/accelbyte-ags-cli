# Naming & Structure — AGS CLI Review Conventions

Part of the [AGS CLI Review Conventions](../review-conventions.md) (RULE-25, RULE-26, and RULE-39, 3 rules).

### RULE-25 — Sprint/planning labels must not appear in production source or test comments

**Rule:** Do not use sprint planning labels ("Task 6", "Task 5.2", "Phase 2 — Task 3") as
section headings or comment labels in shipped source or test code.
Workflow-domain vocabulary ("Phase 1: collect declared inputs") is product terminology
and is acceptable.

**Why:** No reference doc explicitly bans planning IDs. They are meaningful during the
sprint that produced them and become noise for future readers who have no context for
what "Task 6" meant. Distinguishing mark: if the "Phase N" label names a workflow engine
concept (a stage of runtime execution), it is product vocab; if it refers to a sprint
work item, it is a planning label.

**Known current violations:**
- `crates/accelbyte-ags-cli/tests/workflow_integration.rs:631` — `// ── Failure-recovery tests (Task 6) ──`
- `crates/accelbyte-ags-cli/src/frontend/terminal/form_runner.rs:431` — "Task 5" section heading
- `crates/accelbyte-ags-cli/src/frontend/terminal/inline/form.rs:3971, 4017` — "Task 10", "Task 11" headings

**Self-check:**
```
grep -rn "Task [0-9]" crates/ --include="*.rs"
```
All hits should be workflow-domain language (product stage labels), not sprint planning items.

**Provenance:** `tests/workflow_integration.rs:631`; `frontend/terminal/form_runner.rs:431`; `frontend/terminal/inline/form.rs:3971`

---

### RULE-26 — `mod.rs` files stay thin (re-exports and module declarations only); single-concern logic lives in a dedicated file

**Rule:** When adding a new module concern, create a dedicated file rather than adding
logic to an existing `mod.rs`. A `mod.rs` that grows beyond ~100 lines of logic (excluding
re-exports and `mod` declarations) is a candidate violation.

Before creating any new utility function in a `mod.rs`, apply RULE-36 (check for an existing
helper in `support/`, `style/`, or `tests/common/`) first. If no existing helper applies,
the new function belongs in a dedicated file, not in `mod.rs`.

**Why:** `CONTRIBUTING.md §Architecture` says "each module has a single responsibility"
but does not state the file-structure rule. A fattened `mod.rs` hides independent concerns
inside a module namespace and makes it harder to find, test, or replace a single subsystem.

**Reference point:** `frontend/mod.rs` is the largest `mod.rs` in the codebase at ~860
lines, but it is justified — it owns the cross-cutting `Frontend` trait, the
`ExecutionPhaseSurfaces` type, and the three sink helpers. Single-concern modules all have
dedicated files: `ansi.rs`, `tone.rs`, `text.rs`, `span.rs`, `streams.rs`.

**House pattern:**
```
// Correct: each concern in its own file
crates/accelbyte-ags-cli/src/frontend/style/
    ansi.rs    ← ANSI escape encapsulation
    text.rs    ← symbol constants
    tone.rs    ← Tone enum
    span.rs    ← StyledSpan IR
    mod.rs     ← pub use re-exports only (~15 lines)
```

**Self-check:**
```
grep -c "." crates/accelbyte-ags-cli/src/frontend/mod.rs
grep -c "." crates/ags-runtime/src/runtime/*/mod.rs
```
Any `mod.rs` returning more than ~100 non-blank, non-comment lines of business logic
deserves review for extraction.

**Provenance:** `crates/accelbyte-ags-cli/src/frontend/style/` directory layout; `frontend/mod.rs` line count

---

### RULE-39 — Every `#[allow(dead_code)]` requires a justifying comment naming a concrete consumer or future integration point

**Rule:** Do not ship dead code silently. Any `#[allow(dead_code)]` annotation — whether
at item level or module level (`#![allow(dead_code)]`) — must be accompanied by a comment
that identifies at least one of:

- A concrete current consumer (even if that consumer is in the test layer): name the
  function or test that calls it.
- A concrete in-flight integration point: name the branch, PR, or handler that will call
  it when it lands.

A blanket module-level `#![allow(dead_code)]` is acceptable only when the module doc
explains why the suppression covers the whole module (e.g. the module is entirely
test-support, or the non-test-exercised variants must remain modelled for exhaustiveness).
Remove unused paths rather than silently suppressing the compiler warning when no
concrete consumer can be named.

**Why:** Unannounced dead code accumulates over time, is invisible to `cargo clippy` once
silenced, and increases the surface area reviewers must reason about. The `#[allow(dead_code)]`
attribute is a legitimate tool — but it must be traceable to a real use.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/invocation/shape.rs:1-10
//! Interaction-shape classification.
//!
//! `classify_shape` is the live runtime path — `routes/service` calls it to pick the
//! surface. The route only ever constructs the `Service`/`Workflow` `RouteKind`s and
//! their reachable `Shape`s, so the `Auth`/`Builtin` arms, the `AuthSp`/`Static`
//! shapes, and the `AuthSubcommand`/`OutputOnly` flags are exercised only by tests;
//! the module-level allowance keeps those still-modelled variants from warning rather
//! than scattering per-item attributes.
#![allow(dead_code)]

// Per-item example with named consumer:
// crates/accelbyte-ags-cli/src/frontend/terminal/views/nav.rs:9
#[allow(dead_code)] // ConfirmCard used by the inline surface in a subsequent task
```

**Self-check:**
```
grep -rn "#\[allow(dead_code)\]\|#!\[allow(dead_code)\]" crates/accelbyte-ags-cli/src/ --include="*.rs"
```
Every hit must have an adjacent comment (preceding line or inline) naming a concrete
consumer or future integration point. A bare `#[allow(dead_code)]` with no explanation
is a violation. If no concrete consumer can be named, remove the unused item.

**Provenance:** `crates/accelbyte-ags-cli/src/invocation/shape.rs:1-10`; `crates/accelbyte-ags-cli/src/frontend/terminal/views/nav.rs:9`; recurring review finding class #17 (dead code / unused path)

---
