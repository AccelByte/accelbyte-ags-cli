//! `ags extend logs stream` — streams an Extend app's logs over Server-Sent Events.

pub(crate) mod output;
pub(crate) mod resume;
pub(crate) mod session;
pub(crate) mod sse;
pub(crate) mod transport;

use std::time::Instant;

use clap::ArgMatches;
use tokio_util::sync::CancellationToken;

use ags_runtime::runtime::dispatch::http::DEFAULT_TIMEOUT_SECS;

use crate::errors::CliError;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;

use self::output::{LogStreamOutput, LogStreamWriter, OpeningParams};
use self::session::run_stream_loop;
use self::transport::{
    build_stream_client, logs_url, production_token_fns, HttpStreamAttempt, LogsQuery,
    STREAM_IDLE_READ_TIMEOUT,
};

/// Route `ags extend logs stream <flags>`.
pub(crate) async fn handle_logs_stream(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    _frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    // ── Reject --output (streaming to a file is not supported) ──

    if flags.output.is_some() {
        return Err(CliError::Usage {
            message: "--output is not supported for streaming commands; \
                      use shell redirection (> file) instead"
                .to_string(),
            metadata: None,
        });
    }

    // ── Resolve inputs ──

    let namespace = super::remote_debug::resolve_namespace(flags, "logs stream")?;

    let app = matches
        .get_one::<String>("app")
        .ok_or_else(|| CliError::Usage {
            message: "--app is required for logs stream".to_string(),
            metadata: None,
        })?
        .clone();

    let follow = matches.get_flag("follow");
    let tail_lines = *matches.get_one::<u32>("tail-lines").unwrap_or(&100);
    let pods = *matches.get_one::<u32>("pods").unwrap_or(&1);
    let since_seconds = matches.get_one::<u32>("since-seconds").copied();
    let previous = matches.get_flag("previous");

    let profile_name = flags.profile.as_deref().unwrap_or("default").to_string();

    // ── Validate inputs ──

    super::app_ui::upload::validate_safe_component(&namespace, "namespace")?;
    super::app_ui::upload::validate_safe_component(&app, "app")?;

    // ── Derive base URL ──

    let base_url = super::app_ui::upload::resolve_base_url(Some(&profile_name));

    // ── Build query ──

    let query = LogsQuery {
        follow,
        tail_lines,
        pods,
        since_seconds,
        previous,
    };

    let format_json = matches!(
        flags.format,
        Some(ags_protocol::request::OutputFormat::Json)
    );

    // ── Mask SIGPIPE so broken-pipe writes return BrokenPipe errors ──
    //
    // `main::reset_sigpipe` restores SIG_DFL so most commands exit 141 when
    // piped into `head`. This streaming command handles broken pipes
    // explicitly (via `StdoutLineWrite::ReaderClosed`) and must exit 0, so
    // we ignore SIGPIPE for the remainder of the process. The dry-run
    // record also writes to stdout through `write_stdout_line_reporting_closed`,
    // so it needs this guard too. Not restored on return — see the doc
    // comment on `ignore_sigpipe`.
    ignore_sigpipe();

    // ── Dry run: report the first connection's request and return ──

    if flags.is_dry_run {
        let url = logs_url(&base_url, &namespace, &app, &query, false)?;

        let start = Instant::now();
        let elapsed = Box::new(move || start.elapsed());
        let mut writer = LogStreamOutput::new(
            flags.verbosity,
            format_json,
            follow,
            elapsed,
            Box::new(|text: &str| crate::frontend::write_stdout_line_reporting_closed(text)),
            Box::new(|text: &str| {
                crate::frontend::write_stderr_line(text);
            }),
        );
        writer.dry_run("GET", url.as_str(), &namespace, &app)?;
        return Ok(InvocationOutcome::Complete);
    }

    // ── Declare interrupt ownership ──

    crate::invocation::declare_command_owns_interrupt_path();

    let cancel = CancellationToken::new();
    let signal_cancel = cancel.clone();
    let signal_task = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_cancel.cancel();
        }
    });

    // ── Build the stream client and attempt ──

    // Both the HTTP client and the attempt need the same idle limit so that
    // `read_body` reports a timeout only when the silence actually reached it.
    let idle_timeout = STREAM_IDLE_READ_TIMEOUT;
    let stream_client = build_stream_client(idle_timeout)?;

    let header_timeout_secs = flags.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS);
    let header_timeout = std::time::Duration::from_secs(header_timeout_secs);

    let (resolve_token, refresh_token) = production_token_fns(profile_name)?;

    let attempt = HttpStreamAttempt {
        client: stream_client,
        resolve_token,
        refresh_token,
        header_timeout,
        idle_timeout,
        base_url: base_url.clone(),
        namespace: namespace.clone(),
        app: app.clone(),
        query,
    };

    // ── Build the output writer ──

    let start = Instant::now();
    let elapsed = Box::new(move || start.elapsed());
    let mut writer = LogStreamOutput::new(
        flags.verbosity,
        format_json,
        follow,
        elapsed,
        Box::new(|text: &str| crate::frontend::write_stdout_line_reporting_closed(text)),
        Box::new(|text: &str| {
            crate::frontend::write_stderr_line(text);
        }),
    );

    // ── Report opening ──

    writer.opening(&OpeningParams {
        namespace: &namespace,
        app: &app,
        pods: attempt.query.pods,
        tail: attempt.query.tail_lines,
        follow,
        since: attempt.query.since_seconds,
        previous,
    });

    // At verbose, report the request URL (never the Authorization header).
    if flags.verbosity.is_verbose() {
        if let Ok(url) = logs_url(&base_url, &namespace, &app, &attempt.query, false) {
            writer.verbose(&format!("request URL: {url}"));
        }
    }

    // ── Run the reconnect loop ──

    let result = run_stream_loop(
        &attempt,
        super::remote_debug::reconnect_delay,
        &mut writer,
        follow,
        &namespace,
        cancel,
    )
    .await;

    signal_task.abort();

    match result {
        Ok(()) => Ok(InvocationOutcome::Complete),
        Err(error) => {
            // The error record was already written by the loop. Forward the error.
            Err(error)
        }
    }
}

// ── SIGPIPE guard ──

/// Ignore SIGPIPE for the remainder of the process. On non-Unix platforms
/// this is a no-op.
///
/// The streaming logs handler needs write errors to surface as
/// `io::ErrorKind::BrokenPipe` rather than killing the process, so the
/// existing `StdoutLineWrite::ReaderClosed` path can trigger a clean exit.
///
/// SIG_IGN is deliberately NOT restored to SIG_DFL on handler return:
/// downstream teardown (Tokio runtime shutdown, `anstream` stdout flush)
/// can still write to the closed pipe after the handler returns, and a
/// restored SIG_DFL would let that late write deliver SIGPIPE (signal 13)
/// instead of the intended exit-0.
fn ignore_sigpipe() {
    #[cfg(unix)]
    // SAFETY: `signal` is async-signal-safe and process-wide. We are in an
    // async context but the disposition change is instantaneous. The
    // process exits shortly after this handler returns, so the override
    // does not affect other commands.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

// ══════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::CliError;
    use crate::invocation::flags::GlobalFlags;
    use clap::{Arg, Command};

    // ── Env guard for tests that must isolate from host config ──
    // Env-mutating tests are #[serial] per project convention.

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
        /// Restore the original environment variable value.
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

    /// Build a clap Command that mirrors `build_logs_stream_subcommand` for
    /// handler tests. Parsing through the same shape proves that the handler
    /// sees the same flag names the real command registers. Uses the same
    /// `parse_at_least_one_u32` value parser so the copies cannot drift from
    /// the real command.
    fn test_command() -> Command {
        use crate::invocation::builder::parse_at_least_one_u32;
        Command::new("stream")
            .arg(
                Arg::new("app")
                    .long("app")
                    .short('a')
                    .required(true)
                    .value_name("app"),
            )
            .arg(
                Arg::new("follow")
                    .long("follow")
                    .short('f')
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new("tail-lines")
                    .long("tail-lines")
                    .visible_alias("tail")
                    .default_value("100")
                    .value_parser(parse_at_least_one_u32),
            )
            .arg(
                Arg::new("pods")
                    .long("pods")
                    .default_value("1")
                    .value_parser(parse_at_least_one_u32),
            )
            .arg(
                Arg::new("since-seconds")
                    .long("since-seconds")
                    .value_parser(parse_at_least_one_u32),
            )
            .arg(
                Arg::new("previous")
                    .long("previous")
                    .action(clap::ArgAction::SetTrue),
            )
    }

    // ── Spec-named handler tests ──

    #[tokio::test]
    #[serial_test::serial]
    async fn missing_namespace_is_usage_error() {
        // Env-mutating test: sets AGS_HOME and clears AGS_NAMESPACE so
        // resolve_namespace finds no namespace from flag, env, or profile.
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _ns = TempEnvGuard::clear("AGS_NAMESPACE");

        let matches = test_command()
            .try_get_matches_from(["stream", "--app", "my-app"])
            .unwrap();

        let flags = GlobalFlags {
            namespace: None,
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let result = handle_logs_stream(&matches, &flags, &mut frontend).await;

        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(
                    message.contains("namespace"),
                    "error must mention namespace: {message}"
                );
                assert_eq!(result.unwrap_err().exit_code(), 1);
            }
            other => panic!("expected CliError::Usage for missing namespace, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_app_is_usage_error() {
        // Build a command without --app being required, then pass no --app.
        use crate::invocation::builder::parse_at_least_one_u32;
        let cmd = Command::new("stream")
            .arg(Arg::new("app").long("app").short('a').value_name("app"))
            .arg(
                Arg::new("follow")
                    .long("follow")
                    .short('f')
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new("tail-lines")
                    .long("tail-lines")
                    .visible_alias("tail")
                    .default_value("100")
                    .value_parser(parse_at_least_one_u32),
            )
            .arg(
                Arg::new("pods")
                    .long("pods")
                    .default_value("1")
                    .value_parser(parse_at_least_one_u32),
            )
            .arg(
                Arg::new("since-seconds")
                    .long("since-seconds")
                    .value_parser(parse_at_least_one_u32),
            )
            .arg(
                Arg::new("previous")
                    .long("previous")
                    .action(clap::ArgAction::SetTrue),
            );

        let matches = cmd.try_get_matches_from(["stream"]).unwrap();

        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let result = handle_logs_stream(&matches, &flags, &mut frontend).await;

        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(
                    message.contains("--app"),
                    "error must name --app: {message}"
                );
                assert_eq!(result.unwrap_err().exit_code(), 1);
            }
            other => panic!("expected CliError::Usage for missing --app, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_app_is_usage_error() {
        let matches = test_command()
            .try_get_matches_from(["stream", "--app", "bad/app"])
            .unwrap();

        let flags = GlobalFlags {
            namespace: Some("good-ns".to_string()),
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let result = handle_logs_stream(&matches, &flags, &mut frontend).await;

        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(message.contains("app"), "error must mention app: {message}");
                assert_eq!(result.unwrap_err().exit_code(), 1);
            }
            other => panic!("expected CliError::Usage for invalid app, got: {other:?}"),
        }
    }

    #[test]
    fn tail_lines_pods_and_since_below_one_are_usage_errors() {
        // --tail-lines 0
        let result =
            test_command().try_get_matches_from(["stream", "--app", "my-app", "--tail-lines", "0"]);
        assert!(result.is_err(), "clap must reject --tail-lines 0");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("tail-lines") || msg.contains("tail_lines"),
            "error must name --tail-lines: {msg}"
        );

        // --pods 0
        let result =
            test_command().try_get_matches_from(["stream", "--app", "my-app", "--pods", "0"]);
        assert!(result.is_err(), "clap must reject --pods 0");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("pods"), "error must name --pods: {msg}");

        // --since-seconds 0
        let result = test_command().try_get_matches_from([
            "stream",
            "--app",
            "my-app",
            "--since-seconds",
            "0",
        ]);
        assert!(result.is_err(), "clap must reject --since-seconds 0");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("since-seconds") || msg.contains("since_seconds"),
            "error must name --since-seconds: {msg}"
        );
    }

    #[tokio::test]
    async fn output_flag_is_rejected() {
        let matches = test_command()
            .try_get_matches_from(["stream", "--app", "my-app"])
            .unwrap();

        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            output: Some(ags_protocol::request::OutputDestination::File(
                "out.json".into(),
            )),
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let result = handle_logs_stream(&matches, &flags, &mut frontend).await;

        match result {
            Err(CliError::Usage { ref message, .. }) => {
                assert!(
                    message.contains("--output"),
                    "error must mention --output: {message}"
                );
                assert!(
                    message.contains("redirect"),
                    "error must suggest shell redirection: {message}"
                );
                assert_eq!(result.unwrap_err().exit_code(), 1);
            }
            other => panic!("expected CliError::Usage for --output, got: {other:?}"),
        }
    }

    // ── Dry-run test ──

    #[tokio::test]
    async fn test_dry_run_returns_complete_without_network() {
        let matches = test_command()
            .try_get_matches_from(["stream", "--app", "my-app"])
            .unwrap();

        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            is_dry_run: true,
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let result = handle_logs_stream(&matches, &flags, &mut frontend).await;

        match result {
            Ok(InvocationOutcome::Complete) => {} // expected
            other => panic!("expected InvocationOutcome::Complete for dry-run, got: {other:?}"),
        }
    }
}
