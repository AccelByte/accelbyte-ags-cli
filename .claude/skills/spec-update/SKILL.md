---
name: spec-update
description: Land an already-finalized, enriched OpenAPI 2.0 spec into the AGS CLI — gzip it into the bundled specs, run the breaking-change gate, regenerate the catalogue + contract baselines, and guide the hand-authored manifest/alias edits. The mechanical, post-lint half of a spec refresh. Use after openapi-lint has produced a finalized <service>.json. Downstream sibling of the openapi-lint skill.
argument-hint: [<service>] — the service whose finalized spec to land; prompts if omitted
disable-model-invocation: false
---

You land an already-finalized, enriched OpenAPI 2.0 spec into the AGS CLI product repo. This is the mechanical, post-lint half of a spec refresh: the `openapi-lint` skill produces the finalized `<service>.json` (merge/enrich + OID assignment); you gzip it into the bundled specs, run the breaking-change gate, regenerate the catalogue and contract baselines, and guide the hand-authored manifest/alias edits. Chain: `openapi-lint` → `spec-update`.

Two paths: **refresh** an existing service (the common, near-mechanical case) and **new-service onboarding** (rarer, more hand-authored, clearly flagged). You detect which from whether the service is already bundled (Phase 0).

## Critical constraints

You MUST follow these without exception:

1. **One service per run.** Multiple services = run the skill again, once each.
2. **Source spec.** Default `.claude/output/openapi-lint/<service>/<service>.json` (openapi-lint's output). Always confirm the path with the user before reading it; accept an override path.
3. **The skill runs cargo itself** so a run is self-contained. Always run cargo serially (`--test-threads=1`); set the **Bash tool's** timeout to `300000` ms (5 minutes) — this is the tool's millisecond timeout parameter, NOT the shell `timeout` command (whose argument is seconds) — and never start a second cargo run while one is in flight.
4. **The breaking-change gate runs BEFORE any baseline regeneration** — regenerating first would erase the signal.
5. **Reproducible gzip.** Use `gzip -n` so the bundled bytes are reproducible (it strips the original name + mtime).
6. **Never git commit.** End with a change summary and let the user review and commit.
7. **No trailing full stop** on resource/service descriptions or headings.

## Phase 0: Select service and locate the spec

1. Determine the service: from the `<service>` argument, or — if omitted — list the services that have a finalized `<service>.json` under `.claude/output/openapi-lint/` and ask which one.
2. Confirm the source path `.claude/output/openapi-lint/<service>/<service>.json` with the user; accept an override path.
3. Detect the branch:
   - If `crates/ags-runtime/specs/<service>.json.gz` exists AND `<service>` is in `SERVICES` in `crates/ags-runtime/src/catalogue/manifest.rs` → **refresh** (skip Phase 2b).
   - Otherwise → **new-service onboarding** (Phase 2 is skipped — no baseline yet — and Phase 2b runs before Phase 3).

## Phase 1: Install the spec bytes

Gzip the finalized spec directly into the bundled specs, overwriting for a refresh (`<source-path>` is the path confirmed in Phase 0). Gzip straight from the source — no intermediate `/tmp` copy — so a retry can never pick up stale bytes from a prior run:
```
gzip -n -c <source-path> > crates/ags-runtime/specs/<service>.json.gz
```
`-n` suppresses the original filename and zeroes the mtime header (verified on both Apple gzip and GNU gzip), so the bytes are reproducible across machines and runs.

## Phase 2: Breaking-change gate (refresh only)

Skip this phase for a new service (it has no baseline yet — its first baseline is created in Phase 3).

Run (Bash tool timeout `300000` ms):
```
cargo test -p accelbyte-ags-cli --test contract_input test_no_breaking_changes -- --test-threads=1
```

- **Green** → go to Phase 3. (This phase runs only for a refresh, so the next step is always Phase 3; the new-service branch reaches Phase 3 via Phase 2b instead.)
- **Red** → classify **each** failing finding mechanically, not by impression. For the failing operation, find the same **HTTP path + HTTP method** in the new spec and compare its `x-operationId` to the baseline's:
  - **Renamed method segment** — the path+method **still exists** in the new spec but its `x-operationId` method segment differs (so the derived CLI method name changed; the old name is what the gate reports as deleted) → add a `former_method_names` entry in `crates/ags-runtime/src/catalogue/aliases.rs`, keyed `(service, resource, current-method) → [old-name]`. Re-run the gate and re-classify; **repeat while any rename-shaped failure remains** — a single spec update may rename several operations, and each needs its own alias.
  - **Genuinely removed / changed operation** — the path+method is **gone** from the new spec (or the whole path is absent) → add no alias; STOP and surface it to the user. Accepting a breaking change is the user's judgment call, not the skill's.

  The path+method identity is the load-bearing signal: an alias makes the old CLI name resolve to the new operation, so it is only ever correct when that operation still exists. Never add an alias for a path+method that is gone — that would leave a broken command name resolving to nothing. Keep resolving rename-shaped failures (one alias each, re-running) until the gate is green or a removal is found. A still-red gate is not itself the stop signal — the *kind* of remaining failure is.

## Phase 2b: Register a new service (new-service branch only)

Skip this phase for a refresh. Do all four edits BEFORE Phase 3 — the generator only emits a service in its own `SERVICES` list, and the build/tests only embed a service in `BUNDLED_SPECS`.

1. **`crates/ags-runtime/src/catalogue/bundled.rs` — `BUNDLED_SPECS`:** add
   `("<service>", include_bytes!("../../specs/<service>.json.gz"))`, positioned to keep the table in lockstep order with `manifest::SERVICES`.
2. **`crates/ags-runtime/src/catalogue/bundled.rs` — `test_bundled_specs_count`:** bump the asserted count (e.g. `24` → `25`). It also asserts `BUNDLED_SPECS.len() == manifest::SERVICES.len()`, so both tables must move together.
3. **`crates/ags-runtime/src/catalogue/manifest.rs` — `SERVICES`:** add a `ServiceManifest` entry (the display name + description are user-authored, no trailing full stop on the description):
   ```rust
   ServiceManifest {
       internal: "<service>",
       display: "<display-name>",
       description: "<one-line description>",
   },
   ```
4. **`scripts/generate_cli_command_catalogue.py` — `SERVICES`:** add the internal id to the list. Add a `DISPLAY_NAMES` entry ONLY if the display name differs from the internal id (when they are equal, omit it — `DISPLAY_NAMES.get(service, service)` already falls back to the internal id). If you do add one, its value must be copied **character-for-character** from the `display` field of the `ServiceManifest` entry authored in step 3 — a mismatch makes the catalogue document a display name the CLI does not use. Cross-check the two files before Phase 3.

Surface these four edits to the user for review — a missed one fails the lockstep count test or silently drops the service from the catalogue.

## Phase 3: Regenerate the catalogue and baselines (only after the gate is green)

For a new service this depends on Phase 2b — the generator will not emit a service absent from its `SERVICES` list.

Run:
```
python3 scripts/generate_cli_command_catalogue.py --emit-baselines crates/accelbyte-ags-cli/tests/fixtures/baselines/
```

This rewrites both the catalogue (`docs/reference/cli-command-catalogue.md`) and the per-service contract baselines in one pass. It does NOT touch the workflow baselines under `tests/fixtures/baselines/workflows/` (those come from the workflow registry, not specs).

The generator regenerates **all** services' baselines from the specs currently on disk — it has no per-service flag. Combined with one-service-per-run (constraint 1), this is safe only if the target service's `.gz` is the **only** changed spec in the working tree; otherwise another service's baseline would be silently overwritten, masking an ungated breaking change. Before running, assert the working tree is clean apart from the target — checking **both** modified tracked specs (refresh) and untracked new specs (new service: its `.gz` is brand-new and invisible to `git diff`):
```
git diff --name-only HEAD -- crates/ags-runtime/specs/
git ls-files --others --exclude-standard -- crates/ags-runtime/specs/
```
A file is either tracked or untracked, never both, so the two outputs are disjoint. Their **union** must be exactly the single path `crates/ags-runtime/specs/<service>.json.gz` — equivalently, the first command lists at most that one path and the second lists at most that one path, and no *other* path appears in **either**. If any other path appears in either output, STOP and resolve it before regenerating.

After the generator runs, surface **every** changed path — modified and newly created — and confirm it is intentional (new ops/params/summaries) before proceeding to Phase 4:
```
git status --short -- docs/reference/cli-command-catalogue.md crates/accelbyte-ags-cli/tests/fixtures/baselines/
```
Show the line-level diff of tracked changes with `git diff HEAD -- <path>`. For a new service the per-service baseline `crates/accelbyte-ags-cli/tests/fixtures/baselines/<service>_input_contract.json` is **untracked**, so `git diff` shows nothing — review it directly with `git diff --no-index /dev/null <path>` (or just read the file) so the user sees the first baseline. Unexpected changes to an untargeted service mean the working tree was not clean — STOP and investigate.

## Phase 4: Hand-authored resource metadata

Diff the resources present in the new spec against `RESOURCE_DESCRIPTIONS` in `crates/ags-runtime/src/catalogue/manifest.rs`. For each **new** resource, prompt the user to author a one-line description (kebab-case resource id, no trailing full stop) and add the `(service, resource, description)` triple.

**New-service branch:** add every one of the service's resources to `RESOURCE_DESCRIPTIONS` (the `SERVICES` allowlist + `service_description` were already added in Phase 2b). Resource descriptions are not consumed by the generator or baselines, so authoring them here — after regeneration — is fine.

## Phase 5: Cache / version check

Usually **no** bump is needed: the workspace-shared version + the release version bump auto-invalidate users' caches (`cache.rs` keys on `env!("CARGO_PKG_VERSION")`). Note this and move on unless the user explicitly wants a dev-side cache-busting bump.

## Phase 6: Full verification

Run (Bash tool timeout `300000` ms; never start a second cargo run while one is in flight):
```
cargo test -- --test-threads=1
```

`test_baseline_is_current` should now be green (baselines were regenerated in Phase 3), and `test_bundled_specs_count` should be green for a new service (count bumped in Phase 2b). Report the result. If anything is red, return to the relevant phase.

## Phase 7: Summary and handoff

Print an in-session summary:
- the gzipped spec (`crates/ags-runtime/specs/<service>.json.gz`),
- the regenerated catalogue + baselines,
- any `aliases.rs` / `RESOURCE_DESCRIPTIONS` / `SERVICES` / count edits,
- the breaking findings (if any) and how they were resolved.

Do NOT commit. End by asking the user to review and commit.
