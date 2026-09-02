# Testing — AGS CLI Review Conventions

Part of the [AGS CLI Review Conventions](../review-conventions.md) (RULE-19–RULE-24 and RULE-27, 7 rules; RULE-25/RULE-26 are filed under Naming & Structure).

### RULE-19 — Env-mutating tests must be annotated `#[serial_test::serial]`

**Rule:** Any test that calls `std::env::set_var` or `std::env::remove_var` must be
annotated `#[serial_test::serial]` to prevent concurrent execution with other
env-mutating tests in the same process.

**Why:** `testing-reference.md §5.1` requires determinism and says tests should control
nondeterminism sources, but does not name `serial_test::serial` as the mechanism. Env-var
state is process-global; two env-mutating tests running in parallel race on the shared
env, causing silent non-deterministic failures.

**House pattern:**
```rust
// crates/ags-runtime/src/runtime/config/store.rs:529-531
#[test]
#[serial_test::serial]
fn test_config_dir_uses_env_override() {
    // … env mutation here …
}
```

**Self-check:**
```
grep -rn "serial_test::serial" crates/ --include="*.rs"
grep -rn "set_var\|remove_var" crates/ --include="*.rs"
```
Every `set_var`/`remove_var` in test code must appear in a function that also has
`#[serial_test::serial]`.

**Provenance:** `crates/ags-runtime/src/runtime/config/store.rs:530`; `tests/integration/token_refresh_race.rs:33`

---

### RULE-20 — Env-mutating tests must wrap each mutation in a `TempEnvGuard` RAII guard

**Rule:** Each `std::env::set_var` / `std::env::remove_var` call in test code must be
paired with a `TempEnvGuard` that restores the prior value on drop — even if the test
panics. Do not rely on manual cleanup in a teardown block.

**Why:** No reference doc names `TempEnvGuard`. A test that sets an env var and fails
mid-body leaves the variable dirty for the next test in the process, causing silent
contamination even when `#[serial_test::serial]` prevents concurrent races.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/tests/common/env_guard.rs:8-42
pub struct TempEnvGuard { key: &'static str, original: Option<String> }
impl TempEnvGuard {
    pub fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, original }
    }
}
impl Drop for TempEnvGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(val) => std::env::set_var(self.key, val),
            None      => std::env::remove_var(self.key),
        }
    }
}

// Usage:
#[test]
#[serial_test::serial]
fn test_ags_home_guard_restores_env_on_drop() {
    let _guard = TempEnvGuard::set("AGS_HOME", "/tmp/test-home");
    // guard restores AGS_HOME on drop, even on panic
}
```

**Note:** `tests/common/env_guard.rs` is available to all external test crates. A local
copy of the same pattern exists in `crates/ags-runtime/src/runtime/config/store.rs:493-521`
for crate-internal `#[cfg(test)]` modules that cannot import from `tests/common`.

**Self-check:**
```
grep -rn "set_var\|remove_var" crates/ --include="*.rs"
```
Every bare `std::env::set_var` in test code should be preceded by a `TempEnvGuard::set(…)`
assignment on the same variable.

**Provenance:** `crates/accelbyte-ags-cli/tests/common/env_guard.rs:1-42`

---

### RULE-21 — New functional test files must be registered in `tests/functional.rs` via `#[path = …]`

**Rule:** Every `.rs` file added under
`crates/accelbyte-ags-cli/tests/functional/` must have a corresponding
`#[path = "functional/<name>.rs"] mod <name>;` declaration in
`crates/accelbyte-ags-cli/tests/functional.rs`. Without it,
`cargo test --test functional` silently skips every test in that file — no error, no
warning.

**Why:** `testing-reference.md §20.3` mentions `#[path]` for workflow dry-run tests but
does not state the general rule for all functional test files.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/tests/functional.rs (excerpt)
#[path = "functional/competitive_multiplayer.rs"]
mod competitive_multiplayer;
#[path = "functional/config/mod.rs"]
mod config;
// ← add new entries here in alphabetical order
```

**Self-check:**
```
grep "#\[path" crates/accelbyte-ags-cli/tests/functional.rs
```
Any file under `tests/functional/` that lacks a matching `#[path]` entry is silently
invisible to the test harness.

**Provenance:** `crates/accelbyte-ags-cli/tests/functional.rs:1-39`

---

### RULE-22 — New integration test files must be registered in `tests/integration.rs` via `#[path = …]`

**Rule:** Same pattern as RULE-21 but for integration tests. Every `.rs` file added under
`crates/accelbyte-ags-cli/tests/integration/` must have a
`#[path = "integration/<name>.rs"] mod <name>;` declaration in `tests/integration.rs`.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/tests/integration.rs (excerpt)
#[path = "integration/format_precedence.rs"]
mod format_precedence;
#[path = "integration/tui_e2e.rs"]
mod tui_e2e;
// ← add new entries here in alphabetical order
```

**Self-check:**
```
grep "#\[path" crates/accelbyte-ags-cli/tests/integration.rs
```

**Provenance:** `crates/accelbyte-ags-cli/tests/integration.rs:1-41`

---

### RULE-23 — TUI/interactive-surface tests use `portable_pty::native_pty_system()` and must be `#[ignore]`

**Rule:** Tests that exercise TUI surfaces (`InlineFrontend`, `FullscreenFrontend`, or any
path that reads raw keystrokes) must: (1) open a real PTY via
`portable_pty::native_pty_system()`, and (2) carry
`#[ignore = "Requires a real PTY; run with --ignored"]` so CI runners that lack a real
TTY skip them automatically.

**Why:** `testing-reference.md §8.4` lists "interactive vs non-interactive behavior" as a
required test axis but does not specify PTY or the `portable_pty` crate. CI environments
typically have no real TTY; a TUI test that omits the `#[ignore]` will panic or hang on
CI.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/tests/integration/tui_e2e.rs:15,49
use portable_pty::{native_pty_system, PtySize};

#[test]
#[ignore = "Requires a real PTY; run with --ignored"]
fn test_tui_inline_form_submit() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 30, cols: 100, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");
    // … spawn ags into PTY, drive keystrokes, assert output …
}
```

**To run locally:**
```
cargo test -p accelbyte-ags-cli --test integration tui_e2e -- --ignored
```

**Self-check:**
```
grep -rn "native_pty_system\|portable_pty" crates/ --include="*.rs"
```
All TUI live-terminal tests should appear in this list.

**Provenance:** `crates/accelbyte-ags-cli/tests/integration/tui_e2e.rs:15,49`

---

### RULE-24 — Tests that touch config, auth, or profiles must use `ags_isolated()`, not bare `ags()`

**Rule:** Use `ags_isolated()` (not `ags()`) as the base command builder for any
functional or integration test that exercises config files, auth tokens, profile commands,
or the keychain. `ags_isolated()` sets `AGS_NO_KEYCHAIN=1` and a unique `AGS_HOME` per
call to prevent state bleed between tests.

**Why:** `testing-reference.md §5.2` says "use controlled dependencies" but does not name
`ags_isolated()`. A test that uses bare `ags()` may read the developer's real config or
tokens, producing non-deterministic results and potentially contaminating real stored
credentials.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/tests/common/cli_helpers.rs:11-30
/// Command isolated from real credentials and config state.
/// Uses AGS_NO_KEYCHAIN=1 and a unique temp config directory per call
/// to prevent token/config bleed between tests.
pub fn ags_isolated() -> Command {
    let mut command = ags();
    let test_name = std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .replace("::", "--");
    let unique_dir = std::env::temp_dir()
        .join(format!("ags-test-{}", std::process::id()))
        .join(test_name);
    command
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_HOME", unique_dir);
    command
}
```

**Self-check:**
```
grep -rn "ags()" crates/accelbyte-ags-cli/tests/ --include="*.rs"
```
Any bare `ags()` call in a test that exercises auth, config, or profile commands should
be replaced with `ags_isolated()` or `ags_with_base_url(…)` (which calls `ags_isolated()`
internally).

**Provenance:** `crates/accelbyte-ags-cli/tests/common/cli_helpers.rs:11-30`

---

### RULE-27 — Assertion sets on growing collections use `> 0` guards, not exact counts

**Rule:** When asserting that a scanned set is non-empty (architecture tests, spec
validators), use `assert!(count > 0, "…")` rather than `assert_eq!(count, N)`. An
exact-count assertion on a set that grows with new services or commands becomes a
change-detector that fails on every legitimate addition.

**Why:** No reference doc states this rule. `testing-reference.md §11.3` says "Snapshot
approval MUST NOT be treated as proof of quality" (adjacent but different). The
architecture test demonstrates the correct pattern: it guards against silent pass-through
(zero files scanned) without asserting an exact file count.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/tests/architecture.rs:53-55
assert!(
    files_scanned > 0,
    "no .rs files found under {SCANNED_DIRS:?} — the architecture guard would silently pass"
);
// Not: assert_eq!(files_scanned, 147);
```

**Self-check:**
```
grep -rnE "assert_eq!\([^)]*\.(len|count)\(\)" crates/accelbyte-ags-cli/tests/ --include="*.rs"
```
Most `assert_eq!` in tests is correct (exact strings, exit codes, field values) — this
check scopes to length/count assertions. Review any hit where the expected number grows with new
services, commands, or spec files.

**Provenance:** `crates/accelbyte-ags-cli/tests/architecture.rs:53-55`

---
