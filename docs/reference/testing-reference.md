# CLI Testing Reference

- **Status:** Draft
- **Version:** 1.2
- **Last Updated:** 2026-03-22

## 1. Purpose

This reference defines the normative testing model for the CLI.

It standardizes the required test types, their intended purpose, their boundaries, expected coverage, and the criteria for judging success and failure. Its goal is to ensure that the CLI is tested consistently, that gaps between test layers are explicit, and that teams do not misuse one test type as a substitute for another.

This reference covers these test types:

- unit
- integration
- functional
- input contract
- output contract
- snapshot
- end-to-end
- compatibility
- security
- performance

## 2. Scope

This reference applies to all user-facing CLI commands, shared libraries used by the CLI, generated command surfaces, generated or structured outputs, and supported execution environments.

It applies to:

- command execution behavior
- argument parsing
- validation
- invocation surface stability
- output structure stability
- exact output rendering
- filesystem interaction
- backend interaction
- security-sensitive handling
- performance-sensitive behavior
- error handling
- supported platform behavior

It does not define team-specific tooling, framework choice, or CI implementation details except where required to preserve the meaning of a test type.

## 3. Normative Language

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHALL**, **SHALL NOT**, **SHOULD**, **SHOULD NOT**, **RECOMMENDED**, **MAY**, and **OPTIONAL** in this document are to be interpreted as normative requirements.

## 4. Testing Model

The CLI testing model SHALL be layered. Each test type exists to answer a different question. No test type SHALL be treated as a universal substitute for another.

The required questions are:

- Is the small logic correct?
- Do components work together correctly?
- Does the feature behave correctly from the user’s perspective under controlled conditions?
- Did the public invocation surface change?
- Did the observable output contract change?
- Did the exact rendered output change?
- Is the output still good for users?
- Does the whole product work in a realistic environment?
- Does the CLI work in supported environments?
- Does the CLI remain safe to use?
- Does the CLI remain fast enough to use?
- Can users install, upgrade, and invoke it successfully?

Accordingly, the CLI test suite MUST include all of the following categories:

- unit tests
- integration tests
- functional tests
- input contract tests
- output contract tests
- snapshot tests
- end-to-end tests
- compatibility tests
- security tests
- performance tests

## 5. Global Requirements

### 5.1 Determinism

All non-E2E tests MUST be deterministic.

Tests SHOULD control or normalize sources of nondeterminism, including:

- time
- randomness
- locale
- terminal width
- color output
- temporary paths
- hostnames
- dynamic identifiers
- ordering where ordering is not semantically meaningful

A test that is flaky due to uncontrolled nondeterminism SHALL be considered non-compliant.

### 5.2 Isolation

Unit, integration, functional, input contract, output contract, snapshot, security, and performance tests MUST NOT depend on uncontrolled external services unless such dependency is the explicit subject of the test and is fully controlled by policy.

Unless a test is explicitly categorized as end-to-end, it MUST use controlled dependencies such as:

- fakes
- mocks
- fixtures
- temporary filesystems
- stub servers
- injected transports
- local test packages or repositories

### 5.3 Real Network Access

Real backend endpoints MUST NOT be used by:

- unit tests
- integration tests
- functional tests
- input contract tests
- output contract tests
- snapshot tests
- compatibility tests
- security tests
- performance tests

Only end-to-end tests MAY use real backend endpoints, and only when explicitly designated.

### 5.4 Exit Code, Stdout, and Stderr

For every user-facing command testable through normal invocation, the suite SHOULD assert the appropriate combination of:

- exit code
- stdout
- stderr
- created or modified files
- externally visible side effects

### 5.5 Red/Green Meaning

For all test categories, “green” SHALL mean the test outcome meets the required acceptance rule for that category.

For all test categories, “red” SHALL mean at least one required acceptance rule is violated.

A test MUST NOT be marked green based on reviewer intent alone when its category-specific rules fail.

### 5.6 Resilience Coverage

Resilience and failure-mode behavior SHALL be covered, but SHALL NOT constitute a separate first-class test category in this reference.

Resilience scenarios MUST be covered primarily within:

- functional tests
- end-to-end tests
- output contract tests where relevant

Required resilience scenarios SHOULD include, where applicable:

- timeouts
- retries
- rate limits
- malformed responses
- partial failures
- broken pipes
- interrupted file or network operations
- corrupted local state

## 6. Unit Tests

### 6.1 Purpose

Unit tests exist to verify small units of logic in isolation.

They SHALL be used to validate local correctness of code whose behavior can be judged without invoking the full command stack.

Typical subjects include:

- parsing helpers
- validators
- formatters
- mappers
- config precedence rules
- path resolution helpers
- exit code mapping
- serialization helpers
- retry policy logic
- error classification logic

### 6.2 What Unit Tests Are For

Unit tests MUST answer:

- Is this small piece of logic correct for the given inputs?

Unit tests SHOULD provide the fastest and most precise signal in the suite.

### 6.3 What Unit Tests Are Not For

Unit tests MUST NOT be used as the primary evidence that:

- the full feature works
- the public invocation surface is stable
- the output contract is stable
- exact rendered output is approved
- real installation works
- supported platforms behave consistently

### 6.4 When Unit Tests Shall Be Used

Unit tests SHALL be used when:

- the logic can be isolated
- behavior is deterministic
- failure should localize to a function, rule, or module
- external systems are unnecessary

Any non-trivial reusable logic SHOULD have unit coverage.

### 6.5 Typical Coverage

Unit tests SHOULD represent the largest share of the test suite by count.

Typical coverage guidance:

- **high coverage** of pure logic modules is REQUIRED
- **broad coverage** of validation, parsing, mapping, and formatting code is RECOMMENDED
- line coverage targets MAY be used, but numerical coverage alone SHALL NOT be treated as sufficient proof of quality

As a rule of thumb, unit tests will often constitute **50% to 70% of total automated test cases by count**, though not necessarily by maintenance effort.

### 6.6 Red/Green Judgment

A unit test is **green** only if:

- the observed return values, state, or emitted errors match the expected result exactly for the case under test

A unit test is **red** if:

- any asserted logic outcome differs from expectation
- the unit depends on uncontrolled external state
- the test fails nondeterministically

## 7. Integration Tests

### 7.1 Purpose

Integration tests exist to verify that multiple internal components work together correctly under controlled conditions.

They SHALL validate wiring across module boundaries without requiring the entire CLI to operate against real external systems.

Typical subjects include:

- command handler + config loader
- command handler + filesystem
- command handler + auth/token store
- service layer + injected backend client
- renderer + structured response models
- local cache/database interactions

### 7.2 What Integration Tests Are For

Integration tests MUST answer:

- Do these components interact correctly when combined?

They SHOULD catch wiring errors that unit tests cannot expose.

### 7.3 What Integration Tests Are Not For

Integration tests MUST NOT be used as the primary evidence that:

- a feature behaves correctly as a user-facing process in all modes
- the invocation surface is stable
- the output contract is stable
- exact output text is approved
- real installation works
- real backend environments work

### 7.4 When Integration Tests Shall Be Used

Integration tests SHALL be used when:

- correctness depends on interaction between modules
- the risk is in wiring rather than local logic
- the dependency boundary can still be controlled
- filesystem, config, or local state materially affects behavior

### 7.5 Typical Coverage

Integration tests SHOULD cover:

- all major command families
- all major dependency boundaries
- critical config and filesystem interactions
- all major backend interaction paths using fakes or stubs

As a rule of thumb, integration tests will often constitute **15% to 25% of total automated test cases by count**.

Coverage SHOULD be broad by feature area, but selective by case depth.

### 7.6 Red/Green Judgment

An integration test is **green** only if:

- the combined components produce the expected behavior under the controlled scenario
- the test uses controlled dependencies only
- asserted interactions and outcomes match expectation

An integration test is **red** if:

- any required interaction or outcome is incorrect
- real uncontrolled dependencies are used
- the test passes only due to incidental environment state

## 8. Functional Tests

### 8.1 Purpose

Functional tests exist to verify that a CLI feature behaves correctly from the user’s perspective when invoked as a command, while using controlled dependencies.

They SHALL test realistic CLI behavior without requiring real external backends.

Typical assertions include:

- exit code
- stdout
- stderr
- file creation/modification
- environment-variable handling
- flag combinations
- working-directory behavior

### 8.2 What Functional Tests Are For

Functional tests MUST answer:

- Does this feature behave correctly when invoked in a realistic but controlled environment?

They SHOULD be the primary vehicle for validating command-level success and error behavior.

They MUST also cover resilience scenarios relevant to the feature, including failure handling and degraded dependency behavior where applicable.

### 8.3 What Functional Tests Are Not For

Functional tests MUST NOT be used as the sole evidence that:

- the public invocation surface is stable
- the observable output contract is stable
- exact output wording is approved for long-term stability
- real production or staging environments work
- the CLI is compatible across all supported OS/shell variants

### 8.4 When Functional Tests Shall Be Used

Functional tests SHALL be used when:

- the command surface itself is under test
- flags, env vars, cwd, stdout, stderr, and exit codes matter
- the user-visible runtime contract is important
- backend access can be replaced with fakes or interceptors

Every user-facing command MUST have functional tests for:

- at least one success path
- at least one validation error path, where applicable
- at least one dependency/backend error path, where applicable

Commands with meaningful mode differences SHOULD have separate tests for:

- structured output modes
- quiet/verbose modes
- interactive vs non-interactive behavior
- file-writing behavior
- auth/permission failure behavior
- timeout and retry behavior where applicable
- malformed-response handling where applicable

### 8.5 Typical Coverage

Functional tests SHOULD provide broad coverage across commands and common error categories.

As a rule of thumb:

- every top-level command MUST have functional coverage
- all high-value subcommands SHOULD have multiple functional tests
- all major error classes SHOULD be represented
- resilience scenarios SHOULD be represented for commands that depend on external systems
- these tests will often comprise **10% to 20% of total automated test cases by count**, but a disproportionately high share of confidence value

### 8.6 Red/Green Judgment

A functional test is **green** only if:

- the command exits with the expected status
- stdout and stderr behavior match expectations for the scenario
- required side effects occur
- prohibited side effects do not occur
- required failure handling behavior occurs for resilience scenarios under test

A functional test is **red** if:

- exit code is wrong
- stdout or stderr violates the expected runtime behavior for the scenario
- side effects are missing or unexpected
- required failure handling is absent or incorrect
- the command requires uncontrolled real services

## 9. Input Contract Tests

### 9.1 Purpose

Input contract tests exist to verify that the public invocation surface of the CLI remains stable unless explicitly changed and approved.

They SHALL protect the user-facing input contract by detecting changes to how commands are named, addressed, and invoked.

Typical subjects include:

- command names
- subcommand hierarchy
- aliases
- public flag names
- short and long option names
- positional argument presence and placement
- generated command surfaces derived from OpenAPI or code generation rules

### 9.2 What Input Contract Tests Are For

Input contract tests MUST answer:

- Did the public way of invoking the CLI change?

They SHOULD detect breaking changes in the invocation surface even when runtime behavior remains correct.

### 9.3 What Input Contract Tests Are Not For

Input contract tests MUST NOT be used as the primary evidence that:

- the command behaves correctly at runtime
- the backend integration is correct
- the observable output is structurally compatible
- the exact wording of output is approved
- the output is clear or high quality
- real-world workflows succeed

### 9.4 When Input Contract Tests Shall Be Used

Input contract tests SHALL be used whenever the CLI exposes a stable public invocation surface.

They are REQUIRED whenever any portion of the public command surface is generated from:

- OpenAPI specifications
- operation IDs
- path-to-command mapping rules
- naming normalization rules
- code generation templates
- alias generation rules

Input contract tests SHOULD compare the full normalized public invocation surface against an approved baseline.

### 9.5 Typical Coverage

Input contract coverage SHOULD be effectively complete for the public invocation surface.

Coverage guidance:

- all public command paths MUST be covered
- all public subcommands MUST be covered
- all public flags and aliases MUST be covered
- partial sampling is generally insufficient

Input contract tests are typically **full-surface artifact comparisons**, not percentage-driven case collections.

### 9.6 Red/Green Judgment

An input contract test is **green** only if:

- the normalized public invocation surface exactly matches the approved baseline, or
- all differences are explicitly reviewed and approved according to policy

An input contract test is **red** if:

- any unapproved change alters the public invocation surface
- normalization rules are inconsistent or undefined
- generated command metadata cannot be deterministically compared

## 10. Output Contract Tests

### 10.1 Purpose

Output contract tests exist to verify that the observable output interface of the CLI remains structurally and semantically compatible, without requiring exact wording stability.

They SHALL protect the public output contract by detecting changes in channels, structure, schema, and required semantic elements.

Typical subjects include:

- stdout vs stderr routing
- machine-readable JSON or YAML shape
- required fields and field types
- required headings, sections, or labels where contractually relevant
- parseability
- success vs error output mode behavior

### 10.2 What Output Contract Tests Are For

Output contract tests MUST answer:

- Did the observable output contract change?

They SHOULD detect contract-breaking output changes even when the new wording is acceptable or improved.

### 10.3 What Output Contract Tests Are Not For

Output contract tests MUST NOT be used as the primary evidence that:

- the underlying logic is correct
- the exact text matches an approved rendering
- the output is clear, concise, or well-written
- the full feature works end-to-end in a real environment
- the invocation surface is stable

### 10.4 When Output Contract Tests Shall Be Used

Output contract tests SHALL be used whenever a command exposes output that downstream humans, scripts, or tools depend on structurally or semantically.

They are REQUIRED for:

- machine-readable output modes
- output whose channel placement is part of the contract
- output with required fields or sections
- error outputs with stable structural expectations

Human-readable output MAY have lighter output-contract coverage than machine-readable output, but any required structural invariant MUST be asserted explicitly.

### 10.5 Typical Coverage

Output contract coverage SHOULD be:

- complete for machine-readable output modes
- broad for high-value human-readable output modes with structural guarantees
- selective for purely cosmetic presentation details, which belong in snapshot tests instead

Coverage guidance:

- all stable JSON/YAML modes MUST be covered
- stdout/stderr routing for major success and error paths MUST be covered
- required sections or semantic markers for high-value outputs SHOULD be covered

### 10.6 Red/Green Judgment

An output contract test is **green** only if:

- the output satisfies the defined structural and semantic contract for the scenario
- required fields, sections, channels, and types are preserved
- the output remains parseable where parseability is required

An output contract test is **red** if:

- any required structural or semantic invariant is violated
- output is emitted on the wrong channel
- required fields, sections, or types are missing or changed incompatibly
- the test relies on exact wording where only contract-level invariants should be asserted

## 11. Snapshot Tests

### 11.1 Purpose

Snapshot tests exist to detect unexpected changes in exact output.

They SHALL protect stable user-visible artifacts whose exact textual or structured representation matters.

Typical snapshot subjects include:

- help text
- long-form error messages
- table rendering
- generated configuration files
- generated markdown or shell snippets
- stable command transcripts

### 11.2 What Snapshot Tests Are For

Snapshot tests MUST answer:

- Did the exact normalized rendering change from the approved baseline?

They SHOULD act as exactness guards, not semantic or UX judges.

### 11.3 What Snapshot Tests Are Not For

Snapshot tests MUST NOT be used as the primary evidence that:

- the underlying logic is correct
- the invocation surface is stable
- the output contract is stable
- the output is clear, concise, or user-friendly
- the changed output is worse rather than better
- the command works end-to-end in a real environment

Snapshot approval MUST NOT be treated as proof of quality or compatibility.

### 11.4 When Snapshot Tests Shall Be Used

Snapshot tests SHALL be used when:

- exact output stability matters
- output is too large or complex for small inline assertions
- regression in wording, formatting, or layout would be meaningful
- the output can be made deterministic

Snapshot tests SHOULD be used for:

- `--help`
- stable long-form user-facing text
- high-value exact renderings
- generated files intended to remain textually stable

### 11.5 Typical Coverage

Snapshot coverage SHOULD be selective, not exhaustive.

Typical coverage guidance:

- all top-level help output SHOULD be snapshotted
- high-value exact text surfaces SHOULD be snapshotted
- ephemeral or intentionally fluid output SHOULD NOT be snapshotted
- output whose contract is structural rather than exact SHOULD use output contract tests instead

As a rule of thumb, snapshot tests often comprise **5% to 15% of total automated test cases by count**.

### 11.6 Red/Green Judgment

A snapshot test is **green** only if:

- the current normalized output exactly matches the approved snapshot

A snapshot test is **red** if:

- the current output differs from the approved snapshot
- the output contains uncontrolled nondeterminism
- the snapshot was updated without explicit review where review is required by policy

Intentional snapshot changes MAY be approved, but until approval they SHALL remain red.

## 12. End-to-End Tests

### 12.1 Purpose

End-to-end tests exist to verify that the CLI works in a realistic environment from the perspective of a real user workflow.

They SHALL validate cross-system behavior that cannot be proven by lower-level controlled tests alone.

Typical subjects include:

- installation to working command
- authentication to successful operation
- full resource lifecycle operations
- integration with real staging or controlled real services

### 12.2 What End-to-End Tests Are For

End-to-end tests MUST answer:

- Can a real user complete this workflow successfully in a realistic environment?

They SHOULD provide final confidence for critical flows, not broad exhaustive logic coverage.

They MUST also cover critical resilience and degraded-dependency behavior where such behavior materially affects the user workflow.

### 12.3 What End-to-End Tests Are Not For

End-to-end tests MUST NOT be used as the primary mechanism to cover:

- fine-grained logic branches
- exhaustive error mapping
- input contract stability
- output contract stability
- exact output approval
- all permutations of flags and modes
- all user-facing writing quality issues

E2E tests are too expensive and too coarse to replace lower layers.

### 12.4 When End-to-End Tests Shall Be Used

End-to-end tests SHALL be used for a small number of critical workflows, including where applicable:

- install → authenticate → core action
- init → configure → run
- fetch → render → save
- create → verify → clean up

They SHOULD be reserved for:

- highest-value workflows
- release-critical flows
- real integration risks not otherwise covered
- critical degraded-mode workflows where realistic system behavior matters

### 12.5 Typical Coverage

End-to-end coverage SHOULD be intentionally small.

Typical guidance:

- only the highest-value user journeys MUST be covered
- a minimal release confidence set SHOULD exist
- broad branch coverage MUST NOT be pursued at the E2E layer

As a rule of thumb, E2E tests often comprise **less than 5% of total automated test cases by count**.

### 12.6 Red/Green Judgment

An end-to-end test is **green** only if:

- the full workflow succeeds in the designated environment
- all required real-world outcomes are observed
- user-visible success/failure behavior matches the contract where asserted
- required degraded-mode behavior is correct for any resilience scenario under test

An end-to-end test is **red** if:

- the workflow fails at any required step
- required side effects are absent
- environmental assumptions are not satisfied
- degraded-mode behavior is wrong where the scenario requires it
- the test becomes flaky beyond the tolerated threshold

Because E2E tests may involve real systems, transient failures SHALL be investigated, but repeated flakiness SHALL be treated as a defect in either the product or the test.

## 13. Compatibility Tests

### 13.1 Purpose

Compatibility tests exist to verify that the CLI works correctly across all supported environments.

They SHALL validate platform and environment compatibility rather than feature correctness alone.

Typical dimensions include:

- operating systems
- CPU architectures
- shells
- terminal capabilities
- locale/encoding
- filesystem semantics
- supported runtime versions
- old/new config versions

### 13.2 What Compatibility Tests Are For

Compatibility tests MUST answer:

- Does the CLI behave acceptably in each supported environment?

They SHOULD detect environment-specific failures that are invisible in a single default CI environment.

### 13.3 What Compatibility Tests Are Not For

Compatibility tests MUST NOT be used as the primary evidence that:

- all feature logic is correct
- the invocation surface is stable
- the output contract is stable
- exact output wording is approved
- real backends work
- the entire product is validated end-to-end

Compatibility is about supported-environment correctness, not total correctness.

### 13.4 When Compatibility Tests Shall Be Used

Compatibility tests SHALL be used whenever the CLI officially supports more than one of the following:

- operating system
- shell environment
- runtime version
- architecture
- terminal mode
- config version

If support is claimed, compatibility coverage for that claim MUST exist.

### 13.5 Typical Coverage

Compatibility coverage SHOULD be matrix-based and risk-based.

Minimum guidance:

- every claimed supported OS MUST be represented
- every claimed supported shell/runtime combination with meaningful behavior differences SHOULD be represented
- install/run/help for each supported platform MUST be covered
- at least one core command per supported platform SHOULD be covered

Compatibility tests often comprise **5% to 10% of total automated test cases by count**, but their execution cost may be higher.

### 13.6 Red/Green Judgment

A compatibility test is **green** only if:

- the CLI behaves according to contract in the claimed environment

A compatibility test is **red** if:

- behavior differs materially across supported environments without an approved documented exception
- installation or invocation fails in a supported environment
- encoding, shell, or filesystem behavior violates the supported contract

Any unsupported environment MAY be excluded, but supported-environment failures SHALL be treated as release-blocking unless explicitly waived.

## 14. Security Tests

### 14.1 Purpose

Security tests exist to verify that the CLI handles sensitive data, trust boundaries, and security-relevant behaviors safely.

They SHALL protect against release of a CLI that is functionally correct but unsafe to operate.

Typical subjects include:

- token and credential leakage
- secret redaction in stdout, stderr, logs, and debug output
- unsafe subprocess invocation
- path traversal and unsafe file access
- unsafe config loading or interpolation
- permission-boundary behavior
- insecure temporary-file handling

### 14.2 What Security Tests Are For

Security tests MUST answer:

- Is the CLI safe to use within its supported threat and trust model?

They SHOULD detect security-relevant regressions that ordinary functional tests may not catch.

### 14.3 What Security Tests Are Not For

Security tests MUST NOT be used as the primary evidence that:

- ordinary feature logic is correct
- the invocation surface is stable
- the output contract is stable
- the CLI performs well
- installation succeeds across supported platforms

### 14.4 When Security Tests Shall Be Used

Security tests SHALL be used whenever the CLI:

- handles credentials, tokens, or secrets
- accesses files on behalf of the user
- invokes subprocesses or shells
- loads configuration from user-controlled locations
- emits diagnostic or debug output
- enforces or communicates permission-sensitive behavior

### 14.5 Typical Coverage

Security coverage SHOULD be risk-based.

Minimum guidance:

- all credential-bearing flows MUST be covered for redaction and leakage risk
- all shell/subprocess boundaries SHOULD be covered where applicable
- all file-writing and file-reading paths with trust-boundary implications SHOULD be covered
- debug and verbose modes SHOULD be covered when they may expose sensitive state

Security tests are not primarily count-driven; they SHOULD be complete for high-risk surfaces.

### 14.6 Red/Green Judgment

A security test is **green** only if:

- the tested behavior satisfies the defined security requirement for the scenario
- no prohibited secret, credential, or unsafe behavior is observed
- the CLI preserves required redaction, isolation, and trust-boundary handling

A security test is **red** if:

- a secret or credential is exposed
- unsafe command execution or file access behavior occurs
- a required permission or trust-boundary rule is violated
- debug or diagnostic behavior leaks sensitive information

## 15. Performance Tests

### 15.1 Purpose

Performance tests exist to verify that the CLI remains fast and resource-efficient enough for supported usage.

They SHALL protect against regressions that make the CLI materially slower, heavier, or less responsive.

Typical subjects include:

- startup latency
- command latency
- large input handling
- large output rendering
- memory usage
- repeated invocation cost

### 15.2 What Performance Tests Are For

Performance tests MUST answer:

- Is the CLI still fast enough and efficient enough for its supported use cases?

They SHOULD detect regressions in responsiveness and resource usage.

### 15.3 What Performance Tests Are Not For

Performance tests MUST NOT be used as the primary evidence that:

- feature logic is correct
- invocation or output contracts are stable
- installation works
- supported environments are all functionally correct

### 15.4 When Performance Tests Shall Be Used

Performance tests SHALL be used for:

- startup-sensitive commands
- high-frequency commands
- commands processing large datasets or files
- commands rendering large structured output
- workflows where latency is part of the user expectation

Performance thresholds SHOULD be defined for the most important user-facing paths.

### 15.5 Typical Coverage

Performance coverage SHOULD be selective and risk-based.

Minimum guidance:

- startup time SHOULD be covered for the primary executable path
- at least one high-value command SHOULD be covered for normal-scale input
- large-input or large-output scenarios SHOULD be covered where such scale is supported or expected
- memory-sensitive paths SHOULD be covered where resource usage may regress materially

Performance tests are typically benchmark- or threshold-driven rather than count-driven.

### 15.6 Red/Green Judgment

A performance test is **green** only if:

- the measured performance remains within the approved threshold, budget, or regression tolerance for the scenario

A performance test is **red** if:

- latency, throughput, or memory usage exceeds the approved threshold
- a regression exceeds the tolerated change budget
- the measurement method is unstable or uncontrolled

## 16. Coverage Policy

### 16.1 General

Coverage SHALL be judged by both:

- breadth across features and commands
- depth across important success and failure modes

No single numeric metric SHALL be sufficient.

### 16.2 Typical Portfolio Distribution

A typical healthy portfolio will often look approximately like this by count:

| Test Type | Typical Share by Count |
|---|---:|
| Unit | 50%–70% |
| Integration | 15%–25% |
| Functional | 10%–20% |
| Input contract | full-surface, not count-driven |
| Output contract | coverage-driven, especially for machine-readable modes |
| Snapshot | 5%–15% |
| E2E | <5% |
| Compatibility | matrix-based, not count-driven |
| Security | risk-based, not count-driven |
| Performance | benchmark/threshold-driven, not count-driven |

These values are guidance, not quotas. Teams MAY vary, but SHALL preserve the intended layering.

### 16.3 Minimum Required Breadth

The suite MUST provide, at minimum:

- unit coverage for reusable non-trivial logic
- integration coverage for major internal boundaries
- functional coverage for every top-level user-facing command
- input contract coverage for the full public invocation surface
- output contract coverage for stable machine-readable outputs and major output invariants
- snapshot coverage for top-level help output and other high-value exact renderings
- E2E coverage for critical real-user flows
- compatibility coverage for all officially supported environments
- security coverage for high-risk credential, trust-boundary, and file/system interaction paths
- performance coverage for startup-sensitive and high-value performance-sensitive paths

## 17. Red/Green Summary Table

| Test Type | Green Means | Red Means |
|---|---|---|
| Unit | Small logic is exactly correct for the case | Logic outcome differs or depends on uncontrolled state |
| Integration | Combined components behave correctly under controlled conditions | Wiring or combined behavior is wrong |
| Functional | Feature runtime behavior is correct: exit code, output behavior, side effects, failure handling | Runtime behavior violates the expected user-facing behavior |
| Input contract | Public invocation surface matches approved baseline | Unapproved change to commands, flags, aliases, or argument surface |
| Output contract | Output satisfies structural and semantic contract | Unapproved change to channel, schema, fields, sections, or parseability |
| Snapshot | Exact normalized rendering matches approved baseline | Exact rendered output changed unexpectedly |
| E2E | Critical real workflow succeeds in realistic environment | Real workflow fails or is too flaky |
| Compatibility | Supported environment behaves according to contract | Supported environment fails materially |
| Security | Security requirements and redaction/trust-boundary rules hold | Secret leakage or unsafe behavior occurs |
| Performance | Measured behavior stays within approved budget or tolerance | Performance budget or regression tolerance is exceeded |

## 18. Non-Substitution Rule

The following substitutions SHALL NOT be allowed:

- functional tests SHALL NOT substitute for input contract tests
- functional tests SHALL NOT substitute for output contract tests
- snapshot tests SHALL NOT substitute for output contract tests
- E2E tests SHALL NOT substitute for unit or integration coverage
- compatibility tests SHALL NOT substitute for functional or E2E coverage
- security tests SHALL NOT substitute for functional correctness tests
- performance tests SHALL NOT substitute for functional correctness tests
- unit tests SHALL NOT substitute for functional behavior validation

A testing strategy that omits a layer and claims another layer “covers it implicitly” SHALL be considered non-compliant unless an explicit written exception is approved.

## 19. Approval and Failure Policy

### 19.1 Merge/Release Expectations

At minimum, the following SHOULD be merge-gating:

- unit
- integration
- functional
- input contract
- output contract
- snapshot
- required compatibility checks
- required security checks

Performance tests SHOULD gate where approved budgets exist for high-value paths.


E2E tests SHOULD gate releases for critical workflows and MAY be required on protected branches.

### 19.2 Intentional Change

An intentional change in behavior or output MAY turn a previously green test red.

In such cases, the proper resolution is:

- update the expected behavior, approved contract artifact, approved snapshot, threshold, or channel expectation
- review and approve the change
- rerun the test

A test MUST NOT be ignored solely because the change was intentional.

## 20. Compliance Checklist

A CLI implementation is compliant with this reference only if all of the following are true:

- unit tests exist for non-trivial isolated logic
- integration tests cover major internal boundaries
- every top-level command has functional coverage
- the public invocation surface has input contract coverage
- stable output interfaces have output contract coverage
- stable help and other high-value exact renderings have snapshot coverage
- critical workflows have E2E coverage
- all supported environments have compatibility coverage
- high-risk security surfaces have security coverage
- performance-sensitive paths have performance coverage
- lower-level tests do not rely on uncontrolled real services
- red/green rules are explicit for each category

## 21. Recommended Short Definitions

- **Unit:** verifies small isolated logic.
- **Integration:** verifies multiple internal components work together.
- **Functional:** verifies CLI feature behavior through real command invocation in a controlled environment.
- **Input contract:** verifies the public invocation surface remains stable.
- **Output contract:** verifies the observable output structure and semantics remain stable.
- **Snapshot:** verifies exact normalized rendering did not change unexpectedly.
- **E2E:** verifies a real user workflow works in a realistic environment.
- **Compatibility:** verifies the CLI works in every supported environment.
- **Security:** verifies the CLI handles secrets, trust boundaries, and sensitive operations safely.
- **Performance:** verifies the CLI remains within approved responsiveness and resource budgets.

## 22. Final Rule

A compliant CLI testing strategy MUST use multiple test types for different purposes. It MUST NOT collapse correctness, invocation stability, output compatibility, exact presentation, realism, platform support, safety, and performance into a single layer.

Each test type exists because it protects a different failure mode. The suite is only complete when all of those failure modes are intentionally covered.
