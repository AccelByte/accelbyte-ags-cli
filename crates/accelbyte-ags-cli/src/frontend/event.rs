//! Frontend lifecycle and progress events.

use ags_protocol::event::ProgressEvent;

/// Lifecycle events the CLI's invocation layer pushes into the frontend.
///
/// Every invocation — workflow run, synthesised single command, or static
/// command — is bracketed by a single paired `RunStarted` / `RunFinished`
/// model. Per-step events (`StepStarted`, `StepFinished`) and `Progress`
/// nest between them. There is no separate "workflow progress" variant —
/// progress events from a dispatch in flight always flow through
/// `Progress` and carry an optional `step_index` so the frontend can
/// attach the right context.
#[derive(Debug, Clone)]
pub enum FrontendEvent {
    /// The invocation is about to call into the runtime. Emitted once per
    /// run. `workflow_banner` carries the registered-workflow display name
    /// when a "Running workflow: <name>" banner should render, `None`
    /// otherwise (synthesised single commands and static commands). A
    /// registered workflow run emits this twice: a generic `None` from the
    /// shared lifecycle helper, then a `Some(name)` from the workflow
    /// adapter once execution clears the `--no-input` precheck.
    RunStarted {
        /// Registered-workflow display name, when a banner should render.
        workflow_banner: Option<String>,
    },
    /// The invocation's runtime call has returned. Emitted once per run as
    /// the single terminal lifecycle event.
    RunFinished {
        /// Final invocation outcome.
        outcome: RunOutcome,
    },
    /// Progress signal from a dispatch in flight. `step_index` is
    /// `Some(N)` when the progress originated from step N of a workflow
    /// run, `None` for non-workflow invocations. Populated by
    /// `FrontendSink`; not yet read by any terminal surface (the inline
    /// and plain frontends currently render progress without per-step
    /// context).
    Progress {
        /// 0-based step index, if the progress originated from a workflow.
        #[allow(dead_code)]
        step_index: Option<usize>,
        /// Underlying dispatch progress event.
        event: ProgressEvent,
    },
    /// Emitted at the top of each per-step iteration.
    StepStarted {
        /// 0-based step index.
        index: usize,
        /// Stable step id.
        id: String,
    },
    /// Emitted exactly once per started step, immediately after the
    /// step's final outcome is known.
    StepFinished {
        /// 0-based step index.
        index: usize,
        /// Single-line summary destined for the user and scrollback. Carries
        /// "<id> <status>", so the step id is not a separate field.
        summary: String,
        /// Per-step captures (label, value) — the resolved inputs and step-
        /// local options the step used, in display order. Empty for failure
        /// paths that abort before request assembly.
        captures: Vec<(String, String)>,
        /// Per-step outcome.
        outcome: StepOutcome,
    },
}

/// Outcome of the entire invocation (workflow or single-command).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// Every step succeeded and render succeeded.
    Success,
    /// At least one step failed, or render failed after a successful loop.
    Failed,
    /// User declined a confirmation prompt for some step.
    Cancelled,
}

/// Outcome of an individual workflow step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// Dispatch + capture succeeded.
    Success,
    /// Dispatch, capture, gather, confirm, or preview failed for this step.
    Failed,
    /// User declined a confirmation prompt for this step.
    Cancelled,
    /// An optional step whose dispatch failed: the step was skipped and the
    /// run continues.
    Skipped,
}
