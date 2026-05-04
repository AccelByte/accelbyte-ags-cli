//! Top-level command output envelope.
//!
//! This module defines the canonical command-level output variants produced by
//! runtime-facing code. The richer payload and view structs that hang off
//! those variants live in `protocol::output_views` and are re-exported here to
//! preserve a single import surface for callers.
//!
//! In the protocol split:
//! - `output` owns the outer command envelope (`CommandOutput`)
//! - `result` owns shaped service payloads such as `CommandResult`
//! - `output_views` owns the supporting view/presentation payloads attached to
//!   `CommandOutput` variants

pub use super::output_views::{
    ApiBody, ApiOutput, ApiSuccess, AuthActionData, AuthActionStatus, AuthOutput, AuthSource,
    AuthStatusData, AuthView, BinaryWrittenDestination, BinaryWrittenOutput, CommandIntent,
    CompletionsOutput, ConfigOutput, ConfigView, DescribeOutput, ExecutionTrace, FieldEntry,
    LogoutAllData, LogoutData, OperationWarning, Presence, ProfileOutput, ProfileShowData,
    ProfileSummary, ProfileView, RefreshMode, RefreshSpecsOutput, RequestTrace, ResolutionTrace,
    ResponseTrace, Section, SkeletonOutput, TokenState, VersionOutput,
};

/// Top-level output produced by any command before rendering.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum CommandOutput {
    /// Output from auth subcommands (login, logout, status)
    Auth(AuthOutput),
    /// Output from config subcommands (get, set, unset)
    Config(ConfigOutput),
    /// Output from profile subcommands (list, create, use, show, delete, rename)
    Profile(ProfileOutput),
    /// Output from a service API call
    Service(Box<ApiOutput>),
    /// Output from a `--dry-run` invocation showing what would be sent
    DryRun(crate::protocol::result::DryRunResult),
    /// Output from `ags doctor` diagnostics
    Doctor(crate::protocol::diagnostics::DoctorResult),
    /// Output from `ags completions`: shell script + optional detection hint.
    Completions(CompletionsOutput),
    /// Output from `ags version` / `--version` / `-V`
    Version(VersionOutput),
    /// JSON request-body template emitted by `--skeleton`
    Skeleton(SkeletonOutput),
    /// JSON introspection envelope emitted by `ags describe`
    Describe(DescribeOutput),
    /// Output from `ags refresh-specs`
    RefreshSpecs(RefreshSpecsOutput),
    /// A binary (or raw text via `--output`) response body was written to
    /// disk or stdout. The renderer emits a confirmation line on stderr
    /// when the destination is a file; when the destination is stdout,
    /// the renderer emits nothing (the bytes themselves are the output).
    BinaryWritten(BinaryWrittenOutput),
}
