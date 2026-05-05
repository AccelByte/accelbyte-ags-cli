# AGS CLI

AGS CLI is a unified command-line interface for AccelByte Gaming Services. Manage players, entitlements, inventories, sessions, and other live-service workflows from your terminal, scripts, or AI agents.

![AGS CLI demo](demo/reel.gif)

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
```

## Using AGS CLI

### Choose a mode

Choose the mode that matches how you work: interactively in a terminal, non-interactively in scripts, or through an AI agent.

|                | Interactive                                                                | Automation                                                                  | AI agent                                                                    |
| -------------- | -------------------------------------------------------------------------- | --------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| **Auth**       | [Authorization code flow](#authorization-code-flow-public-client)          | [Client credentials flow](#client-credentials-flow-confidential-client)     | reuses the user's local session when available                              |
| **Discovery**  | interactive `--help`                                                       | `ags describe` (JSON)                                                       | `ags describe` (JSON)                                                       |
| **Versioning** | default contract                                                           | pinned contract                                                             | pinned contract                                                             |
| **Input**      | interactive prompts                                                        | non-interactive, auto-confirm                                               | non-interactive, human confirms mutating actions                            |
| **Output**     | human-readable                                                             | [stable JSON](#stable-json-output)                                            | [stable JSON](#stable-json-output)                                            |

### Contracts and input

- **Pinning the contract.** Service commands resolve to a contract: the combination of command, scope (`admin` or `public`), and API version (`v1`, `v2`, …). Without flags, AGS picks the admin scope and the latest stable version it knows about. The latest version can shift between releases, so automation and AI agents should pin both `--api-scope` and `--api-version` on every call. `--api-version latest` is intentionally not supported. Run `ags describe <service> <resource> <method>` to see the full scope/version matrix for a command.
- **Non-interactive input.** Pair `--no-input` (fail rather than prompt for missing values) with `--yes` to auto-confirm mutating operations. AI agents should set `--no-input` but **not** `--yes`, since the user is present and human-in-the-loop confirmation should be preserved for destructive changes. If you're running fully headless on behalf of no user, follow the Automation column instead.
- **Don't parse `--help`.** It is human-only and not a contract. Use `ags describe` for machine-readable introspection, typically at build time (generating typed clients, grounding agent prompts) rather than at runtime.
- **Request bodies.** Operations that take a body accept `--json '<inline-json>'` or `--json @path/to/body.json`. Use `--skeleton` to print a starter template you can edit.

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
```

### Stable JSON output

Default human-readable output is not a contract: wording, layout, and colour can change between releases. `--format json` is the stable contract, and machine-readable fields will not be removed or renamed without a major version bump. JSON works on every command, including `auth`, `config`, `profile`, and `version`.

Set persistently with `ags config set format json` if every invocation in your environment should default to JSON.

> [!WARNING]
> **Treat client secrets and access tokens like production credentials.**
> - Store secrets in a secret manager (HashiCorp Vault, AWS Secrets Manager, GitHub Actions secrets, etc.). Never put them in source control or plain config files.
> - Inject at runtime via `AGS_CLIENT_ID`, `AGS_CLIENT_SECRET`, or a short-lived `AGS_ACCESS_TOKEN`.
> - Restrict the IAM client's permissions to the minimum required for the workload.
> - Rotate client secrets periodically and on personnel changes.
> - Avoid logging command lines that include credential-bearing flag values; the CLI redacts known secrets in its own output but upstream shells, CI logs, and downstream tooling may not.

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

AGS CLI supports two OAuth2 flows. Each requires a different type of IAM client, created in the AccelByte Admin Portal under **Platform Configuration → IAM Clients**.

If your team already gave you a base URL and IAM client, skip client creation and go straight to the appropriate login flow below.

| Flow | IAM client type | When to use |
|------|----------------|-------------|
| Authorization code | **Public** | Interactive use, local development |
| Client credentials | **Confidential** | Headless environments, CI, service-to-service |

### Creating an IAM client

> [!IMPORTANT]
> Public IAM clients must have `http://127.0.0.1:8080` as a redirect URI. The CLI's authorization code flow uses a localhost callback on port 8080 (configurable with `--port`). Without this redirect URI, the browser login will fail with an OAuth error.

> [!NOTE]
> **Where to create your IAM client depends on your deployment.**
>
> **Private cloud** (publisher and game namespaces):
> - **Publisher-level IAM client**: SSO (Google, Apple, Discord, etc.) or email/password login. Best for cross-game work and publisher-level control.
> - **Game-level IAM client**: email/password login. Best for single-game management.
>
> **Shared cloud** (studio and game namespaces):
> - **Studio-level IAM client**: not available.
> - **Game-level IAM client**: email/password login.

1. Go to your AccelByte Admin Portal
2. Navigate to **Platform Configuration → IAM Clients**
3. Create a new client:
   - For interactive use: choose **Public** client type
   - For CI or automation: choose **Confidential** client type and note the client secret
4. Add `http://127.0.0.1:8080` as a redirect URI (public clients only)
5. Assign the permissions your CLI usage requires

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

## Contributing

Build, test, and contribution instructions live in [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
