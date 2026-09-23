# Changelog

All notable changes to the AGS CLI are recorded here. This file starts at 0.5.0; for
earlier versions see the [releases page](https://github.com/AccelByte/accelbyte-ags-cli/releases).

## Unreleased

## 0.5.1 — 2026-09-23

### Added

- `ags auth token` prints the current access token to stdout and nothing else, so a
  script can reuse the CLI's session instead of running its own login, for example by
  passing `$(ags auth token)` as the bearer value of the `Authorization` header. Until now
  there was no way to get the bearer out of the CLI — `ags auth status --format json` reports
  `"access_token": "valid"`, and `--dry-run`/`--verbose` print only a redacted header.
  The token is resolved exactly as an API call resolves it (`AGS_ACCESS_TOKEN`, then the
  stored token, refreshed when expired), so the printed token is the one the next request
  would send. With no token available it exits `2` (authentication failure) or `4`
  (identity service unreachable), leaving stdout empty. `--format json` adds
  `expires_at` (Unix epoch seconds) and `source` (`env` / `stored` / `refreshed` /
  `client_credentials`). The token is printed only on stdout and never reaches stderr,
  including under `--verbose`, or telemetry. `--dry-run` is refused with a usage error
  because the command's only output would be a live credential.

- `ags update` checks GitHub for a newer release and prints the upgrade command for the
  detected install method (installer script, Homebrew, or manual). It does not modify the
  installation. Use `--format json` for scripts.

- `ags update --install` downloads the newest release's installer script and runs it for
  this copy after confirmation (`--yes` for scripts). Refuses a copy installed with
  Homebrew when an update is available. Keeps the previous binary as `.old` and restores it
  on failure. The first Ctrl-C restores the previous binary and exits 2; a second Ctrl-C is
  a force quit that exits at once and does not guarantee lock release or installer
  termination. Exit codes: 0 (installed or already current), 1 (usage or Homebrew refusal),
  2 (declined or interrupted), 4 (download failure), 5 (installer failure, verification
  failure, or restore failure). `--dry-run` prints what it would do and sends no request.
  The CLI never updates itself unasked.

- `ags extend security-assessment request` starts a pen-testing engagement for an Extend
  app's endpoints, and `ags extend security-assessment result` downloads a completed
  engagement's report. Endpoints that accept `PUT`, `PATCH` or `DELETE` are listed and
  confirmed before the request is sent, because the assessment may generate test cases that
  modify or delete data through them. `--yes` confirms in non-interactive mode. `--wait` polls
  every 10 seconds until the engagement reaches `COMPLETED` or `FAILED`, bounded by
  `--wait-limit` (default 1800s). A downloaded report is written with `0600` permissions on
  Unix.

- `ags extend create-app`, `deploy-app`, `start-app`, `stop-app` and `delete-app` accept
  `--wait` to block until the operation finishes, with `--wait-interval` (default 10s) and
  `--wait-limit` (default 600s), matching `extend-helper-cli`. This was listed under 0.5.0 by
  mistake. It merged after that release and ships here.

- When a command exists at more than one API version, the output names the version it used.
  The label goes to stderr, so `--format json` output a script parses is unchanged.

### Fixed

- `ags extend <shortcut> --help` now shows the shortcut's own help page, instead of a usage
  line for a command you did not type.
- The `DO_NOT_TRACK` opt-out link now points at `https://donottrack.sh`. The previous domain
  serves unrelated content.
- `ags ams upload` error messages now quote the `--executable` value as you typed it
  instead of the resolved path. A 403 while finalizing or completing the upload now says
  the identity is missing the `Update` action of `AMS:UPLOAD`, instead of implying the
  whole permission is missing.
- The `competitive-multiplayer` workflow briefing no longer tells you to upload the image with
  the retired AMS CLI. The workflow archives and uploads the build itself.
- `ags ams upload --dry-run` now validates and normalises `--upload-url` the same way a
  live upload does. An invalid override now fails the dry run instead of being echoed back
  unvalidated, and a trailing slash is stripped from the reported `upload_base_url`.
- `ags extend deploy-app --wait` and `start-app --wait` no longer poll for the full
  `--wait-limit` when a deployment comes up and then crashes (bad image, failed readiness,
  CrashLoopBackOff). The app's `deployment-down` status is now treated as a failed rollout,
  so the command exits promptly with `deployment failed: deployment-down` (exit `3`) instead
  of a misleading timeout (exit `6`).

### Changed

- `ags ams upload` human output no longer swaps its streams. The result block — Image ID,
  architecture, entrypoint, archive size and upload host, or the `--dry-run` plan rows — now
  goes to **stdout**, and the `Image "<name>" uploaded` banner, along with the dry-run tip,
  goes to **stderr**. Until now it was the reverse, so `ags ams upload ... > result.txt` and
  `--output result.txt` saved the banner and lost the Image ID. Scripts that read the old
  streams will see a change: read the Image ID from stdout, not stderr. The banner still
  prints before the block on a terminal, and `--format json` is unchanged.
- `ags ams upload` writes its first progress line, `Validating <dir>`, as a normal stderr
  line like the lines after it. It was sent as a transient status update, so the next line
  overwrote it on a terminal and it was absent entirely from a captured run.
- `ags extend deploy-app --wait` now confirms the app is reporting the deployment this
  command created before trusting any terminal state. Previously the wait evaluated
  `appStatus` alone, so its correctness depended on CSM setting `deployment-in-progress` in
  the same transaction as the deployment insert — an ordering the CLI cannot enforce and had
  no test against. Confirming OUR deployment id first means a later change to CSM's write
  ordering, or a concurrent deploy by another actor, cannot produce a false success. This is
  hardening of the (unreleased) `--wait` feature, not a fix for shipped behaviour.
- A `--wait` timeout exits with code `6`, distinct from an API error (`3`), on both the Extend
  app lifecycle commands and `security-assessment request`, so a CI
  caller can tell "the wait timed out, the operation may still land" from "the rollout
  failed, do not retry" without matching message text.
- The CSM spec now bundles the v5 operations for the Extend app lifecycle —
  create-deployment, delete-app, get-app, start-app, stop-app, and list-images. As v5 is the
  highest bundled version it becomes the default, so `ags extend deploy-app`, `delete-app`,
  `get-app-info`, `start-app`, `stop-app`, and `list-images` now target their `/csm/v5/...`
  endpoints. Together with `create-app` (already v5), every Extend app-lifecycle command now
  uses the CSM v5 API. Verified end to end against the development cluster.
- `ags extend update-secret` and `ags extend update-var` also **move** from the CSM v2 to the
  v5 secrets/variables endpoints. These are existing commands, so this is a behaviour change;
  the request and response shapes are unchanged from v2.
- The v5 endpoints use the same IAM permission resources as their v2 counterparts
  (`ADMIN:NAMESPACE:{namespace}:EXTEND:APP` / `:DEPLOYMENT` / `:IMAGE` / `:VARIABLE` /
  `:SECRET`, actions unchanged), so moving these commands to v5 does not change what a caller
  must be granted — no role that worked on v2 will start getting 403 after upgrading.

### Security

- Updated `rustls` from 0.23.43 to 0.23.45 for a published advisory.

## 0.5.0-rc.2 — 2026-09-02

Second release candidate for 0.5.0. The first proved the release pipeline itself — the
seven-target build, the checksums, both installers and the Homebrew formula. This one
exists to confirm anonymous usage telemetry actually works in a build produced by that
pipeline, which was added after the first candidate was cut. Like the first, it is not
intended for general use, it is marked as a pre-release, and the update hint does not
advertise it.

## 0.5.0-rc.1 — 2026-09-02

Release candidate for 0.5.0. The contents are those listed under 0.5.0 below. This tag exists
to exercise the release pipeline — the seven-target build, the checksums, both installers and
the Homebrew formula — before the stable release depends on it. It is not intended for general
use, it is marked as a pre-release, and the update hint does not advertise it.

## 0.5.0 — 2026-09-02

The first release since 0.4.0 on 21 July 2026. It adds three large areas of new functionality,
all released as early preview, and it changes how the CLI is installed and updated.

### Highlights

| # | Change | Status |
|---:|---|---|
| 1 | `ags extend` — the Extend Helper CLI commands, inside AGS CLI | Early preview |
| 2 | `ags ams upload` — the AMS CLI image upload, inside AGS CLI | Early preview |
| 3 | Custom workflows written in YAML, added and run from your own files | Early preview |
| 4 | Install and upgrade with one command on macOS, Linux and Windows | Generally available |
| 5 | The CLI now tells you when a newer version exists | Generally available |

### About the early preview features

Command names, flags and file formats may still change in these three areas, so we would hold
off building automation on them for now. If you run into anything, please tell us.

### Added

- **`ags extend` (early preview).** The commands from `extend-helper-cli` are now available
  inside AGS CLI, so one tool covers both.
- **`ags ams upload` (early preview).** The AMS CLI image upload, so a dedicated-server build
  becomes an AMS image without a second tool.
- **Custom workflows in YAML (early preview).** Workflows can be written in your own files,
  registered, and run.
- **Install and upgrade in one command.** Shell, PowerShell and Homebrew installers, built by
  cargo-dist for all seven supported targets.
- **Update hint.** The CLI reports when a newer version exists.
- A one-time hint on first run, pointing at the commands most people need first.
- `ags extend tunnel` and `ags extend remote-debug connect` report session events while they
  run, so a long session shows progress instead of appearing frozen.
- `ags extend remote-debug disable` is a real command and asks for confirmation before it
  restarts a running app. `--yes` skips the question.
- The bundled CSM specification is refreshed to v1.34.0, adding four operations.
- The Extend Helper Service specification is bundled, so its operations are addressable.
- `ags describe` reports file-picker inputs in its envelope, so a tool reading the catalogue
  sees them.

### Changed

- A stored token is now bound to the client that minted it. `ags auth refresh` refuses a token
  minted by a different client and the CLI re-mints instead of reusing one, so switching between
  clients no longer produces confusing authorisation failures.
- The workflow step review honours edits to fields bound to a constant and to fields that come
  from an earlier step, so an edit made in the review screen is no longer discarded.
- Malformed workflow steps return an error instead of stopping the process.
- The workflows directory is created with owner-only permissions, so registered workflow files
  are not readable by other users on a shared machine.
- Docker output shown by a workflow step is sanitised before display, so control characters in
  a container's output cannot alter your terminal.
- Anonymous usage measurement is built in and active in this release, with values redacted by
  default and a `DO_NOT_TRACK` opt-out. See [Telemetry](README.md#telemetry) for what is and
  is not collected.

### Fixed

Most of the fixes in this release are on code that has not been released before, so they are not
listed. These are the fixes to behaviour that existed in 0.4.0:

- Workflow step review edits were discarded for constant-bound fields and for fields taken from
  an earlier step.
- A malformed workflow step could stop the process instead of reporting an error.
- The workflows directory was created with default permissions rather than owner-only.

### Install

```sh
# macOS and Linux
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/AccelByte/accelbyte-ags-cli/releases/latest/download/accelbyte-ags-cli-installer.sh | sh

# Windows, in PowerShell
powershell -ExecutionPolicy Bypass -c \
  "irm https://github.com/AccelByte/accelbyte-ags-cli/releases/latest/download/accelbyte-ags-cli-installer.ps1 | iex"

# macOS and Linux, with Homebrew
brew install accelbyte/tap/ags-cli
```

The installers place `ags` in `~/.cargo/bin` on macOS and Linux, or
`%USERPROFILE%\.cargo\bin` on Windows. The archive downloads are unchanged — the installers are
an addition, not a replacement.

### Known issues

- **The binaries are not signed.** macOS shows a Gatekeeper warning and Windows shows a
  SmartScreen warning the first time you run `ags`. On macOS, open System Settings, Privacy and
  Security, and choose Open Anyway. On Windows, choose More info and then Run anyway.
- **`docker login` fails on native Windows with Docker Desktop.** The Windows credential store
  cannot hold a secret larger than 2,560 bytes and the token is larger than that, so
  `ags extend docker-login` and `ags extend image-upload --login` both fail. 0.5.0 explains the
  cause when it happens. Use WSL2 as the workaround.
- **`ags extend remote-debug connect` can report an expired refresh token.** Seen on one Windows
  setup where `ags auth status` reported both tokens valid and `ags extend tunnel` worked against
  the same app. Not reproduced elsewhere. Log in again, and report it if it persists.
- **0.4.0 does not know that 0.5.0 exists.** The update hint was added after 0.4.0 was released,
  so a 0.4.0 installation will never announce this release. Upgrade using one of the commands
  above. From 0.5.0 onward the hint works.

### Compatibility

- No command that existed in 0.4.0 has been removed or renamed.
- Migration shortcuts under `ags extend` are a convenience for people moving from
  `extend-helper-cli`. The `ags csm …` address each one forwards to is the address to use in
  scripts.
- **Archive names and format have changed, so an existing download script needs updating.**
  0.4.0 published `ags-<target>.tar.gz` with the binary at the archive root. 0.5.0 publishes
  `accelbyte-ags-cli-<target>.tar.xz` — a different name, compressed with xz rather than gzip,
  and extracting into a directory named after the archive, with `ags` inside it. The Windows
  `.zip` is the exception: it still holds `ags.exe` at the root. A `.sha256` still sits beside
  every archive, and a combined `sha256.sum` is now published too.
- A workflow file that does not declare `workflow_protocol_version` still loads and runs.
  `ags workflow run` prints a short note saying the file predates the field, which you can
  silence by adding it. Nothing is rejected.
