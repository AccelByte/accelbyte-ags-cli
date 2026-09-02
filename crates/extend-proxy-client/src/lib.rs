//! Client-side Rust port of extend-proxy's multiplexed tunneling protocol.
//!
//! Ported from `extend-helper-cli`'s `modules/extend-proxy` Go module. This
//! crate is a library, not a CLI command — `ags extend remote-debug connect`
//! consumes it from the CLI invocation layer, the same way Go's
//! `internal/tunnel` and `internal/cmd/remote_debug.go` sit on top of
//! `pkg/client`.
//!
//! Follows `ags-protocol`/`ags-runtime`'s layering precedent internally:
//! `protocol` has zero networking/async dependencies (pure `serde` frame
//! types); `session`, `client`, and `forwarder` hold the actual `tokio`
//! logic and depend on `protocol`, never the reverse. This makes a future
//! split into a separate `extend-proxy-protocol` leaf crate mechanical if
//! `ags-cli` wants it later.
//!
//! Only the client role is exposed as a supported public surface (see
//! `client::Agent`). `session::Session` internally also supports a server
//! role, used only by this crate's own tests as an in-process stand-in for
//! the sidecar counterpart. The real conformance oracle — a Go test-peer
//! built from `modules/extend-proxy` source, plus the full sidecar
//! docker-compose suite — is not yet wired up; those cross-language suites
//! live under `tests/` gated behind `#[ignore]`.
//!
//! ## Log-safety contract
//!
//! The CLI's session log bridges this crate's `tracing` output at INFO and
//! above directly to the user's terminal (stderr) and, under `--format json`,
//! wraps it in a JSON object. The output is **user-visible and treated as
//! log-safe**.
//!
//! All tracing events emitted by this crate at INFO, WARN, or ERROR level
//! **must never carry tokens, bearer headers, cookies, or other credential
//! material**. Diagnostic fields (status codes, stream IDs, error messages)
//! are fine; secrets are not. Review any new `tracing::info!`, `warn!`, or
//! `error!` call for this contract before merging.

pub mod client;
pub mod forwarder;
pub mod protocol;
pub mod session;
