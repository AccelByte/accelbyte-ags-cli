//! Library root — exposes public modules for integration tests.

/// Service catalogue: spec loading, parsing, and caching
pub mod catalogue;
/// Error types, metadata, and exit code mapping
pub mod errors;
/// User-facing output: consumes structured protocol types and produces bytes
pub mod frontend;
/// CLI invocation: argv parsing, command tree, and dispatch to the runtime façade
pub mod invocation;
/// Protocol types: serde-enabled wire shapes that cross the runtime boundary
pub mod protocol;
/// Execution core: command catalogue, domain dispatch, and ambient context.
pub mod runtime;
/// Shared utilities: filesystem helpers, string transforms, and small process utilities
pub mod support;
