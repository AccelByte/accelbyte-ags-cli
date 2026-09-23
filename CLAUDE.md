# AGS CLI

A Rust CLI that dynamically generates commands from AccelByte's bundled OpenAPI 2.0 specs.

## Workflows

Workflows are runtime-owned sequences of API operations. A single command
(`ags <service> <resource> <method>`) is internally a 1-step synthesised workflow;
`ags workflow run <id>` runs a registered multi-step workflow. The shared
engine lives in `crates/ags-runtime/src/runtime/workflows/`; built-in
workflows are in `runtime/workflows/builtins/`. See
`docs/private/workflow-protocol.md`.

## Workspace layout

Four crates under `crates/`. Dependency direction is strictly `accelbyte-ags-cli → ags-runtime → ags-protocol`. Reverse edges are forbidden.

### `ags-protocol/` — leaf crate (serde, serde_json, thiserror only)

Typed protocol contracts shared across crates.

```
crates/ags-protocol/src/
├── lib.rs                       # Crate root: re-exports every protocol module
├── catalogue.rs                 # ServiceId, OperationId, ServiceSchema, command catalogue types
├── config.rs                    # Config operation types
├── diagnostics.rs               # Diagnostic check types
├── error.rs                     # RuntimeError, ErrorMetadata, SuggestionKind
├── event.rs                     # Runtime event types (progress, lifecycle)
├── output.rs                    # Structured output envelope
├── output_views.rs              # View/payload types attached to command outputs
├── request.rs                   # API request types, GrantType, CommandFormat
├── result.rs                    # Operation result types
└── workflow.rs                  # Workflow contract types: definitions, steps, bindings, inputs, options_source
```

### `ags-runtime/` — business logic, depends on ags-protocol

```
crates/ags-runtime/
├── specs/                       # Gzip-compressed OpenAPI 2.0 specs, bundled via include_bytes!
├── workflows/                   # Bundled workflow YAML files (sibling to specs/), included via include_str!
├── templates/                   # Non-workflow starter YAML (e.g. `ags workflow template`'s skeleton), included via include_str!
└── src/
    ├── lib.rs                   # Crate root: pub mod runtime; catalogue; support;
    ├── catalogue/               # OpenAPI spec loading, parsing, caching
    │   ├── bundled.rs           # Bundled spec loading (include_bytes! + gzip)
    │   ├── cache.rs             # On-disk parsed-schema cache I/O
    │   ├── manifest.rs          # Service allowlist + display names + descriptions
    │   ├── memory_cache.rs      # In-process cache of parsed ServiceSchema values
    │   ├── openapi.rs           # OpenAPI 2.0 (Swagger) wire types used by the parser
    │   ├── parser.rs            # SwaggerSpec → ServiceSchema (driven by x-operationId)
    │   ├── repository.rs        # Orchestrates bundled + cache + memory_cache loads
    │   └── skeleton.rs          # Request body template generation
    ├── runtime/                 # All business logic and external interaction
    │   ├── cleanup.rs           # Startup cleanup of stale temp files
    │   ├── execution.rs         # Top-level command execution coordinator
    │   ├── ams_upload/          # `ags ams upload` — bespoke (non-workflow) upload pipeline
    │   │   ├── mod.rs           # UploadRequest, TargetArchitecture, Runtime facade methods
    │   │   ├── entrypoint.rs    # Name/directory checks, filename case, entrypoint classification
    │   │   ├── elf.rs           # ELF accept matrix (ELFCLASS64 + little-endian + x86-64/aarch64)
    │   │   ├── archive.rs       # Directory walk, symbol-file exclusion, tar.gz build
    │   │   ├── discovery.rs     # AMS upload-host discovery (hard fail, no prod fallback)
    │   │   ├── api.rs           # AMS upload wire contract (images, presign, multipart, complete)
    │   │   ├── pipeline.rs      # Orchestration: validate → pack → upload → complete
    │   │   └── errors.rs        # AmsUploadError domain type
    │   ├── auth/                # OAuth2 flows, credential storage, sessions
    │   │   ├── credentials.rs   # Client/base URL credential resolution
    │   │   ├── errors.rs        # AuthError domain type
    │   │   ├── locking.rs       # Cross-process token lock coordination
    │   │   ├── operations.rs    # Login, logout, status operations
    │   │   ├── session.rs       # Access-token lifecycle policy
    │   │   ├── store.rs         # OS keychain/file token persistence
    │   │   └── tokens.rs        # OAuth token endpoint types and calls
    │   ├── config/              # Configuration management
    │   │   ├── environment.rs   # AGS_* environment variables and defaults
    │   │   ├── errors.rs        # Config-layer error helpers
    │   │   ├── keys.rs          # Config key definitions and validation
    │   │   ├── paths.rs         # Config and cache path derivation
    │   │   └── store.rs         # ConfigStore, GlobalConfig, ProfileConfig
    │   ├── diagnostics/         # Health checks and troubleshooting
    │   │   ├── checks.rs        # Individual diagnostic checks
    │   │   └── runner.rs        # Diagnostic runner and reporting
    │   ├── dispatch/            # API call execution and error classification
    │   │   ├── classify.rs      # HTTP status + error code → user-friendly message
    │   │   ├── confirmation.rs  # Confirmation rules for risky operations
    │   │   ├── error_codes/     # AccelByte error code lookup tables (one file per service)
    │   │   ├── execute.rs       # Main API call execution pipeline
    │   │   ├── http.rs          # HTTP client and request execution (incl. network_error helper)
    │   │   ├── pagination.rs    # Paginated response handling
    │   │   ├── path.rs          # Path placeholder substitution
    │   │   └── shape.rs         # Response shape detection and normalization
    │   ├── facade/              # High-level orchestration consumed by invocation
    │   │   ├── auth.rs          # Auth facade
    │   │   ├── config.rs        # Config facade
    │   │   ├── diagnostics.rs   # Diagnostics facade
    │   │   ├── extend.rs        # Extend facade — EHS credential fetch for docker-login / image-upload
    │   │   ├── profile.rs       # Profile facade
    │   │   ├── service.rs       # Service call facade
    │   │   └── workflow.rs      # `workflow add`/`workflow template` facade — filesystem-only, no HTTP/auth
    │   ├── telemetry/           # PostHog event capture: opt-out gate, compile-time project key, event shaping
    │   ├── update_check/        # `ags update` support: latest-release query, hint cache, install-method detection
    │   │   ├── cache.rs         # Update-hint cache: latest version seen, already-notified marker
    │   │   ├── github.rs        # GitHub latest-release query and version comparison
    │   │   └── install_method.rs # Detect how this copy was installed (installer script, Homebrew, manual)
    │   └── workflows/           # Workflow engine (data types live in ags-protocol::workflow)
    │       ├── synthesised.rs   # Build a 1-step workflow from a single CLI command
    │       ├── auto_derive.rs   # Expand a step's OpenAPI schema into auto-derived input fields
    │       ├── compile.rs       # WorkflowDefinition → CompiledWorkflow (validates bindings, options_source, nested paths)
    │       ├── resolve.rs       # Compute inputs still to gather; assemble the dispatch request
    │       ├── executor.rs      # Drive a compiled workflow: gather → confirm → dispatch → capture
    │       ├── options.rs       # Dynamic-enum option resolution (run a GET, project response → choices)
    │       ├── jsonpath.rs      # JSONPath subset for transforms and capture paths
    │       ├── nested_path.rs   # Parser for nested-field binding paths (`data.x[0].y`)
    │       ├── dry_run.rs       # Synthesise placeholder step outputs for --dry-run previews
    │       ├── external.rs      # Load user-installed workflow YAML from workflows_dir() at registry() init
    │       ├── bundled.rs       # Load the compiled-in YAML-authored builtin(s) (sibling to builtins/, not nested — it's a loader, not a workflow)
    │       ├── local_actions/   # Closed registry of local actions for `kind: local` workflow steps
    │       │   ├── mod.rs       # Action trait, lookup(), known_names(), test-only echo action
    │       │   └── docker_login.rs # `docker-login` action: `docker login --password-stdin` (secret via stdin, never argv)
    │       └── builtins/        # Registered built-in workflows, one Rust file per workflow (competitive_multiplayer.rs, etc.) + registry
    └── support/                 # Shared utilities (also used by frontend)
        ├── mod.rs               # Time, TTY, and small shared helpers
        ├── file_system.rs       # Restricted writes, advisory locks, temp cleanup, FileLock
        ├── output_sink.rs       # Stdout/file destination resolution; OutputSinkError
        ├── process.rs           # Bounded child-process wait with timeout; WaitError; wait_with_timeout
        ├── strings.rs           # Naming, sanitization, and display transforms
        └── test_helpers.rs      # Shared test fixtures (cfg(test))
```

### `accelbyte-ags-cli/` — produces the `ags` binary, depends on both

```
crates/accelbyte-ags-cli/
├── src/
│   ├── main.rs                  # Entry point: SIGPIPE reset, Tokio bootstrap, delegates to invocation::run
│   ├── lib.rs                   # Library root (for integration tests)
│   ├── errors.rs                # CliError enum, ErrorView, exit codes
│   ├── invocation/              # CLI layer: flag parsing, command tree, routing
│   │   ├── builder.rs           # Dynamic Clap tree from ServiceSchema
│   │   ├── clap_helpers.rs      # Reusable clap value-parser and argument builders
│   │   ├── compat_flags.rs     # Backward-compatible flag definitions for migrated commands
│   │   ├── completions_generator.rs  # Completion script generation (clap_complete)
│   │   ├── confirm.rs           # shared confirmation helper (--yes / --no-input rules)
│   │   ├── errors.rs            # Invocation error types
│   │   ├── first_run.rs         # One-time first-run onboarding hint: gate predicate, emitter, seen-flag read
│   │   ├── flags.rs             # GlobalFlags, pre-scan, namespace resolution
│   │   ├── context.rs           # Frontend context: consumer kind + interaction surface resolution
│   │   ├── policy.rs            # (route, shape) → base surface decision matrix
│   │   ├── shape.rs             # Interaction-shape classification
│   │   ├── workflows.rs         # Bridge between parsed CLI commands and the workflow executor
│   │   ├── phase_execution.rs   # Shared post-prologue lifecycle for phase-owned runs
│   │   ├── resolve.rs           # Resolves --api-scope/--api-version to a concrete contract
│   │   ├── router.rs            # Root-route classification + page-limit parsing
│   │   ├── routes/              # Root execution routes
│   │   │   ├── ams_upload/      # `ags ams upload` route ownership (hand-written ams resource)
│   │   │   ├── auth/            # `ags auth ...` route ownership + OAuth callback server
│   │   │   ├── service/         # Dynamic service-command route (parse → synthesize → execute)
│   │   │   ├── builtin/         # Root help/version + built-in command route
│   │   │   ├── extend_docker_login.rs # `ags extend docker-login` route ownership
│   │   │   ├── extend_image_upload.rs # `ags extend image-upload` route ownership
│   │   │   └── workflow/        # `ags workflow run/list/add/template` route ownership
│   │   └── handlers/            # Leaf handlers invoked by the routes
│   │       ├── completions.rs   # `ags completions` dispatch
│   │       ├── config.rs        # Config get/set/unset dispatch
│   │       ├── describe/        # `ags describe` — machine-readable introspection
│   │       ├── doctor.rs        # Diagnostic check dispatch
│   │       ├── extend/          # `ags extend` subcommands: clone-template, app-ui, update-var, update-secret, security-assessment, migration shortcuts
│   │       │   ├── mod.rs       # Route `ags extend <subcommand>` to the appropriate handler
│   │       │   ├── app_lifecycle/ # `--wait` polling for the app lifecycle shims (create/deploy/start/stop/delete-app)
│   │       │   │   ├── mod.rs   # WaitRequest + run_wait_after_dispatch (resolve creds, drive poll loop)
│   │       │   │   ├── api.rs   # Direct CSM v5 GET app status (reqwest, wiremock-testable); Found/NotFound
│   │       │   │   └── wait.rs  # WaitSpec targets per command + sleep-then-poll loop mirroring extend-helper-cli
│   │       │   ├── app_ui/      # `ags extend app-ui` subcommands
│   │       │   │   ├── mod.rs   # Route `ags extend app-ui <subcommand>` to the appropriate handler
│   │       │   │   ├── setup_env.rs # `ags extend app-ui setup-env` — write .env.local from CSM App UI record
│   │       │   │   └── upload.rs    # `ags extend app-ui upload` — build, archive, and upload App UI assets
│   │       │   ├── clone_template/ # `ags extend clone-template` — clone Extend starter templates
│   │       │   ├── image_upload/  # `ags extend image-upload` — build and push container image
│   │       │   │   └── mod.rs     # Handler, pure functions, Docker introspection, OCI tag check, retry logic
│   │       │   ├── remote_debug/ # `ags extend remote-debug` — enable, disable, and connect debug sessions
│   │       │   │   ├── mod.rs     # remote_debug connect handler, retry policy, namespace and output envelopes
│   │       │   │   ├── connect_once.rs # Debug-info preconditions, tunnel/agent/forwarder orchestration
│   │       │   │   ├── debug_mode.rs # Shared debug-mode handler skeleton, dry-run gate, parameter struct, test helpers
│   │       │   │   ├── disable.rs # Disable debug mode with running-app confirmation
│   │       │   │   └── enable.rs  # Performance warning, running-app confirmation, debug-mode update
│   │       │   ├── tunnel/        # `ags extend tunnel` — TCP-to-WebSocket bridge for Extend apps
│   │       │   │   ├── mod.rs     # Handler, exit envelope, namespace/base-URL resolution
│   │       │   │   └── bridge.rs  # TCP listener, WS dial with 401 retry, bidirectional relay
│   │       │   ├── csm_error.rs   # Shared CSM error-detail extraction (errorCode/errorMessage from response body)
│   │       │   ├── update_secret/ # `ags extend update-secret` — upsert a CSM app secret
│   │       │   │   ├── mod.rs     # Handler, dry-run preview, merge-rule dispatch
│   │       │   │   ├── api.rs     # CSM secret list/create/update API calls (reqwest, wiremock-testable)
│   │       │   │   └── merge.rs   # Compute effective applyMask/description from existing record + overrides
│   │       │   ├── update_var/    # `ags extend update-var` — upsert a CSM app configuration variable
│   │       │   │   ├── mod.rs     # Handler, dry-run preview, merge-rule dispatch
│   │       │   │   ├── api.rs     # CSM variable list/create/update API calls (reqwest, wiremock-testable)
│   │       │   │   └── merge.rs   # Compute effective applyMask/description from existing record + overrides
│   │       │   ├── security_assessment_request/ # `ags extend security-assessment request` — discover, select, and submit a pen-test engagement
│   │       │   │   ├── mod.rs     # Handler, mutating-endpoint confirmation, dry-run preview
│   │       │   │   ├── api.rs     # CSM discovery/create API calls (reqwest + catalogue-driven, wiremock-testable)
│   │       │   │   ├── checklist.rs # Interactive ratatui endpoint-selection checklist
│   │       │   │   └── permission.rs # `--permission` override parsing ("RESOURCE [ACTION]")
│   │       │   ├── security_assessment_result/ # `ags extend security-assessment result` — list sessions and download a completed report
│   │       │   │   ├── mod.rs     # Handler, engagement picker, dry-run preview
│   │       │   │   └── api.rs     # CSM list/get-report API calls (reqwest + catalogue-driven, wiremock-testable)
│   │       │   ├── service_shims.rs # Migration shortcut registration table, Clap tree builder, and --wait flag parse/strip + WaitSpec wiring
│   │       │   └── session_log.rs   # Shared session event log for tunnel/remote-debug (lifecycle + connection events + tracing bridge for verbose proxy-client output)
│   │       ├── profile.rs       # Profile CRUD dispatch
│   │       ├── refresh_specs.rs # `ags refresh-specs` subcommand dispatch
│   │       ├── update.rs        # `ags update` release check dispatch
│   │       ├── update_install.rs # install steps of update --install: preserve, download, run installer, verify, restore
│   │       └── version.rs       # Version output dispatch
│   └── frontend/                # All user-facing output, split by responsibility
│       ├── mod.rs               # Frontend/ExecutionInteraction traits, surface selectors, RenderFormat
│       ├── event.rs             # Frontend lifecycle and progress event types
│       ├── sink.rs              # FrontendSink: bridges runtime ProgressSink to Frontend events
│       ├── streams.rs           # UiSink: stderr-only writes for UI chrome (spinners, prompts, hints)
│       ├── dynamic_options.rs   # DynamicOptionResolver trait + ProductionResolver (dynamic-enum picker bridge)
│       ├── output/              # Output-format rendering: dispatch + text serialization
│       │   ├── render.rs        # Shared CommandOutput → RenderedOutput dispatch
│       │   ├── templates.rs     # Backend-agnostic response templates
│       │   ├── human/           # Human-readable output rendering
│       │   │   ├── templates.rs # ANSI-applying text adapters over core templates
│       │   │   └── commands/    # Per-command human renderers
│       │   └── json/            # Machine-readable JSON output rendering
│       │       ├── frontend.rs  # impl Frontend for JsonFrontend
│       │       └── commands/    # Per-command JSON emitters
│       ├── terminal/            # Terminal interaction surfaces
│       │   ├── machine_json.rs  # JsonInteraction: JSON workflow contract seam
│       │   ├── form_runner.rs   # Surface-agnostic form ↔ JSON-editor ↔ confirm driver (Press-only key reads)
│       │   ├── plain/           # Plain (line-oriented) terminal surface
│       │   │   ├── frontend.rs  # impl Frontend for PlainFrontend
│       │   │   ├── interaction.rs # PlainInteraction workflow interaction
│       │   │   ├── progress.rs  # Status lines and spinner helpers
│       │   │   └── prompt.rs    # Interactive confirmation prompts
│       │   ├── inline/          # Inline (stderr-viewport) TUI surface
│       │   │   ├── frontend.rs  # impl Frontend for InlineFrontend
│       │   │   ├── interaction.rs # InlineInteraction workflow interaction
│       │   │   ├── form.rs      # Inline form widget (fields, focus, hints)
│       │   │   ├── form_builder.rs # Build form fields from workflow inputs
│       │   │   ├── json_editor/ # Structured JSON request-body editor
│       │   │   ├── lifecycle.rs # Inline-viewport acquisition and teardown
│       │   │   ├── progress_state.rs # Inline-viewport progress state and redraw
│       │   │   ├── session.rs   # InlineSession: shared live-terminal handle
│       │   │   └── phases/      # Per-phase inline UI (form, confirm, confirm_card, result)
│       │   └── fullscreen/      # Fullscreen alt-screen workflow TUI (§13 four-region layout)
│       │       ├── frontend.rs  # impl Frontend for FullscreenFrontend + dismiss loop
│       │       ├── interaction.rs # FullscreenInteraction: gather/confirm/picker in-layout
│       │       ├── surface.rs   # FullscreenSurface: owns the terminal + render model
│       │       ├── lifecycle.rs # Alt-screen acquire/release
│       │       ├── layout.rs    # §13 region split (header / main / summary / nav)
│       │       ├── nav.rs       # Contextual keymap nav bar
│       │       ├── step_strip.rs # Header step strip
│       │       ├── summary.rs   # Summary panel
│       │       ├── support/     # Briefing inline-format (**bold**, `code`) helpers
│       │       └── phases/      # Per-phase main-area widgets: briefing, fields, confirm,
│       │                        #   enum_picker (dynamic-enum modal), json_edit, running, result, error
│       ├── presenters/          # Format-neutral presentation helpers
│       │   ├── auth.rs          # Auth-source labels, token-state views
│       │   └── service.rs       # Dry-run and API-response views
│       └── style/               # Styling subsystem
│           ├── ansi.rs          # ANSI backend: colour functions, respects NO_COLOR
│           ├── span.rs          # StyledSpan / StyledLine IR
│           ├── text.rs          # Symbol constants (✔ ✖ › …)
│           └── tone.rs          # Tone enum (semantic style vocabulary)
└── tests/                       # Integration tests — functional/, integration/, contract_input/,
                                 # contract_output/, snapshot/, security/, performance/, architecture
```

### `extend-proxy-client/` — standalone leaf crate consumed by `remote-debug connect`

A Rust port of `extend-helper-cli`'s Go tunneling client, consumed directly by the CLI invocation layer for the tunnel agent and service forwarder. See [extend-proxy-client status](CONTRIBUTING.md#extend-proxy-client-status) in CONTRIBUTING.md for its dependency guardrails and conformance-test requirements.

All project conventions, coding rules, design standards, testing, and gotchas are in [CONTRIBUTING.md](CONTRIBUTING.md). Read it before making changes.

## Quick reference

```bash
cargo test                   # All tests
cargo clippy -- -D warnings  # Lint
cargo insta review           # Accept snapshot changes
```
