//! `--wait` support for the Extend app lifecycle migration shims.
//!
//! The lifecycle commands are declarative shims that
//! forward to the generic `ags csm` service dispatch. When the user passes
//! `--wait`, the primary operation runs unchanged and then this module polls
//! `GET .../apps/{app}` until the app reaches a terminal state — mirroring the
//! asynchronous-operation behaviour of `extend-helper-cli`.

pub(crate) mod api;
pub(crate) mod wait;

use crate::errors::CliError;
use crate::frontend::streams::UiSink;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;

use wait::WaitSpec;

/// A resolved wait request: the target spec, the app to poll, and the
/// interval/limit already parsed from the `--wait-*` flags.
pub(crate) struct WaitRequest {
    pub(crate) spec: &'static WaitSpec,
    pub(crate) app: String,
    pub(crate) interval_secs: u64,
    pub(crate) limit_secs: u64,
    /// The deployment id this wait is bound to, captured from the command's own
    /// create response (only `deploy-app`, whose spec sets
    /// `guard_by_deployment_id`). Populated after the primary call succeeds; the
    /// poll only accepts a terminal state once the app reports THIS deployment.
    /// `None` for every other lifecycle command and whenever the create
    /// response carried no id.
    pub(crate) expected_deployment_id: Option<String>,
}

/// Extract the `deploymentId` from a create-deployment response body, for
/// arming `deploy-app --wait`'s identity guard. `None` when the body is absent
/// or carries no string `deploymentId` — the guard then degrades to
/// status-only evaluation (see [`wait::evaluate`]).
pub(crate) fn deployment_id_from_response(body: Option<&serde_json::Value>) -> Option<String> {
    body?
        .get("deploymentId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

/// Poll for the app's terminal state after a lifecycle shim's primary call
/// has succeeded. Resolves credentials the same way the other extend handlers
/// do, then drives [`wait::wait_until_app_reaches`].
///
/// Returns `Ok(Complete)` on success and `Err` on a failed terminal state or
/// timeout so the caller exits non-zero — matching `extend-helper-cli`.
pub(crate) async fn run_wait_after_dispatch(
    request: WaitRequest,
    flags: &GlobalFlags,
) -> Result<InvocationOutcome, CliError> {
    let input = ags_runtime::runtime::execution::ResolutionInput {
        profile: flags.profile.clone(),
        namespace: flags.namespace.clone(),
        is_dry_run: false,
    };
    let http_client = ags_runtime::runtime::dispatch::http::build_http_client(flags.timeout)?;
    let context =
        ags_runtime::runtime::execution::ExecutionContext::resolve(&input, &http_client).await?;
    let namespace = context.namespace.clone().ok_or_else(|| CliError::Usage {
        message: "a namespace is required to wait for app status; pass --namespace".to_string(),
        metadata: None,
    })?;

    let sink = UiSink;
    let _ = sink.write_line(&format!(
        "Waiting for app '{}' (polling every {}s, up to {}s)...",
        request.app, request.interval_secs, request.limit_secs
    ));

    wait::wait_until_app_reaches(
        &http_client,
        &context.base_url,
        &context.access_token,
        &namespace,
        &request.app,
        request.spec,
        request.expected_deployment_id.as_deref(),
        request.interval_secs,
        request.limit_secs,
        &sink,
    )
    .await?;

    Ok(InvocationOutcome::Complete)
}
