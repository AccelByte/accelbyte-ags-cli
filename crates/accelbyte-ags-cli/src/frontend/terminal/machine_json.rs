//! `JsonInteraction` — workflow interaction for the JSON frontend.
//!
//! JSON mode runs workflows non-interactively: all inputs come from flags and
//! confirmations are pre-checked. These methods are therefore a defensive
//! guard — a valid JSON run never reaches them. Both return a `Usage` error.

use crate::errors::CliError;
use crate::frontend::ExecutionInteraction;
use ags_protocol::workflow::{
    CompiledStep, GatherResult, StepPreview, SuppliedInputView, WorkflowInputNeeded,
};

/// Zero-sized interaction handler for the JSON frontend.
///
/// A valid JSON workflow run gathers no input interactively (the no-input
/// precheck rejects anything that would, and confirmations are skipped under
/// `--yes`/`--dry-run`), so these methods are an unreachable defensive guard.
/// Both return a `Usage` error.
pub struct JsonInteraction;

impl ExecutionInteraction for JsonInteraction {
    fn gather_workflow_inputs(
        &mut self,
        _needed: &[WorkflowInputNeeded],
        _step_context: &CompiledStep,
        _supplied: &[SuppliedInputView],
    ) -> Result<GatherResult, CliError> {
        Err(CliError::Usage {
            message: "Interactive workflow input is not available under --format=json".into(),
            metadata: None,
        })
    }

    fn confirm_step(
        &mut self,
        _step: &CompiledStep,
        _preview: &StepPreview,
    ) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
        Err(CliError::Usage {
            message: "Interactive workflow input is not available under --format=json".into(),
            metadata: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::JsonInteraction;
    use crate::errors::CliError;
    use crate::frontend::ExecutionInteraction;
    use ags_protocol::catalogue::{HttpMethod, MutationClass, OperationId, ServiceId};
    use ags_protocol::result::CommandPreview;
    use ags_protocol::workflow::{CompiledStep, OperationReference, StepPreview};

    /// Build a minimal `CompiledStep` for exercising the interaction methods.
    fn make_compiled_step() -> CompiledStep {
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

    #[test]
    fn test_json_interaction_gather_workflow_inputs_returns_usage_error() {
        let mut interaction = JsonInteraction;
        let step = make_compiled_step();
        let result = interaction.gather_workflow_inputs(&[], &step, &[]);
        match result {
            Err(CliError::Usage { message, metadata }) => {
                assert_eq!(
                    message,
                    "Interactive workflow input is not available under --format=json"
                );
                assert!(metadata.is_none());
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[test]
    fn test_json_interaction_confirm_step_returns_usage_error() {
        let mut interaction = JsonInteraction;
        let step = make_compiled_step();
        let preview = StepPreview {
            workflow_name: "test-workflow".to_string(),
            step_id: "test-step".to_string(),
            step_label: "Test Step".to_string(),
            step_index: 0,
            step_total: 1,
            command: CommandPreview {
                service: ServiceId::new("iam"),
                operation_id: OperationId::new("testOp"),
                summary: "Test operation".to_string(),
                http_method: HttpMethod::Get,
                url: "https://example.test/iam/v1/test".to_string(),
                mutation_class: MutationClass::ReadOnly,
                confirmation_required: false,
                warnings: vec![],
            },
        };
        let result = interaction.confirm_step(&step, &preview);
        match result {
            Err(CliError::Usage { message, metadata }) => {
                assert_eq!(
                    message,
                    "Interactive workflow input is not available under --format=json"
                );
                assert!(metadata.is_none());
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    /// An optional step (`is_optional: true`) must NOT weaken the defensive
    /// guard: `JsonInteraction::confirm_step` still returns `CliError::Usage`.
    /// Optionality is an interactive affordance; the JSON surface has no
    /// mechanism for the user to choose Skip vs Proceed.
    #[test]
    fn test_json_confirm_step_returns_usage_error_for_optional_step() {
        let mut interaction = JsonInteraction;
        let step = CompiledStep {
            id: "optional-step".to_string(),
            index: 0,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new("iam"),
                operation: OperationId::new("testOp"),
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
        };
        let preview = StepPreview {
            workflow_name: "test-workflow".to_string(),
            step_id: "optional-step".to_string(),
            step_label: "Optional Step".to_string(),
            step_index: 0,
            step_total: 1,
            command: CommandPreview {
                service: ServiceId::new("iam"),
                operation_id: OperationId::new("testOp"),
                summary: "Test operation".to_string(),
                http_method: HttpMethod::Get,
                url: "https://example.test/iam/v1/test".to_string(),
                mutation_class: MutationClass::ReadOnly,
                confirmation_required: false,
                warnings: vec![],
            },
        };
        let result = interaction.confirm_step(&step, &preview);
        match result {
            Err(CliError::Usage { message, metadata }) => {
                assert_eq!(
                    message,
                    "Interactive workflow input is not available under --format=json"
                );
                assert!(metadata.is_none());
            }
            other => panic!("expected Usage error for optional step, got {other:?}"),
        }
    }
}
