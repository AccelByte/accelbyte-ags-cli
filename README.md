# AGS CLI

AGS CLI is a unified command-line interface for AccelByte Gaming Services. Manage players, entitlements, inventories, sessions, and other live-service workflows from your terminal, scripts, or AI agents.

![AGS CLI demo](demos/onboarding/onboarding.gif)

## Install

Download the prebuilt archive for your OS and architecture from the [latest release](https://github.com/AccelByte/accelbyte-ags-cli/releases/latest). The release page includes per-platform install instructions and checksum verification.

Supported targets: macOS (`x86_64`, `aarch64`), Linux (`x86_64` and `aarch64`, both glibc and musl), Windows (`x86_64`).

Each archive contains a standalone `ags` binary. Place it somewhere on your `PATH`, then verify the install:

```bash
ags --help
```

Building from source is covered in [CONTRIBUTING.md](CONTRIBUTING.md).

## Quick start

AGS CLI supports three common ways of working. Pick the one that matches you and start there:

### Interactive

Log in and fetch a user interactively.

```bash
ags auth login
ags iam users get --namespace my-game --user-id abc-123
```

### Automation

Authenticate with client credentials and return a user's entitlements as JSON.

```bash
# Local testing only. In CI or production, inject these at runtime from your
# secret manager instead of exporting them in shell history or scripts.
export AGS_BASE_URL="https://your-base-url.invalid"
export AGS_CLIENT_ID="your-client-id"
export AGS_CLIENT_SECRET="your-client-secret"

ags auth login --grant client-credentials --no-input
ags platform entitlements list \
  --namespace my-game \
  --user-id abc-123 \
  --api-scope admin --api-version v1 \
  --no-input --format json
```

### AI agent

Inspect a command, then fetch a game session using the existing local session.

```bash
# Reuses the user's existing AGS session when the agent runs as the same OS user
# on the same machine.

ags describe session game-sessions get
ags session game-sessions get \
  --namespace my-game \
  --session-id session-123 \
  --api-scope public --api-version v1 \
  --format json
```

If the agent runs in a container, CI runner, remote sandbox, or separate OS account, authenticate it explicitly instead of relying on the user's local session.

The rest of the README expands on these workflows, then covers authentication, profiles, and configuration as reference material.

## Example tasks

```bash
# List users
ags iam users search --namespace my-game

# Preview the request for a mutating operation without sending it
ags iam users update --namespace my-game --user-id abc-123 --json @body.json --dry-run

# Check auth, config, and connectivity
ags doctor

# Upload a dedicated-server build to AMS
ags ams upload --path ./build --executable server --image-name my-image

# Open a tunnel to an Extend app pod
ags extend tunnel --namespace my-game --resource-name my-app --local-port 8080
```

## Using AGS CLI

### Choose a mode

Choose the mode that matches how you work: interactively in a terminal, non-interactively in scripts, or through an AI agent.

|                | Interactive                                                          | Automation                                                              | AI agent           |
| -------------- | -------------------------------------------------------------------- | ----------------------------------------------------------------------- | ------------------ |
| **Auth**       | [Authorization code](#authorization-code-flow-public-client)         | [Client credentials](#client-credentials-flow-confidential-client)      | Either             |
| **Discovery**  | `ags --help`                                                         | `ags describe`                                                          | `ags describe`     |
| **Versioning** | default contract                                                     | pinned contract                                                         | pinned contract    |
| **Input**      |                                                                      | `--no-input --yes`                                                      | `--no-input`       |
| **Output**     |                                                                      | `--format json`                                                         | `--format json`    |

> [!NOTE]
> - **Versioning.** Service commands resolve to a contract: command + scope (`admin` or `public`) + API version (`v1`, `v2`, …). Without flags, AGS picks the admin scope and the latest stable version it knows about; the latest version can shift between releases, so automation and AI agents should pin both `--api-scope` and `--api-version`. `--api-version latest` is intentionally not supported. Run `ags describe <service> <resource> <method>` to see the full scope/version matrix.
> - **Discovery.** `--help` is human-only and not a contract — don't parse it. Use `ags describe` for machine-readable introspection, typically at build time (generating typed clients, grounding agent prompts) rather than at runtime.
> - **Input.** `--no-input` fails rather than prompting for missing values; `--yes` auto-confirms mutating operations. AI agents should set `--no-input` but **not** `--yes`, since the user is present and human-in-the-loop confirmation should be preserved for destructive changes. If you're running fully headless on behalf of no user, follow the Automation column.
> - **Output.** Default human-readable output is not a contract — wording, layout, and colour can change between releases. `--format json` is the stable contract; machine-readable fields will not be removed or renamed without a major version bump. JSON works on every command, including `auth`, `config`, `profile`, and `version`. Set persistently with `ags config set format json` if every invocation in your environment should default to JSON.

### Command shape

Commands follow this shape:

```bash
ags <service> <resource> <method> [flags]
```

### Discovery

Use `--help` for human exploration:

```bash
ags --help
ags iam --help
ags iam users --help
ags iam users get --help
```

Use `ags describe` for machine-readable introspection:

```bash
ags describe
ags describe iam users get
ags describe workflow
ags describe workflow competitive-multiplayer
```

### Request bodies

Operations that take a body accept `--json '<inline-json>'` or `--json @path/to/body.json`. Use `--skeleton` to print a starter template you can edit.

> **PowerShell:** quote the `@` form — `--json '@body.json'` — otherwise PowerShell treats the leading `@` as a splatting operator and the file is not read.

### Global flags

<details>
<summary>Show all global flags</summary>

| Flag | Description |
|------|-------------|
| `--profile <name>` | Target a specific profile (or set `AGS_PROFILE`) |
| `--namespace <ns>` | Target namespace (or set `AGS_NAMESPACE`) |
| `--format json` | Machine-readable JSON output for automation |
| `--dry-run` | Show the request without sending it |
| `--skeleton` | Output a JSON request body template (for operations with `--json`) |
| `--page-all` | Fetch all pages of paginated results |
| `--page-limit <N>` | Max pages to fetch with `--page-all` (default 10, max 100) |
| `--verbose` | Print request and response details |
| `--quiet` | Suppress progress and status output |
| `--yes` | Skip confirmation prompts for mutating operations |
| `--timeout <secs>` | Request timeout in seconds (default 60) |
| `--no-input` | Disable all interactive prompts |
| `--no-color` | Disable colour output (or set `NO_COLOR`) |
| `--api-scope <scope>` | Endpoint audience for this command (default `admin`) |
| `--api-version <vN>` | Pin a specific API version for the resolved scope |

Global flags can appear anywhere in the command. The `format`, `no-color`, and `timeout` flags can also be set as persistent defaults via `ags config set`.

</details>

## Authentication

> [!WARNING]
> **Treat client secrets and access tokens like production credentials.**
> - Store secrets in a secret manager (HashiCorp Vault, AWS Secrets Manager, GitHub Actions secrets, etc.). Never put them in source control or plain config files.
> - Inject at runtime via `AGS_CLIENT_ID`, `AGS_CLIENT_SECRET`, or a short-lived `AGS_ACCESS_TOKEN`.
> - Restrict the IAM client's permissions to the minimum required for the workload.
> - Rotate client secrets periodically and on personnel changes.
> - Avoid logging command lines that include credential-bearing flag values; the CLI redacts known secrets in its own output but upstream shells, CI logs, and downstream tooling may not.

AGS CLI supports two OAuth2 flows. Each requires a different type of IAM client, created in the AccelByte Admin Portal under **Platform Configuration → IAM Clients**.

If your team already gave you a base URL and IAM client, skip client creation and go straight to the appropriate login flow below.

| Flow | IAM client type | When to use |
|------|----------------|-------------|
| Authorization code | **Public** | Interactive use, local development |
| Client credentials | **Confidential** | Headless environments, CI, service-to-service |

### Creating an IAM client

> [!IMPORTANT]
> Public IAM clients must have `http://127.0.0.1:8080` as a redirect URI. The CLI's authorization code flow uses a localhost callback on port 8080 (configurable with `--port`). Without this redirect URI, the browser login will fail with an OAuth error.

1. Go to your AccelByte Admin Portal
2. Navigate to **Platform Configuration → IAM Clients**
3. Create a new client:
   - For interactive use: choose **Public** client type
   - For CI or automation: choose **Confidential** client type and note the client secret
4. Add `http://127.0.0.1:8080` as a redirect URI (public clients only)
5. Assign the permissions your CLI usage requires

> [!NOTE]
> **Where to create your IAM client depends on your deployment.**
>
> **Private cloud:**
> - **Publisher-level IAM client**: SSO or email/password login. Best for cross-game management.
> - **Game-level IAM client**: email/password login. Best for single-game management.
>
> **Shared cloud:**
> - **Studio-level IAM client**: not available.
> - **Game-level IAM client**: email/password login.

### Authorization code flow (public client)

The default flow for interactive use:

```bash
ags auth login
```

You will be prompted for the base URL and client ID. The CLI then prints a URL for you to open in your browser and completes authentication through a localhost callback.

### Client credentials flow (confidential client)

For headless environments such as CI:

```bash
ags auth login --grant client-credentials
```

You will be prompted for the base URL, client ID, and client secret.

### Token storage

Both flows store tokens in the OS keychain when available:

- macOS Keychain
- Windows Credential Manager
- Linux Secret Service

When the OS keychain is unavailable or `AGS_NO_KEYCHAIN=1` is set, tokens fall back to a file under your config directory.

### Session management

```bash
ags auth status
ags auth refresh        # re-mint the access token from stored credentials
ags auth logout
ags auth logout --all   # clear credentials from all profiles
```

### Environment variables

Most interactive users don't need these. They let you override config without touching files, which is the usual pattern for CI and containerised use.

| Variable | Description |
|----------|-------------|
| `AGS_ACCESS_TOKEN` | Use this token directly and skip interactive auth |
| `AGS_CLIENT_ID` | Client ID for client-credentials flow |
| `AGS_CLIENT_SECRET` | Client secret for client-credentials flow |
| `AGS_BASE_URL` | AccelByte API base URL |
| `AGS_NAMESPACE` | Default namespace |
| `AGS_PROFILE` | Active profile name |
| `AGS_HOME` | Override config and cache directory location |
| `AGS_AUTH_TIMEOUT` | Timeout in seconds for browser auth flow (default 120) |
| `AGS_NO_KEYCHAIN` | Disable OS keychain, use file-based token storage |
| `AGS_NO_UPDATE_CHECK` | Set to `1` to disable the background check for new releases |
| `AGS_UPDATE_CHECK_URL` | Test hook: override the update-check endpoint URL (not for end-user use) |
| `DO_NOT_TRACK` | Set to any non-empty value to disable telemetry ([consoledonottrack.com](https://consoledonottrack.com)) |
| `AGS_TELEMETRY_NO_INPUT_VALUES` | Set to any non-empty value to suppress input field values in telemetry (field names and metadata still transmit) |

## Profiles

Profiles manage separate environments (dev, staging, prod). Each profile stores its own base URL, client ID, namespace, and credentials independently.

A `default` profile is created automatically the first time you run a command. For single-environment use, you don't need to think about profiles at all.

```bash
# Create and switch
ags profile create staging
ags profile use staging

# Authenticate against the active profile
ags auth login

# Inspect
ags profile list
ags profile show staging

# Manage
ags profile rename staging production
ags profile delete old-profile

# Target a specific profile without switching
ags iam users list --namespace my-game --profile staging
```

## Configuration

Use `ags config` to view and manage settings. Profile-scoped keys (like `base-url`, `namespace`) are stored per profile. Global keys (like `format`) apply to all profiles.

```bash
# View all config values and where they come from
ags config get

# Set a value (scope is auto-detected from the key)
ags config set base-url https://your-base-url.invalid
ags config set format json

# Remove a value
ags config unset namespace
```

### Update check

The `update-check` global key controls the passive "a newer release is available" hint. It is on by default; disable it with `ags config set update-check false` or the `AGS_NO_UPDATE_CHECK=1` environment variable (both also skip the background GitHub check entirely, as does running in CI). The hint is printed to stderr only on an interactive terminal — never under `--format json`, when output is piped, or for `doctor` / `version` / `completions`.

## Shell completions

```bash
# zsh
source <(ags completions zsh)

# bash
source <(ags completions bash)

# fish
ags completions fish | source

# PowerShell (add to $PROFILE for persistence)
ags completions powershell | Out-String | Invoke-Expression
```

Running `ags completions` without an argument detects the shell from `$SHELL` (or defaults to PowerShell on Windows) and prints an install hint to stderr.

## Troubleshooting

Use `ags doctor` to check config, auth, and connectivity. Use `ags refresh-specs` if command metadata looks stale after upgrading, a service is missing expected commands, or `ags describe` output seems out of date.

```bash
ags doctor              # check config, auth, and connectivity
ags doctor --offline    # skip network checks
ags doctor --all        # check all profiles

ags refresh-specs       # rebuild the parsed-schema cache for every service
ags refresh-specs iam   # rebuild the cache for a single service
```

## Supported services

AGS CLI covers every AccelByte Gaming Services API. Run `ags --help` for the live list, or `ags describe` for machine-readable metadata.

<details>
<summary>Full service list</summary>

| Service | CLI name | Description |
|---------|----------|-------------|
| Achievement | `achievement` | Player achievements, badges, and progression |
| AMS | `ams` | Multiplayer server fleet management and orchestration |
| Basic | `basic` | User profiles, namespaces, and file uploads |
| Challenge | `challenge` | Challenge definitions, goals, and progression |
| Chat | `chat` | Chat messaging, moderation, and filtering |
| Cloud Save | `cloud-save` | Cloud save records for games and players |
| CSM | `csm` | Custom service management and deployments |
| Game Telemetry | `game-telemetry` | Telemetry event ingestion and playtime tracking |
| GDPR | `gdpr` | Data deletion, retrieval, and account closure |
| Group | `group` | Player groups, memberships, and roles |
| IAM | `iam` | Identity, auth, and user management |
| Inventory | `inventory` | Player inventories, items, and tags |
| Leaderboard | `leaderboard` | Leaderboard config, data, and rankings |
| Legal | `legal` | Legal agreements, policies, and consent |
| Lobby | `lobby` | Friends, presence, party, and notifications |
| Login Queue | `login-queue` | Login queue management and tickets |
| Matchmaking | `matchmaking` | Matchmaking pools, rulesets, and tickets |
| Platform | `platform` | Store, entitlements, wallets, and payments |
| Reporting | `reporting` | Player reporting and moderation |
| Season Pass | `season-pass` | Season passes, tiers, and rewards |
| Session | `session` | Game sessions, parties, and DS config |
| Session History | `session-history` | Session analytics and X-Ray diagnostics |
| Social | `social` | Stats, game profiles, and social slots |
| UGC | `ugc` | User-generated content, channels, and moderation |

</details>

## Workflows

A *workflow* is a multi-step operation: it chains several API calls, passing
values from one step to the next. Run one with:

    ags workflow run <workflow-id> [--<input> <value>]…

Each workflow declares its own inputs as `--<name>` flags; run with `--help`
to list them:

    ags workflow run competitive-multiplayer --help

`competitive-multiplayer` is the bundled example: it stands up competitive
matchmaking with dedicated servers (skill stat → ruleset → session template
→ match pool → server-image upload → AMS fleet → fleet wiring). Its required
inputs are `--namespace`, `--build-path`, `--build-executable`,
`--fleet-region`, and `--fleet-instance-id`; the resource names default to
`ranked-*` (override with `--resource-prefix`). The build directory is
uploaded to AMS as the image the fleet runs, so there is no image id to
supply. When run interactively, the region and instance-type fields are
runtime-fetched pickers — type to filter, ↑/↓ to scroll, Enter to select.

Single commands (`ags <service> <resource> <method>`) are internally
1-step workflows — this is invisible in normal use. Add `--dry-run` to preview a
workflow without calling the API, and `--no-input` to fail instead of
prompting for missing inputs. Under `--format json` a workflow runs
non-interactively: supply every input as a flag (and `--yes` for any
confirm-gated step), and the result is emitted as a JSON envelope.

### Writing your own

A workflow can be a YAML file you write yourself, not just a bundled one. Start from a
skeleton, edit it, then install it:

    ags workflow template --output my-flow.yaml
    ags workflow add my-flow.yaml
    ags workflow run my-flow

`add` validates the file before installing it, so a mistake is reported rather than
discovered mid-run. It installs a *copy* under the CLI's config directory, named after the
file's own `id:` field, and that copy is what runs — editing your original afterwards has no
effect until you run `add` again. `ags workflow remove <id>` uninstalls one. List everything
registered with `ags workflow list` (human) or `ags describe workflow` (machine-readable).

Every workflow file declares a `workflow_protocol_version`. The template fills it in, so
starting from `ags workflow template` is the shortest path to a file that installs.

## Uploading a dedicated-server image

`ags ams upload` archives a build directory and registers it as an AMS image, so a
dedicated server can be shipped without the Admin Portal:

    ags ams upload --path ./build --executable server --image-name my-image

The entrypoint must be a 64-bit little-endian ELF binary (x86-64 or aarch64), or a shell
script — in which case pass `--target-arch linux-x86_64` or `--target-arch linux-arm_64`,
since a script carries no architecture to detect. Debug-symbol files (`.pdb`, `.sym`,
`.debug`) are excluded unless you pass `--symbol-files`.

Credentials and the platform host come from your login or profile, exactly as for every
other command — there are no credential flags. `--namespace` is accepted but ignored; AMS
derives the destination from the access token. Add `--dry-run` to validate the build
directory and list what would be uploaded without archiving anything or calling the API.

Uploading needs its own permission (`AMS:UPLOAD`, with `Create` and `Update`) on a
**confidential** IAM client — it is not the same permission as the one behind
`ags ams images`, so an identity that can list images may still be refused. For client
setup, the minimum permissions, migrating a pipeline from the standalone `ams` CLI, and
what each error means, see **[AMS image upload](docs/reference/ams-upload.md)**.

## Extend

`ags extend` covers the Extend platform: cloning a starter template, logging in to the
container registry, building and pushing an image, tunnelling to a running app pod, and
remote debugging.

    ags extend clone-template --help
    ags extend image-upload --help
    ags extend tunnel --help

If you are moving from `extend-helper-cli`, its commands are here under the names you already
know — `create-app`, `deploy-app`, `list-images` and the rest. Each forwards to the `ags csm …`
command that does the work, and `ags extend --help` shows which. Use the `ags csm …` address in
scripts; the shortcuts are for finding your way across.

## Telemetry

AGS CLI collects anonymous usage telemetry to help the team understand which commands are used, where users hit errors, and how workflows perform. Telemetry is active in official release builds. It can be disabled at any time (see below).

### Opting out

Set `DO_NOT_TRACK=1` to disable telemetry (the [Console Do Not Track](https://consoledonottrack.com) standard). This is the recommended opt-out: official release builds ship with a PostHog key already compiled in, so unsetting `AGS_TELEMETRY_POSTHOG_KEY` does not disable telemetry on its own — it only stops `AGS_TELEMETRY_POSTHOG_KEY` from overriding the built-in key.

### What is collected

When enabled, the CLI sends: which command was run (`iam users list`, not positional values or bodies), which flags were present (values redacted by default — only `--namespace`, `--format`, `--ui`, `--user-id`, `--client-id` transmit values), CLI version, outcome, duration, and error classification on failure. For workflow runs, step-level timing, outcome counts, and how inputs were provided (counts, not values) are also sent. Authenticated users are identified by their AccelByte IAM user id; pre-login invocations use a random anonymous id (never derived from hostname, hardware, or OS username). Email is attached as a user property for lookup, never as a per-event property.

No IP geolocation (explicitly disabled), no file paths, no raw HTTP request or response bodies, no credential values, and no hostname or hardware identifiers are ever collected. On failed workflow steps, structured input field metadata (names, sources, and non-sensitive values for bundled workflows) is sent to aid debugging — field values are redacted when the name matches a sensitive pattern (`secret`, `password`, `token`, `key`, etc.). Set `AGS_TELEMETRY_NO_INPUT_VALUES` to suppress all input values entirely. Every event is fire-and-forget and never affects command behaviour or exit codes.


## Contributing

Build, test, and contribution instructions live in [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
