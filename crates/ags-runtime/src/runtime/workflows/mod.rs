//! Workflow runtime engine: compilation, execution, and the frontend
//! callback interface. Data types live in `ags-protocol::workflow`; this
//! module owns the behaviour.

pub mod auto_derive;
pub mod builtins;
mod bundled;
pub mod compile;
pub mod dry_run;
pub mod executor;
mod external;
pub mod file_picker;
pub mod jsonpath;
pub mod local_actions;
pub mod nested_path;
pub mod options;
pub mod resolve;
pub mod synthesised;
pub mod telemetry_fields;
pub mod version_check;

pub use ags_protocol::workflow::{
    ResolvedOptions, RunOutcome, StepOutcome, WorkflowEvent, WorkflowFrontend,
};
pub use options::{resolve_options, OPTION_ITEM_CAP};

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use ags_protocol::error::RuntimeError;

/// Per-invocation knobs threaded into the executor. `quiet` and `no_color`
/// flow through the frontend, not here.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Skip real dispatch; build typed-placeholder previews per step.
    pub dry_run: bool,
    /// Bypass every per-step confirmation prompt.
    pub assume_yes: bool,
    /// Refuse to gather or prompt; fail with an aggregated message instead.
    pub no_input: bool,
    /// Pause on every step to review/edit its full request. Set by the route
    /// only for an interactive fullscreen workflow run (see the design's
    /// "Where the pause decision lives"). Default false.
    pub review_steps: bool,
    /// Whether the active surface can render a dynamic-enum picker. Set by the
    /// route: true only for the fullscreen surface. When false, the executor
    /// drops picker-support inputs (those that only parameterise a picker) from
    /// the upfront gather, since the picker-backed input is typed directly.
    pub pickers_available: bool,
    /// Selected output format. Threaded through to each step's
    /// `CommandRequest` so dispatch produces the right envelope.
    pub output_format: ags_protocol::request::OutputFormat,
    /// `--output` destination. Threaded into each step's `CommandRequest`
    /// so dispatch writes the raw body to a file / stdout instead of
    /// rendering it through the frontend.
    pub output: Option<ags_protocol::request::OutputDestination>,
    /// Verbosity. Threaded into the `CommandRequest` so `run_command`
    /// builds the resolution trace into `ApiOutput` under `--verbose`.
    pub verbosity: ags_protocol::request::Verbosity,
    /// Pagination policy from `--page-limit` / `--page-all`.
    pub pagination: ags_protocol::request::PaginationHint,
    /// An explicit, opaque request body (from `ags <svc> <op> --json '{…}'`).
    /// When set, `assemble_command_request` uses this value verbatim and
    /// skips per-field body reconstruction and required-field validation —
    /// the user supplied the complete body, so absent fields are intentional
    /// and the server is the authority. `None` means build the body from
    /// resolved per-field inputs as usual.
    pub explicit_body: Option<serde_json::Value>,
    /// True when the workflow being run is registered as bundled (see
    /// `WorkflowRegistry::is_bundled`). Resolved once by the calling route
    /// and threaded through so the executor never needs to reach the
    /// process-wide registry itself. Gates whether a failed step's
    /// `input_fields` may carry real values (bundled) or must withhold every
    /// value (external, or a synthesised single-command run that was never
    /// registered at all — the safe default is `false`).
    pub is_bundled_workflow: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            assume_yes: false,
            no_input: false,
            review_steps: false,
            pickers_available: false,
            output_format: ags_protocol::request::OutputFormat::default(),
            output: None,
            verbosity: ags_protocol::request::Verbosity::default(),
            pagination: ags_protocol::request::PaginationHint::Auto,
            explicit_body: None,
            is_bundled_workflow: false,
        }
    }
}

use ags_protocol::output::BinaryWrittenOutput;
use ags_protocol::output_views::{
    ApiBody, ApiOutput, WorkflowOutputItem, WorkflowOutputProvenance, WorkflowOutputView,
};
use ags_protocol::workflow::{CaptureSource, StepOutputCapture, WorkflowOutputAlias};

/// Per-invocation state held by the executor across steps. Stores each
/// step's full `ApiOutput` (so a 1-step synth path can reuse it verbatim)
/// and each step's named captures (for downstream `from: step/X, output: Y`
/// references). A parallel `provenances` map records how each captured value
/// was obtained (`Captured`, `Missing`, or `Skipped`) for the output view.
///
/// `local_step_bodies` is a parallel storage path for `kind: local` steps,
/// whose handlers return a `serde_json::Value` directly (not an `ApiOutput`).
/// `step_body_json` checks `local_step_bodies` before `step_outputs`, so
/// `bind_step_outputs` works identically for both step kinds.
#[derive(Debug, Default)]
pub struct WorkflowContext {
    step_outputs: BTreeMap<String, ApiOutput>,
    local_step_bodies: BTreeMap<String, serde_json::Value>,
    captures: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    provenances: BTreeMap<String, BTreeMap<String, WorkflowOutputProvenance>>,
    binary_outputs: BTreeMap<String, BinaryWrittenOutput>,
}

impl WorkflowContext {
    /// Build an empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a step's complete dispatch output. Called unconditionally on
    /// dispatch success regardless of whether the step declares any
    /// `outputs`. Needed for byte-identical single-command parity.
    pub fn store_step_output(&mut self, step_id: &str, output: ApiOutput) {
        self.step_outputs.insert(step_id.to_string(), output);
    }

    /// Retrieve the stored `ApiOutput`. Used by `build_final_output` on the
    /// 1-step synth path to clone into `CommandOutput::Service`.
    pub fn step_output(&self, step_id: &str) -> Option<&ApiOutput> {
        self.step_outputs.get(step_id)
    }

    /// Store a local step's result body. Called by the executor after a
    /// `kind: local` handler returns `Ok(value)`. The stored value feeds
    /// `step_body_json`, which `bind_step_outputs` uses for capture
    /// resolution — the same path as an API step's `ApiOutput.body`.
    pub fn store_local_step_body(&mut self, step_id: &str, body: serde_json::Value) {
        self.local_step_bodies.insert(step_id.to_string(), body);
    }

    /// Store a step's binary-write result. Called on dispatch success when
    /// the step produced a `BinaryWritten` envelope (binary response body or
    /// `--output`-to-file/stdout). A binary step has no JSON body, so it
    /// cannot feed downstream `from: step/X` references — only the 1-step
    /// synthesised path consults this.
    pub fn store_binary_output(&mut self, step_id: &str, output: BinaryWrittenOutput) {
        self.binary_outputs.insert(step_id.to_string(), output);
    }

    /// Retrieve a stored `BinaryWrittenOutput`. Used by `build_final_output`
    /// on the 1-step synth path to surface `CommandOutput::BinaryWritten`.
    pub fn binary_output(&self, step_id: &str) -> Option<&BinaryWrittenOutput> {
        self.binary_outputs.get(step_id)
    }

    /// Project the stored step body into a JSON value for capture rules,
    /// JSONPath transforms, and summary formatting.
    ///
    /// Checks `local_step_bodies` first (populated by `kind: local`
    /// handlers), then falls back to `step_outputs` (populated by API
    /// dispatch). Returns `None` when the body is `Empty` or the step was
    /// never stored. `Shaped` bodies serialise via `serde_json::to_value`;
    /// non-serialisable shapes produce `None`.
    pub fn step_body_json(&self, step_id: &str) -> Option<serde_json::Value> {
        // Local step bodies are stored directly as JSON values.
        if let Some(body) = self.local_step_bodies.get(step_id) {
            return Some(body.clone());
        }
        let api_output = self.step_outputs.get(step_id)?;
        // Captures run against the raw JSON response (real API field names), not
        // the presentation-shaped `body`. Fall back to the body only for a
        // non-JSON text response, where no raw JSON exists.
        if let Some(raw) = &api_output.raw_body {
            return Some(raw.clone());
        }
        match &api_output.body {
            ApiBody::Empty | ApiBody::Shaped(_) => None,
            ApiBody::Text(s) => Some(serde_json::Value::String(s.clone())),
        }
    }

    /// Apply each declared output capture to the projected body. Captures
    /// that fail JSONPath resolution and have no `default:` produce an
    /// error that fails the step. Records provenance: `Captured` when the
    /// JSONPath resolved, `Missing` when it fell to the default.
    pub fn bind_step_outputs(
        &mut self,
        step_id: &str,
        captures: &[StepOutputCapture],
        body_json: Option<&serde_json::Value>,
    ) -> Result<(), RuntimeError> {
        let entry = self.captures.entry(step_id.to_string()).or_default();
        let prov_entry = self.provenances.entry(step_id.to_string()).or_default();
        for capture in captures {
            let CaptureSource::ResponseBody { path } = &capture.source;
            let resolved = body_json.and_then(|body| {
                crate::runtime::workflows::jsonpath::apply_jsonpath_subset(body, path)
            });
            let (stored, provenance) = match (resolved, &capture.default) {
                (Some(v), _) => (v, WorkflowOutputProvenance::Captured),
                (None, Some(default)) => (default.clone(), WorkflowOutputProvenance::Missing),
                (None, None) => {
                    return Err(RuntimeError::internal(format!(
                        "step '{step_id}' output '{}' did not resolve (path '{path}') and has no default",
                        capture.name
                    )));
                }
            };
            entry.insert(capture.name.clone(), stored);
            prov_entry.insert(capture.name.clone(), provenance);
        }
        Ok(())
    }

    /// Bind each declared capture to its `default` with provenance `Skipped`.
    /// Called on the failure-tolerant path in the executor when a step failed
    /// and was not dispatched. Errors if any capture has no default (same
    /// defaulting rule as `bind_step_outputs` on a missing path, but tagged
    /// `Skipped`).
    ///
    /// Footgun: the executor calls this with `?`, so a capture without a
    /// `default` hard-fails the whole run rather than skipping the step. A
    /// `continue_on_failure` step must therefore give every capture a `default`.
    pub fn bind_skipped_outputs(
        &mut self,
        step_id: &str,
        captures: &[StepOutputCapture],
    ) -> Result<(), RuntimeError> {
        let entry = self.captures.entry(step_id.to_string()).or_default();
        let prov_entry = self.provenances.entry(step_id.to_string()).or_default();
        for capture in captures {
            let stored = match &capture.default {
                Some(default) => default.clone(),
                None => {
                    return Err(RuntimeError::internal(format!(
                        "step '{step_id}' output '{}' has no default for skipped binding",
                        capture.name
                    )));
                }
            };
            entry.insert(capture.name.clone(), stored);
            prov_entry.insert(capture.name.clone(), WorkflowOutputProvenance::Skipped);
        }
        Ok(())
    }

    /// Inject a step's full capture map directly, bypassing JSONPath
    /// resolution. Used by the dry-run branch in the executor where outputs
    /// come from `synthesise_dry_run_outputs` rather than real responses.
    pub fn inject_step_captures_for_dry_run(
        &mut self,
        step_id: &str,
        captures: &BTreeMap<String, serde_json::Value>,
    ) {
        self.captures.insert(step_id.to_string(), captures.clone());
    }

    /// Look up a captured value (used to resolve `from: step/X, output: Y`).
    pub fn resolve_step_reference(
        &self,
        step_id: &str,
        output_name: &str,
    ) -> Option<&serde_json::Value> {
        self.captures.get(step_id)?.get(output_name)
    }

    /// Resolve the workflow's declared output aliases into the final
    /// envelope's alias map. Aliases that point at captures which did not
    /// land are silently omitted (capture-time errors already failed the
    /// step; reaching here means every alias's source step succeeded).
    /// This is the flat map consumed by `--format json`; it is UNCHANGED by
    /// Task 7 — only `output_view` is new.
    pub fn resolve_workflow_outputs(
        &self,
        aliases: &[WorkflowOutputAlias],
    ) -> BTreeMap<String, serde_json::Value> {
        aliases
            .iter()
            .filter_map(|alias| {
                self.resolve_step_reference(&alias.from_step_id, &alias.output)
                    .cloned()
                    .map(|v| (alias.name.clone(), v))
            })
            .collect()
    }

    /// Look up the captured value and its provenance for a given step/output
    /// pair. Returns `None` when the step or output name was never bound
    /// (aliases pointing at captures that never landed are silently omitted
    /// by `resolve_workflow_output_view`).
    fn capture_with_provenance(
        &self,
        step_id: &str,
        output_name: &str,
    ) -> Option<(serde_json::Value, WorkflowOutputProvenance)> {
        let value = self.captures.get(step_id)?.get(output_name)?.clone();
        let prov = self.provenances.get(step_id)?.get(output_name)?.clone();
        Some((value, prov))
    }

    /// Build the structured `WorkflowOutputView` from the workflow's declared
    /// output aliases. Each alias that resolves to a captured value yields one
    /// `WorkflowOutputItem`; aliases with no matching capture are silently
    /// omitted. Carries `section`, `label`, and provenance so the human
    /// frontend can render a grouped overview panel.
    pub fn resolve_workflow_output_view(
        &self,
        aliases: &[WorkflowOutputAlias],
    ) -> WorkflowOutputView {
        let items = aliases
            .iter()
            .filter_map(|a| {
                let (value, prov) = self.capture_with_provenance(&a.from_step_id, &a.output)?;
                Some(WorkflowOutputItem {
                    name: a.name.clone(),
                    section: a.section.clone(),
                    label: a.label.clone(),
                    value,
                    provenance: prov,
                    item_fields: a.item_fields.clone(),
                })
            })
            .collect();
        WorkflowOutputView { items }
    }
}

use ags_protocol::workflow::{WorkflowDefinition, WorkflowId};

/// Author-facing trait. Each registered workflow returns its definition;
/// the runtime compiles it on demand.
pub trait Workflow: Send + Sync {
    /// Return the workflow's author-input `WorkflowDefinition`.
    fn definition(&self) -> &WorkflowDefinition;
}

/// Thin `Workflow` wrapper around a YAML-parsed `WorkflowDefinition` —
/// shared by both the bundled loader (`bundled`) and the external-file
/// loader (`external`), since neither needs anything beyond exposing the
/// parsed definition.
pub(crate) struct YamlWorkflow(WorkflowDefinition);

impl YamlWorkflow {
    pub(crate) fn new(definition: WorkflowDefinition) -> Self {
        Self(definition)
    }
}

impl Workflow for YamlWorkflow {
    fn definition(&self) -> &WorkflowDefinition {
        &self.0
    }
}

/// Where a registered workflow came from. Determines whether a failed step's
/// telemetry `input_fields` may carry real values (§4.1 of the observability
/// design): AccelByte authors and therefore knows what a bundled workflow's
/// input schemas hold; an external, user-installed workflow's schema is not
/// AccelByte's to vouch for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowOrigin {
    /// Shipped with the binary — a Rust-literal builtin or compiled-in YAML.
    Bundled,
    /// Loaded from a user's local `workflows_dir()` file.
    External,
}

/// In-process registry of available workflows. Plan E adds a builtin
/// registry that pre-populates this; Plan C adds CLI commands that look
/// workflows up here.
#[derive(Default)]
pub struct WorkflowRegistry {
    workflows: BTreeMap<WorkflowId, (Box<dyn Workflow>, WorkflowOrigin)>,
}

impl WorkflowRegistry {
    /// Build an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a bundled workflow. The workflow's `definition().id` becomes its
    /// key. Used by the builtin and bundled-YAML loaders only — an external
    /// (user-installed) workflow must go through `register_external` instead,
    /// so its origin is never mistaken for bundled.
    pub fn register(&mut self, workflow: Box<dyn Workflow>) {
        let id = workflow.definition().id.clone();
        self.workflows
            .insert(id, (workflow, WorkflowOrigin::Bundled));
    }

    /// Add an external (user-installed) workflow. The single insertion point
    /// is `load_external_workflows`, so no other caller can register a user
    /// workflow as bundled by mistake.
    pub fn register_external(&mut self, workflow: Box<dyn Workflow>) {
        let id = workflow.definition().id.clone();
        self.workflows
            .insert(id, (workflow, WorkflowOrigin::External));
    }

    /// Look up a workflow by id.
    pub fn resolve(&self, id: &WorkflowId) -> Option<&dyn Workflow> {
        self.workflows.get(id).map(|(w, _)| w.as_ref())
    }

    /// Look up a registered workflow's origin, if any.
    pub fn origin(&self, id: &WorkflowId) -> Option<WorkflowOrigin> {
        self.workflows.get(id).map(|(_, origin)| *origin)
    }

    /// True when `id` is registered and its origin is `Bundled`. False for an
    /// external workflow and for any id not registered at all (including a
    /// synthesised single-command run, which is never registered) — the safe
    /// default withholds real values.
    pub fn is_bundled(&self, id: &WorkflowId) -> bool {
        matches!(self.origin(id), Some(WorkflowOrigin::Bundled))
    }

    /// Iterator over registered workflow ids (used by future `ags workflow
    /// list` and CLI completion).
    pub fn ids(&self) -> impl Iterator<Item = &WorkflowId> {
        self.workflows.keys()
    }

    /// Return `(id, name)` for every registered workflow — the data behind
    /// `ags workflow list` and the `ags workflow --help` block.
    pub fn entries(&self) -> Vec<(String, String)> {
        self.workflows
            .values()
            .map(|(workflow, _)| {
                let definition = workflow.definition();
                (definition.id.as_str().to_string(), definition.name.clone())
            })
            .collect()
    }
}

/// Process-wide workflow registry consulted by `ags workflow run <id>`.
/// Populated with the built-in workflows the first time it is accessed.
pub fn registry() -> &'static WorkflowRegistry {
    static REGISTRY: std::sync::OnceLock<WorkflowRegistry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = WorkflowRegistry::new();
        builtins::register_builtins(&mut registry);
        external::load_external_workflows(&mut registry);
        registry
    })
}

/// Find the on-disk external workflow file matching `id`, if any — used by
/// `ags workflow remove <id>` to locate the file to delete. `external` is a
/// private module (deliberately encapsulated — see this module's doc comment)
/// so this is a thin pass-through; the real scan lives in
/// `external::find_external_workflow_by_id`.
pub fn find_external_workflow_file(
    id: &WorkflowId,
) -> Result<Option<std::path::PathBuf>, RuntimeError> {
    external::find_external_workflow_by_id(id)
}

#[cfg(test)]
mod context_tests {
    use super::*;
    use ags_protocol::output_views::ApiBody;

    /// Minimal `ApiOutput` builder that constructs only what
    /// `WorkflowContext` actually inspects.
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

    /// Build a minimal `OperationSchema` literal — used only as a placeholder
    /// since `WorkflowContext` does not inspect the operation field.
    fn dummy_operation_schema() -> ags_protocol::catalogue::OperationSchema {
        ags_protocol::catalogue::OperationSchema {
            id: ags_protocol::catalogue::OperationId::new("dummy"),
            name: "dummy".into(),
            summary: String::new(),
            description: None,
            mutation_class: ags_protocol::catalogue::MutationClass::ReadOnly,
            http_method: ags_protocol::catalogue::HttpMethod::Get,
            path_template: "/".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ags_protocol::catalogue::ApiVersion::new(1),
            deprecated: false,
            response_content_type: None,
        }
    }

    #[test]
    fn test_store_and_retrieve_step_output() {
        let mut ctx = WorkflowContext::new();
        ctx.store_step_output("create-stat", dummy_api_output_empty());
        assert!(ctx.step_output("create-stat").is_some());
        assert!(ctx.step_output("missing").is_none());
    }

    #[test]
    fn test_bind_step_outputs_requires_default_when_unresolved() {
        let mut ctx = WorkflowContext::new();
        ctx.store_step_output("s1", dummy_api_output_empty());
        let result = ctx.bind_step_outputs(
            "s1",
            &[ags_protocol::workflow::StepOutputCapture {
                name: "x".into(),
                source: ags_protocol::workflow::CaptureSource::ResponseBody {
                    path: "$.foo".into(),
                },
                default: None,
                sensitive: false,
            }],
            None,
        );
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------
    // Test Plan case 10: store_local_step_body stores and step_body_json
    // retrieves the stored value.
    // -----------------------------------------------------------------
    #[test]
    fn test_local_step_body_stored_and_retrieved() {
        let mut ctx = WorkflowContext::new();
        let body = serde_json::json!({"k": "v", "n": 42});
        ctx.store_local_step_body("local-step", body.clone());

        let retrieved = ctx.step_body_json("local-step");
        assert_eq!(
            retrieved,
            Some(body),
            "step_body_json must return the stored local body"
        );
    }

    // -----------------------------------------------------------------
    // Test Plan case 10 complement: local_step_bodies takes precedence
    // over step_outputs in step_body_json.
    // -----------------------------------------------------------------
    #[test]
    fn test_local_step_body_takes_precedence_over_api_output() {
        let mut ctx = WorkflowContext::new();
        // Store an API output AND a local body for the same step id.
        let mut api = dummy_api_output_empty();
        api.raw_body = Some(serde_json::json!({"api": true}));
        ctx.store_step_output("step-x", api);
        let local_body = serde_json::json!({"local": true});
        ctx.store_local_step_body("step-x", local_body.clone());

        let retrieved = ctx.step_body_json("step-x");
        assert_eq!(
            retrieved,
            Some(local_body),
            "local body must take precedence over API output"
        );
    }

    // -----------------------------------------------------------------
    // Test Plan case 11: bind_step_outputs resolves a capture from the
    // local handler's return value.
    // -----------------------------------------------------------------
    #[test]
    fn test_local_step_captures_bind() {
        let mut ctx = WorkflowContext::new();
        let body = serde_json::json!({"token": "abc123", "registry": "reg.io"});
        ctx.store_local_step_body("auth", body.clone());

        let captures = vec![ags_protocol::workflow::StepOutputCapture {
            name: "token".into(),
            source: ags_protocol::workflow::CaptureSource::ResponseBody {
                path: "$.token".into(),
            },
            default: None,
            sensitive: false,
        }];
        ctx.bind_step_outputs("auth", &captures, Some(&body))
            .expect("bind must succeed");

        let resolved = ctx.resolve_step_reference("auth", "token");
        assert_eq!(
            resolved,
            Some(&serde_json::json!("abc123")),
            "capture must resolve to the JSONPath result from the local body"
        );
    }

    // -----------------------------------------------------------------
    // Test Plan case 12: a second step bound via `from: step/X` reads
    // the value stored by the local step.
    // -----------------------------------------------------------------
    #[test]
    fn test_from_step_reference_reads_local_output() {
        let mut ctx = WorkflowContext::new();
        // Simulate a local step producing output with a capture.
        let body = serde_json::json!({"cred": "secret-value"});
        ctx.store_local_step_body("local-auth", body.clone());
        let captures = vec![ags_protocol::workflow::StepOutputCapture {
            name: "cred".into(),
            source: ags_protocol::workflow::CaptureSource::ResponseBody {
                path: "$.cred".into(),
            },
            default: None,
            sensitive: false,
        }];
        ctx.bind_step_outputs("local-auth", &captures, Some(&body))
            .expect("bind must succeed");

        // A downstream step resolves `from: step/local-auth, output: cred`.
        let downstream_value = ctx.resolve_step_reference("local-auth", "cred");
        assert_eq!(
            downstream_value,
            Some(&serde_json::json!("secret-value")),
            "downstream step must read the local step's captured output"
        );
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    struct FakeWorkflow {
        definition: WorkflowDefinition,
    }

    impl Workflow for FakeWorkflow {
        fn definition(&self) -> &WorkflowDefinition {
            &self.definition
        }
    }

    #[test]
    fn test_registry_resolves_registered_workflow() {
        let mut registry = WorkflowRegistry::new();
        let id = WorkflowId::new("hello");
        let workflow = FakeWorkflow {
            definition: WorkflowDefinition {
                id: id.clone(),
                name: "hello".into(),
                workflow_protocol_version: None,
                intent: None,
                description: None,
                briefing: None,
                inputs: vec![],
                is_reviewed_by_default: true,
                steps: vec![],
                outputs: vec![],
                completion: None,
            },
        };
        registry.register(Box::new(workflow));
        assert!(registry.resolve(&id).is_some());
        assert!(registry.resolve(&WorkflowId::new("missing")).is_none());
    }

    #[test]
    fn test_registry_entries_returns_id_and_name() {
        let mut registry = WorkflowRegistry::new();
        registry.register(Box::new(FakeWorkflow {
            definition: WorkflowDefinition {
                id: WorkflowId::new("hello"),
                name: "Hello Workflow".into(),
                workflow_protocol_version: None,
                intent: None,
                description: None,
                briefing: None,
                inputs: vec![],
                is_reviewed_by_default: true,
                steps: vec![],
                outputs: vec![],
                completion: None,
            },
        }));
        let entries = registry.entries();
        assert_eq!(
            entries,
            vec![("hello".to_string(), "Hello Workflow".to_string())]
        );
    }

    /// Build a minimal `FakeWorkflow` with the given id, for origin tests.
    fn fake_workflow(id: &WorkflowId) -> FakeWorkflow {
        FakeWorkflow {
            definition: WorkflowDefinition {
                id: id.clone(),
                name: "fake".into(),
                workflow_protocol_version: None,
                intent: None,
                description: None,
                briefing: None,
                inputs: vec![],
                is_reviewed_by_default: true,
                steps: vec![],
                outputs: vec![],
                completion: None,
            },
        }
    }

    #[test]
    fn test_register_marks_workflow_bundled() {
        let mut registry = WorkflowRegistry::new();
        let id = WorkflowId::new("bundled-one");
        registry.register(Box::new(fake_workflow(&id)));
        assert_eq!(registry.origin(&id), Some(WorkflowOrigin::Bundled));
        assert!(registry.is_bundled(&id));
    }

    #[test]
    fn test_register_external_marks_workflow_external() {
        let mut registry = WorkflowRegistry::new();
        let id = WorkflowId::new("external-one");
        registry.register_external(Box::new(fake_workflow(&id)));
        assert_eq!(registry.origin(&id), Some(WorkflowOrigin::External));
        assert!(!registry.is_bundled(&id));
    }

    #[test]
    fn test_is_bundled_false_for_unregistered_id() {
        let registry = WorkflowRegistry::new();
        assert!(!registry.is_bundled(&WorkflowId::new("nowhere")));
        assert_eq!(registry.origin(&WorkflowId::new("nowhere")), None);
    }
}
