//! Shared helpers for the Clap-driven command handlers.

use crate::errors::CliError;
use crate::invocation::InvocationOutcome;

/// Convert a Clap parse error into the appropriate `InvocationOutcome`.
///
/// `--help` and missing-arg-help paths print Clap's help text directly and exit
/// cleanly. Other parse errors (unknown subcommand, missing required arg, etc.)
/// are surfaced as `CliError::Usage` so they flow through `Frontend::render_error`,
/// matching the formatting used by the dynamic-service path (`✕ <Capitalized>`,
/// no leading `error: ` prefix). Without this, top-level subcommands would
/// render Clap's default `error: unrecognized subcommand` while service-level
/// errors render `✕ Unrecognized subcommand` — the inconsistency users noticed.
pub(crate) fn outcome_from_clap_error(error: clap::Error) -> Result<InvocationOutcome, CliError> {
    if error.kind() == clap::error::ErrorKind::DisplayHelp
        || error.kind() == clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    {
        let _ = error.print();
        return Ok(InvocationOutcome::Complete);
    }
    Err(CliError::Usage {
        message: strip_clap_prefix(&error.to_string()),
        metadata: None,
    })
}

/// Strip Clap's `error: ` prefix and capitalize the next char so messages
/// render uniformly as `✕ <Capitalized>` after the frontend prepends its
/// error symbol. Used by every Clap entry point in this crate.
pub(crate) fn strip_clap_prefix(message: &str) -> String {
    let stripped = message.strip_prefix("error: ").unwrap_or(message);
    let mut chars = stripped.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            upper + chars.as_str()
        }
    }
}

/// Build the `argv` vector that `try_get_matches_from_mut` expects: the
/// subcommand name as `argv[0]`, then the remaining user-supplied args.
pub(crate) fn build_argv(name: &str, args: &[String]) -> Vec<String> {
    std::iter::once(name.to_string())
        .chain(args.iter().cloned())
        .collect()
}
