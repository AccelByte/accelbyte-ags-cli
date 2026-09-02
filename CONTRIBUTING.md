# Contributing

This document is for developers contributing to AGS CLI. It covers local setup, project architecture, coding standards, testing requirements, and commit conventions.

If you only want to use the CLI, see [README.md](README.md).

## Before You Start

Before making code changes:

- Read [README.md](README.md) for the current user-facing scope and product status
- Read the relevant reference docs in `docs/reference/` for the behavior you plan to change
- Check whether related work is already in progress in the same area

## Prerequisites

- Git
- Rust stable toolchain (install via [rustup](https://rustup.rs/))
- On Linux: `libdbus` and `libsecret` dev packages for keyring support

## Local Setup

```bash
git clone https://github.com/AccelByte/accelbyte-ags-cli.git
cd accelbyte-ags-cli
cargo build
cargo run -- --help
```

## Common Commands

```bash
cargo build                  # Build the CLI
cargo run -- --help          # Show root help (all services)
cargo run -- iam --help      # Show IAM resources
cargo test                   # Run all tests
cargo clippy -- -D warnings  # Lint (required before commit)
cargo fmt --check            # Format check
cargo run -- refresh-specs iam   # Re-parse bundled spec for a single service
cargo run -- refresh-specs       # Re-parse every bundled spec
```

## Development Workflow

1. Read the relevant design docs before changing behavior or output.
2. Make the smallest coherent change you can.
3. Run formatting, linting, and the relevant tests.
4. Update docs, snapshots, or fixtures if the change intentionally affects them.
5. Use a Conventional Commit message.

### Continuous integration

Pushing a branch to Bitbucket triggers a GitLab pipeline
([`accelbyte/project-justice/ags-cli`](https://gitlab.com/accelbyte/project-justice/ags-cli))
that runs `cargo fmt --check`, `cargo clippy -- -D warnings` and the workspace test
suite on Linux, then reports the result back as a build status on the commit. A
ClamAV scan additionally runs on `main`.

The GitHub workflows in `.github/workflows/` remain the source of truth for the
cross-platform test matrix, `cargo-deny`, and release packaging.

### Regenerating the demo reel

The README GIF is produced from `demos/onboarding/onboarding.tape` using [VHS](https://github.com/charmbracelet/vhs) driven against a local mock server. To regenerate after tape or mock changes:

```
brew install vhs        # or see charmbracelet/vhs for other platforms
./demos/record.sh onboarding
```

This rebuilds the CLI in release mode, starts `demos/engine/mock-server.py` with the demo's routes on localhost:8765, drives the tape, and writes `demos/onboarding/onboarding.gif`. Commit the updated GIF alongside any tape or routes changes.

New demos are authored with the `vhs-demo` skill, which grounds the tape and mock in `ags describe`. Each demo lives in `demos/<name>/` (`<name>.tape` + `<name>.routes.json`) and records via `./demos/record.sh <name>`.

## Architecture

### Workspace layout

This repository is a Cargo workspace with four crates under `crates/`:

- `ags-protocol` — leaf crate containing the typed protocol contracts (request, result, event, error, output shapes, and the `catalogue` identifier/schema types). It may also hold pure **port traits** shared across crates — behavioural contracts with no logic that the runtime calls and the frontend implements (e.g. `WorkflowFrontend`), so neither side depends on the other. No `tokio`, `reqwest`, `clap`, or other CLI/HTTP deps. Backwards-compatible types only.
- `ags-runtime` — depends on `ags-protocol`. Contains all business logic: command execution, auth, config, diagnostics, dispatch, the OpenAPI catalogue, and the shared `support` utilities (`output_sink`, `file_system`, `strings`, etc.).
- `accelbyte-ags-cli` — depends on both. Produces the `ags` binary. Contains argv parsing (`invocation/`), rendering (`frontend/`), the top-level `CliError`, the `lib.rs`/`main.rs`, and all integration tests under `crates/accelbyte-ags-cli/tests/`.
- `extend-proxy-client` — standalone leaf crate; a Rust port of `extend-helper-cli`'s Go tunneling client (`modules/extend-proxy`/`pkg/client`), consumed directly by the CLI's `extend remote-debug connect` orchestration — see [extend-proxy-client status](#extend-proxy-client-status) below.

Dependency direction is `accelbyte-ags-cli → ags-runtime → ags-protocol`, with the CLI also allowed to depend on the standalone `extend-proxy-client` leaf crate. Reverse edges are forbidden and rejected by `cargo check --workspace`. When adding a new module, place it in the lowest crate that needs to expose it.

`cargo test --workspace` from the repo root runs everything across all four crates. The `ags` binary builds with `cargo build -p accelbyte-ags-cli` (or `cargo run -p accelbyte-ags-cli -- <args>`).

#### `extend-proxy-client` status

This crate is consumed by `accelbyte-ags-cli::invocation::handlers::extend::remote_debug::connect_once`, which builds the proxy agent and service forwarder used by `ags extend remote-debug connect`. The integration intentionally lives in the CLI invocation layer for consistency with the existing imperative Extend handlers; moving the orchestration into an `ags-runtime` facade remains follow-up architecture work.

- Dependency direction is `accelbyte-ags-cli → extend-proxy-client`. The proxy crate remains a leaf with no dependencies on `ags-protocol`, `ags-runtime`, or `accelbyte-ags-cli`.
- Its own conformance suites (`tests/go_testpeer.rs`, `tests/sidecar_conformance.rs`) are the primary proof of wire-compatibility with the real Go sidecar; they are `#[ignore]`d and not run in CI (require an externally-built Go test-peer binary or a Docker Compose harness). Run them manually before relying on protocol-level changes here — see `crates/extend-proxy-client/tests/README.md` for exact steps.
- `Config::sidecar_ws` is a plain `ws://` (not `wss://`) URL by design, matching the Go original: `extend-helper-cli`'s `internal/cmd/remote_debug.go:227` builds `ws://localhost:<port>/tunnel?gameNamespace=...` — the client always dials the sidecar over loopback in plaintext. The hop that actually leaves the machine is TLS, established separately by the sidecar itself (`internal/tunnel/tunnel.go:57` builds `wss://<host>/csm/v2/admin/namespaces/<ns>/tunnel?...`). So plaintext `ws://` never touches the network. This is why `detect-insecure-websocket` findings on this crate's `ws://` literals are suppressed with `// nosemgrep` rather than fixed — when the future `remote-debug` command wires up a real call site passing a `ws://localhost` URL, it will trip the same rule and can point back at this note.

### Architecture guardrails

Each module has a single responsibility. Cross-module imports must follow the allowed dependency directions.

| Module | Crate | Responsibility |
|--------|-------|----------------|
| `catalogue` | `ags-runtime` (types re-used from `ags-protocol`) | OpenAPI spec loading, x-operationId-driven parsing, and caching |
| `protocol` | `ags-protocol` | Boundary types shared across crates — request, result, event, error, and output envelopes |
| `invocation` | `accelbyte-ags-cli` | Turns user input into typed requests — argv parsing, flag extraction, command routing |
| `runtime` | `ags-runtime` | Owns execution — endpoint calls, auth, config, diagnostics, validation, workflow transitions |
| `frontend` | `accelbyte-ags-cli` | Owns all user-visible formatting — tables, inspect views, JSON output, progress, colour |
| `support` | `ags-runtime` | Small shared utilities — filesystem helpers, string transforms, TTY/time utilities |
| `proxy client` | `extend-proxy-client` | Standalone tunnel-agent protocol, session, and forwarding implementation consumed by `remote-debug connect` |

**Forbidden dependencies:**

Crate-level (enforced by `cargo check --workspace`):

- `ags-protocol` must not depend on any other workspace crate
- `ags-runtime` must not depend on `accelbyte-ags-cli`
- `accelbyte-ags-cli` is the only crate allowed to expose `frontend` or `invocation`
- `extend-proxy-client` must not depend on `ags-protocol`, `ags-runtime`, or `accelbyte-ags-cli`; `accelbyte-ags-cli` may consume it directly

Module-level within a crate:

- `runtime` must not depend on `frontend` or `invocation`
- `frontend` must not depend on `runtime`, `catalogue`, or `invocation`. It may use `support` utilities (`output_sink`, `strings`, TTY/time helpers). The workflow execution port (`WorkflowFrontend`, `WorkflowEvent`, `StepOutcome`, `RunOutcome`, `ResolvedOptions`) lives in `ags-protocol`, so the frontend implements the port without depending on `runtime`. **One sanctioned exception:** the production dynamic-enum resolver (`frontend/dynamic_options.rs::ProductionResolver`) calls `runtime::workflows::resolve_options` to run an interactive option fetch while animating a live spinner on the frontend surface. This is the single place the frontend bridges to `runtime`; it exists because the fetch and the surface spinner are inseparable, and it is kept behind the frontend-owned `DynamicOptionResolver` port
- `catalogue` may import `runtime::config` (path helpers) and `support` (filesystem/string helpers), but must not depend on `frontend` or `invocation`
- `support` must not depend on `runtime`, `catalogue`, `frontend`, or `invocation`

**Operational rules:**

- `runtime` must not print or prompt — all user-visible output flows as structured data through `frontend`
- `frontend` must not call endpoints, execute commands, or decide what action to take (the one sanctioned exception is the dynamic-enum option fetch noted above)
- `invocation` must not own business logic — it builds typed requests and lets `runtime` validate

**Required invariants:**

- Structured output is canonical; human-readable output is derived from it
- Workflows request input via state (`AwaitingConfirmation`), not stdin

## Key Files

| File | Purpose |
|------|---------|
| `scripts/generate_cli_command_catalogue.py` | **Canonical Python reference** for the command catalogue. Rust must match its output exactly |
| `crates/ags-runtime/src/runtime/dispatch/error_codes/` | AccelByte error code lookup tables (one file per service) |
| `crates/ags-runtime/src/runtime/dispatch/classify.rs` | Error classification pipeline: error code → HTTP status → user-friendly message |
| `crates/ags-runtime/specs/*.json.gz` | Every gzip-compressed OpenAPI 2.0 spec bundled into the binary via `include_bytes!` — one per service in the manifest |
| `crates/accelbyte-ags-cli/tests/fixtures/baselines/<service>_input_contract.json` | Per-service Python-generated reference data for parser validation |

## Reference docs

All CLI behaviour is governed by the normative reference docs in `docs/reference/`. Each reference defines requirements (MUST / SHOULD / MAY); the code itself is the implementation. Read the relevant reference before modifying behaviour it governs:

| Document | Purpose |
|----------|---------|
| `docs/reference/cli-reference.md` | Normative requirements (MUST/SHOULD/MAY) for CLI behaviour |
| `docs/reference/output-reference.md` | Output formatting, tone, hierarchy, and rendering rules |
| `docs/reference/testing-reference.md` | Testing strategy, test categories, and validation requirements |
| `docs/reference/cli-command-catalogue.md` | Full command catalogue for every bundled service (auto-generated) |
| `docs/reference/ams-upload.md` | `ags ams upload`: IAM client setup, minimum permissions, troubleshooting, and migration from the standalone `ams` CLI |
| `docs/reference/review-conventions.md` | Review conventions: the RULE-NN rule set enforced in code review, with per-area pages and runnable self-checks |

The current module layout, dependency directions, and operational invariants are captured in [Architecture guardrails](#architecture-guardrails) above.

When writing or modifying help text and error messages, read `docs/reference/output-reference.md` first. It defines the tone, hierarchy, symbol usage, and formatting rules that all user-facing text must follow.

Also follow the CLI design guidelines at https://clig.dev for general principles.

## Commits

Follow [Conventional Commits](https://www.conventionalcommits.org/) with semver:

```text
<type>(<scope>): <summary>
```

### Types

| Type | Semver | When to use |
|------|--------|-------------|
| `feat` | minor | New user-facing behaviour |
| `fix` | patch | Bug fix |
| `refactor` | patch | Internal restructure with no behaviour change |
| `perf` | patch | Performance improvement |
| `docs` | – | Documentation only |
| `test` | – | Test additions or corrections |
| `chore` | – | Build, CI, dependencies, tooling |

Append `!` after the scope for breaking changes (for example `feat(commands)!: ...`) to indicate a major version bump.

### Scopes

Scopes match the top-level module structure:

| Scope | Maps to |
|-------|---------|
| `catalogue` | `crates/ags-runtime/src/catalogue/` — spec loading, parsing, manifest, caching |
| `invocation` | `crates/accelbyte-ags-cli/src/invocation/` — CLI tree, flags, command handlers |
| `protocol` | `crates/ags-protocol/src/` — boundary types shared across crates |
| `frontend` | `crates/accelbyte-ags-cli/src/frontend/` — human/JSON frontends, progress, style, templates, command renderers |
| `runtime` | `crates/ags-runtime/src/runtime/` — auth, config, diagnostics, dispatch, facade |
| `support` | `crates/ags-runtime/src/support/` — file_system, strings, shared helpers |

For root-level or cross-cutting changes:

| Scope | When to use |
|-------|-------------|
| `errors` | `crates/accelbyte-ags-cli/src/errors.rs` |
| `specs` | `crates/ags-runtime/specs/` — bundled OpenAPI specs |

Omit the scope only when a commit genuinely spans the entire codebase (for example `chore: bump MSRV to 1.85`).

### Examples

```text
feat(invocation): add --output flag for file redirection
fix(runtime): handle expired refresh token in client credentials flow
refactor(frontend): split service renderer from shared frontend templates
docs: update architecture tree after module restructure
test(catalogue): add x-operationId edge cases for matchmaking spec
chore: upgrade clap to 4.5
feat(invocation)!: rename --format to --output-format
```

## Coding Rules

### Rust standards

- Follow the official Rust style guide: https://doc.rust-lang.org/style-guide/
- Always run `rustfmt` before committing
- Always run `clippy` before committing: `cargo clippy -- -D warnings`

### Naming

- Prefer human-readable names over brevity — for example `operation_command` not `op_cmd`, `definition` not `def`, `resource` not `res`
- Names should convey intent without requiring the reader to decode shorthand
- Test functions annotated with `#[test]` or `#[tokio::test]` must be named `test_<scenario>` so test output groups them visibly. Test-helper functions (no `#[test]` attribute) must not use the `test_` prefix

### Code quality

- Keep functions short and focused on a single responsibility
- Use guard clauses to reduce nesting
- Avoid magic numbers — use named constants
- Write code for the reader, not the writer
- Prefer explicit over implicit behaviour
- Handle errors at the appropriate level
- Minimise variable scope — declare variables close to first use

### Function ordering

Within each module, order functions as:

1. Public functions (`pub`, `pub(crate)`, `pub(super)`) first — the module's externally visible surface
2. Private helpers after, **grouped by purpose** — keep helpers that serve the same concern adjacent, regardless of call order

Test modules (`#[cfg(test)] mod tests`) sit at the bottom of the file. Within an `impl` block, the same rule applies: public methods first, then private methods grouped by purpose.

### Comments

**Function doc comments (`///`):**

- Every function (public or private, including test-helper functions) has a doc comment. `#[test]` functions are exempt because the function name describes the scenario
- One line only, unless there is something non-obvious that warrants explanation
- Describe what the function does and why it exists, not how it works
- Do not restate what the signature already communicates
- Add extra lines only to explain non-obvious behaviour, important constraints, or design decisions
- Trait-impl methods inherit their documentation from the trait and do not need their own doc comment unless they document impl-specific behaviour

**Inline comments:**

- Default to no inline comments — well-named code should speak for itself
- Only add an inline comment when the code does something unexpected, counter-intuitive, or requires context the reader cannot derive from the code alone
- Do not narrate what the code is doing step by step

### Terminal input (TUI)

- **Act on key *presses* only.** Windows emits both a Press and a Release `KeyEvent` for every keystroke, so any reader that doesn't filter will register each input twice (a double-typed character, a double-submitted Enter). macOS/Linux only send Press, so this bug is invisible there — it must be guarded proactively.
- Prefer reading keys through `frontend::terminal::form_runner::crossterm_next_key`, which already returns only `KeyEventKind::Press` events.
- Any loop that calls `crossterm::event::read()` directly must skip non-press events before acting on them:

  ```rust
  if let Ok(Event::Key(key)) = event::read() {
      if key.kind != crossterm::event::KeyEventKind::Press {
          continue;
      }
      // … handle the keystroke
  }
  ```

  Existing direct readers that follow this: the fullscreen dismiss loop (`fullscreen/frontend.rs`), the inline phase loop (`inline/phases/mod.rs`), and the dynamic-enum spinner loop (`dynamic_options.rs`).

### Security

- **Path parameters**: Always use `strings::encode_url_path_segment()` when interpolating user input into URL path segments. Never use raw `str::replace` with unvalidated values.
- **Display output**: All API-sourced strings must pass through `strings::strip_terminal_control_sequences()` before terminal rendering.
- **Auth error bodies**: Truncate large error response bodies via `strings::truncate_display_text()` before surfacing them to users.
- **No raw API text to terminal**: Non-JSON response bodies must be sanitized before `println!`. JSON passthrough (`--format json`) is intentionally unsanitized for machine consumption.
- **Config dir and file permissions**: 0700 for directories, 0600 for files.
- **No plaintext secret fallback**: Tokens are stored via the system keychain.
- **Service name allowlist**: Prevents path traversal via service names.

## Testing

```bash
cargo test                                              # All tests
cargo test --test functional                            # Functional tests (CLI behaviour, auth, IAM)
cargo test --test integration                           # Integration tests
cargo test --test contract_input                        # Input contract tests (parser vs Python reference)
cargo test --test contract_output                       # Output contract tests
cargo test --test snapshot                              # Snapshot tests
cargo test --test security                              # Security tests
cargo test --test performance                           # Performance tests (debug thresholds)
cargo test --release --test performance -- --ignored    # Performance tests (release thresholds)
cargo test --release -p ags-runtime --lib catalogue     # Parser/catalogue release-mode fallback paths
```

> **Note:** Some parser and catalogue tests are gated on `#[cfg(not(debug_assertions))]` and only run under `cargo test --release`. These cover the graceful-fallback paths for unsupported HTTP verbs, parameter locations, and value types. Run `cargo test --release -p ags-runtime --lib catalogue` when modifying parser error-handling code.

The input contract tests loop over every service in the manifest. For each one they load the bundled spec, parse it via `parser::parse_spec`, and compare the result against the per-service baseline at `crates/accelbyte-ags-cli/tests/fixtures/baselines/<service>_input_contract.json`. The comparison is split into two tests:

- **`test_no_breaking_changes`** — a one-way gate. Every resource, operation, parameter (`name`/`location`/`required`), `http_method`, `path`, and `has_request_body` recorded in the baseline must still be present and unchanged in the parse. Additions (new operations or parameters) are **tolerated**; removals or mutations **fail**. A failure here is a genuine breaking change — **do not regenerate the baselines to silence it**.
- **`test_baseline_is_current`** — the freshness check. Any divergence from the baseline (including additions and summary changes) fails, signalling the baseline is stale. This is the only signal that should prompt regeneration — and only **after** `test_no_breaking_changes` is green:

  ```bash
  python3 scripts/generate_cli_command_catalogue.py --emit-baselines crates/accelbyte-ags-cli/tests/fixtures/baselines
  ```

### Updating snapshots

When intentional output changes cause snapshot tests to fail, review and accept the new snapshots:

```bash
cargo insta review
```

### Exploratory QA via Claude Code

The repo ships a Claude Code skill at `.claude/skills/qa-test/` that drives an interactive QA pass over the CLI. It complements (does not replace) the automated test suite: instead of asserting fixed contracts, it discovers the command surface from `--help`, authenticates against a real environment, and exercises commands with both happy-path and error scenarios — much like a human operator would on a fresh install. The skill produces a structured markdown report under `docs/qa-reports/` (gitignored) with every command and its full output captured verbatim.

Use it when you want manual-style coverage on top of the structured tests — for example after touching auth, error rendering, or a service-wide change where regressions might escape unit-test scope. From a Claude Code session in this repo:

```
/qa-test
```

The skill prompts for the base URL, namespace, OAuth client ID, profile (smoke / standard / thorough), and whether to include mutating operations. It then runs autonomously until the pass is complete. Mutations are scoped to the namespace you provide and to resources the skill itself creates during the session.

## Generating the Command Catalogue

The command catalogue and test baseline are generated from the raw OpenAPI specs using a Python script:

```bash
# Regenerate the full command catalogue (markdown)
python3 scripts/generate_cli_command_catalogue.py > docs/reference/cli-command-catalogue.md

# Regenerate the per-service test baseline fixtures (JSON)
python3 scripts/generate_cli_command_catalogue.py --emit-baselines crates/accelbyte-ags-cli/tests/fixtures/baselines/
```

The Rust parser is tested against the per-service baselines to ensure it matches the Python reference exactly. If you change parser semantics in `crates/ags-runtime/src/catalogue/parser.rs` (or string helpers in `crates/ags-runtime/src/support/strings.rs`), regenerate the baselines and verify the diffs are intentional.

## Adding a Workflow

Workflows are runtime-owned sequences of API operations defined in
`crates/ags-runtime/src/runtime/workflows/`. Follow the steps below when
adding a new built-in workflow. Use `builtins/competitive_multiplayer.rs` as
the worked example throughout.

### Location

Create the workflow in a new file at
`crates/ags-runtime/src/runtime/workflows/builtins/<kebab-name>.rs`, where
`<kebab-name>` is the workflow's id in kebab-case (for example
`competitive-multiplayer` → `competitive_multiplayer.rs` — note the
underscore in the filename, kebab in the id).

### Structure

Each workflow file contains:

- A `struct` holding a single `definition: WorkflowDefinition` field.
- An `impl Workflow` that returns `&self.definition` from `definition()`.
- A `fn build_definition() -> WorkflowDefinition` that constructs the
  complete literal definition. Keep this function private; `new()` calls it.
- Small private helpers that mirror those in `competitive_multiplayer.rs`:
  - `literal(value: Value) -> BindingSource` — wraps a JSON value in a
    non-sensitive `LiteralBinding`.
  - `workflow_ref(input: &str) -> BindingSource` — produces a
    `from: workflow/<input>` reference binding.
  - `bind(field: &str, source: BindingSource) -> StepInputBinding` — pairs
    a field name with its source.
  - `step(id, description, service, operation, dependencies, inputs) -> StepDefinition` —
    builds one step with `confirm: false` and no captured outputs by default.
  - `input(name, description, required, default, schema) -> WorkflowInputSpec` —
    declares one workflow-level input.
  - `input_with_options(name, description, required, default, schema, options_source) -> WorkflowInputSpec` —
    declares a **dynamic-enum** input whose choices are fetched at runtime. The
    `OptionsSource` names a read-only (GET) catalogue operation, its parameter
    bindings, and JSONPath projections (`items_path` → the array, `value` → each
    choice's value, optional `label` → its display text). This is an interactive
    affordance only: the fullscreen Phase-1 form renders it as a type-to-filter
    modal picker, while non-interactive and `--format json` runs treat the input
    as a plain string. `compile_workflow` validates the source (operation exists,
    is GET, paths parse). See `fleetRegion` / `fleetInstanceId` in
    `competitive_multiplayer.rs`.

If your workflow needs step-output capture, extend the `outputs` field on
the relevant `StepDefinition` directly rather than adding a helper.

### Registration

Add a `registry.register(...)` call inside `register_builtins` in
`crates/ags-runtime/src/runtime/workflows/builtins/mod.rs`. Also add the
module declaration (`pub mod <name>;`) at the top of that file.
`registry()` populates the process-wide registry through a `OnceLock` on
first access; `register_builtins` is the init closure.

### Bindings can target nested fields

A `StepInputBinding` binds one operation field to one source. The field is
usually a top-level path parameter, query parameter, or body field, but
**nested-field paths are supported** via the wire form
`field: "data.matching_rule[0].attribute"` — parsed by
`workflows/nested_path.rs` and validated against the operation schema by
`compile_workflow`. To supply a whole free-form object in one go, bind the
top-level field as a single `literal(json!({...}))` value.

### References

Two reference forms are available:

- `workflow_ref("input-name")` — re-uses a workflow-level input, declared in
  the `inputs` vec of `WorkflowDefinition`.
- For step-output references, construct the binding directly using
  `BindingSource::Reference(ReferenceBinding { from: ReferenceTarget::Step { id }, output, transform })`.
  The `from: step/<id>` source re-uses a captured output from an earlier step;
  the earlier step must declare that capture in its `outputs` vec. No helper
  function for this form exists in the built-in module — add one if your
  workflow uses step outputs extensively.

### Required fields

Every required operation field must either be bound in `inputs` or will
auto-derive into a gather slot (the runtime will prompt the user or fail
under `--no-input`). The `compile_workflow` test (below) catches unbound
required fields, so a failing test is a reliable signal that a binding is
missing.

### `confirm: true`

Set `confirm: true` on a step only for risky operations — deletes, bans, or
irreversible mutations. This follows the same policy as the `requires_confirmation`
keyword in the dispatch layer. All steps in `competitive_multiplayer.rs` use
`confirm: false` because they are non-risky creates and updates. Introduce
`confirm: true` deliberately and document why in an inline comment.

### Testing

Add three tests inside a `#[cfg(test)] mod tests` block at the bottom of
your workflow file:

1. **Compile test** — calls `compile_workflow(workflow.definition(), &mut Catalogue::new())`
   and asserts it succeeds, then checks the returned step ids match the
   expected order. See `test_competitive_multiplayer_compiles` for the
   pattern.

2. **No unbound required fields** — iterates `compiled.steps`, filters
   `step.auto_derived` to entries where `f.required` is true, and asserts the
   slice is empty for every step. See `test_no_required_field_left_unbound`
   for the pattern.

3. **Offline dry-run end-to-end test** — add an integration test file under
   `crates/accelbyte-ags-cli/tests/functional/` that runs the workflow with
   `--dry-run` and all required inputs supplied, and asserts the expected
   output (step previews, no network calls). Register it in
   `crates/accelbyte-ags-cli/tests/functional.rs` via a `#[path = ...]`
   attribute so it is picked up by `cargo test --test functional`.

### YAML authoring alternative

Every built-in workflow above is a Rust literal, but you can also author a
workflow as a standalone YAML file and install it without touching the
binary at all — useful for a quick one-off, or for a workflow that shouldn't
be bundled into the CLI itself.

Run `ags workflow template [--output <path>]` to print (or write) a starter
YAML skeleton with the annotated shape `WorkflowDefinition` expects — the
same fields as the Rust struct, since `WorkflowDefinition` derives
`Serialize`/`Deserialize` and nothing downstream cares how it was
constructed. Hand-edit the file: fill in a real `id`, steps, and inputs, following
the same binding wire forms documented above (`from`, `const`, `format`,
nested-field paths, `options_source`, etc.).

Every external workflow must declare `workflow_protocol_version: "<version>"`
— the workflow YAML protocol/schema version the file was authored against,
deliberately unrelated to the `ags` CLI's own release version. `ags workflow
template` fills this in for you automatically with the protocol version this
CLI build speaks (`ags_protocol::workflow::WORKFLOW_PROTOCOL_VERSION`); `ags
workflow add` rejects any file that omits it outright. This isn't a
compatibility check (nothing verifies the workflow will actually work under a
different protocol version) — it's a provenance hint: `ags workflow run`
prints a one-line, non-blocking warning whenever an installed external
workflow's declared protocol version doesn't match this CLI build's own (in
either direction — older or newer), so a shared or long-unused workflow's
confusing behavior has a place to start looking, instead of failing silently
with no explanation.

**If you change the shape of `WorkflowDefinition`/`StepDefinition`** (add,
remove, rename, or change the wire form of a field — anything that changes
what a workflow YAML file is expected to look like), bump
`WORKFLOW_PROTOCOL_VERSION` in `crates/ags-protocol/src/workflow.rs`
alongside that change. Nothing enforces this automatically — no test fails
if you change the schema without bumping it — so it relies on you noticing.
Use ordinary semver judgment: a breaking wire-format change (a required
field's name or type changes, a variant is removed) is at least a minor
bump; a purely additive, backward-compatible field is optional to bump at
all, since an older workflow file still parses fine either way.

Then run `ags workflow add <path>` to validate it (parses the YAML, compiles
it against the bundled catalogue exactly like a built-in) and install it into
`workflows_dir()` under `<id>.yaml`. Pass `--validate-only` to check the file
without installing it. `workflows_dir()` mirrors `profiles_dir()`'s
resolution — a plain subdirectory of the config directory, so it
automatically respects `AGS_HOME` — and every file in it is loaded into the
workflow registry alongside the built-ins the next time `ags` runs. An id
that collides with an already-registered workflow (built-in or previously
installed) is rejected outright; choose a different id, or run
`ags workflow remove <id>` to remove the existing external workflow first.

Run `ags workflow remove <id>` to uninstall a workflow previously added this
way. It only ever deletes a file under `workflows_dir()` — it matches by the
file's own `id:` field (not its filename, since a hand-copied file's name
need not match its id), so it finds a match regardless of how the file was
named. A built-in workflow (a Rust struct or a bundled YAML file compiled
into the `ags` binary) cannot be removed and produces an error naming it as
built-in. If an installed file's id happens to collide with a built-in's
(the file was already shadowed — `external.rs`'s loader lets the built-in
win and never registers the file), `remove` still deletes it, but says so
explicitly rather than implying the workflow is gone entirely.

One wire-format detail is easy to miss: `options_source.parameters` values
(`OptionParameterBinding`) are a plain (non-`#[serde(untagged)]`) enum, so in
YAML they need the native `!from_input <name>` tag form, not a JSON-style
nested map like `{from_input: someInput}`. See the commented-out example in
`ags workflow template`'s output for the exact syntax.

## Key Design Decisions

- **Two-phase parsing**: Pre-scan argv for global flags → validate service name against the manifest → load bundled spec → parse to `ServiceSchema` → build Clap tree → match → execute
- **Global flags accepted anywhere**: Pre-scan extracts `--verbose`, `--namespace`, `--format`, and related flags before Clap sees them
- **No plaintext secret fallback**: Keychain or env vars only. Clear error if keychain is unavailable
- **Service name allowlist**: Validates against the manifest before loading specs and prevents path traversal
- **Mutation confirmation**: All mutating operations (POST, PUT, PATCH, DELETE) require confirmation; use `--yes` to skip in CI
- **`--json` passthrough**: Request bodies are sent as-is with no client-side schema validation
- **`--namespace` resolution**: flag → `AGS_NAMESPACE` env → config → error
- **`--dry-run`**: Shows request details without auth and works even when not logged in
