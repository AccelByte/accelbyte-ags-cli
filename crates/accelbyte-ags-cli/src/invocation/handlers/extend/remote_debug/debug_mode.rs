//! Shared handler skeleton for `ags extend remote-debug enable` and
//! `ags extend remote-debug disable`.
//!
//! Both commands follow the same sequence: resolve namespace and app,
//! validate inputs, optionally emit a performance warning, check for
//! dry-run, build a runtime, GET the debug info to check `appStatus`,
//! conditionally confirm with the user, PUT the debug-mode update, and
//! emit a success message. The only differences are the target boolean,
//! the prompt and error wording, the dry-run text, and whether a
//! performance warning is emitted.

use clap::ArgMatches;

use ags_protocol::event::{ProgressEvent, ProgressSink};

use crate::errors::CliError;
use crate::frontend::{write_stderr, write_stderr_line};
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;

/// No-op progress sink for internal API dispatch. Used by the shared
/// debug-mode handler for its background debug-info and debug-mode-update
/// calls.
pub(super) struct SilentSink;

impl ProgressSink for SilentSink {
    fn on_event(&mut self, _event: ProgressEvent) {}
}

/// Read and trim one line from stdin. Production path only; tests inject
/// a closure that never touches stdin.
pub(super) fn read_line_from_stdin() -> Result<String, CliError> {
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| CliError::Usage {
            message: format!("Failed to read input: {e}"),
            metadata: None,
        })?;
    Ok(input.trim().to_string())
}

/// Determine whether the app status requires a confirmation prompt.
///
/// Returns `true` when `appStatus` is `"deployment-running"`, indicating
/// the app is currently running and changing debug mode will restart it.
pub(crate) fn should_prompt_for_running_app(app_status: Option<&str>) -> bool {
    app_status == Some("deployment-running")
}

/// Static parameters that differentiate the enable and disable commands.
///
/// All per-command text and behaviour differences are captured in one
/// struct so a reader can see the complete difference between the two
/// commands in one place.
pub(super) struct DebugModeParams {
    /// Command label for error messages, e.g. `"remote-debug enable"`.
    pub command_label: &'static str,
    /// The boolean value sent in the `enableDebugMode` JSON field.
    pub enable_debug_mode: bool,
    /// Error message when `--no-input` is set without `--yes` and the
    /// app is running, e.g. `"Enabling debug mode on a running app
    /// requires confirmation"`.
    pub no_input_error_message: &'static str,
    /// Interactive prompt text, e.g. `"The app is currently running.
    /// Enabling debug mode will restart it. Continue? [y/N] "`.
    pub prompt_message: &'static str,
    /// Dry-run info line, e.g. `"Dry run — debug mode will not be
    /// enabled and the app will not be restarted"`.
    pub dry_run_info_message: &'static str,
    /// Dry-run action line, e.g. `"would enable debug mode via PUT
    /// debugmode"`.
    pub dry_run_action: &'static str,
    /// Performance warning emitted before any API call. `Some` for
    /// enable (adds resource overhead), `None` for disable (removes it).
    pub performance_warning: Option<&'static str>,
    /// Format the success message given `(app, namespace)`.
    pub format_success: fn(&str, &str) -> String,
}

/// Confirmation logic with injected line reader, parameterised by the
/// command-specific wording.
///
/// - `--yes` / `-y` → skip prompt, return `Ok`
/// - non-promptable invocation → return `CliError::Usage` naming `--yes`
/// - interactive → prompt, accept only `"y"` / `"Y"`
///
/// The non-promptable gate reads `ctx.allows_input()`, which covers
/// `--no-input`, `--format=json`, and a terminal that cannot prompt
/// (piped stdin or non-TTY stderr). The context is the single source
/// of truth for promptability — handlers do not re-examine raw flags.
pub(super) fn confirm_debug_mode_impl(
    flags: &GlobalFlags,
    read: &mut dyn FnMut() -> Result<String, CliError>,
    params: &DebugModeParams,
    ctx: &crate::invocation::context::FrontendContext,
) -> Result<(), CliError> {
    if flags.is_auto_confirmed {
        return Ok(());
    }

    // Gate on ctx.allows_input(): covers --no-input, --format=json, and
    // a terminal that cannot prompt (piped stdin or non-TTY stderr).
    // Precedent: routes/service/mod.rs gates on !allows_input() with the
    // same three cases. input_unavailable_reason() gives the context line
    // so the error explains WHY the prompt is unavailable.
    if !ctx.allows_input() {
        let reason = crate::invocation::context::input_unavailable_reason(&ctx.terminal);
        return Err(CliError::Usage {
            message: params.no_input_error_message.to_string(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata {
                reason: Some(reason.to_string()),
                suggestion: Some("Use --yes to confirm in non-interactive mode".to_string()),
                ..Default::default()
            })),
        });
    }

    write_stderr(params.prompt_message);

    let input = read()?;

    if !matches!(input.as_str(), "y" | "Y") {
        return Err(CliError::Usage {
            message: "Operation cancelled".to_string(),
            metadata: None,
        });
    }

    Ok(())
}

/// Dry-run preview: no auth, no network. Prints what would happen and exits.
fn dry_run_preview(
    namespace: &str,
    app: &str,
    params: &DebugModeParams,
) -> Result<InvocationOutcome, CliError> {
    let color = crate::frontend::style::is_stderr_enabled();
    write_stderr_line(&crate::frontend::style::info(
        params.dry_run_info_message,
        color,
    ));
    write_stderr_line(&format!("  Namespace: {namespace}"));
    write_stderr_line(&format!("  App:       {app}"));
    // The verb "PUT" matches the method resolved from
    // OperationId::new("csm/admin/debug/v4/update") at the update call site
    // below. This literal is intentionally NOT derived from spec metadata:
    // dry_run_preview must stay free of auth, network, and catalogue
    // dependencies. If a spec regeneration changes the method, update this
    // string to match.
    write_stderr_line(&format!("  Action:    {}", params.dry_run_action));
    Ok(InvocationOutcome::Complete)
}

/// Shared handler implementation for both enable and disable.
///
/// Resolves inputs, validates, optionally emits a performance warning,
/// checks for dry-run, builds a runtime, GETs debug info, conditionally
/// confirms, PUTs the debug-mode update, and emits a success message.
pub(super) async fn handle_debug_mode_impl(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    _frontend: &mut dyn crate::frontend::Frontend,
    read: &mut dyn FnMut() -> Result<String, CliError>,
    params: &DebugModeParams,
    ctx: &crate::invocation::context::FrontendContext,
) -> Result<InvocationOutcome, CliError> {
    use ags_protocol::catalogue::{OperationId, ServiceId};
    use ags_protocol::output::CommandOutput;
    use ags_protocol::request::{
        CommandRequest, OutputFormat, PaginationHint, RequestBody, Verbosity,
    };
    use std::collections::BTreeMap;

    // ── Resolve inputs ──

    let namespace = super::resolve_namespace(flags, params.command_label)?;

    let app = matches
        .get_one::<String>("app")
        .ok_or_else(|| CliError::Usage {
            message: format!("--app is required for {}", params.command_label),
            metadata: None,
        })?
        .clone();

    // ── Validate inputs ──

    super::super::app_ui::upload::validate_safe_component(&namespace, "namespace")?;
    super::super::app_ui::upload::validate_safe_component(&app, "app")?;

    // ── Emit performance warning (enable only; disable passes None) ──

    if let Some(warning) = params.performance_warning {
        write_stderr_line(warning);
    }

    // ── Dry-run short-circuit: no auth, no network ──
    // Matches the pattern in update_var, update_secret, and image_upload.

    if flags.is_dry_run {
        return dry_run_preview(&namespace, &app, params);
    }

    // ── Build runtime ──

    let input = ags_runtime::runtime::execution::ResolutionInput {
        profile: flags.profile.clone(),
        namespace: flags.namespace.clone(),
        is_dry_run: flags.is_dry_run,
    };
    let http_client = ags_runtime::runtime::dispatch::http::build_http_client(flags.timeout)?;
    let context =
        ags_runtime::runtime::execution::ExecutionContext::resolve(&input, &http_client).await?;
    let mut runtime = ags_runtime::runtime::Runtime::from_reqwest(context, http_client);

    // ── Step 1: GET debug info to check appStatus ──

    let mut path_params = BTreeMap::new();
    path_params.insert("namespace".to_string(), namespace.clone());
    path_params.insert("app".to_string(), app.clone());

    let get_request = CommandRequest {
        service: ServiceId::new("csm"),
        operation_id: OperationId::new("csm/admin/debug/v4/get"),
        namespace: Some(namespace.clone()),
        path_params: path_params.clone(),
        query_params: BTreeMap::new(),
        header_params: BTreeMap::new(),
        form_params: BTreeMap::new(),
        body: None,
        output_format: OutputFormat::Json,
        pagination: PaginationHint::Auto,
        verbosity: Verbosity::Quiet,
        output: None,
    };

    let mut sink = SilentSink;
    let get_output = runtime
        .run_command(&get_request, &mut sink)
        .await
        .map_err(CliError::from)?;

    // Extract appStatus from the GET response.
    let app_status: Option<String> = match get_output {
        CommandOutput::Service(api_output) => api_output
            .raw_body
            .as_ref()
            .and_then(|body| body.get("appStatus"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    };

    // ── Step 2: Conditional confirmation ──

    if should_prompt_for_running_app(app_status.as_deref()) {
        confirm_debug_mode_impl(flags, read, params, ctx)?;
    }

    // ── Step 3: Send the update ──

    let body = serde_json::json!({"enableDebugMode": params.enable_debug_mode});

    let update_request = CommandRequest {
        service: ServiceId::new("csm"),
        operation_id: OperationId::new("csm/admin/debug/v4/update"),
        namespace: Some(namespace.clone()),
        path_params,
        query_params: BTreeMap::new(),
        header_params: BTreeMap::new(),
        form_params: BTreeMap::new(),
        body: Some(RequestBody::Json(body)),
        output_format: OutputFormat::Json,
        pagination: PaginationHint::Auto,
        verbosity: Verbosity::Quiet,
        output: None,
    };

    let mut sink2 = SilentSink;
    runtime
        .run_command(&update_request, &mut sink2)
        .await
        .map_err(CliError::from)?;

    // Emit a success confirmation matching the tool being replaced.
    let msg = (params.format_success)(&app, &namespace);
    write_stderr_line(&crate::frontend::style::success(
        &msg,
        crate::frontend::style::is_stderr_enabled(),
    ));

    Ok(InvocationOutcome::Complete)
}

// ── Test helpers shared by enable::tests and disable::tests ──
//
// Each helper was previously duplicated in both test modules.
// Defined once here and re-imported by each consumer.

/// RAII guard that restores an environment variable after a test mutates it.
// Env-mutating tests must be #[serial_test::serial] per repo convention.
#[cfg(test)]
pub(super) struct TempEnvGuard {
    key: &'static str,
    original: Option<String>,
}

#[cfg(test)]
impl TempEnvGuard {
    pub(super) fn set(key: &'static str, value: &str) -> Self {
        use std::env;
        let original = env::var(key).ok();
        env::set_var(key, value);
        Self { key, original }
    }

    pub(super) fn clear(key: &'static str) -> Self {
        use std::env;
        let original = env::var(key).ok();
        env::remove_var(key);
        Self { key, original }
    }
}

#[cfg(test)]
impl Drop for TempEnvGuard {
    fn drop(&mut self) {
        use std::env;
        match &self.original {
            Some(val) => env::set_var(self.key, val),
            None => env::remove_var(self.key),
        }
    }
}

#[cfg(test)]
pub(super) struct NullFrontend;

#[cfg(test)]
impl crate::frontend::Frontend for NullFrontend {
    fn render(&mut self, _output: &ags_protocol::output::CommandOutput) -> Result<(), CliError> {
        Ok(())
    }
    fn render_error(&mut self, _err: &CliError) {}
    fn render_warning(&mut self, _msg: &str, _reason: Option<&str>, _tip: Option<&str>) {}
    fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {}
    fn finish(self: Box<Self>) -> Result<(), CliError> {
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn isolated_runtime_env(
    tmp: &tempfile::TempDir,
    server: &wiremock::MockServer,
) -> [TempEnvGuard; 4] {
    [
        TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap()),
        TempEnvGuard::set("AGS_NO_KEYCHAIN", "1"),
        TempEnvGuard::set("AGS_ACCESS_TOKEN", "fake-test-token"),
        TempEnvGuard::set("AGS_BASE_URL", &server.uri()),
    ]
}

/// A non-promptable human context: piped stdin, so `allows_input()` is
/// false even without `--no-input`. Exercises the gate the defect
/// bypasses: piped stdin → empty read → "Operation cancelled" instead
/// of the correct "requires confirmation" error.
#[cfg(test)]
pub(super) fn non_promptable_context() -> crate::invocation::context::FrontendContext {
    crate::invocation::context::FrontendContext {
        consumer: crate::invocation::context::ConsumerKind::Human,
        interaction: crate::invocation::context::InteractionPolicy {
            allow_input: false,
            prefer_rich_ui: false,
            prefer_fullscreen: false,
        },
        terminal: crate::invocation::context::TerminalCapabilities {
            stdin_is_tty: false,
            stdout_is_tty: false,
            stderr_is_tty: false,
            color_force_off: true,
        },
        ui_intent: crate::invocation::flags::UiFlag::Auto,
    }
}

/// An automation consumer context (`--format=json`): `allows_input()` is
/// false because machine-readable output must never prompt.
#[cfg(test)]
pub(super) fn automation_context() -> crate::invocation::context::FrontendContext {
    crate::invocation::context::FrontendContext {
        consumer: crate::invocation::context::ConsumerKind::Automation,
        interaction: crate::invocation::context::InteractionPolicy {
            allow_input: false,
            prefer_rich_ui: false,
            prefer_fullscreen: false,
        },
        terminal: crate::invocation::context::TerminalCapabilities {
            stdin_is_tty: false,
            stdout_is_tty: false,
            stderr_is_tty: false,
            color_force_off: true,
        },
        ui_intent: crate::invocation::flags::UiFlag::Auto,
    }
}

/// A fully promptable human context: all streams are TTYs and no
/// automation flag is set, so `allows_input()` is true and the
/// interactive prompt path is reachable.
#[cfg(test)]
pub(super) fn promptable_context() -> crate::invocation::context::FrontendContext {
    crate::invocation::context::FrontendContext {
        consumer: crate::invocation::context::ConsumerKind::Human,
        interaction: crate::invocation::context::InteractionPolicy {
            allow_input: true,
            prefer_rich_ui: false,
            prefer_fullscreen: false,
        },
        terminal: crate::invocation::context::TerminalCapabilities {
            stdin_is_tty: true,
            stdout_is_tty: true,
            stderr_is_tty: true,
            color_force_off: false,
        },
        ui_intent: crate::invocation::flags::UiFlag::Auto,
    }
}
