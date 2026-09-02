# Consistency & Reuse — AGS CLI Review Conventions

Part of the [AGS CLI Review Conventions](../review-conventions.md) (RULE-32–RULE-38, 7 rules).

### RULE-32 — Base-URL identity comparisons must apply `.trim_end_matches('/')` consistently

**Rule:** Every code path that constructs a URL from a stored `base_url` value must strip
the trailing slash by calling `.trim_end_matches('/')` before concatenating. Do not use a
stored `base_url` directly in string formatting or comparison without normalisation; two
callers that disagree on trailing-slash presence can silently call different endpoints.

**Why:** No reference doc describes the base-URL normalisation rule. The codebase has at
least three independent call-sites that each do their own `trim_end_matches('/')` without
a shared helper. When a new HTTP call path is added that forgets this step, it silently
generates a double-slash URL (`https://example.io//iam/healthz`) that some servers reject.
Client-ID comparisons are case-sensitive as stored; do not lowercase them before
comparing — they are opaque strings and lowercasing may break case-sensitive client IDs.

**House pattern:**
```rust
// crates/ags-runtime/src/runtime/dispatch/execute.rs:407
let url = format!("{}{}", ctx.base_url.trim_end_matches('/'), path);

// crates/ags-runtime/src/runtime/auth/tokens.rs:24
let trimmed = base_url.trim_end_matches('/');

// crates/ags-runtime/src/runtime/diagnostics/checks.rs:547
let probe_url = format!("{}/iam/healthz", base_url.trim_end_matches('/'));
```

**Self-check:**
```
grep -rn "base_url" crates/ --include="*.rs"
```
Every `format!` or `format_args!` call that concatenates `base_url` with a path must
include `.trim_end_matches('/')`. Any bare `base_url` in a URL-building expression without
the trim is a violation.

**Provenance:** `crates/ags-runtime/src/runtime/dispatch/execute.rs:407`; `crates/ags-runtime/src/runtime/auth/tokens.rs:24`; `crates/ags-runtime/src/runtime/diagnostics/checks.rs:547`; recurring review finding class #9 (normalization / comparison consistency)

---

### RULE-33 — All persistent state writes use the atomic temp-then-persist pattern; open handles use RAII guards

**Rule:** Config files, token stores, and any other on-disk state must be written via
`write_file_restricted` from `crates/ags-runtime/src/support/file_system.rs`, not via
`fs::write`, `File::create`, or `OpenOptions::new().write(true).truncate(true)`. The
helper writes to a temporary file in the same directory, syncs to disk, then renames it
to the destination — making the write atomic and eliminating a TOCTOU window where a
reader could see a partially-written file.

For cross-process coordination on shared files, acquire a `FileLock` RAII guard before
reading or writing the protected resource.

Open file handles, lock guards, and PTY pairs must be held only for the duration of the
I/O operation; RAII (via `Drop`) is the only safe cleanup mechanism — do not call
`.close()` or `.unlock()` manually.

**Why:** `CONTRIBUTING.md §Security` documents the permission bits (0700/0600) but not the
atomic-write requirement. A direct `fs::write` to the config file truncates the existing
content before the new content is ready, creating a window where a concurrent reader sees
an empty or partial file. Partial reads of a config file silently reset all settings to
defaults, which can cause cascading auth failures.

**House pattern:**
```rust
// crates/ags-runtime/src/support/file_system.rs:37-74
// write_file_restricted: temp file → sync_all → persist (rename)
pub(crate) fn write_file_restricted(path: &Path, data: &str) -> std::io::Result<()> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(TEMP_FILE_PREFIX);
    let mut tmp = builder.tempfile_in(dir)?;
    tmp.write_all(data.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| { … })?;
    Ok(())
}

// crates/ags-runtime/src/support/file_system.rs:76-80
// FileLock: RAII advisory lock released on drop
pub struct FileLock { _file: std::fs::File }
```

**Self-check:**
```
grep -rn "fs::write\|File::create\|truncate(true)" crates/ --include="*.rs"
```
Any hit in non-test production code that writes persistent state is a violation. Replace
with `write_file_restricted`.

**Provenance:** `crates/ags-runtime/src/support/file_system.rs:12-80`; recurring review finding class #8 (concurrency / TOCTOU)

---

### RULE-34 — Destructive and file-writing operations require explicit consent before executing

**Rule:** Any command handler that (a) calls an HTTP endpoint that modifies or deletes
server-side data, or (b) writes or overwrites a local file supplied by the user, must
check for explicit consent before proceeding:

- **API destructive operations** (DELETE, risky POST/PUT/PATCH): consent is resolved by
  `requires_confirmation(http_method, op_name)` in `dispatch/confirmation.rs`. When
  confirmation is required and `--yes` / `flags.is_auto_confirmed` is not set, the
  workflow engine prompts the user before dispatch.
- **Local file-writing operations** in auxiliary commands (e.g. cloning a template that
  writes files to the current directory): check `flags.is_auto_confirmed` or prompt via
  `ctx.allows_input()` before overwriting an existing file. Silently clobbering a file the
  user did not intend to overwrite is a footgun.

**Why:** `CONTRIBUTING.md` describes the confirmation rule for API calls indirectly via
the dispatch path, but does not state the rule for local file writes. The `--yes` flag
(`flags.is_auto_confirmed`) is the established bypass for both; new commands must honour it
rather than implementing a bespoke consent mechanism.

**House pattern:**
```rust
// Consent check for API-level destructive ops
// (dispatch layer; auxiliary commands inherit this automatically):
// crates/ags-runtime/src/runtime/dispatch/confirmation.rs:10
pub(crate) fn requires_confirmation(http_method: HttpMethod, op_name: &str) -> bool { … }

// --yes flag field (invocation/flags.rs:41):
pub is_auto_confirmed: bool, // set by "--yes" | "-y"

// Correct pattern before overwriting a local file:
if path.exists() && !flags.is_auto_confirmed {
    if !ctx.allows_input() {
        return Err(CliError::Usage {
            message: format!("'{}' already exists; pass --yes to overwrite", path.display()),
            metadata: None,
        });
    }
    // … interactive confirmation prompt …
}
```

**Self-check:**
```
grep -rn "is_auto_confirmed" crates/accelbyte-ags-cli/src/ --include="*.rs"
```
Any auxiliary command that writes local files must appear in this list or have an
equivalent guard. Any `fs::write` or `write_file_restricted` call that writes to a
user-supplied path without an existence check is a candidate violation.

**Provenance:** `crates/ags-runtime/src/runtime/dispatch/confirmation.rs:10`; `crates/accelbyte-ags-cli/src/invocation/flags.rs:41, 171`; `crates/accelbyte-ags-cli/src/invocation/routes/service/mod.rs:132`; recurring review finding class #11 (footgun / overwrite guard)

---

### RULE-35 — New JSON output types use `#[derive(Serialize)]` with `#[serde(rename_all = "snake_case")]`; field names are consistent across variants

**Rule:** New structured JSON payloads produced by command handlers must be represented
as Rust structs or enums with `#[derive(Serialize)]` and
`#[serde(rename_all = "snake_case")]`. Field names must be `snake_case` consistently
across all `CommandOutput` variants; do not introduce `camelCase` field names or manually
insert keys via `serde_json::Map` when a derive-based type would suffice.

**Why:** The `describe` envelope already demonstrates the derive-based pattern. Some older
renderers (e.g. `json/commands/auth.rs`) build `serde_json::Map` manually; this is an
established deviation for complex branching views, but it is not the preferred pattern for
new code. Mixed naming conventions (`base_url` in one variant, `baseUrl` in another)
break scripts that consume `--format json` output.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/invocation/handlers/describe/envelope.rs:21-43
// Preferred: derive-based, snake_case enforced at the type level:
#[derive(Serialize)]
pub struct DescribeEnvelope<T: Serialize> {
    pub schema_version: &'static str,
    pub kind: DescribeKind,
    pub data: T,
}
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DescribeKind { Catalogue, Command, Error, Workflow }

// Output-views types follow the same pattern with #[serde(skip)] for
// non-serialized fields:
// crates/ags-protocol/src/output_views.rs:66, 228-229
```

**Self-check:**
```
grep -rn "serde_json::Map::new\|serde_json::json!" crates/accelbyte-ags-cli/src/frontend/output/json/ --include="*.rs"
```
Audit each hit: if the payload has a fixed schema, replace with a derive-based type.
Retain `serde_json::json!` only for payloads with dynamic or sparse keys (e.g. the
dry-run request view that iterates over user-supplied query parameters).

**Provenance:** `crates/accelbyte-ags-cli/src/invocation/handlers/describe/envelope.rs:21-43`; `crates/ags-protocol/src/output_views.rs:66, 228-229`; recurring review finding class #15 (serde / JSON output consistency)

---

### RULE-36 — Search for an existing helper before writing a new one

**Rule:** Before writing a new helper function, search the established utility locations:
`crates/ags-runtime/src/support/` (string transforms, filesystem, output sink),
`crates/accelbyte-ags-cli/src/frontend/style/` (ANSI colour, symbols, tone),
and `crates/accelbyte-ags-cli/tests/common/` (shared test helpers). If an existing
function already covers the need, call it; do not write a duplicate.

Specific patterns that have been re-implemented in violation of this rule:

| Need | Existing function | Location |
|------|------------------|----------|
| Arrow prefix for fix/next-step hint lines | `ansi::fix_prefix()` → `"→"` | `frontend/style/ansi.rs:145` |
| CamelCase / snake_case → kebab-case | `strings::to_kebab_case(name)` | `support/strings.rs:52` |
| Percent-encode a URL path segment | `strings::encode_url_path_segment(value, param_name)` | `support/strings.rs:220` |
| Strip ANSI escape sequences from output | `strings::strip_terminal_control_sequences(value)` | `support/strings.rs:273` |
| Atomic config file write with 0600 perms | `write_file_restricted(path, data)` | `support/file_system.rs:39` |
| Isolated test command (no keychain, temp home) | `ags_isolated()` | `tests/common/cli_helpers.rs:17` |

**Why:** Duplicate helpers diverge over time: the copy doesn't inherit bug fixes or
behaviour updates applied to the original. The `fix_prefix` helper was re-implemented in
several review findings; the PR bot flagged the duplication each time.

**Self-check:**
```
grep -rn "fn fix_prefix\|fn to_kebab\|fn encode_url\|fn strip_terminal\|fn write_file_restricted\|fn ags_isolated" crates/ --include="*.rs"
```
If a new helper appears that duplicates one of the above, it is a violation. If the
existing helper almost fits but not quite, extend it rather than copying it.

**Provenance:** `crates/accelbyte-ags-cli/src/frontend/style/ansi.rs:145`; `crates/ags-runtime/src/support/strings.rs:52, 220, 273`; `crates/ags-runtime/src/support/file_system.rs:39`; `crates/accelbyte-ags-cli/tests/common/cli_helpers.rs:17`; recurring review finding class #6 (duplication)

---

### RULE-37 — Do not reload `GlobalConfig` in the same invocation chain after `apply_config_defaults` has already run

**Rule:** `GlobalConfig::load()` issues a filesystem read on every call. The canonical
and only permitted load within the flag-resolution invocation chain is the one in
`crates/accelbyte-ags-cli/src/invocation/flags.rs:337` (inside `apply_config_defaults`).
Any code that runs after flag resolution must consume the already-resolved flags, not call
`GlobalConfig::load()` again in the same request path.

**Legitimate independent loads** are those in separately dispatched commands that each
constitute their own invocation root:

- `crates/ags-runtime/src/runtime/facade/profile.rs:12, 123` — `ags profile` is its own
  dispatch context.
- `crates/ags-runtime/src/runtime/diagnostics/runner.rs:72` — `ags doctor` runs
  independently.
- `crates/ags-runtime/src/runtime/facade/diagnostics.rs:40` — same as above.
- `crates/accelbyte-ags-cli/src/invocation/first_run.rs:30` — startup probe that runs
  before `apply_config_defaults`.

A duplicate load within the **same request path** (e.g. a service-command handler that
already received resolved flags calling `GlobalConfig::load()` to re-read a value that
flags already captured) is a violation.

**Why:** No reference doc describes the load-once rule. The WEAK evidence in the original
gap register has since been confirmed by review findings on hot-path performance.
Re-parsing the config file on every request adds measurable latency for config-heavy
command sequences.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/invocation/flags.rs:337
// Canonical single load during flag resolution:
let config = match ags_runtime::runtime::config::GlobalConfig::load() {
    Ok(cfg) => cfg,
    Err(_)  => return,  // config errors surface as defaults; fatal errors are caught elsewhere
};
apply_config_defaults(&mut flags, &config);

// Correct downstream usage — consume the resolved flag, not a fresh load:
// flags.format      ← already resolved from config
// flags.timeout     ← already resolved from config
```

**Self-check:**
```
grep -rn "GlobalConfig::load()" crates/ --include="*.rs"
```
For every hit that is not in a listed legitimate load site, verify it runs in its own
dispatch root. Any `GlobalConfig::load()` that runs after `apply_config_defaults` on the
same request path is a violation.

**Provenance:** `crates/accelbyte-ags-cli/src/invocation/flags.rs:337`; `crates/ags-runtime/src/runtime/facade/profile.rs:12, 123`; `crates/ags-runtime/src/runtime/diagnostics/runner.rs:72`; recurring review finding class #13 (performance / hot-path)

---

### RULE-38 — Reads from streams and collections must be bounded; open resources use RAII; every write flushes before returning

**Rule:** Three resource-lifecycle requirements apply to all new production code:

1. **Bounded reads:** When rendering a streamed or array-valued response for human output,
   cap the displayed count with a sentinel rather than buffering the entire collection
   into memory. Use `.iter().take(CAP)` and emit a `+N more` trailer when the collection
   exceeds the cap.

2. **Write-then-flush discipline:** Every write to stdout or stderr must be followed by an
   explicit `.flush()` call before the function returns. Do not rely on implicit flush at
   drop; broken-pipe and piped-output scenarios require flush to be the explicit
   final step so that partial writes surface as errors, not silent data loss.

3. **RAII-only lifecycle:** File handles, lock guards, PTY pairs, and terminal handles must
   be released by `Drop`, never by a manual `.close()` or `.unlock()` call. Acquire the
   handle immediately before the I/O operation; release it by letting the binding go out
   of scope.

**Why:** No reference doc states these rules. Collection rendering without a cap can
consume unbounded memory when a service endpoint returns thousands of items into a
human-display buffer. Writes without flush can silently lose the last bytes on piped
output. Manual close calls create cleanup races that RAII avoids.

**House pattern:**
```rust
// crates/accelbyte-ags-cli/src/frontend/output/human/commands/workflow.rs:127-134
// Bounded render: cap array display, emit trailer for overflow
const CAP: usize = 10;
for obj in arr.iter().take(CAP) {
    lines.push(format!("    {}", render_item_line(obj, fields)));
}
if arr.len() > CAP {
    lines.push(format!("    +{} more", arr.len() - CAP));
}

// crates/accelbyte-ags-cli/src/frontend/mod.rs:486
// Write-then-flush: explicit flush in the same expression, broken-pipe handled:
match writeln!(w, "{text}").and_then(|_| w.flush()) {
    Ok(()) => Ok(()),
    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
    Err(e) => Err(CliError::Usage { message: format!("Cannot write to stdout: {e}."), metadata: None }),
}
```

**Self-check:**
```
grep -rn "\.collect::<Vec" crates/accelbyte-ags-cli/src/frontend/ --include="*.rs"
```
Any `collect::<Vec<_>>()` on an iterator derived from a potentially-unbounded API response
should have a preceding `.take(N)` limit. Also check that every call to `writeln!` or
`write_all` is paired with `.flush()` in the same expression or immediately after.

**Provenance:** `crates/accelbyte-ags-cli/src/frontend/output/human/commands/workflow.rs:127-134`; `crates/accelbyte-ags-cli/src/frontend/mod.rs:486, 502`; recurring review finding class #20 (resource / memory / lifecycle)

---
