# Changelog

All notable changes to the AGS CLI are recorded here. This file starts at 0.5.0; for
earlier versions see the [releases page](https://github.com/AccelByte/accelbyte-ags-cli/releases).

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

### What "early preview" means here

An early preview feature is complete enough to use and to give feedback on. It is not covered
by the usual promise that command names, flags and file formats stay stable. The commands work
and are supported, and problems you report are treated as bugs. What is not promised is that a
command will keep its address, that flags will keep their names, or that a file you write today
will load unchanged in the next release.

Our recommendation: use early preview features interactively, and do not build automation on
them yet. When a preview feature changes address or format, the change ships in the next release
without a deprecation period.

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
- Archive file names and checksums keep the same shape as 0.4.0, so an existing download script
  continues to work.
- A workflow file that does not declare `workflow_protocol_version` still loads and runs.
  `ags workflow run` prints a short note saying the file predates the field, which you can
  silence by adding it. Nothing is rejected.
