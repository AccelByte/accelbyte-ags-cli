//! `ags extend remote-debug connect` — precondition-gated connection to an
//! Extend debug session.
//!
//! Resolves debug info via the CSM v4 API, evaluates four preconditions,
//! retries transient failures with exponential backoff, and runs the embedded
//! tunnel, proxy agent, and service forwarder for the session.

pub(super) mod connect_once;
mod debug_mode;
pub(crate) mod disable;
pub(super) mod enable;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use clap::ArgMatches;
use tokio_util::sync::CancellationToken;

use crate::errors::CliError;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use connect_once::{ConnectError, PermanentPrecondition, SessionOutcome};

/// Maximum number of connect attempts before giving up on retriable errors.
pub(crate) const MAX_CONNECT_ATTEMPTS: u32 = 5;

/// Base backoff in seconds for the first retry delay.
const BACKOFF_BASE_SECS: u64 = 5;

/// Maximum backoff cap in seconds.
const BACKOFF_CAP_SECS: u64 = 20;

/// Jitter ratio: ±20 % of the computed delay.
const JITTER_RATIO: f64 = 0.20;

/// Compute the base backoff delay (before jitter) for a given attempt number.
///
/// Uses `BACKOFF_BASE_SECS * 2^(attempt - 1)`, capped at `BACKOFF_CAP_SECS`.
/// `attempt` is 1-based: attempt 1 → 5 s, attempt 2 → 10 s, attempt 3 → 20 s,
/// attempt 4 → 20 s (capped).
pub(crate) fn backoff_base_secs(attempt: u32) -> u64 {
    let exponent = attempt.saturating_sub(1);
    let raw = BACKOFF_BASE_SECS.saturating_mul(1u64 << exponent);
    raw.min(BACKOFF_CAP_SECS)
}

/// Apply ±`JITTER_RATIO` jitter to a base delay, returning the jittered
/// delay in seconds.
///
/// The `jitter_factor` parameter should be in `[0.0, 1.0)` and is typically
/// produced by a random number generator. A value of 0.5 yields zero jitter
/// (the midpoint).
pub(crate) fn apply_jitter(base_secs: u64, jitter_factor: f64) -> f64 {
    let base = base_secs as f64;
    // Map [0, 1) → [-1, +1)
    let offset = (jitter_factor * 2.0 - 1.0) * JITTER_RATIO;
    base * (1.0 + offset)
}

/// Format the warning line emitted before a retry sleep.
pub(crate) fn format_retry_warning(
    attempt: u32,
    max: u32,
    reason: &str,
    delay_secs: f64,
) -> String {
    format!("Attempt {attempt}/{max} failed: {reason}. Retrying in {delay_secs:.0}s...")
}

/// Outcome of the pure retry-decision function.
#[derive(Debug, PartialEq)]
pub(crate) enum RetryDecision {
    /// All preconditions passed.
    Success,
    /// A permanent precondition failed; abort immediately.
    Abort(String),
    /// Authentication failed; retrying cannot repair the credentials.
    Authentication(String),
    /// Session components did not stop before the shutdown deadline.
    ShutdownFailure(String),
    /// The first-ever attempt failed before a session was established.
    FirstFailure(String),
    /// A retriable error occurred and we have attempts remaining.
    Retry { delay: Duration, warning: String },
    /// A retriable error occurred but attempts are exhausted.
    Exhausted(String),
}

/// Pure function: given the outcome of a single `connect_once` call and the
/// current attempt number, decide what the retry loop should do next.
///
/// `attempt == 0` means no session has ever been established. Positive values
/// are reconnect attempts after a prior session.
/// `retry_delay` receives the next reconnect attempt number and returns the
/// exact duration that production will sleep.
pub(crate) fn retry_decision<D>(
    result: &Result<(), ConnectError>,
    attempt: u32,
    max_attempts: u32,
    retry_delay: D,
) -> RetryDecision
where
    D: FnOnce(u32) -> Duration,
{
    match result {
        Ok(()) => RetryDecision::Success,
        Err(ConnectError::Authentication(message)) => {
            RetryDecision::Authentication(message.clone())
        }
        Err(ConnectError::ShutdownTimeout(message)) => {
            RetryDecision::ShutdownFailure(message.clone())
        }
        Err(ConnectError::Permanent(PermanentPrecondition(msg))) => {
            RetryDecision::Abort(msg.clone())
        }
        Err(ConnectError::Retriable(reason)) => {
            if attempt == 0 {
                RetryDecision::FirstFailure(reason.clone())
            } else if attempt >= max_attempts {
                RetryDecision::Exhausted(format!("gave up after {max_attempts} attempts: {reason}"))
            } else {
                let delay = retry_delay(attempt + 1);
                let warning =
                    format_retry_warning(attempt, max_attempts, reason, delay.as_secs_f64());
                RetryDecision::Retry { delay, warning }
            }
        }
    }
}

/// Validate and normalize one local forwarding address.
///
/// A bare port becomes `localhost:<port>`; otherwise the value must be a
/// single `<host>:<port>` pair.
fn normalize_local_address(value: &str, flag: &str) -> Result<String, CliError> {
    let value = value.trim();
    if let Ok(port) = value.parse::<u16>() {
        if port > 0 {
            return Ok(format!("localhost:{port}"));
        }
    }

    let valid = value.rsplit_once(':').and_then(|(host, port)| {
        let port = port.parse::<u16>().ok()?;
        let host_is_valid = !host.is_empty()
            && !host.contains(':')
            && !host.contains('/')
            && !host.chars().any(char::is_whitespace);
        (host_is_valid && port > 0).then_some(())
    });

    if valid.is_some() {
        Ok(value.to_string())
    } else {
        Err(CliError::Usage {
            message: format!(
                "{flag} must be a port or an address in <host>:<port> form (got '{value}')"
            ),
            metadata: None,
        })
    }
}

/// One full debug connection attempt, including the established session.
trait AttemptRunner {
    fn run<'a>(
        &'a mut self,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<SessionOutcome, ConnectError>> + 'a>>;
}

struct RemoteDebugAttempt<'a> {
    runtime: &'a mut ags_runtime::runtime::Runtime,
    namespace: &'a str,
    app: &'a str,
    local_grpc_addr: &'a str,
    local_http_addr: &'a str,
    profile: &'a str,
    base_url_host: &'a str,
    session_log: crate::invocation::handlers::extend::session_log::SessionLog,
}

impl AttemptRunner for RemoteDebugAttempt<'_> {
    fn run<'a>(
        &'a mut self,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<SessionOutcome, ConnectError>> + 'a>> {
        Box::pin(async move {
            // Emit resolving_target before the debug-info dispatch.
            self.session_log.resolving_target(self.namespace, self.app);

            let debug_info = tokio::select! {
                _ = cancel.cancelled() => return Ok(SessionOutcome::Cancelled),
                result = connect_once::connect_once(
                    self.runtime,
                    self.namespace,
                    self.app,
                    self.local_grpc_addr,
                    self.local_http_addr,
                    self.profile,
                ) => result?,
            };
            let params = connect_once::SessionParams {
                namespace: self.namespace,
                local_grpc_addr: self.local_grpc_addr,
                local_http_addr: self.local_http_addr,
                profile: self.profile,
                base_url_host: self.base_url_host,
            };
            connect_once::run_session(&debug_info, &params, self.session_log, cancel).await
        })
    }
}

/// Run the first-connect/reconnect state machine.
async fn run_connect_loop<R, D, W>(
    runner: &mut R,
    cancel: CancellationToken,
    mut retry_delay: D,
    mut write_warning: W,
) -> Result<(), CliError>
where
    R: AttemptRunner,
    D: FnMut(u32) -> Duration,
    W: FnMut(&str),
{
    // Zero means no prior clean session. A retriable failure in this state is
    // surfaced immediately instead of being treated as a reconnect.
    let mut attempt = 0;
    let mut scheduled_delay = None;

    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }

        if attempt > 0 {
            let delay = scheduled_delay
                .take()
                .unwrap_or_else(|| retry_delay(attempt));
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = cancel.cancelled() => return Ok(()),
            }
        }

        // The runner owns cancellation-aware startup and session cleanup. It
        // must be awaited after cancellation so component handles are not
        // dropped before the ordered shutdown completes.
        let result = runner.run(cancel.clone()).await;

        match result {
            Ok(SessionOutcome::Cancelled) => return Ok(()),
            Ok(SessionOutcome::Ended) => {
                // Reset to 1, not 0: attempt == 0 means no prior clean
                // session, where the first retriable error exits immediately.
                attempt = 1;
                scheduled_delay = None;
            }
            Err(error) => {
                match retry_decision(
                    &Err(error.clone()),
                    attempt,
                    MAX_CONNECT_ATTEMPTS,
                    &mut retry_delay,
                ) {
                    RetryDecision::Abort(message) => {
                        return Err(CliError::Api {
                            message,
                            metadata: None,
                            category: crate::errors::ApiErrorCategory::Upstream,
                        });
                    }
                    RetryDecision::Authentication(message) => {
                        return Err(CliError::Auth {
                            message,
                            metadata: None,
                        });
                    }
                    RetryDecision::ShutdownFailure(message) => {
                        return Err(CliError::Network {
                            message,
                            metadata: None,
                        });
                    }
                    RetryDecision::FirstFailure(message) => {
                        return Err(CliError::Network {
                            message,
                            metadata: None,
                        });
                    }
                    RetryDecision::Retry { delay, warning } => {
                        write_warning(&warning);
                        attempt += 1;
                        scheduled_delay = Some(delay);
                    }
                    RetryDecision::Exhausted(message) => {
                        return Err(CliError::Network {
                            message,
                            metadata: None,
                        });
                    }
                    RetryDecision::Success => unreachable!("an error cannot produce success"),
                }
            }
        }
    }
}

/// Route `ags extend remote-debug connect <flags>`.
pub(crate) async fn handle_remote_debug_connect(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    _frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    // ── Resolve inputs ──

    let namespace = resolve_namespace(flags, "remote-debug connect")?;

    let app = matches
        .get_one::<String>("app")
        .ok_or_else(|| CliError::Usage {
            message: "--app is required for remote-debug connect".to_string(),
            metadata: None,
        })?
        .clone();

    let local_grpc_addr = normalize_local_address(
        matches
            .get_one::<String>("local-grpc-port")
            .map(|s| s.as_str())
            .unwrap_or("localhost:6565"),
        "--local-grpc-port",
    )?;

    let local_http_addr = normalize_local_address(
        matches
            .get_one::<String>("local-http-port")
            .map(|s| s.as_str())
            .unwrap_or("localhost:8000"),
        "--local-http-port",
    )?;

    let profile_name = flags.profile.as_deref().unwrap_or("default").to_string();

    // ── Validate inputs ──

    super::app_ui::upload::validate_safe_component(&namespace, "namespace")?;
    super::app_ui::upload::validate_safe_component(&app, "app")?;

    // ── Derive base URL host for the tunnel ──

    let base_url_str = super::app_ui::upload::resolve_base_url(Some(&profile_name));
    let parsed = url::Url::parse(&base_url_str).map_err(|e| CliError::Usage {
        message: format!("invalid base URL '{base_url_str}': {e}"),
        metadata: None,
    })?;
    let host_str = parsed.host_str().ok_or_else(|| CliError::Usage {
        message: format!("base URL '{base_url_str}' has no host"),
        metadata: None,
    })?;
    let base_url_host = match parsed.port() {
        Some(port) => format!("{host_str}:{port}"),
        None => host_str.to_string(),
    };

    let format_json = matches!(
        flags.format,
        Some(ags_protocol::request::OutputFormat::Json)
    );

    // Build the session log. All lifecycle events flow through this single
    // object, which decides whether and in which format each event is emitted.
    let session_log = crate::invocation::handlers::extend::session_log::SessionLog::new(
        flags.verbosity,
        format_json,
    );

    // At Verbose, install a tracing subscriber that bridges
    // extend-proxy-client's tracing::info! events into the session log.
    // At Normal and Quiet, no subscriber is installed (zero cost).
    if flags.verbosity.is_verbose() {
        crate::invocation::handlers::extend::session_log::install_tracing_bridge(session_log);
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

    // ── Retry loop and command-wide Ctrl-C ──

    // Declare that this command owns its signal path, so the global Ctrl-C
    // handler defers to the cancellation-token shutdown, letting it exit 0
    // instead of 130. Set once before the signal-sensitive scope; never
    // cleared (the flag is a one-way sticky declaration).
    crate::invocation::declare_command_owns_interrupt_path();

    let cancel = CancellationToken::new();
    let signal_cancel = cancel.clone();
    let signal_task = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_cancel.cancel();
        }
    });

    let mut runner = RemoteDebugAttempt {
        runtime: &mut runtime,
        namespace: &namespace,
        app: &app,
        local_grpc_addr: &local_grpc_addr,
        local_http_addr: &local_http_addr,
        profile: &profile_name,
        base_url_host: &base_url_host,
        session_log,
    };
    let result = run_connect_loop(
        &mut runner,
        cancel,
        |attempt| {
            Duration::from_secs_f64(apply_jitter(
                backoff_base_secs(attempt),
                rand_jitter_factor(),
            ))
        },
        crate::frontend::write_stderr_line,
    )
    .await;
    signal_task.abort();

    match result {
        Ok(()) => {
            write_json_envelope(
                format_json,
                &build_connected_envelope(&local_grpc_addr, &local_http_addr),
            );
            Ok(InvocationOutcome::Complete)
        }
        Err(error) => {
            let message = match &error {
                CliError::Auth { message, .. }
                | CliError::Api { message, .. }
                | CliError::Network { message, .. } => message.as_str(),
                _ => return Err(error),
            };
            let exit_code = error.exit_code();
            write_json_envelope(format_json, &build_error_envelope(message, exit_code));
            Err(error)
        }
    }
}

/// Resolve the namespace from flag, env, or profile config.
fn resolve_namespace(flags: &GlobalFlags, command_label: &str) -> Result<String, CliError> {
    ags_runtime::runtime::execution::resolve_namespace(
        flags.namespace.as_deref(),
        flags.profile.as_deref(),
    )
    .map(|(namespace, _source)| namespace)
    .ok_or_else(|| CliError::Usage {
        message: format!("--namespace is required for {command_label}"),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            "Supply --namespace <ns>, set AGS_NAMESPACE, or run 'ags config set namespace <ns>'",
        ))),
    })
}

/// Generate a random jitter factor in [0.0, 1.0).
fn rand_jitter_factor() -> f64 {
    use rand::Rng;
    rand::rng().random::<f64>()
}

/// Write a structured exit envelope only when JSON output was requested.
fn write_json_envelope(format_json: bool, envelope: &serde_json::Value) {
    if let Some(line) = json_envelope_line(format_json, envelope) {
        crate::frontend::write_stderr_line(&line);
    }
}

/// Serialize an envelope when JSON output is enabled.
fn json_envelope_line(format_json: bool, envelope: &serde_json::Value) -> Option<String> {
    format_json.then(|| serde_json::to_string(envelope).unwrap_or_default())
}

/// Build the JSON exit envelope emitted on clean Ctrl-C disconnect.
/// Written to stderr; stdout is always empty.
fn build_connected_envelope(grpc_addr: &str, http_addr: &str) -> serde_json::Value {
    serde_json::json!({
        "status": "disconnected",
        "grpc_addr": grpc_addr,
        "http_addr": http_addr,
        "exit_code": 0,
    })
}

/// Build the JSON exit envelope emitted on error exit.
/// Written to stderr; stdout is always empty.
fn build_error_envelope(message: &str, exit_code: i32) -> serde_json::Value {
    serde_json::json!({
        "status": "error",
        "message": message,
        "exit_code": exit_code,
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

        fn clear(key: &'static str) -> Self {
            use std::env;
            let original = env::var(key).ok();
            env::remove_var(key);
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

    // ── Null frontend ──

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

    struct FakeAttemptRunner {
        outcomes: std::collections::VecDeque<Result<SessionOutcome, ConnectError>>,
        calls: usize,
    }

    impl FakeAttemptRunner {
        fn new(outcomes: impl IntoIterator<Item = Result<SessionOutcome, ConnectError>>) -> Self {
            Self {
                outcomes: outcomes.into_iter().collect(),
                calls: 0,
            }
        }
    }

    impl AttemptRunner for FakeAttemptRunner {
        fn run<'a>(
            &'a mut self,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<SessionOutcome, ConnectError>> + 'a>> {
            self.calls += 1;
            let outcome = self
                .outcomes
                .pop_front()
                .expect("fake attempt sequence exhausted");
            Box::pin(std::future::ready(outcome))
        }
    }

    struct BlockingAttemptRunner {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl AttemptRunner for BlockingAttemptRunner {
        fn run<'a>(
            &'a mut self,
            cancel: CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<SessionOutcome, ConnectError>> + 'a>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async move {
                cancel.cancelled().await;
                Ok(SessionOutcome::Cancelled)
            })
        }
    }

    // ── Constants ──

    #[test]
    fn max_connect_attempts_is_five() {
        assert_eq!(MAX_CONNECT_ATTEMPTS, 5);
    }

    // ── Backoff ──

    #[test]
    fn backoff_sequence_is_5_10_20_20() {
        assert_eq!(backoff_base_secs(1), 5);
        assert_eq!(backoff_base_secs(2), 10);
        assert_eq!(backoff_base_secs(3), 20);
        assert_eq!(backoff_base_secs(4), 20, "must cap at 20");
        assert_eq!(backoff_base_secs(5), 20, "must cap at 20");
    }

    #[test]
    fn backoff_attempt_zero_is_base() {
        // Edge case: attempt 0 (should not happen, but must not panic).
        let val = backoff_base_secs(0);
        assert!(val <= BACKOFF_CAP_SECS, "attempt 0 must not exceed cap");
    }

    // ── Jitter ──

    #[test]
    fn jitter_midpoint_is_identity() {
        // jitter_factor = 0.5 → offset = 0 → result = base.
        let result = apply_jitter(10, 0.5);
        assert!(
            (result - 10.0).abs() < 1e-9,
            "midpoint jitter must return base; got {result}"
        );
    }

    #[test]
    fn jitter_low_extreme_reduces_by_20_percent() {
        // jitter_factor = 0.0 → offset = -0.20 → result = 0.80 * base.
        let result = apply_jitter(10, 0.0);
        assert!(
            (result - 8.0).abs() < 1e-9,
            "jitter_factor=0.0 must give 80% of base; got {result}"
        );
    }

    #[test]
    fn jitter_high_extreme_increases_by_20_percent() {
        // jitter_factor ≈ 1.0 → offset ≈ +0.20 → result ≈ 1.20 * base.
        let result = apply_jitter(10, 0.9999);
        assert!(
            (result - 12.0).abs() < 0.1,
            "jitter_factor≈1.0 must give ~120% of base; got {result}"
        );
    }

    // ── format_retry_warning ──

    #[test]
    fn retry_warning_contains_attempt_reason_and_delay() {
        let msg = format_retry_warning(2, 5, "no debug pods", 10.0);
        assert!(msg.contains("2/5"), "must contain attempt fraction: {msg}");
        assert!(msg.contains("no debug pods"), "must contain reason: {msg}");
        assert!(msg.contains("10s"), "must contain delay: {msg}");
    }

    // ── retry_decision ──

    fn midpoint_delay(attempt: u32) -> Duration {
        Duration::from_secs_f64(apply_jitter(backoff_base_secs(attempt), 0.5))
    }

    #[test]
    fn retry_decision_success_on_ok() {
        let result = Ok(());
        assert_eq!(
            retry_decision(&result, 1, 5, |_| panic!("success must not schedule")),
            RetryDecision::Success
        );
    }

    #[test]
    fn retry_decision_first_retriable_failure_does_not_retry() {
        let result = Err(ConnectError::Retriable("no debug pods".to_string()));
        assert_eq!(
            retry_decision(&result, 0, 5, |_| panic!("first failure must not schedule")),
            RetryDecision::FirstFailure("no debug pods".to_string())
        );
    }

    #[test]
    fn retry_decision_abort_on_permanent() {
        let result = Err(ConnectError::Permanent(PermanentPrecondition(
            "debug mode not enabled".to_string(),
        )));
        match retry_decision(&result, 1, 5, |_| {
            panic!("permanent failure must not schedule")
        }) {
            RetryDecision::Abort(msg) => {
                assert!(
                    msg.contains("not enabled"),
                    "abort message must propagate reason: {msg}"
                );
            }
            other => panic!("expected Abort, got {other:?}"),
        }
    }

    #[test]
    fn retry_decision_authentication_failure_does_not_retry() {
        let result = Err(ConnectError::Authentication("token expired".to_string()));
        assert_eq!(
            retry_decision(&result, 0, 5, |_| panic!("auth failure must not schedule")),
            RetryDecision::Authentication("token expired".to_string())
        );
    }

    #[test]
    fn retry_decision_shutdown_timeout_does_not_retry() {
        let result = Err(ConnectError::ShutdownTimeout(
            "timed out waiting for agent to stop".to_string(),
        ));
        assert_eq!(
            retry_decision(&result, 3, 5, |_| panic!(
                "shutdown failure must not schedule"
            )),
            RetryDecision::ShutdownFailure("timed out waiting for agent to stop".to_string())
        );
    }

    #[test]
    fn retry_decision_abort_on_permanent_even_at_attempt_one() {
        let result = Err(ConnectError::Permanent(PermanentPrecondition(
            "bad status".to_string(),
        )));
        match retry_decision(&result, 1, 5, |_| {
            panic!("permanent failure must not schedule")
        }) {
            RetryDecision::Abort(_) => {}
            other => panic!("permanent must abort even on attempt 1: {other:?}"),
        }
    }

    #[test]
    fn retry_decision_retry_when_attempts_remain() {
        let result = Err(ConnectError::Retriable("no debug pods".to_string()));
        match retry_decision(&result, 1, 5, midpoint_delay) {
            RetryDecision::Retry { delay, warning } => {
                assert_eq!(
                    delay,
                    Duration::from_secs(10),
                    "after attempt 1 fails, reconnect attempt 2 must delay 10s"
                );
                assert!(
                    warning.contains("1/5"),
                    "warning must show attempt progress: {warning}"
                );
            }
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn retry_decision_exhausted_at_max_attempts() {
        let result = Err(ConnectError::Retriable("still no pods".to_string()));
        match retry_decision(&result, 5, 5, |_| panic!("exhaustion must not schedule")) {
            RetryDecision::Exhausted(msg) => {
                assert!(
                    msg.contains("5 attempts"),
                    "exhaustion message must mention count: {msg}"
                );
                assert!(
                    msg.contains("still no pods"),
                    "exhaustion message must mention reason: {msg}"
                );
            }
            other => panic!("expected Exhausted, got {other:?}"),
        }
    }

    #[test]
    fn retry_decision_retry_delays_match_backoff_sequence() {
        // A failed reconnect schedules the following attempt. The initial
        // 5-second delay is scheduled when the clean session ends; failures
        // on attempts 1..4 therefore schedule 10/20/20/20 seconds.
        let delays: Vec<Duration> = (1..=4)
            .map(|attempt| {
                match retry_decision(
                    &Err(ConnectError::Retriable("transient".to_string())),
                    attempt,
                    5,
                    midpoint_delay,
                ) {
                    RetryDecision::Retry { delay, .. } => delay,
                    other => panic!("expected Retry for attempt {attempt}, got {other:?}"),
                }
            })
            .collect();
        assert_eq!(
            delays,
            vec![
                Duration::from_secs(10),
                Duration::from_secs(20),
                Duration::from_secs(20),
                Duration::from_secs(20),
            ]
        );
    }

    #[test]
    fn retry_decision_clean_session_reset_attempt_one_not_zero() {
        // After a clean session reset, attempt resets to 1 (not 0).
        // Verify that attempt=1 with a retriable error produces a retry
        // decision with the correct delay, NOT a success or exhaustion.
        let result = Err(ConnectError::Retriable("pods starting".to_string()));
        match retry_decision(&result, 1, 5, midpoint_delay) {
            RetryDecision::Retry { delay, .. } => {
                assert_eq!(
                    delay,
                    Duration::from_secs(10),
                    "after reconnect attempt 1 fails, attempt 2 must delay 10s"
                );
            }
            other => {
                panic!("after clean-session reset to attempt=1, expected Retry, got {other:?}")
            }
        }
    }

    #[tokio::test]
    async fn connect_loop_first_transient_failure_exits_after_one_call() {
        let mut runner =
            FakeAttemptRunner::new([Err(ConnectError::Retriable("no debug pods".to_string()))]);
        let result = run_connect_loop(
            &mut runner,
            CancellationToken::new(),
            |_| panic!("the first failure must not schedule a retry"),
            |_| panic!("the first failure must not emit a retry warning"),
        )
        .await;

        assert!(matches!(result, Err(CliError::Network { .. })));
        assert_eq!(runner.calls, 1);
    }

    #[tokio::test]
    async fn connect_loop_authentication_failure_uses_auth_exit_code() {
        let mut runner = FakeAttemptRunner::new([Err(ConnectError::Authentication(
            "token expired".to_string(),
        ))]);
        let result = run_connect_loop(
            &mut runner,
            CancellationToken::new(),
            |_| panic!("authentication failures must not retry"),
            |_| panic!("authentication failures must not emit a retry warning"),
        )
        .await;

        let error = result.expect_err("authentication failure must stop the loop");
        assert!(matches!(error, CliError::Auth { .. }));
        assert_eq!(error.exit_code(), 2);
        assert_eq!(runner.calls, 1);
    }

    #[tokio::test]
    async fn connect_loop_shutdown_timeout_is_not_swallowed_after_clean_session() {
        let mut runner = FakeAttemptRunner::new([
            Ok(SessionOutcome::Ended),
            Err(ConnectError::ShutdownTimeout(
                "timed out waiting for agent to stop".to_string(),
            )),
        ]);
        let result = run_connect_loop(
            &mut runner,
            CancellationToken::new(),
            |_| Duration::ZERO,
            |_| panic!("shutdown timeouts must not retry"),
        )
        .await;

        let error = result.expect_err("shutdown timeout must be a non-zero exit");
        assert!(matches!(error, CliError::Network { .. }));
        assert_eq!(error.exit_code(), 4);
        assert_eq!(runner.calls, 2);
    }

    #[tokio::test]
    async fn connect_loop_reconnects_after_clean_session_and_exhausts_five_attempts() {
        let outcomes = std::iter::once(Ok(SessionOutcome::Ended)).chain(
            (0..MAX_CONNECT_ATTEMPTS)
                .map(|_| Err(ConnectError::Retriable("session unavailable".to_string()))),
        );
        let mut runner = FakeAttemptRunner::new(outcomes);
        let mut warnings = Vec::new();
        let mut scheduled_attempts = Vec::new();
        let result = run_connect_loop(
            &mut runner,
            CancellationToken::new(),
            |attempt| {
                scheduled_attempts.push(attempt);
                Duration::ZERO
            },
            |warning| warnings.push(warning.to_string()),
        )
        .await;

        assert!(matches!(result, Err(CliError::Network { .. })));
        assert_eq!(runner.calls, 1 + MAX_CONNECT_ATTEMPTS as usize);
        assert_eq!(warnings.len(), (MAX_CONNECT_ATTEMPTS - 1) as usize);
        assert_eq!(scheduled_attempts, vec![1, 2, 3, 4, 5]);
        assert_eq!(
            scheduled_attempts
                .into_iter()
                .map(backoff_base_secs)
                .collect::<Vec<_>>(),
            vec![5, 10, 20, 20, 20],
            "the production delay calculator must receive each reconnect attempt exactly once"
        );
    }

    #[tokio::test]
    async fn connect_loop_retries_session_error_after_session_was_established() {
        let mut runner = FakeAttemptRunner::new([
            Ok(SessionOutcome::Ended),
            Err(ConnectError::Retriable("agent disconnected".to_string())),
            Ok(SessionOutcome::Cancelled),
        ]);
        let mut warnings = Vec::new();
        let result = run_connect_loop(
            &mut runner,
            CancellationToken::new(),
            |_| Duration::ZERO,
            |warning| warnings.push(warning.to_string()),
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(runner.calls, 3);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("agent disconnected"));
    }

    #[tokio::test]
    async fn connect_loop_cancellation_interrupts_an_in_flight_attempt() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut runner = BlockingAttemptRunner {
            calls: calls.clone(),
        };
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            trigger.cancel();
        });

        let result = tokio::time::timeout(
            Duration::from_secs(1),
            run_connect_loop(&mut runner, cancel, |_| Duration::ZERO, |_| {}),
        )
        .await
        .expect("cancellation must stop the active attempt");

        assert!(result.is_ok());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn local_address_normalizes_bare_port_and_accepts_host_port() {
        assert_eq!(
            normalize_local_address("6565", "--local-grpc-port").unwrap(),
            "localhost:6565"
        );
        assert_eq!(
            normalize_local_address("127.0.0.1:8000", "--local-http-port").unwrap(),
            "127.0.0.1:8000"
        );
    }

    #[test]
    fn local_address_rejects_invalid_values_as_usage() {
        for value in ["", "localhost", ":6565", "localhost:nope", "localhost:0"] {
            let error = normalize_local_address(value, "--local-grpc-port")
                .expect_err("invalid local address must fail");
            assert!(matches!(error, CliError::Usage { .. }), "value: {value}");
        }
    }

    // ── Handler: missing --app → CliError::Usage ──

    #[tokio::test]
    async fn test_missing_app_is_usage_error() {
        let matches = clap::Command::new("connect")
            .arg(clap::Arg::new("app").long("app"))
            .arg(clap::Arg::new("local-grpc-port").long("local-grpc-port"))
            .arg(clap::Arg::new("local-http-port").long("local-http-port"))
            .try_get_matches_from(["connect"])
            .unwrap();

        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let result = handle_remote_debug_connect(&matches, &flags, &mut frontend).await;

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
    async fn test_missing_namespace_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _ns = TempEnvGuard::clear("AGS_NAMESPACE");
        let _profile = TempEnvGuard::clear("AGS_PROFILE");

        let matches = clap::Command::new("connect")
            .arg(clap::Arg::new("app").long("app"))
            .arg(clap::Arg::new("local-grpc-port").long("local-grpc-port"))
            .arg(clap::Arg::new("local-http-port").long("local-http-port"))
            .try_get_matches_from(["connect", "--app", "my-app"])
            .unwrap();

        let flags = GlobalFlags {
            namespace: None,
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let result = handle_remote_debug_connect(&matches, &flags, &mut frontend).await;

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

    // ── Handler: invalid namespace is rejected ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_invalid_namespace_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _ns = TempEnvGuard::clear("AGS_NAMESPACE");
        let _profile = TempEnvGuard::clear("AGS_PROFILE");

        let matches = clap::Command::new("connect")
            .arg(clap::Arg::new("app").long("app"))
            .arg(clap::Arg::new("local-grpc-port").long("local-grpc-port"))
            .arg(clap::Arg::new("local-http-port").long("local-http-port"))
            .try_get_matches_from(["connect", "--app", "my-app"])
            .unwrap();

        // Namespace with slash → invalid per validate_safe_component
        let flags = GlobalFlags {
            namespace: Some("bad/namespace".to_string()),
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            handle_remote_debug_connect(&matches, &flags, &mut frontend),
        )
        .await;

        match result {
            Ok(Err(CliError::Usage { ref message, .. })) => {
                assert!(
                    message.contains("namespace"),
                    "error must mention namespace: {message}"
                );
            }
            Ok(Ok(_)) => panic!("expected CliError::Usage for invalid namespace, got success"),
            Ok(Err(ref other)) => {
                panic!("expected CliError::Usage for invalid namespace, got: {other:?}")
            }
            Err(_) => panic!("handler did not validate namespace — timed out"),
        }
    }

    // ── Handler: invalid app name is rejected ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_invalid_app_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _ns = TempEnvGuard::clear("AGS_NAMESPACE");
        let _profile = TempEnvGuard::clear("AGS_PROFILE");

        let matches = clap::Command::new("connect")
            .arg(clap::Arg::new("app").long("app"))
            .arg(clap::Arg::new("local-grpc-port").long("local-grpc-port"))
            .arg(clap::Arg::new("local-http-port").long("local-http-port"))
            .try_get_matches_from(["connect", "--app", "bad/app"])
            .unwrap();

        let flags = GlobalFlags {
            namespace: Some("valid-ns".to_string()),
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            handle_remote_debug_connect(&matches, &flags, &mut frontend),
        )
        .await;

        match result {
            Ok(Err(CliError::Usage { ref message, .. })) => {
                assert!(message.contains("app"), "error must mention app: {message}");
            }
            Ok(Ok(_)) => panic!("expected CliError::Usage for invalid app, got success"),
            Ok(Err(ref other)) => {
                panic!("expected CliError::Usage for invalid app, got: {other:?}")
            }
            Err(_) => panic!("handler did not validate app — timed out"),
        }
    }

    // ── Clap registration: connect subcommand exists ──

    /// A forbidden debug-info response is a permanent API failure. The
    /// handler must return before entering the retry delay and must dispatch
    /// the GET exactly once.
    #[tokio::test]
    #[serial_test::serial]
    async fn forbidden_debug_info_exits_api_immediately_without_retry() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v4/admin/namespaces/test-ns/apps/my-app/debuginfo",
            ))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "errorCode": 20013,
                "errorMessage": "insufficient permissions"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_keychain = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
        let _token = TempEnvGuard::set("AGS_ACCESS_TOKEN", "fake-test-token");
        let _base_url = TempEnvGuard::set("AGS_BASE_URL", &server.uri());

        let matches = clap::Command::new("connect")
            .arg(clap::Arg::new("app").long("app"))
            .arg(clap::Arg::new("local-grpc-port").long("local-grpc-port"))
            .arg(clap::Arg::new("local-http-port").long("local-http-port"))
            .try_get_matches_from(["connect", "--app", "my-app"])
            .unwrap();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            handle_remote_debug_connect(&matches, &flags, &mut frontend),
        )
        .await
        .expect("403 must return immediately without entering retry backoff");

        let error = result.expect_err("403 must fail the connect command");
        assert!(
            matches!(error, CliError::Api { .. }),
            "403 must map to CliError::Api, got: {error:?}"
        );
        assert_eq!(error.exit_code(), 3, "403 must use API exit code 3");
        server.verify().await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn first_empty_pod_response_exits_network_without_retry() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v4/admin/namespaces/test-ns/apps/my-app/debuginfo",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "appStatus": "deployment-running",
                "isDebugModeEnabled": true,
                "isDebugSessionConnected": false,
                "debugPods": []
            })))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_keychain = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
        let _token = TempEnvGuard::set("AGS_ACCESS_TOKEN", "fake-test-token");
        let _base_url = TempEnvGuard::set("AGS_BASE_URL", &server.uri());

        let matches = clap::Command::new("connect")
            .arg(clap::Arg::new("app").long("app"))
            .arg(clap::Arg::new("local-grpc-port").long("local-grpc-port"))
            .arg(clap::Arg::new("local-http-port").long("local-http-port"))
            .try_get_matches_from(["connect", "--app", "my-app"])
            .unwrap();
        let flags = GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..Default::default()
        };

        let mut frontend = NullFrontend;
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            handle_remote_debug_connect(&matches, &flags, &mut frontend),
        )
        .await
        .expect("the first transient failure must not enter retry backoff");

        let error = result.expect_err("empty pods must fail the first connection");
        assert!(matches!(error, CliError::Network { .. }));
        assert_eq!(error.exit_code(), 4);
        server.verify().await;
    }

    #[test]
    fn connect_subcommand_registered_under_remote_debug() {
        let extend_cmd = crate::invocation::builder::build_extend_command();

        let remote_debug = extend_cmd
            .get_subcommands()
            .find(|sub| sub.get_name() == "remote-debug")
            .expect("remote-debug subcommand must exist under extend");

        let connect = remote_debug
            .get_subcommands()
            .find(|sub| sub.get_name() == "connect")
            .expect("connect subcommand must exist under remote-debug");

        // Verify --app is registered.
        let app_arg = connect
            .get_arguments()
            .find(|a| a.get_long() == Some("app"));
        assert!(app_arg.is_some(), "connect must have --app flag");

        // Verify --local-grpc-port is registered.
        let grpc_arg = connect
            .get_arguments()
            .find(|a| a.get_long() == Some("local-grpc-port"));
        assert!(
            grpc_arg.is_some(),
            "connect must have --local-grpc-port flag"
        );

        // Verify --local-http-port is registered.
        let http_arg = connect
            .get_arguments()
            .find(|a| a.get_long() == Some("local-http-port"));
        assert!(
            http_arg.is_some(),
            "connect must have --local-http-port flag"
        );
    }

    #[test]
    fn remote_debug_is_visible_in_extend() {
        let extend_cmd = crate::invocation::builder::build_extend_command();
        let remote_debug = extend_cmd
            .get_subcommands()
            .find(|sub| sub.get_name() == "remote-debug")
            .expect("remote-debug must exist under extend");
        assert!(
            !remote_debug.is_hide_set(),
            "remote-debug must NOT be hidden (it was promoted from shim group)"
        );
    }

    // ── Exit envelope tests ──

    #[test]
    fn test_connected_envelope_shape() {
        let envelope = build_connected_envelope("localhost:6565", "localhost:8000");
        assert_eq!(
            envelope["status"], "disconnected",
            "status must be 'disconnected'"
        );
        assert_eq!(
            envelope["grpc_addr"], "localhost:6565",
            "grpc_addr must match"
        );
        assert_eq!(
            envelope["http_addr"], "localhost:8000",
            "http_addr must match"
        );
        assert_eq!(envelope["exit_code"], 0, "exit_code must be 0");

        let obj = envelope.as_object().unwrap();
        assert_eq!(
            obj.len(),
            4,
            "connected envelope must have exactly 4 fields: {obj:?}"
        );
    }

    #[test]
    fn test_connected_envelope_stdout_is_empty() {
        // The exit envelope is written to stderr. Stdout must be empty
        // on every path. This test verifies the envelope itself carries
        // no stdout payload — the handler's obligation to write only to
        // stderr is a structural property, not a runtime test.
        let envelope = build_connected_envelope("localhost:6565", "localhost:8000");
        assert!(
            envelope.get("stdout").is_none(),
            "connected envelope must have no stdout field"
        );
    }

    #[test]
    fn test_error_envelope_shape() {
        let envelope = build_error_envelope("session lost", 4);
        assert_eq!(envelope["status"], "error", "status must be 'error'");
        assert_eq!(envelope["message"], "session lost", "message must match");
        assert_eq!(envelope["exit_code"], 4, "exit_code must match");

        let obj = envelope.as_object().unwrap();
        assert_eq!(
            obj.len(),
            3,
            "error envelope must have exactly 3 fields: {obj:?}"
        );
    }

    #[test]
    fn test_error_envelope_exit_code_varies() {
        let e3 = build_error_envelope("forbidden", 3);
        let e4 = build_error_envelope("network", 4);
        assert_eq!(e3["exit_code"], 3);
        assert_eq!(e4["exit_code"], 4);
    }

    #[test]
    fn exit_envelopes_are_serialized_only_for_json_format() {
        let envelope = build_error_envelope("network", 4);
        assert!(json_envelope_line(false, &envelope).is_none());

        let line = json_envelope_line(true, &envelope).expect("JSON mode needs an envelope");
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed, envelope);
    }
}
