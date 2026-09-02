//! Shared session log for long-running `extend` commands.
//!
//! Owns the start instant, verbosity, and format flag, and provides one
//! method per event type. Decides in one place whether an event is
//! written and in which format. Neither command formats an event line
//! itself.

use ags_protocol::request::Verbosity;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

// ── Elapsed-time formatting ──

/// Format an elapsed duration as `[+<n>s]`, whole seconds, no zero
/// padding, no conversion to minutes. A two-hour session shows
/// `[+7241s]`.
pub(crate) fn format_elapsed(elapsed: Duration) -> String {
    format!("[+{}s]", elapsed.as_secs())
}

// ── Session log ──

/// Session log for long-running commands that emit lifecycle events
/// to stderr. All fields are `Copy`, so the log can be cheaply cloned
/// into spawned tasks.
#[derive(Clone, Copy)]
pub(crate) struct SessionLog {
    started_at: Instant,
    verbosity: Verbosity,
    format_json: bool,
}

impl SessionLog {
    /// Create a new session log. The start instant is captured at
    /// construction and all subsequent elapsed times are relative to it.
    pub(crate) fn new(verbosity: Verbosity, format_json: bool) -> Self {
        Self {
            started_at: Instant::now(),
            verbosity,
            format_json,
        }
    }

    /// Emit the `listening` event. Called once after the TCP listener
    /// binds successfully.
    pub(crate) fn listening(&self, local_port: u16, resource_name: &str) {
        if let Some(line) =
            self.format_listening(local_port, resource_name, self.started_at.elapsed())
        {
            crate::frontend::write_stderr_line(&line);
        }
    }

    /// Emit the `client_connected` event. Called in the accept branch
    /// before the handler task is spawned.
    pub(crate) fn client_connected(&self, peer_addr: &SocketAddr) {
        if let Some(line) = self.format_client_connected(peer_addr, self.started_at.elapsed()) {
            crate::frontend::write_stderr_line(&line);
        }
    }

    /// Emit the `client_disconnected` event. Called inside the spawned
    /// task after `handle_client_connection` returns `Ok`.
    pub(crate) fn client_disconnected(&self, peer_addr: &SocketAddr) {
        if let Some(line) = self.format_client_disconnected(peer_addr, self.started_at.elapsed()) {
            crate::frontend::write_stderr_line(&line);
        }
    }

    /// Emit the `connection_error` event. Called inside the spawned
    /// task when `handle_client_connection` returns `Err`.
    pub(crate) fn connection_error(&self, peer_addr: &SocketAddr, message: &str) {
        if let Some(line) =
            self.format_connection_error(peer_addr, message, self.started_at.elapsed())
        {
            crate::frontend::write_stderr_line(&line);
        }
    }

    /// Emit the `accept_error` event. Called when `listener.accept()`
    /// fails (non-fatal). Not tied to a specific peer, so it carries no
    /// `peer_addr` — only the error message.
    pub(crate) fn accept_error(&self, message: &str) {
        if let Some(line) = self.format_accept_error(message, self.started_at.elapsed()) {
            crate::frontend::write_stderr_line(&line);
        }
    }

    // ── Format helpers (deterministic elapsed for testing) ──

    /// Format a `listening` event line. Returns `None` when suppressed
    /// by quiet verbosity.
    fn format_listening(
        &self,
        local_port: u16,
        resource_name: &str,
        elapsed: Duration,
    ) -> Option<String> {
        if self.verbosity.is_quiet() {
            return None;
        }
        Some(if self.format_json {
            Self::to_json_line(&Self::listening_json(
                local_port,
                resource_name,
                elapsed.as_millis() as u64,
            ))
        } else {
            crate::frontend::style::ansi::status(
                &format!(
                    "{} listening on localhost:{}  resource={}  (Ctrl-C to stop)",
                    format_elapsed(elapsed),
                    local_port,
                    resource_name
                ),
                crate::frontend::style::ansi::is_stderr_enabled(),
            )
        })
    }

    /// Format a `client_connected` event line.
    fn format_client_connected(&self, peer_addr: &SocketAddr, elapsed: Duration) -> Option<String> {
        if self.verbosity.is_quiet() {
            return None;
        }
        Some(if self.format_json {
            Self::to_json_line(&Self::client_connected_json(
                &peer_addr.to_string(),
                elapsed.as_millis() as u64,
            ))
        } else {
            crate::frontend::style::ansi::status(
                &format!(
                    "{} client connected from {}",
                    format_elapsed(elapsed),
                    peer_addr
                ),
                crate::frontend::style::ansi::is_stderr_enabled(),
            )
        })
    }

    /// Format a `client_disconnected` event line.
    fn format_client_disconnected(
        &self,
        peer_addr: &SocketAddr,
        elapsed: Duration,
    ) -> Option<String> {
        if self.verbosity.is_quiet() {
            return None;
        }
        Some(if self.format_json {
            Self::to_json_line(&Self::client_disconnected_json(
                &peer_addr.to_string(),
                elapsed.as_millis() as u64,
            ))
        } else {
            crate::frontend::style::ansi::status(
                &format!(
                    "{} client disconnected from {}",
                    format_elapsed(elapsed),
                    peer_addr
                ),
                crate::frontend::style::ansi::is_stderr_enabled(),
            )
        })
    }

    /// Format a `connection_error` event line.
    fn format_connection_error(
        &self,
        peer_addr: &SocketAddr,
        message: &str,
        elapsed: Duration,
    ) -> Option<String> {
        if self.verbosity.is_quiet() {
            return None;
        }
        Some(if self.format_json {
            Self::to_json_line(&Self::connection_error_json(
                &peer_addr.to_string(),
                message,
                elapsed.as_millis() as u64,
            ))
        } else {
            crate::frontend::style::ansi::status(
                &format!(
                    "{} connection error from {}: {}",
                    format_elapsed(elapsed),
                    peer_addr,
                    message
                ),
                crate::frontend::style::ansi::is_stderr_enabled(),
            )
        })
    }

    /// Format an `accept_error` event line. Not tied to a peer; carries
    /// only the error message.
    fn format_accept_error(&self, message: &str, elapsed: Duration) -> Option<String> {
        if self.verbosity.is_quiet() {
            return None;
        }
        Some(if self.format_json {
            Self::to_json_line(&Self::accept_error_json(
                message,
                elapsed.as_millis() as u64,
            ))
        } else {
            crate::frontend::style::ansi::status(
                &format!("{} accept error: {}", format_elapsed(elapsed), message),
                crate::frontend::style::ansi::is_stderr_enabled(),
            )
        })
    }

    // ── JSON builders ──

    fn listening_json(local_port: u16, resource_name: &str, elapsed_ms: u64) -> serde_json::Value {
        serde_json::json!({
            "event": "listening",
            "local_port": local_port,
            "resource_name": resource_name,
            "elapsed_ms": elapsed_ms,
        })
    }

    fn client_connected_json(peer_addr: &str, elapsed_ms: u64) -> serde_json::Value {
        serde_json::json!({
            "event": "client_connected",
            "peer_addr": peer_addr,
            "elapsed_ms": elapsed_ms,
        })
    }

    fn client_disconnected_json(peer_addr: &str, elapsed_ms: u64) -> serde_json::Value {
        serde_json::json!({
            "event": "client_disconnected",
            "peer_addr": peer_addr,
            "elapsed_ms": elapsed_ms,
        })
    }

    fn connection_error_json(peer_addr: &str, message: &str, elapsed_ms: u64) -> serde_json::Value {
        serde_json::json!({
            "event": "connection_error",
            "peer_addr": peer_addr,
            "message": message,
            "elapsed_ms": elapsed_ms,
        })
    }

    fn accept_error_json(message: &str, elapsed_ms: u64) -> serde_json::Value {
        serde_json::json!({
            "event": "accept_error",
            "message": message,
            "elapsed_ms": elapsed_ms,
        })
    }

    // ── Connect lifecycle events ──

    /// Emit the `resolving_target` event. Called before the debug-info
    /// dispatch to show which namespace/app pair is being resolved.
    pub(crate) fn resolving_target(&self, namespace: &str, app: &str) {
        if let Some(line) = self.format_resolving_target(namespace, app, self.started_at.elapsed())
        {
            crate::frontend::write_stderr_line(&line);
        }
    }

    /// Emit the `connecting` event. Called after the pod resolves,
    /// before the bridge starts.
    pub(crate) fn connecting(&self, pod_name: &str, pod_port: u16) {
        if let Some(line) = self.format_connecting(pod_name, pod_port, self.started_at.elapsed()) {
            crate::frontend::write_stderr_line(&line);
        }
    }

    /// Emit the `connected` event. Called when the session is ready,
    /// replacing the previous `format_ready_line` call site.
    pub(crate) fn connected(&self, grpc_addr: &str, http_addr: &str) {
        if let Some(line) = self.format_connected(grpc_addr, http_addr, self.started_at.elapsed()) {
            crate::frontend::write_stderr_line(&line);
        }
    }

    /// Emit the `service_listening` event. Called once per forwarder
    /// service listener after it binds.
    pub(crate) fn service_listening(&self, service: &str, local_addr: &str) {
        if let Some(line) =
            self.format_service_listening(service, local_addr, self.started_at.elapsed())
        {
            crate::frontend::write_stderr_line(&line);
        }
    }

    /// Emit the `session_ended` event. Called where the session outcome
    /// is decided.
    pub(crate) fn session_ended(&self, reason: &str) {
        if let Some(line) = self.format_session_ended(reason, self.started_at.elapsed()) {
            crate::frontend::write_stderr_line(&line);
        }
    }

    /// Emit a verbose protocol event. Called by the tracing bridge
    /// layer when a tracing event from `extend_proxy_client` arrives.
    pub(crate) fn verbose_protocol(&self, message: &str) {
        if let Some(line) = self.format_verbose_protocol(message, self.started_at.elapsed()) {
            crate::frontend::write_stderr_line(&line);
        }
    }

    /// Emit the `stopped` event. Called on clean shutdown or
    /// cancellation. In JSON mode the envelope carries the tunnel's
    /// identity fields (`local_port`, `resource_name`) for backward
    /// compatibility with scripts reading the exit status. In human
    /// mode a short status line is emitted.
    pub(crate) fn stopped(&self, local_port: u16, resource_name: &str, exit_code: i32) {
        if let Some(line) = self.format_stopped(
            local_port,
            resource_name,
            exit_code,
            self.started_at.elapsed(),
        ) {
            crate::frontend::write_stderr_line(&line);
        }
    }

    // ── Connect event format helpers ──

    fn format_resolving_target(
        &self,
        namespace: &str,
        app: &str,
        elapsed: Duration,
    ) -> Option<String> {
        if self.verbosity.is_quiet() {
            return None;
        }
        Some(if self.format_json {
            Self::to_json_line(&Self::resolving_target_json(
                namespace,
                app,
                elapsed.as_millis() as u64,
            ))
        } else {
            crate::frontend::style::ansi::status(
                &format!(
                    "{} resolving debug info  namespace={} app={}",
                    format_elapsed(elapsed),
                    namespace,
                    app
                ),
                crate::frontend::style::ansi::is_stderr_enabled(),
            )
        })
    }

    fn format_connecting(
        &self,
        pod_name: &str,
        pod_port: u16,
        elapsed: Duration,
    ) -> Option<String> {
        if self.verbosity.is_quiet() {
            return None;
        }
        Some(if self.format_json {
            Self::to_json_line(&Self::connecting_json(
                pod_name,
                pod_port,
                elapsed.as_millis() as u64,
            ))
        } else {
            crate::frontend::style::ansi::status(
                &format!(
                    "{} connecting to pod {}:{}",
                    format_elapsed(elapsed),
                    pod_name,
                    pod_port
                ),
                crate::frontend::style::ansi::is_stderr_enabled(),
            )
        })
    }

    fn format_connected(
        &self,
        grpc_addr: &str,
        http_addr: &str,
        elapsed: Duration,
    ) -> Option<String> {
        if self.verbosity.is_quiet() {
            return None;
        }
        Some(if self.format_json {
            Self::to_json_line(&Self::connected_json(
                grpc_addr,
                http_addr,
                elapsed.as_millis() as u64,
            ))
        } else {
            crate::frontend::style::ansi::status(
                &format!(
                    "{} debug session ready  gRPC={}, HTTP={}  (Ctrl-C to disconnect)",
                    format_elapsed(elapsed),
                    grpc_addr,
                    http_addr
                ),
                crate::frontend::style::ansi::is_stderr_enabled(),
            )
        })
    }

    fn format_service_listening(
        &self,
        service: &str,
        local_addr: &str,
        elapsed: Duration,
    ) -> Option<String> {
        if self.verbosity.is_quiet() {
            return None;
        }
        Some(if self.format_json {
            Self::to_json_line(&Self::service_listening_json(
                service,
                local_addr,
                elapsed.as_millis() as u64,
            ))
        } else {
            crate::frontend::style::ansi::status(
                &format!(
                    "{} service {} listening on {}",
                    format_elapsed(elapsed),
                    service,
                    local_addr
                ),
                crate::frontend::style::ansi::is_stderr_enabled(),
            )
        })
    }

    fn format_session_ended(&self, reason: &str, elapsed: Duration) -> Option<String> {
        if self.verbosity.is_quiet() {
            return None;
        }
        Some(if self.format_json {
            Self::to_json_line(&Self::session_ended_json(
                reason,
                elapsed.as_millis() as u64,
            ))
        } else {
            crate::frontend::style::ansi::status(
                &format!("{} session ended: {}", format_elapsed(elapsed), reason),
                crate::frontend::style::ansi::is_stderr_enabled(),
            )
        })
    }

    fn format_verbose_protocol(&self, message: &str, elapsed: Duration) -> Option<String> {
        if !self.verbosity.is_verbose() {
            return None;
        }
        Some(if self.format_json {
            Self::to_json_line(&Self::verbose_protocol_json(
                message,
                elapsed.as_millis() as u64,
            ))
        } else {
            crate::frontend::style::ansi::status(
                &format!("{} {}", format_elapsed(elapsed), message),
                crate::frontend::style::ansi::is_stderr_enabled(),
            )
        })
    }

    // ── Connect event JSON builders ──

    fn resolving_target_json(namespace: &str, app: &str, elapsed_ms: u64) -> serde_json::Value {
        serde_json::json!({
            "event": "resolving_target",
            "namespace": namespace,
            "app": app,
            "elapsed_ms": elapsed_ms,
        })
    }

    fn connecting_json(pod_name: &str, pod_port: u16, elapsed_ms: u64) -> serde_json::Value {
        serde_json::json!({
            "event": "connecting",
            "pod_name": pod_name,
            "pod_port": pod_port,
            "elapsed_ms": elapsed_ms,
        })
    }

    fn connected_json(grpc_addr: &str, http_addr: &str, elapsed_ms: u64) -> serde_json::Value {
        serde_json::json!({
            "event": "connected",
            "grpc_addr": grpc_addr,
            "http_addr": http_addr,
            "elapsed_ms": elapsed_ms,
        })
    }

    fn service_listening_json(
        service: &str,
        local_addr: &str,
        elapsed_ms: u64,
    ) -> serde_json::Value {
        serde_json::json!({
            "event": "service_listening",
            "service": service,
            "local_addr": local_addr,
            "elapsed_ms": elapsed_ms,
        })
    }

    fn session_ended_json(reason: &str, elapsed_ms: u64) -> serde_json::Value {
        serde_json::json!({
            "event": "session_ended",
            "reason": reason,
            "elapsed_ms": elapsed_ms,
        })
    }

    fn verbose_protocol_json(message: &str, elapsed_ms: u64) -> serde_json::Value {
        serde_json::json!({
            "event": "verbose_protocol",
            "message": message,
            "elapsed_ms": elapsed_ms,
        })
    }

    // ── Stopped / exit envelope ──

    fn format_stopped(
        &self,
        local_port: u16,
        resource_name: &str,
        exit_code: i32,
        elapsed: Duration,
    ) -> Option<String> {
        if self.verbosity.is_quiet() {
            return None;
        }
        Some(if self.format_json {
            Self::to_json_line(&Self::stopped_json(
                local_port,
                resource_name,
                exit_code,
                elapsed.as_millis() as u64,
            ))
        } else {
            crate::frontend::style::ansi::status(
                &format!(
                    "{} tunnel stopped  localhost:{}  resource={}",
                    format_elapsed(elapsed),
                    local_port,
                    resource_name,
                ),
                crate::frontend::style::ansi::is_stderr_enabled(),
            )
        })
    }

    fn stopped_json(
        local_port: u16,
        resource_name: &str,
        exit_code: i32,
        elapsed_ms: u64,
    ) -> serde_json::Value {
        serde_json::json!({
            "event": "stopped",
            "status": "stopped",
            "local_port": local_port,
            "resource_name": resource_name,
            "exit_code": exit_code,
            "elapsed_ms": elapsed_ms,
        })
    }

    /// Serialise a JSON value to a single line.
    fn to_json_line(value: &serde_json::Value) -> String {
        serde_json::to_string(value).unwrap_or_else(|_| r#"{"event":"unknown"}"#.to_string())
    }
}

// ── Tracing bridge layer ──
//
// A custom `tracing_subscriber::Layer` that receives tracing events from
// `extend_proxy_client` and forwards them through the session log's verbose
// output. Installed as a global subscriber only when `--verbose` is active.

/// A tracing subscriber layer that forwards events to a callback.
///
/// Used to bridge `extend_proxy_client`'s `tracing::info!` calls into
/// the CLI's session log output, respecting `--format json` and verbosity.
pub(crate) struct TracingBridgeLayer<F> {
    on_event: F,
}

impl<F> TracingBridgeLayer<F>
where
    F: Fn(String) + Send + Sync + 'static,
{
    /// Create a new bridge layer that calls `on_event` for each tracing
    /// event, with the formatted message string.
    pub(crate) fn new(on_event: F) -> Self {
        Self { on_event }
    }
}

impl<S, F> tracing_subscriber::Layer<S> for TracingBridgeLayer<F>
where
    S: tracing::Subscriber,
    F: Fn(String) + Send + Sync + 'static,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        // Format: collect the message field and all named fields into a
        // single line. The message field is the first positional argument
        // in `tracing::info!("...", field = value)`.
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);
        let line = if visitor.fields.is_empty() {
            visitor.message
        } else {
            format!("{} {}", visitor.message, visitor.fields.join(", "))
        };
        (self.on_event)(line);
    }
}

/// Visitor that extracts the message and named fields from a tracing event.
#[derive(Default)]
struct EventVisitor {
    message: String,
    fields: Vec<String>,
}

impl tracing::field::Visit for EventVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields.push(format!("{}={:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push(format!("{}={}", field.name(), value));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.push(format!("{}={}", field.name(), value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.push(format!("{}={}", field.name(), value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields.push(format!("{}={}", field.name(), value));
    }
}

/// Install a global tracing subscriber that bridges `extend_proxy_client`
/// events to the session log. Only called when verbosity is `Verbose`.
///
/// Uses `set_global_default` because extend-proxy-client produces its
/// events from tokio tasks that may run on different threads — a
/// thread-local subscriber (`with_default`) would miss them.
///
/// If a subscriber is already installed (returns an error), the error is
/// silently ignored: the CLI process runs one command, so this can only
/// happen in tests where another test installed one first.
pub(crate) fn install_tracing_bridge(session_log: SessionLog) {
    install_tracing_bridge_core(move |line: String| {
        session_log.verbose_protocol(&line);
    });
}

/// Core installation logic: builds the filtered subscriber and installs
/// it globally via `set_global_default`. Factored from
/// `install_tracing_bridge` so the multi-threaded test can supply a test
/// callback while exercising the same filter and installation path.
fn install_tracing_bridge_core<F>(callback: F)
where
    F: Fn(String) + Send + Sync + 'static,
{
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::Layer as _;

    let layer = TracingBridgeLayer::new(callback);

    // Filter to `extend_proxy_client` at `info` and above. No other
    // crate's tracing output is admitted.
    //
    // This is a deliberate pass-through: the bridge forwards tracing
    // output verbatim to stderr (human mode) or into a JSON envelope
    // (--format json). It relies on extend-proxy-client's log-safety
    // contract (documented in that crate's lib.rs) which requires that
    // tracing output at INFO and above never carries tokens, bearer
    // headers, cookies, or other credential material.
    //
    // The literal target name "extend_proxy_client" must track the
    // dependency's crate name. If the crate is renamed in Cargo.toml,
    // update this literal to match.
    let filter = tracing_subscriber::filter::Targets::new()
        .with_target("extend_proxy_client", tracing::Level::INFO);

    let subscriber = tracing_subscriber::Registry::default().with(layer.with_filter(filter));

    // Ignore the error: a second install in the same process is benign.
    let _ = tracing::subscriber::set_global_default(subscriber);
}

// ══════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a session log with a fixed start instant for deterministic
    /// elapsed values in format tests.
    fn log_with(verbosity: Verbosity, format_json: bool) -> SessionLog {
        SessionLog {
            started_at: Instant::now(),
            verbosity,
            format_json,
        }
    }

    // ── U1: format_elapsed ──

    #[test]
    fn test_format_elapsed_zero() {
        assert_eq!(format_elapsed(Duration::from_secs(0)), "[+0s]");
    }

    #[test]
    fn test_format_elapsed_large() {
        assert_eq!(format_elapsed(Duration::from_secs(7241)), "[+7241s]");
    }

    #[test]
    fn test_format_elapsed_subsecond_truncates() {
        // 999 ms rounds down to 0 whole seconds.
        assert_eq!(format_elapsed(Duration::from_millis(999)), "[+0s]");
    }

    // ── U2: at Quiet, every event type produces no output ──

    #[test]
    fn test_quiet_suppresses_listening() {
        let log = log_with(Verbosity::Quiet, false);
        assert!(
            log.format_listening(8080, "my-app", Duration::ZERO)
                .is_none(),
            "listening must be suppressed at Quiet"
        );
    }

    #[test]
    fn test_quiet_suppresses_client_connected() {
        let log = log_with(Verbosity::Quiet, false);
        let addr: SocketAddr = "127.0.0.1:46760".parse().unwrap();
        assert!(
            log.format_client_connected(&addr, Duration::ZERO).is_none(),
            "client_connected must be suppressed at Quiet"
        );
    }

    #[test]
    fn test_quiet_suppresses_client_disconnected() {
        let log = log_with(Verbosity::Quiet, false);
        let addr: SocketAddr = "127.0.0.1:46760".parse().unwrap();
        assert!(
            log.format_client_disconnected(&addr, Duration::ZERO)
                .is_none(),
            "client_disconnected must be suppressed at Quiet"
        );
    }

    #[test]
    fn test_quiet_suppresses_connection_error() {
        let log = log_with(Verbosity::Quiet, false);
        let addr: SocketAddr = "127.0.0.1:46760".parse().unwrap();
        assert!(
            log.format_connection_error(&addr, "boom", Duration::ZERO)
                .is_none(),
            "connection_error must be suppressed at Quiet"
        );
    }

    #[test]
    fn test_quiet_suppresses_json_listening() {
        let log = log_with(Verbosity::Quiet, true);
        assert!(
            log.format_listening(8080, "my-app", Duration::ZERO)
                .is_none(),
            "listening must be suppressed at Quiet even in JSON mode"
        );
    }

    // ── U3: at Normal, each Normal-level event produces output ──

    #[test]
    fn test_normal_emits_listening() {
        let log = log_with(Verbosity::Normal, false);
        assert!(
            log.format_listening(8080, "my-app", Duration::ZERO)
                .is_some(),
            "listening must produce output at Normal"
        );
    }

    #[test]
    fn test_normal_emits_client_connected() {
        let log = log_with(Verbosity::Normal, false);
        let addr: SocketAddr = "127.0.0.1:46760".parse().unwrap();
        assert!(
            log.format_client_connected(&addr, Duration::ZERO).is_some(),
            "client_connected must produce output at Normal"
        );
    }

    #[test]
    fn test_normal_emits_client_disconnected() {
        let log = log_with(Verbosity::Normal, false);
        let addr: SocketAddr = "127.0.0.1:46760".parse().unwrap();
        assert!(
            log.format_client_disconnected(&addr, Duration::ZERO)
                .is_some(),
            "client_disconnected must produce output at Normal"
        );
    }

    #[test]
    fn test_normal_emits_connection_error() {
        let log = log_with(Verbosity::Normal, false);
        let addr: SocketAddr = "127.0.0.1:46760".parse().unwrap();
        assert!(
            log.format_connection_error(&addr, "boom", Duration::ZERO)
                .is_some(),
            "connection_error must produce output at Normal"
        );
    }

    // ── U4: every event emitted at Normal is also emitted at Verbose ──

    #[test]
    fn test_verbose_emits_listening() {
        let log = log_with(Verbosity::Verbose, false);
        assert!(
            log.format_listening(8080, "my-app", Duration::ZERO)
                .is_some(),
            "listening must produce output at Verbose"
        );
    }

    #[test]
    fn test_verbose_emits_client_connected() {
        let log = log_with(Verbosity::Verbose, false);
        let addr: SocketAddr = "127.0.0.1:46760".parse().unwrap();
        assert!(
            log.format_client_connected(&addr, Duration::ZERO).is_some(),
            "client_connected must produce output at Verbose"
        );
    }

    #[test]
    fn test_verbose_emits_client_disconnected() {
        let log = log_with(Verbosity::Verbose, false);
        let addr: SocketAddr = "127.0.0.1:46760".parse().unwrap();
        assert!(
            log.format_client_disconnected(&addr, Duration::ZERO)
                .is_some(),
            "client_disconnected must produce output at Verbose"
        );
    }

    #[test]
    fn test_verbose_emits_connection_error() {
        let log = log_with(Verbosity::Verbose, false);
        let addr: SocketAddr = "127.0.0.1:46760".parse().unwrap();
        assert!(
            log.format_connection_error(&addr, "boom", Duration::ZERO)
                .is_some(),
            "connection_error must produce output at Verbose"
        );
    }

    // ── U5: each event serialises with `event` and `elapsed_ms` plus its fields ──

    #[test]
    fn test_listening_json_has_required_fields() {
        let json = SessionLog::listening_json(8080, "my-app", 42);
        assert_eq!(json["event"], "listening");
        assert_eq!(json["elapsed_ms"], 42);
        assert_eq!(json["local_port"], 8080);
        assert_eq!(json["resource_name"], "my-app");
    }

    #[test]
    fn test_client_connected_json_has_required_fields() {
        let json = SessionLog::client_connected_json("127.0.0.1:46760", 3000);
        assert_eq!(json["event"], "client_connected");
        assert_eq!(json["elapsed_ms"], 3000);
        assert_eq!(json["peer_addr"], "127.0.0.1:46760");
    }

    #[test]
    fn test_client_disconnected_json_has_required_fields() {
        let json = SessionLog::client_disconnected_json("127.0.0.1:46760", 9000);
        assert_eq!(json["event"], "client_disconnected");
        assert_eq!(json["elapsed_ms"], 9000);
        assert_eq!(json["peer_addr"], "127.0.0.1:46760");
    }

    #[test]
    fn test_connection_error_json_has_required_fields() {
        let json = SessionLog::connection_error_json("127.0.0.1:46760", "dial failed", 5000);
        assert_eq!(json["event"], "connection_error");
        assert_eq!(json["elapsed_ms"], 5000);
        assert_eq!(json["peer_addr"], "127.0.0.1:46760");
        assert_eq!(json["message"], "dial failed");
    }

    // ── U7: the `listening` object still carries `local_port` and `resource_name` ──
    // (backward-compatibility pin: a script reading these today keeps working)

    #[test]
    fn test_listening_json_backward_compat_fields() {
        let json = SessionLog::listening_json(27017, "extend-nosql", 0);
        assert!(
            json.get("local_port").is_some(),
            "listening JSON must carry local_port"
        );
        assert!(
            json.get("resource_name").is_some(),
            "listening JSON must carry resource_name"
        );
        assert_eq!(json["local_port"], 27017);
        assert_eq!(json["resource_name"], "extend-nosql");
    }

    // ── Human format content checks ──

    #[test]
    fn test_listening_human_contains_elapsed_and_port() {
        let log = log_with(Verbosity::Normal, false);
        let line = log
            .format_listening(27017, "my-app", Duration::from_secs(0))
            .unwrap();
        assert!(
            line.contains("[+0s]"),
            "listening line must contain elapsed prefix: {line}"
        );
        assert!(
            line.contains("27017"),
            "listening line must contain port: {line}"
        );
        assert!(
            line.contains("listening on"),
            "listening line must contain 'listening on': {line}"
        );
    }

    #[test]
    fn test_listening_human_contains_ctrl_c_hint() {
        let log = log_with(Verbosity::Normal, false);
        let line = log
            .format_listening(8080, "my-app", Duration::from_secs(0))
            .unwrap();
        assert!(
            line.contains("[+0s]"),
            "listening line must contain elapsed prefix: {line}"
        );
        assert!(
            line.contains("(Ctrl-C to stop)"),
            "listening line must contain Ctrl-C hint for usability on a blocking command: {line}"
        );
    }

    #[test]
    fn test_listening_json_does_not_contain_ctrl_c_hint() {
        let log = log_with(Verbosity::Normal, true);
        let line = log
            .format_listening(8080, "my-app", Duration::from_secs(0))
            .unwrap();
        assert!(
            !line.contains("Ctrl-C"),
            "JSON listening event must not carry a Ctrl-C hint: {line}"
        );
    }

    #[test]
    fn test_client_connected_human_contains_peer_addr() {
        let log = log_with(Verbosity::Normal, false);
        let addr: SocketAddr = "127.0.0.1:46760".parse().unwrap();
        let line = log
            .format_client_connected(&addr, Duration::from_secs(3))
            .unwrap();
        assert!(
            line.contains("[+3s]"),
            "connected line must contain elapsed: {line}"
        );
        assert!(
            line.contains("127.0.0.1:46760"),
            "connected line must contain peer addr: {line}"
        );
        assert!(
            line.contains("client connected"),
            "connected line must contain 'client connected': {line}"
        );
    }

    #[test]
    fn test_client_disconnected_human_contains_peer_addr() {
        let log = log_with(Verbosity::Normal, false);
        let addr: SocketAddr = "127.0.0.1:46760".parse().unwrap();
        let line = log
            .format_client_disconnected(&addr, Duration::from_secs(9))
            .unwrap();
        assert!(
            line.contains("[+9s]"),
            "disconnected line must contain elapsed: {line}"
        );
        assert!(
            line.contains("127.0.0.1:46760"),
            "disconnected line must contain peer addr: {line}"
        );
        assert!(
            line.contains("client disconnected"),
            "disconnected line must contain 'client disconnected': {line}"
        );
    }

    // ── JSON format round-trip through format methods ──

    #[test]
    fn test_format_listening_json_parses_back() {
        let log = log_with(Verbosity::Normal, true);
        let line = log
            .format_listening(8080, "my-app", Duration::from_secs(1))
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&line)
            .unwrap_or_else(|e| panic!("listening JSON must parse: {e}\nline: {line}"));
        assert_eq!(parsed["event"], "listening");
        assert!(
            parsed.get("elapsed_ms").is_some(),
            "must carry elapsed_ms: {parsed}"
        );
    }

    #[test]
    fn test_format_client_connected_json_parses_back() {
        let log = log_with(Verbosity::Normal, true);
        let addr: SocketAddr = "127.0.0.1:46760".parse().unwrap();
        let line = log
            .format_client_connected(&addr, Duration::from_secs(3))
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&line)
            .unwrap_or_else(|e| panic!("client_connected JSON must parse: {e}\nline: {line}"));
        assert_eq!(parsed["event"], "client_connected");
        assert!(
            parsed.get("elapsed_ms").is_some(),
            "must carry elapsed_ms: {parsed}"
        );
    }

    // ── U5b: each of the five connect events serialises correctly ──

    #[test]
    fn test_resolving_target_json_has_required_fields() {
        let json = SessionLog::resolving_target_json("test-ns", "my-app", 100);
        assert_eq!(json["event"], "resolving_target");
        assert_eq!(json["elapsed_ms"], 100);
        assert_eq!(json["namespace"], "test-ns");
        assert_eq!(json["app"], "my-app");
    }

    #[test]
    fn test_connecting_json_has_required_fields() {
        let json = SessionLog::connecting_json("pod-abc", 15080, 200);
        assert_eq!(json["event"], "connecting");
        assert_eq!(json["elapsed_ms"], 200);
        assert_eq!(json["pod_name"], "pod-abc");
        assert_eq!(json["pod_port"], 15080);
    }

    #[test]
    fn test_connected_json_has_required_fields() {
        let json = SessionLog::connected_json("localhost:6565", "localhost:8000", 500);
        assert_eq!(json["event"], "connected");
        assert_eq!(json["elapsed_ms"], 500);
        assert_eq!(json["grpc_addr"], "localhost:6565");
        assert_eq!(json["http_addr"], "localhost:8000");
    }

    #[test]
    fn test_service_listening_json_has_required_fields() {
        let json = SessionLog::service_listening_json("metrics", "127.0.0.1:9090", 750);
        assert_eq!(json["event"], "service_listening");
        assert_eq!(json["elapsed_ms"], 750);
        assert_eq!(json["service"], "metrics");
        assert_eq!(json["local_addr"], "127.0.0.1:9090");
    }

    #[test]
    fn test_session_ended_json_has_required_fields() {
        let json = SessionLog::session_ended_json("user cancelled", 12000);
        assert_eq!(json["event"], "session_ended");
        assert_eq!(json["elapsed_ms"], 12000);
        assert_eq!(json["reason"], "user cancelled");
    }

    // ── U7b: backward-compatibility pin for the connected event ──
    // The `connected` event must always carry `grpc_addr` and `http_addr`,
    // matching the existing `format_ready_line` contract.

    #[test]
    fn test_connected_json_backward_compat_grpc_and_http() {
        let json = SessionLog::connected_json("localhost:6565", "localhost:8000", 0);
        assert!(
            json.get("grpc_addr").is_some(),
            "connected JSON must carry grpc_addr"
        );
        assert!(
            json.get("http_addr").is_some(),
            "connected JSON must carry http_addr"
        );
        assert_eq!(json["grpc_addr"], "localhost:6565");
        assert_eq!(json["http_addr"], "localhost:8000");
    }

    // ── Connect event quiet suppression ──

    #[test]
    fn test_quiet_suppresses_resolving_target() {
        let log = log_with(Verbosity::Quiet, false);
        assert!(
            log.format_resolving_target("ns", "app", Duration::ZERO)
                .is_none(),
            "resolving_target must be suppressed at Quiet"
        );
    }

    #[test]
    fn test_quiet_suppresses_connecting() {
        let log = log_with(Verbosity::Quiet, false);
        assert!(
            log.format_connecting("pod", 15080, Duration::ZERO)
                .is_none(),
            "connecting must be suppressed at Quiet"
        );
    }

    #[test]
    fn test_quiet_suppresses_connected() {
        let log = log_with(Verbosity::Quiet, false);
        assert!(
            log.format_connected("localhost:6565", "localhost:8000", Duration::ZERO)
                .is_none(),
            "connected must be suppressed at Quiet"
        );
    }

    #[test]
    fn test_quiet_suppresses_service_listening() {
        let log = log_with(Verbosity::Quiet, false);
        assert!(
            log.format_service_listening("svc", "127.0.0.1:9090", Duration::ZERO)
                .is_none(),
            "service_listening must be suppressed at Quiet"
        );
    }

    #[test]
    fn test_quiet_suppresses_session_ended() {
        let log = log_with(Verbosity::Quiet, false);
        assert!(
            log.format_session_ended("cancelled", Duration::ZERO)
                .is_none(),
            "session_ended must be suppressed at Quiet"
        );
    }

    // ── Connect events at Normal ──

    #[test]
    fn test_normal_emits_resolving_target() {
        let log = log_with(Verbosity::Normal, false);
        assert!(
            log.format_resolving_target("ns", "app", Duration::ZERO)
                .is_some(),
            "resolving_target must produce output at Normal"
        );
    }

    #[test]
    fn test_normal_emits_connected() {
        let log = log_with(Verbosity::Normal, false);
        assert!(
            log.format_connected("localhost:6565", "localhost:8000", Duration::ZERO)
                .is_some(),
            "connected must produce output at Normal"
        );
    }

    #[test]
    fn test_normal_emits_session_ended() {
        let log = log_with(Verbosity::Normal, false);
        assert!(
            log.format_session_ended("cancelled", Duration::ZERO)
                .is_some(),
            "session_ended must produce output at Normal"
        );
    }

    // ── Connected content check (addresses LOW-1) ──

    #[test]
    fn test_connected_human_contains_elapsed_and_addresses() {
        let log = log_with(Verbosity::Normal, false);
        let line = log
            .format_connected("localhost:6565", "localhost:8000", Duration::from_secs(5))
            .unwrap();
        assert!(
            line.contains("[+5s]"),
            "connected line must contain elapsed prefix: {line}"
        );
        assert!(
            line.contains("localhost:6565"),
            "connected line must contain gRPC addr: {line}"
        );
        assert!(
            line.contains("localhost:8000"),
            "connected line must contain HTTP addr: {line}"
        );
    }

    // ── Accept-error event tests ──

    #[test]
    fn test_quiet_suppresses_accept_error() {
        let log = log_with(Verbosity::Quiet, false);
        assert!(
            log.format_accept_error("connection reset", Duration::ZERO)
                .is_none(),
            "accept_error must be suppressed at Quiet"
        );
    }

    #[test]
    fn test_accept_error_json_has_required_fields() {
        let json = SessionLog::accept_error_json("connection reset", 42);
        assert_eq!(json["event"], "accept_error");
        assert_eq!(json["elapsed_ms"], 42);
        assert_eq!(json["message"], "connection reset");
    }

    #[test]
    fn test_accept_error_human_contains_elapsed_and_message() {
        let log = log_with(Verbosity::Normal, false);
        let line = log
            .format_accept_error("connection reset", Duration::from_secs(7))
            .unwrap();
        assert!(
            line.contains("[+7s]"),
            "accept_error line must contain elapsed prefix: {line}"
        );
        assert!(
            line.contains("connection reset"),
            "accept_error line must contain message: {line}"
        );
    }

    // ── Stopped event tests ──

    #[test]
    fn test_quiet_suppresses_stopped() {
        let log = log_with(Verbosity::Quiet, false);
        assert!(
            log.format_stopped(8080, "my-app", 0, Duration::ZERO)
                .is_none(),
            "stopped must be suppressed at Quiet"
        );
    }

    #[test]
    fn test_stopped_json_has_required_fields() {
        let json = SessionLog::stopped_json(8080, "my-app", 0, 42);
        assert_eq!(json["event"], "stopped");
        assert_eq!(json["status"], "stopped");
        assert_eq!(json["local_port"], 8080);
        assert_eq!(json["resource_name"], "my-app");
        assert_eq!(json["exit_code"], 0);
        assert_eq!(json["elapsed_ms"], 42);

        let obj = json.as_object().unwrap();
        assert_eq!(obj.len(), 6, "stopped must have exactly 6 fields: {obj:?}");
    }

    #[test]
    fn test_stopped_json_backward_compat_fields() {
        // The stopped envelope must always carry status, local_port,
        // resource_name, and exit_code for backward compatibility with
        // scripts that read these fields.
        let json = SessionLog::stopped_json(27017, "tunnel-app", 0, 0);
        assert!(
            json.get("status").is_some(),
            "stopped JSON must carry status"
        );
        assert!(
            json.get("local_port").is_some(),
            "stopped JSON must carry local_port"
        );
        assert!(
            json.get("resource_name").is_some(),
            "stopped JSON must carry resource_name"
        );
        assert!(
            json.get("exit_code").is_some(),
            "stopped JSON must carry exit_code"
        );
        assert!(
            json.get("elapsed_ms").is_some(),
            "stopped JSON must carry elapsed_ms"
        );
        assert!(json.get("event").is_some(), "stopped JSON must carry event");
    }

    #[test]
    fn test_stopped_human_contains_elapsed() {
        let log = log_with(Verbosity::Normal, false);
        let line = log
            .format_stopped(8080, "my-app", 0, Duration::from_secs(12))
            .unwrap();
        assert!(
            line.contains("[+12s]"),
            "stopped line must contain elapsed prefix: {line}"
        );
        assert!(
            line.contains("tunnel stopped"),
            "stopped line must contain 'tunnel stopped': {line}"
        );
    }

    #[test]
    fn test_stopped_human_contains_port_and_resource() {
        let log = log_with(Verbosity::Normal, false);
        let line = log
            .format_stopped(27017, "extend-nosql", 0, Duration::from_secs(60))
            .unwrap();
        assert!(
            line.contains("27017"),
            "stopped human line must contain port for multi-tunnel disambiguation: {line}"
        );
        assert!(
            line.contains("extend-nosql"),
            "stopped human line must contain resource name for multi-tunnel disambiguation: {line}"
        );
    }

    // ── U12: global tracing bridge delivers events across threads ──
    //
    // extend-proxy-client produces tracing events from tokio tasks on
    // worker threads. Only a global subscriber (set_global_default)
    // captures them; a thread-local subscriber (set_default) misses
    // events from other threads.
    //
    // This test calls install_tracing_bridge_core — the same function
    // body that install_tracing_bridge delegates to — with a test
    // callback instead of a SessionLog. It runs on a multi-threaded
    // tokio runtime so the spawned task executes on a different OS
    // thread.
    //
    // set_global_default succeeds only once per process. This test is
    // the sole caller in this crate's unit-test binary, so it gets the
    // one shot. Running the full suite twice verifies no ordering
    // dependency.

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_tracing_event_from_spawned_task_reaches_session_log() {
        use std::sync::{Arc, Mutex};

        let collected: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let collected_for_install = collected.clone();

        // Call the production installation path. install_tracing_bridge_core
        // builds the same subscriber, applies the same target filter, and
        // calls set_global_default — the only difference is the callback.
        install_tracing_bridge_core(move |line: String| {
            collected_for_install.lock().unwrap().push(line);
        });

        // Spawn a task on a WORKER thread. On a multi-threaded runtime
        // the task runs on a different OS thread than the test. A
        // thread-local subscriber (set_default) would miss this event;
        // only set_global_default delivers it.
        let handle = tokio::spawn(async {
            tracing::info!(
                target: "extend_proxy_client",
                test_field = "hello",
                "test message from spawned task"
            );
        });
        handle.await.unwrap();

        let lines = collected.lock().unwrap();
        assert!(
            !lines.is_empty(),
            "a tracing event from a spawned task on a worker thread must \
             reach the bridge layer via set_global_default"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("test message from spawned task")),
            "the event message must be captured; got: {lines:?}"
        );
    }

    // ── U13: verbose protocol events are gated on Verbose ──
    //
    // The tracing bridge output (format_verbose_protocol) must be active
    // only at Verbose. At Normal and Quiet, the verbosity guard returns
    // None. Removing the guard makes the Normal/Quiet assertions fail.

    #[test]
    fn test_verbose_only_enables_tracing_bridge() {
        let normal_log = log_with(Verbosity::Normal, false);
        assert!(
            normal_log
                .format_verbose_protocol("test event", Duration::ZERO)
                .is_none(),
            "verbose protocol events must be suppressed at Normal"
        );

        let quiet_log = log_with(Verbosity::Quiet, false);
        assert!(
            quiet_log
                .format_verbose_protocol("test event", Duration::ZERO)
                .is_none(),
            "verbose protocol events must be suppressed at Quiet"
        );

        let verbose_log = log_with(Verbosity::Verbose, false);
        assert!(
            verbose_log
                .format_verbose_protocol("test event", Duration::ZERO)
                .is_some(),
            "verbose protocol events must be emitted at Verbose"
        );
    }

    // ── U14: at Verbose, a proxy-client tracing event becomes a session-log event ──

    #[test]
    fn test_tracing_bridge_formats_event_as_session_log_line() {
        use std::sync::{Arc, Mutex};

        let collected: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let collected_for_layer = collected.clone();

        let layer = TracingBridgeLayer::new(move |line: String| {
            collected_for_layer.lock().unwrap().push(line);
        });

        use tracing_subscriber::layer::SubscriberExt;
        let subscriber = tracing_subscriber::Registry::default().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::info!("SESSION_INIT sent");

        let lines = collected.lock().unwrap();
        assert!(
            lines.iter().any(|l| l.contains("SESSION_INIT sent")),
            "a tracing event must be formatted as a session log line; got: {lines:?}"
        );
    }
}
