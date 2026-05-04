use crate::errors::CliError;
use crate::invocation::{builder, commands, flags, InvocationOutcome};

/// Parse the page-limit flag, then route to the appropriate command handler.
/// Pulled out of `run` so every error here flows through `frontend.render_error`.
pub(super) async fn dispatch(
    frontend: &mut dyn crate::frontend::Frontend,
    raw_args: &[String],
    flags: &mut flags::GlobalFlags,
    remaining: &[String],
) -> Result<InvocationOutcome, CliError> {
    if let Some(raw) = flags.page_limit_raw.take() {
        match raw.parse::<u64>() {
            Ok(limit) if (1..=100).contains(&limit) => flags.page_limit = Some(limit),
            _ => {
                return Err(CliError::Usage {
                    message: format!("Invalid page limit '{raw}'"),
                    metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                        "Use a value between 1 and 100",
                    ))),
                });
            }
        }
    }

    if raw_args.iter().any(|arg| arg == "--version" || arg == "-V") {
        commands::version::handle_version(flags, frontend)?;
        Ok(InvocationOutcome::Complete)
    } else {
        route(flags, remaining, frontend).await
    }
}

/// Match the leading positional arg against the static and dynamic command sets and dispatch.
async fn route(
    flags: &flags::GlobalFlags,
    remaining: &[String],
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    if remaining.is_empty() {
        let mut command = builder::build_root_command();
        let _ = command.print_help();
        return Ok(InvocationOutcome::Exit(1));
    }

    let first = &remaining[0];

    if first == "--help" || first == "-h" || first == "help" {
        let mut command = builder::build_root_command();
        let _ = command.print_help();
        return Ok(InvocationOutcome::Complete);
    }

    if first == "auth" {
        return commands::auth::handle_auth(&remaining[1..], flags, frontend).await;
    }

    if first == "completions" {
        return commands::completions::handle_completions(&remaining[1..], flags, frontend);
    }

    if first == "config" {
        return commands::config::handle_config(&remaining[1..], flags, frontend).await;
    }

    if first == "profile" {
        return commands::profile::handle_profile(&remaining[1..], flags, frontend).await;
    }

    if first == "describe" {
        return commands::describe::handle_describe(&remaining[1..], flags, frontend);
    }

    if first == "doctor" {
        return commands::doctor::handle_doctor(&remaining[1..], flags, frontend).await;
    }

    if first == "refresh-specs" {
        return commands::refresh_specs::handle_refresh_specs(&remaining[1..], flags, frontend);
    }

    if first == "version" {
        commands::version::handle_version(flags, frontend)?;
        return Ok(InvocationOutcome::Complete);
    }

    if first.starts_with('-') {
        return Err(CliError::Usage {
            message: format!("Unknown flag: '{first}'"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Run 'ags --help' for a list of available flags.",
            ))),
        });
    }

    commands::service::handle_service(first, &remaining[1..], flags, frontend).await
}
