//! Command layer: argument parsing, routing, and orchestration.

pub mod builder;
pub(crate) mod clap_helpers;
pub mod completions_generator;
pub mod context;
pub mod errors;
pub mod flags;
pub mod handlers;
mod phase_execution;
pub mod policy;
pub mod resolve;
mod router;
pub mod routes;
pub mod shape;
pub mod workflows;

use crate::errors::CliError;
use crate::frontend;

#[derive(Debug)]
pub(crate) enum InvocationOutcome {
    Complete,
    Exit(i32),
    /// A workflow was cancelled by the user declining a confirmation prompt.
    /// Exits with code 2; recorded as `RunOutcome::Cancelled` in `RunFinished`.
    Cancelled,
}

/// Map an invocation result to the lifecycle `RunOutcome` recorded in the
/// `RunFinished` event. Cancellation is threaded explicitly via
/// `InvocationOutcome::Cancelled` — never inferred from an exit code.
pub(crate) fn run_outcome_for(
    result: &Result<InvocationOutcome, CliError>,
) -> crate::frontend::RunOutcome {
    match result {
        Ok(InvocationOutcome::Cancelled) => crate::frontend::RunOutcome::Cancelled,
        Ok(_) => crate::frontend::RunOutcome::Success,
        Err(_) => crate::frontend::RunOutcome::Failed,
    }
}

/// Emit a single status line to stderr indicating that another process holds the named file lock.
fn report_lock_contention(lock_name: &str) {
    crate::frontend::write_stderr_line(&format!("  Waiting for file lock on {lock_name}\u{2026}"));
}

/// Register the plain lock-contention reporter only when the finalized
/// surface is plain. TUI runs must not register it — a mid-run "Waiting
/// for file lock…" write to stderr would corrupt the frame.
///
/// Gates strictly on `PlainTerminal` (NOT `factory_renders_plain`): the
/// phase-owned routes that call this build a REAL fullscreen surface for a
/// finalized `FullscreenTerminalUi` (via `select_fullscreen_workflow_surfaces`),
/// so fullscreen must skip here. The builtin gate is the degrade case and uses
/// `factory_renders_plain` instead, because builtins go through
/// `frontend_for_surface`, which degrades fullscreen to a plain frontend.
///
/// Service/workflow routes call this AFTER `finalize_surface`, which runs after
/// the runtime prologue. So token-lock contention DURING the prologue is no
/// longer surfaced on plain runs (the old startup registration covered it).
/// Accepted on purpose: the surface is unknown until finalize (it needs the
/// compiled workflow), and registering earlier — before we know the surface —
/// would arm the raw-stderr reporter for inline/fullscreen runs too, which a
/// mid-run lock refresh could then use to corrupt the live frame. TUI safety
/// wins over the narrow prologue-window message.
pub(crate) fn register_reporter_if_plain(ctx: &context::FrontendContext) {
    if matches!(ctx.surface_backend(), context::PhaseBackend::PlainTerminal) {
        ags_runtime::support::register_lock_contention_reporter(report_lock_contention);
    }
}

/// Top-level entry for `workflow run`. Validates the page-limit flag (the
/// rest of the validation lives in `route_workflow_run`), then hands off to
/// the self-owning workflow-run handler. Page-limit validation is shared with
/// the builtin route via `router::parse_page_limit`, so both paths
/// reject an out-of-range limit identically. `--version`/`-V` is handled by
/// the caller before this is reached.
async fn run_workflow_run(
    flags: &mut flags::GlobalFlags,
    options: &crate::frontend::RenderOptions,
    frontend_context: &context::FrontendContext,
    remaining: &[String],
) -> Result<InvocationOutcome, CliError> {
    router::parse_page_limit(flags)?;
    // `remaining` is `["workflow", "run", <rest>]`; pass only `<rest>`.
    routes::workflow::route_workflow_run(&remaining[2..], flags, options.clone(), frontend_context)
        .await
}

/// Top-level entry for a synthesised service command. Mirrors
/// `run_workflow_run`: validates the page-limit flag (the builtin route
/// does the same in `route_builtin`), then hands off to the self-owning
/// service handler. `remaining` is `[<service>, <rest>...]`.
async fn run_service(
    flags: &mut flags::GlobalFlags,
    options: &crate::frontend::RenderOptions,
    frontend_context: &context::FrontendContext,
    remaining: &[String],
) -> Result<InvocationOutcome, CliError> {
    router::parse_page_limit(flags)?;
    routes::service::route_service(
        &remaining[0],
        &remaining[1..],
        flags,
        options.clone(),
        frontend_context,
    )
    .await
}

/// Top-level entry for the `auth` path. Mirrors `run_workflow_run`
/// and `run_service`: validates the page-limit flag, then hands off to the
/// bespoke auth-path wrapper. Unlike those two, auth is NOT executor-backed
/// — `route_auth` is a small auth-specific self-owned wrapper that
/// owns frontend construction and the login-only run lifecycle. `remaining`
/// is `["auth", <rest>...]`; only `<rest>` is passed on.
async fn run_auth(
    flags: &mut flags::GlobalFlags,
    backend: crate::invocation::context::PhaseBackend,
    options: &crate::frontend::RenderOptions,
    frontend_context: &context::FrontendContext,
    remaining: &[String],
) -> Result<InvocationOutcome, CliError> {
    router::parse_page_limit(flags)?;
    routes::auth::route_auth(
        &remaining[1..],
        flags,
        backend,
        options.clone(),
        frontend_context,
    )
    .await
}

/// Shared result post-processing for the self-owned execution paths
/// (`workflow run`, synthesised service commands, the `auth` path, and the
/// builtin route). A pre-surface failure renders through a fresh human
/// frontend; the surface-owned outcomes drive the process exit.
///
/// This fits the bespoke auth wrapper too: a page-limit failure in `run_auth`
/// is returned as `Err` and rendered here, while every surface-owned auth
/// outcome — including a profile-resolution failure caught inside
/// `route_auth` and rendered on the owned frontend — is returned as
/// `Ok(Exit(error.exit_code()))` so its real exit code is preserved, never a
/// hardcoded `Exit(1)`.
fn finish_self_owned(result: Result<InvocationOutcome, CliError>) -> Result<(), CliError> {
    match result {
        Err(e) => {
            // Pre-surface failure: render through a fresh human frontend,
            // exactly as the pre-scan/resolver failure paths do.
            let mut frontend = crate::frontend::frontend_for_surface(
                crate::invocation::context::PhaseBackend::PlainTerminal,
                crate::frontend::RenderOptions::default(),
            )?;
            frontend.render_error(&e);
            let _ = frontend.finish();
            std::process::exit(e.exit_code());
        }
        Ok(InvocationOutcome::Exit(code)) => std::process::exit(code),
        Ok(InvocationOutcome::Cancelled) => std::process::exit(2),
        Ok(InvocationOutcome::Complete) => Ok(()),
    }
}

/// CLI entry point: pre-scan global flags, then classify the root invocation
/// and hand off to the matching self-owned execution path
/// (`workflow run`, service, `auth`, or the builtin route via `route_builtin`).
///
/// All errors that happen after the frontend is constructed are routed through
/// `Frontend::render_error`. Only errors from
/// `frontend::frontend_for_surface` itself (e.g. inline terminal acquisition
/// failure) propagate up to `main` — the bare-stderr escape hatch for the one
/// case where the frontend could not be initialised.
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
            let mut frontend = crate::frontend::frontend_for_surface(
                crate::invocation::context::PhaseBackend::PlainTerminal,
                crate::frontend::RenderOptions::default(),
            )?;
            frontend.render_error(&error);
            let _ = frontend.finish();
            std::process::exit(error.exit_code());
        }
    };
    flags::apply_config_defaults(&mut flags);
    ags_runtime::runtime::bootstrap();

    // `--version`/`-V` and the help paths are meta-commands that never render
    // on a rich UI surface (the builtin route degrades to plain chrome anyway),
    // so a `--ui` gate failure from the resolver must not
    // suppress them — `ags --version --ui=fullscreen | cat` must still print
    // the version, not refuse with a "rich terminal UI cannot be shown" error.
    let is_meta_builtin = raw_args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--version" | "-V" | "--help" | "-h"));

    // `frontend_context` is the resolved source of presentation backend and
    // interaction policy; it is threaded into the router for handlers
    // that gate on promptability.
    let frontend_context = match context::resolve_frontend_context(&flags) {
        Ok(ctx) => ctx,
        Err(_) if is_meta_builtin => context::resolve_plain_meta_context(&flags),
        Err(err) => {
            // A resolver failure is a usage error — render it through a
            // Human frontend before exiting, since no frontend exists yet.
            let mut frontend = crate::frontend::frontend_for_surface(
                crate::invocation::context::PhaseBackend::PlainTerminal,
                crate::frontend::RenderOptions::default(),
            )?;
            frontend.render_error(&err);
            let _ = frontend.finish();
            std::process::exit(err.exit_code());
        }
    };
    let backend = frontend_context.surface_backend();

    let options = crate::frontend::RenderOptions {
        verbosity: flags.verbosity,
        is_page_all: flags.is_page_all,
        output: flags.output.clone(),
    };

    // Every root execution path is self-owned: each constructs its own
    // presentation surface(s) and owns its run lifecycle, so the top level
    // does not build a frontend for any of them. `workflow run` and service
    // are executor-backed and phase-aware; `auth` uses a bespoke (non-
    // executor) wrapper that brackets only the real `auth login` flow; the
    // builtin route (`workflow list`, the built-in commands, the `--help`
    // paths, and `--version`/`-V`) runs the `route_builtin` wrapper. The
    // `--version`/`-V` short-circuit still wins: it bypasses `classify_root`
    // and is dispatched by `route_builtin` itself.
    let is_version = raw_args.iter().any(|arg| arg == "--version" || arg == "-V");
    if !is_version {
        match router::classify_root(&remaining) {
            router::RootDispatch::WorkflowRun => {
                let result =
                    run_workflow_run(&mut flags, &options, &frontend_context, &remaining).await;
                return finish_self_owned(result);
            }
            router::RootDispatch::Service => {
                let result = run_service(&mut flags, &options, &frontend_context, &remaining).await;
                return finish_self_owned(result);
            }
            router::RootDispatch::Auth => {
                let result =
                    run_auth(&mut flags, backend, &options, &frontend_context, &remaining).await;
                return finish_self_owned(result);
            }
            router::RootDispatch::Builtin => {}
        }
    }

    // Builtin route (incl. `--version`): the builtin route owns its frontend
    // and run lifecycle via `route_builtin`. `options` is consumed by value here
    // — the `!is_version` branches above all return before reaching this,
    // so the by-reference borrows in `run_*` and this move never conflict.
    //
    // Plain chrome is what PlainTerminal renders AND what FullscreenTerminalUi
    // degrades to in frontend_for_surface. Builtins take file locks (config,
    // profile, refresh-specs), so register the reporter when the factory will
    // build a plain frontend. Inline → InlineFrontend (no plain chrome) and
    // StructuredJson → JSON both skip. Keyed off the same predicate the factory
    // uses so the two can't drift.
    if crate::frontend::factory_renders_plain(backend) {
        ags_runtime::support::register_lock_contention_reporter(report_lock_contention);
    }
    let result =
        routes::builtin::route_builtin(&mut flags, backend, options, &raw_args, &remaining).await;
    finish_self_owned(result)
}

#[cfg(test)]
mod run_outcome_tests {
    use super::{run_outcome_for, InvocationOutcome};
    use crate::errors::CliError;
    use crate::frontend::RunOutcome;

    #[test]
    fn test_run_outcome_for_complete_is_success() {
        let result: Result<InvocationOutcome, CliError> = Ok(InvocationOutcome::Complete);
        assert_eq!(run_outcome_for(&result), RunOutcome::Success);
    }

    #[test]
    fn test_run_outcome_for_exit_is_success() {
        let result: Result<InvocationOutcome, CliError> = Ok(InvocationOutcome::Exit(1));
        assert_eq!(run_outcome_for(&result), RunOutcome::Success);
    }

    #[test]
    fn test_run_outcome_for_cancelled_is_cancelled() {
        let result: Result<InvocationOutcome, CliError> = Ok(InvocationOutcome::Cancelled);
        assert_eq!(run_outcome_for(&result), RunOutcome::Cancelled);
    }

    #[test]
    fn test_run_outcome_for_err_is_failed() {
        let result: Result<InvocationOutcome, CliError> = Err(CliError::Usage {
            message: "x".into(),
            metadata: None,
        });
        assert_eq!(run_outcome_for(&result), RunOutcome::Failed);
    }
}
