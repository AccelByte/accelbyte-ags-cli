//! Human-readable rendering for workflow dry-run previews.

use crate::errors::CliError;
use crate::frontend::RenderedOutput;
use ags_protocol::workflow::{StepDryRunAction, StepDryRunPreview, WorkflowId};

/// Render the dry-run preview of a multi-step workflow.
///
/// Each step's `DryRunResult` is rendered using the single-command dry-run
/// renderer and the blocks are stitched together with step headers into a
/// single stdout payload. `is_stdout_first` is `true` — there is no stderr
/// content for a dry-run.
pub(crate) fn render_workflow_dry_run(
    workflow_id: &WorkflowId,
    step_previews: &[StepDryRunPreview],
) -> Result<RenderedOutput, CliError> {
    let mut parts: Vec<String> = Vec::new();

    parts.push(format!("Workflow dry-run: {}", workflow_id.as_str()));

    for preview in step_previews {
        parts.push(format!(
            "\nStep {} ({})",
            preview.step_index + 1,
            preview.step_id,
        ));

        let step_text = match &preview.action {
            StepDryRunAction::Request(command) => {
                super::service::render_dry_run_output(command)?.stdout
            }
            // A local-action step sends no HTTP request, so there is
            // nothing to render as one. Naming the action and showing what
            // its dry run produced is the whole of the preview.
            StepDryRunAction::Local { action, preview } => Some(format!(
                "Local action: {}\n{}",
                action,
                serde_json::to_string_pretty(preview).unwrap_or_else(|_| preview.to_string())
            )),
        };

        if let Some(text) = step_text {
            for line in text.lines() {
                parts.push(format!("  {line}"));
            }
        }

        // Synthesised placeholder outputs for downstream steps.
        if !preview.synthesised_outputs.is_empty() {
            parts.push("  Synthesised outputs:".to_string());
            for (key, value) in &preview.synthesised_outputs {
                parts.push(format!("    {key}: {value}"));
            }
        }
    }

    Ok(RenderedOutput {
        stdout: Some(parts.join("\n")),
        stderr: None,
        is_stdout_first: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::catalogue::HttpMethod;
    use ags_protocol::result::DryRunResult;

    /// Build a single-step dry-run preview fixture for the workflow render tests.
    fn make_preview(index: usize, id: &str) -> StepDryRunPreview {
        StepDryRunPreview {
            step_id: id.to_string(),
            step_index: index,
            action: StepDryRunAction::Request(DryRunResult {
                http_method: HttpMethod::Post,
                url: format!("https://example.test/api/{id}"),
                headers: vec![("Authorization".to_string(), "Bearer <redacted>".to_string())],
                query: vec![],
                body: Some(ags_protocol::request::RequestBody::Json(
                    serde_json::json!({"field": "value"}),
                )),
            }),
            synthesised_outputs: {
                let mut m = std::collections::BTreeMap::new();
                m.insert("id".to_string(), serde_json::json!("placeholder-id"));
                m
            },
        }
    }

    #[test]
    fn test_render_workflow_dry_run_contains_workflow_id() {
        let id = WorkflowId::new("my-workflow");
        let previews = vec![make_preview(0, "step-one")];
        let rendered = render_workflow_dry_run(&id, &previews).unwrap();
        let stdout = rendered.stdout.unwrap_or_default();
        assert!(stdout.contains("my-workflow"), "missing workflow id");
        assert!(stdout.contains("step-one"), "missing step id");
    }

    #[test]
    fn test_render_workflow_dry_run_renders_a_local_action_step() {
        let previews = vec![StepDryRunPreview {
            step_id: "upload-image".to_string(),
            step_index: 0,
            action: StepDryRunAction::Local {
                action: "ams/upload-image".to_string(),
                preview: serde_json::json!({"image_name": "demo-image"}),
            },
            synthesised_outputs: std::collections::BTreeMap::new(),
        }];
        let rendered = render_workflow_dry_run(&WorkflowId::new("wf"), &previews).unwrap();
        let stdout = rendered.stdout.unwrap_or_default();
        assert!(stdout.contains("upload-image"), "{stdout}");
        assert!(stdout.contains("ams/upload-image"), "{stdout}");
        assert!(stdout.contains("demo-image"), "{stdout}");
    }

    #[test]
    fn test_render_workflow_dry_run_is_stdout_first() {
        let id = WorkflowId::new("wf");
        let rendered = render_workflow_dry_run(&id, &[]).unwrap();
        assert!(rendered.is_stdout_first);
        assert!(rendered.stderr.is_none());
    }
}
