//! Human-readable renderer for `ags doctor` output.

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::RenderOptions;
use crate::frontend::RenderedOutput;
use crate::protocol::diagnostics::{CheckResult, CheckStatus, CheckTier, DoctorResult};

/// Render doctor output as human-readable text
pub(crate) fn render_doctor_output(
    output: &DoctorResult,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let text = render_doctor_text(output);
    Ok(RenderedOutput {
        stdout: Some(text),
        stderr: None,
        is_stdout_first: true,
    })
}

// ── Human rendering ──

/// Render the full doctor report as human-readable text grouped by profile and tier.
fn render_doctor_text(output: &DoctorResult) -> String {
    let color = style::is_stdout_enabled();
    let mut lines = Vec::new();

    for (i, report) in output.reports.iter().enumerate() {
        if output.reports.len() > 1 {
            let header = if report.is_active {
                format!("Profile: {} (active)", report.profile)
            } else {
                format!("Profile: {}", report.profile)
            };
            lines.push(style::apply_tone(&header, style::Tone::Bold, color));
            lines.push(String::new());
        }

        // Find the single failure or most actionable warning (if any).
        // Prefer Auth/Network warnings over Config informational ones.
        let failed_check = report.checks.iter().find(|c| c.status == CheckStatus::Fail);
        let warning_check = report
            .checks
            .iter()
            .find(|c| {
                c.status == CheckStatus::Warning
                    && (c.tier == CheckTier::Auth || c.tier == CheckTier::Network)
            })
            .or_else(|| {
                report
                    .checks
                    .iter()
                    .find(|c| c.status == CheckStatus::Warning)
            });

        // Render passing/skipped checks as detail rows
        render_tier_section(
            &mut lines,
            &report.checks,
            CheckTier::Config,
            "Config",
            color,
        );
        render_tier_section(&mut lines, &report.checks, CheckTier::Auth, "Auth", color);
        render_tier_section(
            &mut lines,
            &report.checks,
            CheckTier::Network,
            "Network",
            color,
        );

        // Headline at the bottom — like auth status
        if let Some(check) = failed_check {
            let headline = format!("{} — {}", check.title, check.message);
            lines.push(style::error(&headline, color));
            if let Some(ref suggestion) = check.suggestion {
                lines.push(format!("{} {suggestion}", style::fix_prefix()));
            }
        } else if let Some(check) = warning_check {
            let headline = format!("{} — {}", check.title, check.message);
            lines.push(style::warning(&headline, color));
            if let Some(ref suggestion) = check.suggestion {
                lines.push(format!("{} {suggestion}", style::fix_prefix()));
            }
        } else {
            lines.push(style::success("All checks passed", color));
        }

        if i + 1 < output.reports.len() {
            lines.push(String::new());
        }
    }

    // Aggregate footer for multi-profile runs
    if output.reports.len() > 1 {
        let total = output.reports.len();
        let failed = output.reports.iter().filter(|r| r.has_failures()).count();
        let warned = output
            .reports
            .iter()
            .filter(|r| !r.has_failures() && r.has_warnings())
            .count();
        let clean = total - failed - warned;
        lines.push(String::new());

        let mut parts = Vec::new();
        if clean > 0 {
            parts.push(format!("{clean} passed"));
        }
        if warned > 0 {
            parts.push(format!("{warned} with warnings"));
        }
        if failed > 0 {
            parts.push(format!("{failed} failed"));
        }
        let summary = format!("{total} profiles: {}", parts.join(", "));
        if failed > 0 {
            lines.push(style::error(&summary, color));
        } else if warned > 0 {
            lines.push(style::warning(&summary, color));
        } else {
            lines.push(style::success(&summary, color));
        }
    }

    lines.join("\n")
}

/// Append a heading and per-check lines for a single tier (e.g. "Required") into the rendered output.
fn render_tier_section(
    lines: &mut Vec<String>,
    checks: &[CheckResult],
    tier: CheckTier,
    heading: &str,
    color: bool,
) {
    let tier_checks: Vec<&CheckResult> = checks.iter().filter(|c| c.tier == tier).collect();
    if tier_checks.is_empty() {
        return;
    }

    let title_width = tier_checks.iter().map(|c| c.title.len()).max().unwrap_or(0);

    lines.push(style::info(heading, color));

    for check in &tier_checks {
        let label_text = format!("{:<width$}  ", check.title, width = title_width);
        match check.status {
            CheckStatus::Pass => {
                lines.push(format!(
                    "    {label_text}{}",
                    style::apply_tone(&check.message, style::Tone::Success, color)
                ));
            }
            CheckStatus::Fail => {
                lines.push(format!(
                    "    {label_text}{}",
                    style::apply_tone(&check.message, style::Tone::Error, color)
                ));
            }
            CheckStatus::Warning => {
                lines.push(format!(
                    "    {label_text}{}",
                    style::apply_tone(&check.message, style::Tone::Warning, color)
                ));
            }
            CheckStatus::Skipped => {
                let label = style::apply_tone(&label_text, style::Tone::Dim, color);
                lines.push(format!(
                    "    {label}{}",
                    style::apply_tone(&check.message, style::Tone::Dim, color)
                ));
            }
        }
    }

    lines.push(String::new());
}
