//! Human-readable rendering for profile command output.

use crate::errors::CliError;
use crate::frontend::human::templates;
use crate::frontend::style;
use crate::frontend::RenderOptions;
use crate::frontend::RenderedOutput;
use crate::protocol::output::{
    OperationWarning, ProfileOutput, ProfileShowData, ProfileSummary, ProfileView,
};

/// Render profile command output as human-readable text
pub fn render_profile_output(
    output: &ProfileOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let color = style::is_stderr_enabled();
    render_profile_view_text(&output.view, color)
}

/// Render profile output as human-readable text
fn render_profile_view_text(view: &ProfileView, color: bool) -> Result<RenderedOutput, CliError> {
    let stdout = match view {
        ProfileView::List { profiles, .. } => render_profile_list(profiles, color),
        ProfileView::NoActiveProfile => {
            let mut lines = vec![style::info("No active profile", color)];
            lines.push(format!(
                "{} Next: Run 'ags profile create <name>' and 'ags profile use <name>' to get started.",
                style::fix_prefix()
            ));
            lines.join("\n")
        }
        ProfileView::Created { name } => {
            style::success(&format!("Profile '{name}' created"), color)
        }
        ProfileView::Switched { name } => {
            style::success(&format!("Switched to profile '{name}'"), color)
        }
        ProfileView::Show {
            name,
            is_active,
            config,
        } => render_profile_details(name, *is_active, config, color),
        ProfileView::Deleted {
            name,
            warnings,
            tips,
        } => {
            let mut lines = vec![style::success(&format!("Profile '{name}' deleted"), color)];
            render_profile_warnings(&mut lines, warnings, color);
            for tip in tips {
                lines.push(templates::render_tip_text(tip, color));
            }
            lines.join("\n")
        }
        ProfileView::Renamed { old, new, warnings } => {
            let mut lines = vec![style::success(
                &format!("Profile '{old}' renamed to '{new}'"),
                color,
            )];
            render_profile_warnings(&mut lines, warnings, color);
            lines.join("\n")
        }
    };

    Ok(RenderedOutput {
        stdout: Some(stdout),
        stderr: None,
        is_stdout_first: false,
    })
}

/// Render the profile list view
fn render_profile_list(profiles: &[ProfileSummary], color: bool) -> String {
    if profiles.is_empty() {
        return format!(
            "{}\n{} Next: Run 'ags profile create <name>' to create a profile.",
            style::info("No profiles found", color),
            style::fix_prefix()
        );
    }

    let mut lines = Vec::new();
    for profile in profiles {
        let name = &profile.name;
        if profile.is_active {
            let active_label = style::apply_tone("(active)", style::Tone::Success, color);
            lines.push(format!("  {name} {active_label}"));
        } else {
            lines.push(format!("  {name}"));
        }
    }

    lines.join("\n")
}

/// Render the profile show view with config fields
fn render_profile_details(
    name: &str,
    is_active: bool,
    config: &ProfileShowData,
    color: bool,
) -> String {
    let active_label = if is_active { " (active)" } else { "" };
    let heading = style::info(&format!("Profile: {name}{active_label}"), color);

    let value_or_unset = |value: &Option<String>| value.as_deref().unwrap_or("not set").to_string();
    let presence_or_unset = |present: bool| if present { "stored" } else { "not set" }.to_string();

    let rows: Vec<(&str, String)> = vec![
        ("Base URL", value_or_unset(&config.base_url)),
        ("Client ID", value_or_unset(&config.client_id)),
        ("Namespace", value_or_unset(&config.namespace)),
        ("Grant type", value_or_unset(&config.grant_type)),
        ("Secret", presence_or_unset(config.has_secret)),
        ("Token", presence_or_unset(config.has_token)),
    ];

    let block =
        templates::render_label_value_block_text(&rows, crate::frontend::style::Tone::Plain, color);

    format!("{heading}\n{block}")
}

/// Append rendered warnings to an output line buffer
fn render_profile_warnings(lines: &mut Vec<String>, warnings: &[OperationWarning], color: bool) {
    for warning in warnings {
        lines.push(templates::render_warning_text(
            &warning.message,
            warning.reason.as_deref(),
            None,
            color,
        ));
        lines.push(format!("{} Fix: {}", style::fix_prefix(), warning.fix));
    }
}
