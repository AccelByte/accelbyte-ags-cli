# TTY & Interactivity — AGS CLI Review Conventions

Part of the [AGS CLI Review Conventions](../review-conventions.md) (RULE-15–RULE-16, 2 rules).

### RULE-15 — Gate every interactive prompt behind `ctx.allows_input()`; honour the full automation-contract flag set

**Rule:** Before presenting any interactive prompt or confirmation, check
`ctx.allows_input()`. If it returns `false`, return a `CliError::Usage` immediately.
Never call `ctx.is_automation()` alone as the prompt gate — `allows_input()` is the
complete check (it already incorporates the automation flag, `--no-input`, and TTY state).

In addition, every command handler must honour the full automation-contract flag set:

- **`--format json`** (`is_automation()` true): suppress all chrome; `JsonFrontend` is the
  only emitter (RULE-06 covers the output side).
- **`--dry-run`** (`flags.is_dry_run` true): execute no network calls or side effects;
  return the `CommandOutput::DryRun` envelope.
- **`--yes` / `-y`** (`flags.is_auto_confirmed` true): skip confirmation prompts for
  destructive operations without user interaction (see also RULE-34 for the write side).
- **`--no-input`** (included in `allows_input()` resolution): behave as if stdin is
  unavailable; do not block on user input.
- **`--output <path>`** (honoured by `emit_with_options`): route result data to the file
  rather than stdout; do not write result bytes via `println!`.

**Why:** No reference doc names `allows_input()` or `is_automation()` as the gatekeeping
API, nor lists the flag set a handler must honour. Without this rule, a new command could
prompt under `--format json` (automation), ignore `--dry-run`, or silently write result
bytes around `--output`.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/invocation/context.rs:102-109
pub fn allows_input(&self) -> bool {
    self.interaction.allow_input
    // allow_input = !is_json && !flags.is_no_input && terminal.allows_interactive_prompts()
}
pub fn is_automation(&self) -> bool {
    matches!(self.consumer, ConsumerKind::Automation)
}

// Before any interactive prompt:
if !ctx.allows_input() {
    return Err(CliError::Usage { message: "…".into(), metadata: None });
}

// Automation-contract flag fields (invocation/flags.rs:41-46):
// flags.is_dry_run       ← set by --dry-run
// flags.is_auto_confirmed ← set by --yes / -y
```

**Self-check:**
```
grep -rn "allows_input\|is_automation" crates/accelbyte-ags-cli/src/ --include="*.rs"
```
Every interactive prompt call-site must be preceded by an `allows_input()` check. Every
new command handler must handle `is_dry_run` and `is_auto_confirmed` before any
network call or destructive file operation.

**Provenance:** `crates/accelbyte-ags-cli/src/invocation/context.rs:102-109, 411-412`; `invocation/flags.rs:41-46, 171, 220`; recurring review finding class #4 (non-interactive / automation contract)

---

### RULE-16 — `allows_interactive_prompts()` requires **both** stdin **and** stderr to be TTYs

**Rule:** The interactive-prompt capability check requires both `stdin_is_tty &&
stderr_is_tty`. Do not substitute a stdin-only TTY check — prompts render on stderr, so a
piped stderr with a TTY stdin cannot run the prompt.

**Why:** `CONTRIBUTING.md §Terminal input (TUI)` covers key-press filtering for Windows but
does not state the two-channel TTY requirement. A stdout-piped session (`ags … | jq`) has
`stdout_is_tty = false` but `stdin_is_tty = true` and `stderr_is_tty = true` — prompts
should still work. A session with stderr redirected to a file cannot render a prompt.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/invocation/context.rs:69-71
pub fn allows_interactive_prompts(&self) -> bool {
    self.stdin_is_tty && self.stderr_is_tty   // stdout may be piped
}
```

**Self-check:**
```
cargo test -- tests::test_allows_interactive_prompts_requires_stdin_and_stderr_both_tty
```
(In `crates/accelbyte-ags-cli/src/invocation/context.rs`.)

**Provenance:** `crates/accelbyte-ags-cli/src/invocation/context.rs:69-71`

---
