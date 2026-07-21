# AGS CLI Reference

Version: 0.4 RC  
Status: Release Candidate  
Scope: Normative product and engineering reference for the AGS CLI

## 1. Normative language

The key words **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and **MAY** in this document indicate requirement strength.

- **MUST / MUST NOT** indicate mandatory behavior or constraints.
- **SHOULD / SHOULD NOT** indicate strong recommendations; deviations require a deliberate reason.
- **MAY** indicates optional behavior.

## 2. Overview

The AGS CLI MUST be a unified command-line interface for AccelByte Gaming Services generated from OpenAPI specifications.

The AGS CLI MUST provide:

- a consistent command surface across supported AGS services
- deterministic, scriptable execution
- human-readable help and output
- machine-readable output for automation
- secure authentication and credential handling
- a foundation for AI-assisted operation through Skills + CLI

This release-candidate version incorporates the review feedback on authentication, AI integration framing, configuration behavior, spec sourcing, update checks, destructive confirmations, and token persistence.

## 3. Context and positioning

### 3.1 Problem statement

AccelByte has many active API services but no unified CLI. The AGS CLI MUST fill that gap by turning service specifications into a discoverable, deterministic command surface for humans, shells, CI systems, and AI-assisted execution.

The Google Workspace CLI (`gws`) demonstrates a strong pattern for this model: dynamically generating commands from specs and layering higher-level workflows or skills on top. The AGS CLI SHOULD follow that general architecture while adapting it to AccelByte APIs and product constraints.

### 3.2 AI integration framing

The AGS MCP server and the AGS CLI MUST be described as complementary approaches to AI integration, not as direct product substitutes.

The correct comparison is:

1. **AI integration via MCP server**
2. **AI integration via Skills + CLI**

The AGS CLI specification MUST NOT frame this as a broad “MCP vs CLI” comparison, and it MUST NOT describe MCP as IDE-bound. MCP can work with any AI client, GUI or shell-based, that supports the protocol.

### 3.3 AI integration comparison

| Dimension | AI Integration via MCP Server | AI Integration via Skills + CLI |
|---|---|---|
| Primary interface | Tool protocol | Shell commands plus skill documentation |
| Execution model | AI invokes exposed tools directly | AI invokes explicit CLI commands |
| Deployment model | Can be remote/SaaS or local | Local binary in the execution environment |
| Client requirements | Any AI client that supports MCP | Shell access plus optional skill files |
| Authn/Authz integration | Often needs MCP-specific setup; DCR is required in many cases | Reuses CLI auth model and host environment credentials |
| Repeatability | Weaker: exact tool-call sequences are harder to replay | Stronger: commands are copyable, scriptable, reviewable, and version-controllable |
| Human usability | Indirect | Direct |
| CI/CD usability | Indirect | Strong |
| Operational troubleshooting | Split across client/server/tool boundaries | Centered in one command surface |
| Discoverability | AI searches and describes tools dynamically | `ags describe`, `--help`, completions, `--dry-run`, and generated skill docs |

### 3.4 Positioning

The AGS CLI SHOULD be positioned as:

> A deterministic, scriptable operations surface for AccelByte Gaming Services, usable directly by humans and indirectly by AI agents through Skills + CLI.

### 3.5 Repeatability

Repeatability SHOULD be treated as the CLI’s strongest advantage in the AI integration discussion.

A CLI command can be copied into a runbook, committed to a repository, replayed in CI, reviewed in code review, and shared in incident documentation. MCP interactions are generally harder to replay exactly. This point SHOULD be emphasized more strongly than in earlier drafts.

### 3.6 Remote MCP deployment

Remote MCP deployment SHOULD be acknowledged as having real operational benefits, including reduced client-side install friction, easier rollout, and less troubleshooting of local runtime setup. The CLI specification SHOULD describe these benefits fairly while still explaining the strengths of Skills + CLI.

## 4. Goals and non-goals

### 4.1 Goals

The AGS CLI MUST:

1. expose a unified CLI across supported AGS services
2. generate service, resource, and method command trees from OpenAPI specs
3. work well for humans, shell automation, CI, and AI-assisted execution
4. provide deterministic execution with clear validation and safety rails
5. support secure credential and token handling
6. support both human-readable and machine-readable output
7. support future layering of skills and workflows

### 4.2 Non-goals

The AGS CLI MUST NOT be defined as:

- a replacement for the AGS MCP server
- a conversational assistant
- a full client-side schema validator for every business rule
- a complete workflow engine in the initial implementation
- a dynamic service-discovery client unless stable backend discovery endpoints exist

## 5. Supported services and source of truth

The AGS CLI MUST generate commands from the supported AGS OpenAPI 2.0 specifications.

The CLI MUST validate requested services against an explicit allowlist (the `manifest`) before loading bundled spec artifacts. The current allowlist contains 24 services drawn from the AccelByte Go SDK.

## 6. Technology stack

The initial implementation SHOULD use:

- **Rust stable**
- **clap** for CLI argument parsing
- **reqwest** for HTTP
- **serde / serde_json** for data handling
- **keyring** or equivalent for OS keychain integration
- **directories** for platform-appropriate config/cache paths
- **flate2** for bundled spec decompression
- **insta** for snapshot testing
- **wiremock** or equivalent for HTTP mocking

The CLI SHOULD target the same language ecosystem as `gws` to reduce architectural drift and reuse patterns where helpful.

## 7. Architecture

### 7.1 Two-phase argument parsing

The CLI MUST use two-phase parsing:

1. pre-scan argv to identify global flags and the target service
2. load the corresponding spec
3. construct the dynamic command tree
4. re-parse the full argv
5. resolve and execute the operation

This model MUST allow global flags to appear before or after the service name where practical.

### 7.2 OpenAPI to CLI mapping

The CLI MUST derive command trees from OpenAPI operations using:

- service-level grouping
- resource grouping by tag or canonical mapping
- method names derived from `x-operationId` decomposition (see §9.1)
- per-operation parameter mapping to flags and arguments

The implementation MUST preserve stable naming wherever possible so scripts do not churn unexpectedly.

### 7.3 Core runtime data

The runtime SHOULD maintain structures equivalent to:

- a service registry
- loaded spec metadata
- command tree definitions
- normalized operations
- auth/session state
- config state
- output-format state

The exact Rust type names MAY vary, but the functional roles MUST remain.

## 8. Discovery and spec sourcing

### 8.1 Source model

The CLI uses bundled specs only. It does not fetch specs from the network.

The CLI MUST:

1. load bundled gzip-compressed specs from the release binary
2. parse those specs into cached service definitions on demand
3. reuse cached parsed definitions on subsequent runs
4. treat bundled spec corruption as a hard error

### 8.2 Cache semantics

The cache stores parsed service definitions keyed by service name on disk under a structure equivalent to:

```text
<cache_dir>/<service>.json
```

Where `<cache_dir>` is the platform-specific cache directory (see §11.3).

Bundled-spec corruption MUST be treated as a hard error.

### 8.3 Refresh behavior

`ags refresh-specs` MUST clear cached parsed service definitions and rebuild them from the bundled specs.

## 9. Naming and filtering

### 9.1 Command names

Command names MUST be derived from each operation's `x-operationId` using a deterministic `service/scope/resource/version/method` decomposition.

The decomposition MUST:

- treat `x-operationId` as the single source of truth once assigned
- not rewrite or normalize values further at command-build time
- map cleanly onto the CLI command tree (service then resource then method)
- surface collisions as authoring errors rather than silently disambiguating

The `scope` and `version` segments populate the per-command contract matrix consumed by `--api-scope` / `--api-version` (see §10.11). Operations marked deprecated and operations under the `internal` resource MUST be excluded from the generated command surface.

### 9.2 Resource grouping

Resource grouping SHOULD use tags where they are reliable. Where tags are inconsistent, the implementation MAY use curated mappings to produce stable, readable resource names.

### 9.3 Filtering rules

The CLI MUST filter out operations only for explicit reasons such as:

- unsupported or invalid specs
- intentionally blocked administrative operations
- duplicate or conflicting operation surfaces that cannot yet be represented safely

Filtering MUST be deterministic and documented.

### 9.4 Destructive operations

DELETE operations MUST be generated as valid CLI commands.

The CLI SHOULD also identify selected risky PUT, PATCH, or POST mutations as destructive or confirmation-required where the semantic effect is equivalent to a destructive action.

## 10. CLI behavior

### 10.1 Output formats

The CLI MUST support human-readable output by default and structured output for automation.

**Human-readable output** is the default and is designed for interactive terminal use. Human-readable output format is NOT part of the output contract and MAY change between releases without notice. Scripts and CI pipelines MUST NOT parse human-readable output.

**Machine-readable output** (`--format json`) produces stable, structured output intended for automation and scripting. The JSON output structure IS part of the output contract — fields MUST NOT be removed or renamed without a major version bump. New fields MAY be added.

`--format json` MUST work across all commands including auxiliary commands (`auth`, `version`) and service commands.

**Channel routing:** The primary result or data MUST go to stdout. Diagnostics, progress, errors, and human guidance (fix suggestions, tips) MUST go to stderr. In JSON mode, only the JSON object goes to stdout; stderr receives progress and prompts only where necessary (e.g. during interactive login).

The structured format is `json`. The default human-readable output renders tables for list operations.

**Presentation backend (`--ui`):** selects how human output is presented — `auto` (default), `plain` (line-oriented), `inline` (in-cursor viewport), or `fullscreen` (alternate-screen). `auto` resolves the surface from the command and terminal. `--ui` affects presentation only; when `--format json` is also present, `--ui` is silently ignored — the JSON machine contract wins (it is not an error to pass both).

**Schema passthrough:** Request body data passed via `--json` is sent to the backend as-is; the CLI performs no client-side schema validation (the backend is the authority). The `--skeleton` global flag outputs a fillable JSON request body template for any operation that accepts `--json`, showing field names, types, and required/optional status. `--skeleton` requires no auth and makes no API call.

**Output destination (`--output <path>`):** redirects the primary stdout payload to a file. `--output -` is an explicit alias for stdout. When the response body is binary (e.g. exported asset, save file), `--output` is required for terminal use; piping (non-TTY) is also accepted. `--output` only affects the primary payload — diagnostics, progress, and errors continue to go to stderr.

### 10.2 Pagination

The CLI supports:

- `--page-all` for iterative traversal of paginated endpoints
- `--page-limit <N>` to cap the number of pages fetched (default 10, max 100)
- pagination metadata in structured outputs when useful

### 10.3 Pager behavior

The CLI MAY integrate with a pager for long human-readable output. Pager behavior MUST be suppressible in non-interactive and machine-readable contexts.

### 10.4 Help system

The CLI MUST provide a four-level help hierarchy:

1. `ags --help`
2. `ags <service> --help`
3. `ags <service> <resource> --help`
4. `ags <service> <resource> <method> --help`

Help text MUST be generated from the command model and MUST remain consistent with the currently supported auth and configuration behavior.

Help text MUST NOT contain password-grant examples.

For commands that resolve to a contract (see §10.11), `--help` MUST render the default resolved contract — not an abstract summary that forces a second help lookup. Help MUST refine progressively as `--api-scope` and `--api-version` selectors are supplied. The cross-scope and cross-version matrix is exposed through `ags describe`, not duplicated in human help.

The CLI MUST provide an `ags describe` command for machine-readable command discovery and introspection, following the same four-level hierarchy as human help:

1. `ags describe` — catalogue of all available services
2. `ags describe <service>` — service detail including resources and summary
3. `ags describe <service> <resource>` — resource detail including methods and summary
4. `ags describe <service> <resource> <method>` — full method introspection including inputs, examples, and execution semantics

`ags describe` output MUST always be JSON, optimized for AI and tooling consumption rather than terminal rendering. It MUST support a `discover → introspect → execute` workflow: an agent discovers available commands, introspects a specific method, and executes it using the structured metadata.

`ags describe` answers "what commands exist and how do I call them?" while the `--skeleton` flag answers "what data do I send?" by outputting a fillable JSON request body template for operations that accept `--json` (see §10.1).

Beyond the service hierarchy, `ags describe` also exposes registered workflows:

5. `ags describe workflow` — JSON catalogue of registered workflows
6. `ags describe workflow <id>` — full workflow introspection

The root catalogue (`ags describe`) lists a `workflow` node (`node_type: "workflow-catalogue"`) alongside the services. `ags describe workflow <id>` returns an envelope with `kind: "workflow"` whose `data` carries `id`, `name`, `intent`, `description`, `inputs[]` (each: `name` kebab-cased, `type`, `enum_values`, `required`, `default`, `description`, `sensitive`, `dynamic`), and `steps[]` (each: `id`, `service`, `operation`, `description`, `dependencies[]`). An unknown id returns `kind: "error"` with code `unknown_workflow` (plus name suggestions, exit 1); a path segment beyond `<id>` returns code `invalid_workflow_path` (exit 1). The full set of `ags describe` envelope `kind` values is therefore `catalogue`, `command`, `error`, and `workflow`. Consumers MUST branch on `kind` rather than assume a uniform `children`-bearing shape; the envelope contract — every `kind`, its `data` payload, the recursive-walk pattern, and versioning rules — is specified in the Output Reference, §27 "Describe envelope contract".

Method-level `ags describe` output MUST expose the full scope/version contract matrix for the command. The shape MUST include:

- `command` — fully qualified command path
- `default_scope` — scope used when `--api-scope` is omitted
- `scopes` — map of scope name to `{ default_version, supported_versions, contracts }`
- per-contract metadata: HTTP method, path template, parameters, request/response shape, permissions, deprecation marker

Deprecated contracts are excluded upstream and therefore do not appear in `supported_versions` or in the `contracts` map.

### 10.5 Auxiliary commands

The CLI SHOULD include auxiliary commands such as:

- `ags auth login`
- `ags auth logout`
- `ags auth status`
- `ags auth refresh`
- `ags config get`
- `ags config set`
- `ags config unset`
- `ags profile list`
- `ags profile create`
- `ags profile use`
- `ags profile show`
- `ags profile delete`
- `ags profile rename`
- `ags describe`
- `ags describe workflow` / `ags describe workflow <id>`
- `ags doctor`
- `ags refresh-specs`
- `ags completions`
- `ags version`
- `ags workflow run`

### 10.5.1 `ags workflow run`

Synopsis:

```text
ags workflow run <workflow-id> [--<input> <value>]…
```

`ags workflow run` executes a registered multi-step workflow by its id. Each workflow declares its own `--<input>` flags; the exact flags vary by workflow.

**Input flags**

Each workflow contributes one optional `--<kebab-case-name>` flag per declared input. Flag names are the camelCase input names converted to kebab-case (e.g. `sessionDeployment` becomes `--session-deployment`, `fleetImageId` becomes `--fleet-image-id`). Inputs with defaults are optional on the command line; required inputs without defaults must be supplied via their flag, via gather prompts, or through the `--namespace` shortcut described below.

**Help**

```text
ags workflow run <workflow-id> --help
ags workflow run --help <workflow-id>
```

`--help` is recognised on either side of the id. With a concrete id it compiles the workflow and prints the full `--<input>` flag list with descriptions. Without an id it prints the generic usage line.

**Global flag interactions**

- `--dry-run` — builds per-step request previews (URL, headers, body) without calling the API. All six steps of `competitive-multiplayer` produce previews; the output is a `CommandOutput::WorkflowDryRun` envelope.
- `--no-input` — refuses to gather or prompt for missing inputs; fails with an aggregated error if any required input is absent.
- `--skeleton` — rejected; `ags workflow run` does not support skeleton output. The rejection fires before any registry lookup.
- `--format json` — runs the workflow non-interactively and emits a JSON envelope to stdout (see `output-reference.md` §25.3 for the canonical shapes). All inputs MUST be supplied via `--<flag>`s — JSON mode never prompts (it behaves as if `--no-input` were set), so a missing required input fails immediately rather than gathering. `--yes` is required for any confirm-gated step unless `--dry-run` is active (which is exempt from confirmation). `--format json` does NOT imply `--yes`. A single-step workflow that declares no outputs collapses to the bare API response body. Any failure (e.g. a missing required input) emits a JSON error envelope on stderr with the underlying exit code (see Exit codes below).
- `--namespace <value>` — the global namespace flag feeds a `namespace` workflow input when the workflow declares one and no per-workflow `--namespace` was explicitly provided.
- `--output <path>` — threaded into each step's request so the last step's raw response body is written to the specified file or stdout.
- `--verbose` — increases response verbosity for each step.
- `--yes` — bypasses per-step confirmation prompts declared with `confirm: true`.

**Run mode (interactive surfaces only)**

On the interactive fullscreen and inline surfaces, the run-start gather offers a three-button run-mode choice that controls how often the run pauses. It does not apply to `--yes`, `--no-input`, or `--format json` (those never pause for review), and the default is chosen when the user just proceeds, so existing behaviour is unchanged.

- **Run** (default) — pause only on steps that have a field to review or gather; fully auto-bound steps run without a pause.
- **Run & Review** — pause before every reviewable step so the user can review it before it runs.
- **Run & Accept Defaults** — no review pauses; runs straight through on each step's default and bound values, gathering only a genuinely-missing required input. Destructive `confirm: true` steps still prompt (unless `--yes`), matching plain-mode semantics.

The mode governs only the per-step *review* pause; the `confirm: true` safety gate is independent and unaffected.

**Skipping optional steps (interactive surfaces only)**

A step the workflow marks as optional can be skipped at its review or confirm gate on the fullscreen and inline surfaces: press `s`, or move focus to the **Skip** button and submit. Skipping runs no request for that step and continues to the next one, so the step's required-input validation is bypassed (the step is not executed). Non-optional steps offer no skip affordance. Optional steps are always paused for review even under **Run** — they are never silently auto-run — so the skip choice is always reachable. Skipping is unavailable in `--no-input` and `--format json` runs, which never pause; those runs execute every step, optional ones included.

**Handling step failures**

When a step's API call fails, the behaviour depends on the surface and the error:

- **Already-exists conflicts auto-skip (all surfaces, all modes).** A step the workflow marks as `skip_if_exists` that fails with an HTTP 409 whose error identifies it as an "already exists" conflict is skipped automatically, with no prompt, and recorded in the run summary as skipped. This makes a re-run idempotent for those steps. Only the specific already-exists conflict is treated this way; any other error on the step (including a different 409) is a real failure and follows the rules below.

- **Interactive surfaces pause with a failure gate.** On the fullscreen, inline, and plain surfaces, any other step failure pauses the run and offers **Retry** / **Skip** / **Cancel** instead of aborting. **Retry** re-runs the same step. **Skip** is offered only when the step is safely skippable — every output it captures has a default, so skipping cannot leave a later step's reference unresolved — otherwise only **Retry** / **Cancel** are shown. **Cancel** ends the run with the failure (the underlying error class sets the exit code). Retrying is user-driven and unbounded; there is no automatic retry.

- **Non-interactive runs fail fast.** `--no-input` and `--format json` runs never show the gate: any non-auto-skipped step failure is fatal, exactly as before. (The already-exists auto-skip above still applies, since it needs no prompt.)

**Exit codes**

| Code | Meaning |
|------|---------|
| 0 | Every step succeeded |
| 1 | Usage / input error — missing required input, unknown workflow, or `--skeleton` rejected |
| 2 | Auth / authorization failure, or the user declined a confirmation prompt |
| 3 | An AccelByte API call returned an error |
| 4 | Network / transport failure |
| 5 | Unexpected internal error |

A failing step propagates the underlying error's class (codes 2–5); exit 1 is reserved for usage/input errors detected before or during input gathering.

### 10.5.2 `competitive-multiplayer` workflow

Stand up competitive matchmaking with dedicated servers: a skill stat, a match ruleset, a session template, a match pool, an AMS fleet, and the session template wired to the fleet. Matches start only when teams are exactly full (no under-filled sessions) — appropriate for ranked play.

**Input flags**

| Flag | Required | Default | Description |
|------|----------|---------|-------------|
| `--namespace` | yes | — | Game namespace all resources are created in. |
| `--players-per-team` | | `4` | Players per team (symmetric). |
| `--team-count` | | `2` | Number of teams (default 2 for X v X). |
| `--fleet-image-id` | yes | — | AMS image ID to deploy (dynamic — picked from `ams images list`). |
| `--fleet-region` | yes | — | Region the fleet runs in (dynamic — picked from `ams info list-regions`). |
| `--fleet-instance-id` | yes | — | AMS instance UUID, `dsHostConfiguration.instanceId` (dynamic — picked from the AMS instances list). |
| `--stat-code` | | `mmr` | Skill stat code. |
| `--resource-prefix` | | `ranked` | Prefix for ruleset/session/pool/fleet names and claim key. |

The three fleet inputs are **dynamic enums**: in the fullscreen surface they render as type-to-filter pickers populated from a live AMS lookup, while non-interactive and `--format json` runs treat them as plain string flags (no default — the value must be supplied).

**Breaking change from earlier branches:** the previous flag set (`--deployment`, `--image-deployment-profile`, `--session-template-name`, `--ruleset-name`, `--match-pool-name`, `--fleet-name`) is removed. Existing scripts targeting the old contract require updating.

### 10.5.3 `season-pass` workflow

Build a complete, publishable **season pass** in an existing draft store: a `/Season` category, a free and a premium SEASON pass item plus a tier item, then publish the store, create the season with a free and a premium pass, six item rewards across both tracks, and three tiers, and publish the season. Deliberately opinionated — locale is fixed to `en-US`, prices are set for the **US** region only, and the SEASON items are priced in a chosen virtual currency.

**Input flags**

| Flag | Required | Default | Description |
|------|----------|---------|-------------|
| `--namespace` | yes | — | Game namespace all resources are created in. |
| `--store-id` | yes | — | Draft store the items and category are created in (dynamic — picked from `platform stores list`). |
| `--currency-code` | yes | — | Virtual currency the SEASON items are priced in (dynamic — picked from the namespace's VIRTUAL currencies). |
| `--free-reward-item-id` | yes | — | In-game item granted on the free reward track (dynamic — picked from the store's items). |
| `--premium-reward-item-id` | yes | — | In-game item granted on the premium reward track (dynamic — picked from the store's items). |
| `--season-name` | | `Season 1` | Display name of the season. |
| `--start` | | `2020-01-01T00:00:00Z` | Season start (ISO 8601). The default is in the past, so the season auto-starts on publish. |
| `--end` | | `2099-12-31T23:59:59Z` | Season end (ISO 8601). |

The four id/code inputs are **dynamic enums**: in the fullscreen surface they render as type-to-filter pickers populated from live `platform` lookups, while non-interactive and `--format json` runs treat them as plain string flags (the value must be supplied).

`--start` and `--end` are **date-time** inputs: on the interactive surfaces they render as a numbers-only UTC segment editor (year / month / day / hour / minute) rather than a raw ISO field. Arrow keys move between and adjust segments, digits type a value, enter commits, and esc cancels; while a segment is being edited those keys take precedence over field navigation. The value stored and sent is still the ISO 8601 string (minute precision, seconds `00`), so request previews (`--dry-run`, `--format json`) show the raw `…T…Z` form. Non-interactive runs treat them as plain string flags.

The two publish steps — publishing the store and publishing the season — are both marked optional, so on the interactive surfaces they can be skipped at their gate (see §10.5.1, "Skipping optional steps"). Left un-skipped, the final step publishes the season and it is live for players when the run finishes; because the default `--start` is in the past, the season auto-starts. Skip the publish steps (or pass a future `--start`) to leave the season unpublished and editable. **Prerequisite:** a draft store with a virtual currency and at least one in-game item — run the `in-game-store` workflow first if needed.

### 10.5.4 `in-game-store` workflow

Build a structured in-game store in a namespace's draft store: a draft store, a virtual soft currency, a root category with durable and consumable sub-categories, one durable and one consumable item, then publish the draft store so the catalogue goes live. Locale and region are fixed to `en-US` / `US`, and the items are priced in the created currency.

**Input flags**

| Flag | Required | Default | Description |
|------|----------|---------|-------------|
| `--namespace` | yes | — | Game namespace all resources are created in. |
| `--currency-code` | | `GOLD` | Code of the virtual soft currency to create; items are priced in it. |

**Re-running is idempotent for the create steps.** The six create steps (currency, three categories, two items) are marked `skip_if_exists`: on a re-run, each auto-skips its already-exists 409 and the run continues (see §10.5.1, "Handling step failures"). The `create-store` step is **not** — a draft store is a per-namespace singleton whose id later steps depend on, so if it already exists its 409 surfaces at the interactive failure gate (Retry / Cancel; Skip is not offered) and is fatal in non-interactive runs.

**The `publish` step is optional.** It is marked optional and confirm-gated, so on the interactive surfaces it can be skipped at its confirm gate (see §10.5.1, "Skipping optional steps") to leave the store built but unpublished. A publish conflict is **not** auto-skipped — it is a real failure that surfaces at the gate.

### 10.6 Error handling

Errors MUST be actionable and SHOULD point users to likely fixes or next steps.

The CLI SHOULD align its human-readable error formatting with the separate CLI output style specification where that document exists.

### 10.7 Dry run

The CLI SHOULD support `--dry-run` where request construction can be shown meaningfully without executing the mutation.

### 10.8 Verbose and quiet modes

The CLI SHOULD support:

- `--verbose` for expanded diagnostics
- `--quiet` for minimal output

### 10.9 Non-interactive mode

The CLI MUST support non-interactive execution in CI and automation scenarios.

When prompts are impossible or disallowed, the CLI MUST fail with actionable guidance unless the required data or confirmations were provided explicitly.

### 10.10 Color and terminal output

Color MAY be supported, but it MUST NOT be the sole carrier of meaning.

### 10.11 Scope and version resolution

Generated service commands resolve to a **contract** — the combination of command, scope, and API version. Scope and version are selected by flag, not by command-path or subcommand structure, so the user thinks action-first.

#### 10.11.1 Terminology

- **Scope** — the endpoint audience for a command. Initially `admin` and `public`. Additional scopes MAY be added later.
- **Version** — a command-scoped API contract identifier such as `v1`, `v2`. Versions are defined per command and MAY vary independently across commands and scopes. A version is NOT a global platform-wide API version.
- **Contract** — the resolved `(command, scope, version)` triple, which determines valid arguments, help text, endpoint mapping, and request/response behavior.

#### 10.11.2 Design principles

- The CLI MUST default to the `admin` scope unless a command explicitly defines otherwise. AGS CLI is developer-first.
- Scope MUST be selected via `--api-scope <scope>`, not by scope-first command paths.
- Version MUST be selected via `--api-version <vN>`, not by version subcommands.
- Versioning MUST be command-scoped: `--api-version vN` means "use the `vN` contract for this command and resolved scope," not "use platform API version `vN`."
- Omitting `--api-version` MUST resolve to the CLI-defined default version for the command and scope. The default MUST be owned by the CLI contract model, not by whatever endpoint version is "latest" at runtime.
- The CLI MUST NOT require a mutable global "admin mode" / "public mode". Persistent config MAY set a default scope, but command interpretation MUST remain explicit and deterministic.

#### 10.11.3 Scope flag

Commands that support multiple scopes MUST accept `--api-scope <scope>` with at least `admin` and `public` as supported values.

If `--api-scope` is omitted, the CLI MUST resolve scope as: command-specific default if defined, otherwise `admin`.

The CLI SHOULD prefer `--api-scope public` over boolean flags such as `--public`, because `--api-scope` scales as more scopes are introduced.

If a user specifies an unsupported scope, the CLI MUST fail with an error indicating the requested scope, the command, and the supported scopes:

```text
error: scope 'public' is not supported for 'ags iam users create'
Supported scopes: admin
```

#### 10.11.4 Version flag

Commands that support multiple versions MUST accept `--api-version <version>`, e.g. `--api-version v2`. The `v` prefix is optional; bare numerics such as `2` are accepted.

If `--api-version` is omitted, the CLI MUST resolve the default version for the selected command and scope. That default version is the active contract for execution, help rendering, and argument validation.

If `--api-version vN` is provided, the CLI MUST use that exact contract for the resolved scope. This is the supported way to pin behavior for automation.

The CLI MUST NOT support `--api-version latest` as a normal user-facing value. It undermines pinning, blurs the distinction between omitted and explicit-latest behavior, and encourages unstable automation.

Deprecated versions MUST NOT be selectable via `--api-version`. Deprecated contracts are excluded from the catalogue entirely and therefore appear in neither the supported-version list nor help-rendered allowed values.

If a user specifies an unsupported version, the CLI MUST fail with an error indicating the command, scope, requested version, and supported versions for that scope:

```text
error: api version 'v1' is not supported for 'ags iam users get' with --api-scope public
Supported public versions: v2, v3
```

#### 10.11.5 Resolution model

Each invocation MUST be conceptually resolved in this order:

1. identify command
2. resolve scope (explicit `--api-scope`, then command default, then `admin`)
3. resolve version (explicit `--api-version`, then default for the resolved scope)
4. resolve concrete contract
5. validate arguments against that contract
6. dispatch to the mapped endpoint implementation

Different contracts MAY define different valid arguments. The CLI MUST validate arguments against the resolved contract, not against the abstract parent command.

Because scope and version can change the valid argument set, the CLI MAY use lightweight pre-scan extraction of `--api-scope` / `--api-version` followed by contract-specific dynamic parse and validation.

#### 10.11.6 Help semantics

Help MUST resolve defaults the same way normal execution resolves defaults. If `ags <service> <resource> <method> 123` would resolve to `(scope=admin, version=v4)`, then `ags <service> <resource> <method> --help` MUST render help for that exact contract.

Help MUST progressively refine as selectors are supplied:

- no selectors → help for the default resolved contract
- `--api-scope public` → help for the default version within `public`
- `--api-scope public --api-version v2` → help for that exact contract

When a command has more than one scope, the rendered `--api-scope` option MUST list the possible values. When a command has more than one version in the resolved scope, the rendered `--api-version` option MUST list the possible values. When a command has exactly one scope and one version, both flags MUST be omitted from help.

Help SHOULD include the resolved default scope and version, usage for the resolved contract, arguments valid for that contract, allowed values for `--api-scope` / `--api-version` where applicable, and examples.

When the pre-scanned selectors at help time fail to resolve to a real contract, the CLI MUST fall back to the default contract for help rendering. Execution-time invalid selectors still produce the errors in §10.11.3 / §10.11.4.

#### 10.11.7 Regular command feel

Generated commands SHOULD feel:

- action-first
- unversioned by default
- admin by default
- optionally narrowed by `--api-scope`
- optionally pinned by `--api-version`

Examples:

```text
ags iam users get 123
ags iam users get 123 --api-scope public
ags iam users get 123 --api-version v4
ags iam users get 123 --api-scope public --api-version v3
```

For automation, users SHOULD pin both `--api-scope` and `--api-version` to protect scripts from future default shifts.

#### 10.11.8 Contract-aware errors

Errors related to argument validity, version support, or scope support SHOULD be expressed in terms of the resolved contract and SHOULD mention the command, scope, version, and what values are supported.

```text
error: --tenant is not supported for 'ags iam users get' with --api-scope public --api-version v3
Try:
  ags iam users get --api-scope public --help
```

## 11. Configuration

### 11.1 Configuration scopes

The CLI MUST support two configuration scopes:

1. **global config**
2. **profile config**

Global config MUST store CLI-wide behavior.

Profile config MUST store environment-specific AGS context.

This split exists so users can work across multiple AGS environments such as `dev`, `staging`, and `prod` without repeatedly rewriting environment-specific settings.

### 11.2 Profiles

The CLI MUST support multiple named profiles.

The CLI MUST support one active profile used by default when `--profile` is not specified.

The selected profile SHOULD be resolved in this order:

1. `--profile <name>`
2. `AGS_PROFILE`
3. global config `active_profile`
4. built-in default profile name (such as `default`) used as a fallback when no explicit profile is configured
5. error

### 11.3 Config directory

The CLI MUST store state in platform-appropriate directories using platform-idiomatic names:

| Concern | Linux | macOS | Windows |
|---------|-------|-------|---------|
| Config | `$XDG_CONFIG_HOME/ags/` or `~/.config/ags/` | `~/Library/Application Support/com.accelbyte.ags/` | `%APPDATA%\AccelByte\AGS\` |
| Data (tokens, secrets) | `$XDG_DATA_HOME/ags/` or `~/.local/share/ags/` | `~/Library/Application Support/com.accelbyte.ags/` | `%APPDATA%\AccelByte\AGS\` |
| Cache (parsed specs) | `$XDG_CACHE_HOME/ags/` or `~/.cache/ags/` | `~/Library/Caches/com.accelbyte.ags/` | `%LOCALAPPDATA%\AccelByte\AGS\` |

When `AGS_HOME` is set, all three concerns collapse under that single directory. This is the primary mechanism for test isolation and CI environments.

### 11.4 Primary config file

The primary global config file MUST be `config.json`.

### 11.5 Global config

Global config SHOULD contain settings that describe how the CLI behaves, not which AGS environment it targets.

Recommended global config keys include:

- `active_profile` — name of the profile used when `--profile` is omitted
- `format` — default for `--format`
- `no_color` — default for `--no-color`
- `timeout` — default for `--timeout`, in seconds
- `page_limit` — default for `--page-limit`

Global config MUST NOT be used to store environment-specific auth state.

### 11.6 Profile config

Profile config MUST contain settings that are specific to a target AGS environment or auth context.

Recommended profile-scoped keys include:

- `base_url`
- `namespace`
- `client_id`
- `grant_type`

Profile-scoped auth material includes:

- `client_secret`
- `access_token`
- `refresh_token`
- `expires_at` — token metadata (not a secret) stored alongside tokens for operational convenience

Environment-specific values MUST be isolated per profile.

### 11.7 Profile isolation

The CLI MUST NOT treat profiles as labels over one shared credential store.

Each profile MUST have isolated:

- base URL
- namespace
- client ID
- grant type
- secret state
- token state

Authenticating one profile MUST NOT overwrite another profile’s auth state.

### 11.8 Configuration precedence

After the profile is resolved, configuration values SHOULD be resolved according to scope.

For **profile-scoped keys** such as `base_url`, `namespace`, and `client_id`, the CLI SHOULD use this precedence:

1. CLI flag
2. environment variable
3. selected profile config
4. built-in default if one exists
5. error

For **global keys** such as `format` and `color`, the CLI SHOULD use this precedence:

1. CLI flag
2. environment variable
3. global config
4. built-in default
5. error if required

### 11.9 Config commands

The CLI SHOULD provide `profile` commands and `config` commands.

`profile` commands SHOULD manage profile lifecycle.

Recommended profile commands:

- `ags profile list`
- `ags profile create <name>`
- `ags profile use <name>`
- `ags profile show [name]`
- `ags profile delete <name>`
- `ags profile rename <old> <new>`

`config` commands SHOULD manage key/value configuration.

Recommended config commands:

- `ags config get`
- `ags config set`
- `ags config unset`

The CLI SHOULD support:

- `--global`
- `--profile <name>`

for `config` operations.

If neither is given:

- profile-scoped keys SHOULD operate on the resolved active profile
- global-only keys SHOULD operate on global config
- ambiguous keys SHOULD fail with an actionable error

### 11.11 Storage model

The CLI MUST preserve the distinction between global and profile scope in storage.

A recommended model is:

- global config stored in `config.json`
- profile-scoped non-secret config stored per profile
- profile-scoped secrets and tokens stored in OS keychain when available
- profile-scoped fallback credential storage used only when keychain is unavailable

### 11.12 Profile-related errors

The CLI SHOULD produce actionable errors for profile-related failures.

Examples include:

- no active profile configured
- unknown profile name
- required profile value missing
- auth state missing for the selected profile


## 12. Authentication

### 12.1 OAuth endpoints

The CLI MUST target the following AGS IAM OAuth endpoints:

```text
GET  {base_url}/iam/v3/oauth/authorize   (authorization-code: browser redirect)
POST {base_url}/iam/v3/oauth/token        (authorization-code: code exchange; client-credentials: token fetch; refresh)
```

### 12.2 Supported grant types

`ags auth login --grant` MUST support exactly these values:

- `authorization-code`
- `client-credentials`

Password grant MUST be removed entirely from the specification, command help, examples, storage model, and implementation plan.

### 12.3 Grant naming

The CLI SHOULD expose the exact grant names `authorization-code` and `client-credentials` rather than aliases such as `user`, because the explicit names are clearer in help text, scripts, and troubleshooting.

### 12.4 Authorization-code login

For `--grant authorization-code`, the CLI MUST:

- initiate an interactive browser-based OAuth flow
- use a local callback listener or equivalent interactive completion mechanism
- exchange the authorization code for tokens
- document clearly that this flow is interactive

The CLI MUST NOT claim support for fully headless `--no-input` execution of authorization-code login unless a future non-browser flow is implemented.

### 12.5 Client-credentials login

For `--grant client-credentials`, the CLI MUST support:

- interactive secret entry
- stdin-based secret input
- environment-variable-driven automation

Passing secrets by flag MAY be supported for compatibility, but it SHOULD be treated as insecure and SHOULD emit a warning.

### 12.6 Token persistence

Auth state MUST be profile-scoped.

Commands such as `ags auth login`, `ags auth status`, and `ags auth logout` SHOULD accept `--profile <name>`. If `--profile` is omitted, they SHOULD use the resolved active profile.

The CLI MUST persist credential material and tokens according to this rule:

1. store sensitive values in the OS keychain when available
2. if OS keychain is unavailable, fall back to config-backed fallback storage

This rule applies to:

- client secret
- access token
- refresh token

### 12.7 Refresh token support

The CLI MUST support refresh tokens for flows that return them.

When a refresh token is available, the CLI MUST use it to renew access tokens when needed.

### 12.8 Token lifecycle

The CLI SHOULD check token freshness before request execution and before each step of multi-step flows such as workflows or `--page-all`.

If refresh fails, the CLI MUST stop and return an actionable auth error.

### 12.9 Access token persistence rationale

The older in-memory-only token model is not sufficient for a standalone CLI because separate invocations do not share memory. Persisting both access and refresh tokens SHOULD therefore be the default model.

### 12.10 Environment override

The CLI SHOULD support environment-driven auth override for CI and AI-assisted execution.

This includes profile selection, such as:

```text
AGS_PROFILE=staging
AGS_ACCESS_TOKEN=...
AGS_BASE_URL=https://demo.accelbyte.io
AGS_CLIENT_ID=...
AGS_CLIENT_SECRET=...
```

### 12.11 Auth resolution order

Auth resolution SHOULD follow this order, applied after profile resolution:

1. explicit access token environment variable, if supported
2. stored credentials and tokens from prior login
3. credentials (environment variables take priority over stored config) — fetch a fresh token
4. actionable error

### 12.12 Auth status

`ags auth status` SHOULD report:

- base URL
- client ID
- current or last-used grant type, where meaningful
- whether access token is present
- whether refresh token is present
- token expiry, when known
- relevant token claims, when safely decodable

### 12.13 Auth logout

`ags auth logout` MUST clear grant-specific stored auth material for the selected profile from the OS keychain when used, and from config-backed fallback storage when fallback is in use.

`ags auth logout --all` MUST clear credentials from every profile. It MUST iterate all profile directories, clearing keychain entries and fallback files for each. `--all` and `--profile` MUST be mutually exclusive.

## 13. Security

### 13.1 Sensitive data output

The CLI MUST NOT print raw access tokens, refresh tokens, or client secrets in normal output.

### 13.2 Credential storage

The CLI MUST prefer OS keychain for secrets and tokens.

If config-backed fallback storage is used:

- the fallback MUST be documented as less secure than keychain-backed storage
- file permissions MUST be restrictive
- fallback storage MUST exist to preserve CLI usability where keychain is unavailable

Fallback storage MUST NOT be used for unrelated keychain operational failures.

### 13.3 Config permissions

Files under the CLI config directory SHOULD use restrictive permissions such as `0600`, and the containing directory SHOULD use restrictive permissions such as `0700`, subject to platform differences.

### 13.4 Spec integrity

The CLI SHOULD validate downloaded spec content enough to detect malformed or corrupt cache entries and SHOULD recover by re-fetching where possible.

### 13.5 `.env` guidance

Even when `.env` reading is enabled, the CLI SHOULD discourage storing secrets in `.env`.

## 14. Destructive-operation safety

### 14.1 Confirmation policy

The CLI SHOULD require confirmation not only for DELETE operations, but also for selected risky update and mutation operations.

This SHOULD include:

- DELETE operations
- risky PUT/PATCH/POST operations with destructive or forceful effects

### 14.2 `--yes`

In non-interactive mode, confirmation-required operations MUST require explicit opt-in such as `--yes`.

### 14.3 Risk classification

The implementation SHOULD support a curated or rule-based classification of confirmation-required mutations, such as:

- reset actions
- overwrite actions
- revoke actions
- destructive bulk updates
- forceful match or session actions

## 15. File structure

The Rust project SHOULD be organized into modules broadly equivalent to:

```text
src/
  main.rs
  cli/
    mod.rs
    global_flags.rs
    dynamic.rs
    help.rs
  spec/
    mod.rs
    registry.rs
    fetcher.rs
    loader.rs
    normalize.rs
  auth/
    mod.rs
    commands.rs
    client_credentials.rs
    authorization_code.rs
    manager.rs
    store.rs
  config/
    mod.rs
    file.rs
    env.rs
  output/
    mod.rs
    human.rs
    json.rs
  runtime/
    mod.rs
    execute.rs
    pagination.rs
    retry.rs
  workflows/
    mod.rs
```

Exact module names MAY vary, but the design MUST separate spec loading, auth, config, execution, and output concerns.

## 16. Dependencies

The project SHOULD declare dependencies roughly equivalent to:

- `clap`
- `serde`
- `serde_json`
- `reqwest`
- `tokio`
- `keyring`
- `directories`
- `flate2`
- `thiserror`
- `anyhow`
- `rpassword`
- `insta`
- `wiremock`

## 17. Onboarding

### 17.1 Purpose

The CLI SHOULD provide lightweight onboarding that helps users reach a successful first command without requiring a heavy interactive wizard.

Onboarding SHOULD be:

- task-oriented
- profile-aware
- auth-aware
- non-intrusive
- safe for automation

### 17.2 Onboarding surfaces

The CLI SHOULD provide onboarding through:

- top-level help
- a one-time first-run hint
- auth commands
- validation and troubleshooting commands
- recovery-oriented errors

### 17.3 Top-level help

`ags --help` SHOULD include a short getting-started section.

That section SHOULD point users to:

- profile creation or selection
- config setup
- auth login
- auth status
- further help discovery

### 17.4 First-run hint

The CLI SHOULD show a one-time onboarding hint on the first interactive run.

The first-run hint:

- MUST NOT block command execution
- MUST NOT appear in non-interactive mode
- SHOULD be short
- SHOULD point users to profile setup, auth login, auth status, and help

### 17.5 Authentication as onboarding

`ags auth login` MUST remain a primary onboarding entrypoint.

`ags auth status` SHOULD provide a clear confirmation of current auth state and profile context.

### 17.6 Validation and troubleshooting

The CLI SHOULD provide `ags doctor`.

`ags doctor` SHOULD validate:

- profile selection
- config completeness
- config validity
- file permissions
- environment variable overrides
- keychain accessibility
- credential state
- auth state
- token refreshability
- base URL reachability
- namespace validity

### 17.7 Recovery-oriented errors

When setup is incomplete, the CLI SHOULD provide actionable errors with concrete next steps rather than generic failures.

This includes failures such as:

- no active profile
- unknown profile
- missing base URL
- missing namespace
- missing client ID
- missing credentials
- expired access token without successful refresh

### 17.8 Non-interactive behavior

Onboarding features MUST NOT interfere with automation.

Specifically:

- first-run hints MUST NOT appear in non-interactive mode
- normal commands MUST continue to fail with actionable errors rather than attempting to launch a setup flow automatically
- `ags doctor` SHOULD be safe to run non-interactively

### 17.9 Visual style alignment

Onboarding output SHOULD align with the CLI output style specification.

It SHOULD use the same message categories and hierarchy, including:

- info for setup context
- status for in-progress steps
- success for completed setup actions
- warning for incomplete but recoverable states
- error for blocked states
- fix or next-step guidance for user action

## 18. Distribution

### 18.1 Version command

The CLI SHOULD provide `ags version`.

### 18.2 Shell completions

The release process SHOULD generate shell completions for common shells.

### 18.3 Browser-based OAuth support

Release packaging SHOULD account for browser-based authorization-code login requirements on supported platforms.

## 19. Testing

### 19.1 Test layers

The project SHOULD include:

1. unit tests
2. integration tests
3. snapshot tests
4. naming-verification tests
5. mock-server-based HTTP tests

### 19.2 What to test

Testing SHOULD cover at least:

- spec normalization and command generation
- naming stability
- auth flows for supported grants
- token persistence and refresh-token handling
- confirmation behavior for DELETE and selected risky updates
- help output snapshots
- output formatting snapshots
- update-check logic
- keychain-backed and config-fallback credential storage paths where practical

### 19.3 What not to over-test

The test suite SHOULD avoid brittle tests tied to incidental formatting or external-service availability unless those are explicitly part of the contract.

## 20. CI and release automation

The repository SHOULD include CI workflows for:

- formatting and linting
- unit and integration testing
- snapshot verification
- release packaging
- artifact publication

The release workflow SHOULD manage versioning, generated artifacts, and changelog publication consistently.

## 21. Current scope

The CLI ships with:

- spec loading and command generation for all 24 services (bundled)
- human-readable help (4-level hierarchy)
- JSON output format
- `ags auth login` with `authorization-code` and `client-credentials`
- token persistence with keychain-first behavior
- token auto-refresh with expiry buffer
- profile-based configuration and profile-scoped auth (`ags profile list/create/use/show/delete/rename`)
- `ags config get/set/unset`
- `ags describe` (4-level service hierarchy plus `describe workflow [id]` for workflow introspection)
- `--skeleton` flag (fillable JSON request body templates)
- pagination (`--page-all` and `--page-limit` flags)
- namespace resolution (flag → env → config → error)
- human-readable output with field prioritization and templates
- error classification with actionable fix suggestions
- keyword-based confirmation for destructive operations (DELETE + risky POST/PUT/PATCH)
- `--dry-run`, `--verbose`, `--quiet`, `--no-input`, `--yes`, `--no-color`
- `ags auth login/logout/status/refresh`
- `ags doctor`, `ags refresh-specs`, `ags completions`, `ags version`

## 22. Open implementation notes

The following remain implementation choices, but they do not change the direction of the reference:

1. the exact authorization-code callback implementation
2. whether config fallback uses the main config JSON or a sibling credentials file
3. the exact risky-mutation classification rules

## 23. Decisions captured in this revision

This revision captures these decisions:

- password grant is removed entirely
- supported login grants are `authorization-code` and `client-credentials`
- authorization-code login is interactive and browser-based
- refresh tokens are supported
- client secret, access token, and refresh token are persisted
- storage is keychain-first with config-backed fallback
- the AI integration comparison is reframed as MCP server vs Skills + CLI
- repeatability is strengthened as a primary CLI advantage
- MCP is not described as IDE-bound
- configuration precedence is defined once as the general rule
- specs are bundled with the binary; remote spec fetching is out of scope
- destructive confirmations extend beyond DELETE to selected risky updates
- in-memory-only token handling is removed
- configuration is split into global and profile-scoped keys
- auth state is isolated per profile
- config/data/cache directories use platform-idiomatic names (`ags` on Linux, `com.accelbyte.ags` on macOS, `AccelByte\AGS` on Windows)
- onboarding is non-blocking and profile-first
- scope and version resolution is folded into this spec (§10.11); the prior standalone `scope-version-spec.md` is retired
- `--api-scope` selects scope (default `admin`); `--api-version` selects the command-scoped contract version
- deprecated contracts are excluded from the catalogue and not selectable via `--api-version`
- `ags describe` at method level exposes the full scope/version contract matrix as the machine-readable source of truth
