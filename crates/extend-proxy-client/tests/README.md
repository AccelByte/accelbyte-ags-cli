# Running `extend-proxy-client`'s tests

This crate has three layers of tests, each proving a different thing. Layer 1
runs everywhere, always. Layers 2 and 3 prove wire-compatibility with the real
Go implementation and require external dependencies, so they're gated behind
`#[ignore]` and skipped by default (including in CI).

| Layer | Where | Proves | Needs |
|-------|-------|--------|-------|
| 1. Unit tests | `src/**/*.rs` (`#[cfg(test)]`) | Internal correctness: codec round-trips, frame validation, ID generation, RPC behavior, heartbeat | Nothing — runs in CI |
| 2. `go_testpeer` | `tests/go_testpeer.rs` | This Rust client and the real Go server-role code (`pkg/tunnel`) agree on the wire | A `testpeer` binary built from the Go source |
| 3. `sidecar_conformance` | `tests/sidecar_conformance.rs` | This Rust client and the actual sidecar Docker image agree, including auth, allowlist, single-session enforcement, and real iptables redirection | Docker + Docker Compose |

All three layers live in this crate; layers 2 and 3 additionally depend on
`extend-helper-cli`'s `modules/extend-proxy` Go module, which is a **separate
repository** — clone it alongside this one if you don't already have it.

## Layer 1 — unit tests

No setup required:

```bash
cargo test -p extend-proxy-client
```

## Layer 2 — `go_testpeer` (cross-language, in-process peer)

Builds a small Go binary (`cmd/testpeer`) from the real `pkg/tunnel`
server-role logic and drives this crate's client against it over a genuine
TCP WebSocket connection.

1. Build the `testpeer` binary from the `extend-helper-cli` checkout:

   ```bash
   cd <path-to-extend-helper-cli>/modules/extend-proxy
   make build-testpeer
   ```

   This prints the binary's path, e.g. `modules/extend-proxy/bin/testpeer`.

2. Run the suite from this repo, pointing at that binary:

   ```bash
   TESTPEER_BIN=<path-to-extend-helper-cli>/modules/extend-proxy/bin/testpeer \
     cargo test -p extend-proxy-client --test go_testpeer -- --ignored
   ```

If `TESTPEER_BIN` is unset or doesn't point at a real file, the test panics
with this same build command — that's the intended failure mode for an
`--ignored` test that got run without its prerequisite.

Expect `12 passed; 0 failed`.

## Layer 3 — `sidecar_conformance` (cross-language, real sidecar container)

Drives this crate's client against the actual sidecar Docker image, so it
also exercises things `go_testpeer` deliberately doesn't: IAM-gated auth
(via `--iam-dev-mode`), the target allowlist/SSRF guard, the single-session
409 rejection, and real `iptables REDIRECT`-based traffic interception.

1. Bring up the harness from the `extend-helper-cli` checkout:

   ```bash
   cd <path-to-extend-helper-cli>/modules/extend-proxy
   make conformance-up
   ```

   This builds the sidecar image, starts it plus an echo backend via
   `docker-compose.conformance.yml`, and runs `sidecar-init` to install the
   iptables rules. It prints the ports it's listening on
   (`ws=127.0.0.1:18080 exposed=127.0.0.1:18008`).

2. Run the suite from this repo:

   ```bash
   cargo test -p extend-proxy-client --test sidecar_conformance -- --ignored
   ```

   Override `SIDECAR_WS_ADDR` / `SIDECAR_EXPOSED_ADDR` (see
   `sidecar_conformance.rs`) if your harness isn't on the default ports.

   These tests share one sidecar session, so they're serialized internally
   via an in-process lock — no need to pass `--test-threads=1` yourself.

3. Tear the harness down when done:

   ```bash
   cd <path-to-extend-helper-cli>/modules/extend-proxy
   make conformance-down
   ```

Expect `9 passed; 0 failed`.

## Notes

- Layers 2 and 3 are the only tests that prove real interop (handshake,
  version mismatch rejection, sidecar-initiated stream IDs, NACK on dial
  failure, RST propagation, the single-session 409, IAM auth headers) rather
  than the Rust side being internally self-consistent. Run them after any
  change to `src/protocol/`, `src/session/`, or `src/client/`.
- Pass `RUST_LOG=extend_proxy_client=debug` (the default when unset) plus
  `--nocapture` to see per-session/stream tracing output interleaved with
  test output, e.g.:

  ```bash
  cargo test -p extend-proxy-client --test sidecar_conformance -- --ignored --nocapture
  ```
