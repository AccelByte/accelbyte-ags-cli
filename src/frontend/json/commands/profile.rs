//! JSON rendering for profile command output.

use crate::errors::CliError;
use crate::frontend::RenderOptions;
use crate::frontend::RenderedOutput;
use crate::protocol::output::{OperationWarning, ProfileOutput, ProfileView};

/// Render profile command output as JSON
pub(crate) fn render_profile_output(
    output: &ProfileOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    render_profile_view_json(&output.view)
}

/// Render profile output as JSON
fn render_profile_view_json(view: &ProfileView) -> Result<RenderedOutput, CliError> {
    let value = match view {
        ProfileView::List { profiles, active } => {
            let items: Vec<serde_json::Value> = profiles
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "name": p.name,
                        "active": p.is_active,
                    })
                })
                .collect();
            serde_json::json!({
                "profiles": items,
                "active_profile": active,
            })
        }
        ProfileView::Created { name } => {
            serde_json::json!({ "status": "created", "profile": name })
        }
        ProfileView::Switched { name } => {
            serde_json::json!({ "status": "switched", "profile": name })
        }
        ProfileView::NoActiveProfile => {
            serde_json::json!({
                "status": "no_active_profile",
                "profile": null,
            })
        }
        ProfileView::Show {
            name,
            is_active,
            config,
        } => {
            serde_json::json!({
                "profile": name,
                "active": is_active,
                "base_url": config.base_url,
                "client_id": config.client_id,
                "namespace": config.namespace,
                "grant_type": config.grant_type,
                "has_secret": config.has_secret,
                "has_token": config.has_token,
            })
        }
        ProfileView::Deleted {
            name,
            warnings,
            tips,
        } => {
            let mut value = serde_json::json!({ "status": "deleted", "profile": name });
            if !warnings.is_empty() {
                value["warnings"] = warnings_to_json(warnings);
            }
            if !tips.is_empty() {
                value["tips"] = serde_json::json!(tips);
            }
            value
        }
        ProfileView::Renamed { old, new, warnings } => {
            let mut value = serde_json::json!({ "status": "renamed", "old": old, "new": new });
            if !warnings.is_empty() {
                value["warnings"] = warnings_to_json(warnings);
            }
            value
        }
    };

    Ok(RenderedOutput {
        stdout: Some(crate::frontend::json::format_json(&value)?),
        stderr: None,
        is_stdout_first: false,
    })
}

/// Convert warnings to a JSON array for structured output
fn warnings_to_json(warnings: &[OperationWarning]) -> serde_json::Value {
    let items: Vec<serde_json::Value> = warnings
        .iter()
        .map(|w| {
            let mut json_warning = serde_json::json!({
                "message": w.message,
                "fix": w.fix,
            });
            if let Some(reason) = &w.reason {
                json_warning["reason"] = serde_json::Value::String(reason.clone());
            }
            json_warning
        })
        .collect();
    serde_json::Value::Array(items)
}
