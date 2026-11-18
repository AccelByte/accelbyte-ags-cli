//! Auth subcommands: login, logout, status, token, refresh.

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

/// Prompt shown when the user needs to enter a Base URL, in both login flows.
const BASE_URL_PROMPT: &str =
    "Enter Base URL (e.g. https://<studio-id>-<title-id>.prod.gamingservices.accelbyte.io): ";

/// Prompt shown when the user needs to enter a Client ID in the browser flow,
/// with the option to leave it empty to create a new one.
const CLIENT_ID_PROMPT: &str =
    "Enter Client ID (e.g. 2c7308eb4f3d48e3b892fa0794f14a9c, or leave empty to create a new one): ";

/// Prompt shown after the create-client link, asking for the new client's ID.
const NEW_CLIENT_ID_PROMPT: &str = "Paste the new client's Client ID: ";

/// Prompt shown when the user needs to enter a Client ID in the
/// client-credentials flow (no create option).
const CREDENTIALS_CLIENT_ID_PROMPT: &str =
    "Enter Client ID (e.g. 2c7308eb4f3d48e3b892fa0794f14a9c): ";

/// Build the line naming which IAM client the CLI is signing in with.
fn signing_in_line(client_id: &str) -> String {
    format!("Signing in with IAM client {client_id}")
}

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
    crate::invocation::try_emit_first_run_hint(
        frontend_context,
        crate::invocation::HintCommand::Auth,
        flags.profile.as_deref(),
    );
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
                            flags.namespace.as_deref(),
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
        Some(("token", _)) => {
            frontend.on_event(&crate::frontend::FrontendEvent::RunStarted {
                workflow_banner: None,
            });
            if flags.is_dry_run {
                let err = CliError::Usage {
                    message: "ags auth token does not support --dry-run because its output is a live credential.".into(),
                    metadata: None,
                };
                return Ok(finish_auth_run(frontend, Err(err)));
            }
            let token_result: Result<CommandOutput, CliError> =
                match config::resolve_profile_name(flags.profile.as_deref()) {
                    Ok(profile) => handle_auth_token(&profile, &runtime).await,
                    Err(error) => Err(error.into()),
                };
            Ok(finish_auth_run(frontend, token_result))
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
    namespace_flag: Option<&str>,
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
            handle_login_with_browser(
                matches,
                profile,
                namespace_flag,
                runtime,
                frontend,
                frontend_context,
            )
            .await
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
    namespace_flag: Option<&str>,
    runtime: &ags_runtime::runtime::Runtime,
    frontend: &mut dyn crate::frontend::Frontend,
    frontend_context: &crate::invocation::context::FrontendContext,
) -> Result<CommandOutput, CliError> {
    let flag_base_url = matches.get_one::<String>("base-url").cloned();
    let flag_client_id = matches.get_one::<String>("client-id").cloned();
    let callback_port = matches
        .get_one::<u16>("port")
        .copied()
        .unwrap_or(DEFAULT_CALLBACK_PORT);

    // Non-interactive modes can't drive the browser flow, but they CAN still
    // benefit from the probe (a valid stored session means no browser is
    // needed). So we resolve identity → probe → reject only if both fail.
    let is_prompt_blocked = !frontend_context.allows_input();

    let base_url = resolve_login_value(
        flag_base_url,
        ags_runtime::runtime::auth::credentials::resolve_base_url_value(profile),
        is_prompt_blocked,
        BASE_URL_PROMPT,
        "Base URL is required.",
        "Provide --base-url or set AGS_BASE_URL when running non-interactively.",
    )?;

    // Resolve client ID with source tracking so error messages can name
    // where the value came from (--client-id, AGS_CLIENT_ID, or profile).
    let (client_id, source, redirect_hint_shown) = if let Some(id) = flag_client_id {
        (id, ClientIdSource::Flag, false)
    } else if let Some((id, credential_source)) =
        ags_runtime::runtime::auth::credentials::resolve_client_id(profile)
    {
        let source = match credential_source {
            ags_runtime::runtime::auth::credentials::CredentialSource::Environment => {
                ClientIdSource::Environment
            }
            _ => ClientIdSource::Profile(profile.to_string()),
        };
        (id, source, false)
    } else if is_prompt_blocked {
        return Err(CliError::Usage {
            message: "Provide --client-id or set AGS_CLIENT_ID when running non-interactively."
                .to_string(),
            metadata: None,
        });
    } else {
        let (id, hint) =
            prompt_client_id_with_create_guide(&base_url, profile, namespace_flag, callback_port)?;
        (id, ClientIdSource::Prompt, hint)
    };

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

    // Generate the PKCE pair and state before the check so the check and
    // the browser use the same authorize URL.
    let (code_verifier, code_challenge) = oauth::generate_pkce_pair();
    let state = oauth::generate_state();

    // Check the client ID with IAM before the browser step. A wrong
    // client ID is the more specific error, so a non-interactive caller
    // sees it before the "requires browser interaction" refusal.
    let (client_id, authorize_url, skip_old_port_warning, redirect_hint_shown) =
        verify_client_with_iam(
            &base_url,
            client_id,
            &source,
            &code_challenge,
            &state,
            callback_port,
            frontend,
            profile,
            namespace_flag,
            redirect_hint_shown,
        )
        .await?;

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

    // When the check returned a redirect_uri, its verdict replaces
    // today's port warning. Otherwise, the old warning still applies.
    if !skip_old_port_warning && callback_port != DEFAULT_CALLBACK_PORT && !redirect_hint_shown {
        frontend.render_warning(
            &format!(
                "The login callback will use port {callback_port}. Your IAM client's redirect URI must be http://127.0.0.1:{callback_port}."
            ),
            Some("Otherwise the browser is redirected to a port the CLI isn't listening on, and login waits until it times out."),
            None,
        );
    }

    // Bind callback port before printing the URL to avoid TOCTOU races.
    let listener = oauth::bind_callback_port(callback_port).await?;

    // The authorize-URL block writes directly to stderr — no owned login run
    // lifecycle exists yet, and stdout stays free for JSON.
    crate::frontend::write_stderr_line(&style::info(
        &signing_in_line(&client_id),
        style::is_stderr_enabled(),
    ));
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
        BASE_URL_PROMPT,
        "Base URL is required.",
        "Provide --base-url or set AGS_BASE_URL when using --no-input.",
    )?;

    let client_id = resolve_login_value(
        flag_client_id,
        ags_runtime::runtime::auth::credentials::resolve_client_id_value(profile),
        is_prompt_blocked,
        CREDENTIALS_CLIENT_ID_PROMPT,
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

// ── Token ──

/// Handle `ags auth token`.
///
/// Resolution is delegated to `Runtime::auth_token`, which runs the same
/// session resolver the request path runs, so the printed token is the one the
/// next API call would send. No progress sink is passed: the whole point of the
/// command is a clean stdout, and a refresh here is incidental rather than the
/// user's stated intent.
async fn handle_auth_token(
    profile: &str,
    runtime: &ags_runtime::runtime::Runtime,
) -> Result<CommandOutput, CliError> {
    let view = runtime.auth_token(profile).await?;
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

    let spinner_message =
        |remaining: u64| -> String { login_wait_message(callback_port, remaining) };

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
    require_input(read_prompt_line(prompt)?, empty_error)
}

/// Prompt for a public client ID, where an empty answer links to the Admin
/// Portal form for creating one from the `ags_cli` template. Returns the
/// client ID and whether the guide, which states the redirect URI for
/// `callback_port`, was shown.
fn prompt_client_id_with_create_guide(
    base_url: &str,
    profile: &str,
    namespace_flag: Option<&str>,
    callback_port: u16,
) -> Result<(String, bool), CliError> {
    let client_id = read_prompt_line(CLIENT_ID_PROMPT)?;
    if !client_id.is_empty() {
        return Ok((client_id, false));
    }

    let namespace =
        match ags_runtime::runtime::execution::resolve_namespace(namespace_flag, Some(profile)) {
            Some((namespace, _)) => namespace,
            None => require_input(
                read_prompt_line("Enter the namespace to create the IAM client in: ")?,
                "Namespace is required to create an IAM client.",
            )?,
        };
    let create_url = create_client_url(base_url, &namespace)?;

    crate::frontend::write_stderr_line("");
    crate::frontend::write_stderr_line(&style::info(
        "Open this URL in your browser, create the IAM client, and copy its Client ID",
        style::is_stderr_enabled(),
    ));
    crate::frontend::write_stderr_line(&format!("    {create_url}"));
    if let Some(warning) = create_client_port_warning(callback_port) {
        crate::frontend::write_stderr_line(&style::warning(&warning, style::is_stderr_enabled()));
    }
    for line in create_client_form_fields(callback_port) {
        crate::frontend::write_stderr_line(&style::ansi::dim(&line, style::is_stderr_enabled()));
    }
    crate::frontend::write_stderr_line("");

    let client_id = require_input(
        read_prompt_line(NEW_CLIENT_ID_PROMPT)?,
        "Client ID is required.",
    )?;
    Ok((client_id, true))
}

/// The `ags_cli` template registers the default callback port, so any other
/// `--port` needs the client's redirect URI changed before login can complete.
fn create_client_port_warning(callback_port: u16) -> Option<String> {
    (callback_port != DEFAULT_CALLBACK_PORT).then(|| {
        format!(
            "The AGS CLI template registers http://127.0.0.1:{DEFAULT_CALLBACK_PORT}. Because you're using --port {callback_port}, set the client's redirect URI to http://127.0.0.1:{callback_port}."
        )
    })
}

/// Values to enter under the create-client link. Admin Portals without the
/// `ags_cli` template open the form empty.
fn create_client_form_fields(callback_port: u16) -> [String; 4] {
    [
        "If the form is empty, enter:".to_string(),
        "    Client Type:   Public".to_string(),
        "    Client Name:   AGS CLI".to_string(),
        format!("    Redirect URI:  http://127.0.0.1:{callback_port}"),
    ]
}

/// Build the Admin Portal link for creating an IAM client in `namespace`.
fn create_client_url(base_url: &str, namespace: &str) -> Result<String, CliError> {
    // Besides rejecting malformed input, this keeps `.` and `..` out of the
    // link: the URL parser drops those segments, removing the namespace.
    if !ags_runtime::runtime::config::is_valid_namespace(namespace) {
        return Err(CliError::Usage {
            message: format!(
                "Invalid namespace: {namespace}. {}.",
                ags_runtime::runtime::config::NAMESPACE_FORMAT_HINT
            ),
            metadata: None,
        });
    }
    Ok(oauth::build_create_client_url(base_url, namespace)
        .map_err(ags_protocol::error::RuntimeError::from)?)
}

/// Build the login wait message for the callback spinner.
///
/// With the status symbol prefix (`∘ `, 2 columns), the counting-down line
/// at port 8080 / 120 seconds is 79 columns, fitting an 80-column terminal.
pub(crate) fn login_wait_message(callback_port: u16, remaining: u64) -> String {
    if remaining > 0 {
        format!("Waiting for browser sign-in on 127.0.0.1:{callback_port}... {remaining}s left (Ctrl-C to cancel)")
    } else {
        format!("Waiting for browser sign-in on 127.0.0.1:{callback_port}... (Ctrl-C to cancel)")
    }
}

fn read_prompt_line(prompt: &str) -> Result<String, CliError> {
    crate::frontend::write_stderr(prompt);
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| CliError::Usage {
            message: format!("Failed to read input: {e}"),
            metadata: None,
        })?;
    Ok(strip_terminal_control_sequences(input.trim()))
}

fn require_input(value: String, empty_error: &str) -> Result<String, CliError> {
    if value.is_empty() {
        return Err(CliError::Usage {
            message: empty_error.to_string(),
            metadata: None,
        });
    }
    Ok(value)
}

// ── Client check: source tracking, messages, and decision logic ──

/// Where the client ID came from, for error messages that name the source.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ClientIdSource {
    Flag,
    Environment,
    Profile(String),
    Prompt,
}

impl std::fmt::Display for ClientIdSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Flag => write!(f, "--client-id"),
            Self::Environment => write!(f, "AGS_CLIENT_ID"),
            Self::Profile(name) => write!(f, "profile \"{name}\""),
            Self::Prompt => write!(f, "prompt"),
        }
    }
}

/// What the redirect URI from IAM means for this login.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RedirectUriCheck {
    /// Loopback host on the callback port: no warning needed.
    NoWarning,
    /// Loopback host but on a different port.
    PortMismatch { registered_port: u16 },
    /// Not a loopback host: the browser cannot return to the CLI.
    NotLoopback,
}

/// Classify a redirect URI from IAM against the login's callback port.
fn classify_redirect_uri(redirect_uri: &str, callback_port: u16) -> RedirectUriCheck {
    if let Ok(parsed) = reqwest::Url::parse(redirect_uri) {
        let host = parsed.host_str().unwrap_or("");
        let is_loopback = host == "127.0.0.1" || host == "localhost" || host == "[::1]";
        let port = parsed.port_or_known_default().unwrap_or(80);
        if is_loopback && port == callback_port {
            RedirectUriCheck::NoWarning
        } else if is_loopback {
            RedirectUriCheck::PortMismatch {
                registered_port: port,
            }
        } else {
            RedirectUriCheck::NotLoopback
        }
    } else {
        RedirectUriCheck::NotLoopback
    }
}

// ── Client check message builders ──
//
// Each function builds the exact text the spec prescribes. Every
// interpolated value passes through `terminal_safe` so the printed path
// is clean even when IAM returns escape sequences.

/// Maximum length (in characters) for values interpolated into printed
/// messages. Longer values are truncated with a trailing ellipsis.
const MAX_PRINTED_VALUE_CHARS: usize = 200;

/// Strip terminal control sequences and cap the length of a value before
/// interpolation into a printed message.
fn terminal_safe(value: &str) -> String {
    let cleaned = strip_terminal_control_sequences(value);
    if cleaned.chars().count() > MAX_PRINTED_VALUE_CHARS {
        let truncated: String = cleaned.chars().take(MAX_PRINTED_VALUE_CHARS - 1).collect();
        format!("{truncated}\u{2026}")
    } else {
        cleaned
    }
}

/// Warning shown at the prompt when IAM has no client with the entered ID.
fn client_not_found_warning_message(id: &str) -> String {
    let id = terminal_safe(id);
    format!("IAM has no client with ID \"{id}\".")
}

const CLIENT_NOT_FOUND_WARNING_TIP: &str =
    "A client ID looks like 2c7308eb4f3d48e3b892fa0794f14a9c. It is not the client name.";

/// Warning shown at the prompt when IAM rejects the entered client for another reason.
fn client_rejected_warning_message(id: &str, error: &str) -> String {
    let id = terminal_safe(id);
    let error = terminal_safe(error);
    format!("IAM rejected client \"{id}\": {error}.")
}

const CHECK_CLIENT_ID_TIP: &str = "Check the client ID in the Admin Portal.";

/// Usage error for a configured client ID that IAM does not know, naming where the ID came from.
fn client_not_found_error_message(id: &str, source: &ClientIdSource) -> String {
    let id = terminal_safe(id);
    format!("IAM has no client with ID \"{id}\" (from {source}).")
}

const CLIENT_NOT_FOUND_ERROR_SUGGESTION: &str = "Use the client ID shown in the Admin Portal. It looks like 2c7308eb4f3d48e3b892fa0794f14a9c, not the client name.";

/// Usage error for a configured client ID that IAM rejects for another reason, naming where the ID came from.
fn client_rejected_error_message(id: &str, source: &ClientIdSource, error: &str) -> String {
    let id = terminal_safe(id);
    let error = terminal_safe(error);
    format!("IAM rejected client \"{id}\" (from {source}): {error}.")
}

const NO_VALID_CLIENT_MESSAGE: &str = "No valid client ID after 3 attempts.";

const NO_VALID_CLIENT_SUGGESTION: &str =
    "Run 'ags auth login' again, or leave the Client ID prompt empty to create a new client.";

/// Warning when the client's registered redirect URI is on another loopback port than this login.
fn redirect_port_mismatch_message(redirect_uri: &str, callback_port: u16) -> String {
    let redirect_uri = terminal_safe(redirect_uri);
    format!("Your IAM client sends the browser to {redirect_uri}, but this login listens on port {callback_port}.")
}

/// Tip that names both ways to fix a redirect port mismatch.
fn redirect_port_mismatch_tip(registered_port: u16, callback_port: u16) -> String {
    format!("Run with --port {registered_port}, or add http://127.0.0.1:{callback_port} to the client's redirect URIs.")
}

/// Warning when the client's registered redirect URI is not a loopback address.
fn redirect_not_loopback_message(redirect_uri: &str) -> String {
    let redirect_uri = terminal_safe(redirect_uri);
    format!("Your IAM client sends the browser to {redirect_uri}, so the sign-in cannot return to the CLI.")
}

/// Tip that names the redirect URI to add for this login's callback port.
fn redirect_not_loopback_tip(callback_port: u16) -> String {
    format!("Add http://127.0.0.1:{callback_port} to the client's redirect URIs.")
}

/// Warning when the client check cannot reach IAM; the login continues.
fn check_unreachable_message(base_url: &str, reason: &str) -> String {
    let base_url = terminal_safe(base_url);
    let reason = terminal_safe(reason);
    format!("Could not check the client ID with IAM at {base_url}: {reason}.")
}

const CHECK_UNREACHABLE_TIP: &str =
    "The login continues. If the browser shows an error, check the Base URL and the client ID.";

/// Status line shown on a terminal while the client check waits for IAM.
const CLIENT_CHECK_STATUS: &str = "Checking the client ID with IAM...";

/// Effects needed by the client-check loop, injected so unit tests can
/// drive the loop with scripted check results, canned answers, and
/// recorded warnings.
trait ClientCheckIo {
    /// Send one GET to the authorize URL and classify IAM's redirect.
    fn check(
        &mut self,
        authorize_url: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = oauth::ClientCheck> + '_>>;
    /// Ask the user for a new client ID (with the create guide).
    /// Returns the new ID and whether the create guide was shown.
    fn ask_again(&mut self) -> Result<(String, bool), CliError>;
    /// Print a warning to the frontend.
    fn warn(&mut self, message: &str, reason: Option<&str>, tip: Option<&str>);
}

/// A unified rejection from the client check, carrying all the text needed
/// for both the warning (interactive ask-again path) and the usage error
/// (configured-ID stop path).
struct Rejection {
    warning_message: String,
    warning_reason: Option<String>,
    warning_tip: &'static str,
    configured_error_message: String,
    configured_error_suggestion: &'static str,
    configured_error_detail: Option<String>,
}

/// Run the client-check loop: check the client ID with IAM, and on
/// rejection either ask again (up to three attempts for a prompted ID) or
/// stop with a usage error (for a configured ID).
///
/// Returns `(final_client_id, authorize_url, skip_old_port_warning, redirect_hint_shown)`.
#[allow(clippy::too_many_arguments)]
async fn run_client_check_loop(
    io: &mut impl ClientCheckIo,
    base_url: &str,
    initial_client_id: String,
    source: &ClientIdSource,
    code_challenge: &str,
    state: &str,
    callback_port: u16,
    initial_redirect_hint_shown: bool,
) -> Result<(String, String, bool, bool), CliError> {
    let is_prompted = *source == ClientIdSource::Prompt;
    let mut client_id = initial_client_id;
    let mut redirect_hint_shown = initial_redirect_hint_shown;
    let mut attempt: usize = 1;

    loop {
        let authorize_url = oauth::build_authorize_url(base_url, &client_id, state, code_challenge)
            .map_err(ags_protocol::error::RuntimeError::from)?;

        let check = io.check(&authorize_url).await;

        // NotFound and Rejected both map to a Rejection value so the
        // ask-again-or-stop decision is written once.
        let rejection = match &check {
            oauth::ClientCheck::NotFound => Some(Rejection {
                warning_message: client_not_found_warning_message(&client_id),
                warning_reason: None,
                warning_tip: CLIENT_NOT_FOUND_WARNING_TIP,
                configured_error_message: client_not_found_error_message(&client_id, source),
                configured_error_suggestion: CLIENT_NOT_FOUND_ERROR_SUGGESTION,
                configured_error_detail: None,
            }),
            oauth::ClientCheck::Rejected { error, description } => {
                let clean_desc = description.as_deref().map(terminal_safe);
                Some(Rejection {
                    warning_message: client_rejected_warning_message(&client_id, error),
                    warning_reason: clean_desc.clone(),
                    warning_tip: CHECK_CLIENT_ID_TIP,
                    configured_error_message: client_rejected_error_message(
                        &client_id, source, error,
                    ),
                    configured_error_suggestion: CHECK_CLIENT_ID_TIP,
                    configured_error_detail: clean_desc,
                })
            }
            _ => None,
        };

        if let Some(rej) = rejection {
            if is_prompted && attempt < 3 {
                io.warn(
                    &rej.warning_message,
                    rej.warning_reason.as_deref(),
                    Some(rej.warning_tip),
                );
                let (new_id, guide_shown) = io.ask_again()?;
                client_id = new_id;
                redirect_hint_shown = redirect_hint_shown || guide_shown;
                attempt += 1;
                continue;
            }
            if is_prompted {
                return Err(CliError::Usage {
                    message: NO_VALID_CLIENT_MESSAGE.to_string(),
                    metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                        NO_VALID_CLIENT_SUGGESTION,
                    ))),
                });
            }
            let mut meta =
                crate::errors::ErrorMetadata::with_suggestion(rej.configured_error_suggestion);
            meta.detail = rej.configured_error_detail;
            return Err(CliError::Usage {
                message: rej.configured_error_message,
                metadata: Some(Box::new(meta)),
            });
        }

        match check {
            oauth::ClientCheck::Accepted { redirect_uri } => {
                let mut skip_port_warning = false;
                if let Some(ref uri) = redirect_uri {
                    match classify_redirect_uri(uri, callback_port) {
                        RedirectUriCheck::NoWarning => {
                            skip_port_warning = true;
                        }
                        RedirectUriCheck::PortMismatch { registered_port } => {
                            let msg = redirect_port_mismatch_message(uri, callback_port);
                            let tip = redirect_port_mismatch_tip(registered_port, callback_port);
                            io.warn(&msg, None, Some(&tip));
                            skip_port_warning = true;
                        }
                        RedirectUriCheck::NotLoopback => {
                            let msg = redirect_not_loopback_message(uri);
                            let tip = redirect_not_loopback_tip(callback_port);
                            io.warn(&msg, None, Some(&tip));
                            skip_port_warning = true;
                        }
                    }
                }
                return Ok((
                    client_id,
                    authorize_url,
                    skip_port_warning,
                    redirect_hint_shown,
                ));
            }
            oauth::ClientCheck::Inconclusive => {
                return Ok((client_id, authorize_url, false, redirect_hint_shown));
            }
            oauth::ClientCheck::Unreachable { reason } => {
                let msg = check_unreachable_message(base_url, &reason);
                io.warn(&msg, None, Some(CHECK_UNREACHABLE_TIP));
                return Ok((client_id, authorize_url, false, redirect_hint_shown));
            }
            // NotFound and Rejected are handled above via the rejection path.
            _ => unreachable!(),
        }
    }
}

/// Check the client ID with IAM before the browser step.
///
/// For a prompted client ID that IAM rejects, warns and asks again
/// (up to three answers total). For a configured client ID, returns a
/// usage error. The check never blocks a login that would otherwise work:
/// an unreachable IAM or an inconclusive response continues as today.
///
/// Returns `(final_client_id, authorize_url, skip_old_port_warning, redirect_hint_shown)`.
#[allow(clippy::too_many_arguments)]
async fn verify_client_with_iam(
    base_url: &str,
    client_id: String,
    source: &ClientIdSource,
    code_challenge: &str,
    state: &str,
    callback_port: u16,
    frontend: &mut dyn crate::frontend::Frontend,
    profile: &str,
    namespace_flag: Option<&str>,
    redirect_hint_shown: bool,
) -> Result<(String, String, bool, bool), CliError> {
    struct ProductionIo<'a> {
        frontend: &'a mut dyn crate::frontend::Frontend,
        base_url: &'a str,
        profile: &'a str,
        namespace_flag: Option<&'a str>,
        callback_port: u16,
    }

    impl<'a> ClientCheckIo for ProductionIo<'a> {
        fn check(
            &mut self,
            authorize_url: &str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = oauth::ClientCheck> + '_>> {
            let url = authorize_url.to_string();
            Box::pin(async move {
                // Cleared before returning, so no warning, prompt or sign-in
                // link is printed while the line is still on screen.
                let mut status_line = progress::StatusLine::new(false);
                status_line.show(CLIENT_CHECK_STATUS);
                let check = oauth::check_client(&url).await;
                status_line.clear();
                check
            })
        }
        fn ask_again(&mut self) -> Result<(String, bool), CliError> {
            prompt_client_id_with_create_guide(
                self.base_url,
                self.profile,
                self.namespace_flag,
                self.callback_port,
            )
        }
        fn warn(&mut self, message: &str, reason: Option<&str>, tip: Option<&str>) {
            self.frontend.render_warning(message, reason, tip);
        }
    }

    let mut io = ProductionIo {
        frontend,
        base_url,
        profile,
        namespace_flag,
        callback_port,
    };
    run_client_check_loop(
        &mut io,
        base_url,
        client_id,
        source,
        code_challenge,
        state,
        callback_port,
        redirect_hint_shown,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        check_unreachable_message, classify_redirect_uri, client_not_found_error_message,
        client_not_found_warning_message, client_rejected_error_message,
        client_rejected_warning_message, create_client_form_fields, create_client_port_warning,
        create_client_url, is_tip_due, redirect_not_loopback_message, redirect_not_loopback_tip,
        redirect_port_mismatch_message, redirect_port_mismatch_tip, ClientIdSource,
        RedirectUriCheck, CHECK_CLIENT_ID_TIP, CLIENT_CHECK_STATUS,
        CLIENT_NOT_FOUND_ERROR_SUGGESTION, CLIENT_NOT_FOUND_WARNING_TIP, NO_VALID_CLIENT_MESSAGE,
        NO_VALID_CLIENT_SUGGESTION, TIP_AFTER_SECS,
    };
    use crate::errors::CliError;

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

    #[test]
    fn test_create_client_url_uses_valid_namespace() {
        let url = create_client_url(
            "https://my-game.prod.gamingservices.accelbyte.io",
            "my-game",
        )
        .unwrap();
        assert_eq!(
            url,
            "https://my-game.prod.gamingservices.accelbyte.io/admin/namespaces/my-game/iam-clients/create?template=ags_cli"
        );
    }

    #[test]
    fn test_create_client_form_fields_use_callback_port() {
        let fields = create_client_form_fields(9000).join("\n");
        assert!(fields.contains("Client Name:   AGS CLI"), "{fields}");
        assert!(fields.contains("Client Type:   Public"), "{fields}");
        assert!(
            fields.contains("Redirect URI:  http://127.0.0.1:9000"),
            "{fields}"
        );
    }

    #[test]
    fn test_create_client_port_warning_only_for_non_default_port() {
        assert_eq!(create_client_port_warning(8080), None);
        let warning = create_client_port_warning(9000).unwrap();
        assert!(warning.contains("--port 9000"), "{warning}");
        assert!(warning.contains("http://127.0.0.1:9000"), "{warning}");
    }

    #[test]
    fn test_login_wait_message_fits_80_columns() {
        use unicode_width::UnicodeWidthStr;

        let counting = super::login_wait_message(8080, 120);
        assert_eq!(
            counting,
            "Waiting for browser sign-in on 127.0.0.1:8080... 120s left (Ctrl-C to cancel)"
        );
        // With the status symbol prefix "∘ " (2 columns), the line is 79 columns.
        let with_prefix = format!("\u{2218} {counting}");
        assert_eq!(
            UnicodeWidthStr::width(with_prefix.as_str()),
            79,
            "counting-down line with prefix must be 79 columns: {with_prefix:?}"
        );

        let zero = super::login_wait_message(8080, 0);
        assert_eq!(
            zero,
            "Waiting for browser sign-in on 127.0.0.1:8080... (Ctrl-C to cancel)"
        );
    }

    #[test]
    fn test_client_check_status_text() {
        use unicode_width::UnicodeWidthStr;

        assert_eq!(CLIENT_CHECK_STATUS, "Checking the client ID with IAM...");
        // With the status symbol prefix "∘ " (2 columns), the line fits an 80-column terminal.
        let with_prefix = format!("\u{2218} {CLIENT_CHECK_STATUS}");
        assert!(
            UnicodeWidthStr::width(with_prefix.as_str()) <= 79,
            "client check status line with prefix must be at most 79 columns: {with_prefix:?}"
        );
    }

    /// `..` would otherwise drop the namespace from the link's path
    #[test]
    fn test_create_client_url_rejects_invalid_namespace() {
        for namespace in ["..", ".", "My_Game", "a/b"] {
            match create_client_url(
                "https://my-game.prod.gamingservices.accelbyte.io",
                namespace,
            ) {
                Err(CliError::Usage { message, .. }) => {
                    assert!(message.contains("Invalid namespace"), "{message}")
                }
                other => panic!("expected usage error for '{namespace}', got {other:?}"),
            }
        }
    }

    #[test]
    fn test_base_url_prompt_names_game_namespace() {
        assert_eq!(
            super::BASE_URL_PROMPT,
            "Enter Base URL (e.g. https://<studio-id>-<title-id>.prod.gamingservices.accelbyte.io): "
        );
    }

    #[test]
    fn test_client_id_prompt_shows_example_and_empty_option() {
        assert_eq!(
            super::CLIENT_ID_PROMPT,
            "Enter Client ID (e.g. 2c7308eb4f3d48e3b892fa0794f14a9c, or leave empty to create a new one): "
        );
    }

    #[test]
    fn test_new_client_id_prompt_text() {
        assert_eq!(
            super::NEW_CLIENT_ID_PROMPT,
            "Paste the new client's Client ID: "
        );
    }

    #[test]
    fn test_client_credentials_client_id_prompt_text() {
        assert_eq!(
            super::CREDENTIALS_CLIENT_ID_PROMPT,
            "Enter Client ID (e.g. 2c7308eb4f3d48e3b892fa0794f14a9c): "
        );
    }

    #[test]
    fn test_signing_in_line_names_client() {
        assert_eq!(
            super::signing_in_line("abc123"),
            "Signing in with IAM client abc123"
        );
    }

    // ── Client check message and decision tests ──

    #[test]
    fn test_client_not_found_messages_whole_text() {
        // Interactive (prompted) case
        assert_eq!(
            client_not_found_warning_message("scott-cli"),
            "IAM has no client with ID \"scott-cli\"."
        );
        assert_eq!(
            CLIENT_NOT_FOUND_WARNING_TIP,
            "A client ID looks like 2c7308eb4f3d48e3b892fa0794f14a9c. It is not the client name."
        );
        // Configured (flag/env/profile) case
        assert_eq!(
            client_not_found_error_message("scott-cli", &ClientIdSource::Flag),
            "IAM has no client with ID \"scott-cli\" (from --client-id)."
        );
        assert_eq!(
            client_not_found_error_message("scott-cli", &ClientIdSource::Environment),
            "IAM has no client with ID \"scott-cli\" (from AGS_CLIENT_ID)."
        );
        assert_eq!(
            client_not_found_error_message(
                "scott-cli",
                &ClientIdSource::Profile("work".to_string())
            ),
            "IAM has no client with ID \"scott-cli\" (from profile \"work\")."
        );
        assert_eq!(
            CLIENT_NOT_FOUND_ERROR_SUGGESTION,
            "Use the client ID shown in the Admin Portal. It looks like 2c7308eb4f3d48e3b892fa0794f14a9c, not the client name."
        );
        // Rejected (non-not-found) cases
        assert_eq!(
            client_rejected_warning_message("bad-id", "invalid_scope"),
            "IAM rejected client \"bad-id\": invalid_scope."
        );
        assert_eq!(
            CHECK_CLIENT_ID_TIP,
            "Check the client ID in the Admin Portal."
        );
        assert_eq!(
            client_rejected_error_message("bad-id", &ClientIdSource::Flag, "invalid_scope"),
            "IAM rejected client \"bad-id\" (from --client-id): invalid_scope."
        );
        assert_eq!(
            CHECK_CLIENT_ID_TIP,
            "Check the client ID in the Admin Portal."
        );
        // Max-attempts case
        assert_eq!(
            NO_VALID_CLIENT_MESSAGE,
            "No valid client ID after 3 attempts."
        );
        assert_eq!(
            NO_VALID_CLIENT_SUGGESTION,
            "Run 'ags auth login' again, or leave the Client ID prompt empty to create a new client."
        );
    }

    #[test]
    fn test_redirect_port_mismatch_warning_text() {
        assert_eq!(
            redirect_port_mismatch_message("http://127.0.0.1:9090", 8080),
            "Your IAM client sends the browser to http://127.0.0.1:9090, but this login listens on port 8080."
        );
        assert_eq!(
            redirect_port_mismatch_tip(9090, 8080),
            "Run with --port 9090, or add http://127.0.0.1:8080 to the client's redirect URIs."
        );
    }

    #[test]
    fn test_redirect_not_loopback_warning_text() {
        assert_eq!(
            redirect_not_loopback_message("https://example.com/callback"),
            "Your IAM client sends the browser to https://example.com/callback, so the sign-in cannot return to the CLI."
        );
        assert_eq!(
            redirect_not_loopback_tip(8080),
            "Add http://127.0.0.1:8080 to the client's redirect URIs."
        );
    }

    #[test]
    fn test_redirect_on_callback_port_needs_no_warning() {
        assert_eq!(
            classify_redirect_uri("http://127.0.0.1:8080", 8080),
            RedirectUriCheck::NoWarning,
        );
        assert_eq!(
            classify_redirect_uri("http://localhost:8080", 8080),
            RedirectUriCheck::NoWarning,
        );
        assert_eq!(
            classify_redirect_uri("http://[::1]:8080", 8080),
            RedirectUriCheck::NoWarning,
        );
        // https without explicit port uses 443
        assert_eq!(
            classify_redirect_uri("https://localhost/callback", 443),
            RedirectUriCheck::NoWarning,
        );
        // Different port
        assert_eq!(
            classify_redirect_uri("http://127.0.0.1:9090", 8080),
            RedirectUriCheck::PortMismatch {
                registered_port: 9090,
            },
        );
        // Not loopback
        assert_eq!(
            classify_redirect_uri("https://example.com/callback", 8080),
            RedirectUriCheck::NotLoopback,
        );
    }

    /// A prompted client ID that is rejected gets asked again for the first
    /// and second answers; the third rejected answer stops. Exercised
    /// through the real loop with a scripted fake.
    #[test]
    fn test_prompted_id_rejected_asks_again() {
        let mut io = FakeCheckIo::new(
            vec![
                super::oauth::ClientCheck::NotFound,
                super::oauth::ClientCheck::NotFound,
                super::oauth::ClientCheck::Accepted { redirect_uri: None },
            ],
            vec![("second".to_string(), false), ("third".to_string(), false)],
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let result = rt.block_on(super::run_client_check_loop(
            &mut io,
            "https://demo.accelbyte.io",
            "first".to_string(),
            &ClientIdSource::Prompt,
            "challenge",
            "state",
            8080,
            false,
        ));
        assert!(result.is_ok(), "should have accepted on third answer");
        assert_eq!(io.warnings.len(), 2, "two ask-again warnings");
    }

    /// A configured client ID stops at the first rejection without asking
    /// again. Exercised through the real loop with a scripted fake.
    #[test]
    fn test_configured_id_rejected_stops() {
        for source in [
            ClientIdSource::Flag,
            ClientIdSource::Environment,
            ClientIdSource::Profile("p".to_string()),
        ] {
            let mut io = FakeCheckIo::new(vec![super::oauth::ClientCheck::NotFound], vec![]);
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let result = rt.block_on(super::run_client_check_loop(
                &mut io,
                "https://demo.accelbyte.io",
                "bad".to_string(),
                &source,
                "challenge",
                "state",
                8080,
                false,
            ));
            assert!(result.is_err(), "configured source {source} should stop");
            assert!(
                io.warnings.is_empty(),
                "configured source {source} should not warn"
            );
        }
    }

    /// Each message builder strips terminal control sequences from every
    /// value it interpolates. The test gives each builder RAW values and
    /// compares the result whole against the clean text.
    #[test]
    fn test_iam_values_are_stripped_before_printing() {
        // RAW values containing ANSI SGR, reset, and BEL.
        let dirty_id = "scott\x1b[31m-cli\x1b[0m\x07";
        let dirty_error = "invalid\x1b[31m_scope\x1b[0m\x07";
        let dirty_uri = "http://evil\x1b[31m.example\x1b[0m.com\x07:9090";
        let dirty_reason = "connection \x1b[31mrefused\x1b[0m\x07";
        let dirty_base = "https://demo\x1b[31m.accelbyte\x1b[0m.io\x07";

        // 1. client_not_found_warning_message
        assert_eq!(
            client_not_found_warning_message(dirty_id),
            "IAM has no client with ID \"scott-cli\"."
        );

        // 2. client_rejected_warning_message
        assert_eq!(
            client_rejected_warning_message(dirty_id, dirty_error),
            "IAM rejected client \"scott-cli\": invalid_scope."
        );

        // 3. client_not_found_error_message
        assert_eq!(
            client_not_found_error_message(dirty_id, &ClientIdSource::Flag),
            "IAM has no client with ID \"scott-cli\" (from --client-id)."
        );

        // 4. client_rejected_error_message
        assert_eq!(
            client_rejected_error_message(dirty_id, &ClientIdSource::Flag, dirty_error),
            "IAM rejected client \"scott-cli\" (from --client-id): invalid_scope."
        );

        // 5. redirect_port_mismatch_message
        assert_eq!(
            redirect_port_mismatch_message(dirty_uri, 8080),
            "Your IAM client sends the browser to http://evil.example.com:9090, but this login listens on port 8080."
        );

        // 6. redirect_not_loopback_message
        assert_eq!(
            redirect_not_loopback_message(dirty_uri),
            "Your IAM client sends the browser to http://evil.example.com:9090, so the sign-in cannot return to the CLI."
        );

        // 7. check_unreachable_message
        assert_eq!(
            check_unreachable_message(dirty_base, dirty_reason),
            "Could not check the client ID with IAM at https://demo.accelbyte.io: connection refused."
        );
    }

    #[test]
    fn test_printed_values_are_capped() {
        let long_error = "e".repeat(1000);
        let result = client_rejected_warning_message("x", &long_error);
        let capped_error = format!("{}\u{2026}", "e".repeat(199));
        assert_eq!(
            result,
            format!("IAM rejected client \"x\": {capped_error}.")
        );
    }

    #[test]
    fn test_check_unreachable_tip_whole_text() {
        assert_eq!(
            super::CHECK_UNREACHABLE_TIP,
            "The login continues. If the browser shows an error, check the Base URL and the client ID."
        );
    }

    // ── Client check loop tests ──

    /// Scripted fake for [`super::ClientCheckIo`] that drives the loop with
    /// queued check results, queued prompt answers, and recorded warnings.
    struct FakeCheckIo {
        checks: std::collections::VecDeque<super::oauth::ClientCheck>,
        answers: std::collections::VecDeque<(String, bool)>,
        warnings: Vec<(String, Option<String>, Option<String>)>,
    }

    impl FakeCheckIo {
        /// Build a fake whose check results and prompt answers are returned in the given order.
        fn new(checks: Vec<super::oauth::ClientCheck>, answers: Vec<(String, bool)>) -> Self {
            Self {
                checks: checks.into(),
                answers: answers.into(),
                warnings: Vec::new(),
            }
        }
    }

    impl super::ClientCheckIo for FakeCheckIo {
        fn check(
            &mut self,
            _authorize_url: &str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = super::oauth::ClientCheck> + '_>>
        {
            let result = self
                .checks
                .pop_front()
                .expect("FakeCheckIo: no more check results in queue");
            Box::pin(std::future::ready(result))
        }
        fn ask_again(&mut self) -> Result<(String, bool), CliError> {
            let answer = self
                .answers
                .pop_front()
                .expect("FakeCheckIo: no more answers in queue");
            Ok(answer)
        }
        fn warn(&mut self, message: &str, reason: Option<&str>, tip: Option<&str>) {
            self.warnings.push((
                message.to_string(),
                reason.map(|s| s.to_string()),
                tip.map(|s| s.to_string()),
            ));
        }
    }

    #[test]
    fn test_check_loop_asks_again_then_accepts() {
        let mut io = FakeCheckIo::new(
            vec![
                super::oauth::ClientCheck::NotFound,
                super::oauth::ClientCheck::Rejected {
                    error: "invalid_scope".to_string(),
                    description: Some("Scope not allowed".to_string()),
                },
                super::oauth::ClientCheck::Accepted {
                    redirect_uri: Some("http://127.0.0.1:8080".to_string()),
                },
            ],
            vec![("second".to_string(), false), ("third".to_string(), false)],
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let result = rt.block_on(super::run_client_check_loop(
            &mut io,
            "https://demo.accelbyte.io",
            "first".to_string(),
            &ClientIdSource::Prompt,
            "challenge",
            "state",
            8080,
            false,
        ));
        let (client_id, _url, skip_port, _hint) = result.unwrap();
        assert_eq!(client_id, "third");
        assert!(
            skip_port,
            "matching redirect_uri should suppress the old port warning"
        );
        assert_eq!(io.answers.len(), 0, "both answers should be consumed");
        assert_eq!(io.warnings.len(), 2);
        assert_eq!(
            io.warnings[0],
            (
                "IAM has no client with ID \"first\".".to_string(),
                None,
                Some(
                    "A client ID looks like 2c7308eb4f3d48e3b892fa0794f14a9c. It is not the client name."
                        .to_string()
                ),
            )
        );
        assert_eq!(
            io.warnings[1],
            (
                "IAM rejected client \"second\": invalid_scope.".to_string(),
                Some("Scope not allowed".to_string()),
                Some("Check the client ID in the Admin Portal.".to_string()),
            )
        );
    }

    #[test]
    fn test_check_loop_stops_after_three_rejections() {
        let mut io = FakeCheckIo::new(
            vec![
                super::oauth::ClientCheck::NotFound,
                super::oauth::ClientCheck::NotFound,
                super::oauth::ClientCheck::NotFound,
            ],
            vec![("second".to_string(), false), ("third".to_string(), false)],
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let result = rt.block_on(super::run_client_check_loop(
            &mut io,
            "https://demo.accelbyte.io",
            "first".to_string(),
            &ClientIdSource::Prompt,
            "challenge",
            "state",
            8080,
            false,
        ));
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert_eq!(message, "No valid client ID after 3 attempts.");
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
        assert_eq!(io.answers.len(), 0, "should have asked again twice");
    }

    #[test]
    fn test_check_loop_configured_id_stops_at_once() {
        let mut io = FakeCheckIo::new(vec![super::oauth::ClientCheck::NotFound], vec![]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let result = rt.block_on(super::run_client_check_loop(
            &mut io,
            "https://demo.accelbyte.io",
            "scott-cli".to_string(),
            &ClientIdSource::Flag,
            "challenge",
            "state",
            8080,
            false,
        ));
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert_eq!(
                    message,
                    "IAM has no client with ID \"scott-cli\" (from --client-id)."
                );
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
        assert!(
            io.warnings.is_empty(),
            "configured ID should not print a warning"
        );
    }

    #[test]
    fn test_check_loop_caps_iam_description() {
        let long_desc = "d".repeat(1000);
        let expected_capped = format!("{}\u{2026}", "d".repeat(199));

        // With ClientIdSource::Flag the loop returns a Usage error whose
        // detail is the capped description.
        {
            let mut io = FakeCheckIo::new(
                vec![super::oauth::ClientCheck::Rejected {
                    error: "invalid_request".to_string(),
                    description: Some(long_desc.clone()),
                }],
                vec![],
            );
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let result = rt.block_on(super::run_client_check_loop(
                &mut io,
                "https://demo.accelbyte.io",
                "flagged-id".to_string(),
                &ClientIdSource::Flag,
                "challenge",
                "state",
                8080,
                false,
            ));
            match result {
                Err(CliError::Usage { metadata, .. }) => {
                    let meta = metadata.expect("metadata should be present");
                    assert_eq!(
                        meta.detail.as_deref(),
                        Some(expected_capped.as_str()),
                        "Flag path: detail must equal the capped description"
                    );
                }
                other => panic!("expected Usage error, got {other:?}"),
            }
        }

        // With ClientIdSource::Prompt and answers that end in acceptance,
        // the recorded warning's reason is the capped description.
        {
            let mut io = FakeCheckIo::new(
                vec![
                    super::oauth::ClientCheck::Rejected {
                        error: "invalid_request".to_string(),
                        description: Some(long_desc),
                    },
                    super::oauth::ClientCheck::Accepted {
                        redirect_uri: Some("http://127.0.0.1:8080".to_string()),
                    },
                ],
                vec![("good-id".to_string(), false)],
            );
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let result = rt.block_on(super::run_client_check_loop(
                &mut io,
                "https://demo.accelbyte.io",
                "first".to_string(),
                &ClientIdSource::Prompt,
                "challenge",
                "state",
                8080,
                false,
            ));
            assert!(result.is_ok(), "should succeed on second attempt");
            assert_eq!(io.warnings.len(), 1);
            assert_eq!(
                io.warnings[0].1.as_deref(),
                Some(expected_capped.as_str()),
                "Prompt path: warning reason must equal the capped description"
            );
        }
    }

    #[test]
    fn test_check_loop_keeps_create_guide_flag() {
        let mut io = FakeCheckIo::new(
            vec![
                super::oauth::ClientCheck::NotFound,
                super::oauth::ClientCheck::Accepted { redirect_uri: None },
            ],
            vec![("second".to_string(), true)],
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let result = rt.block_on(super::run_client_check_loop(
            &mut io,
            "https://demo.accelbyte.io",
            "first".to_string(),
            &ClientIdSource::Prompt,
            "challenge",
            "state",
            8080,
            false,
        ));
        let (_id, _url, _skip, hint_shown) = result.unwrap();
        assert!(
            hint_shown,
            "the create-guide flag from the re-prompt should be preserved"
        );
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
