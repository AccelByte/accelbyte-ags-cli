//! Test helpers used by every executor and helper unit test.
//! Production code never depends on this module.

use std::collections::BTreeMap;

use super::*;
use ags_protocol::workflow::{CompiledStep, GatherSlotId, StepPreview, WorkflowInputNeeded};

/// Scripted frontend that records every event, returns canned gather
/// responses, and returns canned confirm responses. The default for missing
/// scripted entries: empty map for gather, `true` for confirm.
#[derive(Debug, Default)]
#[allow(dead_code)]
pub(crate) struct MockFrontend {
    /// Every event the executor pushed, in order.
    pub events: Vec<WorkflowEvent>,
    /// Pre-scripted gather responses; consumed in FIFO order.
    pub gather_responses: Vec<BTreeMap<GatherSlotId, serde_json::Value>>,
    /// Pre-scripted confirm responses; consumed in FIFO order.
    pub confirm_responses: Vec<bool>,
    /// Number of `gather_workflow_inputs` calls made.
    pub gather_call_count: usize,
    /// Number of `confirm_step` calls made.
    pub confirm_call_count: usize,
    /// Optional injected gather error; takes precedence over the script.
    pub gather_error: Option<RuntimeError>,
    /// Optional injected confirm error; takes precedence over the script.
    pub confirm_error: Option<RuntimeError>,
    /// Overrides applied to already-supplied inputs on every gather call.
    /// Defaults to empty (preserves existing test behaviour).
    pub input_overrides: BTreeMap<String, serde_json::Value>,
    /// Number of `review_step` calls made.
    pub review_step_call_count: usize,
    /// Step ids (plan.step_label) for which `review_step` was invoked, in order.
    pub reviewed_step_ids: Vec<String>,
    /// Step ids for which `confirm_step` returns `Skip`. Checked before the
    /// legacy `confirm_responses` scripting, so existing tests are unaffected.
    pub skip_confirm_step_ids: Vec<String>,
    /// Step ids for which `review_step` returns `Skip` (matched on `plan.step_label`).
    pub skip_review_step_ids: Vec<String>,
    /// Run mode returned by `collect_workflow_inputs`. Defaults to
    /// `ReviewInputSteps` (preserves existing test behaviour).
    pub mock_collect_run_mode: ags_protocol::workflow::RunMode,
}

#[allow(dead_code)]
impl MockFrontend {
    /// Build an empty mock with no scripted responses.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one scripted gather response.
    pub fn with_gather_response(
        mut self,
        response: BTreeMap<GatherSlotId, serde_json::Value>,
    ) -> Self {
        self.gather_responses.push(response);
        self
    }

    /// Append one scripted confirm response.
    pub fn with_confirm_response(mut self, response: bool) -> Self {
        self.confirm_responses.push(response);
        self
    }

    /// Inject a one-shot gather error for the next gather call.
    pub fn with_gather_error(mut self, error: RuntimeError) -> Self {
        self.gather_error = Some(error);
        self
    }

    /// Inject a one-shot confirm error for the next confirm call.
    pub fn with_confirm_error(mut self, error: RuntimeError) -> Self {
        self.confirm_error = Some(error);
        self
    }

    /// Set the `input_overrides` map returned on every gather call.
    /// Leaves existing test behaviour unchanged when not called (defaults empty).
    pub fn with_input_overrides(mut self, overrides: BTreeMap<String, serde_json::Value>) -> Self {
        self.input_overrides = overrides;
        self
    }
}

impl WorkflowFrontend for MockFrontend {
    fn on_event(&mut self, event: &WorkflowEvent) {
        self.events.push(event.clone());
    }

    fn gather_workflow_inputs(
        &mut self,
        _needed: &[WorkflowInputNeeded],
        _step_context: &CompiledStep,
        _supplied: &[ags_protocol::workflow::SuppliedInputView],
    ) -> Result<ags_protocol::workflow::GatherResult, RuntimeError> {
        self.gather_call_count += 1;
        if let Some(err) = self.gather_error.take() {
            return Err(err);
        }
        let slot_values = self
            .gather_responses
            .get(self.gather_call_count - 1)
            .cloned()
            .unwrap_or_default();
        Ok(ags_protocol::workflow::GatherResult {
            slot_values,
            input_overrides: self.input_overrides.clone(),
        })
    }

    fn confirm_step(
        &mut self,
        step: &CompiledStep,
        _preview: &StepPreview,
    ) -> Result<ags_protocol::workflow::StepConfirmOutcome, RuntimeError> {
        use ags_protocol::workflow::StepConfirmOutcome;
        self.confirm_call_count += 1;
        if let Some(err) = self.confirm_error.take() {
            return Err(err);
        }
        if self.skip_confirm_step_ids.iter().any(|id| id == &step.id) {
            return Ok(StepConfirmOutcome::Skip);
        }
        // Preserve the legacy true/false scripting → Proceed/Cancel.
        let proceed = *self
            .confirm_responses
            .get(self.confirm_call_count - 1)
            .unwrap_or(&true);
        Ok(if proceed {
            StepConfirmOutcome::Proceed
        } else {
            StepConfirmOutcome::Cancel
        })
    }

    fn review_step(
        &mut self,
        plan: &ags_protocol::workflow::StepFieldPlan,
    ) -> Result<ags_protocol::workflow::StepReviewOutcome, RuntimeError> {
        use ags_protocol::workflow::{StepFieldEdits, StepReviewOutcome};
        self.review_step_call_count += 1;
        self.reviewed_step_ids.push(plan.step_label.clone());
        // `review_step` gets only the plan; `plan.step_label` is the step id
        // (set from `step.id` in resolve.rs), so match on that.
        if self
            .skip_review_step_ids
            .iter()
            .any(|id| id == &plan.step_label)
        {
            return Ok(StepReviewOutcome::Skip);
        }
        Ok(StepReviewOutcome::Proceed(StepFieldEdits::default()))
    }

    fn collect_workflow_inputs(
        &mut self,
        _specs: &[ags_protocol::workflow::WorkflowInputSpec],
        current: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Option<ags_protocol::workflow::CollectOutcome>, RuntimeError> {
        Ok(Some(ags_protocol::workflow::CollectOutcome {
            inputs: current.clone(),
            run_mode: self.mock_collect_run_mode,
        }))
    }
}

#[allow(dead_code)]
impl MockFrontend {
    /// Every StepFinished as (id, outcome, summary), in emit order.
    fn finished_steps(&self) -> Vec<(String, StepOutcome, String)> {
        self.events
            .iter()
            .filter_map(|e| match e {
                WorkflowEvent::StepFinished {
                    id,
                    outcome,
                    summary,
                    ..
                } => Some((id.clone(), *outcome, summary.clone())),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod executor_skeleton {
    use super::*;
    use crate::runtime::workflows::executor::{Executor, RunContext};
    use ags_protocol::workflow::{CompiledWorkflow, WorkflowId};

    fn empty_compiled() -> CompiledWorkflow {
        CompiledWorkflow {
            id: WorkflowId::new("smoke"),
            name: "smoke".into(),
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

    #[tokio::test]
    async fn test_skeleton_emits_started_and_finished() {
        let compiled = empty_compiled();
        let mut frontend = MockFrontend::new();
        let options = RunOptions::default();
        let mut runtime = test_runtime();
        let mut run_context = RunContext::new(&mut runtime, &options);
        let (outcome, final_output, pending) = Executor::execute(
            &compiled,
            std::collections::BTreeMap::new(),
            &mut frontend,
            &mut run_context,
        )
        .await
        .unwrap();
        assert_eq!(outcome, RunOutcome::Success);
        assert!(final_output.is_none());
        assert!(pending.is_none());
        assert_eq!(frontend.events.len(), 2);
        assert!(matches!(
            frontend.events[0],
            WorkflowEvent::WorkflowStarted { .. }
        ));
        assert!(matches!(
            frontend.events[1],
            WorkflowEvent::WorkflowFinished {
                outcome: RunOutcome::Success,
                ..
            }
        ));
    }

    /// Build a Runtime usable in tests. Uses the same constructor pattern as
    /// `Runtime::constructor_tests::test_runtime_new_accepts_dyn_http_client`.
    pub(super) fn test_runtime() -> crate::runtime::Runtime {
        use crate::runtime::dispatch::http::{HttpClient, HttpRequest, HttpResponse};

        struct NeverClient;
        #[async_trait::async_trait]
        impl HttpClient for NeverClient {
            async fn send(&self, _: HttpRequest) -> Result<HttpResponse, RuntimeError> {
                unreachable!("skeleton test does not dispatch")
            }
        }

        crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext::default(),
            Box::new(NeverClient),
            reqwest::Client::new(),
        )
    }
}

#[cfg(test)]
mod executor_happy_path {
    use std::collections::BTreeMap;

    use async_trait::async_trait;

    use super::*;
    use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::workflows::executor::{Executor, RunContext};
    use ags_protocol::catalogue::{
        ApiVersion, HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema,
        ParameterLocation, ParameterSchema, ResourceSchema, ScopeEntry, ServiceId, ServiceSchema,
        ValueType,
    };
    use ags_protocol::output::CommandOutput;
    use ags_protocol::workflow::{
        AutoDeriveScope, AutoDerivedField, CompiledStep, CompiledWorkflow, OperationReference,
        StepFieldLocation, WorkflowId, WorkflowInputSpec,
    };

    /// A scripted HTTP client that always returns the given body with status 200.
    struct ScriptedClient {
        body: String,
    }

    #[async_trait]
    impl HttpClient for ScriptedClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            Ok(HttpResponse {
                status: 200,
                body: HttpBody::Text(self.body.clone()),
            })
        }
    }

    /// Build a minimal `ServiceSchema` for a GET /items operation with no
    /// required parameters (simplest possible dispatch path).
    fn make_test_service_schema() -> ServiceSchema {
        let operation = OperationSchema {
            id: OperationId::new("GetItems"),
            name: "list".into(),
            summary: "List items".into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/items".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        ServiceSchema {
            name: "test-svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "list".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![operation],
                    }],
                }],
            }],
        }
    }

    /// Build a 1-step compiled workflow referencing the test service schema.
    pub(super) fn make_one_step_workflow() -> CompiledWorkflow {
        CompiledWorkflow {
            id: WorkflowId::new("test-wf"),
            name: "test workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![CompiledStep {
                id: "get-items".into(),
                index: 0,
                description: None,
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new("test-svc"),
                    operation: OperationId::new("GetItems"),
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
            }],
            outputs: vec![],
            completion: None,
        }
    }

    /// Build a Runtime with a scripted HTTP client and the test service schema
    /// pre-loaded into the catalogue memory cache.
    fn make_runtime(body: &str) -> crate::runtime::Runtime {
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(ScriptedClient {
                body: body.to_string(),
            }),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("test-svc", make_test_service_schema());
        runtime
    }

    /// Happy-path: 1-step workflow dispatches successfully and emits the
    /// expected event sequence with a `Service` envelope as final output.
    #[tokio::test]
    async fn test_one_step_happy_path() {
        let compiled = make_one_step_workflow();
        let mut frontend = MockFrontend::new();
        let options = RunOptions::default();
        let mut runtime = make_runtime(r#"{"id": 42}"#);
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);
        assert!(pending.is_none());
        assert!(
            matches!(final_output, Some(CommandOutput::Service(_))),
            "expected Some(Service(_)), got {final_output:?}"
        );

        // Event order: WorkflowStarted, StepStarted, Progress(Started),
        // Progress(Finished), StepFinished{Success}, WorkflowFinished{Success}.
        // We assert the structural subset the spec requires.
        let workflow_started_pos = frontend
            .events
            .iter()
            .position(|e| matches!(e, WorkflowEvent::WorkflowStarted { .. }))
            .expect("WorkflowStarted missing");
        let step_started_pos = frontend
            .events
            .iter()
            .position(|e| matches!(e, WorkflowEvent::StepStarted { .. }))
            .expect("StepStarted missing");
        let step_finished_pos = frontend
            .events
            .iter()
            .position(|e| {
                matches!(
                    e,
                    WorkflowEvent::StepFinished {
                        outcome: StepOutcome::Success,
                        ..
                    }
                )
            })
            .expect("StepFinished{Success} missing");
        let workflow_finished_pos = frontend
            .events
            .iter()
            .position(|e| {
                matches!(
                    e,
                    WorkflowEvent::WorkflowFinished {
                        outcome: RunOutcome::Success,
                        ..
                    }
                )
            })
            .expect("WorkflowFinished{Success} missing");

        assert!(
            workflow_started_pos < step_started_pos,
            "WorkflowStarted must precede StepStarted"
        );
        assert!(
            step_started_pos < step_finished_pos,
            "StepStarted must precede StepFinished"
        );
        assert!(
            step_finished_pos < workflow_finished_pos,
            "StepFinished must precede WorkflowFinished"
        );
    }

    /// A scripted HTTP client that returns a binary response body.
    struct BinaryScriptedClient;

    #[async_trait]
    impl HttpClient for BinaryScriptedClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            Ok(HttpResponse {
                status: 200,
                body: HttpBody::Binary {
                    content_type: "application/octet-stream".into(),
                    bytes: vec![1, 2, 3, 4],
                },
            })
        }
    }

    /// A binary dispatch result on the 1-step synth path must surface as
    /// `CommandOutput::BinaryWritten`, not a failed "unexpected envelope" step.
    #[tokio::test]
    async fn test_one_step_binary_response_yields_binary_written() {
        let compiled = make_one_step_workflow();
        let mut frontend = MockFrontend::new();
        let temp = tempfile::tempdir().expect("tempdir");
        let options = RunOptions {
            output: Some(ags_protocol::request::OutputDestination::File(
                temp.path().join("body.bin"),
            )),
            ..RunOptions::default()
        };
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(BinaryScriptedClient),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("test-svc", make_test_service_schema());
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);
        assert!(pending.is_none());
        assert!(
            matches!(final_output, Some(CommandOutput::BinaryWritten(_))),
            "expected Some(BinaryWritten(_)), got {final_output:?}"
        );
    }

    /// Pre-supplied inputs must prevent the gather callback from being called.
    /// Build a workflow whose step auto-derives a `namespace` workflow input,
    /// then pre-supply it; assert `gather_call_count == 0`.
    #[tokio::test]
    async fn test_pre_supplied_inputs_skip_gather() {
        // Add a path param `namespace` to the operation so auto-derive fires.
        let operation = OperationSchema {
            id: OperationId::new("GetItems"),
            name: "list".into(),
            summary: "List items".into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/namespaces/{namespace}/items".into(),
            parameters: vec![ParameterSchema {
                name: "namespace".into(),
                location: ParameterLocation::Path,
                required: true,
                value_type: ValueType::String,
                is_file: false,
                description: None,
                default: None,
            }],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        let service_schema = ServiceSchema {
            name: "test-svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "list".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![operation],
                    }],
                }],
            }],
        };

        let compiled = CompiledWorkflow {
            id: WorkflowId::new("test-wf"),
            name: "test workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![WorkflowInputSpec {
                name: "namespace".into(),
                description: None,
                schema: Some(serde_json::json!({"type": "string"})),
                required: true,
                default: None,
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
                file_picker: None,
            }],
            is_reviewed_by_default: true,
            steps: vec![CompiledStep {
                id: "get-items".into(),
                index: 0,
                description: None,
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new("test-svc"),
                    operation: OperationId::new("GetItems"),
                }),
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
                auto_derived: vec![AutoDerivedField {
                    field: "namespace".into(),
                    schema: serde_json::json!({"type": "string"}),
                    required: true,
                    sensitive: false,
                    description: None,
                    scope: AutoDeriveScope::WorkflowInput {
                        name: "namespace".into(),
                    },
                    location: StepFieldLocation::Body,
                }],
            }],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(ScriptedClient {
                body: r#"{"id": 1}"#.to_string(),
            }),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("test-svc", service_schema);

        let mut frontend = MockFrontend::new();
        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("dev"));

        let (outcome, _final_output, pending) =
            Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);
        assert!(pending.is_none());
        assert_eq!(
            frontend.gather_call_count, 0,
            "gather must not be called when inputs are pre-supplied"
        );
    }
}

#[cfg(test)]
mod executor_confirm {
    use std::collections::BTreeMap;

    use async_trait::async_trait;

    use super::*;
    use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::workflows::executor::{Executor, RunContext};
    use ags_protocol::catalogue::{
        ApiVersion, HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema,
        ResourceSchema, ScopeEntry, ServiceId, ServiceSchema,
    };
    use ags_protocol::workflow::{CompiledStep, CompiledWorkflow, OperationReference, WorkflowId};

    struct ScriptedClient {
        body: String,
    }

    #[async_trait]
    impl HttpClient for ScriptedClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            Ok(HttpResponse {
                status: 200,
                body: HttpBody::Text(self.body.clone()),
            })
        }
    }

    struct NeverClient;

    #[async_trait]
    impl HttpClient for NeverClient {
        async fn send(&self, _: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            unreachable!("dispatch must not be reached in this test")
        }
    }

    fn make_service_schema() -> ServiceSchema {
        let operation = OperationSchema {
            id: OperationId::new("GetItems"),
            name: "list".into(),
            summary: "List items".into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/items".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        ServiceSchema {
            name: "test-svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "list".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![operation],
                    }],
                }],
            }],
        }
    }

    /// Build a 1-step compiled workflow with the given `confirm` flag.
    fn make_workflow(confirm: bool) -> CompiledWorkflow {
        CompiledWorkflow {
            id: WorkflowId::new("test-wf"),
            name: "test workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![CompiledStep {
                id: "get-items".into(),
                index: 0,
                description: Some("Get all items".into()),
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new("test-svc"),
                    operation: OperationId::new("GetItems"),
                }),
                dependencies: vec![],
                confirm,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
                auto_derived: vec![],
            }],
            outputs: vec![],
            completion: None,
        }
    }

    fn make_runtime_scripted(body: &str) -> crate::runtime::Runtime {
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(ScriptedClient {
                body: body.to_string(),
            }),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("test-svc", make_service_schema());
        runtime
    }

    fn make_runtime_never() -> crate::runtime::Runtime {
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(NeverClient),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("test-svc", make_service_schema());
        runtime
    }

    /// `confirm: true` and mock returns `Ok(true)` — workflow succeeds and
    /// confirm was called exactly once.
    #[tokio::test]
    async fn test_confirm_true_proceeds_normally() {
        let compiled = make_workflow(true);
        let mut frontend = MockFrontend::new().with_confirm_response(true);
        let options = RunOptions::default();
        let mut runtime = make_runtime_scripted(r#"{"id": 1}"#);
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);
        assert!(pending.is_none());
        assert!(final_output.is_some(), "expected a final output");
        assert_eq!(
            frontend.confirm_call_count, 1,
            "confirm must be called once"
        );
    }

    /// `confirm: true` and mock returns `Ok(false)` — workflow is cancelled;
    /// no dispatch reached (NeverClient panics on send).
    #[tokio::test]
    async fn test_confirm_false_cancels_workflow() {
        let compiled = make_workflow(true);
        let mut frontend = MockFrontend::new().with_confirm_response(false);
        let options = RunOptions::default();
        let mut runtime = make_runtime_never();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Cancelled);
        assert!(final_output.is_none());
        assert!(pending.is_none());

        let step_finished_cancelled = frontend.events.iter().any(|e| {
            matches!(
                e,
                WorkflowEvent::StepFinished {
                    outcome: StepOutcome::Cancelled,
                    ..
                }
            )
        });
        assert!(
            step_finished_cancelled,
            "expected StepFinished{{Cancelled}}"
        );

        let workflow_finished_cancelled = frontend.events.iter().any(|e| {
            matches!(
                e,
                WorkflowEvent::WorkflowFinished {
                    outcome: RunOutcome::Cancelled,
                    ..
                }
            )
        });
        assert!(
            workflow_finished_cancelled,
            "expected WorkflowFinished{{Cancelled}}"
        );
    }

    /// `assume_yes: true` — confirm gate skipped entirely.
    #[tokio::test]
    async fn test_assume_yes_skips_confirm() {
        let compiled = make_workflow(true);
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            assume_yes: true,
            ..Default::default()
        };
        let mut runtime = make_runtime_scripted(r#"{"id": 2}"#);
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, _final_output, _pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);
        assert_eq!(
            frontend.confirm_call_count, 0,
            "confirm must not be called with --yes"
        );
    }

    /// `dry_run: true` — confirm gate skipped; confirm_call_count stays 0.
    /// Dispatch is attempted but may fail (no scripted response); we only
    /// assert the confirm bypass.
    #[tokio::test]
    async fn test_dry_run_skips_confirm() {
        let compiled = make_workflow(true);
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            dry_run: true,
            ..Default::default()
        };
        // Use scripted client so dispatch doesn't panic.
        let mut runtime = make_runtime_scripted(r#"{"id": 3}"#);
        let mut run_context = RunContext::new(&mut runtime, &options);

        let _result =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context).await;

        assert_eq!(
            frontend.confirm_call_count, 0,
            "confirm must not be called with --dry-run"
        );
    }

    /// Confirm callback returns an I/O error — step and workflow both fail.
    #[tokio::test]
    async fn test_confirm_io_error_fails_step() {
        let compiled = make_workflow(true);
        let confirm_err = RuntimeError::internal("terminal hung up");
        let mut frontend = MockFrontend::new().with_confirm_error(confirm_err);
        let options = RunOptions::default();
        let mut runtime = make_runtime_never();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Failed);
        assert!(final_output.is_none());
        assert!(pending.is_some(), "expected a pending error");

        let step_failed = frontend.events.iter().any(|e| {
            matches!(
                e,
                WorkflowEvent::StepFinished {
                    outcome: StepOutcome::Failed,
                    ..
                }
            )
        });
        assert!(step_failed, "expected StepFinished{{Failed}}");
    }

    /// The confirm gate breaking as I/O is a *failure* stage (`confirm`), never
    /// the cancellation stage (`at_confirm`) — one label must not mix the user
    /// abandoning the gate with the gate itself breaking.
    #[tokio::test]
    async fn test_confirm_io_error_reports_the_confirm_failure_stage() {
        let compiled = make_workflow(true);
        let mut frontend =
            MockFrontend::new().with_confirm_error(RuntimeError::internal("terminal hung up"));
        let options = RunOptions::default();
        let mut runtime = make_runtime_never();
        let mut run_context = RunContext::new(&mut runtime, &options);

        Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
            .await
            .unwrap();

        let reason = frontend
            .events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::StepFinished {
                    outcome: StepOutcome::Failed,
                    reason,
                    ..
                } => Some(*reason),
                _ => None,
            })
            .expect("expected StepFinished{Failed}");
        assert_eq!(
            reason,
            Some(ags_protocol::workflow::StepOutcomeReason::Confirm)
        );
    }

    /// The user declining at the confirm gate stays `at_confirm`: the
    /// cancellation label keeps its original, now-exclusive meaning.
    #[tokio::test]
    async fn test_confirm_decline_still_reports_at_confirm() {
        let compiled = make_workflow(true);
        let mut frontend = MockFrontend::new().with_confirm_response(false);
        let options = RunOptions::default();
        let mut runtime = make_runtime_never();
        let mut run_context = RunContext::new(&mut runtime, &options);

        Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
            .await
            .unwrap();

        let reason = frontend
            .events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::StepFinished {
                    outcome: StepOutcome::Cancelled,
                    reason,
                    ..
                } => Some(*reason),
                _ => None,
            })
            .expect("expected StepFinished{Cancelled}");
        assert_eq!(
            reason,
            Some(ags_protocol::workflow::StepOutcomeReason::AtConfirm)
        );
    }
}

#[cfg(test)]
mod executor_failure {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::workflows::executor::{Executor, RunContext};
    use ags_protocol::catalogue::{
        ApiVersion, HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema,
        ResourceSchema, ScopeEntry, ServiceId, ServiceSchema,
    };
    use ags_protocol::workflow::{
        AutoDeriveScope, AutoDerivedField, CaptureSource, CompiledStep, CompiledWorkflow,
        OperationReference, StepFieldLocation, StepOutputCapture, WorkflowId,
    };

    /// HTTP client that serves responses from a queue; each `send` pops the
    /// front entry. An `Err` entry simulates a transport failure.
    struct QueuedClient {
        responses: Arc<Mutex<Vec<Result<HttpResponse, RuntimeError>>>>,
    }

    impl QueuedClient {
        /// Build the scripted client/test double with the given canned responses.
        fn new(responses: Vec<Result<HttpResponse, RuntimeError>>) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses)),
            }
        }
    }

    #[async_trait]
    impl HttpClient for QueuedClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            self.responses.lock().unwrap().remove(0)
        }
    }

    struct NeverClient;

    #[async_trait]
    impl HttpClient for NeverClient {
        async fn send(&self, _: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            unreachable!("dispatch must not be reached in this test")
        }
    }

    fn make_service_schema(service_name: &str, operation_id: &str) -> ServiceSchema {
        let operation = OperationSchema {
            id: OperationId::new(operation_id),
            name: "op".into(),
            summary: "Op".into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/items".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        ServiceSchema {
            name: service_name.into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "op".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![operation],
                    }],
                }],
            }],
        }
    }

    fn make_step(
        id: &str,
        index: usize,
        service: &str,
        operation: &str,
        outputs: Vec<StepOutputCapture>,
        auto_derived: Vec<AutoDerivedField>,
    ) -> CompiledStep {
        CompiledStep {
            id: id.into(),
            index,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new(service),
                operation: OperationId::new(operation),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs,
            auto_derived,
        }
    }

    fn ok_response(body: &str) -> Result<HttpResponse, RuntimeError> {
        Ok(HttpResponse {
            status: 200,
            body: HttpBody::Text(body.to_string()),
        })
    }

    fn err_response() -> Result<HttpResponse, RuntimeError> {
        Err(RuntimeError::internal("simulated transport failure"))
    }

    // ------------------------------------------------------------------ //
    // Test 1: dispatch error at step 2 of a 2-step workflow               //
    // ------------------------------------------------------------------ //

    /// A 2-step workflow where step 0 succeeds and step 1 fails with a
    /// transport error. Asserts:
    ///   WorkflowStarted, StepStarted{s0}, StepFinished{s0 Success},
    ///   StepStarted{s1}, StepFinished{s1 Failed}, WorkflowFinished{Failed}.
    /// No events for any hypothetical s2. `pending_error` must be `Some`.
    #[tokio::test]
    async fn test_dispatch_error_at_step_2_of_3_emits_step_finished_failed() {
        let svc = "test-svc";
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("two-step-wf"),
            name: "two-step workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![
                make_step("s0", 0, svc, "Op0", vec![], vec![]),
                make_step("s1", 1, svc, "Op0", vec![], vec![]),
            ],
            outputs: vec![],
            completion: None,
        };

        // s0 → success, s1 → transport error.
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(QueuedClient::new(vec![
                ok_response(r#"{"id": 1}"#),
                err_response(),
            ])),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests(svc, make_service_schema(svc, "Op0"));

        let mut frontend = MockFrontend::new();
        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Failed);
        assert!(final_output.is_none());
        assert!(pending.is_some(), "expected a pending error");

        // Build positional index of event kinds for order assertions.
        let events = &frontend.events;

        let pos = |pred: fn(&WorkflowEvent) -> bool| {
            events
                .iter()
                .position(pred)
                .expect("expected event not found")
        };

        let wf_started = pos(|e| matches!(e, WorkflowEvent::WorkflowStarted { .. }));
        let s0_started = pos(|e| matches!(e, WorkflowEvent::StepStarted { id, .. } if id == "s0"));
        let s0_finished = pos(
            |e| matches!(e, WorkflowEvent::StepFinished { id, outcome: StepOutcome::Success, .. } if id == "s0"),
        );
        let s1_started = pos(|e| matches!(e, WorkflowEvent::StepStarted { id, .. } if id == "s1"));
        let s1_finished = pos(
            |e| matches!(e, WorkflowEvent::StepFinished { id, outcome: StepOutcome::Failed, .. } if id == "s1"),
        );
        let wf_finished = pos(|e| {
            matches!(
                e,
                WorkflowEvent::WorkflowFinished {
                    outcome: RunOutcome::Failed,
                    ..
                }
            )
        });

        assert!(
            wf_started < s0_started,
            "WorkflowStarted must precede StepStarted{{s0}}"
        );
        assert!(
            s0_started < s0_finished,
            "StepStarted{{s0}} must precede StepFinished{{s0}}"
        );
        assert!(
            s0_finished < s1_started,
            "StepFinished{{s0}} must precede StepStarted{{s1}}"
        );
        assert!(
            s1_started < s1_finished,
            "StepStarted{{s1}} must precede StepFinished{{s1}}"
        );
        assert!(
            s1_finished < wf_finished,
            "StepFinished{{s1}} must precede WorkflowFinished"
        );

        // No s2 events should have fired.
        let s2_any = events.iter().any(|e| match e {
            WorkflowEvent::StepStarted { id, .. } | WorkflowEvent::StepFinished { id, .. } => {
                id == "s2"
            }
            _ => false,
        });
        assert!(!s2_any, "no s2 events should fire after s1 fails");
    }

    // ------------------------------------------------------------------ //
    // Test 2: capture error promotes to Failed                            //
    // ------------------------------------------------------------------ //

    /// A 1-step workflow whose step declares an output capture with
    /// `path: "$.missing"` and no `default:`. The server returns `{}`.
    /// `bind_step_outputs` fails because the path misses and no default
    /// exists. Outcome must be Failed; StepFinished{Failed}; pending_error present.
    #[tokio::test]
    async fn test_capture_error_promotes_to_failed() {
        let svc = "test-svc";
        let bad_capture = StepOutputCapture {
            name: "my-val".into(),
            source: CaptureSource::ResponseBody {
                path: "$.missing".into(),
            },
            default: None,
            sensitive: false,
        };
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("capture-wf"),
            name: "capture workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![make_step("s0", 0, svc, "Op0", vec![bad_capture], vec![])],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            // Server returns `{}` — no `missing` key.
            Box::new(QueuedClient::new(vec![ok_response("{}")])),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests(svc, make_service_schema(svc, "Op0"));

        let mut frontend = MockFrontend::new();
        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Failed);
        assert!(final_output.is_none());
        assert!(
            pending.is_some(),
            "expected a pending error from bad capture"
        );

        let step_failed = frontend.events.iter().any(|e| {
            matches!(
                e,
                WorkflowEvent::StepFinished {
                    outcome: StepOutcome::Failed,
                    ..
                }
            )
        });
        assert!(step_failed, "expected StepFinished{{Failed}}");

        let wf_failed = frontend.events.iter().any(|e| {
            matches!(
                e,
                WorkflowEvent::WorkflowFinished {
                    outcome: RunOutcome::Failed,
                    ..
                }
            )
        });
        assert!(wf_failed, "expected WorkflowFinished{{Failed}}");
    }

    // ------------------------------------------------------------------ //
    // Test 3: gather error fails the step                                 //
    // ------------------------------------------------------------------ //

    /// A 1-step workflow with an auto-derived workflow input that is absent
    /// from the pre-supplied map, so gather fires. The mock injects a gather
    /// error. Outcome must be Failed; no dispatch (NeverClient); pending_error present.
    #[tokio::test]
    async fn test_gather_error_fails_step() {
        let svc = "test-svc";
        let auto_derived_field = AutoDerivedField {
            field: "namespace".into(),
            schema: serde_json::json!({"type": "string"}),
            required: true,
            sensitive: false,
            description: None,
            scope: AutoDeriveScope::WorkflowInput {
                name: "namespace".into(),
            },
            location: StepFieldLocation::Body,
        };
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("gather-err-wf"),
            name: "gather error workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![make_step(
                "s0",
                0,
                svc,
                "Op0",
                vec![],
                vec![auto_derived_field],
            )],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(NeverClient),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests(svc, make_service_schema(svc, "Op0"));

        let gather_err = RuntimeError::internal("stdin closed unexpectedly");
        let mut frontend = MockFrontend::new().with_gather_error(gather_err);
        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) = Executor::execute(
            &compiled,
            BTreeMap::new(), // namespace not pre-supplied → gather fires
            &mut frontend,
            &mut run_context,
        )
        .await
        .unwrap();

        assert_eq!(outcome, RunOutcome::Failed);
        assert!(final_output.is_none());
        assert!(
            pending.is_some(),
            "expected a pending error from gather failure"
        );

        let step_failed = frontend.events.iter().any(|e| {
            matches!(
                e,
                WorkflowEvent::StepFinished {
                    outcome: StepOutcome::Failed,
                    ..
                }
            )
        });
        assert!(
            step_failed,
            "expected StepFinished{{Failed}} after gather error"
        );
    }

    // ------------------------------------------------------------------ //
    // Test 4: catalogue lookup error fails the step                       //
    // ------------------------------------------------------------------ //

    /// A 1-step workflow referencing service "unknown-service" which is NOT
    /// pre-warmed in the catalogue. `get_or_load` fails (bundled spec lookup
    /// returns an error for a non-existent service). Outcome Failed; no
    /// dispatch; pending_error present.
    #[tokio::test]
    async fn test_catalogue_lookup_error_fails_step() {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("cat-err-wf"),
            name: "catalogue error workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![make_step("s0", 0, "unknown-service", "Op0", vec![], vec![])],
            outputs: vec![],
            completion: None,
        };

        // NeverClient: dispatch must not be reached.
        let runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(NeverClient),
            reqwest::Client::new(),
        );
        // Deliberately do NOT insert "unknown-service" into the catalogue.
        let mut runtime = runtime;

        let mut frontend = MockFrontend::new();
        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Failed);
        assert!(final_output.is_none());
        assert!(
            pending.is_some(),
            "expected a pending error from catalogue lookup failure"
        );

        let step_failed = frontend.events.iter().any(|e| {
            matches!(
                e,
                WorkflowEvent::StepFinished {
                    outcome: StepOutcome::Failed,
                    ..
                }
            )
        });
        assert!(
            step_failed,
            "expected StepFinished{{Failed}} after catalogue error"
        );

        let wf_failed = frontend.events.iter().any(|e| {
            matches!(
                e,
                WorkflowEvent::WorkflowFinished {
                    outcome: RunOutcome::Failed,
                    ..
                }
            )
        });
        assert!(
            wf_failed,
            "expected WorkflowFinished{{Failed}} after catalogue error"
        );
    }

    /// Run the catalogue-lookup-error fixture (a step referencing a service
    /// the catalogue was never seeded with) against `frontend`, driving it to
    /// completion. Reuses the setup behind `test_catalogue_lookup_error_fails_step`
    /// so the schema-load failure path is exercised from one source of truth.
    async fn run_workflow_with_unknown_service(frontend: &mut MockFrontend) {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("cat-err-wf-reason"),
            name: "catalogue error workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![make_step(
                "s0",
                0,
                "unknown-service-reason",
                "Op0",
                vec![],
                vec![],
            )],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(NeverClient),
            reqwest::Client::new(),
        );
        // Deliberately do NOT insert "unknown-service-reason" into the catalogue.

        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        Executor::execute(&compiled, BTreeMap::new(), frontend, &mut run_context)
            .await
            .unwrap();
    }

    /// A schema-load failure (the step's service is unknown to the catalogue)
    /// must report the typed `SchemaLoad` reason and carry error facts.
    #[tokio::test]
    async fn test_schema_load_failure_reports_schema_load_reason() {
        let mut frontend = MockFrontend::new();
        run_workflow_with_unknown_service(&mut frontend).await;
        let (reason, error) = frontend
            .events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::StepFinished {
                    outcome: StepOutcome::Failed,
                    reason,
                    error,
                    ..
                } => Some((*reason, error.clone())),
                _ => None,
            })
            .expect("a failed StepFinished must be emitted");
        assert_eq!(
            reason,
            Some(ags_protocol::workflow::StepOutcomeReason::SchemaLoad)
        );
        assert!(error.is_some(), "a failed step must carry error facts");
    }

    /// Run a single-step workflow whose dispatch fails with an HTTP 409
    /// carrying an AccelByte error code, against `frontend`. The default
    /// `MockFrontend::resolve_step_failure` (Cancel) halts the run at the
    /// interactive failure gate, so the emitted `StepFinished{Failed}` carries
    /// the classified error facts.
    async fn run_workflow_with_upstream_409(frontend: &mut MockFrontend) {
        let svc = "conflict-svc";
        let op_id = "ConflictOp";

        let compiled = CompiledWorkflow {
            id: WorkflowId::new("conflict-wf"),
            name: "conflict workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![make_step("s0", 0, svc, op_id, vec![], vec![])],
            outputs: vec![],
            completion: None,
        };

        let service_schema = make_service_schema(svc, op_id);

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(QueuedClient::new(vec![Ok(HttpResponse {
                status: 409,
                body: HttpBody::Text(
                    r#"{"errorCode":20013,"errorMessage":"conflict"}"#.to_string(),
                ),
            })])),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests(svc, service_schema);

        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        Executor::execute(&compiled, BTreeMap::new(), frontend, &mut run_context)
            .await
            .unwrap();
    }

    /// A dispatch failure classified as an upstream 409 must carry the HTTP
    /// status and the "upstream" telemetry class in its error facts.
    #[tokio::test]
    async fn test_dispatch_failure_carries_status_and_class() {
        let mut frontend = MockFrontend::new();
        run_workflow_with_upstream_409(&mut frontend).await;
        let error = frontend
            .events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::StepFinished { error, .. } => error.clone(),
                _ => None,
            })
            .expect("failed step must carry error facts");
        assert_eq!(error.http_status, Some(409));
        assert_eq!(error.class, "upstream");
    }

    // ------------------------------------------------------------------ //
    // Test 5: StepFinished.attempts counts every real dispatch             //
    // ------------------------------------------------------------------ //

    /// HTTP client whose every dispatch fails with a transport error, for
    /// driving the interactive failure gate through repeated retries.
    struct AlwaysErrClient;

    #[async_trait]
    impl HttpClient for AlwaysErrClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            Err(RuntimeError::internal("simulated transport failure"))
        }
    }

    /// Frontend that answers the failure gate with `Retry` a fixed number of
    /// times and then `Cancel`, so a test can drive a known number of
    /// dispatch attempts.
    struct RetryThenCancelFrontend {
        retries_remaining: u32,
        events: Vec<WorkflowEvent>,
    }

    impl RetryThenCancelFrontend {
        /// Retry `retries` times before cancelling.
        fn new(retries: u32) -> Self {
            Self {
                retries_remaining: retries,
                events: Vec::new(),
            }
        }
    }

    impl WorkflowFrontend for RetryThenCancelFrontend {
        fn on_event(&mut self, event: &WorkflowEvent) {
            self.events.push(event.clone());
        }

        fn gather_workflow_inputs(
            &mut self,
            _needed: &[ags_protocol::workflow::WorkflowInputNeeded],
            _step_context: &CompiledStep,
            _supplied: &[ags_protocol::workflow::SuppliedInputView],
        ) -> Result<ags_protocol::workflow::GatherResult, RuntimeError> {
            unreachable!("fixture step needs no gather")
        }

        fn confirm_step(
            &mut self,
            _step: &CompiledStep,
            _preview: &ags_protocol::workflow::StepPreview,
        ) -> Result<ags_protocol::workflow::StepConfirmOutcome, RuntimeError> {
            unreachable!("fixture step does not confirm")
        }

        fn resolve_step_failure(
            &mut self,
            _step: &CompiledStep,
            _error: &RuntimeError,
            _allow_skip: bool,
        ) -> Result<ags_protocol::workflow::StepFailureAction, RuntimeError> {
            if self.retries_remaining > 0 {
                self.retries_remaining -= 1;
                Ok(ags_protocol::workflow::StepFailureAction::Retry)
            } else {
                Ok(ags_protocol::workflow::StepFailureAction::Cancel)
            }
        }
    }

    /// Drive a 1-step workflow whose dispatch always fails through
    /// `Executor::execute`, so a test can exercise the interactive failure
    /// gate under a real dispatch loop.
    async fn run_always_failing_single_step(frontend: &mut dyn WorkflowFrontend) {
        let svc = "test-svc";
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("always-failing-wf"),
            name: "always failing workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![make_step("s0", 0, svc, "Op0", vec![], vec![])],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(AlwaysErrClient),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests(svc, make_service_schema(svc, "Op0"));

        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);
        let _ = Executor::execute(&compiled, BTreeMap::new(), frontend, &mut run_context).await;
    }

    #[tokio::test]
    async fn test_step_finished_reports_every_dispatch_attempt() {
        let mut frontend = RetryThenCancelFrontend::new(2);
        run_always_failing_single_step(&mut frontend).await;

        let attempts = frontend
            .events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::StepFinished { attempts, .. } => Some(*attempts),
                _ => None,
            })
            .expect("StepFinished must be emitted");
        assert_eq!(attempts, 3, "two retries means three dispatch attempts");
    }

    /// Build a one-operation `ServiceSchema` whose body has one required
    /// string field, `storeName` — enough for `resolve_step_fields` to
    /// produce a non-empty field plan for the failing-step tests below.
    fn make_service_schema_with_store_name_body(
        service_name: &str,
        operation_id: &str,
    ) -> ServiceSchema {
        use ags_protocol::catalogue::{BodyField, BodyFieldType, BodySchema};
        let operation = OperationSchema {
            id: OperationId::new(operation_id),
            name: "op".into(),
            summary: "Op".into(),
            description: None,
            mutation_class: MutationClass::Mutating,
            http_method: HttpMethod::Post,
            path_template: "/items".into(),
            parameters: vec![],
            request_body: Some(BodySchema {
                item_type: None,
                is_array: false,
                definition_name: "Body".into(),
                fields: vec![BodyField {
                    name: "storeName".into(),
                    field_type: BodyFieldType::String,
                    required: true,
                    description: None,
                    children: vec![],
                    default: None,
                }],
            }),
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        ServiceSchema {
            name: service_name.into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "op".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![operation],
                    }],
                }],
            }],
        }
    }

    /// Build a 1-step workflow whose step binds a `storeName` body field to
    /// the literal `"acme-store"` — used by the `input_fields` wiring tests
    /// below, so the resolved field plan is non-empty.
    fn make_one_step_workflow_with_store_name_binding(svc: &str) -> CompiledWorkflow {
        let mut step = make_step("s0", 0, svc, "Op0", vec![], vec![]);
        step.inputs = vec![ags_protocol::workflow::StepInputBinding {
            field: "storeName".to_string(),
            source: ags_protocol::workflow::BindingSource::Literal(
                ags_protocol::workflow::LiteralBinding {
                    value: serde_json::json!("acme-store"),
                    sensitive: false,
                },
            ),
            show_in_review: false,
            description: None,
        }];
        CompiledWorkflow {
            id: WorkflowId::new("store-name-wf"),
            name: "store name workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![step],
            outputs: vec![],
            completion: None,
        }
    }

    /// A bundled workflow's failed step reports its resolved `storeName`
    /// field with its real (redacted-if-needed, here harmless) value.
    #[tokio::test]
    async fn test_bundled_workflow_failed_step_reports_real_input_field_value() {
        let svc = "test-svc";
        let compiled = make_one_step_workflow_with_store_name_binding(svc);

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(AlwaysErrClient),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests(svc, make_service_schema_with_store_name_body(svc, "Op0"));

        let options = RunOptions {
            is_bundled_workflow: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);
        let mut frontend = MockFrontend::new();
        let _ =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context).await;

        let input_fields = frontend
            .events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::StepFinished {
                    error: Some(facts), ..
                } => Some(facts.input_fields.clone()),
                _ => None,
            })
            .expect("a failed step must carry StepErrorFacts");
        let store_name = input_fields
            .iter()
            .find(|f| f.field == "storeName")
            .expect("storeName must be in the resolved field plan");
        assert_eq!(store_name.value, Some(serde_json::json!("acme-store")));
        assert_eq!(store_name.source, "literal");
        assert!(store_name.required);
    }

    /// An external (user-installed) workflow's failed step reports every
    /// field with `value: None`, even a field whose name and schema are
    /// entirely harmless.
    #[tokio::test]
    async fn test_external_workflow_failed_step_withholds_every_input_field_value() {
        let svc = "test-svc";
        let compiled = make_one_step_workflow_with_store_name_binding(svc);

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(AlwaysErrClient),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests(svc, make_service_schema_with_store_name_body(svc, "Op0"));

        // Default `is_bundled_workflow: false` — the safe default for an
        // external or unregistered (synthesised) workflow.
        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);
        let mut frontend = MockFrontend::new();
        let _ =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context).await;

        let input_fields = frontend
            .events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::StepFinished {
                    error: Some(facts), ..
                } => Some(facts.input_fields.clone()),
                _ => None,
            })
            .expect("a failed step must carry StepErrorFacts");
        assert!(!input_fields.is_empty());
        assert!(input_fields.iter().all(|f| f.value.is_none()));
    }

    /// A successful step's `StepFinished` carries no `StepErrorFacts` at all
    /// (and therefore no `input_fields`) — the type-level enforcement the
    /// design relies on for "failures only".
    #[tokio::test]
    async fn test_successful_step_carries_no_error_facts_or_input_fields() {
        let svc = "test-svc";
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("one-step-ok-wf"),
            name: "one step ok workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![make_step("s0", 0, svc, "Op0", vec![], vec![])],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(QueuedClient::new(vec![ok_response(r#"{"id": 1}"#)])),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests(svc, make_service_schema(svc, "Op0"));

        let options = RunOptions {
            is_bundled_workflow: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);
        let mut frontend = MockFrontend::new();
        let (outcome, _, _) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();
        assert_eq!(outcome, RunOutcome::Success);

        let error = frontend
            .events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::StepFinished { error, .. } => Some(error.clone()),
                _ => None,
            })
            .expect("StepFinished must be emitted");
        assert!(error.is_none());
    }
}

#[cfg(test)]
mod executor_dry_run {
    use std::collections::BTreeMap;

    use async_trait::async_trait;

    use super::*;
    use crate::runtime::dispatch::http::{HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::workflows::executor::{Executor, RunContext};
    use ags_protocol::catalogue::{
        ApiVersion, HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema,
        ResourceSchema, ScopeEntry, ServiceId, ServiceSchema,
    };
    use ags_protocol::output::CommandOutput;
    use ags_protocol::workflow::{
        CaptureSource, CompiledStep, CompiledWorkflow, OperationReference, StepOutputCapture,
        WorkflowId,
    };

    struct NeverClient;

    #[async_trait]
    impl HttpClient for NeverClient {
        async fn send(&self, _: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            unreachable!("dry-run test must not dispatch")
        }
    }

    fn make_service_schema(service_name: &str, operation_id: &str) -> ServiceSchema {
        let operation = OperationSchema {
            id: OperationId::new(operation_id),
            name: "op".into(),
            summary: "Op".into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/items".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        ServiceSchema {
            name: service_name.into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "op".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![operation],
                    }],
                }],
            }],
        }
    }

    fn make_runtime_never(service_name: &str, operation_id: &str) -> crate::runtime::Runtime {
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(NeverClient),
            reqwest::Client::new(),
        );
        runtime.catalogue_mut().insert_for_tests(
            service_name,
            make_service_schema(service_name, operation_id),
        );
        runtime
    }

    fn make_step(id: &str, index: usize, service: &str, operation: &str) -> CompiledStep {
        CompiledStep {
            id: id.into(),
            index,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new(service),
                operation: OperationId::new(operation),
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

    fn make_step_with_outputs(
        id: &str,
        index: usize,
        service: &str,
        operation: &str,
        outputs: Vec<StepOutputCapture>,
    ) -> CompiledStep {
        CompiledStep {
            id: id.into(),
            index,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new(service),
                operation: OperationId::new(operation),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs,
            auto_derived: vec![],
        }
    }

    // ------------------------------------------------------------------ //
    // Test 1: 1-step dry-run → CommandOutput::DryRun                      //
    // ------------------------------------------------------------------ //

    /// A 1-step workflow with `--dry-run` must produce `CommandOutput::DryRun`
    /// without ever calling the HTTP client.
    #[tokio::test]
    async fn test_dry_run_single_step_builds_dry_run() {
        let svc = "test-svc";
        let op = "GetItems";
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "wf".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![make_step("s0", 0, svc, op)],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = make_runtime_never(svc, op);
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            dry_run: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);
        assert!(pending.is_none());
        assert!(
            matches!(final_output, Some(CommandOutput::DryRun(_))),
            "expected Some(DryRun(_)), got {final_output:?}"
        );
    }

    // ------------------------------------------------------------------ //
    // Test 2: 2-step dry-run → CommandOutput::WorkflowDryRun with 2 steps //
    // ------------------------------------------------------------------ //

    /// A 2-step workflow with `--dry-run` must produce
    /// `CommandOutput::WorkflowDryRun` with exactly 2 previews.
    #[tokio::test]
    async fn test_dry_run_multi_step_builds_workflow_dry_run() {
        let svc = "test-svc";
        let op = "GetItems";
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "wf".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![make_step("s0", 0, svc, op), make_step("s1", 1, svc, op)],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = make_runtime_never(svc, op);
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            dry_run: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);
        assert!(pending.is_none());
        match final_output {
            Some(CommandOutput::WorkflowDryRun { step_previews, .. }) => {
                assert_eq!(step_previews.len(), 2, "expected 2 dry-run previews");
            }
            other => panic!("expected Some(WorkflowDryRun{{..}}), got {other:?}"),
        }
    }

    // ------------------------------------------------------------------ //
    // Test 3: synthesise failure routes through Failed                     //
    // ------------------------------------------------------------------ //

    /// A step referencing a service NOT in the catalogue causes
    /// `synthesise_dry_run_outputs` to fail at `catalogue.get_or_load`.
    /// Outcome must be Failed.
    #[tokio::test]
    async fn test_dry_run_preview_error_routes_through_failure() {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "wf".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![make_step("s0", 0, "unknown-service", "Op0")],
            outputs: vec![],
            completion: None,
        };

        // No service pre-loaded → synthesise_dry_run_outputs fails.
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(NeverClient),
            reqwest::Client::new(),
        );

        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            dry_run: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Failed);
        assert!(final_output.is_none());
        assert!(
            pending.is_some(),
            "expected a pending error from synthesise failure"
        );

        let step_failed = frontend.events.iter().any(|e| {
            matches!(
                e,
                WorkflowEvent::StepFinished {
                    outcome: StepOutcome::Failed,
                    ..
                }
            )
        });
        assert!(step_failed, "expected StepFinished{{Failed}}");
    }

    // ------------------------------------------------------------------ //
    // Test 4: synthesised outputs flow to downstream step                  //
    // ------------------------------------------------------------------ //

    /// A 2-step workflow where step s0 declares an output `id` (with a
    /// `default:` so the placeholder is deterministic). Both steps succeed
    /// in dry-run mode. s0's `StepDryRunPreview.synthesised_outputs` carries
    /// `{"id": "my-placeholder"}`, confirming the default was used.
    #[tokio::test]
    async fn test_dry_run_synthesised_outputs_flow_to_next_step() {
        let svc = "test-svc";
        let op = "GetItems";

        let s0_capture = StepOutputCapture {
            name: "id".into(),
            source: CaptureSource::ResponseBody {
                path: "$.id".into(),
            },
            default: Some(serde_json::json!("my-placeholder")),
            sensitive: false,
        };
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "wf".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![
                make_step_with_outputs("s0", 0, svc, op, vec![s0_capture]),
                make_step("s1", 1, svc, op),
            ],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = make_runtime_never(svc, op);
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            dry_run: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);
        assert!(pending.is_none());

        match final_output {
            Some(CommandOutput::WorkflowDryRun { step_previews, .. }) => {
                assert_eq!(step_previews.len(), 2, "expected 2 previews");
                let s0_preview = &step_previews[0];
                assert_eq!(s0_preview.step_id, "s0");
                let id_val = s0_preview.synthesised_outputs.get("id");
                assert_eq!(
                    id_val,
                    Some(&serde_json::json!("my-placeholder")),
                    "s0 synthesised output 'id' must be the declared default"
                );
            }
            other => panic!("expected Some(WorkflowDryRun{{..}}), got {other:?}"),
        }
    }
}

#[cfg(test)]
mod executor_no_input {
    use std::collections::BTreeMap;

    use async_trait::async_trait;

    use super::*;
    use crate::runtime::dispatch::http::{HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::workflows::executor::{
        no_input_precheck, no_input_violations_to_error, Executor, NoInputViolation, RunContext,
    };
    use ags_protocol::catalogue::{
        ApiVersion, HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema,
        ResourceSchema, ScopeEntry, ServiceId, ServiceSchema,
    };
    use ags_protocol::workflow::{
        AutoDeriveScope, AutoDerivedField, CompiledStep, CompiledWorkflow, OperationReference,
        StepFieldLocation, WorkflowId, WorkflowInputSpec,
    };

    struct NeverClient;

    #[async_trait]
    impl HttpClient for NeverClient {
        async fn send(&self, _: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            unreachable!("no_input precheck must prevent dispatch")
        }
    }

    fn make_service_schema() -> ServiceSchema {
        let operation = OperationSchema {
            id: OperationId::new("GetItems"),
            name: "list".into(),
            summary: "List items".into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/namespaces/{namespace}/items".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        ServiceSchema {
            name: "test-svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "list".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![operation],
                    }],
                }],
            }],
        }
    }

    fn make_runtime_never() -> crate::runtime::Runtime {
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(NeverClient),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("test-svc", make_service_schema());
        runtime
    }

    /// Build a 1-step compiled workflow whose step auto-derives the given
    /// workflow input.
    fn make_workflow_with_input(input_name: &str) -> CompiledWorkflow {
        CompiledWorkflow {
            id: WorkflowId::new("test-wf"),
            name: "test workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![WorkflowInputSpec {
                name: input_name.into(),
                description: None,
                schema: Some(serde_json::json!({"type": "string"})),
                required: true,
                default: None,
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
                file_picker: None,
            }],
            is_reviewed_by_default: true,
            steps: vec![CompiledStep {
                id: "step-a".into(),
                index: 0,
                description: None,
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new("test-svc"),
                    operation: OperationId::new("GetItems"),
                }),
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
                auto_derived: vec![AutoDerivedField {
                    field: input_name.into(),
                    schema: serde_json::json!({"type": "string"}),
                    required: true,
                    sensitive: false,
                    description: None,
                    scope: AutoDeriveScope::WorkflowInput {
                        name: input_name.into(),
                    },
                    location: StepFieldLocation::Body,
                }],
            }],
            outputs: vec![],
            completion: None,
        }
    }

    // ------------------------------------------------------------------ //
    // Test 1: missing workflow input reported exactly once                 //
    // ------------------------------------------------------------------ //

    /// A workflow with two steps that both auto-derive the same workflow input
    /// `X`. With `no_input: true` and no pre-supplied values, the precheck
    /// must report exactly one `MissingInput` entry (dedup by simulation).
    #[test]
    fn test_no_input_missing_workflow_input_reported_once() {
        let step_a = CompiledStep {
            id: "step-a".into(),
            index: 0,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new("test-svc"),
                operation: OperationId::new("GetItems"),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![],
            auto_derived: vec![AutoDerivedField {
                field: "X".into(),
                schema: serde_json::json!({"type": "string"}),
                required: true,
                sensitive: false,
                description: None,
                scope: AutoDeriveScope::WorkflowInput { name: "X".into() },
                location: StepFieldLocation::Body,
            }],
        };
        let step_b = CompiledStep {
            id: "step-b".into(),
            index: 1,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new("test-svc"),
                operation: OperationId::new("GetItems"),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![],
            auto_derived: vec![AutoDerivedField {
                field: "X".into(),
                schema: serde_json::json!({"type": "string"}),
                required: true,
                sensitive: false,
                description: None,
                scope: AutoDeriveScope::WorkflowInput { name: "X".into() },
                location: StepFieldLocation::Body,
            }],
        };
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "wf".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![WorkflowInputSpec {
                name: "X".into(),
                description: None,
                schema: Some(serde_json::json!({"type": "string"})),
                required: true,
                default: None,
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
                file_picker: None,
            }],
            is_reviewed_by_default: true,
            steps: vec![step_a, step_b],
            outputs: vec![],
            completion: None,
        };

        let violations = no_input_precheck(&compiled, &BTreeMap::new(), false, false);

        let missing: Vec<_> = violations
            .iter()
            .filter(|v| matches!(v, NoInputViolation::MissingInput { name, .. } if name == "X"))
            .collect();
        assert_eq!(
            missing.len(),
            1,
            "expected exactly one MissingInput for X, got {violations:?}"
        );
        assert!(
            matches!(&missing[0], NoInputViolation::MissingInput { first_seen_at, .. } if first_seen_at == "step-a"),
            "first_seen_at must be step-a"
        );
    }

    // ------------------------------------------------------------------ //
    // Test 2: confirm: true reported under --no-input without dry-run      //
    // ------------------------------------------------------------------ //

    /// A 1-step workflow with `confirm: true`. Precheck with `dry_run: false`
    /// must contain a `ConfirmRequired` violation for that step.
    #[test]
    fn test_no_input_confirm_required_reported() {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "wf".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![CompiledStep {
                id: "confirm-step".into(),
                index: 0,
                description: None,
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new("test-svc"),
                    operation: OperationId::new("GetItems"),
                }),
                dependencies: vec![],
                confirm: true,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
                auto_derived: vec![],
            }],
            outputs: vec![],
            completion: None,
        };

        let violations = no_input_precheck(&compiled, &BTreeMap::new(), false, false);

        assert!(
            violations.iter().any(|v| matches!(v, NoInputViolation::ConfirmRequired { step } if step == "confirm-step")),
            "expected ConfirmRequired for confirm-step, got {violations:?}"
        );
    }

    // ------------------------------------------------------------------ //
    // Test 2b: confirm: true suppressed under --yes (assume_yes)           //
    // ------------------------------------------------------------------ //

    /// Same workflow as test 2; with `assume_yes: true` the `--yes` flag
    /// pre-approves the confirmation, so `ConfirmRequired` must NOT appear.
    #[test]
    fn test_no_input_confirm_suppressed_with_assume_yes() {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "wf".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![CompiledStep {
                id: "confirm-step".into(),
                index: 0,
                description: None,
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new("test-svc"),
                    operation: OperationId::new("GetItems"),
                }),
                dependencies: vec![],
                confirm: true,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
                auto_derived: vec![],
            }],
            outputs: vec![],
            completion: None,
        };

        let violations = no_input_precheck(&compiled, &BTreeMap::new(), false, true);

        assert!(
            !violations
                .iter()
                .any(|v| matches!(v, NoInputViolation::ConfirmRequired { .. })),
            "ConfirmRequired must not appear when --yes is set, got {violations:?}"
        );
    }

    // ------------------------------------------------------------------ //
    // Test 3: confirm: true suppressed under dry-run                       //
    // ------------------------------------------------------------------ //

    /// Same workflow as test 2; with `dry_run: true` the `ConfirmRequired`
    /// violation must NOT appear.
    #[test]
    fn test_no_input_confirm_suppressed_under_dry_run() {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "wf".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![CompiledStep {
                id: "confirm-step".into(),
                index: 0,
                description: None,
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new("test-svc"),
                    operation: OperationId::new("GetItems"),
                }),
                dependencies: vec![],
                confirm: true,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
                auto_derived: vec![],
            }],
            outputs: vec![],
            completion: None,
        };

        let violations = no_input_precheck(&compiled, &BTreeMap::new(), true, false);

        assert!(
            !violations
                .iter()
                .any(|v| matches!(v, NoInputViolation::ConfirmRequired { .. })),
            "ConfirmRequired must not appear under dry_run=true, got {violations:?}"
        );
    }

    // ------------------------------------------------------------------ //
    // Test 4: StepLocal gather field reported                              //
    // ------------------------------------------------------------------ //

    /// A step with a `StepLocal`-scoped auto-derived field (no matching
    /// workflow input). The precheck must report `StepLocalGather` for that
    /// field.
    #[test]
    fn test_no_input_step_local_gather_reported() {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "wf".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![CompiledStep {
                id: "local-step".into(),
                index: 0,
                description: None,
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new("test-svc"),
                    operation: OperationId::new("GetItems"),
                }),
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
                auto_derived: vec![AutoDerivedField {
                    field: "statCode".into(),
                    schema: serde_json::json!({"type": "string"}),
                    required: true,
                    sensitive: false,
                    description: None,
                    scope: AutoDeriveScope::StepLocal {
                        field_name: "statCode".into(),
                    },
                    location: StepFieldLocation::Body,
                }],
            }],
            outputs: vec![],
            completion: None,
        };

        let violations = no_input_precheck(&compiled, &BTreeMap::new(), false, false);

        assert!(
            violations.iter().any(|v| matches!(
                v,
                NoInputViolation::StepLocalGather { step, field }
                    if step == "local-step" && field == "statCode"
            )),
            "expected StepLocalGather for local-step.statCode, got {violations:?}"
        );
    }

    // ------------------------------------------------------------------ //
    // Test 5: clean run returns empty violation list                       //
    // ------------------------------------------------------------------ //

    /// A workflow with all inputs supplied, no confirm, no step-local fields.
    /// The precheck must return an empty vector.
    #[test]
    fn test_no_input_clean_run_returns_empty() {
        let compiled = make_workflow_with_input("namespace");
        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("dev"));

        let violations = no_input_precheck(&compiled, &supplied, false, false);

        assert!(
            violations.is_empty(),
            "expected no violations when all inputs are supplied, got {violations:?}"
        );
    }

    // ------------------------------------------------------------------ //
    // Test 6: precheck failure emits zero events                           //
    // ------------------------------------------------------------------ //

    /// Drive the full `Executor::execute` with `no_input: true` and an unmet
    /// required workflow input. The executor must return `Err` (not
    /// `Ok(Failed, ...)`) and the frontend must have received zero events.
    #[tokio::test]
    async fn test_no_input_precheck_failure_emits_zero_events() {
        let compiled = make_workflow_with_input("namespace");
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            no_input: true,
            ..Default::default()
        };
        let mut runtime = make_runtime_never();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let result = Executor::execute(
            &compiled,
            BTreeMap::new(), // namespace not supplied → violation
            &mut frontend,
            &mut run_context,
        )
        .await;

        assert!(
            result.is_err(),
            "expected Err from precheck, got {result:?}"
        );
        assert!(
            frontend.events.is_empty(),
            "no events must fire before the precheck error; got {:?}",
            frontend.events
        );

        // Verify the error explains the non-interactive rejection and is
        // Validation kind (a usage error → exit 1), not Internal.
        let err = result.unwrap_err();
        assert!(
            err.message
                .contains("Cannot run this workflow non-interactively"),
            "error message must explain the non-interactive rejection, got: {}",
            err.message
        );
        assert_eq!(err.kind, ags_protocol::error::RuntimeErrorKind::Validation);

        // Smoke-test no_input_violations_to_error produces a non-empty,
        // Validation-kind error.
        let violations = no_input_precheck(&compiled, &BTreeMap::new(), false, false);
        let aggregated = no_input_violations_to_error(&violations, false);
        assert!(
            aggregated.message.contains("namespace"),
            "aggregated error must mention the missing input name"
        );
        assert_eq!(
            aggregated.kind,
            ags_protocol::error::RuntimeErrorKind::Validation
        );
    }

    #[test]
    fn test_no_input_violations_caps_long_list() {
        // Six missing inputs: the first three are listed and the rest collapse
        // into a "(+ N more)" tail so the error stays readable.
        let violations: Vec<NoInputViolation> = (0..6)
            .map(|i| NoInputViolation::MissingInput {
                name: format!("input{i}"),
                first_seen_at: "step-1".to_string(),
            })
            .collect();
        let err = no_input_violations_to_error(&violations, false);
        assert!(
            err.message.contains("input0"),
            "first names shown: {}",
            err.message
        );
        assert!(
            err.message.contains("input2"),
            "first names shown: {}",
            err.message
        );
        assert!(
            !err.message.contains("input5"),
            "later names hidden behind the tail: {}",
            err.message
        );
        assert!(
            err.message.contains("(+ 3 more)"),
            "tail names the hidden count: {}",
            err.message
        );
    }

    #[test]
    fn test_no_input_violations_single_command_omits_workflow_vocabulary() {
        // A synthesised single command (e.g. `platform currencies delete`
        // without --yes) must not leak "workflow" or the internal "step 'main'"
        // sentinel into the user-facing message.
        let violations = vec![NoInputViolation::ConfirmRequired {
            step: "main".to_string(),
        }];
        let err = no_input_violations_to_error(&violations, true);
        assert!(
            err.message
                .contains("This command cannot run non-interactively"),
            "command-oriented headline: {}",
            err.message
        );
        assert!(
            err.message.contains("confirmation is required"),
            "confirm line reworded: {}",
            err.message
        );
        assert!(
            !err.message.contains("workflow"),
            "must not mention 'workflow': {}",
            err.message
        );
        assert!(
            !err.message.contains("step 'main'"),
            "must not leak the 'main' step id: {}",
            err.message
        );
    }

    /// A no-input rejection must carry a machine-readable `details.code`
    /// naming the first violation's kind, so telemetry can report it without
    /// parsing the prose message.
    #[test]
    fn test_no_input_error_carries_a_machine_code_for_the_first_violation() {
        let violations = vec![NoInputViolation::ConfirmRequired {
            step: "create-store".to_string(),
        }];
        let error = no_input_violations_to_error(&violations, false);
        let code = error
            .details
            .as_ref()
            .and_then(|d| d.code.clone())
            .expect("a no-input rejection must carry a machine code");
        assert_eq!(code, "no_input.confirm_required");
    }

    /// The machine code must encode only the violation's kind, never the
    /// user-authored input/step/field names it carries — those are arbitrary
    /// strings from an external workflow and must not reach telemetry.
    #[test]
    fn test_no_input_code_never_contains_an_input_name() {
        let violations = vec![NoInputViolation::MissingInput {
            name: "secret-store-name".to_string(),
            first_seen_at: "create-store".to_string(),
        }];
        let error = no_input_violations_to_error(&violations, false);
        let code = error.details.as_ref().and_then(|d| d.code.clone()).unwrap();
        assert_eq!(code, "no_input.missing_input");
        assert!(
            !code.contains("secret-store-name"),
            "the violation's input name must never reach the code"
        );
    }

    // ------------------------------------------------------------------ //
    // Test 8: optional + confirm step still blocks under no-input         //
    // ------------------------------------------------------------------ //

    /// An optional step with `confirm: true` must STILL yield `ConfirmRequired`
    /// when `assume_yes = false`. Optionality means the user may choose to skip
    /// the step interactively; `--no-input` has no affordance for that choice,
    /// so the confirm gate must still block.
    ///
    /// With `assume_yes = true` (the `--yes` flag), the confirmation is
    /// pre-approved and `ConfirmRequired` must NOT appear.
    #[test]
    fn test_optional_confirm_step_still_requires_confirmation_under_no_input() {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "wf".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![CompiledStep {
                id: "optional-confirm-step".into(),
                index: 0,
                description: None,
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new("test-svc"),
                    operation: OperationId::new("GetItems"),
                }),
                dependencies: vec![],
                confirm: true,
                is_optional: true,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
                auto_derived: vec![],
            }],
            outputs: vec![],
            completion: None,
        };

        // No --yes: confirm gate blocks even though the step is optional.
        let violations = no_input_precheck(&compiled, &BTreeMap::new(), false, false);
        assert!(
            violations.iter().any(|v| matches!(
                v, NoInputViolation::ConfirmRequired { step } if step == "optional-confirm-step"
            )),
            "ConfirmRequired must appear for an optional+confirm step without --yes, got {violations:?}"
        );

        // --yes pre-approves: no ConfirmRequired even for an optional+confirm step.
        let violations_yes = no_input_precheck(&compiled, &BTreeMap::new(), false, true);
        assert!(
            !violations_yes
                .iter()
                .any(|v| matches!(v, NoInputViolation::ConfirmRequired { .. })),
            "ConfirmRequired must NOT appear when --yes is set, got {violations_yes:?}"
        );
    }
}

#[cfg(test)]
mod build_final_output_tests {
    use super::*;
    use crate::runtime::workflows::executor::build_final_output;
    use ags_protocol::catalogue::{
        ApiVersion, HttpMethod, MutationClass, OperationId, OperationSchema, ServiceId,
    };
    use ags_protocol::output::CommandOutput;
    use ags_protocol::output_views::{ApiBody, ApiOutput};
    use ags_protocol::workflow::{
        CompiledStep, CompiledWorkflow, OperationReference, StepDryRunPreview, WorkflowId,
        WorkflowOutputAlias,
    };
    use std::collections::BTreeMap;

    /// Build a minimal `OperationSchema` literal for test use.
    fn dummy_operation_schema() -> OperationSchema {
        OperationSchema {
            id: OperationId::new("dummy"),
            name: "dummy".into(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion::new(1),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// Build a minimal `ApiOutput` with empty body.
    fn dummy_api_output_empty() -> ApiOutput {
        ApiOutput {
            operation: dummy_operation_schema(),
            resource_name: "test".into(),
            body: ApiBody::Empty,
            success: None,
            trace: None,
            raw_body: None,
            has_alternate_versions: false,
        }
    }

    /// Build a 1-step compiled workflow with the given output aliases.
    fn one_step_workflow(outputs: Vec<WorkflowOutputAlias>) -> CompiledWorkflow {
        CompiledWorkflow {
            id: WorkflowId::new("one-step-wf"),
            name: "one step workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![CompiledStep {
                id: "s1".into(),
                index: 0,
                description: None,
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new("test-svc"),
                    operation: OperationId::new("TestOp"),
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
            }],
            outputs,
            completion: None,
        }
    }

    /// Build a 2-step compiled workflow.
    fn two_step_workflow() -> CompiledWorkflow {
        CompiledWorkflow {
            id: WorkflowId::new("two-step-wf"),
            name: "two step workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![
                CompiledStep {
                    id: "s1".into(),
                    index: 0,
                    description: None,
                    kind: ags_protocol::workflow::StepKind::default(),
                    action: None,
                    operation: Some(OperationReference {
                        service: ServiceId::new("test-svc"),
                        operation: OperationId::new("TestOp"),
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
                },
                CompiledStep {
                    id: "s2".into(),
                    index: 1,
                    description: None,
                    kind: ags_protocol::workflow::StepKind::default(),
                    action: None,
                    operation: Some(OperationReference {
                        service: ServiceId::new("test-svc"),
                        operation: OperationId::new("TestOp"),
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
                },
            ],
            outputs: vec![],
            completion: None,
        }
    }

    /// Build a workflow context with one stored output under "s1".
    fn ctx_with_one_stored_output() -> WorkflowContext {
        let mut ctx = WorkflowContext::new();
        ctx.store_step_output("s1", dummy_api_output_empty());
        ctx
    }

    /// Build a minimal `StepDryRunPreview` for the given step.
    fn dry_run_preview(step_id: &str, step_index: usize) -> StepDryRunPreview {
        StepDryRunPreview {
            step_id: step_id.into(),
            step_index,
            action: ags_protocol::workflow::StepDryRunAction::Request(
                ags_protocol::result::DryRunResult {
                    http_method: ags_protocol::catalogue::HttpMethod::Get,
                    url: "https://example.test/".into(),
                    headers: vec![],
                    query: vec![],
                    body: None,
                },
            ),
            synthesised_outputs: BTreeMap::new(),
        }
    }

    /// Test 1: 1-step, empty outputs, non-dry-run → `CommandOutput::Service`.
    #[test]
    fn test_envelope_normal_one_step_empty_outputs_is_service() {
        let workflow = one_step_workflow(vec![]);
        let ctx = ctx_with_one_stored_output();
        let step_summaries = vec!["s1: ok".into()];
        let dry_run_previews = vec![];
        let options = RunOptions {
            dry_run: false,
            ..Default::default()
        };

        let result = build_final_output(
            &workflow,
            &ctx,
            &step_summaries,
            &dry_run_previews,
            &options,
            &std::collections::BTreeMap::new(),
        );

        assert!(
            matches!(result, Some(CommandOutput::Service(_))),
            "expected Some(Service(_)), got {result:?}"
        );
    }

    /// Test 2: 1-step, non-empty outputs, non-dry-run → `CommandOutput::Workflow`.
    #[test]
    fn test_envelope_normal_one_step_with_outputs_is_workflow() {
        let workflow = one_step_workflow(vec![WorkflowOutputAlias {
            name: "output1".into(),
            from_step_id: "s1".into(),
            output: "field1".into(),
            sensitive: false,
            section: None,
            label: None,
            item_fields: None,
        }]);
        let ctx = ctx_with_one_stored_output();
        let step_summaries = vec!["s1: ok".into()];
        let dry_run_previews = vec![];
        let options = RunOptions {
            dry_run: false,
            ..Default::default()
        };

        let result = build_final_output(
            &workflow,
            &ctx,
            &step_summaries,
            &dry_run_previews,
            &options,
            &std::collections::BTreeMap::new(),
        );

        assert!(
            matches!(result, Some(CommandOutput::Workflow { .. })),
            "expected Some(Workflow {{ .. }}), got {result:?}"
        );
    }

    /// Test 3: 2-step, empty outputs, non-dry-run → `CommandOutput::Workflow`.
    #[test]
    fn test_envelope_normal_multi_step_is_workflow() {
        let workflow = two_step_workflow();
        let ctx = ctx_with_one_stored_output();
        let step_summaries = vec!["s1: ok".into(), "s2: ok".into()];
        let dry_run_previews = vec![];
        let options = RunOptions {
            dry_run: false,
            ..Default::default()
        };

        let result = build_final_output(
            &workflow,
            &ctx,
            &step_summaries,
            &dry_run_previews,
            &options,
            &std::collections::BTreeMap::new(),
        );

        assert!(
            matches!(result, Some(CommandOutput::Workflow { .. })),
            "expected Some(Workflow {{ .. }}), got {result:?}"
        );
    }

    /// Test 4: 1-step, dry-run → `CommandOutput::DryRun`.
    #[test]
    fn test_envelope_dry_run_one_step_is_dry_run() {
        let workflow = one_step_workflow(vec![]);
        let ctx = ctx_with_one_stored_output();
        let step_summaries = vec![];
        let dry_run_previews = vec![dry_run_preview("s1", 0)];
        let options = RunOptions {
            dry_run: true,
            ..Default::default()
        };

        let result = build_final_output(
            &workflow,
            &ctx,
            &step_summaries,
            &dry_run_previews,
            &options,
            &std::collections::BTreeMap::new(),
        );

        assert!(
            matches!(result, Some(CommandOutput::DryRun(_))),
            "expected Some(DryRun(_)), got {result:?}"
        );
    }

    /// Test 5: 2-step, dry-run → `CommandOutput::WorkflowDryRun`.
    #[test]
    fn test_envelope_dry_run_multi_step_is_workflow_dry_run() {
        let workflow = two_step_workflow();
        let ctx = ctx_with_one_stored_output();
        let step_summaries = vec![];
        let dry_run_previews = vec![dry_run_preview("s1", 0), dry_run_preview("s2", 1)];
        let options = RunOptions {
            dry_run: true,
            ..Default::default()
        };

        let result = build_final_output(
            &workflow,
            &ctx,
            &step_summaries,
            &dry_run_previews,
            &options,
            &std::collections::BTreeMap::new(),
        );

        assert!(
            matches!(result, Some(CommandOutput::WorkflowDryRun { .. })),
            "expected Some(WorkflowDryRun {{ .. }}), got {result:?}"
        );
    }
}

#[cfg(test)]
mod end_to_end {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::workflows::executor::{Executor, RunContext};
    use ags_protocol::catalogue::{
        ApiVersion, HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema,
        ParameterLocation, ParameterSchema, ResourceSchema, ScopeEntry, ServiceId, ServiceSchema,
        ValueType,
    };
    use ags_protocol::output::CommandOutput;
    use ags_protocol::request::OutputFormat;
    use ags_protocol::workflow::{
        AutoDeriveScope, AutoDerivedField, BindingSource, CaptureSource, CompiledStep,
        CompiledWorkflow, OperationReference, ReferenceBinding, ReferenceTarget, StepFieldLocation,
        StepInputBinding, StepOutputCapture, WorkflowId, WorkflowInputSpec, WorkflowOutputAlias,
    };

    /// HTTP client backed by a FIFO queue; each send pops the front entry.
    struct QueuedClient {
        responses: Arc<Mutex<Vec<Result<HttpResponse, RuntimeError>>>>,
    }

    impl QueuedClient {
        /// Build the scripted client/test double with the given canned responses.
        fn new(responses: Vec<Result<HttpResponse, RuntimeError>>) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses)),
            }
        }
    }

    #[async_trait]
    impl HttpClient for QueuedClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            self.responses.lock().unwrap().remove(0)
        }
    }

    fn ok_json(body: &str) -> Result<HttpResponse, RuntimeError> {
        Ok(HttpResponse {
            status: 200,
            body: HttpBody::Text(body.to_string()),
        })
    }

    /// Build one `OperationSchema` with the given id, path template and path
    /// parameter names (all required, string-typed).
    fn make_op(id: &str, path: &str, path_params: &[&str]) -> OperationSchema {
        OperationSchema {
            id: OperationId::new(id),
            name: id.to_lowercase(),
            summary: id.into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: path.into(),
            parameters: path_params
                .iter()
                .map(|name| ParameterSchema {
                    name: (*name).into(),
                    location: ParameterLocation::Path,
                    required: true,
                    value_type: ValueType::String,
                    is_file: false,
                    description: None,
                    default: None,
                })
                .collect(),
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// Build a `ServiceSchema` with three operations covering the three steps.
    fn make_service_schema() -> ServiceSchema {
        let op_name = make_op(
            "GetName",
            "/svc/v1/namespaces/{namespace}/name",
            &["namespace"],
        );
        let op_id = make_op("GetId", "/svc/v1/namespaces/{namespace}/id", &["namespace"]);
        let op_thing = make_op("GetThing", "/svc/v1/things/{name}/{id}", &["name", "id"]);
        ServiceSchema {
            name: "svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "things".into(),
                description: String::new(),
                methods: vec![
                    MethodSchema {
                        name: "get-name".into(),
                        summary: String::new(),
                        default_scope: None,
                        scopes: vec![ScopeEntry {
                            scope: String::new(),
                            default_version: ApiVersion(1),
                            contracts: vec![op_name],
                        }],
                    },
                    MethodSchema {
                        name: "get-id".into(),
                        summary: String::new(),
                        default_scope: None,
                        scopes: vec![ScopeEntry {
                            scope: String::new(),
                            default_version: ApiVersion(1),
                            contracts: vec![op_id],
                        }],
                    },
                    MethodSchema {
                        name: "get-thing".into(),
                        summary: String::new(),
                        default_scope: None,
                        scopes: vec![ScopeEntry {
                            scope: String::new(),
                            default_version: ApiVersion(1),
                            contracts: vec![op_thing],
                        }],
                    },
                ],
            }],
        }
    }

    fn step_binding(field: &str, from_step: &str, output: &str) -> StepInputBinding {
        StepInputBinding {
            field: field.into(),
            source: BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Step {
                    id: from_step.into(),
                },
                output: Some(output.into()),
                transform: None,
            }),
            show_in_review: false,
            description: None,
        }
    }

    fn step_capture(name: &str, path: &str) -> StepOutputCapture {
        StepOutputCapture {
            name: name.into(),
            source: CaptureSource::ResponseBody { path: path.into() },
            default: None,
            sensitive: false,
        }
    }

    fn namespace_auto_derived() -> AutoDerivedField {
        AutoDerivedField {
            field: "namespace".into(),
            schema: serde_json::json!({"type": "string"}),
            required: true,
            sensitive: false,
            description: None,
            scope: AutoDeriveScope::WorkflowInput {
                name: "namespace".into(),
            },
            location: StepFieldLocation::Body,
        }
    }

    fn make_three_step_workflow() -> CompiledWorkflow {
        let svc = ServiceId::new("svc");

        // Step s1: GET /svc/v1/namespaces/{namespace}/name
        // auto-derives `namespace` from workflow input; captures `name1` from $.name
        let s1 = CompiledStep {
            id: "s1".into(),
            index: 0,
            description: Some("get name".into()),
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: svc.clone(),
                operation: OperationId::new("GetName"),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![step_capture("name1", "$.name")],
            auto_derived: vec![namespace_auto_derived()],
        };

        // Step s2: GET /svc/v1/namespaces/{namespace}/id
        // auto-derives `namespace` from workflow input; captures `id2` from $.id
        let s2 = CompiledStep {
            id: "s2".into(),
            index: 1,
            description: Some("get id".into()),
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: svc.clone(),
                operation: OperationId::new("GetId"),
            }),
            dependencies: vec!["s1".into()],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![step_capture("id2", "$.id")],
            auto_derived: vec![namespace_auto_derived()],
        };

        // Step s3: GET /svc/v1/things/{name}/{id}
        // `name` bound from s1/name1, `id` bound from s2/id2
        // captures `final_value` from $.final
        let s3 = CompiledStep {
            id: "s3".into(),
            index: 2,
            description: Some("get thing".into()),
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: svc,
                operation: OperationId::new("GetThing"),
            }),
            dependencies: vec!["s1".into(), "s2".into()],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![
                step_binding("name", "s1", "name1"),
                step_binding("id", "s2", "id2"),
            ],
            outputs: vec![step_capture("final_value", "$.final")],
            auto_derived: vec![],
        };

        CompiledWorkflow {
            id: WorkflowId::new("three-step-wf"),
            name: "three step workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![WorkflowInputSpec {
                name: "namespace".into(),
                description: None,
                schema: Some(serde_json::json!({"type": "string"})),
                required: true,
                default: None,
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
                file_picker: None,
            }],
            is_reviewed_by_default: true,
            steps: vec![s1, s2, s3],
            outputs: vec![WorkflowOutputAlias {
                name: "result".into(),
                from_step_id: "s3".into(),
                output: "final_value".into(),
                sensitive: false,
                section: None,
                label: None,
                item_fields: None,
            }],
            completion: None,
        }
    }

    fn make_runtime() -> crate::runtime::Runtime {
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(QueuedClient::new(vec![
                ok_json(r#"{"name": "X"}"#),
                ok_json(r#"{"id": "42"}"#),
                ok_json(r#"{"final": "ok"}"#),
            ])),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("svc", make_service_schema());
        runtime
    }

    /// Full 3-step integration test: namespace pre-supplied, each step captures
    /// its response field, s3 binds from s1/s2 outputs, and the workflow output
    /// alias `result` resolves to `"ok"`.
    ///
    /// Captures resolve against the raw response body (real API field names),
    /// so JSONPath `$.<field>` reads the response directly regardless of the
    /// presentation shaping.
    #[tokio::test]
    async fn test_three_step_workflow_drives_alias_map_and_summaries() {
        let compiled = make_three_step_workflow();
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            output_format: OutputFormat::Json,
            ..Default::default()
        };
        let mut runtime = make_runtime();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("dev"));

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context)
                .await
                .unwrap();

        // 1. Run outcome.
        assert_eq!(outcome, RunOutcome::Success);
        assert!(
            pending.is_none(),
            "expected no pending error, got {pending:?}"
        );

        // 2. Final output: CommandOutput::Workflow with the alias map populated.
        match &final_output {
            Some(CommandOutput::Workflow {
                workflow_id,
                outputs,
                step_summaries,
                completion: _,
                output_view: _,
            }) => {
                assert_eq!(workflow_id.as_str(), "three-step-wf");
                assert_eq!(
                    outputs.get("result"),
                    Some(&serde_json::json!("ok")),
                    "alias 'result' must resolve to \"ok\", outputs={outputs:?}"
                );
                assert_eq!(
                    step_summaries.len(),
                    3,
                    "expected 3 step summaries, got {step_summaries:?}"
                );
            }
            other => panic!("expected Some(Workflow {{ .. }}), got {other:?}"),
        }

        // 3. Event sequence: WorkflowStarted, three StepStarted/StepFinished{Success}
        //    pairs in order, then WorkflowFinished{Success}.
        let events = &frontend.events;

        let pos = |pred: fn(&WorkflowEvent) -> bool| {
            events
                .iter()
                .position(pred)
                .expect("expected event not found in stream")
        };

        let wf_started = pos(|e| matches!(e, WorkflowEvent::WorkflowStarted { .. }));
        let s1_started = pos(|e| matches!(e, WorkflowEvent::StepStarted { id, .. } if id == "s1"));
        let s1_finished = pos(
            |e| matches!(e, WorkflowEvent::StepFinished { id, outcome: StepOutcome::Success, .. } if id == "s1"),
        );
        let s2_started = pos(|e| matches!(e, WorkflowEvent::StepStarted { id, .. } if id == "s2"));
        let s2_finished = pos(
            |e| matches!(e, WorkflowEvent::StepFinished { id, outcome: StepOutcome::Success, .. } if id == "s2"),
        );
        let s3_started = pos(|e| matches!(e, WorkflowEvent::StepStarted { id, .. } if id == "s3"));
        let s3_finished = pos(
            |e| matches!(e, WorkflowEvent::StepFinished { id, outcome: StepOutcome::Success, .. } if id == "s3"),
        );
        let wf_finished = pos(|e| {
            matches!(
                e,
                WorkflowEvent::WorkflowFinished {
                    outcome: RunOutcome::Success,
                    ..
                }
            )
        });

        assert!(
            wf_started < s1_started,
            "WorkflowStarted must precede StepStarted{{s1}}"
        );
        assert!(
            s1_started < s1_finished,
            "StepStarted{{s1}} must precede StepFinished{{s1}}"
        );
        assert!(
            s1_finished < s2_started,
            "StepFinished{{s1}} must precede StepStarted{{s2}}"
        );
        assert!(
            s2_started < s2_finished,
            "StepStarted{{s2}} must precede StepFinished{{s2}}"
        );
        assert!(
            s2_finished < s3_started,
            "StepFinished{{s2}} must precede StepStarted{{s3}}"
        );
        assert!(
            s3_started < s3_finished,
            "StepStarted{{s3}} must precede StepFinished{{s3}}"
        );
        assert!(
            s3_finished < wf_finished,
            "StepFinished{{s3}} must precede WorkflowFinished"
        );

        // 4. Namespace pre-supplied — gather must never fire.
        assert_eq!(
            frontend.gather_call_count, 0,
            "gather must not be called when namespace is pre-supplied"
        );
    }

    /// `WorkflowFinished.facts` must aggregate every step's outcome across a
    /// multi-step run: one `steps_started` and one `steps_succeeded` per step,
    /// and `last_step_index` pinned to the final step's index.
    #[tokio::test]
    async fn test_workflow_finished_counts_each_step_outcome() {
        let compiled = make_three_step_workflow();
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            output_format: OutputFormat::Json,
            ..Default::default()
        };
        let mut runtime = make_runtime();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("dev"));

        let (_outcome, _final_output, _pending) =
            Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context)
                .await
                .unwrap();

        let facts = frontend
            .events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::WorkflowFinished { facts, .. } => Some(facts.clone()),
                _ => None,
            })
            .expect("WorkflowFinished must be emitted");

        assert_eq!(facts.steps_started, 3);
        assert_eq!(facts.steps_succeeded, 3);
        assert_eq!(facts.steps_failed, 0);
        assert_eq!(facts.steps_skipped, 0);
        assert_eq!(facts.steps_cancelled, 0);
        assert_eq!(facts.last_step_index, Some(2));
    }

    /// Build and run a two-step workflow where step "skip-me" is
    /// `continue_on_failure` and always fails at dispatch (becoming
    /// `Skipped`), and step "succeed-me" always succeeds. Returns the
    /// `RunFacts` carried by the run's `WorkflowFinished` event — the fixture
    /// behind the outcome-count sum-invariant test, which guards against
    /// `skip_step` and `finish_step_terminal` both tallying the same skip.
    async fn run_two_step_skip_then_success() -> ags_protocol::workflow::RunFacts {
        let svc = "skip-svc";
        let op_skip = "OpSkip";
        let op_succeed = "OpSucceed";

        let step_skip = CompiledStep {
            id: "skip-me".into(),
            index: 0,
            description: None,
            kind: ags_protocol::workflow::StepKind::Api,
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new(svc),
                operation: OperationId::new(op_skip),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: true,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![],
            auto_derived: vec![],
        };

        let step_succeed = CompiledStep {
            id: "succeed-me".into(),
            index: 1,
            description: None,
            kind: ags_protocol::workflow::StepKind::Api,
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new(svc),
                operation: OperationId::new(op_succeed),
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
        };

        let compiled = CompiledWorkflow {
            id: WorkflowId::new("skip-then-success-wf"),
            name: "skip then success workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![step_skip, step_succeed],
            outputs: vec![],
            completion: None,
        };

        let op_skip_schema = make_op(op_skip, "/skip", &[]);
        let op_succeed_schema = make_op(op_succeed, "/succeed", &[]);
        let service_schema = ServiceSchema {
            name: svc.into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "skip".into(),
                description: String::new(),
                methods: vec![
                    MethodSchema {
                        name: "op-skip".into(),
                        summary: String::new(),
                        default_scope: None,
                        scopes: vec![ScopeEntry {
                            scope: String::new(),
                            default_version: ApiVersion(1),
                            contracts: vec![op_skip_schema],
                        }],
                    },
                    MethodSchema {
                        name: "op-succeed".into(),
                        summary: String::new(),
                        default_scope: None,
                        scopes: vec![ScopeEntry {
                            scope: String::new(),
                            default_version: ApiVersion(1),
                            contracts: vec![op_succeed_schema],
                        }],
                    },
                ],
            }],
        };

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(QueuedClient::new(vec![
                Err(RuntimeError::internal("simulated dispatch failure")),
                ok_json(r#"{"ok": true}"#),
            ])),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests(svc, service_schema);

        let mut frontend = MockFrontend::new();
        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (_outcome, _final_output, _pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        frontend
            .events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::WorkflowFinished { facts, .. } => Some(facts.clone()),
                _ => None,
            })
            .expect("WorkflowFinished must be emitted")
    }

    /// The four outcome counters must sum to `steps_started` and, critically,
    /// a `continue_on_failure` skip must be tallied exactly once — proving
    /// `skip_step` does not also tally on top of `finish_step_terminal`.
    #[tokio::test]
    async fn test_run_facts_outcome_counts_sum_to_steps_started() {
        let facts = run_two_step_skip_then_success().await;

        assert_eq!(facts.steps_started, 2);
        assert_eq!(
            facts.steps_skipped, 1,
            "the skipped step must be tallied exactly once, not double-counted"
        );
        assert_eq!(facts.steps_succeeded, 1);
        assert_eq!(facts.steps_failed, 0);
        assert_eq!(facts.steps_cancelled, 0);
        assert_eq!(
            facts.steps_succeeded
                + facts.steps_failed
                + facts.steps_skipped
                + facts.steps_cancelled,
            facts.steps_started,
            "outcome counts must sum to steps_started"
        );
    }

    /// Shared log of `(url, query)` pairs recorded by `RecordingClient`.
    type RecordedRequests = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<(String, String)>)>>>;

    /// An `HttpClient` that records the requests it is handed, so a test can
    /// assert on the resolved URL and query string. Mirrors `QueuedClient` but
    /// keeps a copy of each request's `url` and `query`.
    struct RecordingClient {
        recorded: RecordedRequests,
        responses: std::sync::Mutex<Vec<Result<HttpResponse, RuntimeError>>>,
    }

    impl RecordingClient {
        fn new(responses: Vec<Result<HttpResponse, RuntimeError>>) -> Self {
            Self {
                recorded: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                responses: std::sync::Mutex::new(responses),
            }
        }
    }

    // The `end_to_end` module already does `use async_trait::async_trait;`; the
    // `HttpClient` trait is async-trait based, so this attribute is required (the
    // sibling `QueuedClient` impl carries it too). Omitting it will not compile.
    #[async_trait]
    impl HttpClient for RecordingClient {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            self.recorded
                .lock()
                .unwrap()
                .push((request.url.clone(), request.query.clone()));
            self.responses.lock().unwrap().remove(0)
        }
    }

    /// Like `make_op`, but the trailing `query_params` are declared as required
    /// query-string parameters (string-typed) in addition to the path params.
    fn make_op_query(
        id: &str,
        path: &str,
        path_params: &[&str],
        query_params: &[&str],
    ) -> OperationSchema {
        let mut parameters: Vec<ParameterSchema> = path_params
            .iter()
            .map(|name| ParameterSchema {
                name: (*name).into(),
                location: ParameterLocation::Path,
                required: true,
                value_type: ValueType::String,
                is_file: false,
                description: None,
                default: None,
            })
            .collect();
        for name in query_params {
            parameters.push(ParameterSchema {
                name: (*name).into(),
                location: ParameterLocation::Query,
                required: true,
                value_type: ValueType::String,
                is_file: false,
                description: None,
                default: None,
            });
        }
        OperationSchema {
            id: OperationId::new(id),
            name: id.to_lowercase(),
            summary: id.into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: path.into(),
            parameters,
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// Service with two ops: `MakeStore` (path `namespace`, captures storeId) and
    /// `MakeCategory` (path `namespace`, required query `storeId`).
    fn make_query_service_schema() -> ServiceSchema {
        let op_store = make_op(
            "MakeStore",
            "/svc/v1/namespaces/{namespace}/stores",
            &["namespace"],
        );
        let op_cat = make_op_query(
            "MakeCategory",
            "/svc/v1/namespaces/{namespace}/categories",
            &["namespace"],
            &["storeId"],
        );
        ServiceSchema {
            name: "svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "things".into(),
                description: String::new(),
                methods: vec![
                    MethodSchema {
                        name: "make-store".into(),
                        summary: String::new(),
                        default_scope: None,
                        scopes: vec![ScopeEntry {
                            scope: String::new(),
                            default_version: ApiVersion(1),
                            contracts: vec![op_store],
                        }],
                    },
                    MethodSchema {
                        name: "make-category".into(),
                        summary: String::new(),
                        default_scope: None,
                        scopes: vec![ScopeEntry {
                            scope: String::new(),
                            default_version: ApiVersion(1),
                            contracts: vec![op_cat],
                        }],
                    },
                ],
            }],
        }
    }

    #[tokio::test]
    async fn test_step_capture_resolves_into_query_param() {
        let svc = ServiceId::new("svc");

        // s1: create the store, capture storeId from the response body.
        let s1 = CompiledStep {
            id: "create-store".into(),
            index: 0,
            description: Some("make store".into()),
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: svc.clone(),
                operation: OperationId::new("MakeStore"),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![step_capture("storeId", "$.storeId")],
            auto_derived: vec![namespace_auto_derived()],
        };

        // s2: create the category, binding storeId into the QUERY param.
        let s2 = CompiledStep {
            id: "create-category".into(),
            index: 1,
            description: Some("make category".into()),
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: svc,
                operation: OperationId::new("MakeCategory"),
            }),
            dependencies: vec!["create-store".into()],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![step_binding("storeId", "create-store", "storeId")],
            outputs: vec![],
            auto_derived: vec![namespace_auto_derived()],
        };

        let compiled = CompiledWorkflow {
            id: WorkflowId::new("query-capture-wf"),
            name: "query capture".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![WorkflowInputSpec {
                name: "namespace".into(),
                description: None,
                schema: Some(serde_json::json!({"type": "string"})),
                required: true,
                default: None,
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
                file_picker: None,
            }],
            is_reviewed_by_default: true,
            steps: vec![s1, s2],
            outputs: vec![],
            completion: None,
        };

        let client = RecordingClient::new(vec![
            ok_json(r#"{"storeId": "store-abc"}"#),
            ok_json(r#"{}"#),
        ]);
        let recorded = client.recorded.clone();

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(client),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("svc", make_query_service_schema());

        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            output_format: OutputFormat::Json,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("dev"));

        let (outcome, _final_output, pending) =
            Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success, "pending={pending:?}");

        // The second request (category) must carry the captured storeId as a
        // query parameter, proving a step capture resolves into a query field.
        let calls = recorded.lock().unwrap();
        assert_eq!(calls.len(), 2, "two dispatches expected");
        let (_cat_url, cat_query) = &calls[1];
        assert!(
            cat_query
                .iter()
                .any(|(k, v)| k == "storeId" && v == "store-abc"),
            "category request query must contain storeId=store-abc, got {cat_query:?}"
        );
    }
}

#[cfg(test)]
mod executor_input_overrides {
    //! Verifies that `GatherResult.input_overrides` written by a gather
    //! call are merged back into `workflow_supplied` so that later steps
    //! (and the current step's own request assembly) see the edited values.

    use std::collections::BTreeMap;

    use async_trait::async_trait;

    use super::*;
    use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::workflows::executor::{Executor, RunContext};
    use ags_protocol::catalogue::{
        ApiVersion, HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema,
        ParameterLocation, ParameterSchema, ResourceSchema, ScopeEntry, ServiceId, ServiceSchema,
        ValueType,
    };
    use ags_protocol::output::CommandOutput;
    use ags_protocol::workflow::{
        AutoDeriveScope, AutoDerivedField, CompiledStep, CompiledWorkflow, GatherSlotId,
        OperationReference, StepFieldLocation, WorkflowId, WorkflowInputSpec,
    };

    /// Scripted HTTP client that always succeeds with status 200.
    struct ScriptedClient;

    #[async_trait]
    impl HttpClient for ScriptedClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            Ok(HttpResponse {
                status: 200,
                body: HttpBody::Text(r#"{"id": 1}"#.to_string()),
            })
        }
    }

    /// Service schema where the single operation has a `{namespace}` path
    /// parameter and a `{extra}` path parameter, so both can be supplied from
    /// `workflow_supplied`.
    fn make_service_schema() -> ServiceSchema {
        let operation = OperationSchema {
            id: OperationId::new("GetItems"),
            name: "list".into(),
            summary: "List items".into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/namespaces/{namespace}/items/{extra}".into(),
            parameters: vec![
                ParameterSchema {
                    name: "namespace".into(),
                    location: ParameterLocation::Path,
                    required: true,
                    value_type: ValueType::String,
                    is_file: false,
                    description: None,
                    default: None,
                },
                ParameterSchema {
                    name: "extra".into(),
                    location: ParameterLocation::Path,
                    required: true,
                    value_type: ValueType::String,
                    is_file: false,
                    description: None,
                    default: None,
                },
            ],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        ServiceSchema {
            name: "test-svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "list".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![operation],
                    }],
                }],
            }],
        }
    }

    /// Build a runtime with the scripted client and the test service schema
    /// pre-loaded.
    fn make_runtime() -> crate::runtime::Runtime {
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(ScriptedClient),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("test-svc", make_service_schema());
        runtime
    }

    /// A 1-step workflow with two workflow inputs: `namespace` (pre-supplied)
    /// and `extra` (missing → triggers gather). The step auto-derives both.
    ///
    /// When gather fires for `extra`, the mock also returns an `input_override`
    /// that replaces the pre-supplied `namespace` with "edited". The dry-run
    /// URL of the step must then contain `namespaces/edited/items/` confirming
    /// the override propagated into `workflow_supplied` before request assembly.
    #[tokio::test]
    async fn test_input_overrides_applied_before_request_assembly() {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("override-wf"),
            name: "override workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![
                WorkflowInputSpec {
                    name: "namespace".into(),
                    description: None,
                    schema: Some(serde_json::json!({"type": "string"})),
                    required: true,
                    default: None,
                    sensitive: false,
                    options_source: None,
                    location: ags_protocol::workflow::StepFieldLocation::Body,
                    file_picker: None,
                },
                WorkflowInputSpec {
                    name: "extra".into(),
                    description: None,
                    schema: Some(serde_json::json!({"type": "string"})),
                    required: true,
                    default: None,
                    sensitive: false,
                    options_source: None,
                    location: ags_protocol::workflow::StepFieldLocation::Body,
                    file_picker: None,
                },
            ],
            is_reviewed_by_default: true,
            steps: vec![CompiledStep {
                id: "get-items".into(),
                index: 0,
                description: None,
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new("test-svc"),
                    operation: OperationId::new("GetItems"),
                }),
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
                auto_derived: vec![
                    AutoDerivedField {
                        field: "namespace".into(),
                        schema: serde_json::json!({"type": "string"}),
                        required: true,
                        sensitive: false,
                        description: None,
                        scope: AutoDeriveScope::WorkflowInput {
                            name: "namespace".into(),
                        },
                        location: StepFieldLocation::Body,
                    },
                    AutoDerivedField {
                        field: "extra".into(),
                        schema: serde_json::json!({"type": "string"}),
                        required: true,
                        sensitive: false,
                        description: None,
                        scope: AutoDeriveScope::WorkflowInput {
                            name: "extra".into(),
                        },
                        location: StepFieldLocation::Body,
                    },
                ],
            }],
            outputs: vec![],
            completion: None,
        };

        // `namespace` is pre-supplied; `extra` is absent, so gather fires.
        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("orig"));

        // The gather call returns `extra = "some-val"` via slot_values, and
        // overrides `namespace` to "edited" via input_overrides.
        let mut slot_values: BTreeMap<GatherSlotId, serde_json::Value> = BTreeMap::new();
        // Slot 0 is assigned by `compute_needed_inputs` for the first missing
        // slot in the step; `extra` is the only missing input.
        slot_values.insert(GatherSlotId(0), serde_json::json!("some-val"));

        let mut overrides: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        overrides.insert("namespace".to_string(), serde_json::json!("edited"));

        let mut frontend = MockFrontend::new()
            .with_gather_response(slot_values)
            .with_input_overrides(overrides);

        let options = RunOptions {
            dry_run: true,
            ..Default::default()
        };
        let mut runtime = make_runtime();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);
        assert!(pending.is_none());
        assert_eq!(
            frontend.gather_call_count, 1,
            "gather must be called exactly once for the missing `extra` input"
        );

        // The dry-run URL must contain the overridden namespace "edited", not
        // the pre-supplied value "orig".
        match final_output {
            Some(CommandOutput::DryRun(ref dry)) => {
                assert!(
                    dry.url.contains("edited"),
                    "dry-run URL must contain the overridden namespace 'edited'; got: {}",
                    dry.url
                );
                assert!(
                    !dry.url.contains("orig"),
                    "dry-run URL must NOT contain the original namespace 'orig'; got: {}",
                    dry.url
                );
            }
            other => panic!("expected Some(DryRun(_)), got {other:?}"),
        }
    }
}

#[cfg(test)]
mod executor_review {
    use std::collections::BTreeMap;

    use async_trait::async_trait;

    use super::*;
    use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::workflows::executor::{Executor, RunContext};
    use ags_protocol::catalogue::{
        ApiVersion, BodyField, BodyFieldType, BodySchema, HttpMethod, MethodSchema, MutationClass,
        OperationId, OperationSchema, ParameterLocation, ParameterSchema, ResourceSchema,
        ScopeEntry, ServiceId, ServiceSchema, ValueType,
    };
    use ags_protocol::output::CommandOutput;
    use ags_protocol::workflow::{
        BindingSource, CompiledStep, CompiledWorkflow, OperationReference, ReferenceBinding,
        ReferenceTarget, StepFieldEdits, StepFieldPlan, StepInputBinding, StepReviewOutcome,
        WorkflowId, WorkflowInputSpec,
    };

    struct ScriptedClient;

    #[async_trait]
    impl HttpClient for ScriptedClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            Ok(HttpResponse {
                status: 200,
                body: HttpBody::Text(r#"{"id": 1}"#.to_string()),
            })
        }
    }

    /// A frontend that records each step's reviewed `namespace` value and, on
    /// step 1 (index 0), edits `namespace` to a configured value. Used to prove
    /// the executor applies edits and propagates a workflow-input edit to a
    /// later step.
    struct RecordingReviewFrontend {
        edit_namespace_to: String,
        review_calls: usize,
        step2_namespace_seen: Option<String>,
    }

    impl RecordingReviewFrontend {
        /// Build a fixture that edits the `namespace` input to the given value.
        fn editing_namespace_to(value: &str) -> Self {
            Self {
                edit_namespace_to: value.to_string(),
                review_calls: 0,
                step2_namespace_seen: None,
            }
        }
    }

    impl WorkflowFrontend for RecordingReviewFrontend {
        fn gather_workflow_inputs(
            &mut self,
            _needed: &[WorkflowInputNeeded],
            _step_context: &CompiledStep,
            _supplied: &[ags_protocol::workflow::SuppliedInputView],
        ) -> Result<ags_protocol::workflow::GatherResult, RuntimeError> {
            Ok(ags_protocol::workflow::GatherResult::default())
        }

        fn confirm_step(
            &mut self,
            _step: &CompiledStep,
            _preview: &StepPreview,
        ) -> Result<ags_protocol::workflow::StepConfirmOutcome, RuntimeError> {
            Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
        }

        fn review_step(&mut self, plan: &StepFieldPlan) -> Result<StepReviewOutcome, RuntimeError> {
            self.review_calls += 1;
            let mut edits = StepFieldEdits::default();
            if let Some(ns) = plan.fields.iter().find(|f| f.field == "namespace") {
                if plan.step_index == 1 {
                    self.step2_namespace_seen = ns.value.as_str().map(|s| s.to_string());
                }
                if plan.step_index == 0 {
                    edits
                        .values
                        .insert(ns.id, serde_json::json!(self.edit_namespace_to));
                }
            }
            Ok(StepReviewOutcome::Proceed(edits))
        }
    }

    fn namespaced_operation() -> OperationSchema {
        OperationSchema {
            id: OperationId::new("GetItems"),
            name: "list".into(),
            summary: "List items".into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/namespaces/{namespace}/items".into(),
            parameters: vec![ParameterSchema {
                name: "namespace".into(),
                location: ParameterLocation::Path,
                required: true,
                value_type: ValueType::String,
                is_file: false,
                description: None,
                default: None,
            }],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        }
    }

    fn service_schema() -> ServiceSchema {
        ServiceSchema {
            name: "test-svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "list".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![namespaced_operation()],
                    }],
                }],
            }],
        }
    }

    fn step(id: &str, index: usize) -> CompiledStep {
        CompiledStep {
            id: id.into(),
            index,
            description: Some(format!("step {index}")),
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new("test-svc"),
                operation: OperationId::new("GetItems"),
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

    /// Like [`step`] but binds `namespace` from the workflow input with
    /// `show_in_review: true`, so the step has a reviewable field and the
    /// executor actually calls `review_step`. Without a reviewable field the
    /// review-skip gate (`plan_has_reviewable_fields`) auto-dispatches the step,
    /// which is correct for all-auto-bound steps but means these edit-machinery
    /// tests would never see a review pause.
    fn reviewable_step(id: &str, index: usize) -> CompiledStep {
        CompiledStep {
            inputs: vec![StepInputBinding {
                field: "namespace".into(),
                source: BindingSource::Reference(ReferenceBinding {
                    from: ReferenceTarget::Workflow {
                        input: "namespace".into(),
                    },
                    output: None,
                    transform: None,
                }),
                show_in_review: true,
                description: None,
            }],
            ..step(id, index)
        }
    }

    /// Two steps that both send the `namespace` path param (a declared workflow
    /// input). Run with `namespace` pre-supplied as `dev`.
    async fn run_two_step_namespace_workflow(
        frontend: &mut RecordingReviewFrontend,
        review_steps: bool,
    ) -> Result<(RunOutcome, Option<CommandOutput>, Option<RuntimeError>), RuntimeError> {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("test-wf"),
            name: "test workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![WorkflowInputSpec {
                name: "namespace".into(),
                description: None,
                schema: Some(serde_json::json!({"type": "string"})),
                required: true,
                default: None,
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
                file_picker: None,
            }],
            is_reviewed_by_default: true,
            steps: vec![
                reviewable_step("step-one", 0),
                reviewable_step("step-two", 1),
            ],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(ScriptedClient),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("test-svc", service_schema());

        let options = RunOptions {
            review_steps,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("dev"));

        Executor::execute(&compiled, pre_supplied, frontend, &mut run_context).await
    }

    #[tokio::test]
    async fn test_review_steps_edits_are_step_local_only() {
        // A frontend that tries to edit a workflow-input-backed field is ignored
        // for propagation: the field is read-only, so the executor only ever
        // applies step-local edits. The mock still returns the edit; the executor
        // simply does not propagate it to later steps.
        let mut frontend = RecordingReviewFrontend::editing_namespace_to("prod");
        let outcome = run_two_step_namespace_workflow(&mut frontend, true).await;
        assert!(outcome.is_ok());
        assert_eq!(frontend.review_calls, 2);
        // step 2 still sees the original pre-supplied namespace, not "prod",
        // because workflow-input edits no longer propagate.
        assert_eq!(frontend.step2_namespace_seen.as_deref(), Some("dev"));
    }

    #[tokio::test]
    async fn test_review_steps_false_does_not_call_review_step() {
        let mut frontend = RecordingReviewFrontend::editing_namespace_to("prod");
        let _ = run_two_step_namespace_workflow(&mut frontend, false).await;
        assert_eq!(frontend.review_calls, 0);
    }

    /// Edits the step's "more body" overflow field to add `{"extra":"added"}`.
    struct OverflowEditFrontend;

    impl WorkflowFrontend for OverflowEditFrontend {
        fn gather_workflow_inputs(
            &mut self,
            _needed: &[WorkflowInputNeeded],
            _step_context: &CompiledStep,
            _supplied: &[ags_protocol::workflow::SuppliedInputView],
        ) -> Result<ags_protocol::workflow::GatherResult, RuntimeError> {
            Ok(ags_protocol::workflow::GatherResult::default())
        }

        fn confirm_step(
            &mut self,
            _step: &CompiledStep,
            _preview: &StepPreview,
        ) -> Result<ags_protocol::workflow::StepConfirmOutcome, RuntimeError> {
            Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
        }

        fn review_step(&mut self, plan: &StepFieldPlan) -> Result<StepReviewOutcome, RuntimeError> {
            let mut edits = StepFieldEdits::default();
            if let Some(overflow) = plan.fields.iter().find(|f| f.body_overflow) {
                edits
                    .values
                    .insert(overflow.id, serde_json::json!({"extra": "added"}));
            }
            Ok(StepReviewOutcome::Proceed(edits))
        }
    }

    fn service_schema_with_optional_body() -> ServiceSchema {
        let mut op = namespaced_operation();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            // Unbound + optional → folds into the "more body" overflow field.
            fields: vec![BodyField {
                name: "extra".into(),
                field_type: BodyFieldType::String,
                required: false,
                description: None,
                children: vec![],
                default: None,
            }],
        });
        ServiceSchema {
            name: "test-svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "list".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![op],
                    }],
                }],
            }],
        }
    }

    /// Phase-1 frontend that returns an authoritative input map dropping the
    /// optional `fleetName` (simulating the user clearing it in the Inputs form).
    struct InputsClearingFrontend;
    impl WorkflowFrontend for InputsClearingFrontend {
        fn gather_workflow_inputs(
            &mut self,
            _needed: &[WorkflowInputNeeded],
            _step: &CompiledStep,
            _supplied: &[ags_protocol::workflow::SuppliedInputView],
        ) -> Result<ags_protocol::workflow::GatherResult, RuntimeError> {
            Ok(ags_protocol::workflow::GatherResult::default())
        }
        fn confirm_step(
            &mut self,
            _s: &CompiledStep,
            _p: &StepPreview,
        ) -> Result<ags_protocol::workflow::StepConfirmOutcome, RuntimeError> {
            Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
        }
        #[allow(clippy::type_complexity)]
        fn collect_workflow_inputs(
            &mut self,
            _specs: &[WorkflowInputSpec],
            current: &BTreeMap<String, serde_json::Value>,
        ) -> Result<Option<ags_protocol::workflow::CollectOutcome>, RuntimeError> {
            // Authoritative: keep namespace, drop fleetName.
            let mut out = current.clone();
            out.remove("fleetName");
            Ok(Some(ags_protocol::workflow::CollectOutcome {
                inputs: out,
                run_mode: ags_protocol::workflow::RunMode::ReviewInputSteps,
            }))
        }
    }

    /// Service schema whose `list` operation has one optional `fleetName` body
    /// field. Because a body field whose name matches a declared workflow input
    /// auto-binds to that input, `fleetName` resolves from the `fleetName` input
    /// (no explicit StepInputBinding needed).
    fn service_schema_with_fleet_body() -> ServiceSchema {
        let mut op = namespaced_operation();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![BodyField {
                name: "fleetName".into(),
                field_type: BodyFieldType::String,
                required: false,
                description: None,
                children: vec![],
                default: None,
            }],
        });
        ServiceSchema {
            name: "test-svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "list".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![op],
                    }],
                }],
            }],
        }
    }

    /// 1-step dry-run workflow: declared inputs `namespace` (required, supplied
    /// "dev") and `fleetName` (optional, default "ranked-fleet"); `review_steps`
    /// on, so Phase 1 runs.
    async fn run_inputs_phase_workflow(
        mut frontend: InputsClearingFrontend,
    ) -> Result<(RunOutcome, Option<CommandOutput>, Option<RuntimeError>), RuntimeError> {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("test-wf"),
            name: "test workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![
                WorkflowInputSpec {
                    name: "namespace".into(),
                    description: None,
                    schema: Some(serde_json::json!({"type": "string"})),
                    required: true,
                    default: None,
                    sensitive: false,
                    options_source: None,
                    location: ags_protocol::workflow::StepFieldLocation::Body,
                    file_picker: None,
                },
                WorkflowInputSpec {
                    name: "fleetName".into(),
                    description: None,
                    schema: Some(serde_json::json!({"type": "string"})),
                    required: false,
                    default: Some(serde_json::json!("ranked-fleet")),
                    sensitive: false,
                    options_source: None,
                    location: ags_protocol::workflow::StepFieldLocation::Body,
                    file_picker: None,
                },
            ],
            is_reviewed_by_default: true,
            steps: vec![step("only", 0)],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(ScriptedClient),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("test-svc", service_schema_with_fleet_body());

        let options = RunOptions {
            review_steps: true,
            dry_run: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("dev"));

        Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context).await
    }

    #[tokio::test]
    async fn test_phase1_replace_unsets_cleared_optional() {
        // namespace (required, supplied) + fleetName (optional, default) declared.
        // Phase 1 drops fleetName → it must be absent from the assembled body.
        let outcome = run_inputs_phase_workflow(InputsClearingFrontend).await;
        assert!(outcome.is_ok());
        let (_, final_output, _) = outcome.unwrap();
        match final_output {
            Some(CommandOutput::DryRun(dry)) => {
                assert!(
                    match &dry.body {
                        Some(ags_protocol::request::RequestBody::Json(v)) => {
                            v.get("fleetName")
                        }
                        _ => None,
                    }
                    .is_none(),
                    "cleared optional must not appear in the request; body={:?}",
                    dry.body
                );
            }
            other => panic!("expected dry-run, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_review_body_overflow_edit_merges_into_request_body() {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("test-wf"),
            name: "test workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![WorkflowInputSpec {
                name: "namespace".into(),
                description: None,
                schema: Some(serde_json::json!({"type": "string"})),
                required: true,
                default: None,
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
                file_picker: None,
            }],
            is_reviewed_by_default: true,
            steps: vec![reviewable_step("only", 0)],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(ScriptedClient),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("test-svc", service_schema_with_optional_body());

        let options = RunOptions {
            review_steps: true,
            dry_run: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("dev"));

        let mut frontend = OverflowEditFrontend;
        let (outcome, final_output, _pending) =
            Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);
        match final_output {
            Some(CommandOutput::DryRun(dry)) => {
                let extra = match &dry.body {
                    Some(ags_protocol::request::RequestBody::Json(v)) => v.get("extra"),
                    _ => None,
                };
                assert_eq!(
                    extra,
                    Some(&serde_json::json!("added")),
                    "overflow JSON edit must merge into the request body; body={:?}",
                    dry.body
                );
            }
            other => panic!("expected a dry-run preview, got {other:?}"),
        }
    }

    /// Records the spec names Phase 1 collect receives, then cancels the run so
    /// no step dispatches — lets a test observe exactly which inputs the upfront
    /// gather would show for a given surface.
    struct RecordingInputsFrontend {
        recorded: Vec<String>,
    }
    impl WorkflowFrontend for RecordingInputsFrontend {
        fn gather_workflow_inputs(
            &mut self,
            _needed: &[WorkflowInputNeeded],
            _step: &CompiledStep,
            _supplied: &[ags_protocol::workflow::SuppliedInputView],
        ) -> Result<ags_protocol::workflow::GatherResult, RuntimeError> {
            unreachable!("run cancels at Phase 1 before per-step gather")
        }
        fn confirm_step(
            &mut self,
            _s: &CompiledStep,
            _p: &StepPreview,
        ) -> Result<ags_protocol::workflow::StepConfirmOutcome, RuntimeError> {
            unreachable!("run cancels at Phase 1 before confirm")
        }
        fn collect_workflow_inputs(
            &mut self,
            specs: &[WorkflowInputSpec],
            _current: &BTreeMap<String, serde_json::Value>,
        ) -> Result<Option<ags_protocol::workflow::CollectOutcome>, RuntimeError> {
            self.recorded = specs.iter().map(|s| s.name.clone()).collect();
            Ok(None) // cancel: we only care which specs Phase 1 received
        }
    }

    /// Run a player-overview-shaped workflow (namespace + userId bound by the
    /// step; userId is a picker whose options_source draws searchQuery/searchBy)
    /// through Phase 1 with the given `pickers_available`, returning the spec
    /// names the upfront gather received.
    async fn record_phase1_specs(pickers_available: bool) -> Vec<String> {
        use ags_protocol::workflow::{
            BindingSource, LabelDetail, OptionParameterBinding, OptionsSource, ReferenceBinding,
            ReferenceTarget, StepInputBinding,
        };
        let wf_ref = |input: &str| {
            BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: input.into(),
                },
                output: None,
                transform: None,
            })
        };
        let bind = |field: &str, source: BindingSource| StepInputBinding {
            field: field.into(),
            source,
            show_in_review: false,
            description: None,
        };
        let mut account = step("account", 0);
        account.inputs = vec![
            bind("namespace", wf_ref("namespace")),
            bind("userId", wf_ref("userId")),
        ];

        let input = |name: &str, options: Option<OptionsSource>| WorkflowInputSpec {
            name: name.into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: matches!(name, "namespace" | "searchQuery" | "userId"),
            default: None,
            sensitive: false,
            options_source: options,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        };
        let picker = OptionsSource {
            operation: OperationReference {
                service: ServiceId::new("iam"),
                operation: OperationId::new("iam/admin/users/v3/search"),
            },
            parameters: BTreeMap::from([
                (
                    "namespace".to_string(),
                    OptionParameterBinding::FromInput("namespace".into()),
                ),
                (
                    "query".to_string(),
                    OptionParameterBinding::FromInput("searchQuery".into()),
                ),
                (
                    "by".to_string(),
                    OptionParameterBinding::FromInput("searchBy".into()),
                ),
            ]),
            items_path: "$.data".into(),
            value: "$.userId".into(),
            label: None,
            label_detail: Some(LabelDetail::ByInput {
                input: "searchBy".into(),
                paths: BTreeMap::from([("emailAddress".into(), "$.emailAddress".into())]),
            }),
            fallback_description: None,
            filter: None,
        };
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("picker-wf"),
            name: "picker workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![
                input("namespace", None),
                input("searchBy", None),
                input("searchQuery", None),
                input("userId", Some(picker)),
            ],
            is_reviewed_by_default: true,
            steps: vec![account],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(ScriptedClient),
            reqwest::Client::new(),
        );
        let options = RunOptions {
            review_steps: true,
            pickers_available,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);
        let mut frontend = RecordingInputsFrontend {
            recorded: Vec::new(),
        };
        let (outcome, _out, _pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();
        assert_eq!(
            outcome,
            RunOutcome::Cancelled,
            "frontend cancels at Phase 1"
        );
        frontend.recorded
    }

    #[tokio::test]
    async fn test_phase1_drops_picker_support_inputs_when_no_picker() {
        // Plain/inline (no picker): searchBy/searchQuery are picker-support and
        // must not reach the upfront form. namespace + userId (step-bound) remain.
        let names = record_phase1_specs(false).await;
        assert_eq!(names, vec!["namespace", "userId"]);
    }

    #[tokio::test]
    async fn test_phase1_keeps_all_inputs_when_picker_available() {
        // Fullscreen (picker): unchanged — all four inputs reach the upfront
        // form, picker deps ordered before the picker.
        let names = record_phase1_specs(true).await;
        assert_eq!(
            names,
            vec!["namespace", "searchBy", "searchQuery", "userId"]
        );
    }
}

#[cfg(test)]
mod workflow_frontend_defaults {
    use super::*;

    #[test]
    fn test_present_briefing_default_returns_ok_true() {
        struct Mock;
        impl WorkflowFrontend for Mock {
            fn on_event(&mut self, _e: &WorkflowEvent) {}
            fn gather_workflow_inputs(
                &mut self,
                _: &[ags_protocol::workflow::WorkflowInputNeeded],
                _: &ags_protocol::workflow::CompiledStep,
                _: &[ags_protocol::workflow::SuppliedInputView],
            ) -> Result<ags_protocol::workflow::GatherResult, ags_protocol::error::RuntimeError>
            {
                Ok(ags_protocol::workflow::GatherResult::default())
            }
            fn confirm_step(
                &mut self,
                _: &ags_protocol::workflow::CompiledStep,
                _: &ags_protocol::workflow::StepPreview,
            ) -> Result<ags_protocol::workflow::StepConfirmOutcome, ags_protocol::error::RuntimeError>
            {
                Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
            }
        }
        let mut m = Mock;
        let b = ags_protocol::workflow::WorkflowBriefing {
            overview: "x".into(),
            prerequisites: vec![],
            creates: vec![],
        };
        assert!(m.present_briefing(&b, "wf").unwrap());
    }
}

#[cfg(test)]
mod executor_briefing {
    use std::sync::{Arc, Mutex};

    use ags_protocol::error::RuntimeError as ProtoRuntimeError;
    use ags_protocol::workflow::{
        CompiledStep, CompiledWorkflow, GatherResult, StepPreview, SuppliedInputView,
        WorkflowBriefing, WorkflowId, WorkflowInputNeeded, WorkflowInputSpec,
    };

    use crate::runtime::workflows::executor::{Executor, RunContext};
    use crate::runtime::workflows::{RunOptions, RunOutcome, WorkflowEvent, WorkflowFrontend};

    use super::executor_happy_path::make_one_step_workflow;
    use super::executor_skeleton::test_runtime;

    #[derive(Default)]
    struct Recorder {
        calls: Vec<String>,
        step_started_count: usize,
        /// Aggregate carried by `WorkflowFinished`, for the run-level
        /// cancellation-stage assertions.
        run_facts: Option<ags_protocol::workflow::RunFacts>,
    }

    struct RecordingFrontend {
        recorder: Arc<Mutex<Recorder>>,
        briefing_reply: Option<Result<bool, ProtoRuntimeError>>,
    }

    impl WorkflowFrontend for RecordingFrontend {
        fn on_event(&mut self, e: &WorkflowEvent) {
            let label = match e {
                WorkflowEvent::WorkflowStarted { .. } => "WorkflowStarted",
                WorkflowEvent::StepStarted { .. } => {
                    self.recorder.lock().unwrap().step_started_count += 1;
                    "StepStarted"
                }
                WorkflowEvent::StepFinished { .. } => "StepFinished",
                WorkflowEvent::WorkflowFinished { facts, .. } => {
                    self.recorder.lock().unwrap().run_facts = Some(facts.clone());
                    "WorkflowFinished"
                }
                WorkflowEvent::Progress { .. } => "Progress",
            };
            self.recorder.lock().unwrap().calls.push(label.into());
        }
        fn gather_workflow_inputs(
            &mut self,
            _: &[WorkflowInputNeeded],
            _: &CompiledStep,
            _: &[SuppliedInputView],
        ) -> Result<GatherResult, ProtoRuntimeError> {
            Ok(GatherResult::default())
        }
        fn confirm_step(
            &mut self,
            _: &CompiledStep,
            _: &StepPreview,
        ) -> Result<ags_protocol::workflow::StepConfirmOutcome, ProtoRuntimeError> {
            Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
        }
        fn present_briefing(
            &mut self,
            _b: &WorkflowBriefing,
            _name: &str,
        ) -> Result<bool, ProtoRuntimeError> {
            self.recorder
                .lock()
                .unwrap()
                .calls
                .push("present_briefing".into());
            self.briefing_reply.take().unwrap_or(Ok(true))
        }
    }

    fn compiled_with_briefing_and_no_steps() -> CompiledWorkflow {
        CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "WF".into(),
            intent: None,
            description: None,
            briefing: Some(WorkflowBriefing {
                overview: "x".into(),
                prerequisites: vec![],
                creates: vec![],
            }),
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![],
            outputs: vec![],
            completion: None,
        }
    }

    #[tokio::test]
    async fn test_executor_calls_present_briefing_before_step_started() {
        let recorder = Arc::new(Mutex::new(Recorder::default()));
        let mut frontend = RecordingFrontend {
            recorder: Arc::clone(&recorder),
            briefing_reply: None,
        };
        let compiled = compiled_with_briefing_and_no_steps();
        let options = RunOptions::default();
        let mut runtime = test_runtime();
        let mut ctx = RunContext::new(&mut runtime, &options);
        let _ = Executor::execute(&compiled, Default::default(), &mut frontend, &mut ctx).await;
        let calls = recorder.lock().unwrap().calls.clone();
        assert_eq!(
            calls,
            vec![
                "WorkflowStarted".to_string(),
                "present_briefing".to_string(),
                "WorkflowFinished".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn test_executor_briefing_cancel_sets_run_outcome_cancelled() {
        let recorder = Arc::new(Mutex::new(Recorder::default()));
        let mut frontend = RecordingFrontend {
            recorder: Arc::clone(&recorder),
            briefing_reply: Some(Ok(false)),
        };
        let mut compiled = make_one_step_workflow();
        compiled.briefing = Some(WorkflowBriefing {
            overview: "x".into(),
            prerequisites: vec![],
            creates: vec![],
        });
        let options = RunOptions::default();
        let mut runtime = test_runtime();
        let mut ctx = RunContext::new(&mut runtime, &options);
        let (outcome, _output, pending) =
            Executor::execute(&compiled, Default::default(), &mut frontend, &mut ctx)
                .await
                .expect("executor");

        assert_eq!(outcome, RunOutcome::Cancelled);
        assert!(pending.is_none());
        assert_eq!(recorder.lock().unwrap().step_started_count, 0);
    }

    /// A declined briefing emits no step event at all, so `RunFacts.reason` is
    /// the only place the stage can be reported — without it the run reaches
    /// PostHog as an unexplained bare `cancelled`.
    #[tokio::test]
    async fn test_briefing_decline_reports_at_briefing_in_run_facts() {
        let facts = run_facts_from(RunOptions::default(), Some(Ok(false))).await;
        assert_eq!(
            facts.reason,
            Some(ags_protocol::workflow::StepOutcomeReason::AtBriefing)
        );
    }

    /// A briefing the user accepts leaves the run-level reason unset — the
    /// stage vocabulary must not leak onto runs that proceeded.
    #[tokio::test]
    async fn test_accepted_briefing_leaves_the_run_reason_unset() {
        let facts = run_facts_from(RunOptions::default(), Some(Ok(true))).await;
        assert_eq!(facts.reason, None);
    }

    /// Cancelling the run-start input gather (`collect_workflow_inputs`
    /// returning `Ok(None)`) is the second pre-step stage with no step event of
    /// its own, and must report `at_input_gather`.
    #[tokio::test]
    async fn test_input_gather_cancel_reports_at_input_gather_in_run_facts() {
        let recorder = Arc::new(Mutex::new(Recorder::default()));
        let mut frontend = CancellingGatherFrontend {
            recorder: Arc::clone(&recorder),
        };
        let compiled = compiled_with_briefing_and_no_steps();
        let options = RunOptions {
            review_steps: true,
            ..Default::default()
        };
        let mut runtime = test_runtime();
        let mut ctx = RunContext::new(&mut runtime, &options);
        Executor::execute(&compiled, Default::default(), &mut frontend, &mut ctx)
            .await
            .expect("executor");

        let facts = recorder
            .lock()
            .unwrap()
            .run_facts
            .clone()
            .expect("WorkflowFinished must be emitted");
        assert_eq!(
            facts.reason,
            Some(ags_protocol::workflow::StepOutcomeReason::AtInputGather)
        );
    }

    /// Run a briefed one-step workflow with the given options and briefing
    /// reply, returning the aggregate its `WorkflowFinished` carried.
    async fn run_facts_from(
        options: RunOptions,
        briefing_reply: Option<Result<bool, ProtoRuntimeError>>,
    ) -> ags_protocol::workflow::RunFacts {
        let recorder = Arc::new(Mutex::new(Recorder::default()));
        let mut frontend = RecordingFrontend {
            recorder: Arc::clone(&recorder),
            briefing_reply,
        };
        let mut compiled = make_one_step_workflow();
        compiled.briefing = Some(WorkflowBriefing {
            overview: "x".into(),
            prerequisites: vec![],
            creates: vec![],
        });
        let mut runtime = test_runtime();
        let mut ctx = RunContext::new(&mut runtime, &options);
        Executor::execute(&compiled, Default::default(), &mut frontend, &mut ctx)
            .await
            .expect("executor");
        let facts = recorder.lock().unwrap().run_facts.clone();
        facts.expect("WorkflowFinished must be emitted")
    }

    /// Frontend that accepts the briefing and then cancels the run-start
    /// gather, for the `at_input_gather` stage.
    struct CancellingGatherFrontend {
        recorder: Arc<Mutex<Recorder>>,
    }

    impl WorkflowFrontend for CancellingGatherFrontend {
        fn on_event(&mut self, e: &WorkflowEvent) {
            if let WorkflowEvent::WorkflowFinished { facts, .. } = e {
                self.recorder.lock().unwrap().run_facts = Some(facts.clone());
            }
        }
        fn gather_workflow_inputs(
            &mut self,
            _: &[WorkflowInputNeeded],
            _: &CompiledStep,
            _: &[SuppliedInputView],
        ) -> Result<GatherResult, ProtoRuntimeError> {
            Ok(GatherResult::default())
        }
        fn confirm_step(
            &mut self,
            _: &CompiledStep,
            _: &StepPreview,
        ) -> Result<ags_protocol::workflow::StepConfirmOutcome, ProtoRuntimeError> {
            Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
        }
        fn collect_workflow_inputs(
            &mut self,
            _: &[WorkflowInputSpec],
            _: &std::collections::BTreeMap<String, serde_json::Value>,
        ) -> Result<Option<ags_protocol::workflow::CollectOutcome>, ProtoRuntimeError> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn test_executor_briefing_cancel_skips_phase_1_when_review_steps() {
        // briefing returns Ok(false) AND review_steps is true → Phase 1
        // (collect_workflow_inputs) must not be called.
        struct PhaseOneCounter {
            recorder: Arc<Mutex<Recorder>>,
            briefing_reply: Option<Result<bool, ProtoRuntimeError>>,
        }
        impl WorkflowFrontend for PhaseOneCounter {
            fn on_event(&mut self, _e: &WorkflowEvent) {}
            fn gather_workflow_inputs(
                &mut self,
                _: &[WorkflowInputNeeded],
                _: &CompiledStep,
                _: &[SuppliedInputView],
            ) -> Result<GatherResult, ProtoRuntimeError> {
                Ok(GatherResult::default())
            }
            fn confirm_step(
                &mut self,
                _: &CompiledStep,
                _: &StepPreview,
            ) -> Result<ags_protocol::workflow::StepConfirmOutcome, ProtoRuntimeError> {
                Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
            }
            fn collect_workflow_inputs(
                &mut self,
                _: &[WorkflowInputSpec],
                current: &std::collections::BTreeMap<String, serde_json::Value>,
            ) -> Result<Option<ags_protocol::workflow::CollectOutcome>, ProtoRuntimeError>
            {
                self.recorder
                    .lock()
                    .unwrap()
                    .calls
                    .push("collect_workflow_inputs".into());
                Ok(Some(ags_protocol::workflow::CollectOutcome {
                    inputs: current.clone(),
                    run_mode: ags_protocol::workflow::RunMode::ReviewInputSteps,
                }))
            }
            fn present_briefing(
                &mut self,
                _b: &WorkflowBriefing,
                _name: &str,
            ) -> Result<bool, ProtoRuntimeError> {
                self.recorder
                    .lock()
                    .unwrap()
                    .calls
                    .push("present_briefing".into());
                self.briefing_reply.take().unwrap_or(Ok(true))
            }
        }

        let recorder = Arc::new(Mutex::new(Recorder::default()));
        let mut frontend = PhaseOneCounter {
            recorder: Arc::clone(&recorder),
            briefing_reply: Some(Ok(false)),
        };
        let compiled = compiled_with_briefing_and_no_steps();
        let options = RunOptions {
            review_steps: true,
            ..Default::default()
        };
        let mut runtime = test_runtime();
        let mut ctx = RunContext::new(&mut runtime, &options);
        let _ = Executor::execute(&compiled, Default::default(), &mut frontend, &mut ctx).await;
        let calls = recorder.lock().unwrap().calls.clone();
        assert!(
            calls.contains(&"present_briefing".to_string()),
            "present_briefing should fire; calls: {calls:?}"
        );
        assert!(
            !calls.contains(&"collect_workflow_inputs".to_string()),
            "collect_workflow_inputs must NOT fire after briefing cancel; calls: {calls:?}"
        );
    }
}

#[cfg(test)]
mod executor_optional {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::workflows::executor::{Executor, RunContext};
    use ags_protocol::catalogue::{
        ApiVersion, HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema,
        ResourceSchema, ScopeEntry, ServiceId, ServiceSchema,
    };
    use ags_protocol::output::CommandOutput;
    use ags_protocol::workflow::{
        CaptureSource, CompiledStep, CompiledWorkflow, OperationReference, StepOutputCapture,
        WorkflowId, WorkflowOutputAlias,
    };

    /// HTTP client backed by a FIFO queue; each `send` pops the front entry.
    struct QueuedClient {
        responses: Arc<Mutex<Vec<Result<HttpResponse, RuntimeError>>>>,
    }

    impl QueuedClient {
        fn new(responses: Vec<Result<HttpResponse, RuntimeError>>) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses)),
            }
        }
    }

    #[async_trait]
    impl HttpClient for QueuedClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            self.responses.lock().unwrap().remove(0)
        }
    }

    fn ok_response(body: &str) -> Result<HttpResponse, RuntimeError> {
        Ok(HttpResponse {
            status: 200,
            body: HttpBody::Text(body.to_string()),
        })
    }

    fn err_response() -> Result<HttpResponse, RuntimeError> {
        Err(RuntimeError::internal("simulated transport failure"))
    }

    /// Build and run a two-step workflow where the first step ("a") always
    /// fails at dispatch. `continue_on_failure` controls whether step "a" is
    /// failure-tolerant. Step "b" always succeeds.
    ///
    /// Step "a" declares a capture `a_value` (path `$.value`, default
    /// `"unavailable"`). The workflow exposes `a_value` as a top-level output
    /// alias so it lands in `CommandOutput::Workflow.outputs`.
    async fn run_two_step_with_failing_first(
        continue_on_failure: bool,
    ) -> (RunOutcome, Option<CommandOutput>) {
        let svc = "opt-svc";
        let op_a = "OpA";
        let op_b = "OpB";

        let capture_a = StepOutputCapture {
            name: "a_value".into(),
            source: CaptureSource::ResponseBody {
                path: "$.value".into(),
            },
            default: Some(serde_json::json!("unavailable")),
            sensitive: false,
        };

        let step_a = CompiledStep {
            id: "a".into(),
            index: 0,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new(svc),
                operation: OperationId::new(op_a),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![capture_a],
            auto_derived: vec![],
        };

        let step_b = CompiledStep {
            id: "b".into(),
            index: 1,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new(svc),
                operation: OperationId::new(op_b),
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
        };

        let compiled = CompiledWorkflow {
            id: WorkflowId::new("opt-wf"),
            name: "optional step workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![step_a, step_b],
            // Expose step "a"'s capture as a workflow output so the test can
            // assert the default landed in CommandOutput::Workflow.outputs.
            outputs: vec![WorkflowOutputAlias {
                name: "a_value".into(),
                from_step_id: "a".into(),
                output: "a_value".into(),
                sensitive: false,
                section: None,
                label: None,
                item_fields: None,
            }],
            completion: None,
        };

        // Step "a" → transport error; step "b" → success.
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(QueuedClient::new(vec![
                err_response(),
                ok_response(r#"{"id": 99}"#),
            ])),
            reqwest::Client::new(),
        );
        // Register the same schema twice — op_a and op_b share a path template.
        // Use the same schema with both operation ids registered under the same
        // service; the simplest approach is to insert the schema once with two
        // operations by building a combined schema.
        let op_a_schema = OperationSchema {
            id: OperationId::new(op_a),
            name: "op-a".into(),
            summary: "Op A".into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/items".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        let op_b_schema = OperationSchema {
            id: OperationId::new(op_b),
            name: "op-b".into(),
            summary: "Op B".into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/items".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        let combined_schema = ServiceSchema {
            name: svc.into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![
                    MethodSchema {
                        name: "op-a".into(),
                        summary: String::new(),
                        default_scope: None,
                        scopes: vec![ScopeEntry {
                            scope: String::new(),
                            default_version: ApiVersion(1),
                            contracts: vec![op_a_schema],
                        }],
                    },
                    MethodSchema {
                        name: "op-b".into(),
                        summary: String::new(),
                        default_scope: None,
                        scopes: vec![ScopeEntry {
                            scope: String::new(),
                            default_version: ApiVersion(1),
                            contracts: vec![op_b_schema],
                        }],
                    },
                ],
            }],
        };
        runtime
            .catalogue_mut()
            .insert_for_tests(svc, combined_schema);

        let mut frontend = MockFrontend::new();
        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, _pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        (outcome, final_output)
    }

    /// Failure-tolerant step whose dispatch fails must be recorded as `Skipped`,
    /// have its capture default bound, and leave `RunOutcome::Success`; the
    /// second step runs and the workflow completes normally.
    #[tokio::test]
    async fn test_optional_step_dispatch_failure_is_skipped_and_run_continues() {
        let (outcome, output) =
            run_two_step_with_failing_first(/* continue_on_failure: */ true).await;
        assert_eq!(outcome, RunOutcome::Success);
        // a's capture default must have landed in the workflow outputs.
        let CommandOutput::Workflow { outputs, .. } = output.unwrap() else {
            panic!("expected CommandOutput::Workflow")
        };
        assert_eq!(
            outputs.get("a_value"),
            Some(&serde_json::json!("unavailable"))
        );
    }

    /// Non-failure-tolerant step whose dispatch fails must still halt the run.
    #[tokio::test]
    async fn test_non_optional_step_dispatch_failure_still_halts() {
        let (outcome, _) = run_two_step_with_failing_first(/* continue_on_failure: */ false).await;
        assert_eq!(outcome, RunOutcome::Failed);
    }

    /// Build and run a single `continue_on_failure` step whose one dispatch
    /// answers with `response`, returning the run outcome and every event the
    /// run emitted. Callers pass a transport error or a real HTTP failure
    /// status depending on which error facts the case is about.
    async fn run_continue_on_failure_workflow(
        response: Result<HttpResponse, RuntimeError>,
    ) -> (RunOutcome, Vec<WorkflowEvent>) {
        let svc = "cof-svc";
        let op_id = "CofOp";

        let compiled = CompiledWorkflow {
            id: WorkflowId::new("cof-wf"),
            name: "cof workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![CompiledStep {
                id: "cof-step".into(),
                index: 0,
                description: None,
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new(svc),
                    operation: OperationId::new(op_id),
                }),
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: true,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
                auto_derived: vec![],
            }],
            outputs: vec![],
            completion: None,
        };

        let op_schema = OperationSchema {
            id: OperationId::new(op_id),
            name: "cof-op".into(),
            summary: "Cof Op".into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/cof".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        let service_schema = ServiceSchema {
            name: svc.into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "cof".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "cof-op".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![op_schema],
                    }],
                }],
            }],
        };

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(QueuedClient::new(vec![response])),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests(svc, service_schema);

        let mut frontend = MockFrontend::new();
        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, _final_output, _pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        (outcome, frontend.events)
    }

    /// A `continue_on_failure` step whose dispatch fails must be recorded as
    /// `Skipped` with a `"{id} skipped — {reason}"` summary, and the run must
    /// finish `Success`. Regression guard for the renamed field and the
    /// skip-summary format.
    #[tokio::test]
    async fn test_continue_on_failure_step_skips_and_continues() {
        let (outcome, events) = run_continue_on_failure_workflow(err_response()).await;

        // The run must succeed despite the dispatch failure.
        assert_eq!(
            outcome,
            RunOutcome::Success,
            "continue_on_failure must not halt the run"
        );

        // The step must be recorded Skipped with the "{id} skipped — {reason}" summary.
        let (step_outcome, summary) = events
            .iter()
            .find_map(|e| {
                if let WorkflowEvent::StepFinished {
                    id,
                    outcome,
                    summary,
                    ..
                } = e
                {
                    if id == "cof-step" {
                        return Some((*outcome, summary.clone()));
                    }
                }
                None
            })
            .expect("cof-step must appear in StepFinished events");

        assert_eq!(
            step_outcome,
            StepOutcome::Skipped,
            "cof-step must be Skipped, not {step_outcome:?}"
        );
        assert!(
            summary.starts_with("cof-step skipped — "),
            "summary must follow '{{id}} skipped — {{reason}}' format, got: {summary:?}"
        );
    }

    /// A `continue_on_failure` skip must carry the typed `ToleratedFailure`
    /// telemetry reason, distinct from the other skip causes.
    #[tokio::test]
    async fn test_continue_on_failure_step_emits_tolerated_failure_reason() {
        let (_outcome, events) = run_continue_on_failure_workflow(err_response()).await;

        let reason = events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::StepFinished {
                    outcome: StepOutcome::Skipped,
                    reason,
                    ..
                } => Some(*reason),
                _ => None,
            })
            .expect("a skipped StepFinished must be emitted");
        assert_eq!(
            reason,
            Some(ags_protocol::workflow::StepOutcomeReason::ToleratedFailure)
        );
    }

    /// A skip is a response to a real failure, so it must carry the same error
    /// triple a fatal outcome would: a `tolerated_failure` skip of an upstream
    /// 503 reports the status and class, not an empty error slot.
    #[tokio::test]
    async fn test_tolerated_failure_skip_carries_the_error_triple() {
        let (_outcome, events) = run_continue_on_failure_workflow(Ok(HttpResponse {
            status: 503,
            body: HttpBody::Text(r#"{"errorCode":20013,"errorMessage":"down"}"#.to_string()),
        }))
        .await;

        let error = events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::StepFinished {
                    outcome: StepOutcome::Skipped,
                    error,
                    ..
                } => Some(error.clone()),
                _ => None,
            })
            .expect("a skipped StepFinished must be emitted")
            .expect("a failure-driven skip must carry error facts");
        assert_eq!(error.http_status, Some(503));
        assert_eq!(error.class, "upstream");
        assert_eq!(error.code.as_deref(), Some("20013"));
    }
}

#[cfg(test)]
mod executor_per_step_review {
    use std::collections::BTreeMap;

    use async_trait::async_trait;

    use super::*;
    use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::workflows::executor::{resolved_step_review, Executor, RunContext};
    use ags_protocol::catalogue::{
        ApiVersion, HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema,
        ResourceSchema, ScopeEntry, ServiceId, ServiceSchema,
    };
    use ags_protocol::workflow::{CompiledStep, CompiledWorkflow, OperationReference, WorkflowId};

    struct ScriptedClient {
        body: String,
    }

    #[async_trait]
    impl HttpClient for ScriptedClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            Ok(HttpResponse {
                status: 200,
                body: HttpBody::Text(self.body.clone()),
            })
        }
    }

    fn make_service_schema() -> ServiceSchema {
        let operation = OperationSchema {
            id: OperationId::new("GetItems"),
            name: "list".into(),
            summary: "List items".into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/items".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        ServiceSchema {
            name: "test-svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "list".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![operation],
                    }],
                }],
            }],
        }
    }

    fn make_runtime(body: &str) -> crate::runtime::Runtime {
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(ScriptedClient {
                body: body.to_string(),
            }),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("test-svc", make_service_schema());
        runtime
    }

    /// Build a `CompiledWorkflow` with the given `is_reviewed_by_default` and
    /// no steps; used as a fixture for the pure unit test of the helper.
    fn compiled_with_default_review(is_reviewed_by_default: bool) -> CompiledWorkflow {
        CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "wf".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default,
            steps: vec![],
            outputs: vec![],
            completion: None,
        }
    }

    /// Build a `CompiledStep` with the given `is_reviewed` override.
    fn step_review(is_reviewed: Option<bool>) -> CompiledStep {
        CompiledStep {
            id: "s0".into(),
            index: 0,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new("svc"),
                operation: OperationId::new("op"),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed,
            inputs: vec![],
            outputs: vec![],
            auto_derived: vec![],
        }
    }

    /// Run a 1-step, fully-bound workflow with `review_steps: true` and the
    /// given `is_reviewed_by_default`. Returns the number of times the mock
    /// frontend's `review_step` was called.
    async fn run_single_bound_step_count_review_calls(default_review: bool) -> usize {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "wf".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: default_review,
            steps: vec![CompiledStep {
                id: "s0".into(),
                index: 0,
                description: None,
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new("test-svc"),
                    operation: OperationId::new("GetItems"),
                }),
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None, // inherits workflow default
                inputs: vec![],
                outputs: vec![],
                auto_derived: vec![],
            }],
            outputs: vec![],
            completion: None,
        };
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            review_steps: true,
            ..Default::default()
        };
        let mut runtime = make_runtime(r#"{"id": 1}"#);
        let mut run_context = RunContext::new(&mut runtime, &options);
        Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
            .await
            .unwrap();
        frontend.review_step_call_count
    }

    // ------------------------------------------------------------------ //
    // Test 1: resolved_step_review respects step override then default    //
    // ------------------------------------------------------------------ //

    /// `resolved_step_review` must prefer the step's `is_reviewed` override
    /// when present, and fall back to the workflow default when absent.
    #[test]
    fn test_resolved_step_review_prefers_step_then_workflow_default() {
        let wf_true = compiled_with_default_review(true);
        let wf_false = compiled_with_default_review(false);
        assert!(resolved_step_review(&step_review(None), &wf_true)); // inherit true
        assert!(!resolved_step_review(&step_review(None), &wf_false)); // inherit false
        assert!(!resolved_step_review(&step_review(Some(false)), &wf_true)); // override
        assert!(resolved_step_review(&step_review(Some(true)), &wf_false)); // override
    }

    // ------------------------------------------------------------------ //
    // Test 2: review walk skipped when resolved review is false           //
    // ------------------------------------------------------------------ //

    /// With `is_reviewed_by_default: false` and a fully-bound step, the
    /// executor must not call `review_step` even when `review_steps` is true.
    #[tokio::test]
    async fn test_review_walk_skipped_when_resolved_false() {
        // is_reviewed_by_default: false, fully-bound step → frontend.review_step
        // is NEVER called, step dispatches straight through.
        let calls = run_single_bound_step_count_review_calls(/* default_review */ false).await;
        assert_eq!(calls, 0);
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::*;
    use ags_protocol::output::{WorkflowOutputProvenance, WorkflowOutputView};
    use ags_protocol::workflow::{CaptureSource, StepOutputCapture, WorkflowOutputAlias};

    /// Build a `StepOutputCapture` with a `ResponseBody` source and the given
    /// default string value.
    fn cap(name: &str, path: &str, default: &str) -> StepOutputCapture {
        StepOutputCapture {
            name: name.into(),
            source: CaptureSource::ResponseBody { path: path.into() },
            default: Some(serde_json::json!(default)),
            sensitive: false,
        }
    }

    /// Build a `WorkflowOutputAlias` with no section or label.
    fn alias(name: &str, from_step_id: &str, output: &str) -> WorkflowOutputAlias {
        WorkflowOutputAlias {
            name: name.into(),
            from_step_id: from_step_id.into(),
            output: output.into(),
            sensitive: false,
            section: None,
            label: None,
            item_fields: None,
        }
    }

    /// Return the provenance of the named item in the view.
    fn prov(view: &WorkflowOutputView, name: &str) -> WorkflowOutputProvenance {
        view.items
            .iter()
            .find(|item| item.name == name)
            .unwrap_or_else(|| panic!("item {name:?} not found in view"))
            .provenance
            .clone()
    }

    /// Return the value of the named item in the view.
    fn val(view: &WorkflowOutputView, name: &str) -> serde_json::Value {
        view.items
            .iter()
            .find(|item| item.name == name)
            .unwrap_or_else(|| panic!("item {name:?} not found in view"))
            .value
            .clone()
    }

    /// `Captured` when the JSONPath resolved, `Missing` when the path missed
    /// and the default was used, `Skipped` when bound via `bind_skipped_outputs`.
    #[test]
    fn test_output_view_provenance_captured_missing_skipped() {
        let mut ctx = WorkflowContext::new();
        // captured: path resolves
        ctx.bind_step_outputs(
            "a",
            &[cap("x", "$.v", "def")],
            Some(&serde_json::json!({"v": 5})),
        )
        .unwrap();
        // missing: path absent, default used
        ctx.bind_step_outputs(
            "b",
            &[cap("y", "$.v", "def")],
            Some(&serde_json::json!({"other": 1})),
        )
        .unwrap();
        // skipped: step did not dispatch
        ctx.bind_skipped_outputs("c", &[cap("z", "$.v", "def")])
            .unwrap();
        let view = ctx.resolve_workflow_output_view(&[
            alias("x", "a", "x"),
            alias("y", "b", "y"),
            alias("z", "c", "z"),
        ]);
        use WorkflowOutputProvenance::*;
        assert_eq!(prov(&view, "x"), Captured);
        assert_eq!(prov(&view, "y"), Missing);
        assert_eq!(prov(&view, "z"), Skipped);
        assert_eq!(val(&view, "x"), serde_json::json!(5));
    }
}

#[cfg(test)]
pub(crate) mod builtin_workflow_e2e {
    //! End-to-end executor tests for built-in workflows using the full
    //! bundled-catalogue path. No production code is added by these tests.

    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::workflows::executor::{Executor, RunContext};
    use ags_protocol::error::RuntimeError;
    use ags_protocol::output::{CommandOutput, WorkflowOutputProvenance};

    /// HTTP client backed by a FIFO queue; each `send` pops the front entry.
    struct QueuedClient {
        responses: Arc<Mutex<Vec<Result<HttpResponse, RuntimeError>>>>,
    }

    impl QueuedClient {
        fn new(responses: Vec<Result<HttpResponse, RuntimeError>>) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses)),
            }
        }
    }

    #[async_trait]
    impl HttpClient for QueuedClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            self.responses.lock().unwrap().remove(0)
        }
    }

    fn ok_json(body: &str) -> Result<HttpResponse, RuntimeError> {
        Ok(HttpResponse {
            status: 200,
            body: HttpBody::Text(body.to_string()),
        })
    }

    fn response_404() -> Result<HttpResponse, RuntimeError> {
        Ok(HttpResponse {
            status: 404,
            body: HttpBody::Text(
                r#"{"errorCode": 404001, "errorMessage": "Not found"}"#.to_string(),
            ),
        })
    }

    /// Build and run `player-overview` against a scripted HTTP client where
    /// 10 of the 11 reads return 200 OK and the `inventory` read (step index 9)
    /// returns a 404 so it is skipped. All 4 required inputs are pre-supplied to
    /// avoid any gather/picker interaction.
    async fn run_player_overview_mocked() -> (RunOutcome, Option<CommandOutput>) {
        use crate::runtime::workflows::builtins::player_overview::PlayerOverview;
        use crate::runtime::workflows::compile::compile_workflow;

        let wf = PlayerOverview::new();
        let mut catalogue = crate::catalogue::Catalogue::new();
        let compiled = compile_workflow(wf.definition(), &mut catalogue)
            .expect("player-overview must compile against the bundled catalogue");

        // Step order: account (0), linked-platforms (1), bans (2), entitlements (3),
        // wallet (4), orders (5), stats (6), achievements (7), cloudsave (8),
        // inventory (9) → 404, reports (10).
        // All captures have defaults so any 200 OK body is acceptable. The
        // account body carries real fields so the test can assert a successful
        // read's capture resolves the real value (via the raw response body).
        let responses = vec![
            ok_json(
                r#"{"displayName":"Ada","emailAddress":"ada@example.com","country":"GB","enabled":true,"createdAt":"2020-01-01T00:00:00Z"}"#,
            ), // 0: account
            ok_json(r#"{"data": []}"#), // 1: linked-platforms
            ok_json(r#"{}"#),           // 2: bans
            ok_json(r#"{"data": []}"#), // 3: entitlements
            ok_json(r#"[]"#),           // 4: wallet (capture path "$")
            ok_json(r#"{"data": []}"#), // 5: orders
            ok_json(r#"{"data": []}"#), // 6: stats
            ok_json(r#"{"data": []}"#), // 7: achievements
            ok_json(r#"{"data": []}"#), // 8: cloudsave
            response_404(),             // 9: inventory → skipped
            ok_json(r#"{"data": []}"#), // 10: reports
        ];

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(QueuedClient::new(responses)),
            reqwest::Client::new(),
        );

        let mut frontend = MockFrontend::new();
        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("dev"));
        pre_supplied.insert("searchBy".to_string(), serde_json::json!("emailAddress"));
        pre_supplied.insert(
            "searchQuery".to_string(),
            serde_json::json!("test@example.com"),
        );
        pre_supplied.insert("userId".to_string(), serde_json::json!("user-abc123"));

        let (outcome, final_output, _pending) =
            Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context)
                .await
                .unwrap();

        (outcome, final_output)
    }

    /// Build and run `competitive-multiplayer` against a scripted HTTP client
    /// that returns 200 OK for all 6 API steps. All required inputs are pre-supplied.
    /// Stub the AMS upload endpoints and build a directory with a valid
    /// x86-64 ELF entrypoint, so the local upload step can run offline.
    ///
    /// Returned together because the temp directory must outlive the run.
    pub(crate) async fn stub_ams_upload() -> (wiremock::MockServer, tempfile::TempDir) {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let uri = server.uri();
        Mock::given(method("GET"))
            .and(path("/ams/v1/upload-url"))
            .respond_with(ResponseTemplate::new(200).set_body_string(uri.clone()))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/upload/v1/images"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": "img-abc123" })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/upload/v1/pre-sign-url"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "url": format!("{uri}/storage") })),
            )
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/storage"))
            .respond_with(ResponseTemplate::new(200).insert_header("ETag", "\"etag-1\""))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/upload/v1/complete"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let build = tempfile::tempdir().unwrap();
        let mut header = vec![0u8; 20];
        header[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        header[4] = 2;
        header[5] = 1;
        header[6] = 1;
        header[18..20].copy_from_slice(&62u16.to_le_bytes());
        std::fs::write(build.path().join("server"), &header).unwrap();
        (server, build)
    }

    async fn run_competitive_multiplayer_mocked() -> (RunOutcome, Option<CommandOutput>) {
        use crate::runtime::workflows::builtins::competitive_multiplayer::CompetitiveMultiplayer;
        use crate::runtime::workflows::compile::compile_workflow;

        let wf = CompetitiveMultiplayer::new();
        let mut catalogue = crate::catalogue::Catalogue::new();
        let compiled = compile_workflow(wf.definition(), &mut catalogue)
            .expect("competitive-multiplayer must compile against the bundled catalogue");

        // 6 CREATE/UPDATE steps — no captures, so any 200 OK body suffices.
        let responses = vec![
            ok_json(r#"{}"#), // 0: create-stat
            ok_json(r#"{}"#), // 1: create-ruleset
            ok_json(r#"{}"#), // 2: create-session-template
            ok_json(r#"{}"#), // 3: create-match-pool
            ok_json(r#"{}"#), // 4: create-ams-fleet
            ok_json(r#"{}"#), // 5: update-session-template
        ];

        // Stub the AMS endpoints: the local upload step uses the concrete reqwest
        // client rather than the injected `HttpClient` seam, so
        // `QueuedClient` cannot stand in for it.
        let (ams, build) = stub_ams_upload().await;
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: ams.uri(),
                ..Default::default()
            },
            Box::new(QueuedClient::new(responses)),
            reqwest::Client::new(),
        );

        let mut frontend = MockFrontend::new();
        // review_steps: false so the briefing proceed-gate does not block; the
        // MockFrontend present_briefing default returns Ok(true) anyway.
        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("dev"));
        pre_supplied.insert(
            "buildPath".to_string(),
            serde_json::json!(build.path().to_str().unwrap()),
        );
        pre_supplied.insert("buildExecutable".to_string(), serde_json::json!("server"));
        pre_supplied.insert("fleetRegion".to_string(), serde_json::json!("us-west-2"));
        pre_supplied.insert("fleetInstanceId".to_string(), serde_json::json!("c3.large"));

        let (outcome, final_output, _pending) =
            Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context)
                .await
                .unwrap();

        (outcome, final_output)
    }

    /// A failing local step must surface its `RuntimeError`, not a flattened
    /// summary line. The upload pipeline puts its most useful guidance in
    /// `details` and `hint` — the 403 permission fix and the orphaned-image
    /// notice — and a workflow that dropped them would leave the user with
    /// "upload-image failed" and nothing to act on.
    #[tokio::test]
    async fn test_local_step_failure_propagates_the_structured_error() {
        use crate::runtime::workflows::builtins::competitive_multiplayer::CompetitiveMultiplayer;
        use crate::runtime::workflows::compile::compile_workflow;

        let wf = CompetitiveMultiplayer::new();
        let mut catalogue = crate::catalogue::Catalogue::new();
        let compiled = compile_workflow(wf.definition(), &mut catalogue).unwrap();

        // The four steps before upload-image succeed; the upload then fails on
        // a build directory that does not exist, before any network call.
        let responses = vec![
            ok_json(r#"{}"#),
            ok_json(r#"{}"#),
            ok_json(r#"{}"#),
            ok_json(r#"{}"#),
        ];
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext::default(),
            Box::new(QueuedClient::new(responses)),
            reqwest::Client::new(),
        );
        let mut frontend = MockFrontend::new();
        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("dev"));
        pre_supplied.insert(
            "buildPath".to_string(),
            serde_json::json!("/nonexistent/build/directory"),
        );
        pre_supplied.insert("buildExecutable".to_string(), serde_json::json!("server"));
        pre_supplied.insert("fleetRegion".to_string(), serde_json::json!("us-west-2"));
        pre_supplied.insert("fleetInstanceId".to_string(), serde_json::json!("c3.large"));

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Failed);
        assert!(final_output.is_none());
        let error = pending.expect("a failed local step must yield a pending error");
        assert!(
            error.hint.is_some() || error.details.is_some(),
            "the structured guidance must survive: {error:?}"
        );
        assert!(
            error.message.contains("/nonexistent/build/directory"),
            "the error must name the directory: {}",
            error.message
        );
    }

    /// The executor's local-step `Err` branch must build `StepErrorFacts` from
    /// the underlying `RuntimeError` and attach it to the `StepFinished` event.
    /// This test drives the full `Executor::execute` path — not the helper — so
    /// reverting the branch to pass `None` instead of `Some(facts)` turns it red.
    #[tokio::test]
    async fn test_local_step_err_branch_attaches_error_facts_to_step_finished_event() {
        use crate::runtime::workflows::builtins::competitive_multiplayer::CompetitiveMultiplayer;
        use crate::runtime::workflows::compile::compile_workflow;

        let wf = CompetitiveMultiplayer::new();
        let mut catalogue = crate::catalogue::Catalogue::new();
        let compiled = compile_workflow(wf.definition(), &mut catalogue).unwrap();

        // The four API steps before the local upload-image step succeed;
        // the upload then fails because the build directory does not exist.
        let responses = vec![
            ok_json(r#"{}"#),
            ok_json(r#"{}"#),
            ok_json(r#"{}"#),
            ok_json(r#"{}"#),
        ];
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext::default(),
            Box::new(QueuedClient::new(responses)),
            reqwest::Client::new(),
        );
        let mut frontend = MockFrontend::new();
        let options = RunOptions::default();
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("dev"));
        pre_supplied.insert(
            "buildPath".to_string(),
            serde_json::json!("/nonexistent/build/directory"),
        );
        pre_supplied.insert("buildExecutable".to_string(), serde_json::json!("server"));
        pre_supplied.insert("fleetRegion".to_string(), serde_json::json!("us-west-2"));
        pre_supplied.insert("fleetInstanceId".to_string(), serde_json::json!("c3.large"));

        let (outcome, _final_output, _pending) =
            Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Failed);

        // Find the StepFinished event for the failed local step (upload-image).
        let step_finished_error = frontend
            .events
            .iter()
            .find_map(|event| {
                if let WorkflowEvent::StepFinished {
                    outcome: StepOutcome::Failed,
                    error,
                    ..
                } = event
                {
                    Some(error.clone())
                } else {
                    None
                }
            })
            .expect("a StepFinished event with Failed outcome must be emitted");

        let facts = step_finished_error
            .expect("the executor must attach StepErrorFacts to a local step failure");

        // DirectoryMissing maps to RuntimeErrorKind::Validation → class "usage".
        assert_eq!(
            facts.class, "usage",
            "error class must reflect the underlying RuntimeError kind, got '{}'",
            facts.class
        );
    }

    /// Optional `inventory` read (step 9) returns 404: the workflow still succeeds,
    /// the flat `outputs` map keeps the `inventory` key with its default `[]`, and
    /// the `output_view` item for `inventory` carries provenance `Skipped`.
    #[tokio::test]
    async fn test_player_overview_skipped_read_renders_unavailable_and_json_keeps_key() {
        // Mock: inventory dispatch 404s; the rest 200 with small bodies.
        let (outcome, output) = run_player_overview_mocked().await;
        assert_eq!(outcome, RunOutcome::Success);
        let CommandOutput::Workflow {
            outputs,
            output_view,
            ..
        } = output.unwrap()
        else {
            panic!("expected CommandOutput::Workflow")
        };
        // A successful read's capture resolves a real value from the raw
        // response body (regression guard: captures must read the raw API JSON,
        // not the presentation-shaped body where this would be `none`).
        assert_eq!(
            outputs.get("account_displayName"),
            Some(&serde_json::json!("Ada")),
            "successful-read capture must resolve the real field value"
        );
        // JSON (flat map): the skipped read's alias key is present with its default ([]).
        assert_eq!(outputs.get("inventory"), Some(&serde_json::json!([])));
        // View: that item is provenance Skipped, and the captured account field
        // is Captured (not Missing/Skipped).
        let v = output_view.unwrap();
        let inv = v.items.iter().find(|i| i.name == "inventory").unwrap();
        assert_eq!(inv.provenance, WorkflowOutputProvenance::Skipped);
        let acct = v
            .items
            .iter()
            .find(|i| i.name == "account_displayName")
            .unwrap();
        assert_eq!(acct.provenance, WorkflowOutputProvenance::Captured);
    }

    /// Backward-compat (P1b): a workflow whose aliases carry no section/label
    /// produces `output_view: None` and stays on flat rendering.
    #[tokio::test]
    async fn test_competitive_multiplayer_has_no_output_view() {
        let (_outcome, output) = run_competitive_multiplayer_mocked().await;
        let CommandOutput::Workflow { output_view, .. } = output.unwrap() else {
            panic!("expected CommandOutput::Workflow")
        };
        assert!(output_view.is_none());
    }
}

#[cfg(test)]
mod executor_skip {
    use std::collections::BTreeMap;

    use async_trait::async_trait;

    use super::*;
    use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::workflows::executor::{Executor, RunContext};
    use ags_protocol::catalogue::{
        ApiVersion, HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema,
        ResourceSchema, ScopeEntry, ServiceId, ServiceSchema,
    };
    use ags_protocol::workflow::{
        CaptureSource, CompiledStep, CompiledWorkflow, OperationReference, RunMode,
        StepOutputCapture, WorkflowId,
    };

    struct ScriptedClient {
        body: String,
    }

    #[async_trait]
    impl HttpClient for ScriptedClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            Ok(HttpResponse {
                status: 200,
                body: HttpBody::Text(self.body.clone()),
            })
        }
    }

    fn make_service_schema() -> ServiceSchema {
        let operation = OperationSchema {
            id: OperationId::new("skip-op"),
            name: "op".into(),
            summary: "Op".into(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/items".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        ServiceSchema {
            name: "skip-svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "items".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "op".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![operation],
                    }],
                }],
            }],
        }
    }

    fn make_runtime() -> crate::runtime::Runtime {
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(ScriptedClient {
                body: r#"{"id": 1}"#.to_string(),
            }),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("skip-svc", make_service_schema());
        runtime
    }

    struct RunResult {
        run_outcome: RunOutcome,
    }

    fn run_workflow_blocking(
        compiled: &CompiledWorkflow,
        frontend: &mut MockFrontend,
        options: RunOptions,
    ) -> RunResult {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut runtime = make_runtime();
        let mut run_context = RunContext::new(&mut runtime, &options);
        let (run_outcome, _final_output, _pending) = rt
            .block_on(Executor::execute(
                compiled,
                BTreeMap::new(),
                frontend,
                &mut run_context,
            ))
            .unwrap();
        RunResult { run_outcome }
    }

    fn make_step(
        id: &str,
        index: usize,
        confirm: bool,
        is_optional: bool,
        outputs: Vec<StepOutputCapture>,
    ) -> CompiledStep {
        CompiledStep {
            id: id.into(),
            index,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new("skip-svc"),
                operation: OperationId::new("skip-op"),
            }),
            dependencies: vec![],
            confirm,
            is_optional,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs,
            auto_derived: vec![],
        }
    }

    /// Two-step workflow; step 0 is optional + confirm and has an output with a
    /// default. Step 1 is a plain dispatching step.
    fn two_step_optional_confirm_workflow() -> CompiledWorkflow {
        let s0_capture = StepOutputCapture {
            name: "result".into(),
            source: CaptureSource::ResponseBody {
                path: "$.id".into(),
            },
            default: Some(serde_json::json!("fallback")),
            sensitive: false,
        };
        CompiledWorkflow {
            id: WorkflowId::new("skip-wf"),
            name: "skip workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: false,
            steps: vec![
                make_step("step-0", 0, true, true, vec![s0_capture]),
                make_step("step-1", 1, false, false, vec![]),
            ],
            outputs: vec![],
            completion: None,
        }
    }

    /// Two-step workflow; step 0 is optional + non-confirm and reviewed. Uses
    /// `is_reviewed_by_default: true` so the review gate fires. Step 1 is plain.
    fn two_step_optional_review_workflow() -> CompiledWorkflow {
        let s0_capture = StepOutputCapture {
            name: "result".into(),
            source: CaptureSource::ResponseBody {
                path: "$.id".into(),
            },
            default: Some(serde_json::json!("fallback")),
            sensitive: false,
        };
        CompiledWorkflow {
            id: WorkflowId::new("skip-review-wf"),
            name: "skip review workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![
                make_step("step-0", 0, false, true, vec![s0_capture]),
                make_step("step-1", 1, false, false, vec![]),
            ],
            outputs: vec![],
            completion: None,
        }
    }

    /// Two-step workflow; step 0 is confirm: true but is_optional: false, and its
    /// output has no default. Proves the defensive guard fires before any binding.
    fn two_step_confirm_not_optional_workflow() -> CompiledWorkflow {
        let s0_capture = StepOutputCapture {
            name: "result".into(),
            source: CaptureSource::ResponseBody {
                path: "$.id".into(),
            },
            default: None, // no default — proves guard fires before binding
            sensitive: false,
        };
        CompiledWorkflow {
            id: WorkflowId::new("guard-wf"),
            name: "guard workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: false,
            steps: vec![
                make_step("step-0", 0, true, false, vec![s0_capture]),
                make_step("step-1", 1, false, false, vec![]),
            ],
            outputs: vec![],
            completion: None,
        }
    }

    // ------------------------------------------------------------------ //
    // Test 1: confirm-gate skip                                           //
    // ------------------------------------------------------------------ //

    /// Two-step workflow; step 0 is optional + confirm. The user skips step 0.
    /// The run continues, step 0 is recorded `Skipped`, step 1 succeeds.
    #[test]
    fn test_user_skip_at_confirm_gate_skips_and_continues() {
        let compiled = two_step_optional_confirm_workflow();
        let mut frontend = MockFrontend {
            skip_confirm_step_ids: vec!["step-0".into()],
            ..Default::default()
        };
        let outcome = run_workflow_blocking(&compiled, &mut frontend, RunOptions::default());
        assert_eq!(outcome.run_outcome, RunOutcome::Success);
        let finished = frontend.finished_steps();
        assert!(
            finished.iter().any(|(id, outcome, summary)| {
                id == "step-0" && *outcome == StepOutcome::Skipped && summary == "step-0 skipped"
            }),
            "expected step-0 Skipped with summary 'step-0 skipped', got {finished:?}"
        );
        // Step 1 still ran, resolving the skipped output to its default.
        assert!(
            finished
                .iter()
                .any(|(id, outcome, _)| id == "step-1" && *outcome == StepOutcome::Success),
            "expected step-1 Success, got {finished:?}"
        );
    }

    // ------------------------------------------------------------------ //
    // Test 2: review-gate skip                                            //
    // ------------------------------------------------------------------ //

    /// Two-step workflow; step 0 is optional + non-confirm and reviewed. The user
    /// skips step 0 at the review gate. The run continues, step 1 succeeds.
    #[test]
    fn test_user_skip_at_review_gate_skips_and_continues() {
        let compiled = two_step_optional_review_workflow();
        // ReviewEveryStep ensures the gate fires even with no reviewable fields.
        let mut frontend = MockFrontend {
            skip_review_step_ids: vec!["step-0".into()],
            mock_collect_run_mode: RunMode::ReviewEveryStep,
            ..Default::default()
        };
        let options = RunOptions {
            review_steps: true,
            ..Default::default()
        };
        let outcome = run_workflow_blocking(&compiled, &mut frontend, options);
        assert_eq!(outcome.run_outcome, RunOutcome::Success);
        let finished = frontend.finished_steps();
        assert!(
            finished.iter().any(|(id, outcome, summary)| {
                id == "step-0" && *outcome == StepOutcome::Skipped && summary == "step-0 skipped"
            }),
            "expected step-0 Skipped with summary 'step-0 skipped', got {finished:?}"
        );
        assert!(
            finished
                .iter()
                .any(|(id, outcome, _)| id == "step-1" && *outcome == StepOutcome::Success),
            "expected step-1 Success, got {finished:?}"
        );
    }

    // ------------------------------------------------------------------ //
    // Test 3: non-optional Skip guard                                     //
    // ------------------------------------------------------------------ //

    /// A misbehaving frontend returns Skip for a non-optional step. The executor
    /// must not silently skip — it must fail the run with the step as Failed.
    #[test]
    fn test_frontend_skip_on_non_optional_step_fails_run() {
        // Step 0 is confirm: true but is_optional: false. A misbehaving frontend
        // returns Skip anyway.
        let compiled = two_step_confirm_not_optional_workflow();
        let mut frontend = MockFrontend {
            skip_confirm_step_ids: vec!["step-0".into()],
            ..Default::default()
        };
        let outcome = run_workflow_blocking(&compiled, &mut frontend, RunOptions::default());
        assert_eq!(outcome.run_outcome, RunOutcome::Failed);
        let finished = frontend.finished_steps();
        assert!(
            finished
                .iter()
                .any(|(id, outcome, _)| id == "step-0" && *outcome == StepOutcome::Failed),
            "expected step-0 Failed, got {finished:?}"
        );
        // Step 1 never ran.
        assert!(
            !finished.iter().any(|(id, _, _)| id == "step-1"),
            "step-1 must not run when step-0 fails, got {finished:?}"
        );
    }

    // ------------------------------------------------------------------ //
    // Test 4: optional non-confirm step reaches the review gate           //
    // ------------------------------------------------------------------ //

    /// Build a two-step workflow where step 0 is optional and non-confirm but NOT
    /// reviewed (`is_reviewed_by_default: false`, `is_reviewed: None`). It has no
    /// reviewable fields either. Before Task 3 the review gate would be skipped
    /// entirely; after Task 3 the force-pause path opens it so the user sees the
    /// skip affordance.
    fn two_step_optional_non_confirm_not_reviewed_workflow() -> CompiledWorkflow {
        let s0_capture = StepOutputCapture {
            name: "result".into(),
            source: CaptureSource::ResponseBody {
                path: "$.id".into(),
            },
            default: Some(serde_json::json!("fallback")),
            sensitive: false,
        };
        CompiledWorkflow {
            id: WorkflowId::new("optional-non-confirm-wf"),
            name: "optional non-confirm workflow".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: false,
            steps: vec![
                make_step("step-0", 0, false, true, vec![s0_capture]),
                make_step("step-1", 1, false, false, vec![]),
            ],
            outputs: vec![],
            completion: None,
        }
    }

    #[test]
    fn test_optional_non_confirm_step_reaches_review_gate() {
        // Step 0 is optional + non-confirm + not-reviewed (is_reviewed_by_default:
        // false). Without Task 3's fix the review gate never fired; with it the
        // force-pause path ensures review_step is called even with no reviewable
        // fields, so surfaces can render the skip affordance.
        let compiled = two_step_optional_non_confirm_not_reviewed_workflow();
        let mut frontend = MockFrontend::default();
        // MockFrontend.review_step defaults to Proceed, so the run completes.
        let options = RunOptions {
            review_steps: true,
            ..Default::default()
        };
        let outcome = run_workflow_blocking(&compiled, &mut frontend, options);
        assert_eq!(outcome.run_outcome, RunOutcome::Success);
        assert!(
            frontend.reviewed_step_ids.contains(&"step-0".to_string()),
            "review_step must be called for the optional non-confirm step; got {:?}",
            frontend.reviewed_step_ids
        );
    }
}

// =====================================================================
// Local step type — Test Plan cases 13-17
// =====================================================================

#[cfg(test)]
mod local_step_executor {
    use std::collections::BTreeMap;

    use async_trait::async_trait;

    use super::*;
    use crate::runtime::dispatch::http::{HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::workflows::executor::{Executor, RunContext};
    use ags_protocol::output::CommandOutput;
    use ags_protocol::workflow::{
        CaptureSource, CompiledStep, CompiledWorkflow, StepOutputCapture, WorkflowId,
        WorkflowOutputAlias,
    };

    struct NeverClient;

    #[async_trait]
    impl HttpClient for NeverClient {
        async fn send(&self, _: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            unreachable!("local step tests must not dispatch HTTP")
        }
    }

    /// Build a Runtime that panics on HTTP dispatch. Local steps do not use
    /// the HTTP client, so this proves no API call was made.
    fn make_runtime_never() -> crate::runtime::Runtime {
        crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(NeverClient),
            reqwest::Client::new(),
        )
    }

    /// Build a compiled local step using the `test-echo` action (available
    /// under `#[cfg(test)]`).
    fn local_step(id: &str, index: usize) -> CompiledStep {
        CompiledStep {
            id: id.into(),
            index,
            description: Some("test local step".into()),
            kind: ags_protocol::workflow::StepKind::Local,
            action: Some("test-echo".into()),
            operation: None,
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

    fn local_step_with_outputs(
        id: &str,
        index: usize,
        outputs: Vec<StepOutputCapture>,
    ) -> CompiledStep {
        CompiledStep {
            outputs,
            ..local_step(id, index)
        }
    }

    // -----------------------------------------------------------------
    // Test Plan case 13: --dry-run with a local step produces
    // CommandOutput::WorkflowDryRun and records zero subprocess
    // invocations (the NeverClient panics if HTTP is attempted).
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn test_dry_run_local_step_no_side_effect() {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf-local-dry"),
            name: "local dry-run".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![local_step("local-s1", 0), local_step("local-s2", 1)],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = make_runtime_never();
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            dry_run: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);
        assert!(pending.is_none());
        match &final_output {
            Some(CommandOutput::WorkflowDryRun { step_previews, .. }) => {
                assert_eq!(
                    step_previews.len(),
                    2,
                    "expected 2 dry-run previews for 2 local steps"
                );
            }
            other => panic!("expected Some(WorkflowDryRun{{..}}), got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Test Plan case 14: the dry-run preview for a local step carries
    // the action name in a `StepDryRunAction::Local` variant, and the
    // overall output is `WorkflowDryRun`.
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn test_dry_run_local_step_preview_contains_action_name() {
        use ags_protocol::workflow::StepDryRunAction;

        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf-local-dry-2"),
            name: "local dry-run 2".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![local_step("local-action-step", 0)],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = make_runtime_never();
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            dry_run: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (_outcome, final_output, _pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        match final_output {
            Some(CommandOutput::WorkflowDryRun { step_previews, .. }) => {
                assert_eq!(step_previews.len(), 1, "one local step => one preview");
                match &step_previews[0].action {
                    StepDryRunAction::Local { action, .. } => {
                        assert_eq!(
                            action, "test-echo",
                            "dry-run preview must carry the action name"
                        );
                    }
                    other => panic!("expected Local variant, got {other:?}"),
                }
            }
            other => panic!("expected Some(WorkflowDryRun(_)), got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Test Plan case 15: --format json envelope for a workflow containing
    // a local step is the existing CommandOutput::Workflow shape with an
    // "outputs" key.
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn test_json_envelope_workflow_with_local_step() {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf-local-json"),
            name: "local json envelope".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![local_step_with_outputs(
                "local-auth",
                0,
                vec![StepOutputCapture {
                    name: "token".into(),
                    source: CaptureSource::ResponseBody {
                        path: "$.token".into(),
                    },
                    default: Some(serde_json::json!("default-tok")),
                    sensitive: false,
                }],
            )],
            outputs: vec![WorkflowOutputAlias {
                name: "token".into(),
                from_step_id: "local-auth".into(),
                output: "token".into(),
                sensitive: false,
                section: None,
                label: None,
                item_fields: None,
            }],
            completion: None,
        };

        let mut runtime = make_runtime_never();
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            assume_yes: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, final_output, _) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);
        match final_output {
            Some(CommandOutput::Workflow {
                outputs,
                step_summaries,
                ..
            }) => {
                assert!(
                    outputs.contains_key("token"),
                    "outputs map must contain the 'token' alias: {outputs:?}"
                );
                assert!(
                    !step_summaries.is_empty(),
                    "step_summaries must be non-empty"
                );
            }
            other => panic!("expected Some(Workflow{{..}}), got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Test Plan case 16: --yes --no-input with a local step completes
    // without reading stdin. Local steps never have confirm: true
    // (validate_step_kinds rejects it at compile time).
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn test_yes_flag_does_not_block_at_local_step() {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf-yes"),
            name: "yes-flag test".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![local_step("local-no-confirm", 0)],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = make_runtime_never();
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            assume_yes: true,
            no_input: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, _final, _pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(
            outcome,
            RunOutcome::Success,
            "local step with --yes --no-input and confirm: false must succeed"
        );
    }

    // =================================================================
    // Local-step input binding tests
    // =================================================================

    /// HTTP client returning one canned JSON response per call.
    struct QueuedClient {
        responses: std::sync::Arc<std::sync::Mutex<Vec<Result<HttpResponse, RuntimeError>>>>,
    }

    impl QueuedClient {
        fn with_response(body: &str) -> Self {
            Self {
                responses: std::sync::Arc::new(std::sync::Mutex::new(vec![Ok(HttpResponse {
                    status: 200,
                    body: crate::runtime::dispatch::http::HttpBody::Text(body.to_string()),
                })])),
            }
        }
    }

    #[async_trait]
    impl crate::runtime::dispatch::http::HttpClient for QueuedClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            self.responses.lock().unwrap().remove(0)
        }
    }

    /// Build a compiled API step with a single path parameter and captures.
    fn api_step_with_capture(
        id: &str,
        index: usize,
        captures: Vec<StepOutputCapture>,
    ) -> CompiledStep {
        use ags_protocol::catalogue::{OperationId, ServiceId};
        use ags_protocol::workflow::{
            AutoDeriveScope, AutoDerivedField, OperationReference, StepFieldLocation,
        };
        CompiledStep {
            id: id.into(),
            index,
            description: Some("api step".into()),
            kind: ags_protocol::workflow::StepKind::Api,
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new("svc"),
                operation: OperationId::new("GetCred"),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: captures,
            auto_derived: vec![AutoDerivedField {
                field: "namespace".into(),
                schema: serde_json::json!({"type": "string"}),
                required: true,
                sensitive: false,
                description: None,
                scope: AutoDeriveScope::WorkflowInput {
                    name: "namespace".into(),
                },
                location: StepFieldLocation::Body,
            }],
        }
    }

    /// Build a minimal service schema for the api_step_with_capture helper.
    fn api_service_schema() -> ags_protocol::catalogue::ServiceSchema {
        use ags_protocol::catalogue::*;
        ServiceSchema {
            name: "svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "res".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: "get-cred".into(),
                    summary: String::new(),
                    default_scope: None,
                    scopes: vec![ScopeEntry {
                        scope: String::new(),
                        default_version: ApiVersion(1),
                        contracts: vec![OperationSchema {
                            id: OperationId::new("GetCred"),
                            name: "get-cred".into(),
                            summary: String::new(),
                            description: None,
                            mutation_class: MutationClass::ReadOnly,
                            http_method: HttpMethod::Get,
                            path_template: "/svc/v1/namespaces/{namespace}/cred".into(),
                            parameters: vec![ParameterSchema {
                                name: "namespace".into(),
                                location: ParameterLocation::Path,
                                required: true,
                                value_type: ValueType::String,
                                is_file: false,
                                description: None,
                                default: None,
                            }],
                            request_body: None,
                            response: None,
                            permissions: vec![],
                            scope: String::new(),
                            api_version: ApiVersion(1),
                            deprecated: false,
                            response_content_type: None,
                        }],
                    }],
                }],
            }],
        }
    }

    /// Build a local step that binds fields from a prior step's captured outputs.
    fn local_step_with_bindings(
        id: &str,
        index: usize,
        bindings: Vec<ags_protocol::workflow::StepInputBinding>,
    ) -> CompiledStep {
        CompiledStep {
            inputs: bindings,
            ..local_step(id, index)
        }
    }

    /// Shorthand: a step-output reference binding.
    fn step_ref(
        field: &str,
        step_id: &str,
        output: &str,
    ) -> ags_protocol::workflow::StepInputBinding {
        use ags_protocol::workflow::{
            BindingSource, ReferenceBinding, ReferenceTarget, StepInputBinding,
        };
        StepInputBinding {
            field: field.into(),
            source: BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Step { id: step_id.into() },
                output: Some(output.into()),
                transform: None,
            }),
            show_in_review: false,
            description: None,
        }
    }

    /// Shorthand: a literal binding.
    fn literal_binding(
        field: &str,
        value: serde_json::Value,
    ) -> ags_protocol::workflow::StepInputBinding {
        use ags_protocol::workflow::{BindingSource, LiteralBinding, StepInputBinding};
        StepInputBinding {
            field: field.into(),
            source: BindingSource::Literal(LiteralBinding {
                value,
                sensitive: false,
            }),
            show_in_review: false,
            description: None,
        }
    }

    // -----------------------------------------------------------------
    // Test 1: end-to-end — a captured value from an API step reaches
    // the local step handler through a declared binding.
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn test_local_step_receives_captured_value_from_api_step() {
        use ags_protocol::workflow::WorkflowInputSpec;

        // Step 1: API step returning {"token": "secret-abc"}, capturing "token".
        let api = api_step_with_capture(
            "get-token",
            0,
            vec![StepOutputCapture {
                name: "token".into(),
                source: CaptureSource::ResponseBody {
                    path: "$.token".into(),
                },
                default: None,
                sensitive: false,
            }],
        );
        // Step 2: local test-echo step with a binding `password <- step/get-token/token`.
        // Capture the echoed password so we can verify the bound value arrived.
        let local = CompiledStep {
            outputs: vec![StepOutputCapture {
                name: "echoed_password".into(),
                source: CaptureSource::ResponseBody {
                    path: "$.password".into(),
                },
                default: None,
                sensitive: false,
            }],
            ..local_step_with_bindings(
                "docker-login",
                1,
                vec![step_ref("password", "get-token", "token")],
            )
        };

        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf-e2e-binding"),
            name: "binding e2e".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![WorkflowInputSpec {
                name: "namespace".into(),
                description: None,
                schema: Some(serde_json::json!({"type": "string"})),
                default: None,
                required: true,
                sensitive: false,
                location: ags_protocol::workflow::StepFieldLocation::default(),
                options_source: None,
                file_picker: None,
            }],
            is_reviewed_by_default: true,
            steps: vec![api, local],
            outputs: vec![WorkflowOutputAlias {
                name: "password_out".into(),
                from_step_id: "docker-login".into(),
                output: "echoed_password".into(),
                sensitive: false,
                section: None,
                label: None,
                item_fields: None,
            }],
            completion: None,
        };

        // Build a runtime with a queued HTTP client that returns the token.
        let client = QueuedClient::with_response(r#"{"token":"secret-abc"}"#);
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(client),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("svc", api_service_schema());

        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            assume_yes: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("test-ns"));

        let (outcome, output, pending) =
            Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success, "run must succeed");
        assert!(pending.is_none(), "no pending error expected");

        // The test-echo action echoes its inputs back, so the captured
        // output must contain the password the API step returned.
        let finished_events: Vec<_> = frontend
            .events
            .iter()
            .filter(|e| matches!(e, WorkflowEvent::StepFinished { id, .. } if id == "docker-login"))
            .collect();
        assert_eq!(
            finished_events.len(),
            1,
            "docker-login step must have a StepFinished event"
        );

        // Verify the step ran by asserting a success outcome event.
        match &finished_events[0] {
            WorkflowEvent::StepFinished { outcome, .. } => {
                assert_eq!(
                    *outcome,
                    StepOutcome::Success,
                    "docker-login step must succeed"
                );
            }
            _ => unreachable!(),
        }

        // Verify the handler RECEIVED the captured value: the echo action
        // echoes its inputs back, so the workflow output (captured from
        // $.password) must equal the value the API step returned.
        match output {
            Some(CommandOutput::Workflow { outputs, .. }) => {
                let password = outputs
                    .get("password_out")
                    .expect("password_out must be in workflow outputs");
                assert_eq!(
                    password,
                    &serde_json::json!("secret-abc"),
                    "the binding must deliver the API step's captured token to the local handler"
                );
            }
            other => panic!("expected Workflow output, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Test 2: precedence — a resolved binding overrides a same-named
    // entry in workflow_supplied.
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn test_local_step_binding_takes_precedence_over_workflow_supplied() {
        // The local step has a literal binding for "x" = "from-binding".
        // workflow_supplied also carries "x" = "from-flag".
        // The handler must receive "from-binding".
        let local = local_step_with_bindings(
            "echo",
            0,
            vec![literal_binding("x", serde_json::json!("from-binding"))],
        );

        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf-precedence"),
            name: "precedence".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            // Test-echo echoes its inputs; capture "x" to prove binding won.
            steps: vec![CompiledStep {
                outputs: vec![StepOutputCapture {
                    name: "x_out".into(),
                    source: CaptureSource::ResponseBody { path: "$.x".into() },
                    default: None,
                    sensitive: false,
                }],
                ..local
            }],
            outputs: vec![WorkflowOutputAlias {
                name: "x_final".into(),
                from_step_id: "echo".into(),
                output: "x_out".into(),
                sensitive: false,
                section: None,
                label: None,
                item_fields: None,
            }],
            completion: None,
        };

        let mut runtime = make_runtime_never();
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            assume_yes: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        // Same key "x" also in workflow_supplied — binding must win.
        pre_supplied.insert("x".to_string(), serde_json::json!("from-flag"));

        let (outcome, output, _) =
            Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);
        match output {
            Some(CommandOutput::Workflow { outputs, .. }) => {
                let x = outputs.get("x_final").expect("x_final must be in outputs");
                assert_eq!(
                    x,
                    &serde_json::json!("from-binding"),
                    "binding value must take precedence over workflow_supplied"
                );
            }
            other => panic!("expected Workflow output, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Test 3: an unresolvable binding fails at runtime with a clear
    // error — never a silent empty value.
    // -----------------------------------------------------------------
    #[test]
    fn test_unresolvable_step_reference_produces_error() {
        use crate::runtime::workflows::resolve::resolve_local_step_bindings;
        use crate::runtime::workflows::WorkflowContext;

        let step = local_step_with_bindings(
            "consumer",
            0,
            vec![step_ref("password", "producer", "token")],
        );
        // Empty context — no captures for "producer".
        let ctx = WorkflowContext::new();
        let workflow_supplied = BTreeMap::new();

        let err = resolve_local_step_bindings(&step, &ctx, &workflow_supplied)
            .expect_err("must fail when referenced step output is missing");
        assert!(
            err.message.contains("producer"),
            "error must mention the referenced step id: {}",
            err.message
        );
        assert!(
            err.message.contains("token"),
            "error must mention the referenced output name: {}",
            err.message
        );
    }

    // -----------------------------------------------------------------
    // Test 4: an invalid step reference on a local step is rejected at
    // compile time with the step id in the message.
    // -----------------------------------------------------------------
    #[test]
    fn test_invalid_step_reference_on_local_step_rejected_at_compile() {
        use ags_protocol::workflow::{StepDefinition, WorkflowDefinition};

        let def = WorkflowDefinition {
            id: WorkflowId::new("wf-bad-ref"),
            name: "bad ref".into(),
            workflow_protocol_version: None,
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![StepDefinition {
                id: "local-bad".into(),
                description: None,
                kind: ags_protocol::workflow::StepKind::Local,
                action: Some("test-echo".into()),
                operation: None,
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![step_ref("password", "nonexistent-step", "token")],
                outputs: vec![],
            }],
            outputs: vec![],
            completion: None,
        };
        let mut cat = crate::catalogue::Catalogue::new();
        let err = crate::runtime::workflows::compile::compile_workflow(&def, &mut cat)
            .expect_err("compile must reject a reference to a non-existent step");
        assert!(
            err.message.contains("local-bad"),
            "error must mention the local step id: {}",
            err.message
        );
        assert!(
            err.message.contains("nonexistent-step"),
            "error must mention the non-existent referenced step: {}",
            err.message
        );
    }

    // -----------------------------------------------------------------
    // Test 5: the secret value does NOT surface in the StepPreview or
    // StepFinished captures.
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn test_resolved_input_absent_from_preview_and_captures() {
        use ags_protocol::workflow::WorkflowInputSpec;

        let api = api_step_with_capture(
            "get-token",
            0,
            vec![StepOutputCapture {
                name: "token".into(),
                source: CaptureSource::ResponseBody {
                    path: "$.token".into(),
                },
                default: None,
                sensitive: false,
            }],
        );
        // Local step binds the captured secret. Capture the echoed
        // password so we can first prove the value WAS delivered — without
        // that, the non-leakage assertions below are vacuously true.
        let local = CompiledStep {
            outputs: vec![StepOutputCapture {
                name: "echoed_password".into(),
                source: CaptureSource::ResponseBody {
                    path: "$.password".into(),
                },
                default: None,
                sensitive: false,
            }],
            ..local_step_with_bindings(
                "docker-login",
                1,
                vec![step_ref("password", "get-token", "token")],
            )
        };

        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf-secret-leak"),
            name: "secret leak check".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![WorkflowInputSpec {
                name: "namespace".into(),
                description: None,
                schema: Some(serde_json::json!({"type": "string"})),
                default: None,
                required: true,
                sensitive: false,
                location: ags_protocol::workflow::StepFieldLocation::default(),
                options_source: None,
                file_picker: None,
            }],
            is_reviewed_by_default: true,
            steps: vec![api, local],
            outputs: vec![WorkflowOutputAlias {
                name: "password_check".into(),
                from_step_id: "docker-login".into(),
                output: "echoed_password".into(),
                sensitive: false,
                section: None,
                label: None,
                item_fields: None,
            }],
            completion: None,
        };

        let client = QueuedClient::with_response(r#"{"token":"SUPER-SECRET-TOKEN"}"#);
        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext {
                base_url: "https://example.com".into(),
                ..Default::default()
            },
            Box::new(client),
            reqwest::Client::new(),
        );
        runtime
            .catalogue_mut()
            .insert_for_tests("svc", api_service_schema());

        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            assume_yes: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("namespace".to_string(), serde_json::json!("ns"));

        let (outcome, output, _) =
            Executor::execute(&compiled, pre_supplied, &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(outcome, RunOutcome::Success);

        // First, verify the secret WAS delivered to the handler. Without
        // this, the non-leakage checks below would pass vacuously if
        // binding resolution silently returned nothing.
        match output {
            Some(CommandOutput::Workflow { outputs, .. }) => {
                let password = outputs
                    .get("password_check")
                    .expect("password_check must be in workflow outputs");
                assert_eq!(
                    password,
                    &serde_json::json!("SUPER-SECRET-TOKEN"),
                    "the binding must deliver the secret to the local handler"
                );
            }
            other => panic!("expected Workflow output, got {other:?}"),
        }

        // Assert the secret never appears in any StepFinished event's captures
        // or summary for the local step.
        let secret = "SUPER-SECRET-TOKEN";
        for event in &frontend.events {
            if let WorkflowEvent::StepFinished {
                id,
                summary,
                captures,
                ..
            } = event
            {
                if id == "docker-login" {
                    assert!(
                        !summary.contains(secret),
                        "step summary must not contain the secret: {summary}"
                    );
                    let captures_json = serde_json::to_string(captures).unwrap();
                    assert!(
                        !captures_json.contains(secret),
                        "StepFinished captures must not contain the secret: {captures_json}"
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------
    // Test 6: a local step with NO declared inputs still works
    // identically to the pre-change behaviour (regression guard).
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn test_local_step_no_inputs_unchanged() {
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf-no-inputs"),
            name: "no-inputs regression".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![local_step("echo-step", 0)],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = make_runtime_never();
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            assume_yes: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, _, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(
            outcome,
            RunOutcome::Success,
            "local step with no inputs must still succeed"
        );
        assert!(pending.is_none());
    }

    // -----------------------------------------------------------------
    // FIX 1: dry-run must propagate binding-resolution errors instead of
    // silently falling back to workflow_supplied. A broken binding
    // (bad JSONPath, missing upstream capture) that previews as success
    // defeats --dry-run's purpose.
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn test_dry_run_local_step_binding_error_is_fatal() {
        // Build a step that references a non-existent prior step output.
        // resolve_local_step_bindings will fail because no step "ghost"
        // ever ran and captured "token".
        let step =
            local_step_with_bindings("broken", 0, vec![step_ref("password", "ghost", "token")]);

        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf-dry-broken"),
            name: "dry-run broken binding".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![step],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = make_runtime_never();
        let mut frontend = MockFrontend::new();
        let options = RunOptions {
            dry_run: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);

        let (outcome, _final_output, pending) =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context)
                .await
                .unwrap();

        assert_eq!(
            outcome,
            RunOutcome::Failed,
            "dry-run must report failure when a binding cannot be resolved"
        );
        assert!(
            pending.is_some(),
            "dry-run must carry a pending error for the broken binding"
        );
    }
}
