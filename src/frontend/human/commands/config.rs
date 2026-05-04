//! Human-readable rendering for config command output.

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::RenderOptions;
use crate::frontend::RenderedOutput;
use crate::protocol::config::{ConfigSource, ResolvedEntry};
use crate::protocol::output::{ConfigOutput, ConfigView};

/// Render config command output as human-readable text
pub fn render_config_output(
    output: &ConfigOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    render_config_view_text(&output.view)
}

/// Render config output as human-readable text
fn render_config_view_text(view: &ConfigView) -> Result<RenderedOutput, CliError> {
    let color = style::is_stderr_enabled();

    let stdout = match view {
        ConfigView::GetAll { profile, entries } => {
            render_all_config_entries(profile, entries, color)
        }
        ConfigView::GetOne { key, value, .. } => render_single_config_value(key, value),
        ConfigView::Set { key, value } => style::success(&format!("{key} = {value}"), color),
        ConfigView::Unset { key } => style::success(&format!("{key} unset"), color),
    };

    Ok(RenderedOutput {
        stdout: Some(stdout),
        stderr: None,
        is_stdout_first: false,
    })
}

/// Render the dump-all config view with source annotations
fn render_all_config_entries(profile: &str, entries: &[ResolvedEntry], color: bool) -> String {
    let mut lines = vec![style::info(
        &format!("Configuration (profile: {profile})"),
        color,
    )];

    let max_key_len = entries.iter().map(|e| e.key.len()).max().unwrap_or(0);

    for entry in entries {
        let padding = " ".repeat(max_key_len - entry.key.len() + 2);
        let value_str = entry.value.as_deref().unwrap_or("not set");
        let source_str = match &entry.source {
            ConfigSource::NotSet => String::new(),
            other => format!("  ({})", source_label(other)),
        };
        lines.push(format!("    {}{padding}{value_str}{source_str}", entry.key));
    }

    lines.join("\n")
}

/// Render a single config key value.
/// Source is intentionally omitted for single-key output (shown only in get-all).
fn render_single_config_value(key: &str, value: &Option<String>) -> String {
    match value {
        Some(v) => v.to_string(),
        None => format!("{key}: not set"),
    }
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
