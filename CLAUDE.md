# AGS CLI

A Rust CLI that dynamically generates commands from AccelByte's 24 OpenAPI 2.0 specs.

## Source layout

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
├── frontend/                    # All user-facing output
│   ├── render.rs                # Top-level CommandOutput → RenderedOutput dispatch
│   ├── templates.rs             # Backend-agnostic response templates
│   ├── presenters/              # Format-neutral presentation helpers
│   │   ├── auth.rs              # Auth-source labels, token-state views
│   │   └── service.rs           # Dry-run and API-response views
│   ├── human/                   # Human-readable frontend
│   │   ├── frontend.rs          # impl Frontend for HumanFrontend
│   │   ├── progress.rs          # Status lines and spinner helpers
│   │   ├── prompt.rs            # Interactive confirmation prompts
│   │   ├── templates.rs         # ANSI-applying text adapters
│   │   └── commands/            # Per-command human renderers
│   ├── json/                    # Machine-readable JSON frontend
│   │   ├── frontend.rs          # impl Frontend for JsonFrontend
│   │   ├── progress.rs          # Noop progress sink for JSON mode
│   │   └── commands/            # Per-command JSON emitters
│   ├── style/                   # Styling subsystem
│   │   ├── ansi.rs              # ANSI backend: colour functions, respects NO_COLOR
│   │   ├── span.rs              # StyledSpan / StyledLine IR
│   │   ├── text.rs              # Symbol constants (✔ ✖ › …)
│   │   └── tone.rs              # Tone enum (semantic style vocabulary)
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

All project conventions, coding rules, design standards, testing, and gotchas are in [CONTRIBUTING.md](CONTRIBUTING.md). Read it before making changes.

## Quick reference

```bash
cargo test                   # All tests
cargo clippy -- -D warnings  # Lint
cargo insta review           # Accept snapshot changes
```
