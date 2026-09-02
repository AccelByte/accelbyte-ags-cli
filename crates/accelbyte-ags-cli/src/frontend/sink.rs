//! `FrontendSink` — bridges `ags_protocol::event::ProgressSink` (runtime
//! contract) to `Frontend::on_event(&FrontendEvent::Progress(_))` (trait
//! contract). Zero state; borrows the frontend mutably for the lifetime
//! of a single runtime call.

use ags_protocol::error::RuntimeError;
use ags_protocol::event::{ProgressEvent, ProgressSink};
use ags_protocol::workflow::{
    CompiledStep, GatherResult, StepOutcome as RuntimeStepOutcome, StepPreview, SuppliedInputView,
    WorkflowEvent, WorkflowFrontend, WorkflowInputNeeded,
};
use ags_runtime::runtime::telemetry::{TelemetryClient, WorkflowStepContext};

use crate::errors::CliError;
use crate::frontend::event::{FrontendEvent, StepOutcome};
use crate::frontend::{ExecutionInteraction, Frontend};

/// Step-telemetry identity + client for one registered `ags workflow run`
/// invocation. Built once (the client requires an `.await` to construct) and
/// handed to the adapter so its synchronous `on_event` can call the
/// synchronous `TelemetryClient::capture` directly — no `.await` needed
/// inside the `WorkflowFrontend` trait's non-async callback.
pub struct WorkflowStepTelemetry {
    pub client: TelemetryClient,
    pub sub: String,
    pub context: WorkflowStepContext,
}

/// Adapts a runtime `ProgressSink` onto a CLI `Frontend` for one runtime call.
pub struct FrontendSink<'a> {
    frontend: &'a mut dyn Frontend,
}

impl<'a> FrontendSink<'a> {
    /// Wrap a frontend so the runtime can push `ProgressEvent`s into it.
    pub fn new(frontend: &'a mut dyn Frontend) -> Self {
        Self { frontend }
    }
}

impl ProgressSink for FrontendSink<'_> {
    fn on_event(&mut self, event: ProgressEvent) {
        self.frontend.on_event(&FrontendEvent::Progress {
            step_index: None,
            event,
        });
    }
}

/// Bridges the CLI's [`Frontend`] + [`ExecutionInteraction`] surfaces to the
/// runtime's [`WorkflowFrontend`] trait. Owned by the invocation layer;
/// passed to `Executor::execute`. Holds a `&mut dyn Frontend` (for lifecycle
/// and progress rendering) and a `&mut dyn ExecutionInteraction` (for
/// gather/confirm) for the duration of one workflow run.
pub struct ExecutionFrontendAdapter<'a> {
    frontend: &'a mut dyn Frontend,
    interaction: &'a mut dyn ExecutionInteraction,
    /// When true, workflow lifecycle events (the banner-bearing
    /// `RunStarted`, `StepStarted`, `StepFinished`) are NOT forwarded
    /// to the frontend — only `Progress` events are. Set for the synthesised
    /// 1-step workflow that backs a plain `ags <service> <op>` command, so
    /// the workflow chrome ("Running workflow…", "Step 1: ok") does not leak
    /// into single-command output.
    suppress_lifecycle: bool,
    /// `Some` only for a registered workflow run with telemetry enabled;
    /// always `None` for a synthesised single command (constructed via
    /// `new_for_synthesised_command`, which never sets this).
    step_telemetry: Option<WorkflowStepTelemetry>,
    /// step index → (service, operation) for the running workflow, captured
    /// from `WorkflowStarted` so step events can name the API they called
    /// without widening the protocol event.
    step_operations: Vec<(String, String)>,
    /// Run aggregate stashed from `WorkflowFinished`, kept for the caller to
    /// emit `cli.workflow.run_completed` after the executor returns. `None`
    /// until `WorkflowFinished` fires, which never happens on a `--no-input`
    /// precheck rejection or a `?` that propagates out of `skip_step`/
    /// `decide_step_failure` before the executor reaches it.
    run_facts: Option<ags_protocol::workflow::RunFacts>,
}

impl<'a> ExecutionFrontendAdapter<'a> {
    /// Wrap a frontend + interaction pair so the workflow executor can drive
    /// them. Forwards every event, including workflow lifecycle chrome — use
    /// for registered multi-step workflows.
    pub fn new(
        frontend: &'a mut dyn Frontend,
        interaction: &'a mut dyn ExecutionInteraction,
    ) -> Self {
        Self {
            frontend,
            interaction,
            suppress_lifecycle: false,
            step_telemetry: None,
            step_operations: Vec::new(),
            run_facts: None,
        }
    }

    /// Same as `new`, but attaches step telemetry — use for a registered
    /// multi-step workflow run when telemetry is enabled and a `sub` was
    /// resolved.
    pub fn new_with_telemetry(
        frontend: &'a mut dyn Frontend,
        interaction: &'a mut dyn ExecutionInteraction,
        telemetry: WorkflowStepTelemetry,
    ) -> Self {
        Self {
            frontend,
            interaction,
            suppress_lifecycle: false,
            step_telemetry: Some(telemetry),
            step_operations: Vec::new(),
            run_facts: None,
        }
    }

    /// Wrap a frontend + interaction pair for a synthesised single-command
    /// workflow: suppress workflow lifecycle events but still forward progress.
    pub fn new_for_synthesised_command(
        frontend: &'a mut dyn Frontend,
        interaction: &'a mut dyn ExecutionInteraction,
    ) -> Self {
        Self {
            frontend,
            interaction,
            suppress_lifecycle: true,
            step_telemetry: None,
            step_operations: Vec::new(),
            run_facts: None,
        }
    }

    /// Reclaim the step-telemetry sink and the stashed run aggregate after the
    /// executor has finished with this adapter. The exact `TelemetryClient`
    /// instance matters: `posthog-rs`'s send queue is owned per-`Client` (a
    /// dedicated background thread + channel per instance, confirmed by
    /// reading `posthog-rs` 0.14.3's `client/transport.rs`), so a *different*
    /// client's `flush()` would not touch these events at all — the caller
    /// must flush this exact instance.
    pub fn into_telemetry_parts(
        self,
    ) -> (
        Option<Box<WorkflowStepTelemetry>>,
        Option<ags_protocol::workflow::RunFacts>,
    ) {
        (self.step_telemetry.map(Box::new), self.run_facts)
    }

    /// Service and operation for `index`, or empty strings when the map has no
    /// entry (a synthesised run, or an event arriving before `WorkflowStarted`).
    fn operation_for(&self, index: usize) -> (String, String) {
        self.step_operations
            .get(index)
            .cloned()
            .unwrap_or_else(|| (String::new(), String::new()))
    }
}

impl<'a> WorkflowFrontend for ExecutionFrontendAdapter<'a> {
    fn on_event(&mut self, event: &WorkflowEvent) {
        if self.suppress_lifecycle && !matches!(event, WorkflowEvent::Progress { .. }) {
            return;
        }
        let translated = match event {
            WorkflowEvent::WorkflowStarted { compiled } => {
                self.step_operations = compiled
                    .steps
                    .iter()
                    .map(|step| match &step.operation {
                        Some(op) => (
                            op.service.as_str().to_string(),
                            op.operation.as_str().to_string(),
                        ),
                        None => match &step.action {
                            Some(action_name) => ("local".to_string(), action_name.clone()),
                            None => (String::new(), String::new()),
                        },
                    })
                    .collect();
                FrontendEvent::RunStarted {
                    workflow_banner: Some(compiled.name.clone()),
                }
            }
            WorkflowEvent::StepStarted { index, id } => {
                // Computed before `&self.step_telemetry` is taken so the
                // shared `self` borrow for `operation_for` never overlaps
                // with the field borrow held across the telemetry call.
                let (service, operation) = self.operation_for(*index);
                if let Some(t) = &self.step_telemetry {
                    ags_runtime::runtime::telemetry::capture_workflow_step_started(
                        &t.client, &t.sub, &t.context, *index, id, &service, &operation,
                    );
                }
                FrontendEvent::StepStarted {
                    index: *index,
                    id: id.clone(),
                }
            }
            WorkflowEvent::StepFinished {
                index,
                id,
                summary,
                captures,
                outcome,
                reason,
                attempts,
                duration_ms,
                error,
            } => {
                // Same ordering rationale as `StepStarted` above.
                let (service, operation) = self.operation_for(*index);
                if let Some(t) = &self.step_telemetry {
                    let facts = ags_runtime::runtime::telemetry::StepCompletedFacts {
                        outcome: step_outcome_telemetry_label(*outcome),
                        reason: reason.map(|r| r.as_label()),
                        attempts: *attempts,
                        duration_ms: *duration_ms,
                        error_class: error.as_ref().map(|e| e.class),
                        http_status: error.as_ref().and_then(|e| e.http_status),
                        error_code: error.as_ref().and_then(|e| e.code.clone()),
                        input_fields: error
                            .as_ref()
                            .map(|e| e.input_fields.clone())
                            .unwrap_or_default(),
                        service,
                        operation,
                    };
                    ags_runtime::runtime::telemetry::capture_workflow_step_completed(
                        &t.client, &t.sub, &t.context, *index, id, &facts,
                    );
                }
                FrontendEvent::StepFinished {
                    index: *index,
                    summary: summary.clone(),
                    captures: captures.clone(),
                    outcome: translate_step_outcome(*outcome),
                }
            }
            // The shared lifecycle helper owns the single `RunFinished`
            // event; the adapter does not emit a finish event of its own.
            // Stash the run aggregate so the caller can emit
            // `cli.workflow.run_completed` after the executor returns.
            WorkflowEvent::WorkflowFinished { facts, .. } => {
                self.run_facts = Some(facts.clone());
                return;
            }
            WorkflowEvent::Progress { step_index, event } => FrontendEvent::Progress {
                step_index: *step_index,
                event: event.clone(),
            },
        };
        self.frontend.on_event(&translated);
    }

    fn present_briefing(
        &mut self,
        briefing: &ags_protocol::workflow::WorkflowBriefing,
        workflow_name: &str,
    ) -> Result<bool, RuntimeError> {
        self.interaction
            .present_briefing(briefing, workflow_name)
            .map_err(cli_error_to_runtime_error)
    }

    fn gather_workflow_inputs(
        &mut self,
        needed: &[WorkflowInputNeeded],
        step_context: &CompiledStep,
        supplied: &[SuppliedInputView],
    ) -> Result<GatherResult, RuntimeError> {
        self.interaction
            .gather_workflow_inputs(needed, step_context, supplied)
            .map_err(cli_error_to_runtime_error)
    }

    fn confirm_step(
        &mut self,
        step: &CompiledStep,
        preview: &StepPreview,
    ) -> Result<ags_protocol::workflow::StepConfirmOutcome, RuntimeError> {
        self.interaction
            .confirm_step(step, preview)
            .map_err(cli_error_to_runtime_error)
    }

    fn review_step(
        &mut self,
        plan: &ags_protocol::workflow::StepFieldPlan,
    ) -> Result<ags_protocol::workflow::StepReviewOutcome, RuntimeError> {
        self.interaction
            .review_step(plan)
            .map_err(cli_error_to_runtime_error)
    }

    fn resolve_step_failure(
        &mut self,
        step: &CompiledStep,
        error: &ags_protocol::error::RuntimeError,
        allow_skip: bool,
    ) -> Result<ags_protocol::workflow::StepFailureAction, ags_protocol::error::RuntimeError> {
        self.interaction
            .resolve_step_failure(step, error, allow_skip)
            .map_err(cli_error_to_runtime_error)
    }

    fn collect_workflow_inputs(
        &mut self,
        specs: &[ags_protocol::workflow::WorkflowInputSpec],
        current: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Option<ags_protocol::workflow::CollectOutcome>, RuntimeError> {
        self.interaction
            .collect_workflow_inputs(specs, current)
            .map_err(cli_error_to_runtime_error)
    }
}

/// Map a runtime [`StepOutcome`] to the CLI's [`StepOutcome`].
fn translate_step_outcome(outcome: RuntimeStepOutcome) -> StepOutcome {
    match outcome {
        RuntimeStepOutcome::Success => StepOutcome::Success,
        RuntimeStepOutcome::Failed => StepOutcome::Failed,
        RuntimeStepOutcome::Cancelled => StepOutcome::Cancelled,
        RuntimeStepOutcome::Skipped => StepOutcome::Skipped,
    }
}

/// Map a runtime `StepOutcome` to the telemetry `outcome` property value —
/// distinct from `translate_step_outcome` (UI-facing enum) since telemetry
/// wants a stable string, not a CLI-crate type.
fn step_outcome_telemetry_label(outcome: RuntimeStepOutcome) -> &'static str {
    match outcome {
        RuntimeStepOutcome::Success => "success",
        RuntimeStepOutcome::Failed => "failed",
        RuntimeStepOutcome::Cancelled => "cancelled",
        RuntimeStepOutcome::Skipped => "skipped",
    }
}

/// Convert a CLI-layer [`CliError`] into a runtime-layer [`RuntimeError`].
///
/// Used by the workflow adapter when bubbling gather/confirm errors back
/// up to the executor. The reverse direction (`From<RuntimeError> for
/// CliError`) already exists in `errors.rs`; this direction cannot be a
/// `From` impl because `ags-protocol` cannot depend on the CLI crate.
fn cli_error_to_runtime_error(err: CliError) -> RuntimeError {
    use ags_protocol::error::RuntimeErrorKind;
    let (kind, message) = match err {
        CliError::Usage { message, .. } => (RuntimeErrorKind::Validation, message),
        CliError::Auth { message, .. } => (RuntimeErrorKind::NotAuthenticated, message),
        CliError::Api { message, .. } => (
            RuntimeErrorKind::Upstream {
                status: 0,
                code: None,
            },
            message,
        ),
        CliError::Network { message, .. } => (RuntimeErrorKind::Network, message),
        CliError::Internal(e) => (RuntimeErrorKind::Internal, e.to_string()),
    };
    RuntimeError {
        kind,
        message,
        details: None,
        hint: None,
        trace: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::event::FrontendEvent;

    /// Test double recording every event the adapter forwards.
    #[derive(Default)]
    struct RecordingFrontend {
        events: Vec<String>,
    }

    impl Frontend for RecordingFrontend {
        fn on_event(&mut self, event: &FrontendEvent) {
            self.events.push(format!("{event:?}"));
        }
        fn render(
            &mut self,
            _output: &ags_protocol::output::CommandOutput,
        ) -> Result<(), crate::errors::CliError> {
            unimplemented!()
        }
        fn render_error(&mut self, _err: &crate::errors::CliError) {}
        fn render_warning(&mut self, _msg: &str, _reason: Option<&str>, _tip: Option<&str>) {}
        fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {}
        fn finish(self: Box<Self>) -> Result<(), crate::errors::CliError> {
            Ok(())
        }
    }

    #[test]
    fn test_frontend_sink_forwards_progress_event_as_frontend_event() {
        let mut frontend = RecordingFrontend::default();
        let mut sink = FrontendSink::new(&mut frontend);
        sink.on_event(ProgressEvent::Finished);
        assert_eq!(frontend.events.len(), 1);
        assert!(frontend.events[0].contains("Progress"));
        assert!(frontend.events[0].contains("Finished"));
    }

    #[test]
    fn test_translate_step_outcome_maps_skipped() {
        assert_eq!(
            translate_step_outcome(RuntimeStepOutcome::Skipped),
            StepOutcome::Skipped
        );
    }

    /// A registered workflow's step lifecycle must emit exactly one
    /// `cli.workflow.step_started` and one `cli.workflow.step_completed` per
    /// step, carrying the step's real index/id/outcome — proven here via a
    /// disabled (no-op) `TelemetryClient`, whose `capture` calls are still
    /// exercised (a disabled client is still hit, it just doesn't queue to a
    /// real PostHog buffer) by driving two `StepStarted`/`StepFinished` pairs
    /// through the adapter and confirming it doesn't panic — the ags-runtime
    /// crate's own tests (Task 4) cover the exact JSON shape of each event.
    ///
    /// The `StepFinished` event here deliberately uses an `id` ("create
    /// lobby") whose first whitespace-token differs from the id embedded in
    /// `summary` ("create-lobby ok" → `split_whitespace().next()` would give
    /// `"create-lobby"`, not `"create lobby"`). This proves (at least at the
    /// compile/no-panic level — a disabled client can't be inspected for its
    /// queued payload) that the adapter now passes through the event's real
    /// `id` field for `step_completed` telemetry instead of reverse-
    /// engineering it from `summary`; the exact payload assertion lives in
    /// `ags-runtime`'s `capture_workflow_step_completed` tests.
    #[test]
    fn test_execution_frontend_adapter_emits_step_telemetry_when_configured() {
        use ags_protocol::workflow::{StepOutcome as RuntimeStepOutcome, WorkflowEvent};
        use ags_runtime::runtime::telemetry::{TelemetryClient, WorkflowStepContext};

        let mut recording = RecordingFrontend::default();
        struct NoopInteraction;
        impl crate::frontend::ExecutionInteraction for NoopInteraction {
            fn gather_workflow_inputs(
                &mut self,
                _needed: &[ags_protocol::workflow::WorkflowInputNeeded],
                _step_context: &ags_protocol::workflow::CompiledStep,
                _supplied: &[ags_protocol::workflow::SuppliedInputView],
            ) -> Result<ags_protocol::workflow::GatherResult, CliError> {
                Ok(ags_protocol::workflow::GatherResult::default())
            }
            fn confirm_step(
                &mut self,
                _step: &ags_protocol::workflow::CompiledStep,
                _preview: &ags_protocol::workflow::StepPreview,
            ) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
                Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
            }
        }
        let mut interaction = NoopInteraction;

        let telemetry = WorkflowStepTelemetry {
            client: TelemetryClient::disabled_for_test(),
            sub: "user-123".to_string(),
            context: WorkflowStepContext {
                run_id: "run-1".into(),
                workflow_id: "wf-1".into(),
                steps_total: 1,
                cli_version: "1.2.3".into(),
                is_dry_run: false,
                ui_surface: "fullscreen",
            },
        };
        let mut adapter = ExecutionFrontendAdapter::new_with_telemetry(
            &mut recording,
            &mut interaction,
            telemetry,
        );

        // Must not panic — this is the whole assertion, since capture() with a
        // disabled client is a documented no-op.
        adapter.on_event(&WorkflowEvent::StepStarted {
            index: 0,
            id: "create lobby".into(),
        });
        adapter.on_event(&WorkflowEvent::StepFinished {
            index: 0,
            id: "create lobby".into(),
            summary: "create-lobby ok".into(),
            captures: vec![],
            outcome: RuntimeStepOutcome::Success,
            reason: None,
            attempts: 1,
            duration_ms: 0,
            error: None,
        });
    }
}

#[cfg(test)]
mod adapter_tests {
    use super::*;
    use ags_protocol::workflow::{
        CompiledStep, CompiledWorkflow, RunFacts, RunOutcome as RuntimeRunOutcome, WorkflowId,
    };

    /// Recording frontend used to verify event translation.
    #[derive(Default)]
    struct RecordingFrontend {
        events: Vec<FrontendEvent>,
    }

    impl crate::frontend::Frontend for RecordingFrontend {
        fn on_event(&mut self, event: &FrontendEvent) {
            self.events.push(event.clone());
        }
        fn render(
            &mut self,
            _output: &ags_protocol::output::CommandOutput,
        ) -> Result<(), CliError> {
            unimplemented!()
        }
        fn render_error(&mut self, _err: &CliError) {}
        fn render_warning(&mut self, _msg: &str, _reason: Option<&str>, _tip: Option<&str>) {}
        fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {}
        fn finish(self: Box<Self>) -> Result<(), CliError> {
            Ok(())
        }
    }

    /// Recording interaction used to verify gather/confirm calls land on the
    /// interaction side of the split adapter.
    #[derive(Default)]
    struct RecordingInteraction {
        gather_calls: usize,
        confirm_calls: usize,
        seen_briefing: Vec<(ags_protocol::workflow::WorkflowBriefing, String)>,
        briefing_reply: Option<Result<bool, CliError>>, // None → default Ok(true)
        next_failure_action: Option<ags_protocol::workflow::StepFailureAction>,
    }

    impl ExecutionInteraction for RecordingInteraction {
        fn present_briefing(
            &mut self,
            briefing: &ags_protocol::workflow::WorkflowBriefing,
            workflow_name: &str,
        ) -> Result<bool, CliError> {
            self.seen_briefing
                .push((briefing.clone(), workflow_name.to_string()));
            match self.briefing_reply.take() {
                Some(r) => r,
                None => Ok(true),
            }
        }

        fn gather_workflow_inputs(
            &mut self,
            _needed: &[WorkflowInputNeeded],
            _step_context: &CompiledStep,
            _supplied: &[SuppliedInputView],
        ) -> Result<GatherResult, CliError> {
            self.gather_calls += 1;
            Ok(GatherResult::default())
        }
        fn confirm_step(
            &mut self,
            _step: &CompiledStep,
            _preview: &StepPreview,
        ) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
            self.confirm_calls += 1;
            Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
        }
        fn resolve_step_failure(
            &mut self,
            _step: &CompiledStep,
            _error: &ags_protocol::error::RuntimeError,
            _allow_skip: bool,
        ) -> Result<ags_protocol::workflow::StepFailureAction, CliError> {
            Ok(self
                .next_failure_action
                .unwrap_or(ags_protocol::workflow::StepFailureAction::Cancel))
        }
    }

    #[test]
    fn test_adapter_delegates_resolve_step_failure() {
        use ags_protocol::workflow::{StepFailureAction, WorkflowFrontend};
        let mut frontend = RecordingFrontend::default();
        let mut interaction = RecordingInteraction {
            next_failure_action: Some(StepFailureAction::Skip),
            ..Default::default()
        };
        let mut adapter = ExecutionFrontendAdapter::new(&mut frontend, &mut interaction);
        let step = minimal_compiled_step();
        let error = ags_protocol::error::RuntimeError::internal("boom");
        let action = adapter.resolve_step_failure(&step, &error, true).unwrap();
        assert!(matches!(action, StepFailureAction::Skip));
    }

    /// Build a minimal `CompiledStep` for use in interaction-routing tests.
    fn minimal_compiled_step() -> CompiledStep {
        use ags_protocol::catalogue::{OperationId, ServiceId};
        use ags_protocol::workflow::OperationReference;
        CompiledStep {
            id: "test-step".to_string(),
            index: 0,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new("iam"),
                operation: OperationId::new("testOp"),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![],
            auto_derived: vec![],
        }
    }

    /// Build a minimal `StepPreview` for use in confirm-routing tests.
    fn minimal_step_preview() -> StepPreview {
        use ags_protocol::catalogue::{HttpMethod, MutationClass, OperationId, ServiceId};
        use ags_protocol::result::CommandPreview;
        StepPreview {
            workflow_name: "Test Workflow".to_string(),
            step_id: "test-step".to_string(),
            step_label: "Test Step".to_string(),
            step_index: 0,
            step_total: 1,
            command: CommandPreview {
                service: ServiceId::new("iam"),
                operation_id: OperationId::new("testOp"),
                summary: "test".to_string(),
                http_method: HttpMethod::Get,
                url: "https://example.test/iam/test".to_string(),
                mutation_class: MutationClass::ReadOnly,
                confirmation_required: false,
                warnings: vec![],
            },
        }
    }

    /// Build a minimal `CompiledWorkflow` for use in tests — only the fields
    /// the adapter inspects at translation time.
    fn minimal_compiled_workflow() -> CompiledWorkflow {
        CompiledWorkflow {
            id: WorkflowId::new("test-wf"),
            name: "Test Workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![],
            outputs: vec![],
            completion: None,
        }
    }

    #[test]
    fn test_adapter_translates_workflow_started() {
        let mut frontend = RecordingFrontend::default();
        let mut interaction = RecordingInteraction::default();
        let mut adapter = ExecutionFrontendAdapter::new(&mut frontend, &mut interaction);
        let compiled = minimal_compiled_workflow();
        adapter.on_event(&WorkflowEvent::WorkflowStarted {
            compiled: compiled.clone(),
        });
        assert_eq!(frontend.events.len(), 1);
        assert!(
            matches!(
                &frontend.events[0],
                FrontendEvent::RunStarted { workflow_banner: Some(name) }
                    if *name == compiled.name
            ),
            "expected RunStarted with banner, got {:?}",
            frontend.events[0]
        );
    }

    /// `WorkflowFinished`'s `facts` must be stashed on the adapter and handed
    /// back verbatim by `into_telemetry_parts`, so `drive_run` can build
    /// `cli.workflow.run_completed` from the executor's real aggregate.
    #[test]
    fn test_adapter_stashes_run_facts_from_workflow_finished() {
        let mut frontend = RecordingFrontend::default();
        let mut interaction = RecordingInteraction::default();
        let mut adapter = ExecutionFrontendAdapter::new(&mut frontend, &mut interaction);
        let facts = RunFacts {
            steps_succeeded: 2,
            steps_started: 2,
            ..RunFacts::default()
        };
        adapter.on_event(&WorkflowEvent::WorkflowFinished {
            outcome: RuntimeRunOutcome::Success,
            facts: facts.clone(),
        });
        let (_telemetry, stashed) = adapter.into_telemetry_parts();
        assert_eq!(stashed, Some(facts));
    }

    /// The adapter no longer emits a finish event of its own: the shared
    /// lifecycle helper owns the single `RunFinished`, so a runtime
    /// `WorkflowFinished` produces no frontend event.
    #[test]
    fn test_adapter_drops_workflow_finished() {
        for runtime_outcome in [
            RuntimeRunOutcome::Success,
            RuntimeRunOutcome::Failed,
            RuntimeRunOutcome::Cancelled,
        ] {
            let mut frontend = RecordingFrontend::default();
            let mut interaction = RecordingInteraction::default();
            let mut adapter = ExecutionFrontendAdapter::new(&mut frontend, &mut interaction);
            adapter.on_event(&WorkflowEvent::WorkflowFinished {
                outcome: runtime_outcome,
                facts: RunFacts::default(),
            });
            assert_eq!(
                frontend.events.len(),
                0,
                "adapter must not emit a finish event, got {:?}",
                frontend.events
            );
        }
    }

    /// The banner-bearing `RunStarted` is gated on the explicit lifecycle
    /// mode: a full-lifecycle adapter translates `WorkflowStarted`, a
    /// suppressed-lifecycle adapter (synthesised single command) drops it.
    #[test]
    fn test_adapter_workflow_started_gated_on_lifecycle_mode() {
        // Full-lifecycle adapter: banner present.
        let mut frontend = RecordingFrontend::default();
        let mut interaction = RecordingInteraction::default();
        let mut adapter = ExecutionFrontendAdapter::new(&mut frontend, &mut interaction);
        adapter.on_event(&WorkflowEvent::WorkflowStarted {
            compiled: minimal_compiled_workflow(),
        });
        assert_eq!(frontend.events.len(), 1);
        assert!(matches!(
            &frontend.events[0],
            FrontendEvent::RunStarted {
                workflow_banner: Some(_)
            }
        ));

        // Suppressed-lifecycle adapter: dropped entirely.
        let mut frontend = RecordingFrontend::default();
        let mut interaction = RecordingInteraction::default();
        let mut adapter =
            ExecutionFrontendAdapter::new_for_synthesised_command(&mut frontend, &mut interaction);
        adapter.on_event(&WorkflowEvent::WorkflowStarted {
            compiled: minimal_compiled_workflow(),
        });
        assert_eq!(
            frontend.events.len(),
            0,
            "suppressed adapter must drop WorkflowStarted, got {:?}",
            frontend.events
        );
    }

    /// Progress and lifecycle events are routed to the frontend side; the
    /// interaction side sees nothing.
    #[test]
    fn test_adapter_routes_events_to_frontend_not_interaction() {
        let mut frontend = RecordingFrontend::default();
        let mut interaction = RecordingInteraction::default();
        let mut adapter = ExecutionFrontendAdapter::new(&mut frontend, &mut interaction);
        adapter.on_event(&WorkflowEvent::WorkflowStarted {
            compiled: minimal_compiled_workflow(),
        });
        adapter.on_event(&WorkflowEvent::StepStarted {
            index: 0,
            id: "s".to_string(),
        });
        assert_eq!(frontend.events.len(), 2);
        assert_eq!(interaction.gather_calls, 0);
        assert_eq!(interaction.confirm_calls, 0);
    }

    /// Gather/confirm calls are routed to the interaction side; the frontend
    /// side records no events for them.
    #[test]
    fn test_adapter_routes_gather_and_confirm_to_interaction() {
        let mut frontend = RecordingFrontend::default();
        let mut interaction = RecordingInteraction::default();
        let mut adapter = ExecutionFrontendAdapter::new(&mut frontend, &mut interaction);
        let step = minimal_compiled_step();
        let preview = minimal_step_preview();
        adapter.gather_workflow_inputs(&[], &step, &[]).unwrap();
        adapter.confirm_step(&step, &preview).unwrap();
        assert_eq!(interaction.gather_calls, 1);
        assert_eq!(interaction.confirm_calls, 1);
        assert_eq!(frontend.events.len(), 0);
    }

    #[test]
    fn test_adapter_forwards_present_briefing_args_and_result() {
        use ags_protocol::workflow::WorkflowBriefing;
        let briefing = WorkflowBriefing {
            overview: "ov".into(),
            prerequisites: vec!["p".into()],
            creates: vec!["c".into()],
        };
        let mut frontend = RecordingFrontend::default();
        let mut interaction = RecordingInteraction {
            briefing_reply: Some(Ok(false)),
            ..Default::default()
        };
        let mut adapter = ExecutionFrontendAdapter::new(&mut frontend, &mut interaction);
        let result = adapter.present_briefing(&briefing, "WF").expect("ok");
        assert!(!result);
        assert_eq!(interaction.seen_briefing.len(), 1);
        assert_eq!(interaction.seen_briefing[0].1, "WF");
        assert_eq!(interaction.seen_briefing[0].0, briefing);
    }

    /// A local-action step must be identified as `("local", "<action>")` in
    /// the step_operations map, not as two empty strings. Empty strings would
    /// group every local-step invocation into a single unnamed telemetry bucket.
    #[test]
    fn test_adapter_local_step_yields_local_telemetry_label() {
        use ags_protocol::workflow::WorkflowFrontend;
        let mut frontend = RecordingFrontend::default();
        let mut interaction = RecordingInteraction::default();
        let mut adapter = ExecutionFrontendAdapter::new(&mut frontend, &mut interaction);

        let mut local_step = minimal_compiled_step();
        local_step.operation = None;
        local_step.kind = ags_protocol::workflow::StepKind::Local;
        local_step.action = Some("ams/upload-image".to_string());
        local_step.index = 0;

        let mut compiled = minimal_compiled_workflow();
        compiled.steps = vec![local_step];

        adapter.on_event(&WorkflowEvent::WorkflowStarted {
            compiled: compiled.clone(),
        });

        let (service, operation) = adapter.operation_for(0);
        assert_eq!(
            service, "local",
            "local step service label must be 'local', not empty"
        );
        assert_eq!(
            operation, "ams/upload-image",
            "local step operation label must be the action name, not empty"
        );
    }

    #[test]
    fn test_adapter_maps_cli_error_to_runtime_error() {
        use ags_protocol::workflow::WorkflowBriefing;
        let briefing = WorkflowBriefing {
            overview: "x".into(),
            prerequisites: vec![],
            creates: vec![],
        };
        let mut frontend = RecordingFrontend::default();
        let mut interaction = RecordingInteraction {
            briefing_reply: Some(Err(CliError::Usage {
                message: "nope".into(),
                metadata: None,
            })),
            ..Default::default()
        };
        let mut adapter = ExecutionFrontendAdapter::new(&mut frontend, &mut interaction);
        let err = adapter.present_briefing(&briefing, "WF").expect_err("err");
        assert_eq!(err.message, "nope");
    }
}
