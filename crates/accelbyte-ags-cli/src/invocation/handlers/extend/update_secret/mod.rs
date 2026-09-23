//! Handler for `ags extend update-secret`.
//!
//! Upserts a CSM app secret: lists existing secrets, scans for a matching
//! `--key`, then updates it if found or creates it under `--force` if not.
//! `--sensitive` is a value-taking flag (`--sensitive [true|false]`), and
//! the update response's plaintext `value` field is never captured. See
//! `docs/reference/cli-reference.md` for the command reference.

mod api;
mod merge;

use clap::ArgMatches;

use crate::errors::CliError;
use crate::frontend::{write_stderr_line, Frontend};
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use ags_protocol::output::{CommandOutput, UpdateSecretOutput};

/// Delegate to the shared redaction helper in `csm_error.rs`.
fn redact_secret_in_error(error: CliError, secret_value: &str) -> CliError {
    super::csm_error::redact_submitted_value_in_error(error, secret_value)
}

/// Resolve the secret value from `--value` or `--value-stdin`.
///
/// Returns `(value, from_flag)`: when `from_flag` is `true`, the value came
/// from the `--value` flag and the caller should emit a warning about shell
/// history visibility.
///
/// The `stdin_reader` parameter is an injectable seam so tests can verify
/// the stdin path without a real pipe. Production passes
/// [`read_stdin_line`].
fn resolve_secret_value(
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

/// Delegate to the crate-level shared stdin reader in `errors.rs`.
fn read_stdin_line() -> Result<String, CliError> {
    crate::errors::read_stdin_line()
}

/// Execute `ags extend update-secret`.
pub(crate) async fn handle_update_secret(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    frontend: &mut dyn Frontend,
) -> Result<InvocationOutcome, CliError> {
    handle_update_secret_with_reader(matches, flags, frontend, read_stdin_line).await
}

/// Inner handler that accepts an injectable stdin reader. Production passes
/// [`read_stdin_line`]; tests pass a closure that panics or returns a fixed
/// value to verify ordering and wiring without touching real stdin.
async fn handle_update_secret_with_reader(
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
        message: "--namespace is required for extend update-secret".to_string(),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            "Supply --namespace <ns> or set a default via 'ags config set namespace <ns>'",
        ))),
    })?;

    // Dry-run must short-circuit BEFORE value resolution. The preview
    // never uses the secret value, so reading stdin here would block
    // indefinitely in automation (the HIGH that prompted this ordering).
    if flags.is_dry_run {
        return dry_run_preview(&namespace, &app, &key, force);
    }

    let (value, value_from_flag) = resolve_secret_value(matches, stdin_reader)?;
    // The warning fires even though dry-run already returned — if the user
    // supplied --value on a non-dry-run invocation, argv already carried
    // the plaintext, so the exposure has occurred and the warning is valid.
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

    let existing = api::list_secrets(
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

    let (record, created, effective) = match existing {
        Some(existing) => {
            let effective = merge::compute_effective_fields(
                &existing,
                sensitive_override,
                description_override,
            );
            let record = api::update_secret(
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
            .map_err(|e| redact_secret_in_error(e, &value))?;
            (record, false, effective)
        }
        None => {
            if !force {
                return Err(CliError::Api {
                    message: format!(
                        "secret '{key}' does not exist, use flag '--force' to create it automatically"
                    ),
                    metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                        "Pass --force to create the secret automatically",
                    ))),
                    category: crate::errors::ApiErrorCategory::Upstream,
                });
            }
            let effective = merge::compute_new_fields(sensitive_override, description_override);
            let record = api::create_secret(
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
            .map_err(|e| redact_secret_in_error(e, &value))?;
            (record, true, effective)
        }
    };

    // Neither `SaveSecretV5` (create) nor `UpdateSecretV5` (update) is
    // guaranteed to echo `applyMask`/`description` back — `SaveSecretV5`
    // never does, and `description` is not a required field on
    // `UpdateAppConfigV5Response` either. Use the values actually sent
    // (computed pre-call) for both paths instead of trusting the response,
    // so the reported output is immune to response-shape drift.
    let apply_mask = effective.apply_mask;
    let description = effective.description;

    frontend.render(&CommandOutput::UpdateSecret(UpdateSecretOutput {
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
        "Dry run — no secret will be created or updated",
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

    /// Captures the last rendered `UpdateSecretOutput` and any warnings so
    /// tests can assert on what was actually reported.
    #[derive(Default)]
    struct CapturingFrontend {
        last_update_secret: Option<UpdateSecretOutput>,
        warnings: Vec<String>,
    }

    impl crate::frontend::Frontend for CapturingFrontend {
        fn render(&mut self, output: &CommandOutput) -> Result<(), CliError> {
            if let CommandOutput::UpdateSecret(update_secret) = output {
                self.last_update_secret = Some(update_secret.clone());
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

    fn update_secret_matches(args: &[&str]) -> ArgMatches {
        clap::Command::new("update-secret")
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

    /// Parse `args` (everything after `update-secret`) through the REAL
    /// command tree — `build_extend_command()`, the same builder the CLI
    /// registers and routes through — rather than the hand-rebuilt
    /// surrogate above. Returns the `update-secret` subcommand's own
    /// `ArgMatches`.
    ///
    /// Exists so tests can prove properties about the actual clap wiring
    /// (`build_update_secret_subcommand()` and its registration in
    /// `build_extend_command()`): a test that only exercises the surrogate
    /// would keep passing even if the real subcommand were deleted or its
    /// `--sensitive` spec drifted from the surrogate's copy.
    fn real_update_secret_matches(args: &[&str]) -> ArgMatches {
        let mut command = crate::invocation::builder::build_extend_command();
        let argv: Vec<&str> = std::iter::once("extend")
            .chain(std::iter::once("update-secret"))
            .chain(args.iter().copied())
            .collect();
        let matches = command
            .try_get_matches_from_mut(argv)
            .expect("real command tree must accept these args");
        let (_, sub_matches) = matches
            .subcommand()
            .filter(|(name, _)| *name == "update-secret")
            .expect("update-secret subcommand must match");
        sub_matches.clone()
    }

    // ── Dry-run ──

    #[tokio::test]
    async fn test_dry_run_does_not_call_network() {
        let matches = update_secret_matches(&[
            "update-secret",
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

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;

        assert!(
            result.is_ok(),
            "dry-run should succeed without network: {result:?}"
        );
    }

    // ── Validation ──

    #[tokio::test]
    async fn test_missing_namespace_is_usage_error() {
        let matches = update_secret_matches(&[
            "update-secret",
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

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;

        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(message.contains("namespace"));
            }
            other => panic!("expected CliError::Usage for missing namespace, got: {other:?}"),
        }
    }

    // ── T-USEC-01: key exists → UpdateSecretV5 called; SaveSecretV5 not called ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_key_exists_updates_and_does_not_create() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": "old desc"}]
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets/id-1"))
            // Proves the merge rule directly: unset --sensitive/--description
            // preserve the existing record's applyMask (true) and
            // description ("old desc") in the PUT body, not just that some
            // PUT was made.
            .and(body_json(serde_json::json!({
                "value": "new-value",
                "applyMask": true,
                "description": "old desc"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": "old desc"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_secret_matches(&[
            "update-secret",
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

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;

        assert!(result.is_ok(), "expected success, got: {result:?}");
        server.verify().await;
    }

    /// `--force` must never override "key exists → update": passing
    /// `--force` alongside an existing key still takes the UPDATE path
    /// (UpdateSecretV5), not create (SaveSecretV5).
    #[tokio::test]
    #[serial_test::serial]
    async fn test_force_with_existing_key_still_updates_and_does_not_create() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": "old desc"}]
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets/id-1"))
            .and(body_json(serde_json::json!({
                "value": "new-value",
                "applyMask": true,
                "description": "old desc"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": "old desc"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_secret_matches(&[
            "update-secret",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "new-value",
            "--force",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;

        assert!(result.is_ok(), "expected success, got: {result:?}");
        server.verify().await;
    }

    // ── T-USEC-02: key absent, --force absent → CliError::Api with exact secret message ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_key_absent_no_force_returns_exact_secret_message_and_does_not_create() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_secret_matches(&[
            "update-secret",
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

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;

        match result {
            Err(CliError::Api { ref message, .. }) => {
                assert_eq!(
                    message,
                    "secret 'MY_KEY' does not exist, use flag '--force' to create it automatically"
                );
            }
            other => panic!("expected CliError::Api, got: {other:?}"),
        }
        server.verify().await;
    }

    // ── T-USEC-03: --sensitive defaults to true on create ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_create_without_sensitive_defaults_apply_mask_true() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .and(body_json(serde_json::json!({
                "configName": "MY_KEY",
                "value": "new-value",
                "applyMask": true,
                "description": null,
                "source": "plaintext"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "new-id", "configName": "MY_KEY"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets/new-id",
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_secret_matches(&[
            "update-secret",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "new-value",
            "--force",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = CapturingFrontend::default();

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;

        assert!(result.is_ok(), "expected success, got: {result:?}");
        server.verify().await;

        let output = frontend
            .last_update_secret
            .expect("handler should have rendered UpdateSecretOutput");
        assert!(output.created);
        assert!(
            output.apply_mask,
            "unset --sensitive on create must default applyMask to true"
        );
    }

    // ── T-USEC-04: --sensitive false explicitly removes masking on update ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_explicit_sensitive_false_removes_masking_on_update() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": "kept"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets/id-1"))
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
        let matches = update_secret_matches(&[
            "update-secret",
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
        let mut frontend = CapturingFrontend::default();

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;

        assert!(result.is_ok(), "expected success, got: {result:?}");
        server.verify().await;

        let output = frontend
            .last_update_secret
            .expect("handler should have rendered UpdateSecretOutput");
        assert!(
            !output.apply_mask,
            "--sensitive false must explicitly remove masking even though the existing record was masked"
        );
    }

    // ── Never leak the plaintext value into rendered output ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_update_response_plaintext_value_never_reaches_output() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": null}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets/id-1",
            ))
            // Real CSM echoes plaintext `value` back on update — this mock
            // reproduces that so the test proves the handler never
            // forwards it, rather than merely never testing for it.
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id-1", "configName": "MY_KEY", "applyMask": true,
                "description": null, "source": "plaintext", "value": "super-secret-value"
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_secret_matches(&[
            "update-secret",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "super-secret-value",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = CapturingFrontend::default();

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;

        assert!(result.is_ok(), "expected success, got: {result:?}");
        let output = frontend
            .last_update_secret
            .expect("handler should have rendered UpdateSecretOutput");
        // UpdateSecretOutput has no `value` field at all — this assertion
        // is really about the type, but stands here as the end-to-end
        // proof that the whole path (list → update → render) never
        // introduces one.
        let serialized = serde_json::to_string(&output).unwrap();
        assert!(!serialized.contains("super-secret-value"));
    }

    // ── Failure-mode table: GetListOfSecretsV5 non-200 → CliError::Api ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_list_secrets_failure_is_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_secret_matches(&[
            "update-secret",
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

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;

        assert!(matches!(result, Err(CliError::Api { .. })));
    }

    // ── update-secret is reachable through the real command tree ──

    /// `build_extend_command()` — the actual builder wired into
    /// `ags extend` — must expose `update-secret` with its required args.
    /// Every other test in this module parses against the hand-rebuilt
    /// `update_secret_matches` surrogate, which duplicates the arg spec
    /// but never touches `build_update_secret_subcommand()` or its
    /// registration in `build_extend_command()`, nor the dispatch arm in
    /// `handlers/extend/mod.rs`. Deleting either would leave every other
    /// test in this file passing; this test would not.
    #[test]
    fn test_build_extend_command_includes_update_secret() {
        let cmd = crate::invocation::builder::build_extend_command();
        let update_secret = cmd
            .get_subcommands()
            .find(|sub| sub.get_name() == "update-secret")
            .expect("update-secret must be a subcommand of extend");

        for required in ["app", "key"] {
            let arg = update_secret
                .get_arguments()
                .find(|a| a.get_id().as_str() == required)
                .unwrap_or_else(|| panic!("update-secret must have --{required} flag"));
            assert!(
                arg.is_required_set(),
                "--{required} must be required on the real update-secret command"
            );
        }

        // --value is required_unless_present("value-stdin"), not unconditionally
        // required, so is_required_set() returns false. Verify it is present.
        assert!(
            update_secret
                .get_arguments()
                .any(|a| a.get_id().as_str() == "value"),
            "update-secret must have --value flag"
        );

        // --value-stdin, --sensitive, --description, --force are present but optional.
        for optional in ["value-stdin", "sensitive", "description", "force"] {
            assert!(
                update_secret
                    .get_arguments()
                    .any(|a| a.get_id().as_str() == optional),
                "update-secret must have --{optional} flag"
            );
        }
    }

    /// The real `update-secret` command must route through
    /// `handle_update_secret` end-to-end: parsing via the real command
    /// tree and executing the dry-run path (no network) proves both the
    /// clap registration and the dispatch arm in `handlers/extend/mod.rs`
    /// are wired together, not just independently present.
    #[tokio::test]
    async fn test_real_command_tree_dry_run_reaches_handler() {
        let matches =
            real_update_secret_matches(&["--app", "my-app", "--key", "MY_KEY", "--value", "v"]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            is_dry_run: true,
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;

        assert!(
            result.is_ok(),
            "dry-run via the real command tree should succeed: {result:?}"
        );
    }

    // ── Bare `--sensitive` (no value) ──

    /// Bare `--sensitive` (no value at all) must parse to `Some(true)` via
    /// `default_missing_value("true")`, on both the surrogate and the real
    /// `build_update_secret_subcommand()`. T-USEC-03 covers omitting
    /// `--sensitive` entirely (`None`, preserved/defaulted downstream) and
    /// T-USEC-04 covers `--sensitive false`; neither exercises the bare
    /// form.
    #[test]
    fn test_bare_sensitive_flag_parses_to_some_true() {
        let surrogate = update_secret_matches(&[
            "update-secret",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "v",
            "--sensitive",
        ]);
        assert_eq!(
            surrogate.get_one::<bool>("sensitive").copied(),
            Some(true),
            "bare --sensitive must parse to Some(true) on the surrogate command"
        );

        let real = real_update_secret_matches(&[
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "v",
            "--sensitive",
        ]);
        assert_eq!(
            real.get_one::<bool>("sensitive").copied(),
            Some(true),
            "bare --sensitive must parse to Some(true) on the real update-secret command"
        );
        assert_eq!(
            real.value_source("sensitive"),
            Some(clap::parser::ValueSource::CommandLine),
            "bare --sensitive must report an explicit CommandLine value source"
        );
    }

    /// Bare `--sensitive` on the update path must apply `applyMask: true`
    /// to the PUT body even when the existing record's mask was `false` —
    /// proving the bare-flag form is treated as an explicit override, the
    /// same as `--sensitive true`, and distinct from omitting the flag
    /// (T-USEC-03's create-path default, a different code path).
    #[tokio::test]
    #[serial_test::serial]
    async fn test_bare_sensitive_flag_overrides_existing_mask_on_update() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": false, "description": "kept"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets/id-1",
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
        let matches = real_update_secret_matches(&[
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "new-value",
            "--sensitive",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = CapturingFrontend::default();

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;

        assert!(result.is_ok(), "expected success, got: {result:?}");
        server.verify().await;

        let output = frontend
            .last_update_secret
            .expect("handler should have rendered UpdateSecretOutput");
        assert!(
            output.apply_mask,
            "bare --sensitive must override the existing (false) mask to true"
        );
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
            "update-secret",
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
            .filter(|(name, _)| *name == "update-secret")
            .expect("update-secret subcommand must match");
        assert_eq!(
            sub.get_one::<String>("value").map(String::as_str),
            Some("-1"),
            "the hyphen-leading value must reach the handler intact"
        );
    }

    // ── --value-stdin ──

    /// `resolve_secret_value` with `--value-stdin` must call the reader and
    /// return its result. The injectable seam proves the stdin path works
    /// without requiring a real stdin pipe.
    #[test]
    fn test_value_stdin_resolves_from_reader() {
        let matches = update_secret_matches(&[
            "update-secret",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value-stdin",
        ]);
        let (value, from_flag) = resolve_secret_value(&matches, || Ok("stdin-value".to_string()))
            .expect("resolve_secret_value must succeed with --value-stdin");
        assert_eq!(value, "stdin-value");
        assert!(!from_flag, "--value-stdin must set from_flag to false");
    }

    /// `resolve_secret_value` with `--value` returns the flag value and
    /// signals `from_flag = true` so the caller can emit a warning.
    #[test]
    fn test_value_flag_resolves_and_signals_warning() {
        let matches = update_secret_matches(&[
            "update-secret",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "plain-value",
        ]);
        let (value, from_flag) =
            resolve_secret_value(&matches, || panic!("reader must not be called"))
                .expect("resolve_secret_value must succeed with --value");
        assert_eq!(value, "plain-value");
        assert!(from_flag, "--value must set from_flag to true");
    }

    /// `--value` and `--value-stdin` together must be rejected by clap.
    #[test]
    fn test_value_and_value_stdin_conflict() {
        let mut command = crate::invocation::builder::build_extend_command();
        let argv = [
            "extend",
            "update-secret",
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
        let argv = [
            "extend",
            "update-secret",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
        ];
        let result = command.try_get_matches_from_mut(argv);
        assert!(
            result.is_err(),
            "neither --value nor --value-stdin must be rejected"
        );
    }

    /// Dry-run with `--value-stdin` must NOT call the stdin reader. The
    /// dry-run path only needs namespace, app, key, and force — the value
    /// is irrelevant. If the reader fires, the command blocks on stdin
    /// indefinitely in automation, which is the HIGH finding this test
    /// prevents.
    #[tokio::test]
    async fn test_dry_run_with_value_stdin_does_not_read_stdin() {
        let matches = update_secret_matches(&[
            "update-secret",
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

        // Pass a reader that panics: if the dry-run path reads stdin,
        // this test crashes instead of silently passing.
        let result = handle_update_secret_with_reader(&matches, &flags, &mut frontend, || {
            panic!("dry-run must not read stdin")
        })
        .await;

        assert!(
            result.is_ok(),
            "dry-run with --value-stdin should succeed without reading stdin: {result:?}"
        );
    }

    /// The real command tree must accept `--value-stdin` and parse it as a
    /// boolean flag.
    #[test]
    fn test_value_stdin_accepted_by_real_command_tree() {
        let matches =
            real_update_secret_matches(&["--app", "my-app", "--key", "MY_KEY", "--value-stdin"]);
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

    // ── Secret value redaction from CSM error messages ──

    /// A CSM API error whose `errorMessage` field embeds the submitted secret
    /// value must NOT leak that value into the final `CliError::Api` message.
    /// The handler must redact any occurrence of the submitted value before
    /// the error reaches the caller.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_create_secret_error_redacts_value_from_error_message() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
            .mount(&server)
            .await;
        // The CSM error response embeds the submitted value inside errorMessage.
        Mock::given(method("POST"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "errorCode": 20004,
                "errorMessage": "value 'super-secret-123' failed validation"
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_secret_matches(&[
            "update-secret",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "super-secret-123",
            "--force",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;

        let err = result.expect_err("a 400 response must produce an error");
        let msg = err.to_string();
        assert!(
            !msg.contains("super-secret-123"),
            "error must NOT contain the submitted secret value: {msg}"
        );
        // The surrounding context must survive redaction.
        assert!(
            msg.contains("400"),
            "error must still include the HTTP status: {msg}"
        );
        assert!(
            msg.contains("failed validation"),
            "error must still include non-secret parts of errorMessage: {msg}"
        );
    }

    /// Same redaction must apply to the update (PUT) path.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_update_secret_error_redacts_value_from_error_message() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": null}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets/id-1",
            ))
            .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
                "errorCode": 20005,
                "errorMessage": "rejected value 'another-secret-456' too long"
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_secret_matches(&[
            "update-secret",
            "--app",
            "my-app",
            "--key",
            "MY_KEY",
            "--value",
            "another-secret-456",
        ]);
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };
        let mut frontend = NullFrontend;

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;

        let err = result.expect_err("a 422 response must produce an error");
        let msg = err.to_string();
        assert!(
            !msg.contains("another-secret-456"),
            "error must NOT contain the submitted secret value: {msg}"
        );
        assert!(
            msg.contains("422"),
            "error must still include the HTTP status: {msg}"
        );
    }

    // ── Empty-value redaction ──

    /// `redact_secret_in_error` with an empty value must return the error
    /// message untouched. `str::replace("", "[REDACTED]")` matches at every
    /// byte boundary, turning the message into confetti — a direct consequence
    /// of clap's `required_unless_present` checking presence, not content.
    /// Redacting an empty string never serves the security goal (there is no
    /// credential to hide), so the fix is to short-circuit.
    #[test]
    fn test_redact_empty_value_returns_message_unchanged() {
        let error = CliError::Api {
            message: "CSM SaveSecretV5 returned HTTP 400: validation error".to_string(),
            metadata: None,
            category: crate::errors::ApiErrorCategory::Upstream,
        };
        let redacted = redact_secret_in_error(error, "");
        match redacted {
            CliError::Api { ref message, .. } => {
                assert_eq!(
                    message, "CSM SaveSecretV5 returned HTTP 400: validation error",
                    "empty-value redaction must not mangle the message: {message}"
                );
            }
            other => panic!("expected CliError::Api, got: {other:?}"),
        }
    }

    // ── Warning wiring: --value fires the warning, --value-stdin does not ──

    /// When the handler is called with `--value`, `render_warning` must
    /// actually fire at the call site. A test that only exercises
    /// `resolve_secret_value` cannot catch a flipped condition at the call
    /// site — this test proves the wiring end-to-end.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_handler_warns_when_value_flag_used() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": null}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets/id-1",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": null
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_secret_matches(&[
            "update-secret",
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

        let result = handle_update_secret(&matches, &flags, &mut frontend).await;
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
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": null}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets/id-1",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": null
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_runtime_env(&tmp, &server);
        let matches = update_secret_matches(&[
            "update-secret",
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

        let result = handle_update_secret_with_reader(&matches, &flags, &mut frontend, || {
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
