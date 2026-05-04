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

### Regenerating the demo reel

The README GIF is produced from `demo/reel.tape` using [VHS](https://github.com/charmbracelet/vhs) driven against a local mock server. To regenerate after tape or server changes:

```
brew install vhs        # or see charmbracelet/vhs for other platforms
./demo/record.sh
```

This rebuilds the CLI in release mode, starts `demo/demo-server.py` on localhost:8765, drives the tape, and writes `demo/reel.gif`. Commit the updated GIF alongside any tape or server changes.

## Architecture

```
src/
├── main.rs                      # Entry point: SIGPIPE reset, Tokio bootstrap, delegates to invocation::run
├── lib.rs                       # Library root (for integration tests)
├── catalogue/                   # OpenAPI spec loading, parsing, caching
│   ├── bundled.rs               # Bundled spec loading (include_bytes! + gzip)
│   ├── cache.rs                 # On-disk parsed-schema cache I/O
│   ├── manifest.rs              # 24-service allowlist + display names + descriptions
│   ├── memory_cache.rs          # In-process cache of parsed ServiceSchema values
│   ├── openapi.rs               # OpenAPI 2.0 (Swagger) wire types used by the parser
│   ├── parser.rs                # SwaggerSpec → ServiceSchema (driven by x-operationId)
│   ├── repository.rs            # Orchestrates bundled + cache + memory_cache loads
│   └── skeleton.rs              # Request body template generation
├── errors.rs                    # CliError enum, ErrorMetadata, exit codes
├── invocation/                  # CLI layer: flag parsing, command tree, routing
│   ├── builder.rs               # Dynamic Clap tree from ServiceSchema
│   ├── errors.rs                # Invocation error types
│   ├── flags.rs                 # GlobalFlags, pre-scan, namespace resolution
│   ├── resolve.rs               # Resolves --api-scope/--api-version to a concrete contract
│   ├── router.rs                # Page-limit parsing and per-command dispatch
│   └── commands/                # Command handlers
│       ├── auth/                # Auth subcommands
│       │   ├── mod.rs           # Login/logout/status dispatch
│       │   └── oauth.rs         # OAuth callback server for browser flow
│       ├── completions.rs       # Shell completion generation
│       ├── config.rs            # Config get/set/unset dispatch
│       ├── describe/            # Machine-readable command introspection
│       │   ├── mod.rs           # `ags describe` command dispatch
│       │   └── envelope.rs      # JSON envelope shapes for describe output
│       ├── doctor.rs            # Diagnostic check dispatch
│       ├── profile.rs           # Profile CRUD dispatch
│       ├── refresh_specs.rs     # `ags refresh-specs` subcommand dispatch
│       ├── service/             # Dynamic service-command pipeline
│       │   ├── mod.rs           # Top-level service handler (parse → dispatch)
│       │   ├── clap_tree.rs     # Clap subtree construction for a service
│       │   ├── dispatch.rs      # Dry-run, confirmation, and execution
│       │   ├── help.rs          # Contextual help rendering
│       │   ├── parser.rs        # Args → ParsedServiceCommand
│       │   └── request.rs       # ParsedServiceCommand → CommandRequest
│       └── version.rs           # Version output dispatch
├── protocol/                    # Boundary types between invocation and runtime
│   ├── catalogue.rs             # Catalogue query/result types
│   ├── config.rs                # Config operation types
│   ├── diagnostics.rs           # Diagnostic check types
│   ├── error.rs                 # Protocol-level error types
│   ├── event.rs                 # Runtime event types
│   ├── output.rs                # Structured output envelope
│   ├── output_views.rs          # View/payload types attached to command outputs
│   ├── request.rs               # API request types
│   └── result.rs                # Operation result types
├── frontend/                    # User-facing I/O — output rendering, prompts, progress
│   ├── render.rs                # Top-level CommandOutput → RenderedOutput dispatch
│   ├── templates.rs             # Backend-agnostic templates (return StyledLine IR)
│   ├── presenters/              # Format-neutral presentation helpers
│   │   ├── auth.rs              # Auth-source labels, token-state views
│   │   └── service.rs           # Dry-run and API-response views
│   ├── style/                   # Semantic tones, styled-line IR, ANSI backend
│   │   ├── ansi.rs              # Tone → ANSI, is_stdout/stderr_enabled, init
│   │   ├── span.rs              # StyledSpan, StyledLine
│   │   ├── text.rs              # Prefix symbol constants
│   │   └── tone.rs              # Tone enum (Plain, Dim, Success, …)
│   ├── human/                   # Human-readable frontend
│   │   ├── frontend.rs          # impl Frontend for HumanFrontend
│   │   ├── progress.rs          # StatusLine, StatusLineSink
│   │   ├── prompt.rs            # Interactive confirmation prompts
│   │   ├── templates.rs         # ANSI-applying text adapters (_text variants)
│   │   └── commands/            # Per-command human renderers
│   │       ├── auth.rs
│   │       ├── completions.rs
│   │       ├── config.rs
│   │       ├── doctor.rs
│   │       ├── profile.rs
│   │       ├── refresh_specs.rs
│   │       ├── service.rs       # API response rendering (tables, inspect)
│   │       └── version.rs
│   └── json/                    # Machine-readable JSON frontend
│       ├── frontend.rs          # impl Frontend for JsonFrontend
│       ├── progress.rs          # NoopProgressSink (JSON mode emits no progress)
│       └── commands/            # Per-command JSON emitters
├── runtime/                     # All business logic and external interaction
│   ├── cleanup.rs               # Startup cleanup of stale temp files
│   ├── completions.rs           # Completion script generation
│   ├── execution.rs             # Top-level command execution coordinator
│   ├── auth/                    # OAuth2 flows, credential storage, sessions
│   │   ├── credentials.rs       # Client/base URL credential resolution
│   │   ├── errors.rs            # AuthError domain type
│   │   ├── locking.rs           # Cross-process token lock coordination
│   │   ├── operations.rs        # Login, logout, status operations
│   │   ├── session.rs           # Access-token lifecycle policy
│   │   ├── store.rs             # OS keychain/file token persistence
│   │   └── tokens.rs            # OAuth token endpoint types and calls
│   ├── config/                  # Configuration management
│   │   ├── environment.rs       # AGS_* environment variables and defaults
│   │   ├── errors.rs            # Config-layer error helpers
│   │   ├── keys.rs              # Config key definitions and validation
│   │   ├── paths.rs             # Config and cache path derivation
│   │   └── store.rs             # ConfigStore, GlobalConfig, ProfileConfig
│   ├── diagnostics/             # Health checks and troubleshooting
│   │   ├── checks.rs            # Individual diagnostic checks
│   │   └── runner.rs            # Diagnostic runner and reporting
│   ├── dispatch/                # API call execution and error classification
│   │   ├── classify.rs          # HTTP status + error code → user-friendly message
│   │   ├── confirmation.rs      # Confirmation rules for risky operations
│   │   ├── error_codes.rs       # AccelByte error code lookup table
│   │   ├── execute.rs           # Main API call execution pipeline
│   │   ├── http.rs              # HTTP client and request execution
│   │   ├── pagination.rs        # Paginated response handling
│   │   ├── path.rs              # Path placeholder substitution
│   │   └── shape.rs             # Response shape detection and normalization
│   └── facade/                  # High-level orchestration
│       ├── auth.rs              # Auth facade
│       ├── config.rs            # Config facade
│       ├── diagnostics.rs       # Diagnostics facade
│       ├── profile.rs           # Profile facade
│       └── service.rs           # Service call facade
└── support/                     # Shared utilities
    ├── file_system.rs           # Restricted writes, advisory locks, temp cleanup
    ├── mod.rs                   # Time, TTY, and small shared helpers
    ├── output_sink.rs           # Stdout/file destination resolution and writes
    └── strings.rs               # Naming, sanitization, and display transforms
```

### Architecture guardrails

Each module has a single responsibility. Cross-module imports must follow the allowed dependency directions.

| Module | Responsibility |
|--------|----------------|
| `catalogue` | OpenAPI spec loading, x-operationId-driven parsing, and caching |
| `protocol` | Boundary types shared across modules — request, result, event, error, and output envelopes |
| `invocation` | Turns user input into typed requests — argv parsing, flag extraction, command routing |
| `runtime` | Owns execution — endpoint calls, auth, config, diagnostics, validation, workflow transitions |
| `frontend` | Owns all user-visible formatting — tables, inspect views, JSON output, progress, colour |
| `support` | Small shared utilities — filesystem helpers, string transforms, TTY/time utilities |

**Forbidden dependencies:**

- `runtime` must not depend on `frontend` or `invocation`
- `frontend` must not depend on `runtime` or `invocation`
- `catalogue` must not depend on any other application module
- `support` must not depend on any other application module

**Operational rules:**

- `runtime` must not print or prompt — all user-visible output flows as structured data through `frontend`
- `frontend` must not call endpoints, execute commands, or decide what action to take
- `invocation` must not own business logic — it builds typed requests and lets `runtime` validate

**Required invariants:**

- Structured output is canonical; human-readable output is derived from it
- Workflows request input via state (`AwaitingConfirmation`), not stdin

## Key Files

| File | Purpose |
|------|---------|
| `scripts/generate_cli_command_catalogue.py` | **Canonical Python reference** for the command catalogue. Rust must match its output exactly |
| `src/runtime/dispatch/error_codes.rs` | AccelByte error code lookup table |
| `src/runtime/dispatch/classify.rs` | Error classification pipeline: error code → HTTP status → user-friendly message |
| `specs/*.json.gz` | All 24 gzip-compressed OpenAPI 2.0 specs bundled into the binary via `include_bytes!` |
| `tests/fixtures/baselines/<service>_input_contract.json` | Per-service Python-generated reference data for parser validation |

## Reference docs

All CLI behaviour is governed by the normative reference docs in `docs/reference/`. Each reference defines requirements (MUST / SHOULD / MAY); the code itself is the implementation. Read the relevant reference before modifying behaviour it governs:

| Document | Purpose |
|----------|---------|
| `docs/reference/cli-reference.md` | Normative requirements (MUST/SHOULD/MAY) for CLI behaviour |
| `docs/reference/output-reference.md` | Output formatting, tone, hierarchy, and rendering rules |
| `docs/reference/testing-reference.md` | Testing strategy, test categories, and validation requirements |
| `docs/reference/cli-command-catalogue.md` | Full command catalogue for all 24 services (auto-generated) |

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
| `catalogue` | `src/catalogue/` — spec loading, parsing, manifest, caching |
| `invocation` | `src/invocation/` — CLI tree, flags, command handlers |
| `protocol` | `src/protocol/` — boundary types between invocation and runtime |
| `frontend` | `src/frontend/` — human/JSON frontends, progress, style, templates, command renderers |
| `runtime` | `src/runtime/` — auth, config, diagnostics, dispatch, facade |
| `support` | `src/support/` — file_system, strings, shared helpers |

For root-level or cross-cutting changes:

| Scope | When to use |
|-------|-------------|
| `errors` | `src/errors.rs` |
| `specs` | `specs/` — bundled OpenAPI specs |

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
cargo test --release --lib catalogue                    # Parser/catalogue release-mode fallback paths
```

> **Note:** Some parser and catalogue tests are gated on `#[cfg(not(debug_assertions))]` and only run under `cargo test --release`. These cover the graceful-fallback paths for unsupported HTTP verbs, parameter locations, and value types. Run `cargo test --release --lib catalogue` when modifying parser error-handling code.

The input contract test loops over all 24 services. For each one it loads the bundled spec, parses it via `parser::parse_spec`, and compares the result against the per-service baseline at `tests/fixtures/baselines/<service>_input_contract.json`. Any drift from the Python reference causes the test to fail.

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
python3 scripts/generate_cli_command_catalogue.py --emit-baselines tests/fixtures/baselines/
```

The Rust parser is tested against the per-service baselines to ensure it matches the Python reference exactly. If you change parser semantics in `src/catalogue/parser.rs` (or string helpers in `src/support/strings.rs`), regenerate the baselines and verify the diffs are intentional.

## Key Design Decisions

- **Two-phase parsing**: Pre-scan argv for global flags → validate service name against the manifest → load bundled spec → parse to `ServiceSchema` → build Clap tree → match → execute
- **Global flags accepted anywhere**: Pre-scan extracts `--verbose`, `--namespace`, `--format`, and related flags before Clap sees them
- **No plaintext secret fallback**: Keychain or env vars only. Clear error if keychain is unavailable
- **Service name allowlist**: Validates against the manifest before loading specs and prevents path traversal
- **Mutation confirmation**: All mutating operations (POST, PUT, PATCH, DELETE) require confirmation; use `--yes` to skip in CI
- **`--json` passthrough**: Request bodies are sent as-is with no client-side schema validation
- **`--namespace` resolution**: flag → `AGS_NAMESPACE` env → config → error
- **`--dry-run`**: Shows request details without auth and works even when not logged in
