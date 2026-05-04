# AGS CLI Output Reference

Version: 1.0  
Status: Draft  
Scope: Human-readable terminal output for the AGS CLI

## 1. Overview

This reference defines the formatting, tone, hierarchy, and rendering rules for human-readable CLI output.

The CLI is service-backed. Output MUST therefore optimize for:

- clear operation results
- consistent diagnostics
- actionable recovery guidance
- readable presentation of service data
- stable formatting across commands and services

This reference applies to default terminal output. It does not replace machine-readable output modes such as JSON.

## 2. Design principles

The CLI output system MUST satisfy the following principles:

1. **Outcome first**  
   The first line MUST tell the user what happened or what state was detected.

2. **Actionable failures**  
   Errors SHOULD include a likely fix whenever one can be reasonably inferred.

3. **Readable without colour**  
   Output MUST remain understandable in plain text and no-colour terminals.

4. **Message-first formatting**  
   Human-readable output MUST not default to raw transport JSON.

5. **Consistent hierarchy**  
   Similar message types MUST use the same symbols, labels, indentation, and ordering.

6. **Compact by default**  
   Default output SHOULD be concise. Lower-priority diagnostics SHOULD be omitted or moved to verbose modes.

## 3. Output modes

The CLI MUST support at least these conceptual output modes:

- **human-readable mode**: default
- **machine-readable mode**: exact structured payload, e.g. `--format json`

Human-readable mode and machine-readable mode MUST be distinct.

In machine-readable mode, the CLI SHOULD emit raw structured output without human-oriented symbols or formatting.

## 4. Message taxonomy

The CLI MUST support these message classes:

- Error
- Warning
- Status
- Success
- Info
- Detail
- Fix
- Tip

`Fix` is a rendering role, not a standalone severity.
`Tip` is a rendering role, not a standalone severity.

### 4.1 Intended use

**Error**  
The command could not complete its intended outcome.

**Warning**  
The command completed or continued, but some non-fatal issue, fallback, or incomplete state exists.

**Status**  
An operation is in progress.

**Success**  
A mutation or requested action completed successfully.

**Info**  
Neutral factual or inspect-style output.

**Detail**  
Subordinate technical or supporting context.

**Fix**
A recommended recovery or next-step action.

**Tip**
A helpful hint or contextual suggestion. Same visual weight as Detail: indented by 4 spaces and rendered dimmed. No symbol prefix.

## 5. Symbols and labels

### 5.1 Standard symbols

The CLI SHOULD use the following symbols in human-readable mode:

- Error: `✕`
- Warning: `!`
- Status: `∘`
- Success: `✔`
- Info: `›`
- Fix: `→`

If symbols are disabled, labels alone MUST preserve meaning.

### 5.2 Standard labels

The primary message types (Error, Warning, Success) do NOT use text labels — the symbol and colour are sufficient. The CLI MUST use these canonical labels for subordinate lines:

- `Reason:`
- `Detail:`
- `Fix:`
- `Next:`
- `Tip:`

For inspect/state fields, field labels MUST use `Label: value` format.

Examples:
- `Profile: default`
- `Namespace: studio-live`
- `Access token: stored`

## 6. Colour and emphasis

Colour MAY be used sparingly to reinforce meaning.

Colour MUST NOT be the only signal for category or severity.

Recommended mapping:

- Error: red
- Warning: yellow
- Status: cyan or blue
- Success: green
- Info: default or light blue
- Detail: dim/gray

If a no-colour mode exists, formatting MUST remain readable and structurally identical aside from colour.

## 7. Tone and wording

The CLI tone MUST be:

- clear
- calm
- direct
- operational
- neutral

The CLI SHOULD:

- use specific verbs
- name affected resources explicitly
- avoid blameful language
- avoid unnecessary personality in normal operational output

The CLI SHOULD NOT:

- use theatrical language
- use decorative emoji in standard command output
- use vague language when a concrete explanation is available

Preferred:
- `Failed to create game session.`
- `Access token expired.`
- `Match pool "ranked-eu" not found.`

Avoid:
- `Oops! Something went wrong.`
- `An unexpected issue occurred while processing your request.`

## 8. Layout and indentation

### 8.1 Primary hierarchy

The CLI MUST render primary and secondary information using this hierarchy:

- primary line begins at column 0
- subordinate explanation lines are indented
- fix line returns to column 0 and uses the fix symbol

### 8.2 Indentation rules

The CLI MUST indent `Reason:` and `Detail:` lines by exactly 4 spaces in expanded multi-line messages.

Example:

```text
✕ Failed to update leaderboard config.
    Reason: Revision is out of date.
    Detail: HTTP 409 Conflict.
→ Fix: Fetch the latest config and retry with the new revision.
```

### 8.3 Fix alignment

`Fix:` MUST be aligned with the primary line and MUST NOT be indented as subordinate detail.

Rationale: `Fix` is first-class actionable guidance, not supporting context.

### 8.4 No blank line before fix

The CLI SHOULD NOT insert a blank line before `Fix:` or `Next:`. The fix/next line MUST be adjacent to the preceding content, whether that is the primary line, a `Reason:`, or a `Detail:` line.

This applies to both compact and expanded multi-line messages.

Compact example:

```text
✕ Namespace "prod" not found.
→ Fix: Run 'ags namespace list' to see available namespaces.
```

Expanded example:

```text
✕ Failed to update leaderboard config.
    Reason: Revision is out of date.
    Detail: HTTP 409 Conflict.
→ Fix: Fetch the latest config and retry with the new revision.
```

## 9. Error formatting

### 9.1 Requirements

Error output MUST:

- state what failed
- include `Reason:` when the cause is known and useful
- include `Fix:` whenever a likely recovery step is available

Error output SHOULD:

- include technical `Detail:` only when it adds user value
- omit low-value transport noise in default mode

### 9.2 Canonical formats

Expanded error:

```text
✕<what failed>.
    Reason: <why>.
    Detail: <optional technical context>.
→ Fix: <recommended next step>.
```

Compact error:

```text
✕<what failed>.
→ Fix: <recommended next step>.
```

### 9.3 Error examples

```text
✕ Failed to log in.
    Reason: Invalid client credentials.
→ Fix: Check your client ID and secret, then run 'ags auth login' again.
```

```text
✕ Match pool "ranked-eu" not found.
→ Fix: Run 'ags matchpool list' to view available match pools.
```

```text
✕ Request timed out while calling session service.
→ Fix: Retry the command. If the issue persists, check service status or use '--timeout 30s'.
```

## 10. Warning formatting

### 10.1 Requirements

Warnings MUST indicate a non-fatal issue, fallback, or degraded result.

Warnings MAY include `Reason:`.

Warnings SHOULD use `Next:` for advisory follow-up when correction is optional.

Warnings MAY use `Fix:` when corrective action is genuinely needed.

### 10.2 Canonical formats

```text
!<issue>.
    Reason: <optional explanation>.
→ Next: <recommended next step>.
```

or:

```text
!<issue>.
→ Fix: <corrective action>.
```

### 10.3 Warning examples

```text
! Using default namespace "demo".
→ Next: Pass '--namespace <name>' to target a different environment.
```

```text
! Output may be incomplete.
    Reason: One of three regions did not respond.
```

## 11. Status formatting

### 11.1 Requirements

Status messages MUST represent in-progress work.

Status messages SHOULD be short, present-progressive verb phrases.

### 11.2 Canonical format

```text
∘ Authenticating...
∘ Fetching session details...
∘ Waiting for deployment to become ready...
```

### 11.3 Rules

Status messages SHOULD NOT include excessive punctuation, extra explanation, or decorative formatting.

If transient rendering is supported, status lines MAY be overwritten or cleared.

## 12. Success formatting

### 12.1 Requirements

Success messages MUST be used for completed action results, especially mutations.

Success messages MUST NOT be used for passive inspection or state-report commands.

### 12.2 Canonical format

```text
✔<action result>.
    <Field>: <value>
    <Field>: <value>
```

### 12.3 Examples

```text
✔ Namespace "live" created.
    ID: ns-live
    Status: ACTIVE
```

```text
✔ User "player123" banned.
```

## 13. Info and inspect formatting

### 13.1 Requirements

Info formatting MUST be used for neutral informational or inspect-style output.

Inspect outputs SHOULD prefer object/state headings over `Info:` sentence labels.

### 13.2 Canonical format

```text
› <Resource type>
    <Field>: <value>
    <Field>: <value>
```

Example:

```text
› Session
    ID: gs-12345
    Namespace: studio-live
    Status: ACTIVE
    Region: ap-southeast-1
```

### 13.3 Count/list summary format

List-like info output SHOULD begin with a count summary when applicable.

Example:

```text
› Found 3 namespaces
    Name    Status
    live    ACTIVE
    cert    ACTIVE
    dev     INACTIVE
```

List output MUST include a header row showing column names in sentence case, rendered dim. Headers are derived from the selected field labels (e.g. "Client ID", "Display Name").

## 14. Detail formatting

### 14.1 Requirements

`Detail:` lines MUST be subordinate to the main message.

`Detail:` lines MUST be indented by 4 spaces.

`Detail:` SHOULD be omitted in default mode unless it adds practical value.

Highly technical diagnostics SHOULD be reserved for verbose mode.

### 14.2 Examples

```text
    Detail: Request ID 4f2b9a8d1c
    Detail: Returned by iam-service
    Detail: Revision 42
```

## 15. Fix and next-step formatting

### 15.1 Requirements

`Fix:` MUST represent corrective action.

`Next:` SHOULD represent optional or advisory next steps.

Both MUST align with the primary line and begin at column 0.

### 15.2 Canonical formats

```text
→ Fix: Run 'ags auth login'.
```

```text
→ Next: Pass '--namespace <name>' to target another namespace.
```

### 15.3 Quality requirements

A fix SHOULD:

- be concrete
- be specific
- include an exact command when possible
- avoid generic advice unless no better action is available

Preferred:
- `→ Fix: Run 'ags auth login'.`

Avoid:
- `→ Fix: Try again later.` unless that is the best available advice

## 16. Auth status specification

### 16.1 Classification

`auth status` MUST be treated as a state report, not as a success/error action result.

### 16.2 State headings

The CLI MUST use one of the following patterns:

Healthy:
```text
✔ Authenticated
```

Partial/recoverable:
```text
! Authentication requires attention
```

Unavailable:
```text
✕ Not authenticated
```

### 16.3 Healthy example

```text
✔ Authenticated
    Profile: default
    Base URL: https://api.accelbyte.io
    Namespace: studio-live
    Client ID: stored
    Access token: stored
    Refresh token: stored
```

### 16.4 Unhealthy example

```text
✕ Not authenticated
    Reason: No valid access token is stored.
    Profile: default
    Base URL: https://api.accelbyte.io
    Client ID: stored
    Refresh token: not found
→ Fix: Run 'ags auth login'.
```

### 16.5 Partial example

```text
! Authentication requires attention
    Reason: Access token has expired.
    Profile: default
    Base URL: https://api.accelbyte.io
    Namespace: studio-live
    Client ID: stored
    Access token: expired
    Refresh token: stored
→ Fix: Run 'ags auth login' to refresh credentials.
```

### 16.6 Value vocabulary

For auth/config presence reporting, the CLI SHOULD use a constrained vocabulary:

- `stored` — credential is held in the keychain or fallback store
- `present` — value is supplied (e.g. via env var) but its origin is not interrogated
- `valid` — token is present and not expired (suffix `(expires in …)` MAY follow)
- `expired` — token is present but past its expiry
- `not found` — value is absent

Sensitive values MUST NOT be printed in full by default.

## 17. Ragged vs aligned field layout

### 17.1 Default

The CLI MUST default to ragged field labels for message-style output.

Example:

```text
✔ Authenticated
    Profile: default
    Base URL: https://api.accelbyte.io
    Namespace: studio-live
```

### 17.2 Allowed use of alignment

The CLI MAY use aligned columns for:

- table-like list output
- dense inspection output
- views where vertical comparison is important

The CLI SHOULD prefer ragged layout for:

- errors
- warnings
- auth status
- short inspect/state blocks
- message-oriented output

## 18. Rendering successful service responses

### 18.1 Default rule

The CLI MUST NOT dump raw JSON by default in human-readable mode.

Instead, it MUST render:

- a summary headline or object heading
- selected important fields
- optional nested sections for useful subobjects

Raw JSON SHOULD be reserved for explicit structured output mode.

### 18.2 By command intent

#### Action commands

For create/update/delete/mutation commands:

```text
✔ Game session created.
    ID: gs-12345
    Namespace: studio-live
    Status: ACTIVE
    Region: ap-southeast-1
    Created: 2026-03-20T10:23:11Z
```

#### Inspect commands

For get/show/status commands:

```text
› Session
    ID: gs-12345
    Namespace: studio-live
    Status: ACTIVE
    Region: ap-southeast-1
```

#### List commands

For arrays/collections:

```text
› Found 3 namespaces
    Name    Status
    live    ACTIVE
    cert    ACTIVE
    dev     INACTIVE
```

Column headers are sentence case, rendered dim (ANSI dim attribute). Headers are derived from the selected field labels (e.g. "Client ID", "Display Name"). Noun pluralization MUST handle already-plural resource names (e.g. "clients" → "clients", not "clientss").

### 18.3 Nested structures

Nested JSON SHOULD be rendered as sections rather than braces.

Example:

```text
› Dedicated server config
    ID: ds-ranked-ap
    Region: ap-southeast-1
    Status: READY

    Deployment
        Image: accelbyte/ds:v1.8.2
        Replicas: 3
        Port: 7777
```

## 19. Default field selection heuristic

### 19.1 Goal

The CLI MUST support generic rendering without requiring manual formatting for every API endpoint.

### 19.2 Semantic priority order

Top-level fields MUST be ranked using this priority order:

1. identity
2. status
3. scope
4. actor
5. timing
6. secondary
7. diagnostic

### 19.3 Role definitions

**Identity**
- `id`
- `name`
- `slug`
- `key`
- `code`
- `sessionId`
- `fleetId`
- `userId`

**Status**
- `status`
- `state`
- `enabled`
- `active`
- `healthy`

**Scope**
- `namespace`
- `region`
- `environment`
- `profile`
- `organization`

**Actor**
- `userId`
- `clientId`
- `playerId`
- `publisherId`

**Timing**
- `createdAt`
- `updatedAt`
- `expiresAt`
- `deletedAt`

**Secondary**
- `description`
- `mode`
- `platform`
- `type`
- `version`
- `count`
- `revision`

**Diagnostic**
- `requestId`
- `traceId`
- `errorCode`
- `metadata`
- `headers`
- `links`
- `pagination`

### 19.4 Default selection algorithm

In human-readable mode, the formatter MUST:

1. remove null, empty, and unusable values
2. suppress obvious transport/plumbing fields from default output
3. partition the response into:
   - scalar top-level fields
   - small nested objects
   - arrays
   - large complex objects
4. rank top-level scalar fields by semantic priority
5. choose the top fields according to command intent
6. optionally render one or two useful nested sections
7. omit the rest unless verbose mode is enabled

### 19.5 Field count limits

Recommended defaults:

- action result: 4 to 6 fields
- inspect result: 6 to 10 fields
- list result: 2 to 4 columns per row

## 20. Verbose mode guidance

The CLI SHOULD support a verbose mode for additional human-readable diagnostics.

Verbose mode MAY include:

- request ID
- service name
- HTTP status detail
- revision
- suppressed metadata
- additional nested fields

Verbose mode MUST NOT expose sensitive secrets by default.

## 21. Common standardized recovery messages

Where detectable, the CLI SHOULD normalize common failures into standard user-facing patterns.

### 21.1 Auth error

```text
✕ Request was not authorized.
    Reason: Access token expired.
→ Fix: Run 'ags auth login' to sign in again.
```

### 21.2 Permission error

```text
✕ You do not have permission to delete this namespace.
→ Fix: Ask an administrator for the required role or use an account with namespace admin access.
```

### 21.3 Not found

```text
✕ Match pool "ranked-eu" not found.
→ Fix: Run 'ags matchpool list' to view available match pools.
```

### 21.4 Validation error

```text
✕ Invalid value for '--mode': "hardcoree".
→ Fix: Use one of: casual, ranked, hardcore.
```

### 21.5 Timeout/network

```text
✕ Session service did not respond in time.
→ Fix: Retry the command. If the problem continues, check network connectivity or increase '--timeout'.
```

### 21.6 Conflict/stale revision

```text
✕ Update rejected because the resource has changed.
→ Fix: Fetch the latest version and retry your update.
```

### 21.7 Rate limiting

```text
✕ Too many requests sent to iam-service.
→ Fix: Wait a moment and retry. For scripts, add retry with backoff.
```

## 22. Accessibility requirements

The output system MUST:

- remain readable without colour
- preserve meaning when copied into plain text
- avoid symbol-only semantics
- avoid exposing secrets in default output

The output system SHOULD:

- support quiet mode
- support verbose mode
- support machine-readable mode
- keep default output compact enough for CI/log viewing

## 23. Formatter decision flow

The renderer SHOULD follow this decision flow:

1. Determine output mode  
   - if machine-readable: emit exact structured payload
   - else continue

2. Determine command intent  
   - action
   - inspect
   - list
   - state/status

3. Determine message class  
   - error / warning / status / success / info

4. Determine resource family if available

5. Select renderer template  
   - error template
   - warning template
   - status template
   - success template
   - inspect/state template
   - list template

6. Select fields  
   - apply resource-family override if present
   - otherwise use generic field heuristic

7. Render primary line

8. Render indented reason/detail fields if applicable

9. Render fix/next line if applicable

## 24. Suggested pseudocode

```python
def render(result, intent, output_mode="human", resource_family=None, verbose=False):
    if output_mode == "json":
        return emit_json(result)

    if result.is_error:
        return render_error(result, verbose=verbose)

    if result.is_warning:
        return render_warning(result, verbose=verbose)

    if result.is_status:
        return render_status(result)

    profile = get_resource_profile(resource_family)

    if intent == "action":
        fields = select_fields(result.data, profile, intent="action", verbose=verbose)
        return render_success(result.summary, fields)

    if intent in ("inspect", "state"):
        fields, sections = select_fields_and_sections(
            result.data, profile, intent=intent, verbose=verbose
        )
        return render_info_object(result.heading, fields, sections)

    if intent == "list":
        rows, columns = select_list_columns(result.data, profile, verbose=verbose)
        return render_list(result.summary, rows, columns)

    fields, sections = select_fields_and_sections(
        result.data, profile, intent="inspect", verbose=verbose
    )
    return render_info_object(result.heading or "Result", fields, sections)
```

```python
def select_fields(data, profile=None, intent="inspect", verbose=False):
    cleaned = remove_empty_and_noise(data, verbose=verbose)

    if profile and profile.primary_fields:
        ranked = rank_with_profile(cleaned, profile)
    else:
        ranked = rank_by_semantic_priority(cleaned)

    limit = {
        "action": 5,
        "inspect": 8,
        "state": 8,
        "list": 4,
    }.get(intent, 6)

    return ranked[:limit]
```

```python
def render_error(err, verbose=False):
    lines = [f"✕ {err.message}"]

    if err.reason:
        lines.append(f"    Reason: {err.reason}")

    if err.detail and should_show_detail(err.detail, verbose):
        lines.append(f"    Detail: {err.detail}")

    if err.fix:
        lines.append(f"→ Fix: {err.fix}")

    return "\n".join(lines)
```

## 25. Canonical examples

### 25.1 Progress + success

```text
∘ Authenticating...

✔ Logged in as studio-admin.
    Namespace: live
```

### 25.2 Compact error

```text
✕ Namespace "prod" not found.
→ Fix: Run 'ags namespace list' to see available namespaces.
```

### 25.3 Expanded error

```text
✕ Failed to update leaderboard config.
    Reason: Revision is out of date.
    Detail: HTTP 409 Conflict.
→ Fix: Fetch the latest config and retry with the new revision.
```

### 25.4 Warning

```text
! Using default namespace "demo".
→ Next: Pass '--namespace <name>' to target a different environment.
```

### 25.5 Auth healthy

```text
✔ Authenticated
    Profile: default
    Base URL: https://api.accelbyte.io
    Namespace: studio-live
    Client ID: stored
    Access token: stored
    Refresh token: stored
```

### 25.6 Auth broken

```text
✕ Not authenticated
    Reason: No valid access token is stored.
    Profile: default
    Base URL: https://api.accelbyte.io
    Client ID: stored
    Refresh token: not found
→ Fix: Run 'ags auth login'.
```

### 25.7 Successful service response

```text
✔ Game session created.
    ID: gs-12345
    Namespace: studio-live
    Status: ACTIVE
    Region: ap-southeast-1
    Created: 2026-03-20T10:23:11Z
```

### 25.8 Inspect with section

```text
› Dedicated server config
    ID: ds-ranked-ap
    Region: ap-southeast-1
    Status: READY

    Deployment
        Image: accelbyte/ds:v1.8.2
        Replicas: 3
        Port: 7777
```
