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
    ResponseTrace, Section, SkeletonOutput, TokenState, VersionOutput, WorkflowCompletionView,
    WorkflowOutputItem, WorkflowOutputProvenance, WorkflowOutputView,
};

/// Top-level output produced by any command before rendering.
#[derive(Debug, Clone)]
pub enum CommandOutput {
    /// Output from auth subcommands (login, logout, status)
    Auth(AuthOutput),
    /// Output from config subcommands (get, set, unset)
    Config(ConfigOutput),
    /// Output from profile subcommands (list, create, use, show, delete, rename)
    Profile(ProfileOutput),
    /// Output from a service API call
    Service(Box<ApiOutput>),
    /// Final output from a multi-step workflow (or a 1-step workflow that
    /// declares outputs). Carries the alias map plus the completed-step
    /// record. Empty `outputs` is legal for multi-step workflows that don't
    /// declare aliases — the envelope still surfaces the completion panel and
    /// output view.
    Workflow {
        /// Workflow identifier this output came from.
        workflow_id: crate::workflow::WorkflowId,
        /// Alias map (`WorkflowOutputAlias.name` → captured value).
        outputs: std::collections::BTreeMap<String, serde_json::Value>,
        /// One terminal summary per executed step, in order. Not rendered by
        /// the human or JSON frontends — per-step lines stream live during the
        /// run via `WorkflowEvent::StepFinished`. Retained as the "all steps
        /// ran" record that integration tests assert on, and as the hook a
        /// future partial-run envelope would surface (see the JSON renderer's
        /// note on adding step-level status if partial output is introduced).
        step_summaries: Vec<String>,
        /// Resolved post-run completion panel, when the workflow authored one.
        /// `CommandOutput` is not a serde type, so this field carries no serde
        /// attribute.
        completion: Option<WorkflowCompletionView>,
        /// Human-only structured view (sections/labels/provenance). `None`
        /// for workflows that declare no view (e.g. competitive-multiplayer).
        /// Not part of the JSON output — `--format json` uses `outputs`.
        output_view: Option<WorkflowOutputView>,
    },
    /// `--dry-run` output for a multi-step workflow (or a 1-step workflow
    /// that declares outputs). Carries per-step previews with placeholder
    /// outputs that flowed into downstream steps.
    WorkflowDryRun {
        /// Workflow identifier this dry-run came from.
        workflow_id: crate::workflow::WorkflowId,
        /// Per-step previews in execution order.
        step_previews: Vec<crate::workflow::StepDryRunPreview>,
    },
    /// The registered-workflow catalogue, emitted by `ags workflow list`.
    WorkflowCatalogue {
        /// One entry per registered workflow, in alphabetical id order.
        entries: Vec<crate::workflow::WorkflowListEntry>,
    },
    /// Output from a `--dry-run` invocation showing what would be sent
    DryRun(crate::result::DryRunResult),
    /// Output from `ags doctor` diagnostics
    Doctor(crate::diagnostics::DoctorResult),
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
