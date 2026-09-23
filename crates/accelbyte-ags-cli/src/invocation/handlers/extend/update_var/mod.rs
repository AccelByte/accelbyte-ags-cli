//! Handler for `ags extend update-var`.
//!
//! Upserts a CSM app configuration variable: lists existing variables,
//! scans for a matching `--key`, then updates it if found or creates it
//! under `--force` if not. See `docs/reference/cli-reference.md` for the
//! command reference.

mod api;
mod merge;

use clap::ArgMatches;

use crate::errors::CliError;
use crate::frontend::{write_stderr_line, Frontend};
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use ags_protocol::output::{CommandOutput, UpdateVarOutput};

/// Delegate to the crate-level shared stdin reader in `errors.rs`.
fn read_stdin_line() -> Result<String, CliError> {
    crate::errors::read_stdin_line()
}

/// Resolve the variable value from `--value` or `--value-stdin`.
///
/// Returns `(value, from_flag)`: when `from_flag` is `true`, the value came
/// from the `--value` flag and the caller should emit a warning about shell
/// history visibility.
///
/// The `stdin_reader` parameter is an injectable seam so tests can verify
/// the stdin path without a real pipe. Production passes
/// [`read_stdin_line`].
fn resolve_var_value(
    matches: &ArgMatches,
    stdin_reader: impl FnOnce() -> Result<String, CliError>,
) -> Result<(String, bool), CliError> {
    if let Some(value) = matches.get_one::<String>("value") {
        return Ok((value.clone(), true));
    }
    if matches.get_flag("value-stdin") {
        let value = stdin_reader()?;
        return Ok((value, false));
    }
    // Clap's `required_unless_present` rejects this at parse time, but
    // defence in depth.
    Err(CliError::Usage {
        message: "either --value or --value-stdin is required".to_string(),
        metadata: None,
    })
}

/// Execute `ags extend update-var`.
pub(crate) async fn handle_update_var(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    frontend: &mut dyn Frontend,
) -> Result<InvocationOutcome, CliError> {
    handle_update_var_with_reader(matches, flags, frontend, read_stdin_line).await
}

/// Inner handler that accepts an injectable stdin reader. Production passes
/// [`read_stdin_line`]; tests pass a closure that panics or returns a fixed
/// value to verify ordering and wiring without touching real stdin.
async fn handle_update_var_with_reader(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    frontend: &mut dyn Frontend,
    stdin_reader: impl FnOnce() -> Result<String, CliError>,
) -> Result<InvocationOutcome, CliError> {
    let key = matches
        .get_one::<String>("key")
        .ok_or_else(|| CliError::Usage {
            message: "--key is required".to_string(),
            metadata: None,
        })?
        .clone();
    let app = matches
        .get_one::<String>("app")
        .ok_or_else(|| CliError::Usage {
            message: "--app is required".to_string(),
            metadata: None,
        })?
        .clone();
    let force = matches.get_flag("force");

    let namespace = flags.namespace.clone().ok_or_else(|| CliError::Usage {
        message: "--namespace is required for extend update-var".to_string(),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            "Supply --namespace <ns> or set a default via 'ags config set namespace <ns>'",
        ))),
    })?;

    // Dry-run must short-circuit BEFORE value resolution. The preview
    // never uses the value, so reading stdin here would block
    // indefinitely in automation.
    if flags.is_dry_run {
        return dry_run_preview(&namespace, &app, &key, force);
    }

    let (value, value_from_flag) = resolve_var_value(matches, stdin_reader)?;
    if value_from_flag {
        frontend.render_warning(
            super::csm_error::VALUE_FLAG_SHELL_HISTORY_WARNING,
            None,
            None,
        );
    }
    let description_override = matches.get_one::<String>("description").cloned();
    let sensitive_override: Option<bool> =
        if matches.value_source("sensitive") == Some(clap::parser::ValueSource::CommandLine) {
            matches.get_one::<bool>("sensitive").copied()
        } else {
            None
        };

    let input = ags_runtime::runtime::execution::ResolutionInput {
        profile: flags.profile.clone(),
        namespace: Some(namespace.clone()),
        is_dry_run: false,
    };
    let http_client = ags_runtime::runtime::dispatch::http::build_http_client(flags.timeout)?;
    let context =
        ags_runtime::runtime::execution::ExecutionContext::resolve(&input, &http_client).await?;
    let resolved_namespace = context.namespace.clone().unwrap_or(namespace);

    let existing = api::list_variables(
        &http_client,
        &context.base_url,
        &context.access_token,
        &resolved_namespace,
        &app,
        &key,
    )
    .await?
    .into_iter()
    .find(|record| record.config_name == key);

    let (record, created, effective_for_create) = match existing {
        Some(existing) => {
            let effective = merge::compute_effective_fields(
                &existing,
                sensitive_override,
                description_override,
            );
            let record = api::update_variable(
                &http_client,
                &context.base_url,
                &context.access_token,
                &resolved_namespace,
                &app,
                &existing.config_id,
                &value,
                effective.apply_mask,
                effective.description.as_deref(),
            )
            .await
            .map_err(|e| super::csm_error::redact_submitted_value_in_error(e, &value))?;
            (record, false, None)
        }
        None => {
            if !force {
                return Err(CliError::Api {
                    message: format!(
                        "variable '{key}' does not exist, use flag '--force' to create it automatically"
                    ),
                    metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                        "Pass --force to create the variable automatically",
                    ))),
                    category: crate::errors::ApiErrorCategory::Upstream,
                });
            }
            let effective = merge::compute_new_fields(sensitive_override, description_override);
            let record = api::create_variable(
                &http_client,
                &context.base_url,
                &context.access_token,
                &resolved_namespace,
                &app,
                &key,
                &value,
                effective.apply_mask,
                effective.description.as_deref(),
            )
            .await
            .map_err(|e| super::csm_error::redact_submitted_value_in_error(e, &value))?;
            (record, true, Some(effective))
        }
    };

    // `SaveVariableV5`'s response only echoes `configId`/`configName` — it
    // never returns `applyMask`/`description`, so on the create path those
    // two fields must come from what was actually sent, not from the
    // (always-default) values `VariableRecord`'s `#[serde(default)]`
    // fields end up with when deserializing that response.
    let (apply_mask, description) = match effective_for_create {
        Some(effective) => (effective.apply_mask, effective.description),
        None => (record.apply_mask, record.description),
    };

    frontend.render(&CommandOutput::UpdateVar(UpdateVarOutput {
        config_id: record.config_id,
        config_name: record.config_name,
        apply_mask,
        description,
        created,
    }))?;

    Ok(InvocationOutcome::Complete)
}

/// Dry-run preview: no auth, no network. Prints what would happen and exits.
fn dry_run_preview(
    namespace: &str,
    app: &str,
    key: &str,
    force: bool,
) -> Result<InvocationOutcome, CliError> {
    let color = crate::frontend::style::is_stderr_enabled();
    write_stderr_line(&crate::frontend::style::info(
        "Dry run — no variable will be created or updated",
        color,
    ));
    write_stderr_line(&format!("  Namespace: {namespace}"));
    write_stderr_line(&format!("  App:       {app}"));
    write_stderr_line(&format!("  Key:       {key}"));
    write_stderr_line(&format!(
        "  Action:    would update if it exists{}",
        if force {
            ", or create it if not"
        } else {
            " (pass --force to create it if not)"
        }
    ));
    Ok(InvocationOutcome::Complete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── Null frontend ──

    struct NullFrontend;

    impl crate::frontend::Frontend for NullFrontend {
        fn render(&mut self, _output: &CommandOutput) -> Result<(), CliError> {
            Ok(())
        }
        fn render_error(&mut self, _err: &CliError) {}
        fn render_warning(&mut self, _msg: &str, _reason: Option<&str>, _tip: Option<&str>) {}
        fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {}
        fn finish(self: Box<Self>) -> Result<(), CliError> {
            Ok(())
        }
    }

    // ── Capturing frontend ──

    /// Captures the last rendered `UpdateVarOutput` and any warnings so
    /// tests can assert on what was actually reported.
    #[derive(Default)]
    struct CapturingFrontend {
        last_update_var: Option<UpdateVarOutput>,
        warnings: Vec<String>,
    }

    impl crate::frontend::Frontend for CapturingFrontend {
        fn render(&mut self, output: &CommandOutput) -> Result<(), CliError> {
            if let CommandOutput::UpdateVar(update_var) = output {
                self.last_update_var = Some(update_var.clone());
            }
            Ok(())
        }
        fn render_error(&mut self, _err: &CliError) {}
        fn render_warning(&mut self, msg: &str, _reason: Option<&str>, _tip: Option<&str>) {
            self.warnings.push(msg.to_string());
        }
        fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {}
        fn finish(self: Box<Self>) -> Result<(), CliError> {
            Ok(())
        }
    }

    // ── RAII env guard ──

    struct TempEnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl TempEnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            use std::env;
            let original = env::var(key).ok();
            env::set_var(key, value);
            Self { key, original }
        }
    }

    impl Drop for TempEnvGuard {
        fn drop(&mut self) {
            use std::env;
            match &self.original {
                Some(val) => env::set_var(self.key, val),
                None => env::remove_var(self.key),
            }
        }
    }

    fn isolated_runtime_env(
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

    fn update_var_matches(args: &[&str]) -> ArgMatches {
        clap::Command::new("update-var")
            .arg(clap::Arg::new("app").long("app").required(true))
            .arg(clap::Arg::new("key").long("key").required(true))
            .arg(
                clap::Arg::new("value")
                    .long("value")
                    .required_unless_present("value-stdin")
                    .conflicts_with("value-stdin"),
            )
            .arg(
                clap::Arg::new("value-stdin")
                    .long("value-stdin")
                    .action(clap::ArgAction::SetTrue)
                    .conflicts_with("value"),
            )
            .arg(clap::Arg::new("description").long("description"))
            .arg(
                clap::Arg::new("sensitive")
                    .long("sensitive")
                    .value_parser(clap::value_parser!(bool))
                    .num_args(0..=1)
                    .default_missing_value("true"),
            )
            .arg(
                clap::Arg::new("force")
                    .long("force")
                    .action(clap::ArgAction::SetTrue),
            )
            .try_get_matches_from(args)
            .unwrap()
    }

    /// Parse `args` through the REAL command tree (`build_extend_command()`).
    fn real_update_var_matches(args: &[&str]) -> ArgMatches {
        let mut command = crate::invocation::builder::build_extend_command();
        let argv: Vec<&str> = std::iter::once("extend")
            .chain(std::iter::once("update-var"))
            .chain(args.iter().copied())
            .collect();
        let matches = command
            .try_get_matches_from_mut(argv)
            .expect("real command tree must accept these args");
        let (_, sub_matches) = matches
            .subcommand()
            .filter(|(name, _)| *name == "update-var")
            .expect("update-var subcommand must match");
        sub_matches.clone()
    }

    // ── Dry-run ──

    #[tokio::test]
    async fn test_dry_run_does_not_call_network() {
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "v",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            is_dry_run: true,
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_var(&matches, &flags, &mut frontend).await;

        assert!(
            result.is_ok(),
            "dry-run should succeed without network: {result:?}"
        );
    }

    // ── Validation ──

    #[tokio::test]
    async fn test_missing_namespace_is_usage_error() {
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "v",
        ]);
        let flags = GlobalFlags {
            namespace: None,
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_var(&matches, &flags, &mut frontend).await;

        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(message.contains("namespace"));
            }
            other => panic!("expected CliError::Usage for missing namespace, got: {other:?}"),
        }
    }

    // ── T-UVAR-01: key exists → UpdateVariableV5 called; SaveVariableV5 not called ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_key_exists_updates_and_does_not_create() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/variables"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": false, "description": "old desc"}]
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/variables/id-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id-1", "configName": "MY_KEY", "applyMask": false, "description": "old desc"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "new-value",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_var(&matches, &flags, &mut frontend).await;

        assert!(result.is_ok(), "expected success, got: {result:?}");
        server.verify().await;
    }

    // ── T-UVAR-02: key absent, --force absent → CliError::Api with exact message; no create ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_key_absent_no_force_returns_api_error_and_does_not_create() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "new-value",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_var(&matches, &flags, &mut frontend).await;

        match result {
            Err(CliError::Api { ref message, .. }) => {
                assert_eq!(
                    message,
                    "variable 'MY_KEY' does not exist, use flag '--force' to create it automatically"
                );
            }
            other => panic!("expected CliError::Api, got: {other:?}"),
        }
        server.verify().await;
    }

    // ── T-UVAR-03: key absent, --force set → SaveVariableV5 called; UpdateVariableV5 not called ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_key_absent_with_force_creates_and_does_not_update() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .and(body_json(serde_json::json!({
                "configName": "MY_KEY",
                "value": "new-value",
                "applyMask": true,
                "description": "created desc",
                "source": "plaintext"
            })))
            // Real `SaveAppConfigV5Response` contract only echoes
            // configId/configName — no applyMask/description.
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "new-id", "configName": "MY_KEY"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables/new-id",
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "new-value",
            "--force",
            "--sensitive",
            "--description",
            "created desc",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = CapturingFrontend::default();

        let result = handle_update_var(&matches, &flags, &mut frontend).await;

        assert!(result.is_ok(), "expected success, got: {result:?}");
        server.verify().await;

        // The response body never carried applyMask/description, so the
        // rendered output must reflect what was *sent* (true / "created
        // desc"), not the deserialized-default false/None the old mock
        // would have hidden this bug behind.
        let output = frontend
            .last_update_var
            .expect("handler should have rendered UpdateVarOutput");
        assert!(output.created, "create path must report created=true");
        assert!(
            output.apply_mask,
            "apply_mask must reflect the value that was sent, not the response's absent field"
        );
        assert_eq!(
            output.description.as_deref(),
            Some("created desc"),
            "description must reflect the value that was sent, not the response's absent field"
        );
    }

    // ── T-UVAR-04: unset --sensitive preserves existing applyMask ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_unset_sensitive_preserves_existing_apply_mask() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/variables"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": "kept"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables/id-1",
            ))
            .and(body_json(serde_json::json!({
                "value": "new-value",
                "applyMask": true,
                "description": "kept"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": "kept"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "new-value",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_var(&matches, &flags, &mut frontend).await;

        assert!(result.is_ok(), "expected success, got: {result:?}");
        server.verify().await;
    }

    // ── Explicit `--sensitive false` overrides an existing `applyMask: true` ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_explicit_sensitive_false_overrides_existing_apply_mask_true() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/variables"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": "kept"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables/id-1",
            ))
            .and(body_json(serde_json::json!({
                "value": "new-value",
                "applyMask": false,
                "description": "kept"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id-1", "configName": "MY_KEY", "applyMask": false, "description": "kept"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "new-value",
            "--sensitive",
            "false",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_var(&matches, &flags, &mut frontend).await;

        assert!(result.is_ok(), "expected success, got: {result:?}");
        server.verify().await;
    }

    // ── T-UVAR-05: unset --description preserves existing description ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_unset_description_preserves_existing_description() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/variables"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": false, "description": "keep me"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/variables/id-1"))
            .and(body_json(serde_json::json!({
                "value": "new-value",
                "applyMask": false,
                "description": "keep me"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id-1", "configName": "MY_KEY", "applyMask": false, "description": "keep me"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "new-value",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_var(&matches, &flags, &mut frontend).await;

        assert!(result.is_ok(), "expected success, got: {result:?}");
        server.verify().await;
    }

    // ── Failure-mode table: GetListOfVariablesV5 non-200 → CliError::Api ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_list_variables_failure_is_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "v",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_var(&matches, &flags, &mut frontend).await;

        assert!(matches!(result, Err(CliError::Api { .. })));
    }

    // ── update-var is reachable through the real command tree ──

    /// `build_extend_command()` — the actual builder wired into
    /// `ags extend` — must expose `update-var` with its required args.
    /// Every other test in this module parses against the hand-rebuilt
    /// `update_var_matches` surrogate, which duplicates the arg spec
    /// but never touches `build_update_var_subcommand()` or its
    /// registration in `build_extend_command()`, nor the dispatch arm in
    /// `handlers/extend/mod.rs`. Deleting either would leave every other
    /// test in this file passing; this test would not.
    #[test]
    fn test_build_extend_command_includes_update_var() {
        let cmd = crate::invocation::builder::build_extend_command();
        let update_var = cmd
            .get_subcommands()
            .find(|sub| sub.get_name() == "update-var")
            .expect("update-var must be a subcommand of extend");

        for required in ["app", "key"] {
            let arg = update_var
                .get_arguments()
                .find(|a| a.get_id().as_str() == required)
                .unwrap_or_else(|| panic!("update-var must have --{required} flag"));
            assert!(
                arg.is_required_set(),
                "--{required} must be required on the real update-var command"
            );
        }

        // --value is required_unless_present("value-stdin"), not unconditionally
        // required, so is_required_set() returns false. Verify it is present.
        assert!(
            update_var
                .get_arguments()
                .any(|a| a.get_id().as_str() == "value"),
            "update-var must have --value flag"
        );

        for optional in ["value-stdin", "sensitive", "description", "force"] {
            assert!(
                update_var
                    .get_arguments()
                    .any(|a| a.get_id().as_str() == optional),
                "update-var must have --{optional} flag"
            );
        }
    }

    /// Routes through `handle_extend` (not `handle_update_var` directly)
    /// to prove the dispatch arm in `handlers/extend/mod.rs` is wired.
    /// Removing `Some(("update-var", sub)) => ...` from `handle_extend`
    /// makes this test return `Exit(1)` instead of `Complete`.
    #[tokio::test]
    async fn test_dispatch_routes_update_var_through_handle_extend() {
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            is_dry_run: true,
            ..Default::default()
        };
        let mut frontend = NullFrontend;
        let args: Vec<String> = [
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "v",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let ctx = crate::invocation::context::FrontendContext {
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
                color_force_off: true,
            },
            ui_intent: crate::invocation::flags::UiFlag::Auto,
        };
        let result =
            crate::invocation::handlers::extend::handle_extend(&args, &flags, &mut frontend, &ctx)
                .await;

        match result {
            Ok(InvocationOutcome::Complete) => {}
            other => panic!("update-var via handle_extend dry-run should Complete, got: {other:?}"),
        }
    }

    // ── Hyphen-leading --value ──

    /// `--value -1` (hyphen-leading) must parse through the real command
    /// tree. Without `allow_hyphen_values(true)`, clap rejects it as an
    /// unknown flag.
    #[test]
    fn test_hyphen_leading_value_parses_through_real_command_tree() {
        let mut command = crate::invocation::builder::build_extend_command();
        let argv = [
            "extend",
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "-1",
        ];
        let result = command.try_get_matches_from_mut(argv);
        let matches =
            result.expect("--value -1 must parse successfully through the real command tree");
        let (_, sub) = matches
            .subcommand()
            .filter(|(name, _)| *name == "update-var")
            .expect("update-var subcommand must match");
        assert_eq!(
            sub.get_one::<String>("value").map(String::as_str),
            Some("-1"),
            "the hyphen-leading value must reach the handler intact"
        );
    }

    // ── --value-stdin ──

    /// `resolve_var_value` with `--value-stdin` must call the reader and
    /// return its result.
    #[test]
    fn test_value_stdin_resolves_from_reader() {
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value-stdin",
        ]);
        let (value, from_flag) = resolve_var_value(&matches, || Ok("stdin-value".to_string()))
            .expect("resolve_var_value must succeed with --value-stdin");
        assert_eq!(value, "stdin-value");
        assert!(!from_flag, "--value-stdin must set from_flag to false");
    }

    /// `resolve_var_value` with `--value` returns the flag value and
    /// signals `from_flag = true` so the caller can emit a warning.
    #[test]
    fn test_value_flag_resolves_and_signals_warning() {
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "plain-value",
        ]);
        let (value, from_flag) =
            resolve_var_value(&matches, || panic!("reader must not be called"))
                .expect("resolve_var_value must succeed with --value");
        assert_eq!(value, "plain-value");
        assert!(from_flag, "--value must set from_flag to true");
    }

    /// `--value` and `--value-stdin` together must be rejected by clap.
    #[test]
    fn test_value_and_value_stdin_conflict() {
        let mut command = crate::invocation::builder::build_extend_command();
        let argv = [
            "extend",
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "v",
            "--value-stdin",
        ];
        let result = command.try_get_matches_from_mut(argv);
        assert!(
            result.is_err(),
            "--value and --value-stdin together must be rejected"
        );
    }

    /// Neither `--value` nor `--value-stdin` must be rejected by clap.
    #[test]
    fn test_neither_value_nor_value_stdin_rejected() {
        let mut command = crate::invocation::builder::build_extend_command();
        let argv = ["extend", "update-var", "--app", "my-app", "--key", "MY_KEY"];
        let result = command.try_get_matches_from_mut(argv);
        assert!(
            result.is_err(),
            "neither --value nor --value-stdin must be rejected"
        );
    }

    /// The real command tree must accept `--value-stdin` and parse it as a
    /// boolean flag.
    #[test]
    fn test_value_stdin_accepted_by_real_command_tree() {
        let matches =
            real_update_var_matches(&["--app", "my-app", "--key", "MY_KEY", "--value-stdin"]);
        assert!(
            matches.get_flag("value-stdin"),
            "--value-stdin must parse to true on the real command tree"
        );
        assert_eq!(
            matches.get_one::<String>("value"),
            None,
            "--value must be absent when --value-stdin is set"
        );
    }

    /// Dry-run with `--value-stdin` must NOT call the stdin reader.
    #[tokio::test]
    async fn test_dry_run_with_value_stdin_does_not_read_stdin() {
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value-stdin",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            is_dry_run: true,
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_var_with_reader(&matches, &flags, &mut frontend, || {
            panic!("dry-run must not read stdin")
        })
        .await;

        assert!(
            result.is_ok(),
            "dry-run with --value-stdin should succeed without reading stdin: {result:?}"
        );
    }

    // ── Value redaction from CSM error messages ──

    /// A CSM API error whose `errorMessage` field embeds the submitted
    /// variable value must NOT leak that value into the final error.
    /// Operators store API keys and tokens as masked config variables
    /// (`--sensitive`), so the CSM echo behaviour that motivated secret
    /// redaction applies here too.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_create_var_error_redacts_value_from_error_message() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "errorCode": 20004,
                "errorMessage": "value 'my-api-key-789' failed validation"
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "my-api-key-789",
            "--force",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_var(&matches, &flags, &mut frontend).await;

        let err = result.expect_err("a 400 response must produce an error");
        let msg = err.to_string();
        assert!(
            !msg.contains("my-api-key-789"),
            "error must NOT contain the submitted value: {msg}"
        );
        assert!(
            msg.contains("400"),
            "error must still include the HTTP status: {msg}"
        );
        assert!(
            msg.contains("failed validation"),
            "error must still include non-value parts of errorMessage: {msg}"
        );
    }

    /// Same redaction must apply to the update (PUT) path.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_update_var_error_redacts_value_from_error_message() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": null}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables/id-1",
            ))
            .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
                "errorCode": 20005,
                "errorMessage": "rejected value 'another-token-456' too long"
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "another-token-456",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_var(&matches, &flags, &mut frontend).await;

        let err = result.expect_err("a 422 response must produce an error");
        let msg = err.to_string();
        assert!(
            !msg.contains("another-token-456"),
            "error must NOT contain the submitted value: {msg}"
        );
        assert!(
            msg.contains("422"),
            "error must still include the HTTP status: {msg}"
        );
    }

    // ── Warning wiring: --value vs --value-stdin ──

    /// When the handler is called with `--value`, `render_warning` must
    /// fire to alert the user that the value is visible in shell history.
    /// Unit-testing `resolve_var_value` alone cannot catch a flipped
    /// condition at the call site — this test proves the wiring end-to-end.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_handler_warns_when_value_flag_used() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/variables"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": null}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables/id-1",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": null
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "v",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = CapturingFrontend::default();

        let result = handle_update_var(&matches, &flags, &mut frontend).await;
        assert!(result.is_ok(), "handler should succeed: {result:?}");
        assert!(
            frontend
                .warnings
                .iter()
                .any(|w| w.contains("shell history")),
            "handler must warn about shell history when --value is used: {:?}",
            frontend.warnings
        );
    }

    /// When the handler is called with `--value-stdin`, `render_warning`
    /// must NOT fire — the value never appeared in argv.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_handler_does_not_warn_when_value_stdin_used() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/variables"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": null}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/variables/id-1",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": null
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_var_matches(&[
            "update-var",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value-stdin",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = CapturingFrontend::default();

        let result = handle_update_var_with_reader(&matches, &flags, &mut frontend, || {
            Ok("stdin-value".to_string())
        })
        .await;
        assert!(result.is_ok(), "handler should succeed: {result:?}");
        assert!(
            frontend.warnings.is_empty(),
            "handler must NOT warn when --value-stdin is used: {:?}",
            frontend.warnings
        );
    }
}
