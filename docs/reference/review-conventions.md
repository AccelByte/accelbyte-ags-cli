# AGS CLI Review Conventions

Contributor-facing conventions derived from the codebase and from recurring review
findings on merged pull requests #79–#88.

---

## How to use this document

This document is a **contributor-facing conventions reference** derived from the
codebase and from recurring review findings on merged pull requests. It captures rules
that are enforced only by code patterns, architecture tests, or PR review — not yet written
down anywhere, or only partially documented.

**The maintainer owns the standard.** Nothing here overrides or amends the published
reference docs (`docs/reference/output-reference.md`, `docs/reference/testing-reference.md`,
`docs/reference/cli-reference.md`, `CONTRIBUTING.md`). **On any conflict, the existing
reference docs win.** This document covers the gaps between them.

**Rule numbering:** Rules are numbered `RULE-01` through `RULE-39` in a single scheme.
Three numbers are intentionally absent: `RULE-14` required stronger evidence before
promotion (the performance rule is captured at `RULE-37`); `RULE-17` duplicates
`CONTRIBUTING.md §Common Commands` (bundled specs are not hand-edited; use
`cargo run -- refresh-specs`); and `RULE-18` is reserved for the hand-written
command-group convention — the direction is settled, but this document only cites code
that exists on `main`, so the rule is added when that implementation lands. As a
checksum, the area table's per-row counts must always sum to the highest rule number
minus the reserved numbers (currently 39 − 3 = 36); update the numbering note and the
table together.

---

**Citation maintenance:** House patterns and provenance cite `file:line` locations as
they stood when each rule was last verified. Symbol and function names are the
authoritative anchor; line numbers are a convenience snapshot and may drift as files
evolve. The self-check commands grep for symbols, not lines, so they stay valid. A change
that touches a file cited here updates the affected citations in the same change — this
document is subject to its own RULE-28.

## The conventions, by area

Each area is its own page; rules are numbered in one `RULE-NN` scheme across all pages.

| Area | Rules | Count |
|------|-------|:-----:|
| 1. [Output](review-conventions/output.md) | `RULE-01`, `RULE-02`, `RULE-03`, `RULE-04`, `RULE-05`, `RULE-06`, `RULE-07` | 7 |
| 2. [Errors](review-conventions/errors.md) | `RULE-08`, `RULE-09`, `RULE-10`, `RULE-11` | 4 |
| 3. [Config & State](review-conventions/config-and-state.md) | `RULE-12`, `RULE-13` | 2 |
| 4. [TTY & Interactivity](review-conventions/tty-and-interactivity.md) | `RULE-15`, `RULE-16` | 2 |
| 5. Spec vs Hand-Written Commands | `RULE-18` (reserved — awaiting the hand-written command-group convention landing on `main`) | — |
| 6. [Testing](review-conventions/testing.md) | `RULE-19`, `RULE-20`, `RULE-21`, `RULE-22`, `RULE-23`, `RULE-24`, `RULE-27` | 7 |
| 7. [Docs & Comments](review-conventions/docs-and-comments.md) | `RULE-28`, `RULE-29` | 2 |
| 8. [CI & Supply Chain](review-conventions/ci-and-supply-chain.md) | `RULE-30` | 1 |
| 9. [Security](review-conventions/security.md) | `RULE-31` | 1 |
| 10. [Consistency & Reuse](review-conventions/consistency-and-reuse.md) | `RULE-32`, `RULE-33`, `RULE-34`, `RULE-35`, `RULE-36`, `RULE-37`, `RULE-38` | 7 |
| 11. [Naming & Structure](review-conventions/naming-and-structure.md) | `RULE-25`, `RULE-26`, `RULE-39` | 3 |

## Appendix — rule index and review-finding class coverage

The area table at the top of this page is the canonical rule-to-area index.

### Review-finding classes: coverage and exclusions

The 23 recurring review-finding classes observed across pull requests #79–#88 are all
accounted for below. Classes with insufficient evidence for a general rule are explicitly
excluded with a reason.

| Catalogue class | Disposition |
|----------------|-------------|
| **output / surface contract** (class #1) | Covered: RULE-02, RULE-03, RULE-06. Dynamic ordering concern deferred to maintainer. |
| **test coverage gap** (class #2) | Covered: RULE-19..RULE-24. Pre-empted by `testing-reference.md`. |
| **CI / supply-chain / release config** (class #3) | Covered: RULE-30. |
| **non-interactive / automation contract** (class #4) | Covered: RULE-15, RULE-16. |
| **docs / comment drift** (class #5) | Covered: RULE-28, RULE-29. |
| **duplication** (class #6) | Covered: RULE-36. |
| **security: traversal / redaction / creds** (class #7) | Covered: RULE-31. |
| **concurrency / TOCTOU** (class #8) | Covered: RULE-33. |
| **normalization / comparison consistency** (class #9) | Covered: RULE-32. |
| **error classification / handling** (class #10) | Covered: RULE-08, RULE-09, RULE-10, RULE-11. |
| **footgun / overwrite guard** (class #11) | Covered: RULE-34. |
| **comment / code / test parity** (class #12) | Covered: RULE-29. |
| **performance / hot-path** (class #13) | Covered: RULE-37. |
| **new-command doc completeness** (class #14) | Covered: RULE-28. |
| **serde / JSON output consistency** (class #15) | Covered: RULE-35. |
| **config-key / flag surface** (class #16) | Covered: RULE-12, RULE-13. |
| **dead code / unused path** (class #17) | Covered: RULE-39. |
| **contract / versioning break** (class #18) | **Excluded:** One finding (single HIGH); no stable pattern in codebase to cite. Deferred to maintainer for API-versioning policy. |
| **control-flow / state edge-case** (class #19) | Covered by RULE-11, RULE-08 (partial); residual findings are command-specific. |
| **resource / memory / lifecycle** (class #20) | Covered: RULE-38. |
| **argv / argument parsing** (class #21) | **Excluded:** argv validation for hand-written commands belongs to the reserved `RULE-18`; until it lands, clap-built parsing per the existing top-level groups is the norm. |
| **input validation / UX** (class #22) | **Excluded:** `CONTRIBUTING.md §Security` documents the canonical path (`encode_url_path_segment`); display sanitization (`strip_terminal_control_sequences`) documented. |
| **protocol / type leakage & validation** (class #23) | **Excluded:** Marked "blind spot" but weight 3 (2L); insufficient recurrence to establish a general rule without stronger evidence. |
