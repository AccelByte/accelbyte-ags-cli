//! Auth subcommands: login, logout, status.

mod oauth;

use clap::ArgMatches;

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::terminal::plain::progress;
use crate::invocation::builder;
use crate::invocation::clap_helpers;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use ags_protocol::output::{AuthOutput, CommandOutput};
use ags_runtime::runtime::config;
use ags_runtime::runtime::dispatch::http::build_http_client;
use ags_runtime::support::strings::strip_terminal_control_sequences;

use ags_protocol::error::RuntimeError;
use ags_runtime::runtime::auth::errors::AuthError;

/// Default localhost port for the OAuth browser-login callback server when
/// `--port` is not supplied.
const DEFAULT_CALLBACK_PORT: u16 = 8080;

/// Run the `ags auth` command path, owning frontend construction and lifecycle.
///
/// Parse/help exits happen before frontend construction; each subcommand emits
/// its own `RunStarted` boundary and `finish_auth_run` emits the matching
/// `RunFinished`.
pub(crate) async fn route_auth(
    args: &[String],
    flags: &GlobalFlags,
    backend: crate::invocation::context::PhaseBackend,
    options: crate::frontend::RenderOptions,
    frontend_context: &crate::invocation::context::FrontendContext,
) -> Result<InvocationOutcome, CliError> {
    let mut command = builder::build_auth_command();
    let argv = clap_helpers::build_argv("auth", args);

    // Parse before building any frontend, so help and parse-error exits emit
    // no run lifecycle.
    let matches = match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
        Ok(matches) => matches,
        // `--help` / missing-arg help: clap prints help itself and this
        // returns `Ok(Complete)` — help is not a run.
        Err(error) => match clap_helpers::outcome_from_clap_error(error) {
            Ok(outcome) => return Ok(outcome),
            // Parse errors render through the pre-surface frontend.
            Err(usage_error) => {
                let exit_code = usage_error.exit_code();
                let mut frontend = crate::frontend::frontend_for_surface(
                    frontend_context.pre_surface_backend(),
                    options.clone(),
                )?;
                frontend.render_error(&usage_error);
                let _ = frontend.finish();
                return Ok(InvocationOutcome::Exit(exit_code));
            }
        },
    };

    crate::invocation::register_reporter_if_plain(frontend_context);
    // The Auto policy matrix resolves Auth to a plain surface, but an explicit
    // --ui=fullscreen or --ui=inline can still upgrade it (see the
    // FullscreenFrontend branch below). It is try_emit_first_run_hint's
    // surface_backend() gate — not this call site or the matrix — that
    // suppresses the hint on an upgraded surface, so it never writes to stderr
    // mid terminal-acquisition. Help exits before this point, so auth
    // subcommands are never meta.
    crate::invocation::try_emit_first_run_hint(frontend_context, false);
    let mut frontend: Box<dyn crate::frontend::Frontend> = if matches!(
        backend,
        crate::invocation::context::PhaseBackend::FullscreenTerminalUi
    ) {
        // Explicit `--ui=fullscreen auth <sub>`: render the result/error in the
        // alt-screen Result/Error panel and dismiss with `q` (single-shot, no
        // step strip). `frontend_for_surface` would degrade fullscreen to plain.
        let title = match matches.subcommand_name() {
            Some(sub) => format!("auth {sub}"),
            None => "auth".to_string(),
        };
        Box::new(
            crate::frontend::terminal::fullscreen::frontend::FullscreenFrontend::new(
                options,
                title,
                Vec::new(), // no step strip for single-shot auth
                frontend_context.terminal.stdout_is_tty,
            )?,
        )
    } else {
        crate::frontend::frontend_for_surface(backend, options)?
    };

    let runtime = ags_runtime::runtime::Runtime::from_reqwest(
        ags_runtime::runtime::execution::ExecutionContext::default(),
        build_http_client(flags.timeout)?,
    );

    match matches.subcommand() {
        Some(("login", sub_matches)) => {
            // `RunStarted` fires before the login flow (existing-session
            // probe, credential prompts, token exchange). The callback-wait
            // spinner inside `await_callback` writes directly and is not part
            // of this lifecycle; `finish_auth_run` emits the matching
            // `RunFinished`.
            frontend.on_event(&crate::frontend::FrontendEvent::RunStarted {
                workflow_banner: None,
            });
            let login_result: Result<CommandOutput, CliError> =
                match config::resolve_profile_name(flags.profile.as_deref()) {
                    Ok(profile) => {
                        handle_auth_login(
                            sub_matches,
                            &profile,
                            &runtime,
                            frontend.as_mut(),
                            frontend_context,
                        )
                        .await
                    }
                    Err(error) => Err(error.into()),
                };
            Ok(finish_auth_run(frontend, login_result))
        }
        Some(("logout", sub_matches)) => {
            frontend.on_event(&crate::frontend::FrontendEvent::RunStarted {
                workflow_banner: None,
            });
            let result = handle_auth_logout(sub_matches, flags, &runtime).await;
            Ok(finish_auth_run(frontend, result))
        }
        Some(("status", _)) => {
            frontend.on_event(&crate::frontend::FrontendEvent::RunStarted {
                workflow_banner: None,
            });
            let result: Result<CommandOutput, CliError> =
                match config::resolve_profile_name(flags.profile.as_deref()) {
                    Ok(profile) => handle_auth_status(flags, &profile, &runtime).await,
                    Err(error) => Err(error.into()),
                };
            Ok(finish_auth_run(frontend, result))
        }
        Some(("refresh", _)) => {
            frontend.on_event(&crate::frontend::FrontendEvent::RunStarted {
                workflow_banner: None,
            });
            let refresh_result: Result<CommandOutput, CliError> =
                match config::resolve_profile_name(flags.profile.as_deref()) {
                    Ok(profile) => handle_auth_refresh(&profile, &runtime, frontend.as_mut()).await,
                    Err(error) => Err(error.into()),
                };
            Ok(finish_auth_run(frontend, refresh_result))
        }
        _ => {
            // `--help` / no subcommand: the help path is not a run.
            let _ = command.print_help();
            let _ = frontend.finish();
            Ok(InvocationOutcome::Complete)
        }
    }
}

/// Close out an auth run: emit `RunFinished`, render the result/error on the
/// owned frontend, then `finish()`.
///
/// Shared by `auth login`, `auth status`, and `auth logout` — every caller has
/// already emitted `RunStarted`. A failure returns `Exit(error.exit_code())`;
/// a final-render failure is also surfaced and downgraded to a failed exit.
fn finish_auth_run(
    mut frontend: Box<dyn crate::frontend::Frontend>,
    result: Result<CommandOutput, CliError>,
) -> InvocationOutcome {
    let outcome = match &result {
        Ok(_) => crate::frontend::RunOutcome::Success,
        Err(_) => crate::frontend::RunOutcome::Failed,
    };
    frontend.on_event(&crate::frontend::FrontendEvent::RunFinished { outcome });
    match result {
        Ok(output) => {
            if let Err(error) = frontend.render(&output) {
                let exit_code = error.exit_code();
                frontend.render_error(&error);
                let _ = frontend.finish();
                return InvocationOutcome::Exit(exit_code);
            }
            let _ = frontend.finish();
            InvocationOutcome::Complete
        }
        Err(error) => {
            let exit_code = error.exit_code();
            frontend.render_error(&error);
            let _ = frontend.finish();
            InvocationOutcome::Exit(exit_code)
        }
    }
}

// ── Login ──

/// Handle `ags auth login`.
async fn handle_auth_login(
    matches: &ArgMatches,
    profile: &str,
    runtime: &ags_runtime::runtime::Runtime,
    frontend: &mut dyn crate::frontend::Frontend,
    frontend_context: &crate::invocation::context::FrontendContext,
) -> Result<CommandOutput, CliError> {
    use ags_protocol::request::GrantType;
    use std::str::FromStr;

    let grant_type = matches
        .get_one::<String>("grant")
        .map(|s| GrantType::from_str(s).expect("clap PossibleValuesParser already validated"))
        .unwrap_or(GrantType::AuthorizationCode);

    match grant_type {
        GrantType::ClientCredentials => {
            handle_login_with_client_credentials(
                matches,
                profile,
                runtime,
                frontend,
                frontend_context,
            )
            .await
        }
        GrantType::AuthorizationCode => {
            handle_login_with_browser(matches, profile, runtime, frontend, frontend_context).await
        }
    }
}

/// Handle browser-based OAuth2 login (authorization code + PKCE).
///
/// The CLI handles all terminal interaction (printing the authorize URL,
/// running the callback server, displaying the wait spinner) and delegates
/// the token exchange + persistence to `operations::login_with_authorization_code`.
async fn handle_login_with_browser(
    matches: &ArgMatches,
    profile: &str,
    runtime: &ags_runtime::runtime::Runtime,
    frontend: &mut dyn crate::frontend::Frontend,
    frontend_context: &crate::invocation::context::FrontendContext,
) -> Result<CommandOutput, CliError> {
    let flag_base_url = matches.get_one::<String>("base-url").cloned();
    let flag_client_id = matches.get_one::<String>("client-id").cloned();
    let callback_port = matches
        .get_one::<String>("port")
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(DEFAULT_CALLBACK_PORT);

    // Non-interactive modes can't drive the browser flow, but they CAN still
    // benefit from the probe (a valid stored session means no browser is
    // needed). So we resolve identity → probe → reject only if both fail.
    let is_prompt_blocked = !frontend_context.allows_input();

    let base_url = resolve_login_value(
        flag_base_url,
        ags_runtime::runtime::auth::credentials::resolve_base_url_value(profile),
        is_prompt_blocked,
        "Enter Base URL (e.g. https://demo.accelbyte.io): ",
        "Base URL is required.",
        "Provide --base-url or set AGS_BASE_URL when running non-interactively.",
    )?;

    let client_id = resolve_login_value(
        flag_client_id,
        ags_runtime::runtime::auth::credentials::resolve_client_id_value(profile),
        is_prompt_blocked,
        "Enter Client ID: ",
        "Client ID is required.",
        "Provide --client-id or set AGS_CLIENT_ID when running non-interactively.",
    )?;

    // Probe BEFORE binding the callback port or printing any browser URL.
    // If the user already has a usable (or refreshable) session, we report
    // that and stop here — no browser tab, no callback wait.
    if let Some(output) = probe_existing_session_short_circuit(
        runtime,
        frontend,
        profile,
        base_url.clone(),
        client_id.clone(),
        "authorization code",
    )
    .await?
    {
        return Ok(output);
    }

    // Probe didn't short-circuit and the browser flow needs a TTY. Reject
    // cleanly instead of opening a browser the caller can't drive.
    if is_prompt_blocked {
        return Err(CliError::Usage {
            message: "Authorization code flow requires browser interaction and cannot run non-interactively.\n\
                     Use '--grant client-credentials' for headless authentication."
                .to_string(),
            metadata: None,
        });
    }

    let (code_verifier, code_challenge) = oauth::generate_pkce_pair();
    let state = oauth::generate_state();

    // Build the authorize URL first so an invalid base URL fails fast,
    // before we bind the callback port.
    let authorize_url = oauth::build_authorize_url(&base_url, &client_id, &state, &code_challenge)
        .map_err(ags_protocol::error::RuntimeError::from)?;

    // Bind callback port before printing the URL to avoid TOCTOU races.
    let listener = oauth::bind_callback_port(callback_port).await?;

    // The authorize-URL block writes directly to stderr — no owned login run
    // lifecycle exists yet, and stdout stays free for JSON.
    crate::frontend::write_stderr_line(""); // blank line separator
    crate::frontend::write_stderr_line(&style::info(
        "Open this URL in your browser to authenticate",
        style::is_stderr_enabled(),
    ));
    crate::frontend::write_stderr_line(&format!("    {authorize_url}"));
    crate::frontend::write_stderr_line(""); // blank line separator

    let (code, returned_state) = await_callback(listener, callback_port).await?;

    if returned_state != state {
        return Err(RuntimeError::from(AuthError::OAuthStateMismatch).into());
    }

    let view = {
        let mut sink = crate::frontend::FrontendSink::new(&mut *frontend);
        runtime
            .auth_login_authorization_code(
                profile,
                base_url,
                client_id,
                code,
                code_verifier,
                &mut sink,
            )
            .await?
    };

    Ok(CommandOutput::Auth(AuthOutput { view }))
}

/// Handle client credentials login.
async fn handle_login_with_client_credentials(
    matches: &ArgMatches,
    profile: &str,
    runtime: &ags_runtime::runtime::Runtime,
    frontend: &mut dyn crate::frontend::Frontend,
    frontend_context: &crate::invocation::context::FrontendContext,
) -> Result<CommandOutput, CliError> {
    let flag_base_url = matches.get_one::<String>("base-url").cloned();
    let flag_client_id = matches.get_one::<String>("client-id").cloned();
    let flag_client_secret = matches.get_one::<String>("client-secret").cloned();
    let flag_client_secret_stdin = matches.get_flag("client-secret-stdin");

    if flag_client_secret.is_some() {
        frontend.render_warning(
            "--client-secret is visible in shell history. Use --client-secret-stdin for better security.",
            None,
            None,
        );
    }

    let is_prompt_blocked = !frontend_context.allows_input() || flag_client_secret_stdin;

    let base_url = resolve_login_value(
        flag_base_url,
        ags_runtime::runtime::auth::credentials::resolve_base_url_value(profile),
        is_prompt_blocked,
        "Enter Base URL (e.g. https://demo.accelbyte.io): ",
        "Base URL is required.",
        "Provide --base-url or set AGS_BASE_URL when using --no-input.",
    )?;

    let client_id = resolve_login_value(
        flag_client_id,
        ags_runtime::runtime::auth::credentials::resolve_client_id_value(profile),
        is_prompt_blocked,
        "Enter Client ID: ",
        "Client ID is required.",
        "Provide --client-id or set AGS_CLIENT_ID when using --no-input.",
    )?;

    // Probe BEFORE prompting for the client secret. If the existing session
    // is good (or can be refreshed), we don't need the secret at all.
    if let Some(output) = probe_existing_session_short_circuit(
        runtime,
        frontend,
        profile,
        base_url.clone(),
        client_id.clone(),
        "client credentials",
    )
    .await?
    {
        return Ok(output);
    }

    let client_secret = resolve_client_secret_for_login(
        profile,
        flag_client_secret,
        flag_client_secret_stdin,
        is_prompt_blocked,
    )
    .await?;

    // Visual separator between credential input and the login progress
    // spinner, written to stderr to keep the JSON stdout contract intact.
    crate::frontend::write_stderr_line("");

    let view = {
        let mut sink = crate::frontend::FrontendSink::new(&mut *frontend);
        runtime
            .auth_login_client_credentials(profile, base_url, client_id, client_secret, &mut sink)
            .await?
    };

    Ok(CommandOutput::Auth(AuthOutput { view }))
}

// ── Logout ──

/// Handle `ags auth logout` or `ags auth logout --all`.
async fn handle_auth_logout(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    runtime: &ags_runtime::runtime::Runtime,
) -> Result<CommandOutput, CliError> {
    let is_all = matches.get_flag("all");

    if is_all && flags.profile.is_some() {
        return Err(CliError::Usage {
            message: "--all and --profile are mutually exclusive".into(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Use --all to log out from every profile, or --profile to target one.",
            ))),
        });
    }

    if is_all {
        let view = runtime.auth_logout_all().await?;
        return Ok(CommandOutput::Auth(AuthOutput { view }));
    }

    let profile = config::resolve_profile_name(flags.profile.as_deref())?;
    let view = runtime.auth_logout(&profile).await?;
    Ok(CommandOutput::Auth(AuthOutput { view }))
}

// ── Status ──

/// Handle `ags auth status`.
///
/// Three-state display:
/// - Authenticated: valid access token
/// - RequiresAttention: credentials present but token expired
/// - NotAuthenticated: no credentials
async fn handle_auth_status(
    _flags: &GlobalFlags,
    profile: &str,
    runtime: &ags_runtime::runtime::Runtime,
) -> Result<CommandOutput, CliError> {
    let view = runtime.auth_status(profile)?;
    Ok(CommandOutput::Auth(AuthOutput { view }))
}

// ── Refresh ──

/// Handle `ags auth refresh`.
async fn handle_auth_refresh(
    profile: &str,
    runtime: &ags_runtime::runtime::Runtime,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<CommandOutput, CliError> {
    let view = {
        let mut sink = crate::frontend::FrontendSink::new(&mut *frontend);
        runtime.auth_refresh(profile, &mut sink).await?
    };
    Ok(CommandOutput::Auth(AuthOutput { view }))
}

// ── Helpers ──

/// Probe for a usable existing session before starting a fresh login flow.
///
/// Shared by both login modes: if `profile` already has a usable (or
/// refreshable) session, returns `Some(CommandOutput)` so the caller can stop
/// and report it — no browser tab, no callback wait, no client-secret prompt.
/// Returns `None` when a fresh flow is required.
async fn probe_existing_session_short_circuit(
    runtime: &ags_runtime::runtime::Runtime,
    frontend: &mut dyn crate::frontend::Frontend,
    profile: &str,
    base_url: String,
    client_id: String,
    login_type: &'static str,
) -> Result<Option<CommandOutput>, CliError> {
    let mut sink = crate::frontend::FrontendSink::new(&mut *frontend);
    let probed = runtime
        .auth_probe_existing_session(profile, base_url, client_id, login_type, &mut sink)
        .await?;
    Ok(probed.map(|view| CommandOutput::Auth(AuthOutput { view })))
}

/// Seconds of waiting before the redirect-URI tip is printed.
const TIP_AFTER_SECS: u64 = 10;

/// Decide whether the redirect-URI tip should be emitted on this iteration.
///
/// One-shot: returns true only when the tip has not already been shown and
/// elapsed seconds have crossed `TIP_AFTER_SECS`.
fn is_tip_due(elapsed_secs: u64, is_tip_shown: bool) -> bool {
    !is_tip_shown && elapsed_secs >= TIP_AFTER_SECS
}

/// Wait for the OAuth callback server to receive the authorization code.
async fn await_callback(
    listener: tokio::net::TcpListener,
    callback_port: u16,
) -> Result<(String, String), CliError> {
    let timeout_seconds: u64 = std::env::var(ags_runtime::runtime::config::ENV_AUTH_TIMEOUT)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    let server_handle = tokio::spawn(oauth::start_callback_server(listener, timeout_seconds));
    let start = std::time::Instant::now();

    // Spinner appends "(Ctrl-C to cancel)" so the user knows
    // the wait is interruptible.
    let spinner_message = |remaining: u64| -> String {
        if remaining == 0 {
            format!(
                "Listening on http://127.0.0.1:{callback_port} for browser callback... \
                 waiting (Ctrl-C to cancel)"
            )
        } else {
            format!(
                "Listening on http://127.0.0.1:{callback_port} for browser callback... \
                 {remaining}s remaining (Ctrl-C to cancel)"
            )
        }
    };

    let mut status_line = progress::StatusLine::new(false);
    status_line.show(&spinner_message(timeout_seconds));

    let mut is_tip_shown = false;

    loop {
        if server_handle.is_finished() {
            status_line.clear();
            let result = server_handle
                .await
                .map_err(|e| RuntimeError::from(AuthError::CallbackServerError(e.to_string())))?;
            return Ok(result?);
        }
        let elapsed = start.elapsed().as_secs();
        let remaining = timeout_seconds.saturating_sub(elapsed);

        if is_tip_due(elapsed, is_tip_shown) {
            // Stop the running spinner before printing a persistent line,
            // otherwise the next blink overwrites the cursor position. A
            // cleared StatusLine is inert, so a fresh one resumes blinking
            // below the tip.
            status_line.clear();
            let tip_body = format!(
                "Stuck here after signing in? Your AccelByte IAM client must \
                 be Public and allow http://127.0.0.1:{callback_port} as a \
                 redirect URI."
            );
            let tip = crate::frontend::output::human::templates::render_tip_text(
                &tip_body,
                style::is_stderr_enabled(),
            );
            crate::frontend::write_stderr_line(&tip);
            crate::frontend::write_stderr_line("");
            is_tip_shown = true;
            status_line = progress::StatusLine::new(false);
            status_line.show(&spinner_message(remaining));
        } else {
            status_line.update(&spinner_message(remaining));
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// Resolve client secret from: flag → stdin → environment → keychain → interactive prompt.
async fn resolve_client_secret_for_login(
    profile: &str,
    flag_value: Option<String>,
    is_from_stdin: bool,
    is_prompt_blocked: bool,
) -> Result<String, CliError> {
    if let Some(secret) = flag_value {
        return Ok(secret);
    }
    if is_from_stdin {
        return read_stdin_line();
    }
    if let Ok(secret) = std::env::var(ags_runtime::runtime::config::ENV_CLIENT_SECRET) {
        return Ok(secret);
    }
    if let Some(secret) =
        ags_runtime::runtime::auth::credentials::resolve_stored_client_secret(profile).await
    {
        return Ok(secret);
    }
    if is_prompt_blocked {
        return Err(CliError::Usage {
            message: "Client secret not found. Provide --client-secret, set AGS_CLIENT_SECRET, \
             or run 'ags auth login' first to store it in the keychain."
                .to_string(),
            metadata: None,
        });
    }
    let secret =
        rpassword::prompt_password("Enter Client Secret: ").map_err(|e| CliError::Usage {
            message: format!("Failed to read secret: {e}"),
            metadata: None,
        })?;
    if secret.is_empty() {
        return Err(CliError::Usage {
            message: "Client Secret is required".to_string(),
            metadata: None,
        });
    }
    Ok(secret)
}

/// Delegate to the crate-level shared stdin reader in `errors.rs`.
fn read_stdin_line() -> Result<String, CliError> {
    crate::errors::read_stdin_line()
}

/// Resolve a value from: flag -> stored (environment/config) -> interactive prompt.
fn resolve_login_value(
    flag_value: Option<String>,
    stored_value: Option<String>,
    is_prompt_blocked: bool,
    prompt: &str,
    empty_error: &str,
    blocked_error: &str,
) -> Result<String, CliError> {
    if let Some(value) = flag_value {
        return Ok(value);
    }
    if let Some(value) = stored_value {
        return Ok(value);
    }
    if is_prompt_blocked {
        return Err(CliError::Usage {
            message: blocked_error.to_string(),
            metadata: None,
        });
    }
    crate::frontend::write_stderr(prompt);
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| CliError::Usage {
            message: format!("Failed to read input: {e}"),
            metadata: None,
        })?;
    let value = strip_terminal_control_sequences(input.trim());
    if value.is_empty() {
        return Err(CliError::Usage {
            message: empty_error.to_string(),
            metadata: None,
        });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{is_tip_due, TIP_AFTER_SECS};

    #[test]
    fn test_tip_not_due_before_threshold() {
        assert!(!is_tip_due(0, false));
        assert!(!is_tip_due(TIP_AFTER_SECS - 1, false));
    }

    #[test]
    fn test_tip_due_at_and_after_threshold_when_unshown() {
        assert!(is_tip_due(TIP_AFTER_SECS, false));
        assert!(is_tip_due(TIP_AFTER_SECS + 60, false));
    }

    #[test]
    fn test_tip_never_due_once_shown() {
        assert!(!is_tip_due(TIP_AFTER_SECS, true));
        assert!(!is_tip_due(TIP_AFTER_SECS + 60, true));
        assert!(!is_tip_due(u64::MAX, true));
    }
}

/// Tests for the auth-path run-boundary lifecycle: exactly one `RunFinished`
/// per run, the success/failure outcome mapping, exit-code parity, and the
/// render/error counts.
#[cfg(test)]
mod run_boundary_tests {
    use super::finish_auth_run;
    use crate::errors::CliError;
    use crate::frontend::event::{FrontendEvent, RunOutcome};
    use crate::frontend::Frontend;
    use crate::invocation::InvocationOutcome;
    use ags_protocol::output::{AuthOutput, AuthView, CommandOutput};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Shared mutable state for [`RecordingFrontend`], so the test can inspect
    /// what was recorded after `finish_auth_run` consumes the owned `Box`.
    #[derive(Default)]
    struct Recorded {
        events: Vec<FrontendEvent>,
        renders: usize,
        errors: usize,
        finished: bool,
    }

    /// Recording frontend backed by shared state, so a `Box<dyn Frontend>`
    /// can be handed to `finish_auth_run` and still inspected afterwards.
    struct RecordingFrontend {
        state: Rc<RefCell<Recorded>>,
    }

    impl RecordingFrontend {
        /// Build a boxed recording frontend paired with a handle to its
        /// shared state, so the test can inspect it after the box is moved.
        fn paired() -> (Box<dyn Frontend>, Rc<RefCell<Recorded>>) {
            let state = Rc::new(RefCell::new(Recorded::default()));
            (
                Box::new(RecordingFrontend {
                    state: Rc::clone(&state),
                }),
                state,
            )
        }
    }

    impl Frontend for RecordingFrontend {
        fn on_event(&mut self, event: &FrontendEvent) {
            self.state.borrow_mut().events.push(event.clone());
        }
        fn render(&mut self, _output: &CommandOutput) -> Result<(), CliError> {
            self.state.borrow_mut().renders += 1;
            Ok(())
        }
        fn render_error(&mut self, _err: &CliError) {
            self.state.borrow_mut().errors += 1;
        }
        fn render_warning(&mut self, _msg: &str, _reason: Option<&str>, _tip: Option<&str>) {}
        fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {}
        fn finish(self: Box<Self>) -> Result<(), CliError> {
            self.state.borrow_mut().finished = true;
            Ok(())
        }
    }

    /// A minimal `CommandOutput::Auth` for the success path.
    fn sample_auth_output() -> CommandOutput {
        CommandOutput::Auth(AuthOutput {
            view: AuthView::NotAuthenticated {
                next_step: None,
                tip: None,
            },
        })
    }

    #[test]
    fn test_finish_auth_run_success_emits_single_runfinished_and_renders() {
        let (frontend, state) = RecordingFrontend::paired();
        let outcome = finish_auth_run(frontend, Ok(sample_auth_output()));
        assert!(matches!(outcome, InvocationOutcome::Complete));

        let recorded = state.borrow();
        // `finish_auth_run` owns exactly ONE `RunFinished`; `RunStarted` is
        // emitted by the caller, so this helper records a single event.
        assert_eq!(recorded.events.len(), 1, "exactly one RunFinished");
        assert!(matches!(
            recorded.events[0],
            FrontendEvent::RunFinished {
                outcome: RunOutcome::Success
            }
        ));
        assert_eq!(recorded.renders, 1);
        assert_eq!(recorded.errors, 0);
        assert!(recorded.finished);
    }

    #[test]
    fn test_finish_auth_run_failure_preserves_exit_code_and_renders_error() {
        let (frontend, state) = RecordingFrontend::paired();
        let err = CliError::Usage {
            message: "boom".into(),
            metadata: None,
        };
        let exit_code = err.exit_code();
        let outcome = finish_auth_run(frontend, Err(err));
        // Exit-code parity: the real `CliError::exit_code()` is surfaced,
        // never a hardcoded `Exit(1)`.
        match outcome {
            InvocationOutcome::Exit(code) => assert_eq!(code, exit_code),
            other => panic!("expected Exit, got {other:?}"),
        }
        let recorded = state.borrow();
        assert_eq!(recorded.events.len(), 1);
        assert!(matches!(
            recorded.events[0],
            FrontendEvent::RunFinished {
                outcome: RunOutcome::Failed
            }
        ));
        assert_eq!(recorded.errors, 1);
        assert_eq!(recorded.renders, 0);
        assert!(recorded.finished);
    }

    /// Parse errors must render through the pre-surface frontend so
    /// automation consumers keep the JSON error envelope.
    #[test]
    fn test_auth_parse_error_does_not_fall_through_to_human() {
        use crate::frontend::RenderOptions;
        use crate::invocation::context::PhaseBackend;
        use crate::invocation::context::{
            ConsumerKind, FrontendContext, InteractionPolicy, TerminalCapabilities,
        };

        // An automation (JSON) consumer: pre-surface backend resolves to JSON.
        let automation_ctx = FrontendContext {
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
            ui_intent: crate::invocation::flags::UiFlag::Auto,
        };
        assert_eq!(
            automation_ctx.pre_surface_backend(),
            PhaseBackend::StructuredJson,
            "automation consumer must render parse errors through the JSON pre-surface backend"
        );

        let flags = crate::invocation::flags::GlobalFlags::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("tokio runtime");
        // `auth login --badflag` is a genuine clap parse error.
        let outcome = runtime.block_on(super::route_auth(
            &["login".to_string(), "--badflag".to_string()],
            &flags,
            // `backend` could be Tui; the parse-error path must NOT use it —
            // it uses `frontend_context.pre_surface_backend()` instead.
            PhaseBackend::StructuredJson,
            RenderOptions::default(),
            &automation_ctx,
        ));

        // The outcome is `Ok(Exit(..))` — `route_auth` rendered the
        // usage error itself. A bare `Err` would mean it escaped to
        // `finish_self_owned`'s hardcoded Human frontend.
        match outcome {
            Ok(InvocationOutcome::Exit(code)) => {
                let usage = CliError::Usage {
                    message: String::new(),
                    metadata: None,
                };
                assert_eq!(
                    code,
                    usage.exit_code(),
                    "exit code must be the CliError::Usage code"
                );
            }
            Ok(other) => panic!("expected Ok(Exit(..)), got Ok({other:?})"),
            Err(error) => panic!(
                "parse error fell through as Err — it would reach finish_self_owned's \
                 Human path: {error:?}"
            ),
        }
    }

    /// `auth --help` is not a run and not a parse error: clap prints help and
    /// `route_auth` returns `Ok(Complete)` — unchanged by the
    /// pre-surface parse-error fix.
    #[test]
    fn test_auth_help_returns_complete_unchanged() {
        use crate::frontend::RenderOptions;
        use crate::invocation::context::PhaseBackend;
        use crate::invocation::context::{
            ConsumerKind, FrontendContext, InteractionPolicy, TerminalCapabilities,
        };

        let human_ctx = FrontendContext {
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
                color_force_off: true,
            },
            ui_intent: crate::invocation::flags::UiFlag::Auto,
        };

        let flags = crate::invocation::flags::GlobalFlags::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("tokio runtime");
        let outcome = runtime.block_on(super::route_auth(
            &["--help".to_string()],
            &flags,
            PhaseBackend::PlainTerminal,
            RenderOptions::default(),
            &human_ctx,
        ));
        assert!(
            matches!(outcome, Ok(InvocationOutcome::Complete)),
            "auth --help must return Ok(Complete), got {outcome:?}"
        );
    }

    #[test]
    fn test_auth_run_boundary_is_a_single_pair() {
        // Emulate one full auth run: a subcommand arm emits `RunStarted`
        // before its work, then `finish_auth_run` emits `RunFinished`. The
        // result is exactly ONE run-boundary pair — `auth login` does not
        // double-emit, because it no longer routes through the top-level
        // builtin route that would have wrapped it a second time.
        let (mut frontend, state) = RecordingFrontend::paired();
        frontend.on_event(&FrontendEvent::RunStarted {
            workflow_banner: None,
        });
        let _ = finish_auth_run(frontend, Ok(sample_auth_output()));

        let recorded = state.borrow();
        assert_eq!(
            recorded.events.len(),
            2,
            "exactly one RunStarted/RunFinished pair"
        );
        assert!(matches!(
            recorded.events[0],
            FrontendEvent::RunStarted {
                workflow_banner: None
            }
        ));
        assert!(matches!(
            recorded.events[1],
            FrontendEvent::RunFinished {
                outcome: RunOutcome::Success
            }
        ));
    }
}
