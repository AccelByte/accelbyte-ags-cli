//! End-to-end workflow integration test.
//!
//! Drives the workflow runtime through the CLI's `ExecutionFrontendAdapter`,
//! proving the adapter correctly bridges the `ags-runtime` `WorkflowFrontend`
//! trait to the CLI `Frontend` trait.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ags_protocol::catalogue::{
    ApiVersion, HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema,
    ResourceSchema, ScopeEntry, ServiceId, ServiceSchema,
};
use ags_protocol::error::RuntimeError;
use ags_protocol::output::CommandOutput;
use ags_protocol::workflow::{
    CompiledStep, CompiledWorkflow, OperationReference, StepPreview, WorkflowId,
    WorkflowInputNeeded, WorkflowInputSpec, WorkflowOutputAlias,
};
use ags_runtime::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
use ags_runtime::runtime::execution::ExecutionContext;
use ags_runtime::runtime::workflows::executor::{Executor, RunContext};
use ags_runtime::runtime::workflows::{RunOptions, RunOutcome};
use ags_runtime::runtime::Runtime;

use ags::errors::CliError;
use ags::frontend::event::{FrontendEvent, StepOutcome as CliFrontendStepOutcome};
use ags::frontend::sink::ExecutionFrontendAdapter;
use ags::frontend::Frontend;

// ── Scripted HTTP client ────────────────────────────────────────────────────

/// HTTP client that returns pre-scripted responses in FIFO order. Panics
/// if more calls are made than responses were enqueued.
struct QueuedClient {
    responses: Arc<Mutex<Vec<Result<HttpResponse, RuntimeError>>>>,
}

impl QueuedClient {
    /// Build a client that drains the given response queue in order.
    fn new(responses: Vec<Result<HttpResponse, RuntimeError>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
        }
    }
}

#[async_trait::async_trait]
impl HttpClient for QueuedClient {
    async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
        self.responses.lock().unwrap().remove(0)
    }
}

/// Build an `Ok(200)` response with a JSON text body.
fn ok_json(body: &str) -> Result<HttpResponse, RuntimeError> {
    Ok(HttpResponse {
        status: 200,
        body: HttpBody::Text(body.to_string()),
    })
}

/// A scripted HTTP response with an arbitrary status (e.g. a 409 conflict).
fn status_json(status: u16, body: &str) -> Result<HttpResponse, RuntimeError> {
    Ok(HttpResponse {
        status,
        body: HttpBody::Text(body.to_string()),
    })
}

// ── Minimal service schema ──────────────────────────────────────────────────

/// Build an `OperationSchema` for a GET at `path` with no parameters.
fn simple_get_operation(op_id: &str, path: &str) -> OperationSchema {
    OperationSchema {
        id: OperationId::new(op_id),
        name: op_id.to_ascii_lowercase(),
        summary: op_id.into(),
        description: None,
        mutation_class: MutationClass::ReadOnly,
        http_method: HttpMethod::Get,
        path_template: path.into(),
        parameters: vec![],
        request_body: None,
        response: None,
        permissions: vec![],
        scope: String::new(),
        api_version: ApiVersion(1),
        deprecated: false,
        response_content_type: None,
        has_file_upload: false,
    }
}

/// Build a `ServiceSchema` for `wf-svc` with two GET operations used by the
/// two-step workflow under test.
fn make_service_schema() -> ServiceSchema {
    let op_a = simple_get_operation("wf-svc/public/step-a/v1/run", "/wf-svc/step-a");
    let op_b = simple_get_operation("wf-svc/public/step-b/v1/run", "/wf-svc/step-b");
    ServiceSchema {
        name: "wf-svc".into(),
        description: String::new(),
        resources: vec![ResourceSchema {
            name: "steps".into(),
            description: String::new(),
            methods: vec![
                MethodSchema {
                    name: "run-step-a".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![op_a],
                    }],
                },
                MethodSchema {
                    name: "run-step-b".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![op_b],
                    }],
                },
            ],
        }],
    }
}

// ── Two-step compiled workflow ──────────────────────────────────────────────

/// Build a two-step `CompiledWorkflow` against `wf-svc`. Neither step
/// requires path parameters or workflow inputs, so no gather call is made.
fn make_two_step_workflow() -> CompiledWorkflow {
    let svc = ServiceId::new("wf-svc");

    let step_a = CompiledStep {
        id: "step-a".into(),
        index: 0,
        description: Some("run step A".into()),
        operation: OperationReference {
            service: svc.clone(),
            operation: OperationId::new("wf-svc/public/step-a/v1/run"),
        },
        dependencies: vec![],
        confirm: false,
        is_optional: false,
        continue_on_failure: false,
        skip_if_exists: false,
        is_reviewed: None,
        inputs: vec![],
        outputs: vec![],
        auto_derived: vec![],
    };

    let step_b = CompiledStep {
        id: "step-b".into(),
        index: 1,
        description: Some("run step B".into()),
        operation: OperationReference {
            service: svc,
            operation: OperationId::new("wf-svc/public/step-b/v1/run"),
        },
        dependencies: vec!["step-a".into()],
        confirm: false,
        is_optional: false,
        continue_on_failure: false,
        skip_if_exists: false,
        is_reviewed: None,
        inputs: vec![],
        outputs: vec![],
        auto_derived: vec![],
    };

    CompiledWorkflow {
        id: WorkflowId::new("two-step-wf"),
        name: "two step workflow".into(),
        intent: None,
        description: None,
        briefing: None,
        inputs: vec![WorkflowInputSpec {
            name: "unused-input".into(),
            description: None,
            schema: None,
            required: false,
            default: Some(serde_json::json!("default-value")),
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }],
        is_reviewed_by_default: true,
        steps: vec![step_a, step_b],
        // Non-empty outputs forces `CommandOutput::Workflow` envelope.
        outputs: vec![WorkflowOutputAlias {
            name: "summary".into(),
            from_step_id: "step-a".into(),
            output: "nonexistent-capture".into(),
            sensitive: false,
            section: None,
            label: None,
            item_fields: None,
        }],
        completion: None,
    }
}

// ── Recording CLI frontend ──────────────────────────────────────────────────

/// CLI `Frontend` implementation that records every event pushed into it.
/// All render methods are no-ops.
#[derive(Default)]
struct RecordingFrontend {
    /// Every `FrontendEvent` pushed via `on_event`, in arrival order.
    events: Vec<FrontendEvent>,
}

impl Frontend for RecordingFrontend {
    fn on_event(&mut self, event: &FrontendEvent) {
        self.events.push(event.clone());
    }

    fn render(&mut self, _output: &CommandOutput) -> Result<(), CliError> {
        Ok(())
    }

    fn render_error(&mut self, _err: &CliError) {}

    fn render_warning(&mut self, _msg: &str, _reason: Option<&str>, _tip: Option<&str>) {}

    fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {}

    fn finish(self: Box<Self>) -> Result<(), CliError> {
        Ok(())
    }
}

/// CLI `ExecutionInteraction` double: gather returns an empty map (this
/// workflow's inputs all have defaults); `confirm_step` always proceeds.
#[derive(Default)]
struct RecordingInteraction;

impl ags::frontend::ExecutionInteraction for RecordingInteraction {
    fn gather_workflow_inputs(
        &mut self,
        _needed: &[WorkflowInputNeeded],
        _step_context: &CompiledStep,
        _supplied: &[ags_protocol::workflow::SuppliedInputView],
    ) -> Result<ags_protocol::workflow::GatherResult, CliError> {
        Ok(ags_protocol::workflow::GatherResult::default())
    }

    fn confirm_step(
        &mut self,
        _step: &CompiledStep,
        _preview: &StepPreview,
    ) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
        Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
    }
}

// ── Integration test ────────────────────────────────────────────────────────

/// Build a `Runtime` with a queued HTTP client and the test service schema
/// pre-loaded into the in-memory catalogue.
fn make_runtime() -> Runtime {
    make_runtime_with(vec![
        ok_json(r#"{"result": "A"}"#),
        ok_json(r#"{"result": "B"}"#),
    ])
}

/// Build a `Runtime` whose HTTP client drains `responses` in order.
fn make_runtime_with(responses: Vec<Result<HttpResponse, RuntimeError>>) -> Runtime {
    let mut runtime = Runtime::new(
        ExecutionContext {
            base_url: "https://example.test".into(),
            ..Default::default()
        },
        Box::new(QueuedClient::new(responses)),
        reqwest::Client::new(),
    );
    runtime
        .catalogue_mut()
        .insert_for_tests("wf-svc", make_service_schema());
    runtime
}

/// A one-step version of `make_two_step_workflow` (dispatches `step-a` only).
fn make_one_step_workflow() -> CompiledWorkflow {
    let mut wf = make_two_step_workflow();
    wf.id = WorkflowId::new("one-step-wf");
    wf.steps.truncate(1); // keep step-a only
    wf
}

/// End-to-end test: the `ExecutionFrontendAdapter` correctly bridges two
/// dispatched steps through the CLI `Frontend` trait.
///
/// Asserts:
/// 1. The recording frontend received a banner-bearing `RunStarted` event.
/// 2. Each step emitted both `StepStarted` and `StepFinished{Success}` events.
/// 3. The adapter emits no finish event of its own — the run-finish lifecycle
///    event is owned by the shared lifecycle helper, not the adapter.
/// 4. The executor's final output is `CommandOutput::Workflow { .. }`.
#[tokio::test]
async fn test_workflow_frontend_adapter_drives_two_step_workflow() {
    let compiled = make_two_step_workflow();
    let mut recording = RecordingFrontend::default();
    let mut interaction = RecordingInteraction;

    {
        let mut adapter = ExecutionFrontendAdapter::new(&mut recording, &mut interaction);
        let options = RunOptions::default();
        let mut runtime = make_runtime();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut adapter, &mut run_context)
                .await
                .expect("executor must not return Err");

        // Terminal outcome.
        assert_eq!(
            outcome,
            RunOutcome::Success,
            "expected RunOutcome::Success, got {outcome:?}"
        );
        assert!(
            pending.is_none(),
            "expected no pending error, got {pending:?}"
        );

        // Assert 4: final envelope is CommandOutput::Workflow.
        match final_output {
            Some(CommandOutput::Workflow {
                ref workflow_id,
                ref step_summaries,
                ..
            }) => {
                assert_eq!(workflow_id.as_str(), "two-step-wf", "workflow_id mismatch");
                assert_eq!(
                    step_summaries.len(),
                    2,
                    "expected 2 step summaries, got {step_summaries:?}"
                );
            }
            other => panic!("expected Some(Workflow {{ .. }}), got {other:?}"),
        }
    }

    // The adapter has been dropped, releasing the borrow on `recording`.
    let events = &recording.events;

    // Assert 1: banner-bearing RunStarted received.
    let wf_started_pos = events
        .iter()
        .position(|e| {
            matches!(
                e,
                FrontendEvent::RunStarted {
                    workflow_banner: Some(_)
                }
            )
        })
        .expect("banner-bearing RunStarted event must be present");

    // Assert 2a: step-a started and finished with Success.
    let step_a_started_pos = events
        .iter()
        .position(|e| matches!(e, FrontendEvent::StepStarted { id, .. } if id == "step-a"))
        .expect("StepStarted{step-a} must be present");

    let step_a_finished_pos = events
        .iter()
        .position(|e| {
            matches!(
                e,
                FrontendEvent::StepFinished {
                    summary,
                    outcome: CliFrontendStepOutcome::Success,
                    ..
                } if summary == "step-a ok"
            )
        })
        .expect("StepFinished{step-a, Success} must be present");

    // Assert 2b: step-b started and finished with Success.
    let step_b_started_pos = events
        .iter()
        .position(|e| matches!(e, FrontendEvent::StepStarted { id, .. } if id == "step-b"))
        .expect("StepStarted{step-b} must be present");

    let step_b_finished_pos = events
        .iter()
        .position(|e| {
            matches!(
                e,
                FrontendEvent::StepFinished {
                    summary,
                    outcome: CliFrontendStepOutcome::Success,
                    ..
                } if summary == "step-b ok"
            )
        })
        .expect("StepFinished{step-b, Success} must be present");

    // Assert 3: the adapter emits no finish event of its own.
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, FrontendEvent::RunFinished { .. })),
        "adapter must not emit a RunFinished event; got {events:?}"
    );

    // Validate ordering.
    assert!(
        wf_started_pos < step_a_started_pos,
        "RunStarted must precede StepStarted{{step-a}}"
    );
    assert!(
        step_a_started_pos < step_a_finished_pos,
        "StepStarted{{step-a}} must precede StepFinished{{step-a}}"
    );
    assert!(
        step_a_finished_pos < step_b_started_pos,
        "StepFinished{{step-a}} must precede StepStarted{{step-b}}"
    );
    assert!(
        step_b_started_pos < step_b_finished_pos,
        "StepStarted{{step-b}} must precede StepFinished{{step-b}}"
    );
}

/// Run the supplied compiled workflow through the existing
/// RecordingInteraction + RecordingFrontend pair (RecordingInteraction
/// inherits the default `present_briefing` -> Ok(true)) and return the
/// captured FrontendEvent stream. Used by the default-interaction
/// regression to compare a with-briefing vs without-briefing run.
async fn run_through_default_interaction(compiled: &CompiledWorkflow) -> Vec<FrontendEvent> {
    let mut frontend = RecordingFrontend::default();
    let mut interaction = RecordingInteraction;
    {
        let mut adapter = ExecutionFrontendAdapter::new(&mut frontend, &mut interaction);
        let options = RunOptions::default();
        let mut runtime = make_runtime();
        let mut run_context = RunContext::new(&mut runtime, &options);
        let _ = Executor::execute(compiled, BTreeMap::new(), &mut adapter, &mut run_context)
            .await
            .expect("executor must not return Err");
    }
    frontend.events.clone()
}

#[tokio::test]
async fn test_default_execution_interaction_ignores_briefing() {
    // Two runs of the same two-step workflow: one with a briefing
    // attached, one without. The default `present_briefing` impl on the
    // file's unit-struct `RecordingInteraction` returns Ok(true) without
    // emitting any FrontendEvent, so the event streams must be identical.
    let without_briefing = make_two_step_workflow();

    let mut with_briefing = make_two_step_workflow();
    with_briefing.briefing = Some(ags_protocol::workflow::WorkflowBriefing {
        overview: "**Bold** opener.\n\nSecond paragraph.".into(),
        prerequisites: vec!["A namespace.".into()],
        creates: vec!["A stat.".into(), "A ruleset.".into()],
    });

    // Each run consumes a fresh queued HTTP client (make_runtime builds
    // one with two queued ok responses, one per step). They must NOT
    // share a runtime because the queue would be drained.
    let events_without = run_through_default_interaction(&without_briefing).await;
    let events_with = run_through_default_interaction(&with_briefing).await;

    // FrontendEvent does not implement PartialEq, so compare via Debug
    // formatting — semantically equivalent for the regression and stable
    // across runs because every captured event derives Debug.
    let fmt_without: Vec<String> = events_without.iter().map(|e| format!("{e:?}")).collect();
    let fmt_with: Vec<String> = events_with.iter().map(|e| format!("{e:?}")).collect();
    assert_eq!(
        fmt_with, fmt_without,
        "default ExecutionInteraction must observe identical FrontendEvent streams whether or not the workflow has a briefing"
    );
}

// ── Briefing sequence shims & tests ─────────────────────────────────────────

/// Recording shim alongside the file's existing unit-struct
/// `RecordingInteraction`. Records every interaction call in order and
/// steers the briefing reply.
struct BriefingRecordingInteraction {
    recorder: Arc<Mutex<Vec<String>>>,
    briefing_reply: Option<Result<bool, CliError>>, // None → default Ok(true)
}

impl ags::frontend::ExecutionInteraction for BriefingRecordingInteraction {
    fn present_briefing(
        &mut self,
        _briefing: &ags_protocol::workflow::WorkflowBriefing,
        _workflow_name: &str,
    ) -> Result<bool, CliError> {
        self.recorder
            .lock()
            .unwrap()
            .push("present_briefing".into());
        match self.briefing_reply.take() {
            Some(r) => r,
            None => Ok(true),
        }
    }
    fn gather_workflow_inputs(
        &mut self,
        _needed: &[WorkflowInputNeeded],
        _step_context: &CompiledStep,
        _supplied: &[ags_protocol::workflow::SuppliedInputView],
    ) -> Result<ags_protocol::workflow::GatherResult, CliError> {
        self.recorder
            .lock()
            .unwrap()
            .push("gather_workflow_inputs".into());
        Ok(ags_protocol::workflow::GatherResult::default())
    }
    fn confirm_step(
        &mut self,
        _step: &CompiledStep,
        _preview: &StepPreview,
    ) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
        self.recorder.lock().unwrap().push("confirm_step".into());
        Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
    }
}

#[tokio::test]
async fn test_fullscreen_briefing_runs_before_first_step() {
    let mut compiled = make_two_step_workflow();
    compiled.briefing = Some(ags_protocol::workflow::WorkflowBriefing {
        overview: "x".into(),
        prerequisites: vec![],
        creates: vec![],
    });

    let recorder = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut frontend = RecordingFrontend::default();
    let mut interaction = BriefingRecordingInteraction {
        recorder: Arc::clone(&recorder),
        briefing_reply: None, // default Ok(true)
    };

    let (outcome, _output, pending) = {
        let mut adapter = ExecutionFrontendAdapter::new(&mut frontend, &mut interaction);
        let options = RunOptions::default();
        let mut runtime = make_runtime();
        let mut run_context = RunContext::new(&mut runtime, &options);
        Executor::execute(&compiled, BTreeMap::new(), &mut adapter, &mut run_context)
            .await
            .expect("executor must not return Err")
    };

    assert_eq!(outcome, RunOutcome::Success);
    assert!(pending.is_none());

    let seq = recorder.lock().unwrap().clone();
    let briefing_idx = seq
        .iter()
        .position(|s| s == "present_briefing")
        .expect("present_briefing must be observed");
    let first_step_idx = seq
        .iter()
        .position(|s| s == "confirm_step")
        .or_else(|| seq.iter().position(|s| s == "gather_workflow_inputs"))
        .unwrap_or(seq.len());
    assert!(
        briefing_idx < first_step_idx,
        "briefing must run before first per-step interaction; sequence: {seq:?}"
    );
    assert_eq!(
        seq.iter()
            .filter(|s| s.as_str() == "present_briefing")
            .count(),
        1,
        "present_briefing must be observed exactly once; sequence: {seq:?}"
    );
}

#[tokio::test]
async fn test_fullscreen_briefing_cancel_aborts_run() {
    let mut compiled = make_two_step_workflow();
    compiled.briefing = Some(ags_protocol::workflow::WorkflowBriefing {
        overview: "x".into(),
        prerequisites: vec![],
        creates: vec![],
    });

    let recorder = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut frontend = RecordingFrontend::default();
    let mut interaction = BriefingRecordingInteraction {
        recorder: Arc::clone(&recorder),
        briefing_reply: Some(Ok(false)), // user cancels at the briefing
    };

    let (outcome, _output, pending) = {
        let mut adapter = ExecutionFrontendAdapter::new(&mut frontend, &mut interaction);
        let options = RunOptions::default();
        let mut runtime = make_runtime();
        let mut run_context = RunContext::new(&mut runtime, &options);
        Executor::execute(&compiled, BTreeMap::new(), &mut adapter, &mut run_context)
            .await
            .expect("executor must not return Err")
    };

    assert_eq!(outcome, RunOutcome::Cancelled);
    assert!(pending.is_none());

    let seq = recorder.lock().unwrap().clone();
    assert!(
        seq.contains(&"present_briefing".to_string()),
        "present_briefing must be observed; sequence: {seq:?}"
    );
    assert!(
        !seq.contains(&"confirm_step".to_string()),
        "confirm_step must NOT be observed when briefing cancels; sequence: {seq:?}"
    );
    assert!(
        !seq.contains(&"gather_workflow_inputs".to_string()),
        "gather_workflow_inputs must NOT be observed when briefing cancels; sequence: {seq:?}"
    );
}

// ── Failure-recovery tests (Task 6) ─────────────────────────────────────────

use ags_protocol::workflow::StepFailureAction;
// Note: `FrontendEvent` and `StepOutcome as CliFrontendStepOutcome` are already
// imported at the top of this file; only `StepFailureAction` is new.

/// `ExecutionInteraction` double that scripts `resolve_step_failure` and counts calls.
#[derive(Default)]
struct FailureInteraction {
    action: Option<StepFailureAction>,
    resolve_calls: usize,
}
impl ags::frontend::ExecutionInteraction for FailureInteraction {
    fn gather_workflow_inputs(
        &mut self,
        _n: &[WorkflowInputNeeded],
        _s: &CompiledStep,
        _v: &[ags_protocol::workflow::SuppliedInputView],
    ) -> Result<ags_protocol::workflow::GatherResult, CliError> {
        Ok(Default::default())
    }
    fn confirm_step(
        &mut self,
        _s: &CompiledStep,
        _p: &StepPreview,
    ) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
        Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
    }
    fn resolve_step_failure(
        &mut self,
        _s: &CompiledStep,
        _e: &ags_protocol::error::RuntimeError,
        _allow_skip: bool,
    ) -> Result<StepFailureAction, CliError> {
        self.resolve_calls += 1;
        Ok(self.action.unwrap_or(StepFailureAction::Cancel))
    }
}

const CONFLICT_BODY: &str = r#"{"errorCode": 30171, "errorMessage": "already exists"}"#;

#[tokio::test]
async fn test_failure_gate_retry_redispatches_then_succeeds() {
    let compiled = make_one_step_workflow();
    let mut recording = RecordingFrontend::default();
    let mut interaction = FailureInteraction {
        action: Some(StepFailureAction::Retry),
        resolve_calls: 0,
    };
    let outcome = {
        let mut adapter = ExecutionFrontendAdapter::new(&mut recording, &mut interaction);
        let options = RunOptions::default();
        // 1st dispatch 409 → gate Retry → 2nd dispatch 200.
        let mut runtime = make_runtime_with(vec![
            status_json(409, CONFLICT_BODY),
            ok_json(r#"{"result":"A"}"#),
        ]);
        let mut run_context = RunContext::new(&mut runtime, &options);
        let (outcome, _out, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut adapter, &mut run_context)
                .await
                .unwrap();
        assert!(
            pending.is_none(),
            "retry that succeeds leaves no pending error"
        );
        outcome
    };
    assert_eq!(outcome, RunOutcome::Success);
    assert_eq!(
        interaction.resolve_calls, 1,
        "gate consulted once for the single 409"
    );
}

#[tokio::test]
async fn test_failure_gate_cancel_fails_run_preserving_error() {
    let compiled = make_one_step_workflow();
    let mut recording = RecordingFrontend::default();
    let mut interaction = FailureInteraction {
        action: Some(StepFailureAction::Cancel),
        resolve_calls: 0,
    };
    let (outcome, had_pending) = {
        let mut adapter = ExecutionFrontendAdapter::new(&mut recording, &mut interaction);
        let options = RunOptions::default();
        let mut runtime = make_runtime_with(vec![status_json(409, CONFLICT_BODY)]);
        let mut run_context = RunContext::new(&mut runtime, &options);
        let (outcome, _out, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut adapter, &mut run_context)
                .await
                .unwrap();
        (outcome, pending.is_some())
    };
    assert_eq!(outcome, RunOutcome::Failed);
    assert!(had_pending, "cancel preserves the pending error");
    assert_eq!(interaction.resolve_calls, 1);
}

#[tokio::test]
async fn test_failure_no_input_is_fatal_without_gate() {
    let compiled = make_one_step_workflow();
    let mut recording = RecordingFrontend::default();
    let mut interaction = FailureInteraction {
        action: Some(StepFailureAction::Retry),
        resolve_calls: 0,
    };
    let outcome = {
        let mut adapter = ExecutionFrontendAdapter::new(&mut recording, &mut interaction);
        let options = RunOptions {
            no_input: true,
            ..RunOptions::default()
        };
        let mut runtime = make_runtime_with(vec![status_json(409, CONFLICT_BODY)]);
        let mut run_context = RunContext::new(&mut runtime, &options);
        let (outcome, _out, _pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut adapter, &mut run_context)
                .await
                .unwrap();
        outcome
    };
    assert_eq!(outcome, RunOutcome::Failed);
    assert_eq!(
        interaction.resolve_calls, 0,
        "no gate is consulted in no_input mode"
    );
}

#[tokio::test]
async fn test_output_binding_failure_routes_through_gate() {
    // A step that dispatches 200 but whose capture path is absent (schema drift
    // or an empty body) fails to bind its output. That failure must pause at the
    // same gate as an HTTP-layer failure — not hard-fail without offering
    // Retry/Cancel. The step is not safely skippable (the capture has no
    // default), so the gate offers Retry/Cancel only; Cancel fails the run.
    let mut compiled = make_one_step_workflow();
    compiled.steps[0].outputs = vec![ags_protocol::workflow::StepOutputCapture {
        name: "id".into(),
        source: ags_protocol::workflow::CaptureSource::ResponseBody {
            path: "$.id".into(),
        },
        default: None,
        sensitive: false,
    }];
    let mut recording = RecordingFrontend::default();
    let mut interaction = FailureInteraction {
        action: Some(StepFailureAction::Cancel),
        resolve_calls: 0,
    };
    let (outcome, had_pending) = {
        let mut adapter = ExecutionFrontendAdapter::new(&mut recording, &mut interaction);
        let options = RunOptions::default();
        // 200, but the body has no `id`, so `$.id` does not resolve → bind fails.
        let mut runtime = make_runtime_with(vec![ok_json(r#"{"result":"A"}"#)]);
        let mut run_context = RunContext::new(&mut runtime, &options);
        let (outcome, _out, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut adapter, &mut run_context)
                .await
                .unwrap();
        (outcome, pending.is_some())
    };
    assert_eq!(outcome, RunOutcome::Failed);
    assert!(
        had_pending,
        "the bind error is preserved as the pending error"
    );
    assert_eq!(
        interaction.resolve_calls, 1,
        "a 200 that cannot bind must reach the failure gate, like an HTTP failure"
    );
}

#[tokio::test]
async fn test_skip_if_exists_409_auto_skips_and_continues() {
    let mut compiled = make_one_step_workflow();
    compiled.steps[0].skip_if_exists = true; // safe: the step captures nothing
    let mut recording = RecordingFrontend::default();
    let mut interaction = FailureInteraction::default();
    let outcome = {
        let mut adapter = ExecutionFrontendAdapter::new(&mut recording, &mut interaction);
        let options = RunOptions::default();
        let mut runtime = make_runtime_with(vec![status_json(409, CONFLICT_BODY)]);
        let mut run_context = RunContext::new(&mut runtime, &options);
        let (outcome, _out, _pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut adapter, &mut run_context)
                .await
                .unwrap();
        outcome
    };
    assert_eq!(outcome, RunOutcome::Success, "auto-skip continues the run");
    assert_eq!(
        interaction.resolve_calls, 0,
        "auto-skip never consults the gate"
    );
    let finished: Vec<_> = recording
        .events
        .iter()
        .filter(|e| matches!(e, FrontendEvent::StepFinished { .. }))
        .collect();
    assert_eq!(finished.len(), 1, "exactly one StepFinished (no duplicate)");
    assert!(
        matches!(
            finished[0],
            FrontendEvent::StepFinished {
                outcome: CliFrontendStepOutcome::Skipped,
                ..
            }
        ),
        "auto-skipped step emits StepFinished{{Skipped}}"
    );
}

#[tokio::test]
async fn test_failure_gate_retry_twice_then_succeeds() {
    // 409 → Retry → 409 → Retry → 200: the gate is consulted once per failure
    // (twice), proving the loop re-enters the gate across consecutive failures.
    let compiled = make_one_step_workflow();
    let mut recording = RecordingFrontend::default();
    let mut interaction = FailureInteraction {
        action: Some(StepFailureAction::Retry),
        resolve_calls: 0,
    };
    let outcome = {
        let mut adapter = ExecutionFrontendAdapter::new(&mut recording, &mut interaction);
        let options = RunOptions::default();
        let mut runtime = make_runtime_with(vec![
            status_json(409, CONFLICT_BODY),
            status_json(409, CONFLICT_BODY),
            ok_json(r#"{"result":"A"}"#),
        ]);
        let mut run_context = RunContext::new(&mut runtime, &options);
        let (outcome, _out, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut adapter, &mut run_context)
                .await
                .unwrap();
        assert!(pending.is_none());
        outcome
    };
    assert_eq!(outcome, RunOutcome::Success);
    assert_eq!(interaction.resolve_calls, 2, "gate consulted once per 409");
}
