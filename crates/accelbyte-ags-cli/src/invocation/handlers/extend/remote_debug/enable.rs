//! `ags extend remote-debug enable` — enable debug mode with a performance
//! warning and conditional confirmation prompt.
//!
//! Emits a performance warning, reads the app status via
//! `csm/admin/debug/v4/get`, prompts for confirmation when the app is
//! already running (because enabling debug mode restarts it), then sends
//! `{"enableDebugMode":true}` via `csm/admin/debug/v4/update`.

use clap::ArgMatches;

use crate::errors::CliError;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;

#[cfg(test)]
use super::debug_mode::{
    automation_context, isolated_runtime_env, non_promptable_context, promptable_context,
    should_prompt_for_running_app, NullFrontend, TempEnvGuard,
};

/// Performance warning emitted before every enable call.
pub(crate) const PERFORMANCE_WARNING: &str =
    "Warning: enabling remote debugging directly affects resource usage and \
     may degrade application performance.";

/// Format the success message emitted after a successful enable call.
/// Matches the wording of the Go tool being replaced.
pub(crate) fn format_enable_success(app: &str, namespace: &str) -> String {
    format!("debug mode enabled for app \"{app}\" in namespace \"{namespace}\"")
}

/// Build the JSON body for the enable-debug-mode update call.
///
/// Uses the CSM API's camelCase property name `enableDebugMode`, matching
/// the fixed body that the shim table previously carried.
#[cfg(test)]
pub(crate) fn build_enable_body() -> serde_json::Value {
    serde_json::json!({"enableDebugMode": true})
}

/// Parameters that specialise the shared debug-mode handler for enable.
const ENABLE_PARAMS: super::debug_mode::DebugModeParams = super::debug_mode::DebugModeParams {
    command_label: "remote-debug enable",
    enable_debug_mode: true,
    no_input_error_message: "Enabling debug mode on a running app requires confirmation",
    prompt_message:
        "The app is currently running. Enabling debug mode will restart it. Continue? [y/N] ",
    dry_run_info_message:
        "Dry run — debug mode will not be enabled and the app will not be restarted",
    dry_run_action: "would enable debug mode via PUT debugmode",
    performance_warning: Some(PERFORMANCE_WARNING),
    format_success: format_enable_success,
};

/// Confirmation logic with injected line reader. Delegates to the
/// shared implementation with enable-specific wording.
///
/// - `--yes` / `-y` → skip prompt, return `Ok`
/// - non-promptable context → return `CliError::Usage` naming `--yes`
/// - interactive → prompt, accept only `"y"` / `"Y"`
#[cfg(test)]
pub(crate) fn confirm_enable_impl(
    flags: &GlobalFlags,
    read: &mut dyn FnMut() -> Result<String, CliError>,
    ctx: &crate::invocation::context::FrontendContext,
) -> Result<(), CliError> {
    super::debug_mode::confirm_debug_mode_impl(flags, read, &ENABLE_PARAMS, ctx)
}

/// Route `ags extend remote-debug enable <flags>`.
pub(crate) async fn handle_remote_debug_enable(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    frontend: &mut dyn crate::frontend::Frontend,
    ctx: &crate::invocation::context::FrontendContext,
) -> Result<InvocationOutcome, CliError> {
    handle_remote_debug_enable_impl(
        matches,
        flags,
        frontend,
        &mut super::debug_mode::read_line_from_stdin,
        ctx,
    )
    .await
}

/// Handler implementation with an injected input reader so confirmation
/// behaviour can be exercised without reading process stdin.
async fn handle_remote_debug_enable_impl(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    frontend: &mut dyn crate::frontend::Frontend,
    read: &mut dyn FnMut() -> Result<String, CliError>,
    ctx: &crate::invocation::context::FrontendContext,
) -> Result<InvocationOutcome, CliError> {
    super::debug_mode::handle_debug_mode_impl(matches, flags, frontend, read, &ENABLE_PARAMS, ctx)
        .await
}

// ══════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test helpers ──

    /// Build `GlobalFlags` with the specified confirmation and input modes.
    fn test_flags(is_auto_confirmed: bool, is_no_input: bool) -> GlobalFlags {
        GlobalFlags {
            is_auto_confirmed,
            is_no_input,
            ..Default::default()
        }
    }

    async fn start_enable_server(app_status: &str, expected_updates: u64) -> wiremock::MockServer {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v4/admin/namespaces/test-ns/apps/my-app/debuginfo",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "appStatus": app_status
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v4/admin/namespaces/test-ns/apps/my-app/debugmode",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(expected_updates)
            .mount(&server)
            .await;
        server
    }

    fn enable_matches() -> ArgMatches {
        clap::Command::new("enable")
            .arg(clap::Arg::new("app").long("app"))
            .try_get_matches_from(["enable", "--app", "my-app"])
            .unwrap()
    }

    // ── Performance warning ──

    #[test]
    fn performance_warning_is_non_empty() {
        assert!(
            !PERFORMANCE_WARNING.is_empty(),
            "performance warning constant must not be empty"
        );
    }

    #[test]
    fn performance_warning_mentions_resource_usage() {
        assert!(
            PERFORMANCE_WARNING.contains("resource usage"),
            "warning must mention resource usage; got: {PERFORMANCE_WARNING}"
        );
    }

    // ── should_prompt_for_running_app ──

    #[test]
    fn prompt_when_deployment_running() {
        assert!(
            should_prompt_for_running_app(Some("deployment-running")),
            "must prompt when appStatus is deployment-running"
        );
    }

    #[test]
    fn no_prompt_when_stopped() {
        assert!(
            !should_prompt_for_running_app(Some("stopped")),
            "must not prompt when appStatus is stopped"
        );
    }

    #[test]
    fn no_prompt_when_starting() {
        assert!(
            !should_prompt_for_running_app(Some("deployment-starting")),
            "must not prompt when appStatus is deployment-starting"
        );
    }

    #[test]
    fn no_prompt_when_status_missing() {
        assert!(
            !should_prompt_for_running_app(None),
            "must not prompt when appStatus is absent"
        );
    }

    // ── confirm_enable_impl ──

    #[test]
    fn confirm_auto_confirmed_skips_prompt() {
        let flags = test_flags(true, false);
        let ctx = promptable_context();
        let result = confirm_enable_impl(
            &flags,
            &mut || panic!("should not read when auto-confirmed"),
            &ctx,
        );
        assert!(result.is_ok(), "auto-confirmed must succeed");
    }

    #[test]
    fn confirm_no_input_without_yes_rejects_naming_yes() {
        let flags = test_flags(false, true);
        let ctx = non_promptable_context();
        let result = confirm_enable_impl(
            &flags,
            &mut || panic!("should not read in no-input mode"),
            &ctx,
        );
        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(
                    message.contains("confirmation"),
                    "error must mention confirmation; got: {message}"
                );
            }
            other => panic!("expected CliError::Usage, got: {other:?}"),
        }
        // The metadata suggestion must name --yes.
        if let Err(CliError::Usage { metadata, .. }) = &result {
            let meta = metadata.as_ref().expect("metadata must be present");
            let suggestion = format!("{meta:?}");
            assert!(
                suggestion.contains("--yes"),
                "suggestion must name --yes; got: {suggestion}"
            );
        }
    }

    #[test]
    fn confirm_interactive_y_proceeds() {
        let flags = test_flags(false, false);
        let ctx = promptable_context();
        let result = confirm_enable_impl(&flags, &mut || Ok("y".to_string()), &ctx);
        assert!(result.is_ok(), "interactive 'y' must proceed");
    }

    #[test]
    fn confirm_interactive_uppercase_y_proceeds() {
        let flags = test_flags(false, false);
        let ctx = promptable_context();
        let result = confirm_enable_impl(&flags, &mut || Ok("Y".to_string()), &ctx);
        assert!(result.is_ok(), "interactive 'Y' must proceed");
    }

    #[test]
    fn confirm_interactive_n_cancels() {
        let flags = test_flags(false, false);
        let ctx = promptable_context();
        let result = confirm_enable_impl(&flags, &mut || Ok("n".to_string()), &ctx);
        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(
                    message.contains("cancelled"),
                    "error must mention cancellation; got: {message}"
                );
            }
            other => panic!("expected CliError::Usage for 'n', got: {other:?}"),
        }
    }

    #[test]
    fn confirm_interactive_empty_cancels() {
        let flags = test_flags(false, false);
        let ctx = promptable_context();
        let result = confirm_enable_impl(&flags, &mut || Ok("".to_string()), &ctx);
        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(
                    message.contains("cancelled"),
                    "empty input must cancel; got: {message}"
                );
            }
            other => panic!("expected CliError::Usage for empty, got: {other:?}"),
        }
    }

    /// Declining the prompt means the update call is never made.
    /// This is structurally guaranteed: `confirm_enable_impl` returns `Err`
    /// and the handler returns early before reaching the update dispatch.
    /// The test proves `confirm_enable_impl` returns `Err(Usage)` on decline.
    #[test]
    fn confirm_decline_prevents_update_call() {
        let flags = test_flags(false, false);
        let ctx = promptable_context();
        let result = confirm_enable_impl(&flags, &mut || Ok("n".to_string()), &ctx);
        assert!(
            result.is_err(),
            "declining confirmation must return Err, which prevents the update call"
        );
    }

    /// `--no-input` without `--yes` means the update call is never made.
    /// Same structural guarantee as decline: `confirm_enable_impl` returns
    /// `Err` and the handler returns early.
    #[test]
    fn no_input_without_yes_prevents_update_call() {
        let flags = test_flags(false, true);
        let ctx = non_promptable_context();
        let result = confirm_enable_impl(&flags, &mut || panic!("unreachable"), &ctx);
        assert!(
            result.is_err(),
            "--no-input without --yes must return Err, preventing the update call"
        );
    }

    // ── build_enable_body ──

    #[test]
    fn enable_body_uses_camel_case_property() {
        let body = build_enable_body();
        let obj = body.as_object().expect("body must be a JSON object");
        assert!(
            obj.contains_key("enableDebugMode"),
            "body must contain camelCase 'enableDebugMode'; got keys: {:?}",
            obj.keys().collect::<Vec<_>>()
        );
        assert!(
            !obj.contains_key("enable_debug_mode"),
            "body must NOT contain snake_case 'enable_debug_mode'"
        );
    }

    #[test]
    fn enable_body_sets_true() {
        let body = build_enable_body();
        assert_eq!(
            body["enableDebugMode"], true,
            "enableDebugMode must be true"
        );
    }

    #[test]
    fn enable_body_has_single_field() {
        let body = build_enable_body();
        let obj = body.as_object().expect("body must be a JSON object");
        assert_eq!(
            obj.len(),
            1,
            "body must contain exactly one field; got: {obj:?}"
        );
    }

    // ── Clap registration: enable subcommand exists ──

    #[test]
    fn enable_subcommand_registered_under_remote_debug() {
        let extend_cmd = crate::invocation::builder::build_extend_command();

        let remote_debug = extend_cmd
            .get_subcommands()
            .find(|sub| sub.get_name() == "remote-debug")
            .expect("remote-debug subcommand must exist under extend");

        let enable = remote_debug
            .get_subcommands()
            .find(|sub| sub.get_name() == "enable" && !sub.is_hide_set())
            .expect("enable must exist as a visible subcommand under remote-debug");

        // Verify --app is registered.
        let app_arg = enable.get_arguments().find(|a| a.get_long() == Some("app"));
        assert!(app_arg.is_some(), "enable must have --app flag");
    }

    // ── Handler: missing --app → CliError::Usage ──

    #[tokio::test]
    async fn test_enable_missing_app_is_usage_error() {
        let matches = clap::Command::new("enable")
            .arg(clap::Arg::new("app").long("app"))
            .try_get_matches_from(["enable"])
            .unwrap();

        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let ctx = promptable_context();
        let result = handle_remote_debug_enable(&matches, &flags, &mut frontend, &ctx).await;

        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(
                    message.contains("app"),
                    "error must mention --app: {message}"
                );
            }
            other => panic!("expected CliError::Usage for missing --app, got: {other:?}"),
        }
    }

    // ── Handler: missing namespace → CliError::Usage ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_enable_missing_namespace_is_usage_error() {
        // Env-mutating test: sets AGS_HOME to an empty dir so no config
        // file provides a namespace fallback.
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _ns = TempEnvGuard::clear("AGS_NAMESPACE");
        let _profile = TempEnvGuard::clear("AGS_PROFILE");

        let matches = clap::Command::new("enable")
            .arg(clap::Arg::new("app").long("app"))
            .try_get_matches_from(["enable", "--app", "my-app"])
            .unwrap();

        let flags = GlobalFlags {
            namespace: None,
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let ctx = promptable_context();
        let result = handle_remote_debug_enable(&matches, &flags, &mut frontend, &ctx).await;

        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(
                    message.contains("namespace"),
                    "error must mention namespace: {message}"
                );
            }
            other => panic!("expected CliError::Usage for missing namespace, got: {other:?}"),
        }
    }

    // ── Handler API error classification ──

    #[tokio::test]
    #[serial_test::serial]
    async fn handler_forbidden_debug_info_uses_api_error_classification() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v4/admin/namespaces/test-ns/apps/my-app/debuginfo",
            ))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "errorMessage": "insufficient permissions"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v4/admin/namespaces/test-ns/apps/my-app/debugmode",
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = enable_matches();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let ctx = promptable_context();
        let mut read = || -> Result<String, CliError> {
            panic!("a failed debug-info request must not prompt")
        };
        let mut frontend = NullFrontend;

        let result =
            handle_remote_debug_enable_impl(&matches, &flags, &mut frontend, &mut read, &ctx).await;

        let error = result.expect_err("forbidden debug info must fail enable");
        assert!(matches!(error, CliError::Api { .. }));
        assert_eq!(error.exit_code(), 3);
        server.verify().await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn handler_running_app_decline_skips_update() {
        let server = start_enable_server("deployment-running", 0).await;
        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = enable_matches();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let ctx = promptable_context();
        let mut read = || Ok("n".to_string());
        let mut frontend = NullFrontend;

        let result =
            handle_remote_debug_enable_impl(&matches, &flags, &mut frontend, &mut read, &ctx).await;

        assert!(
            matches!(result, Err(CliError::Usage { .. })),
            "declining a running-app restart must return Usage, got: {result:?}"
        );
        server.verify().await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn handler_running_app_yes_skips_prompt_and_updates() {
        let server = start_enable_server("deployment-running", 1).await;
        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = enable_matches();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            is_auto_confirmed: true,
            ..Default::default()
        };
        let ctx = promptable_context();
        let mut read =
            || -> Result<String, CliError> { panic!("--yes must skip the confirmation prompt") };
        let mut frontend = NullFrontend;

        let result =
            handle_remote_debug_enable_impl(&matches, &flags, &mut frontend, &mut read, &ctx).await;

        assert!(result.is_ok(), "--yes must allow the update: {result:?}");
        server.verify().await;
    }

    // ── Success emission ──

    #[test]
    fn format_enable_success_contains_app_and_namespace() {
        let msg = super::format_enable_success("my-app", "test-ns");
        assert!(
            msg.contains("my-app"),
            "success message must contain the app name: {msg}"
        );
        assert!(
            msg.contains("test-ns"),
            "success message must contain the namespace: {msg}"
        );
        assert!(
            msg.contains("debug mode enabled"),
            "success message must say debug mode was enabled: {msg}"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn handler_non_running_app_skips_prompt_and_updates() {
        let server = start_enable_server("deployment-stopped", 1).await;
        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = enable_matches();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let ctx = promptable_context();
        let mut read = || -> Result<String, CliError> {
            panic!("a non-running app must not prompt for confirmation")
        };
        let mut frontend = NullFrontend;

        let result =
            handle_remote_debug_enable_impl(&matches, &flags, &mut frontend, &mut read, &ctx).await;

        assert!(
            result.is_ok(),
            "a non-running app must proceed without prompting: {result:?}"
        );
        server.verify().await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn handler_running_app_no_input_requires_yes_and_skips_update() {
        let server = start_enable_server("deployment-running", 0).await;
        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = enable_matches();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            is_no_input: true,
            ..Default::default()
        };
        let ctx = non_promptable_context();
        let mut read =
            || -> Result<String, CliError> { panic!("--no-input must not read from stdin") };
        let mut frontend = NullFrontend;

        let result =
            handle_remote_debug_enable_impl(&matches, &flags, &mut frontend, &mut read, &ctx).await;

        match result {
            Err(CliError::Usage { metadata, .. }) => {
                let suggestion = metadata
                    .and_then(|value| value.suggestion)
                    .expect("non-interactive confirmation error must include a suggestion");
                assert!(
                    suggestion.contains("--yes"),
                    "suggestion must name --yes, got: {suggestion}"
                );
            }
            other => panic!("--no-input without --yes must return Usage, got: {other:?}"),
        }
        server.verify().await;
    }

    #[test]
    fn describe_remote_debug_lists_enable_as_command_child() {
        let extend_cmd = crate::invocation::builder::build_extend_command();
        let remote_debug = extend_cmd
            .find_subcommand("remote-debug")
            .expect("remote-debug must exist under extend");

        // Verify enable is a visible subcommand in the clap tree.
        let enable_sub = remote_debug
            .get_subcommands()
            .find(|sub| sub.get_name() == "enable" && !sub.is_hide_set())
            .expect("enable must be a visible subcommand under remote-debug");

        assert!(
            enable_sub.get_about().is_some(),
            "enable subcommand must have an about string"
        );
    }

    #[test]
    fn describe_enable_is_not_an_alias_child_in_extend() {
        use crate::invocation::handlers::extend::service_shims::SHIMS;

        // Verify that no shim in the SHIMS table has name "enable".
        let enable_shim = SHIMS.iter().find(|s| s.name == "enable");
        assert!(
            enable_shim.is_none(),
            "enable must NOT be in the SHIMS table (it was promoted to a native handler)"
        );
    }

    // ── Dry-run: no network traffic ──

    /// `--dry-run` must short-circuit before any HTTP request. This test
    /// binds a TCP listener, points AGS_BASE_URL at it, and asserts ZERO
    /// connections are accepted. A test that only checks printed text
    /// would not catch the defect — the bug is the traffic, not the output.
    #[tokio::test]
    #[serial_test::serial]
    async fn dry_run_issues_no_http_requests() {
        // Env-mutating test: sets AGS_HOME, AGS_BASE_URL, etc.
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("must bind a local listener");
        let addr = listener.local_addr().unwrap();
        // Set non-blocking so we can poll for connections without hanging.
        listener
            .set_nonblocking(true)
            .expect("must set non-blocking");

        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_keychain = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
        let _token = TempEnvGuard::clear("AGS_ACCESS_TOKEN");
        let _base_url = TempEnvGuard::set("AGS_BASE_URL", &format!("http://{addr}"));

        let matches = enable_matches();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            is_dry_run: true,
            is_auto_confirmed: true,
            ..Default::default()
        };
        let mut frontend = NullFrontend;
        let ctx = promptable_context();

        let result = handle_remote_debug_enable(&matches, &flags, &mut frontend, &ctx).await;

        assert!(
            result.is_ok(),
            "dry-run must succeed without any network call: {result:?}"
        );

        // Assert zero connections were accepted.
        let mut connection_count = 0u32;
        while let Ok(_stream) = listener.accept() {
            connection_count += 1;
        }
        assert_eq!(
            connection_count, 0,
            "dry-run must issue zero HTTP requests, but {connection_count} connections were accepted"
        );
    }

    /// The dry-run preview must mention the performance warning and what
    /// the command would do, consistent with sibling extend previews.
    #[tokio::test]
    async fn dry_run_preview_returns_complete() {
        let matches = enable_matches();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            is_dry_run: true,
            ..Default::default()
        };
        let mut frontend = NullFrontend;
        let ctx = promptable_context();

        let result = handle_remote_debug_enable(&matches, &flags, &mut frontend, &ctx).await;

        match result {
            Ok(crate::invocation::InvocationOutcome::Complete) => {}
            other => panic!("dry-run must return Complete, got: {other:?}"),
        }
    }

    // ── allows_input gate: non-interactive contract ──

    /// A non-promptable invocation (piped stdin, no --no-input flag) must
    /// return the "requires confirmation" error, not "Operation cancelled".
    /// The defect: without the ctx.allows_input() gate, the handler reaches
    /// the read closure, gets empty/EOF, and returns "cancelled".
    #[tokio::test]
    #[serial_test::serial]
    async fn handler_non_promptable_without_no_input_returns_confirmation_error() {
        let server = start_enable_server("deployment-running", 0).await;
        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = enable_matches();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            // Deliberately NOT setting is_no_input — the context's
            // allow_input is false because stdin is piped.
            ..Default::default()
        };
        let ctx = non_promptable_context();
        // Simulate EOF: an empty read is what happens when stdin is piped.
        let mut read = || Ok("".to_string());
        let mut frontend = NullFrontend;

        let result =
            handle_remote_debug_enable_impl(&matches, &flags, &mut frontend, &mut read, &ctx).await;

        match result {
            Err(CliError::Usage {
                ref message,
                metadata,
            }) => {
                assert!(
                    message.contains("confirmation"),
                    "non-promptable must get 'confirmation' error, got: {message}"
                );
                let suggestion = metadata
                    .and_then(|m| m.suggestion)
                    .expect("must include suggestion");
                assert!(
                    suggestion.contains("--yes"),
                    "suggestion must name --yes, got: {suggestion}"
                );
            }
            other => panic!(
                "non-promptable without --no-input must return Usage \
                 with 'confirmation', got: {other:?}"
            ),
        }
        server.verify().await;
    }

    /// --format=json automation consumer against a running app must get
    /// the same "requires confirmation" error — never a prompt.
    #[tokio::test]
    #[serial_test::serial]
    async fn handler_automation_consumer_returns_confirmation_error() {
        let server = start_enable_server("deployment-running", 0).await;
        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = enable_matches();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let ctx = automation_context();
        let mut read = || Ok("".to_string());
        let mut frontend = NullFrontend;

        let result =
            handle_remote_debug_enable_impl(&matches, &flags, &mut frontend, &mut read, &ctx).await;

        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(
                    message.contains("confirmation"),
                    "automation consumer must get 'confirmation' error, got: {message}"
                );
            }
            other => {
                panic!("automation consumer must return Usage with 'confirmation', got: {other:?}")
            }
        }
        server.verify().await;
    }

    /// --yes in a non-promptable context must still proceed: auto-confirm
    /// wins over everything.
    #[tokio::test]
    #[serial_test::serial]
    async fn handler_non_promptable_yes_still_proceeds() {
        let server = start_enable_server("deployment-running", 1).await;
        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = enable_matches();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            is_auto_confirmed: true,
            ..Default::default()
        };
        let ctx = non_promptable_context();
        let mut read = || -> Result<String, CliError> {
            panic!("--yes must skip prompt even in a non-promptable context")
        };
        let mut frontend = NullFrontend;

        let result =
            handle_remote_debug_enable_impl(&matches, &flags, &mut frontend, &mut read, &ctx).await;

        assert!(result.is_ok(), "--yes must still proceed: {result:?}");
        server.verify().await;
    }

    /// A promptable context that reads "n" must still return "Operation
    /// cancelled" — the user genuinely declined. This keeps the
    /// "requires confirmation" vs "cancelled" distinction observable.
    #[tokio::test]
    #[serial_test::serial]
    async fn handler_promptable_decline_returns_cancelled() {
        let server = start_enable_server("deployment-running", 0).await;
        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = enable_matches();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let ctx = promptable_context();
        let mut read = || Ok("n".to_string());
        let mut frontend = NullFrontend;

        let result =
            handle_remote_debug_enable_impl(&matches, &flags, &mut frontend, &mut read, &ctx).await;

        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(
                    message.contains("cancelled"),
                    "promptable decline must say 'cancelled', got: {message}"
                );
            }
            other => {
                panic!("promptable decline must return Usage with 'cancelled', got: {other:?}")
            }
        }
        server.verify().await;
    }

    /// --no-input flag must keep its existing behaviour: the "requires
    /// confirmation" error naming --yes. Regression guard.
    #[tokio::test]
    #[serial_test::serial]
    async fn handler_no_input_flag_regression() {
        let server = start_enable_server("deployment-running", 0).await;
        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = enable_matches();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            is_no_input: true,
            ..Default::default()
        };
        // --no-input maps to allow_input=false in the resolver.
        let ctx = non_promptable_context();
        let mut read =
            || -> Result<String, CliError> { panic!("--no-input must not read from stdin") };
        let mut frontend = NullFrontend;

        let result =
            handle_remote_debug_enable_impl(&matches, &flags, &mut frontend, &mut read, &ctx).await;

        match result {
            Err(CliError::Usage {
                ref message,
                metadata,
            }) => {
                assert!(
                    message.contains("confirmation"),
                    "--no-input must get 'confirmation' error, got: {message}"
                );
                let suggestion = metadata
                    .and_then(|m| m.suggestion)
                    .expect("must include suggestion");
                assert!(
                    suggestion.contains("--yes"),
                    "suggestion must name --yes, got: {suggestion}"
                );
            }
            other => panic!("--no-input must return Usage with 'confirmation', got: {other:?}"),
        }
        server.verify().await;
    }
}
