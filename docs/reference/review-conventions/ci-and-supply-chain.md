# CI & Supply Chain — AGS CLI Review Conventions

Part of the [AGS CLI Review Conventions](../review-conventions.md) (RULE-30, 1 rule).

### RULE-30 — GitHub Actions workflows must pin third-party actions by commit SHA and carry a concurrency group

**Rule:** When GitHub Actions workflow files are added or modified in `.github/workflows/`,
every `uses:` reference to a third-party action (any action not in the
`actions/` or `github/` first-party namespace) must be pinned by full commit SHA rather
than by a mutable tag. Each workflow job that may be triggered by a PR must also declare a
`concurrency` group to cancel redundant in-progress runs.

Release-trigger workflows must use path filters or branch filters to prevent a plain PR
merge from triggering a release build.

**Why:** Mutable tags (e.g. `uses: some/action@v3`) can silently receive malicious commits
or breaking changes without the repository's knowledge. SHA pinning (`uses:
some/action@abc1234…`) locks the exact binary that runs in CI. Concurrent redundant
workflow runs waste CI minutes and can produce race conditions on shared resources.

The repository's two workflows already follow this rule: every third-party action in
`.github/workflows/ci.yml` and `.github/workflows/release.yml` is pinned by full commit
SHA with a trailing version comment, `ci.yml` declares a concurrency group, and
`release.yml` triggers only on version tags. (`release.yml` declares no concurrency
group; that is acceptable under this rule because it is tag-triggered, not PR-triggered.)

**House pattern:**
```yaml
# .github/workflows/ci.yml — SHA-pinned third-party action, version noted in a comment:
- uses: Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32 # v2

# .github/workflows/ci.yml — concurrency group cancels superseded PR runs:
concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}

# .github/workflows/release.yml — release trigger fires on version tags only:
on:
  push:
    tags:
      - "v[0-9]+.[0-9]+.[0-9]+"
```

**Self-check:**
```
grep -rn "uses:" .github/workflows/ | grep -v "uses: actions/\|uses: github/" | grep -v "@[0-9a-f]\{40\}"
```
Expected: no output — every action is pinned. The trailing `# v2`-style comments annotate
the pinned version and are not violations. Also verify that every workflow triggered by
PRs declares a `concurrency:` key.

**Provenance:** `.github/workflows/ci.yml`; `.github/workflows/release.yml`; recurring review finding class #3 (CI / supply-chain / release config); PR findings on merged pull requests #83–#87

---
