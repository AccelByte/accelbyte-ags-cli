//! Command layer: argument parsing, routing, and orchestration.

use base64::Engine;

pub mod builder;
pub(crate) mod clap_helpers;
pub(crate) mod compat_flags;
pub mod completions_generator;
pub(crate) mod confirm;
pub mod context;
pub mod errors;
mod first_run;
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

/// Map an invocation result to the telemetry `Outcome` attached to the single
/// end-of-command `cli.command.invoked` event. `status` reuses `run_outcome_for`
/// so the two never drift; `exit_code` is the real process exit code — the
/// error's own code for a pre-surface failure, the explicit code for
/// `InvocationOutcome::Exit`, `2` for a cancelled workflow (mirroring
/// `finish_self_owned`'s `std::process::exit(2)`), or `0` on a clean complete.
/// `http_status`/`error_code` are read from `CliError::metadata()` — never
/// from `message`/`reason`/`detail`, which may embed free text.
fn telemetry_outcome(
    result: &Result<InvocationOutcome, CliError>,
) -> ags_runtime::runtime::telemetry::Outcome {
    let status = match run_outcome_for(result) {
        crate::frontend::RunOutcome::Success => "completed",
        crate::frontend::RunOutcome::Failed => "failed",
        crate::frontend::RunOutcome::Cancelled => "cancelled",
    };
    let exit_code = match result {
        Err(e) => e.exit_code(),
        Ok(InvocationOutcome::Exit(code)) => *code,
        Ok(InvocationOutcome::Cancelled) => 2,
        Ok(InvocationOutcome::Complete) => 0,
    };
    let error_class = result.as_ref().err().map(CliError::telemetry_class);
    let metadata = result.as_ref().err().and_then(CliError::metadata);
    let http_status = metadata.and_then(|m| m.http_status);
    let error_code = metadata.and_then(|m| m.code.clone());
    ags_runtime::runtime::telemetry::Outcome {
        status,
        exit_code,
        error_class,
        http_status,
        error_code,
    }
}

/// Exit code reported when the user interrupts a run with Ctrl-C: the
/// conventional shell code for a process terminated by SIGINT (128 + 2).
const INTERRUPT_EXIT_CODE: i32 = 130;

/// Telemetry outcome for a run the user interrupted with Ctrl-C. Reported as
/// `"cancelled"` — the same status a declined confirmation produces — with
/// [`INTERRUPT_EXIT_CODE`] and no error facts: an abandoned run is not a
/// failure and never reached an upstream response.
fn interrupt_outcome() -> ags_runtime::runtime::telemetry::Outcome {
    ags_runtime::runtime::telemetry::Outcome {
        status: "cancelled",
        exit_code: INTERRUPT_EXIT_CODE,
        error_class: None,
        http_status: None,
        error_code: None,
    }
}

/// Telemetry context for the in-flight command, published by the spawned
/// gather task as soon as `gather_context` resolves — deliberately *not* at
/// join time — so the Ctrl-C handler can reach it for the whole duration of
/// the command, which is exactly the window in which a user abandons a slow
/// run. Without this a Ctrl-C is indistinguishable from a crash or a hang in
/// the data.
///
/// The `Option` is the exactly-once handoff: whoever `take()`s the context
/// emits for it. The Ctrl-C handler takes it and then exits the process, so
/// the normal path never runs; if the normal path took it first, the handler
/// finds `None` and exits without emitting. Either ordering yields exactly
/// one `cli.command.invoked` event.
static INTERRUPT_TELEMETRY: std::sync::OnceLock<
    std::sync::Mutex<Option<ags_runtime::runtime::telemetry::CommandTelemetry>>,
> = std::sync::OnceLock::new();

/// The [`INTERRUPT_TELEMETRY`] slot, initialised empty on first access.
fn interrupt_telemetry_slot(
) -> &'static std::sync::Mutex<Option<ags_runtime::runtime::telemetry::CommandTelemetry>> {
    INTERRUPT_TELEMETRY.get_or_init(|| std::sync::Mutex::new(None))
}

/// Publish the gathered context so a Ctrl-C arriving mid-command has
/// something to emit for. Fire-and-forget: a poisoned lock is ignored rather
/// than panicking, since telemetry must never affect the command.
fn publish_interrupt_telemetry(ctx: ags_runtime::runtime::telemetry::CommandTelemetry) {
    if let Ok(mut guard) = interrupt_telemetry_slot().lock() {
        *guard = Some(ctx);
    }
}

/// Take the published context, leaving the slot empty. `None` when telemetry
/// is disabled, when gathering failed, or when another path already took it —
/// the exactly-once guard described on [`INTERRUPT_TELEMETRY`].
fn take_interrupt_telemetry() -> Option<ags_runtime::runtime::telemetry::CommandTelemetry> {
    interrupt_telemetry_slot()
        .lock()
        .ok()
        .and_then(|mut guard| guard.take())
}

/// The `ui_surface` label for the surface the user actually got, published by
/// [`context::FrontendContext::finalize_surface`] the moment the decision
/// matrix resolves.
///
/// Needed because `cli.command.invoked`'s context is gathered *before* the
/// route runs, when `--ui=auto` has not been resolved yet and
/// `surface_backend()` therefore still reports `PlainTerminal` for every
/// invocation. Reading the finalized label back at emit time — which always
/// happens after the route has returned — is what keeps
/// `cli.command.invoked.ui_surface` equal to the `ui_surface` on the run and
/// step events, which are built from the finalized context.
///
/// Empty for a route that never finalizes (auth, builtin, the `--help`/
/// `--version` paths): those render on the pre-finalize backend, so the
/// gathered label is already the one they used.
static FINALIZED_UI_SURFACE: std::sync::OnceLock<std::sync::Mutex<Option<&'static str>>> =
    std::sync::OnceLock::new();

/// The [`FINALIZED_UI_SURFACE`] slot, initialised empty on first access.
fn finalized_ui_surface_slot() -> &'static std::sync::Mutex<Option<&'static str>> {
    FINALIZED_UI_SURFACE.get_or_init(|| std::sync::Mutex::new(None))
}

/// Publish the finalized `ui_surface` label. Fire-and-forget: a poisoned lock
/// is ignored rather than panicking, since telemetry must never affect the
/// command. Called from `finalize_surface` itself — not from each route — so a
/// future route cannot forget to and silently reintroduce the mismatch.
pub(crate) fn publish_finalized_ui_surface(label: &'static str) {
    if let Ok(mut guard) = finalized_ui_surface_slot().lock() {
        *guard = Some(label);
    }
}

/// The published finalized `ui_surface` label, or `None` when no route
/// finalized a surface for this invocation.
fn finalized_ui_surface() -> Option<&'static str> {
    finalized_ui_surface_slot()
        .lock()
        .ok()
        .and_then(|guard| *guard)
}

/// Retag `ctx` with the finalized `ui_surface` when a route resolved one, so
/// every event of one invocation reports the same surface. Applied at each of
/// the two emit sites (the normal path and the Ctrl-C handler) rather than at
/// gather time, because gathering races the route that finalizes.
fn with_finalized_ui_surface(
    ctx: ags_runtime::runtime::telemetry::CommandTelemetry,
) -> ags_runtime::runtime::telemetry::CommandTelemetry {
    retag_ui_surface(ctx, finalized_ui_surface())
}

/// Overwrite `ctx.ui_surface` with `label` when a route resolved one, leaving
/// the gathered label in place otherwise. Pure — the global read lives in
/// [`with_finalized_ui_surface`] — so the retag itself is testable without
/// touching process state.
fn retag_ui_surface(
    mut ctx: ags_runtime::runtime::telemetry::CommandTelemetry,
    label: Option<&'static str>,
) -> ags_runtime::runtime::telemetry::CommandTelemetry {
    if let Some(label) = label {
        ctx.ui_surface = label;
    }
    ctx
}

/// Whether a SIGINT has been observed for this process. Set **only** by
/// [`spawn_interrupt_handler`], after `ctrl_c()` resolves and before the
/// bounded flush begins — never speculatively, since a spurious `true` would
/// mask a genuine command exit code.
///
/// Needed because the handler's flush and the command itself run
/// concurrently: a command that finishes inside the flush window would
/// otherwise reach its own `std::process::exit` first and report success for a
/// run the user interrupted. [`finish_self_owned`] checks this and exits
/// [`INTERRUPT_EXIT_CODE`] instead.
static INTERRUPT_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether the user interrupted this run with Ctrl-C (see
/// [`INTERRUPT_REQUESTED`]).
fn interrupt_requested() -> bool {
    INTERRUPT_REQUESTED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Whether the active command owns its own signal-handling path — a
/// **sticky, one-way** flag set once at command start and never cleared.
///
/// When `true`, [`spawn_interrupt_handler`] defers to the command's own
/// shutdown rather than calling `process::exit`, and
/// [`should_force_interrupt_exit`] returns `false` so
/// [`finish_self_owned`] reports the command's own outcome rather than
/// forcing exit 130.
///
/// Deliberately one-way: the decision is made before any signal can
/// arrive, so it is free of the scheduler race that plagued the previous
/// claim/release design. Once an owning command has declared itself, a
/// Ctrl-C arriving after the command has already finished will also be
/// deferred — the process reports the command's own outcome, which is
/// correct because the work completed. The second Ctrl-C force-quit
/// remains available.
///
/// Commands that install their own `ctrl_c()` watcher (e.g. `tunnel`,
/// `remote-debug connect`) call [`declare_command_owns_interrupt_path`]
/// before entering their signal-sensitive scope.
static COMMAND_OWNS_INTERRUPT_PATH: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether the active command has declared ownership of its signal path.
fn command_owns_interrupt_path() -> bool {
    COMMAND_OWNS_INTERRUPT_PATH.load(std::sync::atomic::Ordering::SeqCst)
}

/// Declare that the active command owns its signal-handling path. This is
/// a one-way operation — the flag is never cleared. Call before entering
/// the signal-sensitive scope (e.g. the tunnel accept loop or
/// remote-debug retry loop), so the decision is recorded before any
/// Ctrl-C can arrive.
pub(crate) fn declare_command_owns_interrupt_path() {
    COMMAND_OWNS_INTERRUPT_PATH.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Whether the post-command path should force-exit with the interrupt
/// code. True when a Ctrl-C was observed AND the command did NOT declare
/// ownership of its signal path. Called by [`finish_self_owned`].
fn should_force_interrupt_exit() -> bool {
    interrupt_requested() && !command_owns_interrupt_path()
}

/// Spawn the Ctrl-C watcher. When
/// [`COMMAND_OWNS_INTERRUPT_PATH`] is set, the handler sets
/// [`INTERRUPT_REQUESTED`] but does NOT call `process::exit` — it lets
/// the command run its own shutdown and report its own exit code.
/// [`should_force_interrupt_exit`] reads the same flag after the
/// command returns, so [`finish_self_owned`] honours the command's
/// outcome rather than forcing exit 130. A second Ctrl-C in that mode
/// is the force-quit escape hatch, exiting [`INTERRUPT_EXIT_CODE`].
///
/// When no command owns the signal path, the handler exits
/// [`INTERRUPT_EXIT_CODE`] immediately (after a bounded telemetry
/// flush), preserving the original behaviour.
///
/// Installed unconditionally, regardless of whether telemetry is enabled,
/// so signal handling never varies with a telemetry setting — with
/// telemetry off the slot is simply empty and the handler only exits.
///
/// Two limits are deliberately accepted. A SIGINT arriving *before*
/// `gather_context` resolves finds an empty slot and emits no event —
/// inherent to the design, and part of the abandoned-run case this cannot
/// cover. And with telemetry enabled Ctrl-C is no longer instant: the
/// command's own tasks keep running while the flush completes, so an
/// aborted operation can still finish (a second Ctrl-C cuts that short).
///
/// Restoring terminal state here is unnecessary: crossterm's raw mode
/// clears `ISIG`, so the inline and fullscreen surfaces receive Ctrl-C as
/// an ordinary keypress and turn it into their own `Cancel` (see
/// `frontend::terminal::form_runner::is_ctrl_c`). A real SIGINT can
/// therefore only be delivered while the process is *not* in raw mode —
/// both `enter_raw_mode` call sites are crossterm-managed surface
/// `lifecycle::acquire` functions — leaving no terminal state for this
/// exit path to undo.
pub fn spawn_interrupt_handler() {
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_err() {
            return;
        }
        // Set before the flush so a command finishing inside the flush window
        // still exits 130 rather than reporting its own outcome.
        INTERRUPT_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);

        if command_owns_interrupt_path() {
            // The command declared ownership before any signal could
            // arrive, so this read is race-free. Do NOT exit — let the
            // command's own cancellation token drive an orderly shutdown
            // and report its own exit code. Re-arm for a second Ctrl-C
            // as the force-quit escape hatch (tokio replaced the default
            // SIGINT disposition, so without this the user has no way out
            // if the command hangs during shutdown).
            // Force quit: this path runs no destructors (process::exit
            // skips Drop), so any lock file or child process owned by
            // the running command outlives it.  Commands that hold such
            // resources (e.g. update --install's InstallLock and
            // installer child) must be able to recover on the next run
            // (the lock expires via the staleness window; the child is
            // killed via kill_on_drop on the first Ctrl-C path).
            if tokio::signal::ctrl_c().await.is_ok() {
                std::process::exit(INTERRUPT_EXIT_CODE);
            }
            return;
        }

        // No ownership claim — the global handler takes control.
        if let Some(ctx) = take_interrupt_telemetry().map(with_finalized_ui_surface) {
            tokio::select! {
                _ = ags_runtime::runtime::telemetry::emit_with_outcome(&ctx, interrupt_outcome()) => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        }
        std::process::exit(INTERRUPT_EXIT_CODE);
    });
}

/// Generate a per-invocation correlation id for a registered `ags workflow
/// run`, attached both to the parent `cli.command.invoked` event
/// (`workflow_run_id`) and to every `cli.workflow.step_*` event for the same
/// run, so a PostHog query can join "started but never completed" per step
/// within one run. Not a secret — only needs to be unique enough to avoid
/// collisions within a dashboard's query window, mirroring
/// `generate_state`'s PKCE-state pattern in `invocation/routes/auth/oauth.rs`.
fn generate_workflow_run_id() -> String {
    use rand::Rng;
    let bytes: [u8; 16] = rand::rng().random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Extract the registered workflow's id from a root invocation's `remaining`
/// argv, for the `cli.command.invoked` event's `workflow_id` property.
///
/// Only fires for `router::RootDispatch::WorkflowRun`, whose `remaining` is
/// always `["workflow", "run", ...]` (see `routes::workflow::is_workflow_run`)
/// — so `remaining[2..]`'s first non-flag token is the id. A bare `ags
/// workflow run` or one followed only by flags (e.g. `--dry-run`) yields
/// `None` rather than picking up a flag; every non-workflow-run command path
/// yields `None` too.
fn workflow_id_for_dispatch(
    dispatch: &router::RootDispatch,
    remaining: &[String],
) -> Option<String> {
    matches!(dispatch, router::RootDispatch::WorkflowRun)
        .then(|| {
            remaining
                .iter()
                .skip(2)
                .find(|token| !token.starts_with('-'))
                .cloned()
        })
        .flatten()
}

/// Emit a single status line to stderr indicating that another process holds the named file lock.
fn report_lock_contention(lock_name: &str) {
    crate::frontend::write_stderr_line(&format!("  Waiting for file lock on {lock_name}\u{2026}"));
}

/// Write an already-formatted `tdbg!` diagnostic line to stderr.
///
/// Registered once at startup as the runtime's telemetry debug reporter
/// (mirrors `report_lock_contention` / `register_reporter_if_plain` for the
/// same reason: `ags-runtime` must never do I/O directly — see
/// `test_runtime_layer_has_no_user_facing_io`). Unlike the lock-contention
/// reporter this is registered unconditionally regardless of surface, since
/// the pre-fix `tdbg!` already wrote to stderr unconditionally whenever
/// `AGS_TELEMETRY_DEBUG` was set — this wiring preserves that behaviour
/// rather than changing it.
fn report_telemetry_debug(line: &str) {
    crate::frontend::write_stderr_line(line);
}

/// Try to emit the one-time first-run onboarding hint after the surface
/// has been finalized. Delegates to the single `should_show_first_run_hint`
/// predicate, passing the disk-read as a lazy closure so it only runs when
/// all cheap in-memory checks pass.
///
/// Call alongside `register_reporter_if_plain` after `finalize_surface` in
/// each route handler, or after frontend construction in routes that do not
/// call `finalize_surface` (auth, builtin).
pub(crate) fn try_emit_first_run_hint(ctx: &context::FrontendContext, is_meta: bool) {
    if first_run::should_show_first_run_hint(ctx, first_run::is_first_run_hint_seen, is_meta) {
        first_run::emit_first_run_hint();
    }
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
    workflow_run_id: Option<&str>,
) -> Result<InvocationOutcome, CliError> {
    router::parse_page_limit(flags)?;
    // `remaining` is `["workflow", "run", <rest>]`; pass only `<rest>`.
    routes::workflow::route_workflow_run(
        &remaining[2..],
        flags,
        options.clone(),
        frontend_context,
        workflow_run_id,
    )
    .await
}

/// Top-level entry for a synthesised service command. Mirrors
/// `run_workflow_run`: validates the page-limit flag (the builtin route
/// does the same in `route_builtin`), then hands off to the self-owning
/// service handler. `remaining` is `[<service>, <rest>...]`.
///
/// `shim_presentation` is `Some` when the invocation came through an `extend`
/// migration shortcut, so the help printer can render the shortcut's own help
/// page instead of the canonical CSM page. The returned `Option<Value>` is the
/// final call's raw JSON response body (when the run produced one); the shim
/// path reads the new `deploymentId` out of it to arm `deploy-app --wait`'s
/// identity guard, and every other caller ignores it.
async fn run_service(
    flags: &mut flags::GlobalFlags,
    options: &crate::frontend::RenderOptions,
    frontend_context: &context::FrontendContext,
    remaining: &[String],
    shim_presentation: Option<handlers::extend::service_shims::ShimPresentation>,
) -> Result<(InvocationOutcome, Option<serde_json::Value>), CliError> {
    router::parse_page_limit(flags)?;

    routes::service::route_service(
        &remaining[0],
        &remaining[1..],
        flags,
        options.clone(),
        frontend_context,
        shim_presentation.as_ref(),
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
///
/// `telemetry_task` — spawned before dispatch by `gather_context` running
/// concurrently with it (see `run()`) — is joined and emitted exactly once,
/// at the single point every arm below funnels through: this function itself
/// is called exactly once per `run()` invocation (each `!is_version` dispatch
/// arm `return`s its own call to this function, and the trailing
/// builtin-route call is only reached when none of those arms ran), and the
/// one `emit_with_outcome` call sits before the match that decides the exit
/// path, so it always runs — and only once — before whichever
/// `std::process::exit` (or the `Ok(())` return) follows.
///
/// Joining the task here (rather than awaiting `gather_context` itself before
/// dispatch) is what keeps its `fetch_email` network round-trip off the
/// command's critical path: by the time this function runs, the task has
/// had the whole command's execution to complete concurrently in the
/// background. A join failure (panic) or a `None` gather result (telemetry
/// disabled, or gathering failed) is treated the same as telemetry having
/// nothing to emit — fire-and-forget, never surfaced to the caller.
///
/// The task does not return the context: it publishes it into
/// [`INTERRUPT_TELEMETRY`] the moment gathering resolves, and this function
/// takes it back out. That take is the exactly-once handoff with
/// [`spawn_interrupt_handler`] — see [`INTERRUPT_TELEMETRY`]. This function
/// also honours [`INTERRUPT_REQUESTED`], exiting [`INTERRUPT_EXIT_CODE`]
/// instead of the command's own exit code when the run was interrupted.
async fn finish_self_owned(
    result: Result<InvocationOutcome, CliError>,
    telemetry_task: Option<tokio::task::JoinHandle<()>>,
    flags: &flags::GlobalFlags,
    remaining: &[String],
    is_version: bool,
) -> Result<(), CliError> {
    if let Some(task) = telemetry_task {
        // Await the gather task purely to guarantee its publish into
        // `INTERRUPT_TELEMETRY` has already happened (an empty slot means
        // telemetry is off, gathering failed, or the Ctrl-C handler got there
        // first), then take the context back out.
        let _ = task.await;
        if let Some(ctx) = take_interrupt_telemetry().map(with_finalized_ui_surface) {
            ags_runtime::runtime::telemetry::emit_with_outcome(&ctx, telemetry_outcome(&result))
                .await;
        }
    }

    // The user interrupted this run: report the interrupt's exit code rather
    // than the command's own, which would say "success" for a run that was
    // abandoned. Skip when the command declared ownership of its signal
    // path — it ran its orderly shutdown and its outcome (e.g. exit 0 for
    // a tunnel) is the correct one to report.
    if should_force_interrupt_exit() {
        std::process::exit(INTERRUPT_EXIT_CODE);
    }

    // Best-effort background refresh, regardless of the command's outcome,
    // but never under --dry-run, which must stay side-effect-free (no detached
    // child, no cache write).
    if !flags.is_dry_run {
        spawn_update_check_if_due();
    }

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
        Ok(outcome) => {
            // The command's own output is already written; footer goes last.
            if !flags.is_dry_run {
                maybe_show_update_footer(flags, remaining, is_version);
            }
            match outcome {
                InvocationOutcome::Exit(code) => std::process::exit(code),
                InvocationOutcome::Cancelled => std::process::exit(2),
                InvocationOutcome::Complete => Ok(()),
            }
        }
    }
}

const UPDATE_RELEASES_URL: &str = "https://github.com/AccelByte/accelbyte-ags-cli/releases/latest";

/// Hidden argument marking a detached update-check child process.
const UPDATE_CHECK_CHILD_ARG: &str = "__update-check";

/// Launch a detached child process to run the GitHub update check, unless
/// suppressed (env/config/CI) or not yet due (cache fresh, <24h old).
///
/// The child (`ags __update-check`) is fully detached — null stdio and its own
/// process group — so it outlives this process and completes the network call
/// after we exit. That detachment is what makes the check actually run, matching
/// how `gh` runs its notifier. Fire-and-forget: we never wait on it.
fn spawn_update_check_if_due() {
    use ags_runtime::runtime::update_check;
    if update_check::is_check_suppressed() || !update_check::is_check_due() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut command = std::process::Command::new(exe);
    command
        .arg(UPDATE_CHECK_CHILD_ARG)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    detach_child(&mut command);
    let _ = command.spawn();
}

/// Detach the child from the parent's process group on Unix so terminal signals
/// (Ctrl-C) to the parent don't also kill the background check. No-op elsewhere,
/// where a spawned child already survives the parent's exit.
#[cfg(unix)]
fn detach_child(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn detach_child(_command: &mut std::process::Command) {}

/// Whether the update footer is suppressed for this command. Meta-commands
/// and machine-oriented builtins never show the hint: `--version`/`-V` (via
/// `is_version`), and `doctor`, `version`, and `completions` by name.
pub(crate) fn is_footer_suppressed_command(command: &str, is_version: bool) -> bool {
    is_version || matches!(command, "doctor" | "update" | "version" | "completions")
}

/// Whether the output format indicates an automation/machine-readable context,
/// suppressing human-only chrome like the update footer.
pub(crate) fn is_automation_format(format: Option<ags_protocol::request::OutputFormat>) -> bool {
    matches!(format, Some(ags_protocol::request::OutputFormat::Json))
}

/// Render the update-available footer to stderr, if a fresh hint exists and the
/// display gates pass (real TTY, human format, not a suppressed command). Marks
/// the version notified so it never repeats.
fn maybe_show_update_footer(flags: &flags::GlobalFlags, remaining: &[String], is_version: bool) {
    use ags_runtime::runtime::update_check;

    let is_automation = is_automation_format(flags.format);
    let command = remaining.first().map(String::as_str).unwrap_or("");
    let is_suppressed = is_footer_suppressed_command(command, is_version);
    let stderr_is_tty = ags_runtime::support::is_stderr_tty();

    let hint = update_check::footer_to_show(
        update_check::cached_hint(),
        stderr_is_tty,
        is_automation,
        is_suppressed,
    );

    if let Some(hint) = hint {
        let text = format!(
            "{} ags {} is available (current: {}) \u{2014} {}",
            frontend::style::text::SYMBOL_UPGRADE,
            hint.latest,
            hint.current,
            UPDATE_RELEASES_URL
        );

        let styled = frontend::style::apply_tone(
            &text,
            frontend::style::Tone::Info,
            frontend::style::is_stderr_enabled(),
        );
        frontend::write_stderr_line(&styled);
        update_check::mark_notified(&hint.latest);
    }
}

/// Install the `ring` crypto provider for rustls, once per process.
///
/// Both `ring` and `aws-lc-rs` provider features are resolved in the
/// dependency graph (from `reqwest/rustls-tls` and `posthog-rs`
/// respectively), so rustls cannot auto-detect a default. Without an
/// explicit install, every outbound TLS connection (including the
/// WebSocket tunnel) panics. An `Err` from `install_default` means
/// another call already installed a provider, which is harmless.
pub fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
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
    // Registered as early as possible, unconditionally: `gather_context`
    // (which can fire the first `tdbg!` diagnostic) runs before any surface
    // is chosen below, so this cannot be deferred until a surface is known
    // the way `register_reporter_if_plain` is.
    ags_runtime::runtime::telemetry::register_debug_reporter(report_telemetry_debug);

    let raw_args: Vec<String> = std::env::args().skip(1).collect();

    // Detached update-check child (spawned by `spawn_update_check_if_due`): run
    // the GitHub check to completion, write the cache, and exit silently — never
    // parsing flags or rendering anything.
    //
    // Re-check suppression here because the documented guarantee
    // (`AGS_NO_UPDATE_CHECK=1` / CI / config) is per-invocation: a user or CI
    // job that sets the env var expects no GitHub call from ANY `ags` process,
    // including this detached child which is reachable directly via `ags __update-check`.
    if raw_args.first().map(String::as_str) == Some(UPDATE_CHECK_CHILD_ARG) {
        if ags_runtime::runtime::update_check::is_check_suppressed() {
            return Ok(());
        }
        if let Some(client) = ags_runtime::runtime::update_check::build_client() {
            ags_runtime::runtime::update_check::run_check(client).await;
        }
        return Ok(());
    }

    // Windows cannot delete the running executable's old copy during the
    // upgrade itself, so the next start cleans it up. On other platforms
    // the upgrade removes `.old` on success; a leftover after a force quit
    // is the user's rollback copy and intentionally stays.
    if cfg!(windows) {
        if let Ok(exe) = std::env::current_exe() {
            handlers::update_install::cleanup_previous_binary(&exe);
        }
    }

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
    // Snapshot the flag pairs the user actually typed BEFORE
    // `apply_config_defaults` fills in any on-disk config defaults (e.g.
    // `format`, `is_no_color`, `timeout`) — otherwise a config-file default
    // would be indistinguishable from a flag the user typed on this
    // invocation in the `gather_context` call below (the 2026-08-14 bug).
    let global_flag_pairs = flags.telemetry_pairs();
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

    let is_version = raw_args.iter().any(|arg| arg == "--version" || arg == "-V");

    // Classify the root invocation for the routing dispatch below.
    let dispatch = router::classify_root(&remaining);

    // Only a registered multi-step workflow run gets a correlation id — every
    // other command path leaves this `None`, and `gather_context`/`build_event`
    // simply omit the `workflow_run_id` property.
    let workflow_run_id: Option<String> =
        matches!(dispatch, router::RootDispatch::WorkflowRun).then(generate_workflow_run_id);

    // The registered workflow's id — the token `command_path_names`
    // deliberately truncates away (see `command_path`'s doc comment: only
    // leading bare-word command names are kept, precisely to keep positional
    // values like this one out of `command_path`). Sent verbatim as its own
    // property: a workflow id is a name the developer gave their own
    // automation, not a credential or a path, and without it a run that dies
    // before its first step loses the workflow's identity entirely.
    let workflow_id: Option<String> = workflow_id_for_dispatch(&dispatch, &remaining);

    // Best-effort CLI telemetry: gather the context for a single
    // `cli.command.invoked` event keyed on the IAM user id decoded from the
    // stored access token, joining CLI usage to the Admin Portal's PostHog
    // identity. Spawned here (before dispatch) as its own task so its
    // `fetch_email` network round-trip runs concurrently with the command
    // instead of blocking it, and joined once at the end of the command,
    // inside `finish_self_owned`, alongside the outcome — that keeps this a
    // single end-of-command send instead of a fire at invocation time. A
    // cheap no-op unless an API key resolves — from `AGS_TELEMETRY_POSTHOG_KEY`
    // at run time, or from a key compiled into an official release build; strictly
    // fire-and-forget and never affects the command. The gathered context is
    // handed over through `INTERRUPT_TELEMETRY` rather than through this
    // task's return value, so the Ctrl-C handler can reach it mid-command.
    let mut telemetry_task = if !remaining.is_empty() {
        let profile = flags.profile.clone();
        let namespace = flags.namespace.clone();
        let global_flag_pairs = global_flag_pairs.clone();
        let remaining_for_telemetry = remaining.clone();
        let workflow_run_id_for_telemetry = workflow_run_id.clone();
        let workflow_id_for_telemetry = workflow_id.clone();
        let ui_surface = backend.telemetry_label();
        Some(tokio::spawn(async move {
            let gathered = ags_runtime::runtime::telemetry::gather_context(
                profile.as_deref(),
                namespace.as_deref(),
                &global_flag_pairs,
                &remaining_for_telemetry,
                env!("CARGO_PKG_VERSION"),
                workflow_run_id_for_telemetry.as_deref(),
                ui_surface,
                workflow_id_for_telemetry.as_deref(),
            )
            .await;
            // Published here — as soon as gathering resolves, rather than
            // where this task is joined at the end of the command — so a
            // Ctrl-C during a slow run finds a context to emit for. The
            // normal path takes it back out in `finish_self_owned`.
            if let Some(ctx) = gathered {
                publish_interrupt_telemetry(ctx);
            }
        }))
    } else {
        None
    };

    // Every root execution path is self-owned: each constructs its own
    // presentation surface(s) and owns its run lifecycle, so the top level
    // does not build a frontend for any of them. `workflow run` and service
    // are executor-backed and phase-aware; `auth` uses a bespoke (non-
    // executor) wrapper that brackets only the real `auth login` flow; the
    // builtin route (`workflow list`, the built-in commands, the `--help`
    // paths, and `--version`/`-V`) runs the `route_builtin` wrapper.
    //
    // The first-run onboarding hint is emitted INSIDE each route handler
    // after the surface is finalized, so "is it plain?" is answered by the
    // surface machinery rather than predicted ahead of it.
    if !is_version {
        match dispatch {
            router::RootDispatch::WorkflowRun => {
                let result = run_workflow_run(
                    &mut flags,
                    &options,
                    &frontend_context,
                    &remaining,
                    workflow_run_id.as_deref(),
                )
                .await;
                return finish_self_owned(result, telemetry_task, &flags, &remaining, is_version)
                    .await;
            }
            router::RootDispatch::Service => {
                let result = run_service(&mut flags, &options, &frontend_context, &remaining, None)
                    .await
                    .map(|(outcome, _raw_body)| outcome);
                return finish_self_owned(result, telemetry_task, &flags, &remaining, is_version)
                    .await;
            }
            router::RootDispatch::Auth => {
                let result =
                    run_auth(&mut flags, backend, &options, &frontend_context, &remaining).await;
                return finish_self_owned(result, telemetry_task, &flags, &remaining, is_version)
                    .await;
            }
            router::RootDispatch::ExtendDockerLogin => {
                let result = routes::extend_docker_login::route_extend_docker_login(
                    &remaining[2..], // skip "extend" and "docker-login"
                    &mut flags,
                    options.clone(),
                    &frontend_context,
                )
                .await;
                return finish_self_owned(result, telemetry_task, &flags, &remaining, is_version)
                    .await;
            }
            router::RootDispatch::ExtendImageUpload => {
                let result = routes::extend_image_upload::route_extend_image_upload(
                    &remaining[2..], // skip "extend" and "image-upload"
                    &mut flags,
                    options.clone(),
                    &frontend_context,
                )
                .await;
                return finish_self_owned(result, telemetry_task, &flags, &remaining, is_version)
                    .await;
            }
            router::RootDispatch::Builtin => {
                // Extend migration shim redirect: when the invocation
                // matches a registered shim address, rewrite the args
                // and forward to the service dispatch path before the
                // builtin lifecycle starts. The service path is self-
                // owned and manages its own frontend and lifecycle.
                if let Some(dispatch) =
                    handlers::extend::service_shims::try_rewrite_shim(&remaining)
                {
                    // A shim with a malformed `--wait-*` value fails before
                    // dispatch; render the usage error through the normal path.
                    let dispatch = match dispatch {
                        Ok(dispatch) => dispatch,
                        Err(error) => {
                            return finish_self_owned(
                                Err(error),
                                telemetry_task.take(),
                                &flags,
                                &remaining,
                                is_version,
                            )
                            .await;
                        }
                    };

                    // Pass the shortcut presentation so the operation's `--help`
                    // renders the shim's own page — summary, canonical block,
                    // and, for wait-capable shims, the `--wait*` flags (added in
                    // `apply_shim_overrides`). Keep the raw response body so the
                    // deploy-app guard can read the new `deploymentId`.
                    let handlers::extend::service_shims::ShimDispatch {
                        service_args,
                        wait,
                        presentation,
                    } = dispatch;

                    let service_result = run_service(
                        &mut flags,
                        &options,
                        &frontend_context,
                        &service_args,
                        Some(presentation),
                    )
                    .await;

                    // `--dry-run` skips the poll entirely (there is no real app
                    // to poll). Say so when `--wait` was also passed: `--wait`
                    // is what introduces exit code 6, so a silent drop would be
                    // a false pass for someone dry-running to check their
                    // timeout handling.
                    if wait.is_some() && flags.is_dry_run {
                        crate::frontend::write_stderr_line(
                            "note: --wait is ignored under --dry-run; no polling is performed",
                        );
                    }

                    // When `--wait` was requested and the primary call
                    // succeeded, poll until the app reaches a terminal state.
                    // Skipped under --dry-run (no app to poll) and when the
                    // primary call did not complete cleanly. For the shims that
                    // guard by deployment id (deploy-app), arm the guard from
                    // the create response's `deploymentId` first, so the poll
                    // only accepts OUR deployment.
                    let result = match (service_result, wait) {
                        (Ok((InvocationOutcome::Complete, raw_body)), Some(mut wait))
                            if !flags.is_dry_run =>
                        {
                            if wait.spec.guard_by_deployment_id {
                                wait.expected_deployment_id =
                                    handlers::extend::app_lifecycle::deployment_id_from_response(
                                        raw_body.as_ref(),
                                    );
                            }
                            handlers::extend::app_lifecycle::run_wait_after_dispatch(wait, &flags)
                                .await
                        }
                        (Ok((outcome, _raw_body)), _) => Ok(outcome),
                        (Err(error), _) => Err(error),
                    };

                    return finish_self_owned(
                        result,
                        telemetry_task.take(),
                        &flags,
                        &remaining,
                        is_version,
                    )
                    .await;
                }
            }
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
    let result = routes::builtin::route_builtin(
        &mut flags,
        backend,
        options,
        &raw_args,
        &remaining,
        &frontend_context,
        is_meta_builtin,
    )
    .await;
    finish_self_owned(result, telemetry_task, &flags, &remaining, is_version).await
}

#[cfg(test)]
mod command_owns_interrupt_tests {
    use super::{command_owns_interrupt_path, declare_command_owns_interrupt_path};

    /// Declaring ownership sets the flag.
    #[test]
    #[serial_test::serial]
    fn declaring_ownership_sets_the_flag() {
        super::COMMAND_OWNS_INTERRUPT_PATH.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(!command_owns_interrupt_path(), "flag must start unset");
        declare_command_owns_interrupt_path();
        assert!(
            command_owns_interrupt_path(),
            "after declaring, the flag must be set"
        );
        // Clean up for other tests.
        super::COMMAND_OWNS_INTERRUPT_PATH.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Setting the flag twice is idempotent — it stays true.
    #[test]
    #[serial_test::serial]
    fn declaring_ownership_twice_is_idempotent() {
        super::COMMAND_OWNS_INTERRUPT_PATH.store(false, std::sync::atomic::Ordering::SeqCst);
        declare_command_owns_interrupt_path();
        declare_command_owns_interrupt_path();
        assert!(
            command_owns_interrupt_path(),
            "two declarations must leave the flag set"
        );
        super::COMMAND_OWNS_INTERRUPT_PATH.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
mod interrupt_ownership_tests {
    use super::*;

    /// Clean up process-wide atomics to avoid cross-test contamination.
    /// Serialised because the atomics are process-global.
    fn reset_interrupt_state() {
        INTERRUPT_REQUESTED.store(false, std::sync::atomic::Ordering::SeqCst);
        COMMAND_OWNS_INTERRUPT_PATH.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// A command that declared ownership receives a Ctrl-C, then its
    /// handler finishes. `should_force_interrupt_exit` must return false
    /// because the flag is sticky — no release, no race.
    #[test]
    #[serial_test::serial]
    fn owned_interrupt_skips_force_exit() {
        reset_interrupt_state();
        declare_command_owns_interrupt_path();
        INTERRUPT_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(
            !should_force_interrupt_exit(),
            "when the command owns its signal path, \
             finish_self_owned must NOT force exit 130"
        );
        reset_interrupt_state();
    }

    /// A command with no ownership claim, interrupted mid-run: the global
    /// handler takes control and `should_force_interrupt_exit` must
    /// return true to force exit 130.
    #[test]
    #[serial_test::serial]
    fn unowned_interrupt_forces_exit() {
        reset_interrupt_state();
        INTERRUPT_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(
            should_force_interrupt_exit(),
            "when no command owned the interrupt, \
             finish_self_owned must force exit 130"
        );
        reset_interrupt_state();
    }

    /// No interrupt observed: the post-command path must not force-exit.
    #[test]
    #[serial_test::serial]
    fn no_interrupt_does_not_force_exit() {
        reset_interrupt_state();
        assert!(
            !should_force_interrupt_exit(),
            "without an interrupt, the command's own outcome prevails"
        );
        reset_interrupt_state();
    }
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

#[cfg(test)]
mod telemetry_outcome_tests {
    use super::{telemetry_outcome, InvocationOutcome};
    use crate::errors::{ApiErrorCategory, CliError, ErrorMetadata};

    #[test]
    fn test_telemetry_outcome_maps_success_and_exit_code() {
        let r: Result<InvocationOutcome, CliError> = Ok(InvocationOutcome::Complete);
        let o = telemetry_outcome(&r);
        assert_eq!(o.status, "completed");
        assert_eq!(o.exit_code, 0);
    }

    #[test]
    fn test_telemetry_outcome_maps_explicit_exit_code() {
        let r: Result<InvocationOutcome, CliError> = Ok(InvocationOutcome::Exit(7));
        let o = telemetry_outcome(&r);
        assert_eq!(o.status, "completed");
        assert_eq!(o.exit_code, 7);
    }

    #[test]
    fn test_telemetry_outcome_sets_error_class_only_on_failure() {
        let ok: Result<InvocationOutcome, CliError> = Ok(InvocationOutcome::Complete);
        assert_eq!(telemetry_outcome(&ok).error_class, None);

        let cancelled: Result<InvocationOutcome, CliError> = Ok(InvocationOutcome::Cancelled);
        assert_eq!(telemetry_outcome(&cancelled).error_class, None);

        let failed: Result<InvocationOutcome, CliError> = Err(CliError::Auth {
            message: "no token".into(),
            metadata: None,
        });
        assert_eq!(telemetry_outcome(&failed).error_class, Some("auth"));
    }

    /// A failing command carries the upstream HTTP status and AccelByte error
    /// code end to end, read from `CliError::metadata()` — never from
    /// `message`/`reason`/`detail`, which may embed free text.
    #[test]
    fn test_telemetry_outcome_carries_http_status_and_error_code_from_metadata() {
        let failed: Result<InvocationOutcome, CliError> = Err(CliError::Api {
            message: "store not found".into(),
            metadata: Some(Box::new(ErrorMetadata {
                http_status: Some(500),
                code: Some("20013".to_string()),
                ..Default::default()
            })),
            category: ApiErrorCategory::Upstream,
        });
        let outcome = telemetry_outcome(&failed);
        assert_eq!(outcome.http_status, Some(500));
        assert_eq!(outcome.error_code, Some("20013".to_string()));
    }

    /// A successful invocation carries neither `http_status` nor `error_code`
    /// — there is no failure metadata to read them from.
    #[test]
    fn test_telemetry_outcome_omits_http_status_and_error_code_on_success() {
        let ok: Result<InvocationOutcome, CliError> = Ok(InvocationOutcome::Complete);
        let outcome = telemetry_outcome(&ok);
        assert_eq!(outcome.http_status, None);
        assert_eq!(outcome.error_code, None);
    }
}

#[cfg(test)]
mod interrupt_telemetry_tests {
    use super::{
        interrupt_outcome, publish_interrupt_telemetry, take_interrupt_telemetry,
        INTERRUPT_EXIT_CODE,
    };

    /// A minimal gathered context for exercising the interrupt slot. Field
    /// values are arbitrary — only the handoff is under test.
    fn sample_context() -> ags_runtime::runtime::telemetry::CommandTelemetry {
        ags_runtime::runtime::telemetry::CommandTelemetry {
            distinct_id: "user-1".to_string(),
            identity: "authenticated",
            ui_surface: "plain",
            email: None,
            namespace: None,
            studio: None,
            command_path: "iam users get".to_string(),
            flags: ags_runtime::runtime::telemetry::FlagCapture::default(),
            cli_version: "0.0.0-test".to_string(),
            auth_grant: None,
            started_at: std::time::Instant::now(),
            workflow_run_id: None,
            workflow_id: None,
        }
    }

    #[test]
    fn test_interrupt_outcome_is_cancelled_with_exit_130() {
        let outcome = interrupt_outcome();
        assert_eq!(outcome.status, "cancelled");
        assert_eq!(outcome.exit_code, 130);
        assert_eq!(outcome.exit_code, INTERRUPT_EXIT_CODE);
        assert_eq!(outcome.error_class, None);
        assert_eq!(outcome.http_status, None);
        assert_eq!(outcome.error_code, None);
    }

    #[test]
    fn test_interrupt_telemetry_hands_the_context_to_exactly_one_taker() {
        // Empty before anything is published: the Ctrl-C handler emits nothing
        // when telemetry is off or gathering has not resolved yet.
        assert!(take_interrupt_telemetry().is_none());

        publish_interrupt_telemetry(sample_context());

        // The first taker gets the context and is the one that emits...
        let taken = take_interrupt_telemetry();
        assert_eq!(
            taken.map(|ctx| ctx.command_path),
            Some("iam users get".to_string())
        );

        // ...and every later taker finds nothing, so no second event is sent.
        assert!(take_interrupt_telemetry().is_none());
    }
}

#[cfg(test)]
mod telemetry_pipeline_tests {
    use super::flags;

    /// Drives the REAL pipeline end to end — `pre_scan_global_flags` on raw
    /// argv, a `telemetry_pairs()` snapshot taken before any config-default
    /// mutation (mirroring the real call site's ordering after the
    /// 2026-08-14 fix), then `extract_flags` + `merge_global_flags` — rather
    /// than hand-constructing a `GlobalFlags` literal or a pairs `Vec` in
    /// isolation. This is the exact real-world bug scenario: `ags iam users
    /// get-information --namespace X --user-id Y --format json` must report
    /// ALL THREE flags (both global and command-specific), not just the
    /// command-specific `--user-id`.
    ///
    /// This test lives here (not in `ags-runtime`) because it calls both
    /// `flags::pre_scan_global_flags` (`accelbyte-ags-cli`-only) and
    /// `ags_runtime::runtime::telemetry::{extract_flags, merge_global_flags}`
    /// — the reverse dependency direction (`ags-runtime` calling into
    /// `accelbyte-ags-cli`) is forbidden, so only this crate can exercise
    /// both halves of the real pipeline together.
    #[test]
    fn test_real_pipeline_captures_global_and_command_flags_together() {
        let argv: Vec<String> = [
            "iam",
            "users",
            "get-information",
            "--namespace",
            "X",
            "--user-id",
            "Y",
            "--format",
            "json",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let (flags, remaining) = flags::pre_scan_global_flags(&argv).unwrap();

        // Snapshot BEFORE any `apply_config_defaults` call — this test
        // isolates the pipeline itself, not config-file interaction (that is
        // Finding 1's concern, fixed at the real call site in `run()`).
        let global_flag_pairs = flags.telemetry_pairs();

        let mut capture = ags_runtime::runtime::telemetry::extract_flags(&remaining);
        ags_runtime::runtime::telemetry::merge_global_flags(&mut capture, &global_flag_pairs);

        assert!(
            capture.names.contains(&"--user-id".to_string()),
            "expected --user-id in flag_names, got {:?}",
            capture.names
        );
        assert!(
            capture.names.contains(&"--namespace".to_string()),
            "expected --namespace in flag_names, got {:?}",
            capture.names
        );
        assert!(
            capture.names.contains(&"--format".to_string()),
            "expected --format in flag_names, got {:?}",
            capture.names
        );
        assert_eq!(capture.values.get("--namespace"), Some(&"X".to_string()));
        assert_eq!(capture.values.get("--format"), Some(&"json".to_string()));
    }
}

#[cfg(test)]
mod workflow_run_id_tests {
    use super::generate_workflow_run_id;

    #[test]
    fn test_generate_workflow_run_id_is_nonempty_and_varies() {
        let a = generate_workflow_run_id();
        let b = generate_workflow_run_id();
        assert!(!a.is_empty());
        assert_ne!(a, b, "two calls must not collide in a trivial test run");
    }
}

#[cfg(test)]
mod workflow_id_tests {
    use super::{router, workflow_id_for_dispatch};

    /// Build the post-prescan `remaining` slice from a list of tokens.
    fn remaining(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn test_workflow_id_for_dispatch_extracts_the_registered_id() {
        let dispatch = router::RootDispatch::WorkflowRun;
        let args = remaining(&["workflow", "run", "in-game-store"]);
        assert_eq!(
            workflow_id_for_dispatch(&dispatch, &args),
            Some("in-game-store".to_string())
        );
    }

    #[test]
    fn test_workflow_id_for_dispatch_skips_a_leading_flag() {
        let dispatch = router::RootDispatch::WorkflowRun;
        let args = remaining(&["workflow", "run", "--dry-run", "in-game-store"]);
        assert_eq!(
            workflow_id_for_dispatch(&dispatch, &args),
            Some("in-game-store".to_string()),
            "the id must be found even when a flag precedes it"
        );
    }

    #[test]
    fn test_workflow_id_for_dispatch_is_none_for_flags_only() {
        let dispatch = router::RootDispatch::WorkflowRun;
        let args = remaining(&["workflow", "run", "--dry-run"]);
        assert_eq!(
            workflow_id_for_dispatch(&dispatch, &args),
            None,
            "a flag must never be mistaken for the workflow id"
        );
    }

    #[test]
    fn test_workflow_id_for_dispatch_is_none_for_a_plain_service_command() {
        let dispatch = router::RootDispatch::Service;
        let args = remaining(&["iam", "users", "get", "user-1"]);
        assert_eq!(
            workflow_id_for_dispatch(&dispatch, &args),
            None,
            "only a registered workflow run may carry a workflow_id"
        );
    }
}

#[cfg(test)]
mod footer_suppression_tests {
    use super::{is_automation_format, is_footer_suppressed_command};
    use ags_protocol::request::OutputFormat;

    // ── is_footer_suppressed_command: table-driven over command names ──

    #[test]
    fn test_doctor_is_suppressed() {
        assert!(is_footer_suppressed_command("doctor", false));
    }

    #[test]
    fn test_version_command_is_suppressed() {
        assert!(is_footer_suppressed_command("version", false));
    }

    #[test]
    fn test_completions_is_suppressed() {
        assert!(is_footer_suppressed_command("completions", false));
    }

    #[test]
    fn test_update_is_suppressed() {
        assert!(is_footer_suppressed_command("update", false));
    }

    #[test]
    fn test_version_flag_suppresses_any_command() {
        // --version / -V sets is_version=true regardless of the remaining arg
        assert!(is_footer_suppressed_command("iam", true));
        assert!(is_footer_suppressed_command("", true));
    }

    #[test]
    fn test_ordinary_command_is_not_suppressed() {
        assert!(!is_footer_suppressed_command("iam", false));
        assert!(!is_footer_suppressed_command("platform", false));
        assert!(!is_footer_suppressed_command("lobby", false));
    }

    #[test]
    fn test_empty_command_is_not_suppressed() {
        assert!(!is_footer_suppressed_command("", false));
    }

    // ── is_automation_format: JSON suppresses, human does not ──

    #[test]
    fn test_json_format_is_automation() {
        assert!(is_automation_format(Some(OutputFormat::Json)));
    }

    #[test]
    fn test_human_format_is_not_automation() {
        assert!(!is_automation_format(Some(OutputFormat::Human)));
    }

    #[test]
    fn test_no_format_is_not_automation() {
        assert!(!is_automation_format(None));
    }
}

#[cfg(test)]
mod finalized_ui_surface_tests {
    use ags_runtime::runtime::telemetry::CommandTelemetry;

    use super::{retag_ui_surface, with_finalized_ui_surface};
    use crate::invocation::context::{
        ConsumerKind, FrontendContext, InteractionPolicy, TerminalCapabilities,
    };
    use crate::invocation::shape::{RouteKind, Shape};

    /// A human at a fully interactive terminal with no explicit `--ui` — the
    /// case where the pre-finalize backend is `PlainTerminal` for every route
    /// and the decision matrix is what picks the real surface.
    fn human_auto_interactive() -> FrontendContext {
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
            ui_intent: crate::invocation::flags::UiFlag::Auto,
        }
    }

    /// A gathered context carrying the pre-finalize `ui_surface` label, as
    /// `gather_context` produces before any route has run.
    fn gathered_with_ui_surface(ui_surface: &'static str) -> CommandTelemetry {
        CommandTelemetry {
            distinct_id: "user-1".into(),
            identity: "authenticated",
            ui_surface,
            email: None,
            namespace: None,
            studio: None,
            command_path: "workflow run".into(),
            flags: Default::default(),
            cli_version: "1.2.3".into(),
            auth_grant: None,
            started_at: std::time::Instant::now(),
            workflow_run_id: Some("run-1".into()),
            workflow_id: Some("competitive-multiplayer".into()),
        }
    }

    /// The cross-event invariant: for one invocation every event carrying
    /// `ui_surface` carries the same value. A workflow run on an interactive
    /// terminal finalizes to fullscreen — which is the label the run and step
    /// events are built from — while the context gathered before the route ran
    /// still says `plain`. The emit-time retag is what reconciles them.
    #[test]
    fn test_command_event_ui_surface_matches_the_finalized_run_event_label() {
        let finalized =
            human_auto_interactive().finalize_surface(RouteKind::Workflow, Shape::Multi);
        // Exactly how the workflow route labels its run/step events.
        let run_event_label = finalized.surface_backend().telemetry_label();
        assert_eq!(run_event_label, "fullscreen");

        let gathered = gathered_with_ui_surface("plain");
        let emitted = retag_ui_surface(gathered, Some(run_event_label));
        assert_eq!(
            emitted.ui_surface, run_event_label,
            "cli.command.invoked must report the surface the user actually got"
        );
    }

    /// A route that never finalizes (auth, builtin) leaves the gathered label
    /// alone rather than blanking it — that label is already the surface those
    /// routes render on.
    #[test]
    fn test_retag_keeps_the_gathered_label_when_no_route_finalized() {
        let emitted = retag_ui_surface(gathered_with_ui_surface("plain"), None);
        assert_eq!(emitted.ui_surface, "plain");
    }

    /// `finalize_surface` itself must publish, so no route can forget to: the
    /// slot is populated afterwards. Serialised because the slot is process
    /// state; only its presence is asserted, since any other test's
    /// `finalize_surface` call publishes into the same slot.
    #[test]
    #[serial_test::serial]
    fn test_finalize_surface_publishes_the_label_for_the_emit_path() {
        let _finalized = human_auto_interactive().finalize_surface(RouteKind::Service, Shape::Form);
        // Read the published slot exactly once, via `with_finalized_ui_surface`
        // itself. That slot is process-global and is also written by roughly
        // nine non-serial `finalize_surface` callers elsewhere in this test
        // binary (context.rs, first_run.rs, the sibling test above);
        // `#[serial_test::serial]` only excludes other *serial* tests, so a
        // second read here could land after one of those non-serial writers
        // and observe a different label than this first read did. Comparing
        // `emitted.ui_surface` — captured from that single read — against the
        // exact label a Service/Form route resolves to on an auto-interactive
        // human context, rather than re-reading the slot for the comparison,
        // removes that race entirely while still pinning down the precise
        // value.
        let emitted = with_finalized_ui_surface(gathered_with_ui_surface("plain"));
        assert_eq!(
            emitted.ui_surface, "inline",
            "finalize_surface must publish the resolved label for the emit path to adopt"
        );
    }
}
