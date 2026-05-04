# AGS CLI

AGS CLI is a command-line interface for [AccelByte Gaming Services](https://accelbyte.io). It generates commands directly from AccelByte's OpenAPI specs so you can explore and call APIs from the terminal with a consistent command structure.

![AGS CLI demo](demo/reel.gif)

## Current Status

AGS CLI ships full coverage of all 24 AccelByte services, with bundled OpenAPI specs and end-to-end runtime support for every command.

Prebuilt binaries are published on the [GitHub Releases page](https://github.com/AccelByte/accelbyte-ags-cli/releases) for:

- macOS: `x86_64-apple-darwin`, `aarch64-apple-darwin`
- Linux: `x86_64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-gnu`, `aarch64-unknown-linux-musl`
- Windows: `x86_64-pc-windows-msvc`

There is no package-manager distribution (Homebrew, apt, `cargo install`, etc.) yet.

## Install

Download the prebuilt archive for your OS and architecture from the [latest release](https://github.com/AccelByte/accelbyte-ags-cli/releases/latest). The release page includes per-platform install instructions and checksum verification.

If you'd rather build from source, see [Build from source](#build-from-source) below.

## Build from source

### Prerequisites

- Git
- [Rust](https://rustup.rs/) 1.84 or newer
- On Linux, system packages required for keyring support may also be needed: `libdbus-1-dev` and `libsecret-1-dev`

### 1. Clone the repository

```bash
git clone https://github.com/AccelByte/accelbyte-ags-cli.git
cd accelbyte-ags-cli
```

### 2. Build the binary

```bash
cargo build --release
```

This creates the CLI binary in:

- macOS and Linux: `target/release/ags`
- Windows: `target\release\ags.exe`

### 3. Run it

Without installing globally:

```bash
./target/release/ags --help
```

On Windows PowerShell:

```powershell
.\target\release\ags.exe --help
```

### Optional: add it to your PATH

#### macOS and Linux

You can copy the binary to a directory already on your `PATH`, such as `/usr/local/bin`:

```bash
cp target/release/ags /usr/local/bin/
```

If you do not want to write to a system directory, add `target/release` to your `PATH` instead.

#### Windows

You can:

- Run `target\release\ags.exe` directly
- Copy `ags.exe` to a folder already on your `PATH`
- Add `target\release` to your `PATH`

After that, verify the installation:

```bash
ags --help
```

## Shell completions

AGS prints a completion script for `bash`, `zsh`, `fish`, or `powershell`:

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

## Quick Start

```bash
# Authenticate using the authorization code flow
ags auth login

# List IAM users
ags iam users list --namespace my-game

# Inspect a specific user
ags iam users get --userId abc-123 --namespace my-game

# Preview a request without sending it
ags iam users list --namespace my-game --dry-run

# Generate a request body template
ags iam roles create --skeleton > body.json

# Discover commands programmatically (JSON)
ags describe iam users list
```

## Usage

```bash
ags <service> <resource> <method> [flags]
```

### Global Flags

| Flag | Description |
|------|-------------|
| `--profile <name>` | Target a specific profile (or set `AGS_PROFILE`) |
| `--namespace <ns>` | Target namespace (or set `AGS_NAMESPACE`) |
| `--format json` | Machine-readable JSON output for automation |
| `--dry-run` | Show the request without sending it |
| `--skeleton` | Output a JSON request body template (for operations with `--json`) |
| `--page-all` | Fetch all pages of paginated results |
| `--page-limit <N>` | Max pages to fetch with --page-all (default 10, max 100) |
| `--verbose` | Print request and response details |
| `--quiet` | Suppress progress and status output |
| `--yes` | Skip confirmation prompts for mutating operations |
| `--timeout <secs>` | Request timeout in seconds (default 60) |
| `--no-input` | Disable all interactive prompts |
| `--no-color` | Disable colour output (or set `NO_COLOR`) |

Global flags can appear anywhere in the command. The `format`, `no-color`, and `timeout` flags can also be set as persistent defaults via `ags config set`.

### Selecting API scope and version

Generated service commands resolve to a contract — the combination of command, scope (`admin` or `public`), and API version (`v1`, `v2`, …). Two flags select the contract:

| Flag | Description |
|------|-------------|
| `--api-scope <scope>` | Endpoint audience for this command (default `admin`) |
| `--api-version <vN>` | Pin a specific API version for the resolved scope |

```bash
ags iam users get 123                                       # admin scope, default version
ags iam users get 123 --api-scope public                    # public scope, default version
ags iam users get 123 --api-version v3                      # admin scope, v3
ags iam users get 123 --api-scope public --api-version v2   # fully pinned
```

Both flags only appear on commands that offer a choice. Run `ags describe <service> <resource> <method>` to see the full scope/version matrix for a command, including the default contract and supported versions per scope.

> **Pin scope and version in automation.** Omitting the flags resolves to the CLI's *current* defaults. Those defaults can shift between releases as new versions are introduced or old ones retired. Scripts, CI pipelines, and AI agents that need stable behaviour across upgrades **must** pass both `--api-scope` and `--api-version` explicitly. The `--api-version latest` token is intentionally not supported — pinning means pinning to a specific `vN`.

### Output Formats

**Do not parse human-readable output.** The default output is designed for humans and may change format, wording, or layout between releases without notice. Scripts, CI pipelines, and AI agents that parse CLI output **must** use `--format json`. JSON output is the stable contract — fields will not be removed or renamed without a major version bump.

JSON output works across all commands, including `auth`, `config`, `profile`, and `version`.

### Exploring Commands

```bash
ags --help
ags iam --help
ags iam users --help
ags iam users get --help
```

For machine-readable discovery, use `ags describe` instead of `--help`. It outputs JSON with command metadata including parameters, body schema, and execution semantics. This is designed for AI agents, scripts, and tooling.

```bash
# List all services
ags describe

# Introspect a specific command
ags describe iam users list
```

## Authentication

AGS CLI supports two OAuth2 flows. Each requires a different type of IAM client, created in the AccelByte Admin Portal under **Platform Configuration → IAM Clients**.

| Flow | IAM client type | When to use |
|------|----------------|-------------|
| Authorization code | **Public** | Interactive use, local development |
| Client credentials | **Confidential** | Headless environments, CI, service-to-service |

### Creating an IAM client

> **Important:** Public IAM clients must have `http://127.0.0.1:8080` as a redirect URI. The CLI's authorization code flow uses a localhost callback on port 8080 (configurable with `--port`). Without this redirect URI, the browser login will fail with an OAuth error.

1. Go to your AccelByte Admin Portal
2. Navigate to **Platform Configuration → IAM Clients**
3. Create a new client:
   - For interactive use: choose **Public** client type
   - For CI or automation: choose **Confidential** client type and note the client secret
4. Add `http://127.0.0.1:8080` as a redirect URI (public clients only)
5. Assign the permissions your CLI usage requires

### Authorization code flow

This is the default flow for interactive use. Requires a **public** IAM client.

```bash
ags auth login
```

You will be prompted for the base URL and client ID. The CLI then prints a URL for you to open in your browser and completes authentication through a localhost callback.

### Client credentials flow

Use this for headless environments such as CI. Requires a **confidential** IAM client.

```bash
ags auth login --grant client-credentials
```

You will be prompted for the base URL, client ID, and client secret.

Tokens are stored in the OS keychain when available:

- macOS Keychain
- Windows Credential Manager
- Linux Secret Service

Credentials are never stored in plaintext config files.

### Environment Variable Overrides

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

### Session Management

```bash
ags auth status
ags auth logout
ags auth logout --all   # clear credentials from all profiles
```

### Troubleshooting

```bash
ags doctor              # check config, auth, and connectivity
ags doctor --offline    # skip network checks
ags doctor --all        # check all profiles

ags refresh-specs       # rebuild the parsed-schema cache for every service
ags refresh-specs iam   # rebuild the cache for a single service
```

## Profiles

AGS CLI uses profiles to manage separate environments (dev, staging, prod). Each profile stores its own base URL, client ID, namespace, and credentials independently.

A `default` profile is created automatically the first time you run a command. For single-environment use, you don't need to think about profiles at all.

```bash
# Create a profile for a different environment
ags profile create staging

# Switch the active profile
ags profile use staging

# Authenticate against the active profile
ags auth login

# List all profiles
ags profile list

# Show profile details
ags profile show staging

# Rename or delete a profile
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
ags config set base-url https://demo.accelbyte.io
ags config set format json

# Remove a value
ags config unset namespace
```

## Supported Services

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

## Contributing

If you want to build, test, or contribute to AGS CLI itself, see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
