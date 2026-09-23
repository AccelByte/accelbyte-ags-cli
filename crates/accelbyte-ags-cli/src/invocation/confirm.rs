//! Shared confirmation helper for commands that need interactive
//! confirmation before a destructive or one-way action.

use crate::errors::{CliError, ErrorMetadata};
use crate::invocation::context::FrontendContext;
use crate::invocation::flags::GlobalFlags;

/// The outcome of a successful confirmation check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confirmation {
    /// The user (or `--yes`) confirmed.
    Confirmed,
    /// The user typed something other than `y` / `Y` (including empty input).
    Declined,
}

/// Confirm or refuse an action, respecting automation flags and the
/// frontend context's promptability.
///
/// - `flags.is_auto_confirmed` (`--yes`) returns `Ok(Confirmed)` without
///   reading
/// - `!ctx.allows_input()` returns `Err(CliError::Usage)` with
///   `no_input_message` and a suggestion to use `--yes`, without reading
/// - Otherwise writes `prompt` to stderr, reads one line via `read`, and
///   returns `Ok(Confirmed)` for `y` / `Y` or `Ok(Declined)` for anything
///   else
pub(crate) fn confirm_or_refuse(
    flags: &GlobalFlags,
    ctx: &FrontendContext,
    prompt: &str,
    no_input_message: &str,
    read: &mut dyn FnMut() -> Result<String, CliError>,
) -> Result<Confirmation, CliError> {
    if flags.is_auto_confirmed {
        return Ok(Confirmation::Confirmed);
    }

    if !ctx.allows_input() {
        let reason = crate::invocation::context::input_unavailable_reason(&ctx.terminal);
        return Err(CliError::Usage {
            message: no_input_message.to_string(),
            metadata: Some(Box::new(ErrorMetadata {
                reason: Some(reason.to_string()),
                suggestion: Some("Use --yes to confirm in non-interactive mode".to_string()),
                ..Default::default()
            })),
        });
    }

    crate::frontend::write_stderr(prompt);

    let input = read()?;

    if matches!(input.as_str(), "y" | "Y") {
        Ok(Confirmation::Confirmed)
    } else {
        Ok(Confirmation::Declined)
    }
}

/// Read and trim one line from stdin. Production path only; tests inject
/// a closure that never touches stdin.
pub(crate) fn read_line_from_stdin() -> Result<String, CliError> {
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| CliError::Usage {
            message: format!("Failed to read input: {e}"),
            metadata: None,
        })?;
    Ok(input.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invocation::context::{
        ConsumerKind, FrontendContext, InteractionPolicy, TerminalCapabilities,
    };
    use crate::invocation::flags::{GlobalFlags, UiFlag};

    /// A fully promptable human context: all streams are TTYs and no
    /// automation flag is set, so `allows_input()` is true.
    fn promptable_context() -> FrontendContext {
        FrontendContext {
            consumer: ConsumerKind::Human,
            interaction: InteractionPolicy {
                allow_input: true,
                prefer_rich_ui: false,
                prefer_fullscreen: false,
            },
            terminal: TerminalCapabilities {
                stdin_is_tty: true,
                stdout_is_tty: true,
                stderr_is_tty: true,
                color_force_off: false,
            },
            ui_intent: UiFlag::Auto,
        }
    }

    /// An automation consumer context: `allows_input()` is false.
    fn automation_context() -> FrontendContext {
        FrontendContext {
            consumer: ConsumerKind::Automation,
            interaction: InteractionPolicy {
                allow_input: false,
                prefer_rich_ui: false,
                prefer_fullscreen: false,
            },
            terminal: TerminalCapabilities {
                stdin_is_tty: false,
                stdout_is_tty: false,
                stderr_is_tty: false,
                color_force_off: true,
            },
            ui_intent: UiFlag::Auto,
        }
    }

    #[test]
    fn confirm_or_refuse_rules() {
        use super::Confirmation;

        let no_input_msg = "Upgrading requires confirmation; pass --yes to confirm.";

        // auto-confirm never calls read, returns Confirmed
        {
            let flags = GlobalFlags {
                is_auto_confirmed: true,
                ..GlobalFlags::default()
            };
            let ctx = promptable_context();
            let mut read_called = false;
            let mut read = || -> Result<String, CliError> {
                read_called = true;
                Ok("n".to_string())
            };
            let result =
                confirm_or_refuse(&flags, &ctx, "Continue? [y/N] ", no_input_msg, &mut read);
            assert_eq!(
                result.unwrap(),
                Confirmation::Confirmed,
                "auto-confirm must return Confirmed"
            );
            assert!(!read_called, "auto-confirm must not call read");
        }

        // no-input without yes returns usage error, never calls read
        {
            let flags = GlobalFlags::default();
            let ctx = automation_context();
            let mut read_called = false;
            let mut read = || -> Result<String, CliError> {
                read_called = true;
                Ok("y".to_string())
            };
            let result =
                confirm_or_refuse(&flags, &ctx, "Continue? [y/N] ", no_input_msg, &mut read);
            assert!(!read_called, "no-input must not call read");
            match result {
                Err(CliError::Usage { message, .. }) => {
                    assert_eq!(message, no_input_msg);
                }
                other => panic!("expected Usage error, got {other:?}"),
            }
        }

        // "y" returns Confirmed
        {
            let flags = GlobalFlags::default();
            let ctx = promptable_context();
            let mut read = || -> Result<String, CliError> { Ok("y".to_string()) };
            let result =
                confirm_or_refuse(&flags, &ctx, "Continue? [y/N] ", no_input_msg, &mut read);
            assert_eq!(
                result.unwrap(),
                Confirmation::Confirmed,
                "y must return Confirmed"
            );
        }

        // "Y" returns Confirmed
        {
            let flags = GlobalFlags::default();
            let ctx = promptable_context();
            let mut read = || -> Result<String, CliError> { Ok("Y".to_string()) };
            let result =
                confirm_or_refuse(&flags, &ctx, "Continue? [y/N] ", no_input_msg, &mut read);
            assert_eq!(
                result.unwrap(),
                Confirmation::Confirmed,
                "Y must return Confirmed"
            );
        }

        // "n" returns Ok(Declined), not an error
        {
            let flags = GlobalFlags::default();
            let ctx = promptable_context();
            let mut read = || -> Result<String, CliError> { Ok("n".to_string()) };
            let result =
                confirm_or_refuse(&flags, &ctx, "Continue? [y/N] ", no_input_msg, &mut read);
            assert_eq!(
                result.unwrap(),
                Confirmation::Declined,
                "n must return Declined"
            );
        }

        // empty input returns Ok(Declined), not an error
        {
            let flags = GlobalFlags::default();
            let ctx = promptable_context();
            let mut read = || -> Result<String, CliError> { Ok(String::new()) };
            let result =
                confirm_or_refuse(&flags, &ctx, "Continue? [y/N] ", no_input_msg, &mut read);
            assert_eq!(
                result.unwrap(),
                Confirmation::Declined,
                "empty input must return Declined"
            );
        }
    }
}
