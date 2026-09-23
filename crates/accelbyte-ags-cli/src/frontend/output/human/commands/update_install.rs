//! Human-readable `ags update --install` output rendering.

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::{UpdateInstallAction, UpdateInstallOutput};

/// Render update-install output as human-readable text.
pub(crate) fn render_update_install_output(
    output: &UpdateInstallOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let color = style::is_stdout_enabled();
    let text = match output.action {
        UpdateInstallAction::Installed => render_installed(output, color),
        UpdateInstallAction::AlreadyCurrent => render_already_current(output, color),
        UpdateInstallAction::DryRun => render_dry_run(output),
    };
    Ok(RenderedOutput {
        stdout: Some(text),
        stderr: None,
        is_stdout_first: true,
    })
}

/// `Installed`: `✔ ags <latest> installed at <binary_path> (was <previous>)`
fn render_installed(output: &UpdateInstallOutput, color: bool) -> String {
    let latest = output.latest.as_deref().unwrap_or("unknown");
    let text = format!(
        "{} ags {} installed at {} (was {})",
        style::text::SYMBOL_SUCCESS,
        latest,
        output.binary_path,
        output.previous,
    );
    style::apply_tone(&text, style::Tone::Success, color)
}

/// `AlreadyCurrent`: `✔ ags <previous> is the latest release. Nothing to install.`
fn render_already_current(output: &UpdateInstallOutput, color: bool) -> String {
    let text = format!(
        "{} ags {} is the latest release. Nothing to install.",
        style::text::SYMBOL_SUCCESS,
        output.previous,
    );
    style::apply_tone(&text, style::Tone::Success, color)
}

/// `DryRun`: three lines naming the URL, the environment, and the binary.
fn render_dry_run(output: &UpdateInstallOutput) -> String {
    let url = output.installer_url.as_deref().unwrap_or("<unknown>");

    let env_pairs: String = output
        .installer_env
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");

    let mut lines = Vec::with_capacity(3);
    lines.push(format!("Would download {url}"));
    lines.push(format!("Would run it with {env_pairs}"));
    lines.push(format!(
        "Would replace {} ({}) with the latest release",
        output.binary_path, output.previous,
    ));

    lines.join("\n")
}
