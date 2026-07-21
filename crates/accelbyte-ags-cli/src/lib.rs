//! Library root — exposes public modules for integration tests.

/// Error types, metadata, and exit code mapping
pub mod errors;
/// User-facing output: consumes structured protocol types and produces bytes
pub mod frontend;
/// CLI invocation: argv parsing, command tree, and dispatch to the runtime façade
pub mod invocation;
