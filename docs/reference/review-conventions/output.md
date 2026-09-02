# Output — AGS CLI Review Conventions

Part of the [AGS CLI Review Conventions](../review-conventions.md) (RULE-01–RULE-07, 7 rules).

### RULE-01 — Runtime crate is IO-free: architecture test enforces the banned-macro list

**Rule:** Never call `print!`, `println!`, `eprint!`, or `eprintln!` in any `.rs` file
under `crates/ags-runtime/src/` (runtime, catalogue, support). An architecture test
enforces this automatically; a violation causes `cargo test --test architecture` to fail.

**Why:** The runtime is a library crate. User-visible output is a presentation concern
owned by `frontend/`. Any direct print in the runtime bypasses VT-mode setup,
stdout/stderr flushing discipline, and the `--output` file sink.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/tests/architecture.rs:15-21
const BANNED_SUBSTRINGS: &[&str] = &[
    "print!(",
    "println!(",
    "eprint!(",
    "eprintln!(",
    "indicatif::",
];
const SCANNED_DIRS: &[&str] = &[
    "../ags-runtime/src/runtime",
    "../ags-runtime/src/catalogue",
    "../ags-runtime/src/support",
];
```

**Self-check:**
```
cargo test --test architecture
```
Green = no violations in any scanned runtime directory.

**Provenance:** `crates/accelbyte-ags-cli/tests/architecture.rs:15-32`

---

### RULE-02 — CLI binary I/O routes through the three sink helpers, never bare `println!`/`eprintln!`

**Rule:** All user-visible writes in the `accelbyte-ags-cli` binary must route through
`write_stdout_line`, `write_stderr_line`, or `write_stderr` from
`crates/accelbyte-ags-cli/src/frontend/mod.rs`. Call bare `println!` / `eprintln!` there
only in comments or test scaffolding.

**Why:** The three helpers do three things bare macros cannot: (1) `write_stdout_line`
calls `anstream::stdout()` so Windows VT-mode is enabled before the first escape code;
(2) all three handle broken-pipe silently rather than panicking; (3) `write_stderr_line` /
`write_stderr` route through `UiSink` for uniform flushing. A bare `println!` also bypasses
the `--output <path>` sink — use `emit_with_options` for result data that may be
file-redirected.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/frontend/mod.rs:508-522
pub(crate) fn write_stdout_line(text: &str) -> Result<(), crate::errors::CliError> {
    write_stdout_line_into(anstream::stdout().lock(), text)
}
pub(crate) fn write_stderr_line(text: &str) {
    let _ = crate::frontend::streams::UiSink.write_line(text);
}
pub(crate) fn write_stderr(text: &str) {
    write_stderr_into(anstream::stderr().lock(), text);
}
```

**Self-check:**
```
grep -rn "println!\|eprintln!" crates/accelbyte-ags-cli/src/ --include="*.rs"
```
Expected: zero hits outside comments and test scaffolding.

**Provenance:** `crates/accelbyte-ags-cli/src/frontend/mod.rs:508-522`

---

### RULE-03 — UI chrome goes to stderr via `UiSink`; result data goes to stdout via `emit_with_options`

**Rule:** Spinners, prompts, banners, hint lines, and in-progress status messages are
written to **stderr** through `UiSink`. Final command results (success body, JSON envelope,
inspect output) are written to **stdout** through `emit_with_options` (or
`Frontend::render` / `Frontend::render_error`). The two channels must never be swapped.

**Why:** Stdout is the machine-readable channel. Mixing UI chrome into stdout breaks
`| jq`, `--output <file>`, and any script that consumes the result. The `streams.rs`
module doc captures the design intent explicitly: `UiSink` covers stderr-only chrome;
result emission is intentionally out of scope and routes through `emit_with_options`
so `--output` is honoured.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/frontend/streams.rs:1-7 (module doc)
//! Stderr UI-chrome sink.
//!
//! Stdout / result emission is NOT covered by this module — results must
//! always route through `emit_with_options` so that `--output <path>` is
//! honoured and the JSON frontend can intercept result data on its own path.
```

**Self-check:**
```
grep -rn "UiSink" crates/accelbyte-ags-cli/src/ --include="*.rs"
```
All `UiSink` call-sites should be non-result writes (status, hints, prompts). Verify
separately that `emit_with_options` is the only function that writes result data to
stdout in production paths.

**Provenance:** `crates/accelbyte-ags-cli/src/frontend/streams.rs:1-7`; `frontend/mod.rs:50-72`

---

### RULE-04 — All colour and emphasis routed through `ansi.rs`; no raw ANSI escapes outside that file

**Rule:** Colour, dim, bold, and semantic decoration are applied via helpers in
`crates/accelbyte-ags-cli/src/frontend/style/ansi.rs` (e.g. `ansi::green`, `ansi::dim`,
`apply_tone(&text, Tone::Error, color_enabled)`) or through the `StyledSpan` / `StyledLine`
IR. Raw ANSI escape sequences (`\x1b[31m`, etc.) must not appear in caller code outside
`ansi.rs` itself.

**Why:** Encapsulating escapes in one file means `--no-color` / `AGS_NO_COLOR` control is
applied uniformly. Callers that roll their own escapes bypass the `STDOUT_COLOR` /
`STDERR_COLOR` atomics that `ansi::init()` sets at startup and break no-colour terminals.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/frontend/style/ansi.rs:57-110
pub fn green(text: &str, enabled: bool) -> String {
    if enabled { format!("\x1b[32m{text}\x1b[0m") } else { text.to_string() }
}
pub fn dim(text: &str, enabled: bool) -> String {
    if enabled { format!("\x1b[2m{text}\x1b[0m") } else { text.to_string() }
}
// Tone-based (preferred for semantic meaning):
// apply_tone(&text, Tone::Error, ansi::is_stderr_enabled())
```

**Self-check:**
```
grep -rn "\\x1b\[" crates/accelbyte-ags-cli/src/ --include="*.rs"
```
Expected: hits only in `style/ansi.rs`. Any hit in another file is a violation.

**Provenance:** `crates/accelbyte-ags-cli/src/frontend/style/ansi.rs:57-110`

---

### RULE-05 — Standard symbols referenced via constants in `text.rs`, never hardcoded inline

**Rule:** The seven standard output symbols (`✔ ✕ — ! › ∘ →`) must be referenced via the
`pub const` values in `crates/accelbyte-ags-cli/src/frontend/style/text.rs`. Do not
hardcode the Unicode characters inline in caller code.

**Why:** `text.rs` is the single source of truth for the symbol set. A hardcoded `"✔"` in
caller code won't be found by a global symbol-replacement and drifts from the canonical
encoding. The semantic helpers in `ansi.rs` already consume these constants; callers that
build message strings manually should do the same. See also RULE-36 on reuse before writing.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/frontend/style/text.rs:4-10
pub const SYMBOL_SUCCESS: &str = "\u{2714}"; // ✔
pub const SYMBOL_ERROR:   &str = "\u{2715}"; // ✕
pub const SYMBOL_SKIPPED: &str = "\u{2014}"; // —
pub const SYMBOL_WARNING: &str = "!";
pub const SYMBOL_INFO:    &str = "\u{203a}"; // ›
pub const SYMBOL_STATUS:  &str = "\u{2218}"; // ∘
pub const SYMBOL_FIX:     &str = "\u{2192}"; // →
```

**Self-check:**
```
grep -rnE "[✔✕✖›∘→]" crates/accelbyte-ags-cli/src/ --include="*.rs" | grep -v "style/text.rs"
```
Expected: no hits that hardcode a symbol in a string literal (the class match is
intentionally broad; comment hits like the ones annotating the constants are fine).

**Provenance:** `crates/accelbyte-ags-cli/src/frontend/style/text.rs:4-10`

---

### RULE-06 — JSON mode is `ConsumerKind::Automation`; `JsonFrontend` is the only allowed stdout emitter on that path

**Rule:** When `--format json` is active, the resolved context is
`ConsumerKind::Automation`. This toggles `ctx.is_automation() → true` and
`ctx.allows_input() → false`, and routes the surface backend to `JsonFrontend`. On the
JSON path, `JsonFrontend` is the only frontend allowed to emit to stdout; no human chrome
may leak to stderr on success (see `output-reference.md §25.3`).

**Why:** `output-reference.md §3` documents that machine and human modes must be distinct,
but names neither `ConsumerKind` nor `is_automation()`. Without knowing the code mechanism,
a new author may add conditional output that bypasses the frontend factory and leaks chrome
onto the JSON path.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/invocation/context.rs:102-109
pub fn allows_input(&self) -> bool {
    self.interaction.allow_input
}
pub fn is_automation(&self) -> bool {
    matches!(self.consumer, ConsumerKind::Automation)
}
// Resolution (context.rs:411-412):
// allow_input = !is_json && !flags.is_no_input && terminal.allows_interactive_prompts()
```

**Self-check:**
```
cargo test --test integration -- format_precedence
```

**Provenance:** `crates/accelbyte-ags-cli/src/invocation/context.rs:102-109, 411-412`; `frontend/mod.rs:444-447`

---

### RULE-07 — `indicatif::` is banned from the runtime layer (same architecture test as RULE-01)

**Rule:** Do not import `indicatif::` in any file under `crates/ags-runtime/src/`. The
same `BANNED_SUBSTRINGS` / `SCANNED_DIRS` architecture test that catches `println!` also
catches `indicatif::`.

**Why:** Spinners are a presentation concern. The runtime communicates progress through
`ProgressEvent::Message` on the protocol event channel; the frontend owns the spinner. An
`indicatif` import in the runtime would couple the library layer to a terminal-rendering
crate.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/tests/architecture.rs:20
const BANNED_SUBSTRINGS: &[&str] = &[
    // ... (print!/println!/eprint!/eprintln! omitted for brevity)
    "indicatif::",
];
```

**Self-check:**
```
cargo test --test architecture
```

**Provenance:** `crates/accelbyte-ags-cli/tests/architecture.rs:20`

---
