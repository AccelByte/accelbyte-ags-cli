//! Human-readable rendering for config command output.

use crate::errors::CliError;
use crate::frontend::output::templates::config_source_label;
use crate::frontend::style;
use crate::frontend::RenderOptions;
use crate::frontend::RenderedOutput;
use ags_protocol::config::{ConfigSource, ResolvedEntry};
use ags_protocol::output::{ConfigOutput, ConfigView};

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
        ConfigView::GetOne {
            key,
            value,
            source,
            read_only,
        } => render_single_config_value(key, value, source, *read_only),
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
        let value_str = entry_value_display(entry);
        let source_str = match &entry.source {
            ConfigSource::NotSet => String::new(),
            other => format!("  ({})", config_source_label(other)),
        };
        lines.push(format!("    {}{padding}{value_str}{source_str}", entry.key));
    }

    lines.join("\n")
}

/// The value-column text for a get-all entry. Read-only entries (the keychain
/// client secret) never show a value — only masked presence — so the secret is
/// never printed.
fn entry_value_display(entry: &ResolvedEntry) -> String {
    if entry.read_only {
        return masked_presence(&entry.source);
    }
    entry.value.as_deref().unwrap_or("not set").to_string()
}

/// Masked display for a read-only secret: bullets when set, "not set" otherwise,
/// always flagged read-only.
fn masked_presence(source: &ConfigSource) -> String {
    if matches!(source, ConfigSource::NotSet) {
        "not set (read-only)".to_string()
    } else {
        "•••••••• (read-only)".to_string()
    }
}

/// Render a single config key value. Source is normally omitted for single-key
/// output (shown only in get-all), but a read-only secret shows its masked
/// presence and source so the user learns where it lives.
fn render_single_config_value(
    key: &str,
    value: &Option<String>,
    source: &ConfigSource,
    read_only: bool,
) -> String {
    if read_only {
        return match source {
            ConfigSource::NotSet => format!("{key}: {}", masked_presence(source)),
            other => format!(
                "{} ({})",
                masked_presence(source),
                config_source_label(other)
            ),
        };
    }
    match value {
        Some(v) => v.to_string(),
        None => format!("{key}: not set"),
    }
}
