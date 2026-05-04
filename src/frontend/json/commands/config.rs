//! JSON rendering for config command output.

use crate::errors::CliError;
use crate::frontend::RenderOptions;
use crate::frontend::RenderedOutput;
use crate::protocol::config::ConfigSource;
use crate::protocol::output::{ConfigOutput, ConfigView};

/// Render config command output as JSON
pub(crate) fn render_config_output(
    output: &ConfigOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    render_config_view_json(&output.view)
}

/// Render config output as JSON
fn render_config_view_json(view: &ConfigView) -> Result<RenderedOutput, CliError> {
    let value = match view {
        ConfigView::GetAll { profile, entries } => {
            let items: Vec<serde_json::Value> = entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "key": e.key,
                        "value": e.value,
                        "source": source_label(&e.source),
                    })
                })
                .collect();
            serde_json::json!({
                "profile": profile,
                "config": items,
            })
        }
        ConfigView::GetOne { key, value, source } => {
            serde_json::json!({
                "key": key,
                "value": value,
                "source": source_label(source),
            })
        }
        ConfigView::Set { key, value } => {
            serde_json::json!({ "status": "set", "key": key, "value": value })
        }
        ConfigView::Unset { key } => {
            serde_json::json!({ "status": "unset", "key": key })
        }
    };

    Ok(RenderedOutput {
        stdout: Some(crate::frontend::json::format_json(&value)?),
        stderr: None,
        is_stdout_first: false,
    })
}

/// Convert a ConfigSource to a display label
fn source_label(source: &ConfigSource) -> String {
    match source {
        ConfigSource::Environment => "environment".to_string(),
        ConfigSource::Profile(name) => format!("profile:{name}"),
        ConfigSource::Global => "global".to_string(),
        ConfigSource::NotSet => "not set".to_string(),
    }
}
