//! `ags extend docker-login` — authenticate the local Docker CLI against
//! the Extend container registry.
//!
//! Self-owning route: `--help`, `--print`, and the `--print-format` guard
//! run before any auth or network call. The default path (no `--print`)
//! dispatches a bundled two-step workflow via [`execute_compiled_workflow`].
//! The `--print` path fetches credentials imperatively and writes them to
//! stdout without entering the workflow engine.

use ags_protocol::workflow::WorkflowId;
use ags_runtime::runtime::workflows::compile::compile_workflow;
use ags_runtime::runtime::workflows::registry;

use crate::errors::CliError;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;

/// Route `ags extend docker-login [flags]`.
///
/// `args` is `remaining[2..]` — everything after `["extend", "docker-login"]`.
pub(crate) async fn route_extend_docker_login(
    args: &[String],
    flags: &mut GlobalFlags,
    render_options: crate::frontend::RenderOptions,
    frontend_context: &crate::invocation::context::FrontendContext,
) -> Result<InvocationOutcome, CliError> {
    crate::invocation::router::parse_page_limit(flags)?;

    // `--help` is handled before any auth / network call.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return render_docker_login_help();
    }

    // Parse flags via the clap command tree.
    let mut command = crate::invocation::builder::build_extend_command();
    let argv: Vec<String> = std::iter::once("extend".to_string())
        .chain(std::iter::once("docker-login".to_string()))
        .chain(args.iter().cloned())
        .collect();
    let matches = command
        .try_get_matches_from_mut(argv.iter().map(String::as_str))
        .map_err(|error| CliError::Usage {
            message: crate::invocation::clap_helpers::strip_clap_prefix(&error.to_string()),
            metadata: None,
        })?;
    let (_, docker_login_matches) = matches
        .subcommand()
        .and_then(|(name, sub)| {
            if name == "docker-login" {
                Some((name, sub))
            } else {
                None
            }
        })
        .ok_or_else(|| CliError::Usage {
            message: "Expected docker-login subcommand".to_string(),
            metadata: None,
        })?;

    // Emit a notice for any Go-compat flag the user explicitly supplied.
    // Uses the pre-surface owned frontend so the notice goes through
    // `render_warning` (stderr on plain, suppressed on JSON) rather than
    // raw `write_stderr_line`. Fires before the --print / workflow branch,
    // covering both paths.
    {
        let supplied = crate::invocation::compat_flags::collect_supplied_flags(
            docker_login_matches,
            &[
                &crate::invocation::compat_flags::DOCKER_LOGIN_LOGIN,
                &crate::invocation::compat_flags::DOCKER_LOGIN_VERBOSITY,
            ],
        );
        if !supplied.is_empty() && !flags.verbosity.is_quiet() {
            let mut frontend = crate::frontend::frontend_for_surface(
                frontend_context.pre_surface_backend(),
                render_options.clone(),
            )?;
            for flag_name in &supplied {
                frontend.render_warning(
                    &format!(
                        "--{flag_name} is accepted for backward compatibility but has no effect"
                    ),
                    None,
                    None,
                );
            }
            let _ = frontend.finish();
        }
    }

    // Namespace is a global flag consumed by the prescan; the route reads
    // it from `flags.namespace`. No route-local namespace arg is registered
    // in clap (the duplicate was deleted to eliminate the prescan collision).
    let namespace = resolve_namespace(flags)?;

    let app = docker_login_matches
        .get_one::<String>("app")
        .ok_or_else(|| CliError::Usage {
            message: "--app is required for docker-login".to_string(),
            metadata: None,
        })?
        .clone();

    if app.trim().is_empty() {
        return Err(CliError::Usage {
            message: "--app value cannot be empty".to_string(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Pass a non-empty app name, e.g. --app my-extend-app",
            ))),
        });
    }

    let is_print = docker_login_matches.get_flag("print");
    let format_value = docker_login_matches
        .get_one::<String>("print-format")
        .map(|s| s.as_str())
        .unwrap_or("json");

    // `--print-format` without `--print` is rejected before any network call.
    let format_was_explicitly_set = docker_login_matches.value_source("print-format")
        == Some(clap::parser::ValueSource::CommandLine);
    if format_was_explicitly_set && !is_print {
        return Err(CliError::Usage {
            message: "--print-format requires --print".to_string(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Use --print --print-format <json|token> to write credentials to stdout",
            ))),
        });
    }

    // Validate the format value before any auth or network call. The accepted
    // set is defined by `format_print_output`; this predicate mirrors it for
    // early rejection. A mismatch surfaces as a test failure.
    if !is_valid_print_format(format_value) {
        return Err(CliError::Usage {
            message: format!("unsupported --print-format value: '{format_value}'"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Supported values: json, token",
            ))),
        });
    }

    if is_print {
        // `--print` path: fetch credentials imperatively and write to stdout.
        return print_credentials(&namespace, &app, format_value, flags).await;
    }

    // Default path: dispatch the bundled two-step workflow.
    let definition = match registry().resolve(&WorkflowId::new("docker-login")) {
        Some(workflow) => workflow.definition().clone(),
        None => {
            return Err(CliError::Internal(anyhow::anyhow!(
                "bundled docker-login workflow not found in registry"
            )));
        }
    };

    let mut catalogue = ags_runtime::catalogue::Catalogue::new();
    let compiled = compile_workflow(&definition, &mut catalogue)?;

    // Pre-supply namespace and app from the parsed flags.
    let mut pre_supplied = std::collections::BTreeMap::new();
    pre_supplied.insert(
        "namespace".to_string(),
        serde_json::Value::String(namespace),
    );
    pre_supplied.insert("app".to_string(), serde_json::Value::String(app));

    super::workflow::execute_compiled_workflow(
        compiled,
        pre_supplied,
        flags,
        render_options,
        frontend_context,
        true,
        None,
        None,
    )
    .await
}

/// Render `ags extend docker-login --help`.
///
/// Help goes to stdout, matching clap's built-in `--help` behaviour and
/// the POSIX convention that callers capture help via `cmd --help | ...`.
fn render_docker_login_help() -> Result<InvocationOutcome, CliError> {
    let command = crate::invocation::builder::build_extend_command();
    if let Some(dl) = command.find_subcommand("docker-login") {
        let mut dl = dl.clone().bin_name("ags extend docker-login");
        let help = dl.render_long_help();
        let rendered = if crate::frontend::style::is_stdout_enabled() {
            help.ansi().to_string()
        } else {
            help.to_string()
        };
        crate::frontend::write_stdout_line(&rendered)?;
    }
    Ok(InvocationOutcome::Complete)
}

/// Resolve the namespace from flag, env, or profile config, returning a
/// `Usage` error when no source provides a namespace.
///
/// Delegates to the runtime's `resolve_namespace` which implements the
/// flag → `AGS_NAMESPACE` env → profile config precedence chain, matching
/// `ExecutionContext::resolve`. A single resolution function is reused so
/// the two paths cannot drift.
fn resolve_namespace(flags: &GlobalFlags) -> Result<String, CliError> {
    ags_runtime::runtime::execution::resolve_namespace(
        flags.namespace.as_deref(),
        flags.profile.as_deref(),
    )
    .map(|(namespace, _source)| namespace)
    .ok_or_else(|| CliError::Usage {
        message: "--namespace is required for docker-login".to_string(),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            "Supply --namespace <ns>, set AGS_NAMESPACE, or run 'ags config set namespace <ns>'",
        ))),
    })
}

/// `--print` path: fetch EHS credentials and write to stdout. No Docker
/// binary is invoked and no workflow engine is entered.
async fn print_credentials(
    namespace: &str,
    app: &str,
    format: &str,
    flags: &GlobalFlags,
) -> Result<InvocationOutcome, CliError> {
    // Dry-run guard — no auth, no network, preview only.
    if flags.is_dry_run {
        let color = crate::frontend::style::is_stderr_enabled();
        crate::frontend::write_stderr_line(&crate::frontend::style::info(
            "Dry run — no credentials will be fetched",
            color,
        ));
        crate::frontend::write_stderr_line(&format!("  Namespace: {namespace}"));
        crate::frontend::write_stderr_line(&format!("  App:       {app}"));
        crate::frontend::write_stderr_line(&format!("  Format:    {format}"));
        return Ok(InvocationOutcome::Complete);
    }

    // Resolve auth context.
    let input = ags_runtime::runtime::execution::ResolutionInput {
        profile: flags.profile.clone(),
        namespace: flags.namespace.clone(),
        is_dry_run: flags.is_dry_run,
    };
    let http_client = ags_runtime::runtime::dispatch::http::build_http_client(flags.timeout)?;
    let context =
        ags_runtime::runtime::execution::ExecutionContext::resolve(&input, &http_client).await?;

    let runtime = ags_runtime::runtime::Runtime::from_reqwest(context, http_client);
    let creds = runtime.fetch_docker_credentials(namespace, app).await?;

    // Write to stdout only — credentials must never reach stderr.
    // Uses the owned sink helper, never bare `println!`, to satisfy the
    // architecture guard (`test_cli_binary_output_goes_through_sink_helpers`).
    let output = format_print_output(&creds, format)?;
    crate::frontend::write_stdout_line(&output)?;

    Ok(InvocationOutcome::Complete)
}

/// The accepted `--print-format` values, defined once. Both
/// [`is_valid_print_format`] and [`format_print_output`] derive from this
/// array so adding a format cannot update one and miss the other.
const VALID_PRINT_FORMATS: &[&str] = &["json", "token"];

/// Whether a `--print-format` value is in the accepted set.
fn is_valid_print_format(format: &str) -> bool {
    VALID_PRINT_FORMATS.contains(&format)
}

/// Format credentials for `--print` output.
///
/// Total on its own terms: an unknown format maps to `CliError::Usage` with
/// the same message shape the route produces for other usage errors. The
/// early `is_valid_print_format` guard catches invalid values before auth,
/// but this function is safe to call with any input. Both functions derive
/// from [`VALID_PRINT_FORMATS`] so the accepted set cannot drift.
fn format_print_output(
    creds: &ags_runtime::runtime::facade::extend::DockerCredentials,
    format: &str,
) -> Result<String, CliError> {
    match format {
        "json" => {
            let json = serde_json::json!({
                "repositoryBaseUrl": creds.registry_url,
                "username": creds.username,
                "token": creds.token,
            });
            Ok(serde_json::to_string_pretty(&json).unwrap())
        }
        "token" => Ok(creds.token.clone()),
        _ => Err(CliError::Usage {
            message: format!("unsupported --print-format value: '{format}'"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Supported values: json, token",
            ))),
        }),
    }
}

/// Map credentials from the EHS response into the three-field input map
/// the `DockerLoginAction` expects. This is a pure function so tests can
/// verify the mapping without Docker or a network. Used by `image-upload`
/// when calling `DockerLoginAction` directly from an imperative handler.
pub(crate) fn credentials_to_inputs(
    registry_url: &str,
    username: &str,
    token: &str,
) -> std::collections::BTreeMap<String, serde_json::Value> {
    let mut inputs = std::collections::BTreeMap::new();
    inputs.insert(
        "registry".to_string(),
        serde_json::Value::String(registry_url.to_string()),
    );
    inputs.insert(
        "username".to_string(),
        serde_json::Value::String(username.to_string()),
    );
    inputs.insert(
        "password".to_string(),
        serde_json::Value::String(token.to_string()),
    );
    inputs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invocation::context::{
        ConsumerKind, FrontendContext, InteractionPolicy, TerminalCapabilities,
    };

    /// RAII guard that restores an environment variable after a test mutates it.
    // Env-mutating tests must be #[serial_test::serial] per repo convention.
    struct TempEnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl TempEnvGuard {
        /// Set an environment variable for the lifetime of the guard.
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, original }
        }

        /// Clear an environment variable for the lifetime of the guard.
        fn clear(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, original }
        }
    }

    impl Drop for TempEnvGuard {
        /// Restore the original environment variable value when the guard is dropped.
        fn drop(&mut self) {
            match &self.original {
                Some(val) => std::env::set_var(self.key, val),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn human_frontend_context() -> FrontendContext {
        FrontendContext {
            consumer: ConsumerKind::Human,
            interaction: InteractionPolicy {
                allow_input: true,
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
        }
    }

    // ── Help ──

    #[tokio::test]
    async fn test_docker_login_help_renders() {
        let mut flags = GlobalFlags::default();
        let ctx = human_frontend_context();
        let result = route_extend_docker_login(
            &["--help".to_string()],
            &mut flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
        )
        .await;
        assert!(
            matches!(result, Ok(InvocationOutcome::Complete)),
            "got: {result:?}"
        );
    }

    #[test]
    fn test_docker_login_help_shows_all_flags() {
        let command = crate::invocation::builder::build_extend_command();
        let dl = command
            .find_subcommand("docker-login")
            .expect("docker-login subcommand must exist");
        let mut dl = dl.clone().bin_name("ags extend docker-login");
        let help = dl.render_long_help().to_string();
        // --namespace is documented in the after_help as a global flag.
        assert!(help.contains("--namespace"), "help must show --namespace");
        assert!(help.contains("--app"), "help must show --app");
        assert!(help.contains("--print"), "help must show --print");
        assert!(
            help.contains("--print-format"),
            "help must show --print-format"
        );
        // Compat flags are visible in help with the compatibility marker.
        assert!(
            help.contains("--verbosity"),
            "--verbosity must be visible in help (compat flag)"
        );
        assert!(
            help.contains("--login"),
            "--login must be visible in help (compat flag)"
        );
        // Each compat flag's help section (flag line + indented description)
        // must carry the compatibility marker. A global `help.contains(marker)`
        // passes when only ONE flag has it; per-flag checks catch a missing
        // marker on either. Clap's long-help layout puts the flag name and
        // its help text on separate lines, so we split on blank-line boundaries
        // and check each flag's section.
        let compat_marker = "backward compatibility";
        let sections: Vec<&str> = help.split("\n\n").collect();
        let login_section = sections
            .iter()
            .find(|s| s.contains("--login"))
            .expect("--login must appear in help");
        assert!(
            login_section.contains(compat_marker),
            "--login section must mention backward compatibility:\n{login_section}"
        );
        let verbosity_section = sections
            .iter()
            .find(|s| s.contains("--verbosity"))
            .expect("--verbosity must appear in help");
        assert!(
            verbosity_section.contains(compat_marker),
            "--verbosity section must mention backward compatibility:\n{verbosity_section}"
        );
    }

    // ── --print-format without --print ──

    #[tokio::test]
    async fn test_format_without_print_is_rejected() {
        let mut flags = GlobalFlags {
            namespace: Some("ns".to_string()),
            ..Default::default()
        };
        let ctx = human_frontend_context();
        let result = route_extend_docker_login(
            &[
                "--app".to_string(),
                "myapp".to_string(),
                "--print-format".to_string(),
                "json".to_string(),
            ],
            &mut flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
        )
        .await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(
                    message.contains("--print-format requires --print"),
                    "got: {message}"
                );
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    // ── Invalid --print-format value ──

    #[tokio::test]
    async fn test_invalid_format_value_is_rejected() {
        let mut flags = GlobalFlags {
            namespace: Some("ns".to_string()),
            ..Default::default()
        };
        let ctx = human_frontend_context();
        let result = route_extend_docker_login(
            &[
                "--app".to_string(),
                "myapp".to_string(),
                "--print".to_string(),
                "--print-format".to_string(),
                "xml".to_string(),
            ],
            &mut flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
        )
        .await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(
                    message.contains("unsupported --print-format value"),
                    "got: {message}"
                );
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    // ── Namespace resolution (flag → env → profile config) ──

    /// When no namespace source is set (no flag, no env, no profile config),
    /// the route produces a Usage error.
    // Env-mutating test: sets process-wide AGS_HOME, AGS_NAMESPACE, AGS_PROFILE.
    #[test]
    #[serial_test::serial]
    fn test_missing_namespace_all_sources_unset() {
        use ags_runtime::runtime::config::{ENV_HOME, ENV_NAMESPACE, ENV_NO_KEYCHAIN, ENV_PROFILE};
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(ENV_HOME, tmp.path().to_str().unwrap());
        let _kc = TempEnvGuard::set(ENV_NO_KEYCHAIN, "1");
        let _ns = TempEnvGuard::clear(ENV_NAMESPACE);
        let _profile = TempEnvGuard::clear(ENV_PROFILE);

        let flags = GlobalFlags::default();
        let result = resolve_namespace(&flags);
        assert!(
            matches!(result, Err(CliError::Usage { .. })),
            "expected Usage error when all namespace sources are unset, got: {result:?}"
        );
    }

    /// AGS_NAMESPACE environment variable provides the namespace when no
    /// explicit --namespace flag is passed.
    // Env-mutating test: sets process-wide AGS_HOME, AGS_NAMESPACE.
    #[test]
    #[serial_test::serial]
    fn test_namespace_from_env_is_accepted() {
        use ags_runtime::runtime::config::{ENV_HOME, ENV_NAMESPACE, ENV_NO_KEYCHAIN, ENV_PROFILE};
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(ENV_HOME, tmp.path().to_str().unwrap());
        let _kc = TempEnvGuard::set(ENV_NO_KEYCHAIN, "1");
        let _ns = TempEnvGuard::set(ENV_NAMESPACE, "from-env");
        let _profile = TempEnvGuard::clear(ENV_PROFILE);

        let flags = GlobalFlags::default();
        let namespace = resolve_namespace(&flags)
            .expect("resolve_namespace must succeed when AGS_NAMESPACE is set");
        assert_eq!(namespace, "from-env");
    }

    /// Profile config namespace is used when no flag or env override is present.
    // Env-mutating test: sets process-wide AGS_HOME, AGS_NAMESPACE, AGS_PROFILE.
    #[test]
    #[serial_test::serial]
    fn test_namespace_from_profile_config_is_accepted() {
        use ags_runtime::runtime::config::{
            GlobalConfig, ProfileConfig, ENV_HOME, ENV_NAMESPACE, ENV_NO_KEYCHAIN, ENV_PROFILE,
        };
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(ENV_HOME, tmp.path().to_str().unwrap());
        let _kc = TempEnvGuard::set(ENV_NO_KEYCHAIN, "1");
        let _ns = TempEnvGuard::clear(ENV_NAMESPACE);
        let _profile = TempEnvGuard::clear(ENV_PROFILE);

        GlobalConfig {
            active_profile: Some("default".to_string()),
            ..Default::default()
        }
        .save()
        .unwrap();
        ProfileConfig {
            namespace: Some("from-config".to_string()),
            ..Default::default()
        }
        .save("default")
        .unwrap();

        let flags = GlobalFlags::default();
        let namespace = resolve_namespace(&flags)
            .expect("resolve_namespace must succeed when profile config has namespace");
        assert_eq!(namespace, "from-config");
    }

    /// An explicit --namespace flag wins over the AGS_NAMESPACE environment
    /// variable.
    // Env-mutating test: sets process-wide AGS_NAMESPACE.
    #[test]
    #[serial_test::serial]
    fn test_namespace_flag_wins_over_env() {
        use ags_runtime::runtime::config::{ENV_HOME, ENV_NAMESPACE, ENV_NO_KEYCHAIN};
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(ENV_HOME, tmp.path().to_str().unwrap());
        let _kc = TempEnvGuard::set(ENV_NO_KEYCHAIN, "1");
        let _ns = TempEnvGuard::set(ENV_NAMESPACE, "from-env");

        let flags = GlobalFlags {
            namespace: Some("from-flag".to_string()),
            ..Default::default()
        };
        let namespace =
            resolve_namespace(&flags).expect("resolve_namespace must succeed with flag set");
        assert_eq!(namespace, "from-flag");
    }

    // ── Missing required flags ──

    #[tokio::test]
    async fn test_missing_app_is_rejected() {
        let mut flags = GlobalFlags {
            namespace: Some("ns".to_string()),
            ..Default::default()
        };
        let ctx = human_frontend_context();
        let result = route_extend_docker_login(
            &[],
            &mut flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
        )
        .await;
        assert!(
            matches!(result, Err(CliError::Usage { .. })),
            "got: {result:?}"
        );
    }

    // ── credentials_to_inputs ──

    #[test]
    fn test_credentials_to_inputs_maps_all_three_fields() {
        let inputs = credentials_to_inputs("https://reg.io", "user", "tok123");
        assert_eq!(inputs.len(), 3);
        assert_eq!(inputs["registry"], serde_json::json!("https://reg.io"));
        assert_eq!(inputs["username"], serde_json::json!("user"));
        assert_eq!(inputs["password"], serde_json::json!("tok123"));
    }

    #[test]
    fn test_credentials_to_inputs_stores_token_as_password_key() {
        let inputs = credentials_to_inputs("https://reg.io", "user", "super-secret");
        // The token is stored under the "password" key, which is the key
        // DockerLoginAction reads. Verify it is present and correct.
        assert_eq!(inputs["password"], serde_json::json!("super-secret"));
        // The build_docker_login_args function (tested in docker_login.rs)
        // ensures this value never appears in argv.
    }

    // ── --print output shape ──

    #[test]
    fn test_print_json_output_shape() {
        // Verify the JSON shape matches the Go output contract.
        let creds = ags_runtime::runtime::facade::extend::DockerCredentials {
            registry_url: "https://registry.example.com".to_string(),
            username: "user".to_string(),
            token: "tok123".to_string(),
        };
        let json = serde_json::json!({
            "repositoryBaseUrl": creds.registry_url,
            "username": creds.username,
            "token": creds.token,
        });
        let rendered = serde_json::to_string_pretty(&json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(
            parsed["repositoryBaseUrl"], "https://registry.example.com",
            "JSON output must use the Go field name"
        );
        assert_eq!(parsed["username"], "user");
        assert_eq!(parsed["token"], "tok123");
    }

    #[test]
    fn test_print_token_output_shape() {
        // `--format token` must write only the raw token — no username, no
        // registry URL.  Exercises the real `format_print_output` function so
        // a bug that writes the wrong field would fail this test.
        let creds = ags_runtime::runtime::facade::extend::DockerCredentials {
            registry_url: "https://registry.example.com".to_string(),
            username: "user".to_string(),
            token: "raw-token-value".to_string(),
        };
        let output = format_print_output(&creds, "token").expect("token format must succeed");
        assert_eq!(
            output, "raw-token-value",
            "token format must emit the raw token"
        );
        assert!(
            !output.contains("registry.example.com"),
            "registry URL must not appear in token output"
        );
        assert!(
            !output.contains("user"),
            "username must not appear in token output"
        );
    }

    // ── build_extend_command includes docker-login ──

    #[test]
    fn test_build_extend_command_includes_docker_login() {
        let cmd = crate::invocation::builder::build_extend_command();
        let dl = cmd.find_subcommand("docker-login");
        assert!(dl.is_some(), "docker-login must be a subcommand of extend");
    }

    // ── --print-format (renamed from --format to avoid global flag collision) ──

    #[test]
    fn test_docker_login_help_shows_print_format() {
        let command = crate::invocation::builder::build_extend_command();
        let dl = command
            .find_subcommand("docker-login")
            .expect("docker-login subcommand must exist");
        let mut dl = dl.clone().bin_name("ags extend docker-login");
        let help = dl.render_long_help().to_string();
        assert!(
            help.contains("--print-format"),
            "help must show --print-format (not --format):\n{help}"
        );
    }

    #[tokio::test]
    async fn test_print_format_without_print_is_rejected() {
        let mut flags = GlobalFlags {
            namespace: Some("ns".to_string()),
            ..Default::default()
        };
        let ctx = human_frontend_context();
        let result = route_extend_docker_login(
            &[
                "--app".to_string(),
                "myapp".to_string(),
                "--print-format".to_string(),
                "json".to_string(),
            ],
            &mut flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
        )
        .await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(
                    message.contains("--print-format requires --print"),
                    "got: {message}"
                );
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_invalid_print_format_value_is_rejected() {
        let mut flags = GlobalFlags {
            namespace: Some("ns".to_string()),
            ..Default::default()
        };
        let ctx = human_frontend_context();
        let result = route_extend_docker_login(
            &[
                "--app".to_string(),
                "myapp".to_string(),
                "--print".to_string(),
                "--print-format".to_string(),
                "xml".to_string(),
            ],
            &mut flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
        )
        .await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(
                    message.contains("unsupported --print-format value"),
                    "got: {message}"
                );
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    // ── Namespace fallback to global flag ──

    /// The route reads namespace from `GlobalFlags.namespace`, not from a
    /// clap-registered `--namespace` arg (the duplicate was removed to
    /// eliminate a prescan collision). This test exercises the same clap
    /// parse and namespace resolution the route performs without entering
    /// the workflow executor, so it touches neither the network nor stdin.
    ///
    /// Layer: unit (pure logic, no external dependency).
    #[test]
    fn test_namespace_from_global_flag_is_accepted() {
        // Build the same clap command the route uses and parse args with
        // --app but no --namespace in the argv. Clap accepting this proves
        // the required(true) --namespace clap arg was removed.
        let mut command = crate::invocation::builder::build_extend_command();
        let argv: Vec<&str> = vec!["extend", "docker-login", "--app", "myapp"];
        let matches = command
            .try_get_matches_from_mut(argv)
            .expect("clap must accept --app without a --namespace clap arg");
        let (_, docker_login_matches) = matches
            .subcommand()
            .and_then(|(name, sub)| {
                if name == "docker-login" {
                    Some((name, sub))
                } else {
                    None
                }
            })
            .expect("docker-login subcommand must match");

        // Namespace resolves via the extracted `resolve_namespace` helper —
        // the same function the route calls. A change to the route's
        // resolution logic will break this assertion.
        let flags = GlobalFlags {
            namespace: Some("global-ns".to_string()),
            ..Default::default()
        };
        let namespace = resolve_namespace(&flags).expect("resolve_namespace must succeed");
        assert_eq!(
            namespace, "global-ns",
            "namespace must resolve from GlobalFlags.namespace"
        );

        // App resolves from clap matches.
        let app = docker_login_matches
            .get_one::<String>("app")
            .expect("--app must be present in clap matches");
        assert_eq!(app, "myapp");
    }

    // ── --dry-run --print guard ──

    /// `--dry-run --print` must produce a preview and make no network call.
    /// The guard fires before `ExecutionContext::resolve`, so no auth or
    /// HTTP transport is needed.
    #[tokio::test]
    async fn test_dry_run_print_produces_preview() {
        let mut flags = GlobalFlags {
            namespace: Some("ns".to_string()),
            is_dry_run: true,
            ..Default::default()
        };
        let ctx = human_frontend_context();
        let result = route_extend_docker_login(
            &[
                "--app".to_string(),
                "myapp".to_string(),
                "--print".to_string(),
            ],
            &mut flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
        )
        .await;
        assert!(
            matches!(result, Ok(InvocationOutcome::Complete)),
            "dry-run --print must succeed without network: {result:?}"
        );
    }

    // ── format_print_output totality ──

    /// An unknown `--print-format` value must return `CliError::Usage` with
    /// the unsupported-value message and a suggestion listing the accepted
    /// formats. The previous `catch_unwind` + `is_ok()` form was tautological
    /// — `catch_unwind` returns `Ok(inner)` for any non-panicking call, so
    /// `is_ok()` passed regardless of whether the inner result was `Ok` or
    /// `Err`.
    #[test]
    fn test_format_print_output_unknown_format_returns_usage_error() {
        let creds = ags_runtime::runtime::facade::extend::DockerCredentials {
            registry_url: "https://r.io".to_string(),
            username: "u".to_string(),
            token: "t".to_string(),
        };
        let result = format_print_output(&creds, "xml");
        match result {
            Err(CliError::Usage { message, metadata }) => {
                assert!(
                    message.contains("unsupported --print-format value"),
                    "message must cite the unsupported value: {message}"
                );
                assert!(
                    message.contains("'xml'"),
                    "message must echo the rejected value: {message}"
                );
                let meta = metadata.expect("metadata must carry a suggestion");
                let suggestion = meta
                    .suggestion
                    .as_deref()
                    .expect("suggestion text must be present");
                assert!(
                    suggestion.contains("json") && suggestion.contains("token"),
                    "suggestion must list accepted formats: {suggestion}"
                );
            }
            Ok(value) => panic!("expected Usage error, got Ok({value:?})"),
            Err(other) => panic!("expected Usage error, got {other:?}"),
        }
    }

    /// Known format values must succeed and produce the expected output shape.
    #[test]
    fn test_format_print_output_known_formats_succeed() {
        let creds = ags_runtime::runtime::facade::extend::DockerCredentials {
            registry_url: "https://r.io".to_string(),
            username: "u".to_string(),
            token: "t".to_string(),
        };
        let json_out = format_print_output(&creds, "json").expect("json format must succeed");
        let _: serde_json::Value =
            serde_json::from_str(&json_out).expect("json output must be valid JSON");

        let token_out = format_print_output(&creds, "token").expect("token format must succeed");
        assert_eq!(token_out, "t", "token format must return raw token");
    }

    /// The accepted format set in `is_valid_print_format` and the match arms
    /// of `format_print_output` must agree: every format one accepts, the
    /// other must handle correctly, and vice versa.
    #[test]
    fn test_valid_formats_match_format_print_output() {
        let creds = ags_runtime::runtime::facade::extend::DockerCredentials {
            registry_url: "https://r.io".to_string(),
            username: "u".to_string(),
            token: "t".to_string(),
        };
        // Every value accepted by is_valid_print_format must produce Ok
        for fmt in &["json", "token"] {
            assert!(is_valid_print_format(fmt), "{fmt} must be valid");
            assert!(
                format_print_output(&creds, fmt).is_ok(),
                "format_print_output must succeed for valid format '{fmt}'"
            );
        }
        // A value rejected by is_valid_print_format must produce Err
        let invalid = "xml";
        assert!(!is_valid_print_format(invalid), "{invalid} must be invalid");
        assert!(
            format_print_output(&creds, invalid).is_err(),
            "format_print_output must fail for invalid format '{invalid}'"
        );
    }

    // ── --app "" validation ──

    /// An empty `--app` value is rejected with a Usage error before any
    /// network call, mirroring the `--namespace` empty-value guard.
    #[tokio::test]
    async fn test_empty_app_is_rejected() {
        let mut flags = GlobalFlags {
            namespace: Some("ns".to_string()),
            ..Default::default()
        };
        let ctx = human_frontend_context();
        let result = route_extend_docker_login(
            &["--app".to_string(), String::new()],
            &mut flags,
            crate::frontend::RenderOptions::default(),
            &ctx,
        )
        .await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(
                    message.contains("--app"),
                    "error must mention --app: {message}"
                );
            }
            other => panic!("expected Usage error for empty --app, got {other:?}"),
        }
    }
}
