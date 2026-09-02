//! Debug-info fetch, precondition evaluation, and session orchestration for
//! `ags extend remote-debug connect`.
//!
//! Step 2 dispatches `csm/admin/debug/v4/get` through the standard runtime path.
//! Steps 3-6 evaluate four preconditions, three permanent and one retriable.
//! Steps 8-18 run the embedded tunnel, agent, and forwarder until Ctrl-C.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ags_protocol::catalogue::{OperationId, ServiceId};
use ags_protocol::error::RuntimeErrorKind;
use ags_protocol::event::{ProgressEvent, ProgressSink};
use ags_protocol::output::CommandOutput;
use ags_protocol::request::{CommandRequest, OutputFormat, PaginationHint, Verbosity};
use tokio_util::sync::CancellationToken;

use crate::invocation::handlers::extend::tunnel::bridge::{run_tunnel, TunnelConfig, TunnelError};
use extend_proxy_client::client::{Agent, AgentError, Config as AgentConfig, TokenProvider};
use extend_proxy_client::forwarder::{self, Config as ForwarderConfig, ServiceSpec};

/// Operation identifier for the CSM v4 debug info endpoint.
const DEBUG_INFO_OPERATION: &str = "csm/admin/debug/v4/get";

/// Debug info fields from the CSM v4 `GET .../debuginfo` response.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DebugInfo {
    pub app_status: Option<String>,
    pub is_debug_mode_enabled: Option<bool>,
    pub is_debug_session_connected: Option<bool>,
    pub debug_pods: Option<Vec<serde_json::Value>>,
    pub exposed_services: Option<Vec<serde_json::Value>>,
    pub allowed_intercepted_ports: Option<Vec<serde_json::Value>>,
}

/// A precondition that will never pass regardless of how many times we retry.
#[derive(Debug, Clone)]
pub(crate) struct PermanentPrecondition(pub String);

/// Error from a single connect attempt.
#[derive(Debug, Clone)]
pub(crate) enum ConnectError {
    /// Credentials are missing or no longer valid after refresh.
    Authentication(String),
    /// Ordered component shutdown exceeded its deadline.
    ShutdownTimeout(String),
    /// A precondition that cannot change (e.g. debug mode not enabled).
    Permanent(PermanentPrecondition),
    /// A transient condition (HTTP error, empty pods) that may resolve.
    Retriable(String),
}

/// Preserve authentication failures across `TokenProvider`'s string-only
/// boundary and combine them with auth-specific agent failures.
fn agent_authentication_message(
    error: Option<&AgentError>,
    token_failure: Option<String>,
) -> Option<String> {
    token_failure.or_else(|| match error {
        Some(error @ AgentError::DialRejected { status: 401, .. }) => Some(format!(
            "authentication failed while opening the debug session: {}; run `ags auth login` and retry",
            error
        )),
        _ => None,
    })
}

fn classify_agent_error(
    error: &AgentError,
    token_failure: Option<String>,
    context: &str,
) -> ConnectError {
    match agent_authentication_message(Some(error), token_failure) {
        Some(message) => ConnectError::Authentication(message),
        None => ConnectError::Retriable(format!("{context}: {error}")),
    }
}

/// No-op progress sink for internal API dispatch. The debug-info call
/// is a background side-fetch; its progress events are not surfaced.
struct SilentSink;

impl ProgressSink for SilentSink {
    fn on_event(&mut self, _event: ProgressEvent) {}
}

/// Dispatch the debug-info call and evaluate preconditions.
///
/// Returns `Ok(DebugInfo)` when all preconditions pass, or an error
/// indicating whether the failure is permanent or retriable.
pub(super) async fn connect_once(
    runtime: &mut ags_runtime::runtime::Runtime,
    namespace: &str,
    app: &str,
    _local_grpc_addr: &str,
    _local_http_addr: &str,
    _profile: &str,
) -> Result<DebugInfo, ConnectError> {
    // Step 2: resolve debug info via the standard runtime dispatch.
    let debug_info = fetch_debug_info(runtime, namespace, app).await?;

    // Steps 3-6: evaluate preconditions.
    evaluate_preconditions(&debug_info)?;

    Ok(debug_info)
}

/// Dispatch `csm/admin/debug/v4/get` through the standard runtime path
/// and parse the response into a `DebugInfo`.
async fn fetch_debug_info(
    runtime: &mut ags_runtime::runtime::Runtime,
    namespace: &str,
    app: &str,
) -> Result<DebugInfo, ConnectError> {
    let mut path_params = BTreeMap::new();
    path_params.insert("namespace".to_string(), namespace.to_string());
    path_params.insert("app".to_string(), app.to_string());

    let request = CommandRequest {
        service: ServiceId::new("csm"),
        operation_id: OperationId::new(DEBUG_INFO_OPERATION),
        namespace: Some(namespace.to_string()),
        path_params,
        query_params: BTreeMap::new(),
        header_params: BTreeMap::new(),
        form_params: BTreeMap::new(),
        body: None,
        output_format: OutputFormat::Json,
        pagination: PaginationHint::Auto,
        verbosity: Verbosity::Quiet,
        output: None,
    };

    let mut sink = SilentSink;
    let output = runtime
        .run_command(&request, &mut sink)
        .await
        .map_err(|e| match e.kind {
            RuntimeErrorKind::NotAuthenticated => ConnectError::Authentication(e.message),
            RuntimeErrorKind::Forbidden => {
                ConnectError::Permanent(PermanentPrecondition(e.message))
            }
            _ => ConnectError::Retriable(e.message),
        })?;

    // Extract the raw JSON body from the API output.
    let raw_body = match output {
        CommandOutput::Service(api_output) => api_output.raw_body,
        _ => None,
    };

    let body = raw_body.ok_or_else(|| {
        ConnectError::Retriable("debug-info response contained no JSON body".to_string())
    })?;

    serde_json::from_value::<DebugInfo>(body)
        .map_err(|e| ConnectError::Retriable(format!("failed to parse debug-info response: {e}")))
}

/// Evaluate the four preconditions against resolved debug info.
///
/// Three are permanent (can never pass by retrying) and one is retriable
/// (empty pod list). The order matches the specification table:
///
/// 1. `appStatus` must be `"deployment-running"` (permanent)
/// 2. `isDebugModeEnabled` must be `true` (permanent)
/// 3. `isDebugSessionConnected` must be `false` (permanent)
/// 4. `debugPods` must be non-empty (retriable)
pub(super) fn evaluate_preconditions(info: &DebugInfo) -> Result<(), ConnectError> {
    // Step 3: appStatus must be "deployment-running".
    match info.app_status.as_deref() {
        Some("deployment-running") => {}
        Some(other) => {
            return Err(ConnectError::Permanent(PermanentPrecondition(format!(
                "app status is '{other}', expected 'deployment-running'"
            ))));
        }
        None => {
            return Err(ConnectError::Permanent(PermanentPrecondition(
                "app status is missing from debug-info response".to_string(),
            )));
        }
    }

    // Step 4: isDebugModeEnabled must be true.
    match info.is_debug_mode_enabled {
        Some(true) => {}
        Some(false) => {
            return Err(ConnectError::Permanent(PermanentPrecondition(
                "debug mode is not enabled — run 'ags extend remote-debug enable' first"
                    .to_string(),
            )));
        }
        None => {
            return Err(ConnectError::Permanent(PermanentPrecondition(
                "isDebugModeEnabled is missing from debug-info response".to_string(),
            )));
        }
    }

    // Step 5: isDebugSessionConnected must be false.
    match info.is_debug_session_connected {
        Some(false) | None => {}
        Some(true) => {
            return Err(ConnectError::Permanent(PermanentPrecondition(
                "another debug session is already connected".to_string(),
            )));
        }
    }

    // Step 6: debugPods must not be empty.
    let pods = info.debug_pods.as_deref().unwrap_or(&[]);
    if pods.is_empty() {
        return Err(ConnectError::Retriable(
            "no debug pods are available yet".to_string(),
        ));
    }

    Ok(())
}

// ── Orchestration types and pure helpers (steps 8-18) ──

/// Parsed debug pod entry selected for the session.
#[derive(Debug, Clone)]
pub(super) struct SelectedPod {
    pub name: String,
    pub port: u16,
}

/// Fixed session-init timeout (five seconds), matching the tool being replaced.
/// Deliberately fixed as a parity choice with extend-helper-cli; exposing it
/// as a CLI flag stays a one-site change because the agent's Config field is
/// already parameterised.
pub(super) const SESSION_INIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Per-component shutdown timeout during deferred cleanup.
pub(super) const COMPONENT_SHUTDOWN_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(10);

/// The embedded tunnel is always called with quiet=true so it writes no
/// ready line of its own; `connect_once` owns the ready emission.
pub(super) const EMBEDDED_TUNNEL_QUIET: bool = true;

/// The resource name the embedded tunnel sends to the CSM tunnel endpoint.
/// The Go tool being replaced uses the literal `"remote-debug"` — the pod
/// name is passed separately via the `podName` query parameter. The
/// standalone `ags extend tunnel` uses the user's own `--resource-name`
/// argument, which is a different code path and remains unchanged.
pub(super) const EMBEDDED_TUNNEL_RESOURCE_NAME: &str = "remote-debug";

/// Select the first debug pod from the precondition-validated list.
/// The list is guaranteed non-empty by `evaluate_preconditions`, but this
/// function validates that `podName` and `port` exist and are parseable
/// (the CSM OpenAPI spec defines `domain.DebugPod` with required
/// properties `podName` (string) and `port` (integer)).
pub(super) fn select_pod(pods: &[serde_json::Value]) -> Result<SelectedPod, ConnectError> {
    let pod = pods
        .first()
        .ok_or_else(|| ConnectError::Retriable("no debug pods available".to_string()))?;
    let name = pod
        .get("podName")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ConnectError::Retriable("debug pod has no 'podName' field".to_string()))?
        .to_string();
    let port = pod
        .get("port")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ConnectError::Retriable("debug pod has no 'port' field".to_string()))?
        as u16;
    Ok(SelectedPod { name, port })
}

/// Build the port mappings for the agent from `allowedInterceptedPorts`.
///
/// Port 6565 maps to `local_grpc_addr`, port 8000 maps to `local_http_addr`,
/// and every other port falls back to `localhost:<port>`.
pub(super) fn build_allow_ports(
    intercepted_ports: &[serde_json::Value],
    local_grpc_addr: &str,
    local_http_addr: &str,
) -> Vec<extend_proxy_client::client::PortMapping> {
    intercepted_ports
        .iter()
        .filter_map(|entry| {
            let port = entry.get("port")?.as_i64()? as i32;
            let local_addr = match port {
                6565 => local_grpc_addr.to_string(),
                8000 => local_http_addr.to_string(),
                other => format!("localhost:{other}"),
            };
            Some(extend_proxy_client::client::PortMapping {
                target_port: port,
                local_addr,
            })
        })
        .collect()
}

/// A parsed entry from the `exposedServices` array. The CSM OpenAPI spec
/// defines `domain.ExposedService` with three required properties: `host`
/// (string), `name` (string), and `port` (integer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExposedService {
    pub name: String,
    pub host: String,
    pub port: u16,
}

/// Parse exposed services from the debug info response for the forwarder.
/// Each entry must have `name` (string), `host` (string), and `port`
/// (number). Entries missing any required field are skipped, matching the
/// existing skip-malformed behaviour.
pub(super) fn parse_exposed_services(services: &[serde_json::Value]) -> Vec<ExposedService> {
    services
        .iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.to_string();
            let host = entry.get("host")?.as_str()?.to_string();
            let port = entry.get("port")?.as_u64()? as u16;
            Some(ExposedService { name, host, port })
        })
        .collect()
}

/// Why a successfully established session stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionOutcome {
    /// The remote side ended the session; the caller should reconnect.
    Ended,
    /// The command-wide cancellation token fired; the caller should exit cleanly.
    Cancelled,
}

/// Await a single spawned component handle with the shutdown timeout.
/// Returns immediately when the handle is `None` (component was never started).
pub(super) async fn await_component<T>(
    handle: Option<tokio::task::JoinHandle<T>>,
    timeout: std::time::Duration,
) -> bool {
    if let Some(mut handle) = handle {
        if tokio::time::timeout(timeout, &mut handle).await.is_err() {
            handle.abort();
            return false;
        }
    }
    true
}

/// Observe a component during the established session without completing
/// when that optional component was never started.
async fn observe_component<T>(
    handle: &mut Option<tokio::task::JoinHandle<T>>,
) -> Option<Result<T, tokio::task::JoinError>> {
    match handle {
        Some(handle) => Some(handle.await),
        None => std::future::pending().await,
    }
}

/// Create cancellation scoped to one connection attempt.
fn attempt_cancellation(outer_cancel: &CancellationToken) -> CancellationToken {
    outer_cancel.child_token()
}

/// Classify a tunnel task exit so the retry warning carries the real failure
/// (bind error, auth failure) instead of a generic agent-side disconnect.
pub(super) fn describe_tunnel_exit(
    result: Result<Result<(), TunnelError>, tokio::task::JoinError>,
) -> String {
    match result {
        Ok(Ok(())) => "tunnel exited unexpectedly".to_string(),
        Ok(Err(e)) => e.to_string(),
        Err(join_err) => format!("tunnel task failed: {join_err}"),
    }
}

/// Classify a forwarder task exit while the debug session is established.
pub(super) fn describe_forwarder_exit(
    result: Result<Result<(), std::io::Error>, tokio::task::JoinError>,
) -> String {
    match result {
        Ok(Ok(())) => "service forwarder exited unexpectedly".to_string(),
        Ok(Err(error)) => format!("service forwarder failed: {error}"),
        Err(join_error) => format!("service forwarder task failed: {join_error}"),
    }
}

/// Build forwarder specifications when the deployment exposes services.
/// Sets `remote_host` from the service's `host` field (the address the
/// forwarder dials) and `name` as the display label, matching the Go
/// original which uses `svc.Host` for the remote and `svc.Name` for the
/// display name.
fn build_forwarder_services(services: &[ExposedService]) -> Option<Vec<ServiceSpec>> {
    if services.is_empty() {
        return None;
    }
    Some(
        services
            .iter()
            .map(|svc| ServiceSpec {
                name: svc.name.clone(),
                remote_host: svc.host.clone(),
                ports: vec![svc.port],
            })
            .collect(),
    )
}

/// Cohesive identity and addressing inputs for a debug session: the
/// namespace/app pair, the local forward addresses, the auth profile, and
/// the base-URL host the tunnel dials. Grouped so `run_session` receives
/// one unit instead of a flat argument list.
pub(super) struct SessionParams<'a> {
    pub namespace: &'a str,
    pub local_grpc_addr: &'a str,
    pub local_http_addr: &'a str,
    pub profile: &'a str,
    pub base_url_host: &'a str,
}

/// Forward to `run_tunnel` for the embedded (in-session) tunnel.
///
/// Exists so tests can observe the `quiet` value actually delivered to the
/// bridge: the recorded value and the forwarded argument are the same
/// binding, so a call-site change cannot bypass the probe.
async fn run_embedded_tunnel(
    cfg: TunnelConfig,
    listener_ready: tokio::sync::oneshot::Sender<()>,
    cancel: CancellationToken,
    profile: Option<&str>,
    quiet: bool,
) -> Result<(), TunnelError> {
    #[cfg(test)]
    tests::record_embedded_tunnel_quiet(quiet);
    #[cfg(test)]
    tests::record_embedded_tunnel_resource_name(&cfg.resource_name);
    // The embedded tunnel uses Quiet verbosity when quiet=true, and
    // format_json=false: with `listener_ready` set the bridge writes no
    // ready line at all, so the JSON flag never applies on this path.
    let verbosity = if quiet {
        ags_protocol::request::Verbosity::Quiet
    } else {
        ags_protocol::request::Verbosity::Normal
    };
    let session_log =
        crate::invocation::handlers::extend::session_log::SessionLog::new(verbosity, false);
    run_tunnel(cfg, Some(listener_ready), cancel, profile, session_log).await
}

/// Run the orchestration: tunnel, agent, and forwarder.
///
/// Called after preconditions pass. Starts the three components, emits
/// lifecycle events through the session log, and blocks until the session
/// ends (agent done) or the outer cancel fires. Cleans up all components
/// on every return path.
pub(super) async fn run_session(
    debug_info: &DebugInfo,
    params: &SessionParams<'_>,
    session_log: crate::invocation::handlers::extend::session_log::SessionLog,
    outer_cancel: CancellationToken,
) -> Result<SessionOutcome, ConnectError> {
    let SessionParams {
        namespace,
        local_grpc_addr,
        local_http_addr,
        profile,
        base_url_host,
    } = *params;

    // ── Extract data from debug info ──

    let pods = debug_info.debug_pods.as_deref().unwrap_or(&[]);
    let pod = select_pod(pods)?;

    // Emit the connecting event now that the pod is known.
    session_log.connecting(&pod.name, pod.port);

    let intercepted = debug_info
        .allowed_intercepted_ports
        .as_deref()
        .unwrap_or(&[]);
    let allow_ports = build_allow_ports(intercepted, local_grpc_addr, local_http_addr);

    let exposed = debug_info.exposed_services.as_deref().unwrap_or(&[]);
    let services = parse_exposed_services(exposed);

    // Per-attempt cancellation: fired BEFORE awaiting components during
    // shutdown so the await calls terminate rather than blocking for the
    // full component timeout.
    let attempt_cancel = attempt_cancellation(&outer_cancel);

    // ── Start the bridge (tunnel) ──

    let (listener_ready_tx, listener_ready_rx) = tokio::sync::oneshot::channel();

    let tunnel_cfg = TunnelConfig {
        host: base_url_host.to_string(),
        namespace: namespace.to_string(),
        resource_name: EMBEDDED_TUNNEL_RESOURCE_NAME.to_string(),
        local_port: pod.port,
        pod_name: Some(pod.name.clone()),
    };

    let tunnel_cancel = attempt_cancel.clone();
    let profile_for_tunnel = profile.to_string();
    let mut tunnel_handle = Some(tokio::spawn(async move {
        run_embedded_tunnel(
            tunnel_cfg,
            listener_ready_tx,
            tunnel_cancel,
            Some(profile_for_tunnel.as_str()),
            EMBEDDED_TUNNEL_QUIET,
        )
        .await
    }));

    // Wait for the tunnel's TCP listener to bind. When the tunnel task
    // finishes before firing ready, classify its exit so the retriable
    // error carries the real failure.
    let mut tunnel_exit: Option<String> = None;
    let tunnel_ready = tokio::select! {
        result = listener_ready_rx => result.is_ok(),
        result = async { tunnel_handle.as_mut().unwrap().await } => {
            tunnel_handle = None;
            tunnel_exit = Some(describe_tunnel_exit(result));
            false
        },
        _ = attempt_cancel.cancelled() => false,
    };

    if !tunnel_ready {
        attempt_cancel.cancel();
        if !await_component(tunnel_handle, COMPONENT_SHUTDOWN_TIMEOUT).await {
            return Err(ConnectError::ShutdownTimeout(
                "timed out waiting for tunnel to stop".to_string(),
            ));
        }
        if outer_cancel.is_cancelled() {
            return Ok(SessionOutcome::Cancelled);
        }
        let reason = match tunnel_exit {
            Some(detail) => format!("tunnel failed before becoming ready: {detail}"),
            None => "tunnel failed before becoming ready".to_string(),
        };
        return Err(ConnectError::Retriable(reason));
    }

    // Tunnel listener is ready on pod.port.

    // ── Start the agent ──

    // Dial the exact address the tunnel bound (`build_bind_addr` uses
    // 127.0.0.1). Dialing `localhost` instead risks a dual-stack detour:
    // on Windows the resolver offers ::1 first, and the failed IPv6
    // connect adds seconds before the IPv4 attempt reaches the listener.
    let sidecar_ws = format!(
        "ws://127.0.0.1:{}/tunnel?gameNamespace={}",
        pod.port, namespace,
    );

    let (session_ready_tx, mut session_ready_rx) = tokio::sync::mpsc::channel(1);

    let token_failure = Arc::new(Mutex::new(None));
    let token_failure_for_provider = token_failure.clone();
    let profile_for_token = profile.to_string();
    let token_provider: TokenProvider = Arc::new(move |_cancel: CancellationToken| {
        let profile = profile_for_token.clone();
        let token_failure = token_failure_for_provider.clone();
        Box::pin(async move {
            match ags_runtime::runtime::auth::store::get_token_data_async(&profile).await {
                Ok(Some(data)) if !data.access_token.is_empty() => data.access_token.clone(),
                Ok(Some(_)) => {
                    *token_failure.lock().expect("token-failure lock") = Some(format!(
                        "the stored access token for profile '{profile}' is empty; run `ags auth login` and retry"
                    ));
                    String::new()
                }
                Ok(None) => {
                    *token_failure.lock().expect("token-failure lock") = Some(format!(
                        "no authentication token is available for profile '{profile}'; run `ags auth login` and retry"
                    ));
                    String::new()
                }
                Err(error) => {
                    *token_failure.lock().expect("token-failure lock") = Some(format!(
                        "failed to load the authentication token for profile '{profile}': {error}"
                    ));
                    String::new()
                }
            }
        })
    });

    let agent_cfg = AgentConfig {
        sidecar_ws,
        allow_ports,
        session_init_timeout: SESSION_INIT_TIMEOUT,
        token_provider: Some(token_provider),
        session_ready_tx: Some(session_ready_tx),
    };

    let agent = Agent::new(agent_cfg);
    let sessions = agent.sessions();
    let agent_cancel = attempt_cancel.clone();
    let mut agent_handle = Some(tokio::spawn(async move { agent.run(agent_cancel).await }));

    // Wait for the agent's session to be established.
    let mut agent_start_result = None;
    let session_ok = tokio::select! {
        session = session_ready_rx.recv() => session.is_some(),
        result = async { agent_handle.as_mut().unwrap().await } => {
            agent_handle = None;
            agent_start_result = Some(result);
            false
        },
        _ = attempt_cancel.cancelled() => false,
    };

    if !session_ok {
        attempt_cancel.cancel();
        let agent_stopped = await_component(agent_handle, COMPONENT_SHUTDOWN_TIMEOUT).await;
        let tunnel_stopped = await_component(tunnel_handle, COMPONENT_SHUTDOWN_TIMEOUT).await;
        if !agent_stopped {
            return Err(ConnectError::ShutdownTimeout(
                "timed out waiting for agent to stop".to_string(),
            ));
        }
        if !tunnel_stopped {
            return Err(ConnectError::ShutdownTimeout(
                "timed out waiting for tunnel to stop".to_string(),
            ));
        }
        if outer_cancel.is_cancelled() {
            return Ok(SessionOutcome::Cancelled);
        }
        let token_failure = token_failure.lock().expect("token-failure lock").clone();
        let agent_error = match agent_start_result.as_ref() {
            Some(Ok(Err(error))) => Some(error),
            _ => None,
        };
        if let Some(error) = agent_error {
            return Err(classify_agent_error(
                error,
                token_failure,
                "agent failed before session was established",
            ));
        }
        if let Some(message) = token_failure {
            return Err(ConnectError::Authentication(message));
        }
        let message = match agent_start_result {
            Some(Err(error)) => format!("debug agent task failed during startup: {error}"),
            _ => "agent failed before session was established".to_string(),
        };
        return Err(ConnectError::Retriable(message));
    }

    // Agent session established.

    // ── Start the forwarder ──

    // `forwarder::run` rejects an empty service list. Empty exposed services
    // mean there is simply no optional remote service to forward, so skip the
    // component instead of starting a task that is guaranteed to fail.
    let mut forwarder_handle = if let Some(forwarder_services) = build_forwarder_services(&services)
    {
        let get_stream_opener: extend_proxy_client::forwarder::GetStreamOpener =
            Arc::new(move || {
                sessions
                    .current()
                    .map(|s| s as Arc<dyn extend_proxy_client::forwarder::StreamOpener>)
            });
        let forwarder_cfg = ForwarderConfig {
            services: forwarder_services,
            get_stream_opener,
        };
        let forwarder_cancel = attempt_cancel.clone();
        Some(tokio::spawn(async move {
            forwarder::run(forwarder_cfg, forwarder_cancel).await
        }))
    } else {
        None
    };

    // Emit service_listening for each forwarder service.
    for svc in &services {
        session_log.service_listening(&svc.name, &format!("localhost:{}", svc.port));
    }

    // ── Emit connected event ──

    session_log.connected(local_grpc_addr, local_http_addr);

    // ── Block until session ends or outer cancel fires ──

    let session_outcome = tokio::select! {
        // Ctrl-C is authoritative when cancellation and a component exit are
        // both ready in the same scheduler tick.
        biased;
        _ = outer_cancel.cancelled() => Ok(SessionOutcome::Cancelled),
        result = observe_component(&mut tunnel_handle) => {
            tunnel_handle = None;
            let detail = result
                .map(describe_tunnel_exit)
                .unwrap_or_else(|| "tunnel stopped unexpectedly".to_string());
            Err(ConnectError::Retriable(format!("debug tunnel ended: {detail}")))
        },
        result = observe_component(&mut forwarder_handle) => {
            forwarder_handle = None;
            let detail = result
                .map(describe_forwarder_exit)
                .unwrap_or_else(|| "service forwarder stopped unexpectedly".to_string());
            Err(ConnectError::Retriable(detail))
        },
        result = observe_component(&mut agent_handle) => {
            agent_handle = None;
            match result {
                Some(Ok(Ok(()))) => Ok(SessionOutcome::Ended),
                Some(Ok(Err(error))) => {
                    let token_failure = token_failure.lock().expect("token-failure lock").clone();
                    Err(classify_agent_error(
                        &error,
                        token_failure,
                        "debug session ended with an error",
                    ))
                }
                Some(Err(error)) => Err(ConnectError::Retriable(format!(
                    "debug agent task failed: {error}"
                ))),
                None => Ok(SessionOutcome::Ended),
            }
        },
    };

    // Emit the session_ended event with the reason for stopping.
    let session_end_reason = match &session_outcome {
        Ok(SessionOutcome::Cancelled) => "user cancelled",
        Ok(SessionOutcome::Ended) => "remote session ended",
        Err(ConnectError::Authentication(msg)) => msg.as_str(),
        Err(ConnectError::ShutdownTimeout(msg)) => msg.as_str(),
        Err(ConnectError::Permanent(PermanentPrecondition(msg))) => msg.as_str(),
        Err(ConnectError::Retriable(msg)) => msg.as_str(),
    };
    session_log.session_ended(session_end_reason);

    // ── Shutdown: cancel FIRST, then await each component ──
    // Cancelling before awaiting is what makes the awaits terminate;
    // awaiting first would deadlock if the component blocks on the token.

    attempt_cancel.cancel();
    let agent_stopped = await_component(agent_handle, COMPONENT_SHUTDOWN_TIMEOUT).await;
    let tunnel_stopped = await_component(tunnel_handle, COMPONENT_SHUTDOWN_TIMEOUT).await;
    let forwarder_stopped = await_component(forwarder_handle, COMPONENT_SHUTDOWN_TIMEOUT).await;

    let timed_out_component = if !agent_stopped {
        Some("agent")
    } else if !tunnel_stopped {
        Some("tunnel")
    } else if !forwarder_stopped {
        Some("forwarder")
    } else {
        None
    };
    if let Some(component) = timed_out_component {
        return Err(ConnectError::ShutdownTimeout(format!(
            "timed out waiting for {component} to stop"
        )));
    }

    session_outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    // ── RAII env guard ──

    /// RAII guard that restores an environment variable after a test mutates it.
    // Env-mutating tests must be #[serial_test::serial] per repo convention.
    // No crate-visible TempEnvGuard exists for in-source test modules; this
    // mirrors the bridge.rs pattern.
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

    /// Isolate the auth environment for `run_session` tests. The embedded
    /// tunnel and the agent both resolve tokens; without isolation the tests
    /// sample the developer's real auth store and keychain, which is slow
    /// and nondeterministic. `AGS_ACCESS_TOKEN` short-circuits token
    /// resolution before any storage or network access.
    fn isolated_auth_env(tmp: &tempfile::TempDir) -> [TempEnvGuard; 3] {
        [
            TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap()),
            TempEnvGuard::set("AGS_NO_KEYCHAIN", "1"),
            TempEnvGuard::set("AGS_ACCESS_TOKEN", "fake-test-token"),
        ]
    }

    /// Upstream endpoint that accepts each TCP connection and immediately
    /// drops it, so the tunnel's upstream TLS handshake fails instantly.
    /// (A freed/closed port is NOT fast on Windows loopback: a refused
    /// connect takes ~2 s, which blows prompt-shutdown budgets.)
    async fn spawn_accept_then_drop_upstream() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind accept-then-drop upstream");
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                // Accept and drop immediately — the peer sees an instant
                // connection reset instead of a slow refused connect.
                let _ = listener.accept().await;
            }
        });
        format!("127.0.0.1:{}", addr.port())
    }

    /// Build a quiet session log for tests. Output is suppressed, matching
    /// the pre-existing test convention that passes quiet=true.
    fn test_session_log() -> crate::invocation::handlers::extend::session_log::SessionLog {
        crate::invocation::handlers::extend::session_log::SessionLog::new(
            ags_protocol::request::Verbosity::Quiet,
            false,
        )
    }

    /// Build a `DebugInfo` with all preconditions passing.
    fn good_debug_info() -> DebugInfo {
        DebugInfo {
            app_status: Some("deployment-running".to_string()),
            is_debug_mode_enabled: Some(true),
            is_debug_session_connected: Some(false),
            debug_pods: Some(vec![serde_json::json!({"podName": "pod-1", "port": 15080})]),
            exposed_services: Some(vec![]),
            allowed_intercepted_ports: Some(vec![]),
        }
    }

    #[test]
    fn agent_unauthorized_dial_is_authentication_failure() {
        let error = AgentError::DialRejected {
            status: 401,
            message: Some("token expired".to_string()),
        };

        match classify_agent_error(&error, None, "agent startup failed") {
            ConnectError::Authentication(message) => {
                assert!(message.contains("authentication failed"));
                assert!(message.contains("ags auth login"));
            }
            other => panic!("HTTP 401 must use the auth exit path; got {other:?}"),
        }
    }

    #[test]
    fn token_provider_failure_is_preserved_across_string_only_boundary() {
        let agent_error = AgentError::InvalidRequest("unrelated agent error".to_string());
        let error = classify_agent_error(
            &agent_error,
            Some("failed to load the authentication token".to_string()),
            "agent startup failed",
        );

        match error {
            ConnectError::Authentication(message) => {
                assert_eq!(message, "failed to load the authentication token");
            }
            other => panic!("token-provider failure must use the auth exit path; got {other:?}"),
        }
    }

    #[test]
    fn non_auth_agent_failure_remains_retriable() {
        let error = AgentError::DialRejected {
            status: 409,
            message: Some("session already active".to_string()),
        };

        assert!(matches!(
            classify_agent_error(&error, None, "agent startup failed"),
            ConnectError::Retriable(_)
        ));
    }

    // ── Precondition: all pass ──

    #[test]
    fn all_preconditions_pass() {
        let info = good_debug_info();
        assert!(
            evaluate_preconditions(&info).is_ok(),
            "all preconditions should pass for a well-formed debug info"
        );
    }

    // ── Precondition: appStatus ──

    #[test]
    fn app_status_not_running_is_permanent() {
        let info = DebugInfo {
            app_status: Some("stopped".to_string()),
            ..good_debug_info()
        };
        match evaluate_preconditions(&info) {
            Err(ConnectError::Permanent(PermanentPrecondition(msg))) => {
                assert!(
                    msg.contains("stopped"),
                    "error must name the actual status; got: {msg}"
                );
                assert!(
                    msg.contains("deployment-running"),
                    "error must name the expected status; got: {msg}"
                );
            }
            other => panic!(
                "expected Permanent error for wrong appStatus, got: {}",
                match other {
                    Ok(()) => "Ok".to_string(),
                    Err(ConnectError::Authentication(m)) => format!("Authentication({m})"),
                    Err(ConnectError::ShutdownTimeout(m)) => format!("ShutdownTimeout({m})"),
                    Err(ConnectError::Retriable(m)) => format!("Retriable({m})"),
                    Err(ConnectError::Permanent(_)) => unreachable!(),
                }
            ),
        }
    }

    #[test]
    fn app_status_missing_is_permanent() {
        let info = DebugInfo {
            app_status: None,
            ..good_debug_info()
        };
        match evaluate_preconditions(&info) {
            Err(ConnectError::Permanent(_)) => {}
            other => panic!(
                "expected Permanent for missing appStatus, got: {}",
                match other {
                    Ok(()) => "Ok".to_string(),
                    Err(ConnectError::Authentication(m)) => format!("Authentication({m})"),
                    Err(ConnectError::ShutdownTimeout(m)) => format!("ShutdownTimeout({m})"),
                    Err(ConnectError::Retriable(m)) => format!("Retriable({m})"),
                    Err(ConnectError::Permanent(_)) => unreachable!(),
                }
            ),
        }
    }

    // ── Precondition: isDebugModeEnabled ──

    #[test]
    fn debug_mode_disabled_is_permanent() {
        let info = DebugInfo {
            is_debug_mode_enabled: Some(false),
            ..good_debug_info()
        };
        match evaluate_preconditions(&info) {
            Err(ConnectError::Permanent(PermanentPrecondition(msg))) => {
                assert!(
                    msg.contains("not enabled"),
                    "error must say debug mode is not enabled; got: {msg}"
                );
            }
            other => panic!(
                "expected Permanent for disabled debug mode, got: {}",
                match other {
                    Ok(()) => "Ok".to_string(),
                    Err(ConnectError::Authentication(m)) => format!("Authentication({m})"),
                    Err(ConnectError::ShutdownTimeout(m)) => format!("ShutdownTimeout({m})"),
                    Err(ConnectError::Retriable(m)) => format!("Retriable({m})"),
                    Err(ConnectError::Permanent(_)) => unreachable!(),
                }
            ),
        }
    }

    #[test]
    fn debug_mode_missing_is_permanent() {
        let info = DebugInfo {
            is_debug_mode_enabled: None,
            ..good_debug_info()
        };
        match evaluate_preconditions(&info) {
            Err(ConnectError::Permanent(_)) => {}
            _ => panic!("expected Permanent for missing isDebugModeEnabled"),
        }
    }

    // ── Precondition: isDebugSessionConnected ──

    #[test]
    fn session_already_connected_is_permanent() {
        let info = DebugInfo {
            is_debug_session_connected: Some(true),
            ..good_debug_info()
        };
        match evaluate_preconditions(&info) {
            Err(ConnectError::Permanent(PermanentPrecondition(msg))) => {
                assert!(
                    msg.contains("already connected"),
                    "error must mention existing session; got: {msg}"
                );
            }
            other => panic!(
                "expected Permanent for connected session, got: {}",
                match other {
                    Ok(()) => "Ok".to_string(),
                    Err(ConnectError::Authentication(m)) => format!("Authentication({m})"),
                    Err(ConnectError::ShutdownTimeout(m)) => format!("ShutdownTimeout({m})"),
                    Err(ConnectError::Retriable(m)) => format!("Retriable({m})"),
                    Err(ConnectError::Permanent(_)) => unreachable!(),
                }
            ),
        }
    }

    #[test]
    fn session_not_connected_passes() {
        // Explicit false is the happy path.
        let info = DebugInfo {
            is_debug_session_connected: Some(false),
            ..good_debug_info()
        };
        assert!(evaluate_preconditions(&info).is_ok());
    }

    #[test]
    fn session_connected_missing_passes() {
        // Missing field is treated as "not connected" (conservative default).
        let info = DebugInfo {
            is_debug_session_connected: None,
            ..good_debug_info()
        };
        assert!(evaluate_preconditions(&info).is_ok());
    }

    // ── Precondition: debugPods empty ──

    #[test]
    fn empty_pods_is_retriable() {
        let info = DebugInfo {
            debug_pods: Some(vec![]),
            ..good_debug_info()
        };
        match evaluate_preconditions(&info) {
            Err(ConnectError::Retriable(msg)) => {
                assert!(
                    msg.contains("no debug pods"),
                    "error must mention empty pods; got: {msg}"
                );
            }
            other => panic!(
                "expected Retriable for empty pods, got: {}",
                match other {
                    Ok(()) => "Ok".to_string(),
                    Err(ConnectError::Authentication(m)) => format!("Authentication({m})"),
                    Err(ConnectError::ShutdownTimeout(m)) => format!("ShutdownTimeout({m})"),
                    Err(ConnectError::Permanent(PermanentPrecondition(m))) =>
                        format!("Permanent({m})"),
                    Err(ConnectError::Retriable(_)) => unreachable!(),
                }
            ),
        }
    }

    #[test]
    fn missing_pods_is_retriable() {
        let info = DebugInfo {
            debug_pods: None,
            ..good_debug_info()
        };
        match evaluate_preconditions(&info) {
            Err(ConnectError::Retriable(_)) => {}
            _ => panic!("expected Retriable for missing debugPods"),
        }
    }

    // ── Parse: debug info from JSON ──

    #[test]
    fn parse_debug_info_from_valid_json() {
        let json = serde_json::json!({
            "appStatus": "deployment-running",
            "isDebugModeEnabled": true,
            "isDebugSessionConnected": false,
            "debugPods": [{"podName": "pod-abc", "port": 15080}],
            "exposedServices": [],
            "allowedInterceptedPorts": []
        });
        let info: DebugInfo = serde_json::from_value(json).expect("valid JSON must parse");
        assert_eq!(info.app_status.as_deref(), Some("deployment-running"));
        assert_eq!(info.is_debug_mode_enabled, Some(true));
        assert_eq!(info.is_debug_session_connected, Some(false));
        assert_eq!(info.debug_pods.as_ref().map(|p| p.len()), Some(1));
    }

    #[test]
    fn parse_debug_info_with_missing_optional_fields() {
        // The API may omit optional fields; parse must not fail.
        let json = serde_json::json!({});
        let info: DebugInfo = serde_json::from_value(json).expect("empty object must parse");
        assert!(info.app_status.is_none());
        assert!(info.is_debug_mode_enabled.is_none());
    }

    // ── Operation ID exists in the bundled CSM spec ──

    #[test]
    fn debug_info_operation_exists_in_csm_spec() {
        use ags_runtime::catalogue::Catalogue;

        let schema = Catalogue::load_bundled("csm").expect("bundled CSM spec must load");
        let found = schema.resources.iter().any(|r| {
            r.operations()
                .any(|op| op.id.as_str() == DEBUG_INFO_OPERATION)
        });
        assert!(
            found,
            "operation '{DEBUG_INFO_OPERATION}' must exist in the bundled CSM spec"
        );
    }

    // ── Embedded tunnel resource name ──

    #[test]
    fn test_embedded_tunnel_resource_name_is_remote_debug_literal() {
        assert_eq!(
            EMBEDDED_TUNNEL_RESOURCE_NAME, "remote-debug",
            "the embedded tunnel must use the literal 'remote-debug' as its \
             resource name, matching the Go tool being replaced"
        );
    }

    // ── Orchestration pure function tests ──

    #[test]
    fn test_session_init_timeout_is_five_seconds() {
        assert_eq!(
            SESSION_INIT_TIMEOUT,
            std::time::Duration::from_secs(5),
            "session-init timeout must be five seconds (parity with extend-helper-cli)"
        );
    }

    #[test]
    fn test_component_shutdown_timeout_is_ten_seconds() {
        assert_eq!(
            COMPONENT_SHUTDOWN_TIMEOUT,
            std::time::Duration::from_secs(10),
            "per-component shutdown timeout must be ten seconds"
        );
    }

    #[test]
    fn test_allow_ports_maps_known_ports_and_fallback() {
        let ports = vec![
            serde_json::json!({"port": 6565}),
            serde_json::json!({"port": 8000}),
            serde_json::json!({"port": 9000}),
        ];
        let result = build_allow_ports(&ports, "localhost:6565", "localhost:8000");
        assert_eq!(result.len(), 3, "must produce one mapping per port");
        assert_eq!(result[0].target_port, 6565);
        assert_eq!(
            result[0].local_addr, "localhost:6565",
            "port 6565 must map to the resolved local gRPC address"
        );
        assert_eq!(result[1].target_port, 8000);
        assert_eq!(
            result[1].local_addr, "localhost:8000",
            "port 8000 must map to the resolved local HTTP address"
        );
        assert_eq!(result[2].target_port, 9000);
        assert_eq!(
            result[2].local_addr, "localhost:9000",
            "port 9000 must fall back to localhost:9000"
        );
    }

    #[test]
    fn test_allow_ports_custom_grpc_and_http_addrs() {
        let ports = vec![
            serde_json::json!({"port": 6565}),
            serde_json::json!({"port": 8000}),
        ];
        let result = build_allow_ports(&ports, "127.0.0.1:7777", "127.0.0.1:9999");
        assert_eq!(
            result[0].local_addr, "127.0.0.1:7777",
            "port 6565 must use the custom gRPC address"
        );
        assert_eq!(
            result[1].local_addr, "127.0.0.1:9999",
            "port 8000 must use the custom HTTP address"
        );
    }

    #[test]
    fn test_allow_ports_empty_input() {
        let result = build_allow_ports(&[], "localhost:6565", "localhost:8000");
        assert!(result.is_empty(), "empty input must produce empty output");
    }

    #[test]
    fn test_select_pod_picks_first_pod() {
        let pods = vec![
            serde_json::json!({"podName": "pod-abc", "port": 15080}),
            serde_json::json!({"podName": "pod-def", "port": 15081}),
        ];
        let pod = select_pod(&pods).expect("must select first pod");
        assert_eq!(pod.name, "pod-abc");
        assert_eq!(pod.port, 15080);
    }

    #[test]
    fn test_select_pod_empty_list_is_retriable() {
        match select_pod(&[]) {
            Err(ConnectError::Retriable(msg)) => {
                assert!(
                    msg.contains("no debug pods"),
                    "error must mention empty pods: {msg}"
                );
            }
            other => panic!("expected Retriable for empty pods, got: {other:?}"),
        }
    }

    #[test]
    fn test_select_pod_missing_port_is_retriable() {
        let pods = vec![serde_json::json!({"podName": "pod-abc"})];
        match select_pod(&pods) {
            Err(ConnectError::Retriable(msg)) => {
                assert!(
                    msg.contains("port"),
                    "error must mention missing port: {msg}"
                );
            }
            other => panic!("expected Retriable for missing port, got: {other:?}"),
        }
    }

    #[test]
    fn test_select_pod_missing_pod_name_is_retriable() {
        let pods = vec![serde_json::json!({"port": 15080})];
        match select_pod(&pods) {
            Err(ConnectError::Retriable(msg)) => {
                assert!(
                    msg.contains("podName"),
                    "error must mention the API field 'podName': {msg}"
                );
            }
            other => panic!("expected Retriable for missing podName, got: {other:?}"),
        }
    }

    #[test]
    fn test_parse_exposed_services_extracts_name_host_and_port() {
        let services = vec![
            serde_json::json!({"name": "svc-a", "host": "10.0.0.1", "port": 8080}),
            serde_json::json!({"name": "svc-b", "host": "10.0.0.2", "port": 9090}),
        ];
        let parsed = parse_exposed_services(&services);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "svc-a");
        assert_eq!(parsed[0].host, "10.0.0.1");
        assert_eq!(parsed[0].port, 8080);
        assert_eq!(parsed[1].name, "svc-b");
        assert_eq!(parsed[1].host, "10.0.0.2");
        assert_eq!(parsed[1].port, 9090);
    }

    #[test]
    fn test_parse_exposed_services_skips_malformed() {
        let services = vec![
            serde_json::json!({"name": "svc-a", "host": "10.0.0.1", "port": 8080}),
            serde_json::json!({"name": "bad", "host": "10.0.0.2"}), // missing port
            serde_json::json!({"port": 1234, "host": "10.0.0.3"}),  // missing name
            serde_json::json!({"name": "no-host", "port": 5555}),   // missing host
        ];
        let parsed = parse_exposed_services(&services);
        assert_eq!(parsed.len(), 1, "malformed entries must be skipped");
        assert_eq!(parsed[0].name, "svc-a");
    }

    #[test]
    fn test_empty_exposed_services_skip_forwarder() {
        let empty: &[ExposedService] = &[];
        assert!(
            build_forwarder_services(empty).is_none(),
            "an empty service list must not start a guaranteed-to-fail forwarder"
        );
    }

    #[test]
    fn test_nonempty_exposed_services_build_forwarder_config() {
        let services = vec![ExposedService {
            name: "metrics".to_string(),
            host: "10.0.0.5".to_string(),
            port: 9090,
        }];
        let specs = build_forwarder_services(&services).expect("forwarder services");

        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "metrics");
        assert_eq!(
            specs[0].remote_host, "10.0.0.5",
            "remote_host must come from the parsed host, not from name"
        );
        assert_eq!(specs[0].ports, vec![9090]);
    }

    // The `format_ready_line` and `should_emit_ready_line` tests were moved
    // to the session log module (U5b / U7b / quiet suppression tests) when
    // those functions were replaced by `session_log.connected()`.

    #[tokio::test]
    async fn test_shutdown_skips_unstarted_components() {
        let start = std::time::Instant::now();
        await_component::<()>(None, COMPONENT_SHUTDOWN_TIMEOUT).await;
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "await_component(None) must return instantly, took {elapsed:?}"
        );
    }

    #[test]
    fn test_attempt_cancellation_follows_command_cancellation() {
        let outer = CancellationToken::new();
        let attempt = attempt_cancellation(&outer);
        assert!(!attempt.is_cancelled());
        outer.cancel();
        assert!(
            attempt.is_cancelled(),
            "Ctrl-C cancellation must propagate to the active session attempt"
        );
    }

    #[tokio::test]
    async fn test_shutdown_cancels_before_awaiting() {
        // A component that blocks until a cancellation token fires.
        // If shutdown cancels FIRST, the component completes promptly.
        // If shutdown awaits FIRST (without cancelling), this would hang
        // for 10 seconds.
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let handle = tokio::spawn(async move {
            cancel_clone.cancelled().await;
        });

        // The correct shutdown pattern: cancel first, then await.
        cancel.cancel();

        let start = std::time::Instant::now();
        await_component(Some(handle), COMPONENT_SHUTDOWN_TIMEOUT).await;
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "component must finish promptly after cancellation, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_shutdown_timeout_is_reported() {
        let handle = tokio::spawn(std::future::pending::<()>());
        let stopped = await_component(handle.into(), std::time::Duration::from_millis(10)).await;
        assert!(
            !stopped,
            "a component timeout must be observable by the caller"
        );
    }

    // ── Orchestration integration tests ──

    /// Every `quiet` value forwarded to `run_tunnel` for the embedded
    /// tunnel, recorded by `run_embedded_tunnel` as the argument flows to
    /// the bridge. Shared across parallel tests: every embedded call site
    /// passes `EMBEDDED_TUNNEL_QUIET`, so all entries must be `true`.
    static EMBEDDED_TUNNEL_QUIET_CALLS: std::sync::Mutex<Vec<bool>> =
        std::sync::Mutex::new(Vec::new());

    /// Called by `run_embedded_tunnel` (under `cfg(test)`) with the `quiet`
    /// value it forwards to `run_tunnel`.
    pub(super) fn record_embedded_tunnel_quiet(quiet: bool) {
        EMBEDDED_TUNNEL_QUIET_CALLS
            .lock()
            .expect("quiet-call recorder lock")
            .push(quiet);
    }

    /// Every `resource_name` value forwarded to `run_tunnel` for the
    /// embedded tunnel, recorded by `run_embedded_tunnel` as the config
    /// flows to the bridge. Changing the call site to `app.to_string()`
    /// (the original defect) turns the corresponding test red.
    static EMBEDDED_TUNNEL_RESOURCE_NAMES: std::sync::Mutex<Vec<String>> =
        std::sync::Mutex::new(Vec::new());

    /// Called by `run_embedded_tunnel` (under `cfg(test)`) with the
    /// `resource_name` the `TunnelConfig` carries.
    pub(super) fn record_embedded_tunnel_resource_name(name: &str) {
        EMBEDDED_TUNNEL_RESOURCE_NAMES
            .lock()
            .expect("resource-name recorder lock")
            .push(name.to_string());
    }

    /// The embedded tunnel must be started with quiet=true: `connect_once`
    /// owns the single ready emission, so the bridge must never write one.
    /// The probe records the argument as it is forwarded to `run_tunnel`,
    /// so flipping the call-site argument to false turns this test red.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_run_session_passes_quiet_true_to_embedded_tunnel() {
        // Both `ring` and `aws-lc-rs` features are resolved for the `rustls`
        // crate (from different transitive dependents), so auto-detection
        // fails. Install `ring` explicitly; ignore the error when already
        // installed by a prior serial test.
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Process-wide env mutation: AGS_HOME, AGS_NO_KEYCHAIN, AGS_ACCESS_TOKEN.
        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_auth_env(&tmp);

        // Free port for the tunnel listener.
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind probe");
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        // Accept-then-drop upstream: the session fails fast, so the
        // timeout below is slack, not expected wait.
        let upstream_host = spawn_accept_then_drop_upstream().await;

        let debug_info = DebugInfo {
            app_status: Some("deployment-running".to_string()),
            is_debug_mode_enabled: Some(true),
            is_debug_session_connected: Some(false),
            debug_pods: Some(vec![
                serde_json::json!({"podName": "pod-test", "port": port}),
            ]),
            exposed_services: Some(vec![]),
            allowed_intercepted_ports: Some(vec![]),
        };

        // Pre-cancelled outer token bounds the run; the tunnel is still
        // spawned before the outer cancel is consulted.
        let cancel = CancellationToken::new();
        cancel.cancel();

        let params = SessionParams {
            namespace: "test-ns",
            local_grpc_addr: "localhost:6565",
            local_http_addr: "localhost:8000",
            profile: "default",
            base_url_host: &upstream_host,
        };
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_session(&debug_info, &params, test_session_log(), cancel),
        )
        .await;

        let recorded: Vec<bool> = EMBEDDED_TUNNEL_QUIET_CALLS
            .lock()
            .expect("quiet-call recorder lock")
            .clone();
        assert!(
            !recorded.is_empty(),
            "run_session must start the embedded tunnel (no quiet value was recorded)"
        );
        assert!(
            recorded.iter().all(|&q| q),
            "the embedded tunnel must always be started with quiet=true; recorded: {recorded:?}"
        );
    }

    // ── Tunnel exit classification ──

    #[test]
    fn test_describe_tunnel_exit_surfaces_tunnel_error() {
        let bind_err = std::io::Error::new(std::io::ErrorKind::AddrInUse, "address in use");
        let msg = describe_tunnel_exit(Ok(Err(TunnelError::Bind(bind_err))));
        assert!(
            msg.contains("failed to bind local port"),
            "bind failures must surface the TunnelError message; got: {msg}"
        );
    }

    #[test]
    fn test_describe_tunnel_exit_clean_exit() {
        let msg = describe_tunnel_exit(Ok(Ok(())));
        assert!(
            msg.contains("unexpectedly"),
            "clean tunnel exit must be described; got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_forwarder_task_failure_is_observable() {
        let mut handle = Some(tokio::spawn(async {
            Err::<(), std::io::Error>(std::io::Error::other("listener failed"))
        }));

        let result = observe_component(&mut handle)
            .await
            .expect("started forwarder must produce an observable result");
        let message = describe_forwarder_exit(result);

        assert!(message.contains("service forwarder failed"));
        assert!(message.contains("listener failed"));
    }

    /// Prove run_tunnel is actually called: the tunnel binds the pod port.
    ///
    /// A stalling upstream endpoint keeps the tunnel alive long enough for
    /// the port check to succeed. If `run_tunnel` is deleted from the
    /// orchestration, the port is never bound and this test fails on timeout.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_run_session_binds_tunnel_port() {
        // Both `ring` and `aws-lc-rs` features are resolved for the `rustls`
        // crate (from different transitive dependents), so auto-detection
        // fails. Install `ring` explicitly; ignore the error when already
        // installed by a prior serial test.
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Process-wide env mutation: AGS_HOME, AGS_NO_KEYCHAIN, AGS_ACCESS_TOKEN.
        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_auth_env(&tmp);

        // Stalling upstream: accepts TCP but never responds — keeps the
        // tunnel's upstream dial (TLS handshake) pending so the tunnel
        // stays alive and the port stays bound.
        let stall = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stalling endpoint");
        let stall_addr = stall.local_addr().unwrap();
        tokio::spawn(async move {
            // Hold every accepted stream open without reading or writing —
            // dropping them would reset the tunnel's upstream dial and let
            // the whole session collapse before the port check runs.
            let mut held = Vec::new();
            loop {
                if let Ok((stream, _)) = stall.accept().await {
                    held.push(stream);
                }
            }
        });

        // Free port for the tunnel.
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind probe");
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let debug_info = DebugInfo {
            app_status: Some("deployment-running".to_string()),
            is_debug_mode_enabled: Some(true),
            is_debug_session_connected: Some(false),
            debug_pods: Some(vec![
                serde_json::json!({"podName": "pod-test", "port": port}),
            ]),
            exposed_services: Some(vec![]),
            allowed_intercepted_ports: Some(vec![]),
        };

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let host = format!("127.0.0.1:{}", stall_addr.port());

        let session_handle = tokio::spawn(async move {
            let params = SessionParams {
                namespace: "test-ns",
                local_grpc_addr: "localhost:6565",
                local_http_addr: "localhost:8000",
                profile: "default",
                base_url_host: &host,
            };
            run_session(&debug_info, &params, test_session_log(), cancel_clone).await
        });

        // Poll until the tunnel port is bound, proving run_tunnel was called.
        let port_bound = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
                    .await
                    .is_ok()
                {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await;

        assert!(
            port_bound.is_ok(),
            "tunnel must bind port {port} (proves run_tunnel was called)"
        );

        cancel.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), session_handle).await;
    }

    /// Shutdown must cancel components BEFORE awaiting them, otherwise the
    /// await hangs for the full component-shutdown timeout.
    ///
    /// This test spawns a blocking component behind a cancellation token,
    /// calls run_session which must cancel-then-await. Without the cancel()
    /// call, shutdown takes >= 10 s; with it, < 2 s.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_run_session_shutdown_completes_promptly() {
        // Both `ring` and `aws-lc-rs` features are resolved for the `rustls`
        // crate (from different transitive dependents), so auto-detection
        // fails. Install `ring` explicitly; ignore the error when already
        // installed by a prior serial test.
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Process-wide env mutation: AGS_HOME, AGS_NO_KEYCHAIN, AGS_ACCESS_TOKEN.
        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_auth_env(&tmp);

        // Free port for the tunnel.
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind probe");
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        // Accept-then-drop upstream: the tunnel's upstream handshake fails
        // instantly, so the agent's failure path drives the shutdown being
        // timed here. (A freed or filtered port is NOT instant on Windows
        // loopback — refused connects take ~2 s each.)
        let upstream_host = spawn_accept_then_drop_upstream().await;

        let debug_info = DebugInfo {
            app_status: Some("deployment-running".to_string()),
            is_debug_mode_enabled: Some(true),
            is_debug_session_connected: Some(false),
            debug_pods: Some(vec![
                serde_json::json!({"podName": "pod-test", "port": port}),
            ]),
            exposed_services: Some(vec![]),
            allowed_intercepted_ports: Some(vec![]),
        };

        // Cancel immediately — the orchestration starts, then is told to stop.
        let cancel = CancellationToken::new();
        cancel.cancel();

        let start = std::time::Instant::now();
        let params = SessionParams {
            namespace: "test-ns",
            local_grpc_addr: "localhost:6565",
            local_http_addr: "localhost:8000",
            profile: "default",
            base_url_host: &upstream_host,
        };
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            run_session(&debug_info, &params, test_session_log(), cancel),
        )
        .await;
        let elapsed = start.elapsed();

        assert!(
            result.is_ok(),
            "run_session must complete within 3 s (took {elapsed:?})"
        );
        assert!(
            matches!(result, Ok(Ok(SessionOutcome::Cancelled))),
            "command cancellation must be distinguished from a server-ended session: {result:?}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "shutdown must complete in under 2 s (took {elapsed:?}); \
             check that attempt_cancel.cancel() fires before awaiting components"
        );
    }

    /// The session-init timeout used by the agent must be the module constant.
    #[test]
    fn test_session_init_timeout_const_is_five_seconds() {
        assert_eq!(
            SESSION_INIT_TIMEOUT,
            std::time::Duration::from_secs(5),
            "session-init timeout must be 5 s (parity with extend-helper-cli)"
        );
    }

    /// The embedded tunnel must use `"remote-debug"` as its resource name,
    /// matching the Go tool being replaced. The probe records the
    /// `TunnelConfig.resource_name` as it is forwarded to `run_tunnel`,
    /// so changing the call site to `app.to_string()` turns this test red.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_run_session_uses_remote_debug_as_resource_name() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let tmp = tempfile::tempdir().unwrap();
        let _env = isolated_auth_env(&tmp);

        let probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind probe");
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let upstream_host = spawn_accept_then_drop_upstream().await;

        let debug_info = DebugInfo {
            app_status: Some("deployment-running".to_string()),
            is_debug_mode_enabled: Some(true),
            is_debug_session_connected: Some(false),
            debug_pods: Some(vec![
                serde_json::json!({"podName": "pod-test", "port": port}),
            ]),
            exposed_services: Some(vec![]),
            allowed_intercepted_ports: Some(vec![]),
        };

        let cancel = CancellationToken::new();
        cancel.cancel();

        let params = SessionParams {
            namespace: "test-ns",
            local_grpc_addr: "localhost:6565",
            local_http_addr: "localhost:8000",
            profile: "default",
            base_url_host: &upstream_host,
        };
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_session(&debug_info, &params, test_session_log(), cancel),
        )
        .await;

        let recorded: Vec<String> = EMBEDDED_TUNNEL_RESOURCE_NAMES
            .lock()
            .expect("resource-name recorder lock")
            .clone();
        assert!(
            !recorded.is_empty(),
            "run_session must start the embedded tunnel \
             (no resource_name was recorded)"
        );
        assert!(
            recorded.iter().all(|n| n == "remote-debug"),
            "the embedded tunnel must use 'remote-debug' as resource_name; \
             recorded: {recorded:?}"
        );
    }
}
