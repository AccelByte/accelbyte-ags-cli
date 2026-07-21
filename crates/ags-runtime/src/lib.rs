//! Shared runtime for the AGS CLI workspace.
//!
//! This crate contains every piece of business logic that sits between the
//! CLI's invocation layer and the bundled OpenAPI specs: command execution,
//! OAuth flows, configuration storage, the spec catalogue, dispatch and
//! response classification, and diagnostics. It also exposes a small
//! `support` module of shared utilities (file-system helpers, output-sink
//! resolution, string transforms) used by both the runtime and the CLI
//! frontend.
//!
//! The only sibling dependency allowed is `ags-protocol`, which holds the
//! typed contracts crossing the crate boundary. Nothing here may depend on
//! the `accelbyte-ags-cli` crate or its frontend types.
pub mod catalogue;
pub mod runtime;
pub mod support;
