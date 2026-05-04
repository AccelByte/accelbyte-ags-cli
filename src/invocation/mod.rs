//! Command layer: argument parsing, routing, and orchestration.

pub mod builder;
pub(crate) mod clap_helpers;
pub mod commands;
pub mod errors;
pub mod flags;
pub mod resolve;
mod router;

use crate::errors::CliError;
use crate::frontend;

pub(crate) enum InvocationOutcome {
    Complete,
    Exit(i32),
}

/// Emit a single status line to stderr indicating that another process holds the named file lock.
fn report_lock_contention(lock_name: &str) {
    crate::frontend::write_stderr_line(&format!("  Waiting for file lock on {lock_name}\u{2026}"));
}

/// CLI entry point: pre-scan global flags, route to auth/version/service handlers.
///
/// All errors that happen after the frontend is constructed are routed through
/// `Frontend::render_error`. Only errors from `frontend.enter()` itself propagate
/// up to `main` — the bare-stderr escape hatch for the one case where the
/// frontend could not be initialised.
pub async fn run() -> Result<(), CliError> {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let pre_scan_result = flags::pre_scan_global_flags(&raw_args);

    // Initialise styling before any error rendering so the colour/no-color
    // pre-scan is honoured for validation errors that fire from the pre-scan
    // itself (e.g. --namespace "", --timeout abc).
    let no_color_pre = raw_args.iter().any(|arg| arg == "--no-color");
    frontend::style::init(no_color_pre);

    let (mut flags, remaining) = match pre_scan_result {
        Ok(value) => value,
        Err(error) => {
            // Pre-scan errors happen before we can pick a frontend, so render
            // through a default human frontend rather than falling through to
            // main.rs's bare-stderr `ags:` escape hatch.
            let mut frontend = crate::frontend::select(
                crate::frontend::FrontendKind::Human,
                crate::protocol::request::Verbosity::default(),
            );
            let _ = frontend.enter();
            frontend.render_error(&error);
            let _ = frontend.exit();
            std::process::exit(error.exit_code());
        }
    };
    flags::apply_config_defaults(&mut flags);
    crate::runtime::bootstrap();

    let frontend_kind = if flags.format == Some(crate::protocol::request::OutputFormat::Json) {
        crate::frontend::FrontendKind::Json
    } else {
        crate::frontend::FrontendKind::Human
    };
    if matches!(frontend_kind, crate::frontend::FrontendKind::Human) {
        crate::support::register_lock_contention_reporter(report_lock_contention);
    }
    let mut frontend = crate::frontend::select(frontend_kind, flags.verbosity);
    frontend.enter()?;

    let result = router::dispatch(frontend.as_mut(), &raw_args, &mut flags, &remaining).await;

    // Tear down the frontend before rendering any error so a TUI alt-screen
    // is exited before stderr output.
    let _ = frontend.exit();

    match result {
        Err(e) => {
            frontend.render_error(&e);
            std::process::exit(e.exit_code());
        }
        Ok(InvocationOutcome::Exit(code)) => std::process::exit(code),
        Ok(InvocationOutcome::Complete) => {}
    }
    Ok(())
}
