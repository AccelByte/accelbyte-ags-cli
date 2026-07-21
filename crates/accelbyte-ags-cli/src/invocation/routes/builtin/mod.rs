//! Built-in / help-like execution path.
//!
//! This covers root commands that are not phase-owned runs and not the bespoke
//! `auth` path. Root help/version exits are lifecycle-free; everything else
//! in this path is wrapped in `RunStarted` / `RunFinished`.

use crate::errors::CliError;
use crate::invocation::{builder, flags, handlers, InvocationOutcome};

/// Whether `first` names a built-in command dispatched by the builtin route.
///
/// `classify_root` consults this, so the builtin inventory cannot drift from
/// what `route` actually dispatches.
pub(crate) fn is_builtin_command(first: &str) -> bool {
    first.starts_with('-') // covers --help / -h / unknown leading flags
        || matches!(
            first,
            "help" | "completions" | "config" | "profile" | "describe"
                | "doctor" | "refresh-specs" | "version" | "workflow"
        )
}

/// Own frontend construction and lifecycle for the builtin route.
pub(crate) async fn route_builtin(
    flags: &mut crate::invocation::flags::GlobalFlags,
    backend: crate::invocation::context::PhaseBackend,
    options: crate::frontend::RenderOptions,
    raw_args: &[String],
    remaining: &[String],
) -> Result<InvocationOutcome, CliError> {
    // Pre-surface failure: let the caller render it on a fresh plain frontend.
    crate::invocation::router::parse_page_limit(flags)?;

    let mut frontend = crate::frontend::frontend_for_surface(backend, options)?;

    // Root help/version exits are lifecycle-free; other builtin commands are not.
    let lifecycle_free = is_lifecycle_free_root(raw_args, remaining);

    if !lifecycle_free {
        frontend.on_event(&crate::frontend::FrontendEvent::RunStarted {
            workflow_banner: None,
        });
    }
    let result = dispatch(frontend.as_mut(), raw_args, flags, remaining).await;
    let outcome = crate::invocation::run_outcome_for(&result);
    if !lifecycle_free {
        frontend.on_event(&crate::frontend::FrontendEvent::RunFinished { outcome });
    }

    match result {
        Err(e) => {
            // Render post-construction failures on the owned frontend so JSON
            // mode keeps its error envelope.
            let exit_code = e.exit_code();
            frontend.render_error(&e);
            let _ = frontend.finish();
            Ok(InvocationOutcome::Exit(exit_code))
        }
        Ok(InvocationOutcome::Exit(code)) => {
            let _ = frontend.finish();
            Ok(InvocationOutcome::Exit(code))
        }
        Ok(InvocationOutcome::Cancelled) => {
            let _ = frontend.finish();
            Ok(InvocationOutcome::Cancelled)
        }
        Ok(InvocationOutcome::Complete) => {
            frontend.finish()?;
            Ok(InvocationOutcome::Complete)
        }
    }
}

/// Whether this builtin-route invocation is a root-level non-run exit that must
/// NOT be bracketed in the `RunStarted` / `RunFinished` run lifecycle.
///
/// Covers only root help/version exits. Subcommand-local help stays wrapped.
fn is_lifecycle_free_root(raw_args: &[String], remaining: &[String]) -> bool {
    if remaining.is_empty() {
        return true;
    }
    if has_version_flag(raw_args) {
        return true;
    }
    let first = remaining[0].as_str();
    matches!(first, "--help" | "-h" | "help" | "version")
}

/// Whether the raw args carry the `--version`/`-V` short-circuit flag.
///
/// Single source for the version-flag rule: both `is_lifecycle_free_root`
/// (lifecycle gating) and `dispatch` (the actual short-circuit) consult this,
/// so the lifecycle decision and the dispatch decision cannot drift.
fn has_version_flag(raw_args: &[String]) -> bool {
    raw_args.iter().any(|arg| arg == "--version" || arg == "-V")
}

/// Handle the `--version`/`-V` short-circuit, then route to the built-in
/// command handlers. Separate from `route_builtin` so every error here flows
/// through `frontend.render_error`.
async fn dispatch(
    frontend: &mut dyn crate::frontend::Frontend,
    raw_args: &[String],
    flags: &mut flags::GlobalFlags,
    remaining: &[String],
) -> Result<InvocationOutcome, CliError> {
    if has_version_flag(raw_args) {
        handlers::version::handle_version(flags, frontend)?;
        Ok(InvocationOutcome::Complete)
    } else {
        route(flags, remaining, frontend).await
    }
}

/// Match the leading positional arg against the built-in command set and
/// dispatch.
///
/// The per-token match arms below are the dispatch counterpart to
/// [`is_builtin_command`]: the two MUST stay in sync — `classify_root` uses
/// `is_builtin_command` to route here, and any token it admits must have a
/// matching arm (or hit the unknown-flag / `unreachable!` fallthrough).
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

    // `auth` is carved out to `RootDispatch::Auth` at the top level and runs
    // its own bespoke self-owned wrapper, so it never reaches `route`.

    if first == "completions" {
        return handlers::completions::handle_completions(&remaining[1..], flags, frontend);
    }

    if first == "config" {
        return handlers::config::handle_config(&remaining[1..], flags, frontend).await;
    }

    if first == "profile" {
        return handlers::profile::handle_profile(&remaining[1..], flags, frontend).await;
    }

    if first == "describe" {
        return handlers::describe::handle_describe(&remaining[1..], flags, frontend);
    }

    if first == "doctor" {
        return handlers::doctor::handle_doctor(&remaining[1..], flags, frontend).await;
    }

    if first == "refresh-specs" {
        return handlers::refresh_specs::handle_refresh_specs(&remaining[1..], flags, frontend);
    }

    if first == "version" {
        handlers::version::handle_version(flags, frontend)?;
        return Ok(InvocationOutcome::Complete);
    }

    if first == "workflow" {
        return crate::invocation::routes::workflow::route_workflow(&remaining[1..], frontend)
            .await;
    }

    if first.starts_with('-') {
        return Err(CliError::Usage {
            message: format!("Unknown flag: '{first}'"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Run 'ags --help' for a list of available flags.",
            ))),
        });
    }

    // A non-flag, non-builtin positional is a synthesised service command —
    // `classify_root` routes those to `Service` at the top level, so `route`
    // can never reach here.
    unreachable!("synthesised service commands are dispatched at the top level by run()")
}

/// Tests for the builtin-route run-vs-non-run lifecycle contract.
///
/// The `RunStarted` / `RunFinished` run-boundary emission in [`route_builtin`] is
/// gated solely by [`is_lifecycle_free_root`], so pinning that function pins
/// the whole contract. The `route_builtin`-level tests then drive the real
/// wrapper end-to-end for the side-effect-free paths only.
#[cfg(test)]
mod run_contract_tests {
    use super::{has_version_flag, is_builtin_command, is_lifecycle_free_root, route_builtin};
    use crate::frontend::RenderOptions;
    use crate::invocation::context::PhaseBackend;
    use crate::invocation::flags::GlobalFlags;
    use crate::invocation::InvocationOutcome;

    /// Build a `Vec<String>` from a `&[&str]` for test argument lists.
    fn remaining(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    // ── is_lifecycle_free_root ──

    #[test]
    fn test_lifecycle_free_root_empty_remaining_is_free() {
        // No-args root help is a non-run exit.
        assert!(is_lifecycle_free_root(&remaining(&[]), &remaining(&[])));
    }

    #[test]
    fn test_lifecycle_free_root_leading_help_tokens_are_free() {
        for token in ["--help", "-h", "help"] {
            let args = remaining(&[token]);
            assert!(
                is_lifecycle_free_root(&args, &args),
                "{token} as leading remaining arg must be lifecycle-free"
            );
        }
    }

    #[test]
    fn test_lifecycle_free_root_leading_version_word_is_free() {
        let args = remaining(&["version"]);
        assert!(is_lifecycle_free_root(&args, &args));
    }

    #[test]
    fn test_lifecycle_free_root_version_flag_anywhere_is_free() {
        for flag in ["--version", "-V"] {
            let raw = remaining(&[flag]);
            assert!(
                is_lifecycle_free_root(&raw, &remaining(&[flag])),
                "{flag} must make the root lifecycle-free"
            );
        }
    }

    #[test]
    fn test_lifecycle_free_root_version_flag_wins_for_mixed_invocation() {
        // `ags iam users list --version` keeps today's `--version` precedence:
        // the version flag in raw_args makes the root lifecycle-free even
        // though the leading remaining token is a service token.
        let raw = remaining(&["iam", "users", "list", "--version"]);
        let rest = remaining(&["iam", "users", "list"]);
        assert!(is_lifecycle_free_root(&raw, &rest));
    }

    #[test]
    fn test_lifecycle_free_root_true_static_command_is_wrapped() {
        // `config` is a true builtin command run — lifecycle-wrapped.
        let args = remaining(&["config"]);
        assert!(!is_lifecycle_free_root(&args, &args));
    }

    #[test]
    fn test_lifecycle_free_root_subcommand_local_help_stays_wrapped() {
        // Subcommand-local help stays lifecycle-wrapped.
        let args = remaining(&["config", "--help"]);
        assert!(!is_lifecycle_free_root(&args, &args));
    }

    #[test]
    fn test_lifecycle_free_root_other_static_runs_are_wrapped() {
        for args in [remaining(&["doctor"]), remaining(&["workflow", "list"])] {
            assert!(
                !is_lifecycle_free_root(&args, &args),
                "{args:?} is a true builtin run and must stay lifecycle-wrapped"
            );
        }
    }

    #[test]
    fn test_lifecycle_free_root_unknown_leading_flag_stays_wrapped() {
        // Unknown leading flags stay lifecycle-wrapped.
        let args = remaining(&["--foo"]);
        assert!(!is_lifecycle_free_root(&args, &args));
    }

    // ── is_builtin_command ──

    #[test]
    fn test_is_builtin_command_admits_builtin_tokens() {
        for token in [
            "help",
            "completions",
            "config",
            "profile",
            "describe",
            "doctor",
            "refresh-specs",
            "version",
            "workflow",
        ] {
            assert!(
                is_builtin_command(token),
                "{token} must be a builtin command"
            );
        }
    }

    #[test]
    fn test_is_builtin_command_admits_dash_prefixed_tokens() {
        for token in ["--help", "-h", "--version", "-V", "--foo"] {
            assert!(
                is_builtin_command(token),
                "{token} (dash-prefixed) must be a builtin command"
            );
        }
    }

    #[test]
    fn test_is_builtin_command_rejects_service_token() {
        // A bare service token like `iam` is NOT builtin — it routes to
        // `RootDispatch::Service`.
        assert!(!is_builtin_command("iam"));
    }

    // ── has_version_flag ──

    #[test]
    fn test_has_version_flag_detects_long_and_short() {
        assert!(has_version_flag(&remaining(&["--version"])));
        assert!(has_version_flag(&remaining(&["-V"])));
        assert!(has_version_flag(&remaining(&["iam", "users", "--version"])));
    }

    #[test]
    fn test_has_version_flag_absent_is_false() {
        assert!(!has_version_flag(&remaining(&[])));
        assert!(!has_version_flag(&remaining(&["config", "--help"])));
    }

    // ── route_builtin end-to-end (side-effect-free routes only) ──

    /// Drive the real `route_builtin` wrapper to completion on a current-thread
    /// tokio runtime. `PhaseBackend::StructuredJson` avoids any TTY
    /// requirement; help/version/catalogue text is printed to stdout during
    /// the run.
    fn route_builtin_blocking(
        raw_args: &[&str],
        rest: &[&str],
    ) -> Result<InvocationOutcome, crate::errors::CliError> {
        let mut flags = GlobalFlags::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("tokio runtime");
        runtime.block_on(route_builtin(
            &mut flags,
            PhaseBackend::StructuredJson,
            RenderOptions::default(),
            &remaining(raw_args),
            &remaining(rest),
        ))
    }

    #[test]
    fn test_route_builtin_root_help_is_complete() {
        let outcome = route_builtin_blocking(&["--help"], &["--help"]);
        assert!(
            matches!(outcome, Ok(InvocationOutcome::Complete)),
            "root --help must return Ok(Complete), got {outcome:?}"
        );
    }

    #[test]
    fn test_route_builtin_no_args_exits_one() {
        // Success Criterion: root no-args help still exits 1, not Complete.
        let outcome = route_builtin_blocking(&[], &[]);
        match outcome {
            Ok(InvocationOutcome::Exit(1)) => {}
            other => panic!("no-args must return Ok(Exit(1)), got {other:?}"),
        }
    }

    #[test]
    fn test_route_builtin_version_word_is_complete() {
        let outcome = route_builtin_blocking(&["version"], &["version"]);
        assert!(
            matches!(outcome, Ok(InvocationOutcome::Complete)),
            "`version` must return Ok(Complete), got {outcome:?}"
        );
    }

    #[test]
    fn test_route_builtin_version_flag_is_complete() {
        // `--version` short-circuits inside `dispatch` via `has_version_flag`.
        let outcome = route_builtin_blocking(&["--version"], &["--version"]);
        assert!(
            matches!(outcome, Ok(InvocationOutcome::Complete)),
            "`--version` must return Ok(Complete), got {outcome:?}"
        );
    }

    #[test]
    fn test_route_builtin_workflow_list_is_complete() {
        // `workflow list` is the required builtin-workflow + true-builtin-run
        // coverage. `route_workflow_list` is offline (bundled registry only,
        // no runtime prologue, no network), so this drives the real wrapper
        // including the `RunStarted` / `RunFinished` bracketing.
        ags_runtime::runtime::bootstrap();
        let outcome = route_builtin_blocking(&["workflow", "list"], &["workflow", "list"]);
        assert!(
            matches!(outcome, Ok(InvocationOutcome::Complete)),
            "`workflow list` must return Ok(Complete), got {outcome:?}"
        );
    }
}
