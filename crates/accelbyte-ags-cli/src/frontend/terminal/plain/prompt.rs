//! Interactive confirmation prompts for mutating operations.

use crate::errors::CliError;
use crate::frontend::style;

/// Prompt for a line of text: writes `label` (without a trailing newline) so
/// the user's input appears on the same line after the colon, then reads from
/// stdin. Returns the trimmed value. Mirrors `ags auth login` — no description
/// hint (descriptions live in `--help`).
pub(crate) fn gather_text(label: &str) -> Result<String, CliError> {
    crate::frontend::write_stderr(label);
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| CliError::Internal(anyhow::anyhow!(e)))?;
    Ok(line.trim().to_string())
}

/// Like [`gather_text`] but reads the value without echoing it, for sensitive
/// inputs (mirrors `ags auth login`'s client-secret prompt).
pub(crate) fn gather_secret(label: &str) -> Result<String, CliError> {
    let value = rpassword::prompt_password(label).map_err(|e| CliError::Usage {
        message: format!("Failed to read input: {e}"),
        metadata: None,
    })?;
    Ok(value.trim().to_string())
}

/// Display the workflow step context on stderr and read the user's choice.
///
/// Header is `! Step N: <id>` in the same `{glyph} Step N: {id}` family as the
/// step-outcome lines, followed by the step description (the user-facing action
/// and risk). The request method and URL are backend detail and are not shown.
///
/// For optional steps a three-way `[y] run / [s] skip / [n] cancel` prompt is
/// shown; for non-optional steps the existing `[y/N]` yes/no prompt is kept and
/// `s` is **not** treated as skip (it cancels, matching the invalid-input
/// behaviour).
pub(crate) fn confirm_step(
    preview: &ags_protocol::workflow::StepPreview,
    optional: bool,
) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
    confirm_step_impl(preview, optional, &mut read_line_from_stdin)
}

/// Inner implementation with an injected line reader so tests never touch stdin.
fn confirm_step_impl(
    preview: &ags_protocol::workflow::StepPreview,
    optional: bool,
    read: &mut dyn FnMut() -> Result<String, CliError>,
) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
    use ags_protocol::workflow::StepConfirmOutcome;

    // `style::warning` supplies the "!" glyph and the warning colour.
    crate::frontend::write_stderr_line(&style::warning(
        &format!("Step {}: {}", preview.step_index + 1, preview.step_id),
        style::is_stderr_enabled(),
    ));
    // Description in the default colour — it is the important context here, not
    // secondary detail.
    crate::frontend::write_stderr_line(&format!("    {}", preview.step_label));

    if optional {
        crate::frontend::write_stderr("Run this step? [y] run  [s] skip  [n] cancel ");
        let input = read()?;
        Ok(match input.chars().next().map(|c| c.to_ascii_lowercase()) {
            Some('y') => StepConfirmOutcome::Proceed,
            Some('s') => StepConfirmOutcome::Skip,
            _ => StepConfirmOutcome::Cancel,
        })
    } else {
        crate::frontend::write_stderr("Continue? [y/N] ");
        let input = read()?;
        // Only 'y' proceeds; 's' and every other character cancel — the same
        // invalid-input behaviour the legacy yes/no prompt had.
        Ok(match input.chars().next().map(|c| c.to_ascii_lowercase()) {
            Some('y') => StepConfirmOutcome::Proceed,
            _ => StepConfirmOutcome::Cancel,
        })
    }
}

/// Parse a failure-gate key. `allow_skip` gates the `s` option. Returns `None`
/// for an unrecognised key so the caller can re-prompt.
pub(crate) fn parse_failure_choice(
    input: &str,
    allow_skip: bool,
) -> Option<ags_protocol::workflow::StepFailureAction> {
    use ags_protocol::workflow::StepFailureAction::*;
    match input.trim().to_ascii_lowercase().as_str() {
        "r" => Some(Retry),
        "s" if allow_skip => Some(Skip),
        "c" => Some(Cancel),
        _ => None,
    }
}

/// Render a failed step's error and prompt for Retry / Skip / Cancel.
pub(crate) fn resolve_step_failure(
    error: &ags_protocol::error::RuntimeError,
    allow_skip: bool,
) -> Result<ags_protocol::workflow::StepFailureAction, CliError> {
    resolve_step_failure_impl(error, allow_skip, &mut read_line_from_stdin)
}

fn resolve_step_failure_impl(
    error: &ags_protocol::error::RuntimeError,
    allow_skip: bool,
    read: &mut dyn FnMut() -> Result<String, CliError>,
) -> Result<ags_protocol::workflow::StepFailureAction, CliError> {
    use ags_protocol::workflow::StepFailureAction;

    crate::frontend::write_stderr_line(&style::warning(
        &format!("Step failed: {}", error.message),
        style::is_stderr_enabled(),
    ));
    if let Some(reason) = error.reason_detail() {
        crate::frontend::write_stderr_line(&format!("    {reason}"));
    }

    let prompt = if allow_skip {
        "[r] retry  [s] skip  [c] cancel "
    } else {
        "[r] retry  [c] cancel "
    };
    loop {
        crate::frontend::write_stderr(prompt);
        let input = read()?;
        // An empty line (EOF or a bare Enter) gives up, so an exhausted stdin
        // cannot spin the loop forever.
        if input.trim().is_empty() {
            return Ok(StepFailureAction::Cancel);
        }
        if let Some(action) = parse_failure_choice(&input, allow_skip) {
            return Ok(action);
        }
        // Unrecognised key: re-prompt.
    }
}

/// Read and trim one line from stdin.
fn read_line_from_stdin() -> Result<String, CliError> {
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|error| CliError::Usage {
            message: format!("Failed to read input: {error}"),
            metadata: None,
        })?;
    Ok(input.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::confirm_step_impl;
    use ags_protocol::catalogue::{HttpMethod, MutationClass, OperationId, ServiceId};
    use ags_protocol::result::CommandPreview;
    use ags_protocol::workflow::{StepConfirmOutcome, StepPreview};

    fn make_preview() -> StepPreview {
        StepPreview {
            workflow_name: "test-workflow".to_string(),
            step_id: "step-1".to_string(),
            step_label: "Do a thing".to_string(),
            step_index: 0,
            step_total: 1,
            command: CommandPreview {
                service: ServiceId::new("iam"),
                operation_id: OperationId::new("testOp"),
                summary: "Test".to_string(),
                http_method: HttpMethod::Post,
                url: "https://example.com/test".to_string(),
                mutation_class: MutationClass::Mutating,
                confirmation_required: true,
                warnings: vec![],
            },
        }
    }

    fn scripted(answer: &str) -> impl FnMut() -> Result<String, crate::errors::CliError> + '_ {
        let mut called = false;
        move || {
            assert!(!called, "read called more than once");
            called = true;
            Ok(answer.to_string())
        }
    }

    #[test]
    fn test_confirm_step_optional_y_yields_proceed() {
        let result = confirm_step_impl(&make_preview(), true, &mut scripted("y")).unwrap();
        assert_eq!(result, StepConfirmOutcome::Proceed);
    }

    #[test]
    fn test_confirm_step_optional_s_yields_skip() {
        let result = confirm_step_impl(&make_preview(), true, &mut scripted("s")).unwrap();
        assert_eq!(result, StepConfirmOutcome::Skip);
    }

    #[test]
    fn test_confirm_step_optional_n_yields_cancel() {
        let result = confirm_step_impl(&make_preview(), true, &mut scripted("n")).unwrap();
        assert_eq!(result, StepConfirmOutcome::Cancel);
    }

    #[test]
    fn test_confirm_step_non_optional_y_yields_proceed() {
        let result = confirm_step_impl(&make_preview(), false, &mut scripted("y")).unwrap();
        assert_eq!(result, StepConfirmOutcome::Proceed);
    }

    #[test]
    fn test_confirm_step_non_optional_n_yields_cancel() {
        let result = confirm_step_impl(&make_preview(), false, &mut scripted("n")).unwrap();
        assert_eq!(result, StepConfirmOutcome::Cancel);
    }

    /// `s` on a non-optional step must NOT skip — it cancels (same
    /// invalid-input behaviour as the legacy yes/no prompt).
    #[test]
    fn test_confirm_step_non_optional_s_does_not_skip() {
        let result = confirm_step_impl(&make_preview(), false, &mut scripted("s")).unwrap();
        assert_ne!(
            result,
            StepConfirmOutcome::Skip,
            "s must not skip a non-optional step"
        );
        assert_eq!(result, StepConfirmOutcome::Cancel);
    }

    #[test]
    fn test_failure_choice_parses_r_s_c() {
        use ags_protocol::workflow::StepFailureAction::*;
        assert!(matches!(
            super::parse_failure_choice("r", true),
            Some(Retry)
        ));
        assert!(matches!(super::parse_failure_choice("s", true), Some(Skip)));
        assert!(matches!(
            super::parse_failure_choice("c", true),
            Some(Cancel)
        ));
        // Skip not allowed → 's' rejected.
        assert!(super::parse_failure_choice("s", false).is_none());
        assert!(super::parse_failure_choice("x", true).is_none());
    }
}
