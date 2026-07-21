---
name: vhs-demo
description: Author a VHS demo (tape + mock + runner) for an ags command or workflow, grounded in `ags describe` and `--dry-run --format json`. Produces convention-correct artifacts under demos/<name>/ and hands off the record command; never runs vhs itself. Use when asked to create or update a demo GIF/recording of the CLI.
argument-hint: <demo-name> — then describe the command(s)/workflow and surface mode
disable-model-invocation: false
---

You author a VHS demo for the AGS CLI: a `.tape`, a mock routing table, and the
runner wiring, all under `demos/<name>/`. You **never run `vhs`** — you produce
artifacts and hand off the exact record command. Correctness comes from grounding
every command, input, endpoint, and response in the CLI's own introspection, not
from guessing.

## Inputs to collect first

1. **Demo name** (`<name>`, kebab-case) → artifacts live in `demos/<name>/`.
2. **What to show:** the single command(s) (`ags <service> <resource> <method>`)
   and/or a workflow (`ags workflow run <id>`), with the concrete input values.
3. **Surface mode** (exactly one): `no-ui` (all inputs as flags, deterministic),
   `plain` (`--ui plain`, line prompts), `inline` (`--ui inline`), or `fullscreen`
   (`--ui fullscreen`).

## Procedure

1. **Ground inputs.** Run `ags describe <service> <resource> <method>` (single) or
   `ags describe workflow <id>` (workflow). Use this for exact flag names (kebab),
   types, required/default, enum values, and — for workflows — the input order and
   the step list (`service` + `operation` per step).
2. **Ground endpoints.** Run the matching dry-run JSON:
   - Single: `ags <service> <resource> <method> --dry-run --format json --<flags>`
   - Workflow: `ags workflow run <id> --dry-run --format json --<inputs>`
   Parse the request (`method`, `url`, `body`) / `steps[]`. Strip each `url`'s query
   string — the mock matches path only.
3. **Build `demos/<name>/<name>.routes.json`.** One route per request/step plus the
   OAuth token route. For each route's response `body`, run
   `python3 demos/engine/spec-reader.py <service> <x-operation-id>`. Read the
   operation-id from the right field: for a **single command** it is the
   per-contract `x_operation_id` (nested at
   `data.scopes.<scope>.contracts.<version>.x_operation_id`, e.g.
   `social/admin/stat-definitions/v1/create`) in `ags describe <service>
   <resource> <method> --format json`; for a **workflow step** it is the step's
   `operation` field from `ags
   describe workflow <id> --format json` (e.g. `iam/admin/roles/v4/list`) — NOT the
   step's `service` or `id` (the short label). On a non-zero exit, fall back to a
   minimal payload (`{}` or `{"id": "..."}`); if a downstream workflow step binds a
   captured field, add just that field with a plausible value. Give the token route
   `delay: 1.2` and others `delay: 0.6`.
4. **Build `demos/<name>/<name>.tape`** for the surface mode (see below), using the
   fixed look + timing conventions.
5. **Build `demos/<name>/prewarm`** — one service *display* name per line for every
   service the demo touches (for a workflow, map each step's `service` to its CLI
   display name via `ags describe`). This warms specs so the first on-camera command
   shows no "Preparing specs…". For a non-dry-run demo that should NOT show auth,
   also drop an empty `demos/<name>/prelogin` marker so `record.sh` logs in
   off-camera (see Auth bootstrap) — and do not put a login in the tape.
6. **Scaffold the engine on first use.** If `demos/engine/spec-reader.py`,
   `demos/engine/mock-server.py`, or `demos/record.sh` are missing, copy them from
   the `onboarding` reference (they already exist once this skill has shipped).
7. **Hand off.** Print `demos/record.sh <name>` and the requirements (cargo,
   python3, vhs). For `inline`/`fullscreen`, also print the timing caveat and the
   404-loop instruction (below).

## Mandatory conventions (every generated demo must meet these)

### Fixed look (baseline for all demos)
```
Set Theme "Dracula"
Set FontSize 14
Set Width 1200
Set Height 700
Set TypingSpeed 55ms
Set PlaybackSpeed 1.0
```
**Viewport override for inline-vs-fullscreen demos.** A demo that shows the
`inline` and `fullscreen` surfaces side by side needs a *taller* viewport
(e.g. `Width 1400`, `Height 1000`) — inline renders in a fixed-height region, so
on a small terminal it fills the screen and looks almost identical to fullscreen.
A taller terminal makes inline occupy a visibly smaller fraction. Single-surface
demos keep the baseline size.

### Isolation header (every tape)
```
Env AGS_BASE_URL "http://localhost:8765"
Env AGS_HOME "/tmp/ags-demo-state"
Env AGS_NO_KEYCHAIN "1"
```

### Auth bootstrap (every non-dry-run demo)
A fresh `AGS_HOME` starts unauthenticated, so a live command/workflow needs a token
first or it fails.

**Default: authenticate off-camera via `record.sh`, NOT in the tape.** Drop a
`demos/<name>/prelogin` marker file; `record.sh` then runs `ags auth login --grant
client-credentials --client-id demo --client-secret-stdin` (against the mocked
token route) into the same throwaway `AGS_HOME` before recording starts. The token
persists, so every section runs authenticated and **no login ever appears in the
GIF**. This is more reliable than VHS `Hide`/`Show`, which has been observed to
leak the login into the recording — prefer `prelogin`.

Show auth **on-camera only when authentication is itself the subject of the demo**
(e.g. an onboarding/login reel): omit the `prelogin` marker and `Type` the login
as a visible step in the tape. Dry-run-only demos skip auth entirely (no live
call).

### Timing
`Sleep` between logical steps sized to read time: ~`1500ms` for a short result,
~`3s`-`4s` for a table or multi-line result. `Ctrl+L` between sections to clear.
After a printed section header (`Type "# N. …"` + `Enter`), add a second `Enter`
so a blank line separates the header from the command beneath it.

**Interactive forms read far faster on playback than they feel while authoring** —
pace them deliberately. Settled values from the workflow-surfaces demo:
- **Per-step review pause ~`2800ms`.** A multi-step workflow auto-advances each
  per-step confirm with an `Enter` from the tape; without a generous pause the
  Continue fires before the viewer can register the step. Hold ~2.8s per step.
- **Post-navigation delay ~`700ms`.** After a `Tab` (or `Tab N`) that moves focus,
  pause before typing so the focus move registers before input appears.
- **Pre-input settle ~`2s`-`3s`** after launching a TUI (briefing/first frame) and
  ~`900ms` between successive `plain` line answers.
- **Final result pause ~`6s`-`7s`** after a workflow completes so the result/summary
  is readable before `Ctrl+L`.

## Surface modes

- **no-ui** — type the full flagged command (`--yes` for confirm-gated workflow
  steps), `Enter`, `Sleep`. Optionally also show `--dry-run` and/or `--format json`
  variants. Fully deterministic.
- **plain** — append `--ui plain`. The CLI prompts line-by-line; the tape `Type`s
  each value in `ags describe` order, `Enter` per value.
- **inline** — append `--ui inline`. Keystroke-driven; ground the sequence in the
  describe field order. Inline has **no dynamic-enum picker** (`options_source`
  inputs are plain text), so no option endpoints are needed.
- **fullscreen** — append `--ui fullscreen`. Keystroke-driven; ground in field
  order + the nav keymap. Fullscreen **does** enable the dynamic-enum picker for
  `dynamic: true` inputs — either pre-supply those inputs as flags to skip it, or
  drive filter+select keystrokes and add the picker's options `GET` to the routes
  (found via the 404 loop).

**For `inline` and `fullscreen`:** verify the keystroke sequence headlessly under a
pty first (see "Verifying interactive sections" below) so it reaches completion;
recording is then only for tuning `Sleep` durations and confirming the visual
layout (e.g. that inline reads differently from fullscreen).

### Demonstrating `--ui auto` (surface chosen by missing-input count)
`--ui auto` (the default when no `--ui` is passed) picks the surface from how many
**required** inputs are still missing after flag resolution — so one command can be
shown three ways by varying only which flags you supply, with no `--ui` anywhere:
- **0 missing → runs unattended.** Auto resolves to plain, but there is nothing to
  prompt, so the command executes straight to the result.
- **1–2 missing scalars → plain.** Auto resolves to plain and prompts line-by-line
  for the missing scalars.
- **a missing *structured* body field (object/array), or ≥3 missing scalars →
  inline** (the `Form` shape).

What counts as a scalar (`classify_service_like` + `is_body_field_input`): a
**structured** body field — an object or array, the same fields the inline form
opens in the JSON editor — sets "has body field" and forces inline. **Scalar**
body fields (string / number / bool / enum) instead count as `required_scalars`,
because plain can prompt them line-by-line; path/query params are always scalars.
`--json` pre-supplies the *whole* body at once (clearing both counts) — there are
no per-body-field flags. Consequences:
- To show all three branches from ONE command it needs **≥1 required scalar param
  (path/query) AND a body**: supply both → unattended; omit the scalar(s) but keep
  `--json` → plain; omit `--json` → inline (the missing body makes it a `Form`). A
  **body-only** command (no scalar param, e.g. `iam roles create`) can only ever
  show unattended-vs-inline — it cannot reach the plain middle case.
- The inline branch needs a **structured** (object/array) body field *or* ≥3
  missing scalars; a command whose only missing body field is a single scalar
  lands in plain, not inline.
- `--format json` forces the automation consumer (JSON, never prompts), and an
  auto-selected inline/fullscreen degrades to plain when stdin+stderr aren't both
  TTYs — neither is the "auto by missing count" behaviour, so don't conflate them.

See `demos/auto-surfaces/` for a worked example (`iam roles add-permissions`:
role-id scalar + a `permissions` array body).

## Verifying interactive sections headlessly (do this BEFORE recording)
**Every surface — plain, inline, fullscreen — can be verified without `vhs`** by
driving the real binary under a Python pseudo-terminal and checking the run reaches
"completed". This is the single most valuable step: it confirms the keystroke
sequence is correct so a section can't run long and leak keystrokes into the next
one (the #1 demo failure). The driver:

1. `pid, fd = pty.fork()`; in the child `os.execv` the `ags` workflow run.
2. Set a window size: `fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))` — without a size the TUI bails.
3. **Answer the cursor-position query.** crossterm (inline/fullscreen) writes `ESC[6n` (DSR) and waits; if you don't reply it errors with "cursor position could not be read". Whenever the master output contains `b"\x1b[6n"`, write back `b"\x1b[1;1R"`. (plain doesn't query, but answering is harmless.)
4. Start the mock with the demo's routes, `auth login` off-camera first, then feed keystrokes (`\r` Enter, `\t` Tab, `\x13` Ctrl+S, literal chars) with small waits and read between sends.
5. Strip ANSI from the transcript and assert it contains "completed".

Two gotchas this reveals (and that bite the tape if missed):
- **Briefing screen.** A workflow with a `briefing` shows it FIRST on inline/
  fullscreen (an `[Enter] continue` screen); `plain` skips it. So the tape's first
  inline/fullscreen keystroke is `Enter` to dismiss the briefing, *then* the form.
- **Form keymap.** `Enter` begins editing the focused field and commits it; `Tab`
  moves to the next field; `Ctrl+S` submits. Default fields are skipped by tabbing
  past. In `fullscreen`, the dynamic-enum fields open a picker on `Enter` — type to
  filter, `Enter` to select (the options come from the mocked endpoint). A `plain`
  run prompts line-by-line only when stdin AND stderr are TTYs (a pipe hits the
  no-input precheck), so the pty is required there too.
- **JSON editor (object/array body fields).** A body field that is an object or
  array renders as a `JsonBody` form field; `Enter` on it opens the structured JSON
  editor. Tree keymap: `↑↓` move, `→` expand / `←` collapse, `Enter` edit a scalar
  (toggles a bool / cycles an enum in place), `+`/`−` add/remove an array entry,
  `Ctrl+R` raw view, `Ctrl+S` save, `Esc` cancel. Both modes are drivable; the
  catch in each:
  - **Structured (tree).** `+` adds an entry but leaves focus on the *array root*,
    not the new entry — so after `+` you must `Down` onto the entry, `Right` to
    expand it, `Down` onto a field, `Enter` to open its scalar editor, `Type`,
    `Enter` to commit. (Getting this wrong is why edits "silently miss": you were
    editing the wrong node.) Scalar-edit keymap: type a value, `Enter` save, `Esc`
    cancel.
  - **Raw.** `Ctrl+R` seeds a pretty-printed buffer; the cursor starts *before*
    the seed, so `Right` moves past the opening `[`/`{`. `Enter` inserts a newline
    (no auto-indent) and `Tab` inserts two spaces, so an inserted entry can be
    hand-indented to match the seed's 2-/4-space formatting. Add a trailing `,`
    when inserting before an existing entry. `Ctrl+S` saves. (VHS has no
    `Home`/`End` keys — use `Right`/`Left`; since the seed's first line is just
    `[`, one `Right` already reaches its end.)
  After the editor, `Ctrl+S` submits the form. Empty required fields are dropped on
  save, so the JSON must be valid or the later submit fails. `demos/auto-surfaces/`
  shows both modes (structured entry, then a raw entry) — and `--ui auto` routes
  such a command to inline because the missing object/array body sets "has body
  field" (see the auto-mode section above).

Use the pty to read the exact prompt/field order and confirm completion for each
interactive section; then translate the verified key sequence into tape commands
and record. Recording is still where you tune `Sleep` durations and confirm the
visual layout (e.g. inline vs fullscreen distinctness) — but the keystrokes should
already be correct.

## The authoring loop (workflows / TUI)
The mock returns `404` and logs any unmatched request. After the first recording
attempt, read the mock's stderr log: any logged 404 (commonly a fullscreen dynamic-
enum options fetch) is an endpoint to add to `<name>.routes.json`, then re-record.

## Response models
Response bodies come from `demos/engine/spec-reader.py`, which reads the bundled
OpenAPI specs (`ags describe` does not expose response schemas — that is a known
gap). Do not hand-write response shapes when the spec-reader can supply them.
