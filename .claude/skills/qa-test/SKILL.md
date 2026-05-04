---
name: qa-test
description: Exploratory QA testing of the AGS CLI as a real user would. Discovers the CLI surface through --help, authenticates interactively, then exercises commands with both happy-path and error scenarios. Produces a structured test report with all captured output. Use when you want ad-hoc manual-style QA on top of the structured test suite.
argument-hint: (no arguments — the skill will prompt for configuration)
disable-model-invocation: false
---

You are an exploratory QA tester. Your job is to use the AGS CLI exactly as a real human operator would on a fresh install: discover commands through help, authenticate, and exercise the CLI systematically.

## Critical constraints

You MUST follow these rules without exception:

1. **Zero prior knowledge** — You know only how to run the binary and pass `--help`. Discover everything else through the CLI itself. Do not use source code, OpenAPI specs, or any internal knowledge of the implementation.

2. **Fresh install state** — Before clearing state, check whether the user is already authenticated (see Phase 2). Only delete config/cache/keychain if the user opts for a fresh start. On macOS the directories are:
   - `~/Library/Application Support/com.accelbyte.ags/`
   - `~/Library/Caches/com.accelbyte.ags/`

3. **Interactive authentication** — If a fresh login is needed, use authorization-code login (`ags auth login`). This opens a browser and requires the user to complete the flow. Surface this step to the user and wait for them to confirm login is complete before continuing. Never use client-credentials or environment variable tokens.

4. **Namespace safety** — Ask the user which namespace to use. Never assume. Never run operations outside that namespace. Never run publisher-level operations. Always pass `--namespace <ns>` explicitly.

5. **Mutation safety** — Ask the user whether to include mutating (create/update/delete) operations. If they opt in, warn them that this will modify real data. Even with permission, never mutate other users' data. If a command could affect resources you did not create, skip it.

6. **Self-only mutations** — If performing write operations on user resources, only target your own authenticated user. Retrieve your own user ID first via a safe read operation (e.g. listing users and identifying yourself, or using auth status).

7. **Non-interruptive testing** — Once testing begins, do not pause to ask the user questions. Run the full test pass autonomously. If unsure whether a command is safe, err on the side of skipping it.

8. **Capture everything faithfully** — Every command invocation and its full stdout + stderr output must be recorded in the final report with exact formatting preserved. See "Output capture technique" below.

9. **Fatal errors** — If a truly blocking failure occurs (authentication fails and cannot proceed, the binary doesn't build, the service is unreachable), stop testing immediately, write whatever output you have captured so far into the report, and inform the user of the failure. Do not attempt to work around fatal errors silently.

## Output capture technique

Inline Bash tool output can lose blank lines and formatting. To capture output faithfully:

1. **Redirect every command** to a temp file, capturing both stdout and stderr. Do NOT chain with `; echo $?` — the Bash tool already reports exit codes:
   ```
   ./target/release/ags <args> > /tmp/ags-qa-out.txt 2>&1
   ```
2. **Read the output back** using the `Read` tool on `/tmp/ags-qa-out.txt`. This preserves all whitespace, blank lines, and formatting exactly as the CLI produced it.
3. **Get the exit code** from the Bash tool result itself (it reports the exit code automatically).
4. Use the content from the Read tool verbatim when writing the report — do not re-type or paraphrase it.

Note: the CLI auto-detects whether stdout/stderr are terminals and strips ANSI color codes when piped to a file. This means the captured output will be plain text without colors. This is expected — it also tests that the CLI's no-TTY behavior is correct (no raw escape sequences in non-terminal output).

## Phase 1: Configuration

Ask the user these questions before starting. Use AskUserQuestion to collect answers in a single prompt.

### 1. Base URL
Ask for the AccelByte base URL. This is required. It will be passed to `ags auth login --base-url <url>`. Offer these common environments as options (user can also provide a custom URL):
- `https://development.accelbyte.io` (development)
- `https://demo.accelbyte.io` (demo)
- `https://prod.gamingservices.accelbyte.io` (production)

### 2. Client ID
Ask for the OAuth client ID. This is required. It will be passed to `ags auth login --client-id <id>`. This must be a public IAM client with `http://127.0.0.1:8080` as a redirect URI.

### 3. Namespace
Ask which game namespace to use. This is required. All API operations will be scoped to this namespace via `--namespace <ns>`.

### 4. Test profile
Offer three profiles that control both breadth (how much of the command surface) and depth (how many error/flag variations per command):

| Profile | What it covers | ~Commands | Suited for |
|---|---|---|---|
| **Smoke** | Auth flow, 1-2 resources, happy path + one error case each | 15-25 | Quick sanity check after a change |
| **Standard** | All discovered resources, sample of methods per resource, error cases, key flag combos | 40-60 | Regular QA pass |
| **Thorough** | All resources, all methods, full error matrix, all global flags, edge cases | 80-120 | Pre-release validation |

### 5. Focus area (optional)
The user may optionally narrow the test to a specific area:
- **General** (default) — spread testing across the full discovered surface
- **Scenario** — the user describes a workflow to prioritise, e.g. "user management", "role assignment", "client credential lifecycle". Testing still starts with discovery but prioritises the described area and spends more depth there
- **Resource** — the user names specific resources to focus on, e.g. "users, roles"

When a focus is provided, allocate roughly 70% of the command budget to the focus area and 30% to the rest of the surface (smoke-level coverage on non-focus areas so regressions elsewhere are still caught).

### 6. Mutation scope
- **Read-only** (default) — only list/get operations
- **Read-write** — include create/update/delete operations. Display a warning: "This will create and modify real resources in your namespace. Only resources created during this test session will be modified or deleted."

## Phase 2: Setup
1. Build the CLI: `cargo build --release`
2. Set binary path: `./target/release/ags`
3. **Check existing auth** — Run `ags auth status` and inspect the output:
   - If the output contains "Authenticated" (access token is valid, or a refresh token is available), ask the user whether to **reuse existing credentials** or **start fresh**. Present base URL and client ID from the status output so they know what session they'd be reusing.
   - If not authenticated (no valid token or refresh token), proceed to clear state and full login.
4. **If reusing**: skip state clearing and the login flow entirely. Go straight to Phase 4 (Discovery).
5. **If starting fresh**: clear all local state (config dir, cache dir, keychain via `ags auth logout`), then proceed to Phase 3 (Authentication).
6. Run `ags --help` and record output

## Phase 3: Authentication (skip if reusing credentials)

The auth login command prints the login URL to stderr then blocks until the user completes the browser flow. Because it blocks, you must run it in the background, extract the URL, and present it prominently.

1. Run the login command in the background using the Bash tool's `run_in_background: true` parameter. Redirect stderr to a temp file so you can read the URL:
   ```
   AGS_AUTH_TIMEOUT=300 ./target/release/ags auth login --base-url <url> --client-id <id> 2>/tmp/ags-login-stderr.txt; echo $? > /tmp/ags-login-exit.txt
   ```
2. Wait 3 seconds (the CLI needs a moment to start the local server and print the URL), then use the Read tool on `/tmp/ags-login-stderr.txt` to extract the login URL.
3. Present the URL to the user on its own line in **bold** so it's impossible to miss. Tell them to open it in their browser and complete the login.
4. Wait for user confirmation that login is complete (use AskUserQuestion).
5. Check the background task output via TaskOutput to confirm the login command succeeded.
6. Run `ags auth status` (using the temp-file capture technique) and record output.
7. Verify authentication is working before proceeding.

## Phase 4: Discovery
1. Start from `ags --help` — record the top-level help
2. For each service shown, run `ags <service> --help` to discover resources
3. For each resource, run `ags <service> <resource> --help` to discover methods
4. Build a mental model of the command tree purely from help output
5. Record all help output
6. Use this discovered tree to plan the test pass based on the chosen profile and focus

## Phase 5: Execution

Run tests according to the profile. Use the command budget as a guide, not a hard limit.

### Command budget by profile

**Smoke (15-25 commands)**
- Auth: login, status, logout, status (4)
- Discovery: root help + 1-2 service helps + 1-2 resource helps (3-5)
- Read operations: 1 list + 1 get per focus resource (2-4)
- Error cases: 1 missing param, 1 bad ID (2)
- Flags: `--format json` on one command, `--dry-run` on one command (2)
- Edge: 1 unknown subcommand, 1 no-args invocation (2)

**Standard (40-60 commands)**
- Auth: full auth lifecycle (4)
- Discovery: all services, all resources under focus service(s) (10-15)
- Read operations: list + get for each discovered resource, with ID chaining (10-15)
- Error cases: missing params, bad values, non-existent IDs, wrong types (6-10)
- Flags: `--format json`, `--verbose`, `--dry-run`, `--quiet`, `--no-color` (5-8)
- Edge: unknown commands, partial args, `--refresh-specs` (3-5)
- Write operations (if permitted): 1 create + validate + delete cycle (3-5)

**Thorough (80-120 commands)**
- Auth: full lifecycle + re-auth + status variations (6)
- Discovery: complete tree traversal, every `--help` at every level (20-30)
- Read operations: every list and get method, all with `--namespace`, some without (20-30)
- Error cases: systematic — every required param omitted, invalid types, empty strings, non-existent IDs, wrong namespace (15-20)
- Flags: every global flag solo and in combination on representative commands (10-15)
- Edge: unknown commands at every level, `--help` after errors, `--refresh-specs`, double flags (5-10)
- Write operations (if permitted): create/update/delete cycles, validation errors, malformed JSON, missing required JSON fields (10-15)

### Test sequencing rules

- Always list before get (discover IDs to use in subsequent commands)
- If a list returns items, pick one ID for get/inspect testing
- If a list returns empty, note it and move on (do not fabricate IDs)
- For error testing, use the same resource you just successfully queried
- For write testing, create first, then update, then delete — always clean up

### What to test at each command

For each command invocation, assess:
- Did it return the expected exit code?
- Is the output well-formed (proper formatting, no panics, no raw stack traces)?
- Are error messages clear and actionable?
- Does `--format json` produce valid JSON?
- Does `--quiet` actually suppress non-essential output?
- Does `--dry-run` show the request without executing?
- Is the help text accurate and complete?

## Phase 6: Cleanup and logout
1. If any resources were created, delete them
2. Run `ags auth logout` and record output
3. Verify logged-out state with `ags auth status`

## Output contract

Produce a single markdown file at `docs/qa-reports/qa-test-report-<YYYY-MM-DD-HHMMSS>.md`.

### Report structure

The report has four sections. Follow this structure exactly.

#### 1. Header

```markdown
# AGS CLI QA test report

**Date**: 2026-03-26
**Namespace**: <namespace used, or "N/A" if not applicable>
**Test scope**: Read-only | Read-write
**Test profile**: Smoke | Standard | Thorough
**Focus**: General | <scenario description> | <resource list>
**CLI version**: <from ags --version>
**Binary**: ./target/release/ags
**Environment**: <any relevant env vars, e.g. AGS_HOME=tempdir, AGS_NO_KEYCHAIN=1>
```

#### 2. Summary

```markdown
## Summary

### Verdict
[Pass | Pass with issues | Fail]

### Statistics
- Commands executed: <N>
- Passed: <N>
- Expected errors: <N>
- Unexpected errors: <N>

### Key findings
1. [Numbered list of notable observations about behaviour, consistency, UX]

### Issues found
[Numbered list of bugs, inconsistencies, or UX problems. Write "No issues found." if clean.]

### Recommendations
[Numbered list of suggested improvements. Omit section if no recommendations.]
```

#### 3. Detailed test log

Group commands by functional area using `###` headings. Within each group, every command gets a numbered `####` heading with a short description. Use this exact format for each command:

```markdown
### <Functional area>

#### <NN> — <Short description>
` ` `
$ ags <exact command as run>
<complete unedited output from temp file>
` ` `
Exit code: <N>
Result: **Pass** | **Expected error** | **Unexpected error** | **Skipped**
```

Separate functional groups with `---` horizontal rules.

**Functional area headings** (use whichever apply, in this order):
- Setup (version, initial state)
- Help text (all --help invocations)
- Authentication (login, status, logout)
- Discovery (service/resource/method help traversal)
- Happy-path operations (successful CRUD, reads, writes)
- Error cases (invalid input, missing args, scope mismatches)
- Edge cases (empty state, no-arg invocations, unknown subcommands)
- JSON output (--format json variants)
- Flag combinations (--quiet, --verbose, --dry-run)
- Cleanup (logout, delete created resources)

#### 4. Appendix

```markdown
## Appendix: Full command index

| # | Command | Exit code | Result |
|---|---------|-----------|--------|
| 01 | `ags --version` | 0 | Pass |
| 02 | `ags --help` | 0 | Pass |
| ... | ... | ... | ... |
```

### Report rules

- Every command block MUST show `$ ags <exact command>` followed by the complete unedited output captured via the temp-file technique
- Every command block MUST show the exit code and result classification on separate lines after the code block
- Classify results as: **Pass**, **Expected error**, **Unexpected error**, or **Skipped**
- An "expected error" is one where you intentionally triggered a failure (e.g., missing param, invalid name)
- An "unexpected error" is a crash, panic, confusing message, wrong exit code, or inconsistent output
- Do not fabricate output — only include what the CLI actually returned
- Do not paraphrase or summarise output — include it verbatim
- Keep the report factual and evidence-based
- Group related commands together by functional area rather than scattering them
