//! `ags extend tunnel` — TCP-to-WebSocket bridge for Extend apps.
//!
//! Binds a local TCP port and bridges connections to the CSM v2 tunnel
//! endpoint via WebSocket. The command runs until Ctrl-C.

pub(crate) mod bridge;

use bridge::FORBIDDEN_PREFIX;
pub(crate) use bridge::{run_tunnel, TunnelConfig, TunnelError};

use clap::ArgMatches;

use crate::errors::CliError;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;

/// Route `ags extend tunnel <flags>`.
pub(crate) async fn handle_tunnel(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    _frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    // Namespace is a global flag consumed by the prescan; read from flags.
    let namespace = resolve_namespace(flags)?;

    // Read per-command args from clap matches.
    let resource_name = matches
        .get_one::<String>("resource-name")
        .ok_or_else(|| CliError::Usage {
            message: "--resource-name is required for tunnel".to_string(),
            metadata: None,
        })?
        .clone();

    let local_port = matches
        .get_one::<u16>("local-port")
        .copied()
        .ok_or_else(|| CliError::Usage {
            message: "--local-port is required for tunnel".to_string(),
            metadata: None,
        })?;

    let pod_name = matches.get_one::<String>("pod-name").cloned();

    // Validate inputs: namespace, resource-name, and pod-name become URL
    // path/query segments, so they must contain only safe characters.
    // Consistent with app_ui::upload which validates namespace the same way.
    super::app_ui::upload::validate_safe_component(&namespace, "namespace")?;
    super::app_ui::upload::validate_safe_component(&resource_name, "resource-name")?;
    if let Some(ref pn) = pod_name {
        super::app_ui::upload::validate_safe_component(pn, "pod-name")?;
    }

    // Derive base URL and extract host (with port when present).
    let profile_name = flags.profile.as_deref().unwrap_or("default");
    let base_url_str = super::app_ui::upload::resolve_base_url(Some(profile_name));

    let parsed = url::Url::parse(&base_url_str).map_err(|e| CliError::Usage {
        message: format!("invalid base URL '{base_url_str}': {e}"),
        metadata: None,
    })?;

    let host_str = parsed.host_str().ok_or_else(|| CliError::Usage {
        message: format!("base URL '{base_url_str}' has no host"),
        metadata: None,
    })?;

    let host = match parsed.port() {
        Some(port) => format!("{host_str}:{port}"),
        None => host_str.to_string(),
    };

    // Build tunnel config.
    let cfg = TunnelConfig {
        host,
        namespace: namespace.clone(),
        resource_name: resource_name.clone(),
        local_port,
        pod_name,
    };

    let format_json = matches!(
        flags.format,
        Some(ags_protocol::request::OutputFormat::Json)
    );

    // Declare that this command owns its signal path, so the global Ctrl-C
    // handler defers to the tunnel's own `ctrl_c()` watcher, letting it
    // exit 0 instead of 130. Set once before the signal-sensitive scope;
    // never cleared (the flag is a one-way sticky declaration).
    crate::invocation::declare_command_owns_interrupt_path();

    // Create the session log that governs all event output.
    let session_log = super::session_log::SessionLog::new(flags.verbosity, format_json);

    // Run the tunnel bridge (blocks until Ctrl-C or cancellation).
    let result = run_tunnel(
        cfg,
        None,
        tokio_util::sync::CancellationToken::new(),
        flags.profile.as_deref(),
        session_log,
    )
    .await;

    match result {
        Ok(()) => {
            // Clean shutdown (Ctrl-C).
            session_log.stopped(local_port, &resource_name, 0);
            Ok(InvocationOutcome::Complete)
        }
        Err(TunnelError::Cancelled) => {
            // Programmatic cancellation: clean exit 0.
            session_log.stopped(local_port, &resource_name, 0);
            Ok(InvocationOutcome::Complete)
        }
        Err(TunnelError::Bind(e)) => Err(CliError::Usage {
            message: format!("Failed to bind localhost:{local_port}: {e}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Choose a different --local-port or free the port in use",
            ))),
        }),
        Err(TunnelError::Auth(ref msg)) if msg.starts_with(FORBIDDEN_PREFIX) => {
            // HTTP 403: permission error, not an auth error. Re-auth
            // cannot fix a permission gap; exit 3 (Api) so scripts do
            // not trigger re-auth on exit 2.
            Err(CliError::Api {
                message: msg.clone(),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Check that your IAM client has the required permissions for the tunnel endpoint",
                ))),
                category: crate::errors::ApiErrorCategory::Permission,
            })
        }
        Err(TunnelError::Auth(msg)) => Err(CliError::Auth {
            message: msg,
            metadata: None,
        }),
    }
}

/// Resolve the namespace from flag, env, or profile config.
fn resolve_namespace(flags: &GlobalFlags) -> Result<String, CliError> {
    ags_runtime::runtime::execution::resolve_namespace(
        flags.namespace.as_deref(),
        flags.profile.as_deref(),
    )
    .map(|(namespace, _source)| namespace)
    .ok_or_else(|| CliError::Usage {
        message: "--namespace is required for tunnel".to_string(),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            "Supply --namespace <ns>, set AGS_NAMESPACE, or run 'ags config set namespace <ns>'",
        ))),
    })
}

// ══════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── RAII env guard ──

    /// RAII guard that restores an environment variable after a test mutates it.
    // Env-mutating tests must be #[serial_test::serial] per repo convention.
    // No crate-visible TempEnvGuard exists for in-source test modules:
    // ags-runtime's is pub(crate) and tests/common/env_guard.rs is for
    // integration tests only. Follows the extend_docker_login.rs pattern.
    struct TempEnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl TempEnvGuard {
        /// Set an environment variable for the lifetime of the guard.
        fn set(key: &'static str, value: &str) -> Self {
            use std::env;
            let original = env::var(key).ok();
            env::set_var(key, value);
            Self { key, original }
        }

        /// Clear an environment variable for the lifetime of the guard.
        fn clear(key: &'static str) -> Self {
            use std::env;
            let original = env::var(key).ok();
            env::remove_var(key);
            Self { key, original }
        }
    }

    impl Drop for TempEnvGuard {
        /// Restore the original environment variable value when the guard is dropped.
        fn drop(&mut self) {
            use std::env;
            match &self.original {
                Some(val) => env::set_var(self.key, val),
                None => env::remove_var(self.key),
            }
        }
    }

    // ── Null frontend for handler-level tests ──

    struct NullFrontend;

    impl crate::frontend::Frontend for NullFrontend {
        fn render(
            &mut self,
            _output: &ags_protocol::output::CommandOutput,
        ) -> Result<(), CliError> {
            Ok(())
        }
        fn render_error(&mut self, _err: &CliError) {}
        fn render_warning(&mut self, _msg: &str, _reason: Option<&str>, _tip: Option<&str>) {}
        fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {}
        fn finish(self: Box<Self>) -> Result<(), CliError> {
            Ok(())
        }
    }

    // ── Test 3: missing --resource-name → CliError::Usage ──

    #[tokio::test]
    async fn test_missing_resource_name_is_usage_error() {
        // Build matches WITHOUT resource-name (clap arg is not required
        // in this test command so the handler's own check fires).
        let matches = clap::Command::new("tunnel")
            .arg(clap::Arg::new("resource-name").long("resource-name"))
            .arg(
                clap::Arg::new("local-port")
                    .long("local-port")
                    .value_parser(clap::value_parser!(u16)),
            )
            .arg(clap::Arg::new("pod-name").long("pod-name"))
            .try_get_matches_from(["tunnel", "--local-port", "8080"])
            .unwrap();

        // Provide a valid namespace so the namespace check passes.
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let result = handle_tunnel(&matches, &flags, &mut frontend).await;

        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(
                    message.contains("resource-name"),
                    "error must mention resource-name: {message}"
                );
            }
            other => panic!("expected CliError::Usage for missing resource-name, got: {other:?}"),
        }
    }

    // ── Test 4: missing namespace on GlobalFlags → CliError::Usage ──
    //
    // Process-wide env mutation: AGS_HOME, AGS_NAMESPACE, AGS_PROFILE.

    #[tokio::test]
    #[serial_test::serial]
    async fn test_missing_namespace_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _ns = TempEnvGuard::clear("AGS_NAMESPACE");
        let _profile = TempEnvGuard::clear("AGS_PROFILE");

        // Build valid matches (all clap args present).
        let matches = clap::Command::new("tunnel")
            .arg(clap::Arg::new("resource-name").long("resource-name"))
            .arg(
                clap::Arg::new("local-port")
                    .long("local-port")
                    .value_parser(clap::value_parser!(u16)),
            )
            .arg(clap::Arg::new("pod-name").long("pod-name"))
            .try_get_matches_from([
                "tunnel",
                "--resource-name",
                "my-app",
                "--local-port",
                "8080",
            ])
            .unwrap();

        // Namespace is NOT set on GlobalFlags — proves namespace comes
        // from the global, not from matches.
        let flags = GlobalFlags {
            namespace: None,
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let result = handle_tunnel(&matches, &flags, &mut frontend).await;

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

    // ── Finding 5: invalid namespace is rejected ──
    //
    // Process-wide env mutation: AGS_HOME, AGS_NAMESPACE, AGS_PROFILE.

    #[tokio::test]
    #[serial_test::serial]
    async fn test_invalid_namespace_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _ns = TempEnvGuard::clear("AGS_NAMESPACE");
        let _profile = TempEnvGuard::clear("AGS_PROFILE");

        let matches = clap::Command::new("tunnel")
            .arg(clap::Arg::new("resource-name").long("resource-name"))
            .arg(
                clap::Arg::new("local-port")
                    .long("local-port")
                    .value_parser(clap::value_parser!(u16)),
            )
            .arg(clap::Arg::new("pod-name").long("pod-name"))
            .try_get_matches_from(["tunnel", "--resource-name", "my-app", "--local-port", "0"])
            .unwrap();

        // Namespace with a slash — invalid per validate_safe_component.
        let flags = GlobalFlags {
            namespace: Some("bad/namespace".to_string()),
            ..Default::default()
        };

        let mut frontend = NullFrontend;

        // Without namespace validation, the handler proceeds to
        // run_tunnel and enters the accept loop; the timeout catches
        // this hang. With validation, it returns CliError::Usage.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            handle_tunnel(&matches, &flags, &mut frontend),
        )
        .await;

        match result {
            Ok(Err(CliError::Usage { ref message, .. })) => {
                assert!(
                    message.contains("namespace"),
                    "error must mention namespace: {message}"
                );
            }
            Ok(Ok(_)) => {
                panic!("expected CliError::Usage for invalid namespace, got success")
            }
            Ok(Err(ref other)) => {
                panic!("expected CliError::Usage for invalid namespace, got: {other:?}")
            }
            Err(_) => {
                panic!("handler did not validate namespace — reached run_tunnel and hung");
            }
        }
    }
}
