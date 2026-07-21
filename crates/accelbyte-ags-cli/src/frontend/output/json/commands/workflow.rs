//! Machine-readable rendering for workflow meta-commands.

use ags_protocol::output_views::WorkflowCompletionView;
use ags_protocol::workflow::{StepDryRunPreview, WorkflowId, WorkflowListEntry};
use std::collections::BTreeMap;

use crate::errors::CliError;
use crate::frontend::RenderedOutput;

/// Render the registered-workflow catalogue as a JSON array of
/// `{ "id": ..., "name": ... }` objects.
pub(crate) fn render_workflow_catalogue(
    entries: &[WorkflowListEntry],
    _options: &crate::frontend::RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let value = serde_json::to_value(entries)
        .map_err(|error| CliError::Internal(anyhow::anyhow!(error)))?;
    Ok(RenderedOutput {
        stdout: Some(crate::frontend::output::json::format_json(&value)?),
        stderr: None,
        is_stdout_first: true,
    })
}

/// Render the final output of a completed multi-step workflow run as a single
/// JSON envelope on stdout. `step_summaries` is accepted so this mirrors the
/// human renderer's signature for the symmetrical `render.rs` dispatch arm, but
/// it is unused: the JSON success envelope carries no per-step list (a
/// successful run executes every step — the inventory is static metadata
/// available via `ags describe workflow <id>`).
///
/// This only ever receives a `CommandOutput::Workflow`, which the executor emits
/// **only after full success** (a failed step breaks the run and yields no final
/// output). If partial-run output is ever introduced, step-level status must be
/// added to this envelope so consumers don't read `status: "success"` as full
/// completion.
pub(crate) fn render_workflow(
    workflow_id: &WorkflowId,
    outputs: &BTreeMap<String, serde_json::Value>,
    _step_summaries: &[String],
    completion: &Option<WorkflowCompletionView>,
    _options: &crate::frontend::RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let mut envelope = serde_json::Map::new();
    envelope.insert(
        "workflow".to_string(),
        serde_json::Value::String(workflow_id.as_str().to_string()),
    );
    envelope.insert(
        "status".to_string(),
        serde_json::Value::String("success".to_string()),
    );
    envelope.insert(
        "outputs".to_string(),
        serde_json::to_value(outputs).map_err(|e| CliError::Internal(anyhow::anyhow!(e)))?,
    );
    if let Some(view) = completion {
        envelope.insert("completion".to_string(), completion_to_value(view));
    }
    let value = serde_json::Value::Object(envelope);
    Ok(RenderedOutput {
        stdout: Some(crate::frontend::output::json::format_json(&value)?),
        stderr: None,
        is_stdout_first: true,
    })
}

/// Build the `completion` object by hand so `created` and `next_steps` are
/// always present. `WorkflowCompletionView` skips empty arrays on serialize
/// (`skip_serializing_if = "Vec::is_empty"`), so serializing it directly would
/// drop an empty array; the per-row types serialize correctly, so only the
/// wrapper needs hand-building. Serialization of these `String`-only rows is
/// infallible (`json!` mirrors the infallible-serialize pattern used elsewhere
/// in this module), so this returns a plain `Value`.
fn completion_to_value(view: &WorkflowCompletionView) -> serde_json::Value {
    serde_json::json!({
        "created": view.created,
        "next_steps": view.next_steps,
    })
}

/// Render a multi-step workflow `--dry-run` as a JSON envelope: one object per
/// step carrying the same request shape the single-command dry-run JSON
/// renderer emits, reusing `present_dry_run` for parity.
pub(crate) fn render_workflow_dry_run(
    workflow_id: &WorkflowId,
    step_previews: &[StepDryRunPreview],
) -> Result<RenderedOutput, CliError> {
    let steps: Vec<serde_json::Value> = step_previews
        .iter()
        .map(|preview| {
            let view = crate::frontend::presenters::service::present_dry_run(&preview.command);
            let headers: serde_json::Map<String, serde_json::Value> = view
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect();
            let query: serde_json::Map<String, serde_json::Value> = view
                .query
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect();
            serde_json::json!({
                "id": preview.step_id,
                "method": view.http_method,
                "url": view.url,
                "headers": headers,
                "query": query,
                "body": view.body,
            })
        })
        .collect();
    let value = serde_json::json!({
        "workflow": workflow_id.as_str(),
        "dry_run": true,
        "steps": steps,
    });
    Ok(RenderedOutput {
        stdout: Some(crate::frontend::output::json::format_json(&value)?),
        stderr: None,
        is_stdout_first: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::catalogue::HttpMethod;
    use ags_protocol::output_views::WorkflowCompletionView;
    use ags_protocol::result::DryRunResult;
    use ags_protocol::workflow::{CompletionResource, StepDryRunPreview, WorkflowId};
    use std::collections::BTreeMap;

    #[test]
    fn test_render_workflow_catalogue_json_is_array() {
        let entries = vec![WorkflowListEntry {
            id: "competitive-multiplayer".into(),
            name: "Set up competitive multiplayer".into(),
        }];
        let rendered =
            render_workflow_catalogue(&entries, &crate::frontend::RenderOptions::default())
                .unwrap();
        let stdout = rendered.stdout.as_deref().unwrap_or("");
        let parsed: serde_json::Value = serde_json::from_str(stdout).unwrap();
        assert!(parsed.is_array());
        assert_eq!(parsed[0]["id"], "competitive-multiplayer");
        assert_eq!(parsed[0]["name"], "Set up competitive multiplayer");
    }

    #[test]
    fn test_render_workflow_success_envelope_with_completion() {
        let id = WorkflowId::new("competitive-multiplayer");
        let mut outputs = BTreeMap::new();
        outputs.insert("poolName".to_string(), serde_json::json!("ranked-1v1"));
        let completion = Some(WorkflowCompletionView {
            created: vec![CompletionResource {
                label: "Match pool".into(),
                value: "ranked-pool".into(),
            }],
            next_steps: Vec::new(),
        });
        let rendered = render_workflow(
            &id,
            &outputs,
            &["create-pool: ok".to_string()],
            &completion,
            &crate::frontend::RenderOptions::default(),
        )
        .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["workflow"], "competitive-multiplayer");
        assert_eq!(json["status"], "success");
        assert_eq!(json["outputs"]["poolName"], "ranked-1v1");
        assert_eq!(json["completion"]["created"][0]["label"], "Match pool");
        assert_eq!(json["completion"]["created"][0]["value"], "ranked-pool");
        assert_eq!(json["completion"]["next_steps"], serde_json::json!([]));
        assert!(
            json.get("steps").is_none(),
            "success envelope must omit steps"
        );
        assert!(rendered.stderr.is_none());
    }

    #[test]
    fn test_render_workflow_success_envelope_empty_outputs_no_completion() {
        let id = WorkflowId::new("no-outputs");
        let rendered = render_workflow(
            &id,
            &BTreeMap::new(),
            &["step-one: ok".to_string()],
            &None,
            &crate::frontend::RenderOptions::default(),
        )
        .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["outputs"], serde_json::json!({}));
        assert!(
            json.get("completion").is_none(),
            "no completion key when None"
        );
        assert!(json.get("steps").is_none());
    }

    #[test]
    fn test_render_workflow_dry_run_envelope() {
        let id = WorkflowId::new("competitive-multiplayer");
        let preview = StepDryRunPreview {
            step_id: "create-stat".to_string(),
            step_index: 0,
            command: DryRunResult {
                http_method: HttpMethod::Post,
                url: "https://example.test/social/v1/admin/namespaces/dev/stats".to_string(),
                headers: vec![("Authorization".to_string(), "Bearer <token>".to_string())],
                query: vec![],
                body: Some(serde_json::json!({ "statCode": "mmr" })),
            },
            synthesised_outputs: BTreeMap::new(),
        };
        let rendered = render_workflow_dry_run(&id, &[preview]).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["workflow"], "competitive-multiplayer");
        assert_eq!(json["dry_run"], true);
        assert_eq!(json["steps"][0]["id"], "create-stat");
        assert_eq!(json["steps"][0]["method"], "POST");
        assert_eq!(json["steps"][0]["body"]["statCode"], "mmr");
    }
}
