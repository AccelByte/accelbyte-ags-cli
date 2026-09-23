//! Handler for `ags extend security-assessment request`.
//!
//! Discovers an Extend app's testable endpoints, lets the operator choose
//! which to include in a pen-testing engagement — interactively via a
//! checklist, or non-interactively via `--all-endpoints`/`--operation-ids` —
//! warns before submitting if any selected endpoint uses a mutating HTTP
//! method, and creates the engagement via CSM.

mod api;
mod checklist;
mod permission;

use std::collections::HashMap;
use std::time::Duration;

use clap::ArgMatches;
use tokio_util::sync::CancellationToken;

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::{write_stderr, write_stderr_line, Frontend};
use crate::invocation::context::TerminalCapabilities;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use ags_protocol::output::{CommandOutput, SecurityAssessmentRequestOutput};

use self::api::{Endpoint, EndpointInfoResult, EndpointSelection};
use self::permission::ParsedPermission;

/// `--wait`'s poll interval — frequent enough to feel responsive, sparse
/// enough not to hammer CSM's list endpoint over a run that can take minutes.
const WAIT_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// `--wait-limit` default when `--wait` is passed without an explicit
/// value — long enough to comfortably cover the full observed
/// SUBMITTED → ANALYZING → TESTING → COMPLETED lifecycle.
const DEFAULT_WAIT_LIMIT_SECS: u64 = 1800;

const TERMINAL_STATUS_COMPLETED: &str = "COMPLETED";
const TERMINAL_STATUS_FAILED: &str = "FAILED";

fn read_line_from_stdin() -> Result<String, CliError> {
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| CliError::Usage {
            message: format!("Failed to read input: {e}"),
            metadata: None,
        })?;
    Ok(input.trim().to_string())
}

pub(crate) async fn handle_security_assessment_request(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    frontend: &mut dyn Frontend,
) -> Result<InvocationOutcome, CliError> {
    handle_with_reader(matches, flags, frontend, &mut read_line_from_stdin).await
}

async fn handle_with_reader(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    frontend: &mut dyn Frontend,
    read: &mut dyn FnMut() -> Result<String, CliError>,
) -> Result<InvocationOutcome, CliError> {
    let app = matches
        .get_one::<String>("app")
        .ok_or_else(|| CliError::Usage {
            message: "--app is required".to_string(),
            metadata: None,
        })?
        .clone();
    let namespace = flags.namespace.clone().ok_or_else(|| CliError::Usage {
        message: "--namespace is required for extend security-assessment request".to_string(),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            "Supply --namespace <ns> or set a default via 'ags config set namespace <ns>'",
        ))),
    })?;

    let all_endpoints_flag = matches.get_flag("all-endpoints");
    let operation_ids: Option<Vec<String>> =
        matches.get_one::<String>("operation-ids").map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        });
    let permission_flags: Vec<String> = matches
        .get_many::<String>("permission")
        .map(|values| values.cloned().collect())
        .unwrap_or_default();

    // Validated here, before any request: the wait budget is read after the
    // engagement exists, and a usage error at that point would leave a created
    // engagement behind. Same rule and wording as the app lifecycle commands,
    // which now share the flag name.
    let wait_limit_secs = matches
        .get_one::<u64>("wait-limit")
        .copied()
        .unwrap_or(DEFAULT_WAIT_LIMIT_SECS);
    if wait_limit_secs == 0 {
        return Err(CliError::Usage {
            message: "--wait-limit must be greater than 0".to_string(),
            metadata: None,
        });
    }

    let http_client = ags_runtime::runtime::dispatch::http::build_http_client(flags.timeout)?;
    let input = ags_runtime::runtime::execution::ResolutionInput {
        profile: flags.profile.clone(),
        namespace: Some(namespace.clone()),
        is_dry_run: false,
    };
    let context =
        ags_runtime::runtime::execution::ExecutionContext::resolve(&input, &http_client).await?;
    let resolved_namespace = context.namespace.clone().unwrap_or(namespace);
    // `create_engagement` stays hand-rolled reqwest, so it needs these kept
    // aside before `context`/`http_client` are moved into the `Runtime`
    // that `get_app_endpoints` dispatches through.
    let base_url = context.base_url.clone();
    let access_token = context.access_token.clone();
    let create_client = http_client.clone();

    let mut runtime = ags_runtime::runtime::Runtime::from_reqwest(context, http_client);
    let discovery = api::get_app_endpoints(&mut runtime, &resolved_namespace, &app).await?;

    check_preconditions(&discovery, &app)?;
    print_banners(&discovery, &app);

    let overrides =
        resolve_permission_overrides(&discovery, &permission_flags, &resolved_namespace)?;

    let selections = if all_endpoints_flag || operation_ids.is_some() {
        let selections = resolve_from_flags(
            &discovery,
            all_endpoints_flag,
            operation_ids.as_deref(),
            &overrides,
            &app,
        )?;
        if selections.is_empty() {
            return Err(CliError::Usage {
                message: "No endpoints selected".to_string(),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Check --operation-ids — it must include at least one valid operation id",
                ))),
            });
        }
        selections
    } else {
        let caps = TerminalCapabilities::detect(flags.is_no_color);
        if flags.is_no_input || !caps.allows_interactive_prompts() {
            return Err(CliError::Usage {
                message: "Selecting endpoints requires an interactive terminal".to_string(),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Use --all-endpoints or --operation-ids <id1,id2,...> in non-interactive mode",
                ))),
            });
        }
        checklist::run(
            &discovery.endpoints,
            discovery.maximum_selectable_endpoints as usize,
            resolved_namespace.clone(),
            &overrides,
        )?
    };

    // Dry-run must short-circuit BEFORE the mutating-endpoint confirmation —
    // reading stdin here would block indefinitely in automation (the same
    // class of HIGH `extend/update_secret/mod.rs` already hardened against).
    if flags.is_dry_run {
        return dry_run_preview(&resolved_namespace, &app, &selections);
    }

    let mutating: Vec<(String, String)> = selections
        .iter()
        .filter_map(|selection| {
            discovery
                .endpoints
                .iter()
                .find(|e| e.operation_id == selection.operation_id)
                .filter(|e| e.is_mutating())
                .map(|e| (e.method.clone(), e.path.clone()))
        })
        .collect();
    if !mutating.is_empty() {
        confirm_mutating_endpoints(&app, &mutating, flags, read)?;
    }

    let engagement = api::create_engagement(
        &create_client,
        &base_url,
        &access_token,
        &resolved_namespace,
        &app,
        &selections,
    )
    .await?;
    let engagement_id = engagement.engagement_id;
    let mut status = engagement.status;

    if matches.get_flag("wait") {
        let quiet = flags.verbosity.is_quiet();
        if !quiet {
            write_stderr_line(&style::info(
                &format!(
                    "Waiting for engagement #{engagement_id} to reach a terminal state \
                     (timeout {wait_limit_secs}s)..."
                ),
                style::is_stderr_enabled(),
            ));
        }
        status = wait_for_terminal_status(
            &mut runtime,
            &resolved_namespace,
            &app,
            engagement_id,
            wait_limit_secs,
            quiet,
        )
        .await?;
    }

    let output = SecurityAssessmentRequestOutput {
        namespace: resolved_namespace,
        app,
        engagement_id,
        status,
        endpoint_count: selections.len(),
    };
    frontend.render(&CommandOutput::SecurityAssessmentRequest(output))?;

    Ok(InvocationOutcome::Complete)
}

// ── `--wait`: poll until a terminal engagement status ──

/// Poll `engagement_id` until it reaches `COMPLETED`/`FAILED` or
/// `timeout_secs` elapses. A Ctrl-C during the wait cancels only this local
/// loop — it does not cancel the engagement server-side — following the
/// same `declare_command_owns_interrupt_path` + `CancellationToken` pattern
/// as `remote_debug::run_connect_loop`.
async fn wait_for_terminal_status(
    runtime: &mut ags_runtime::runtime::Runtime,
    namespace: &str,
    app: &str,
    engagement_id: i64,
    timeout_secs: u64,
    quiet: bool,
) -> Result<String, CliError> {
    crate::invocation::declare_command_owns_interrupt_path();
    let cancel = CancellationToken::new();
    let signal_cancel = cancel.clone();
    let signal_task = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_cancel.cancel();
        }
    });

    let result = poll_until_terminal(
        runtime,
        namespace,
        engagement_id,
        timeout_secs,
        quiet,
        &cancel,
    )
    .await;
    signal_task.abort();

    match result {
        Ok(status) if status == TERMINAL_STATUS_COMPLETED => Ok(status),
        Ok(status) => Err(CliError::Api {
            message: format!(
                "Security assessment for '{app}' (engagement #{engagement_id}) finished with \
                 status {status}"
            ),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_info(format!(
                "Run 'ags extend security-assessment list --namespace {namespace}' to check its status"
            )))),
            category: crate::errors::ApiErrorCategory::Upstream,
        }),
        Err(e) => Err(e),
    }
}

async fn poll_until_terminal(
    runtime: &mut ags_runtime::runtime::Runtime,
    namespace: &str,
    engagement_id: i64,
    timeout_secs: u64,
    quiet: bool,
    cancel: &CancellationToken,
) -> Result<String, CliError> {
    let start = tokio::time::Instant::now();
    let deadline = start + Duration::from_secs(timeout_secs);
    loop {
        let status = api::get_engagement_status(runtime, namespace, engagement_id).await?;
        if matches!(
            status.as_str(),
            TERMINAL_STATUS_COMPLETED | TERMINAL_STATUS_FAILED
        ) {
            return Ok(status);
        }
        if !quiet {
            write_stderr_line(&style::info(
                &format!(
                    "Engagement #{engagement_id}: {status} (elapsed {}s)",
                    start.elapsed().as_secs()
                ),
                style::is_stderr_enabled(),
            ));
        }
        if cancel.is_cancelled() {
            return Err(wait_interrupted_error(namespace, engagement_id));
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(CliError::Api {
                message: format!(
                    "Timed out after {timeout_secs}s waiting for engagement #{engagement_id} \
                     (last status: {status})"
                ),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_info(format!(
                    "The engagement is still running server-side — check back later with \
                     'ags extend security-assessment list --namespace {namespace}', or increase \
                     --wait-limit"
                )))),
                // Timeout, not Upstream: exit 6 rather than 3, so a caller can
                // tell "the budget ran out, the engagement may still finish"
                // from "the server failed it" without matching message text.
                // Same contract as the app lifecycle wait.
                category: crate::errors::ApiErrorCategory::Timeout,
            });
        }
        // `now` is reused from the deadline check above rather than read
        // again, and the remaining budget is computed with the non-panicking
        // form, so a future edit that puts an await between the two cannot
        // panic on a negative duration.
        let step = next_poll_step(WAIT_POLL_INTERVAL, deadline.saturating_duration_since(now));
        tokio::select! {
            _ = tokio::time::sleep(step) => {}
            _ = cancel.cancelled() => return Err(wait_interrupted_error(namespace, engagement_id)),
        }
    }
}

/// How long to sleep before the next poll: a full `interval`, but never past
/// `remaining`, so the wait honours `--wait-limit` to the second instead of
/// overshooting by up to one interval when the limit is not a multiple of the
/// interval. Mirrors `next_poll_step` in the app lifecycle wait.
fn next_poll_step(interval: Duration, remaining: Duration) -> Duration {
    interval.min(remaining)
}

fn wait_interrupted_error(namespace: &str, engagement_id: i64) -> CliError {
    CliError::Usage {
        message: format!(
            "Wait interrupted — engagement #{engagement_id} is still running server-side"
        ),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_info(format!(
            "Check its status later with 'ags extend security-assessment list --namespace {namespace}'"
        )))),
    }
}

// ── Preconditions and banners ──

fn check_preconditions(discovery: &EndpointInfoResult, app: &str) -> Result<(), CliError> {
    if !discovery.is_app_running {
        return Err(CliError::Usage {
            message: format!(
                "'{app}' isn't running. Start the app before requesting a security assessment."
            ),
            metadata: None,
        });
    }
    if !discovery.has_api_spec {
        return Err(CliError::Usage {
            message: format!(
                "No OpenAPI specification found for '{app}'. A security assessment requires \
                 your Extend App to expose an OpenAPI specification."
            ),
            metadata: None,
        });
    }
    if discovery.endpoints.is_empty() {
        return Err(CliError::Usage {
            message: "No endpoints found for this app.".to_string(),
            metadata: None,
        });
    }
    Ok(())
}

fn print_banners(discovery: &EndpointInfoResult, app: &str) {
    let color = style::is_stderr_enabled();
    write_stderr_line(&style::warning(
        &format!("'{app}' can't be stopped or redeployed until this test finishes."),
        color,
    ));
    if discovery.has_grpc_reflection {
        write_stderr_line(&style::info(
            "Permissions are auto-filled from your OpenAPI spec or gRPC reflection where \
             available. Confirm the filled-in values are correct before submitting.",
            color,
        ));
    } else {
        write_stderr_line(&style::warning(
            &format!(
                "gRPC reflection isn't exposed on '{app}'. Permissions were auto-filled from \
                 your OpenAPI spec only, so some endpoints below may need manual entry. Confirm \
                 every value before submitting."
            ),
            color,
        ));
    }
}

// ── `--permission` override parsing ──

fn resolve_permission_overrides(
    discovery: &EndpointInfoResult,
    permission_flags: &[String],
    namespace: &str,
) -> Result<HashMap<String, ParsedPermission>, CliError> {
    let mut overrides = HashMap::new();
    for raw in permission_flags {
        let (operation_id, permission_str) =
            raw.split_once('=').ok_or_else(|| CliError::Usage {
                message: format!(
                "Invalid --permission value '{raw}' — expected <operationId>=<RESOURCE> [<ACTION>]"
            ),
                metadata: None,
            })?;
        let endpoint = discovery
            .endpoints
            .iter()
            .find(|e| e.operation_id == operation_id)
            .ok_or_else(|| CliError::Usage {
                message: format!("--permission targets unknown operation id '{operation_id}'"),
                metadata: None,
            })?;
        if !endpoint.is_permission_editable() {
            return Err(CliError::Usage {
                message: format!(
                    "--permission targets '{operation_id}' ({} {}), which already has a \
                     discovered permission (or doesn't require authentication) and can't be \
                     overridden",
                    endpoint.method, endpoint.path
                ),
                metadata: None,
            });
        }
        if overrides.contains_key(operation_id) {
            return Err(CliError::Usage {
                message: format!("--permission for '{operation_id}' was given more than once"),
                metadata: None,
            });
        }
        let parsed =
            permission::parse_permission(permission_str, namespace).map_err(|message| {
                CliError::Usage {
                    message,
                    metadata: None,
                }
            })?;
        overrides.insert(operation_id.to_string(), parsed);
    }
    Ok(overrides)
}

// ── Non-interactive endpoint resolution ──

fn resolve_from_flags(
    discovery: &EndpointInfoResult,
    all_endpoints: bool,
    operation_ids: Option<&[String]>,
    overrides: &HashMap<String, ParsedPermission>,
    app: &str,
) -> Result<Vec<EndpointSelection>, CliError> {
    let selected: Vec<&Endpoint> = if let Some(ids) = operation_ids {
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            if !seen.insert(id) {
                return Err(CliError::Usage {
                    message: format!("--operation-ids '{id}' was given more than once"),
                    metadata: None,
                });
            }
        }
        ids.iter()
            .map(|id| {
                discovery
                    .endpoints
                    .iter()
                    .find(|e| &e.operation_id == id)
                    .ok_or_else(|| CliError::Usage {
                        message: format!("--operation-ids includes unknown operation id '{id}'"),
                        metadata: None,
                    })
            })
            .collect::<Result<_, _>>()?
    } else {
        debug_assert!(all_endpoints);
        discovery.endpoints.iter().collect()
    };

    // Enforced on the final selected count regardless of which flag produced
    // it — `--operation-ids` used to skip this cap entirely, so a too-large
    // explicit list would sail past this check and only fail with a generic
    // 400 from CSM after the mutating-endpoint confirmation prompt.
    let cap = discovery.maximum_selectable_endpoints as usize;
    if selected.len() > cap {
        return Err(CliError::Usage {
            message: format!(
                "Too many endpoints ({}) — the limit for '{app}' is {cap}",
                selected.len()
            ),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Use --operation-ids <id1,id2,...> to narrow the selection",
            ))),
        });
    }

    // Not a hard blocker — an operator may not know every endpoint's
    // permission up front, and CSM re-validates on submit.
    let missing_permission: Vec<&str> = selected
        .iter()
        .filter(|e| e.is_permission_editable() && !overrides.contains_key(&e.operation_id))
        .map(|e| e.operation_id.as_str())
        .collect();
    if !missing_permission.is_empty() {
        write_stderr_line(&style::warning(
            &format!(
                "No permission supplied for: {} — submitting without an override for these.",
                missing_permission.join(", ")
            ),
            style::is_stderr_enabled(),
        ));
    }

    Ok(selected
        .into_iter()
        .map(|e| EndpointSelection {
            operation_id: e.operation_id.clone(),
            permission_override: overrides.get(&e.operation_id).cloned(),
        })
        .collect())
}

// ── Mutating-method confirmation ──
//
// This warning/confirmation gate is unique to this native handler. The
// underlying operation (`csm/admin/security-assessment/v1/create`) is also
// reachable directly through the generic `ags csm security-assessment
// create` dispatch path, which bypasses this prompt entirely — the
// catalogue-wide `requires_confirmation` heuristic only recognises risky
// *keywords* in an operation's generic CLI method name (e.g. "delete",
// "revoke"), and this operation's name is just "create", shared by dozens
// of unrelated resources, so it cannot be gated there without affecting
// every other `create` operation in the CLI. Accepted trade-off: `list` and
// `get-app-endpoints` are already dual-exposed (native + generic) the same
// way, and unlike this operation, they're both read-only.

fn confirm_mutating_endpoints(
    app: &str,
    endpoints: &[(String, String)],
    flags: &GlobalFlags,
    read: &mut dyn FnMut() -> Result<String, CliError>,
) -> Result<(), CliError> {
    if flags.is_auto_confirmed {
        return Ok(());
    }
    if flags.is_no_input {
        return Err(CliError::Usage {
            message: "This request includes endpoints that can modify or delete data and \
                      requires confirmation"
                .to_string(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Use --yes to confirm in non-interactive mode",
            ))),
        });
    }

    // PUT → PATCH → DELETE ordering, matching the Admin Portal — reuses
    // `Endpoint::is_mutating`'s method set so the two can't drift apart.
    let mut methods: Vec<&str> = Vec::new();
    for m in api::MUTATING_METHODS {
        if endpoints
            .iter()
            .any(|(method, _)| method.eq_ignore_ascii_case(m))
        {
            methods.push(m);
        }
    }

    let color = style::is_stderr_enabled();
    write_stderr_line(&style::warning(
        "This request includes endpoints that can modify or delete data",
        color,
    ));
    write_stderr_line("");
    write_stderr_line(&format!(
        "The endpoints below in '{app}' accept {} requests. The security assessment may \
         generate test cases that modify or delete existing data through them:",
        methods.join(", ")
    ));
    for (method, path) in endpoints {
        write_stderr_line(&format!(
            "  {}",
            style::ansi::bold(&format!("{method:<7} {path}"), color)
        ));
    }
    write_stderr_line("");
    write_stderr(&style::ansi::bold("Proceed with request? [y/N] ", color));

    let input = read()?;
    if !matches!(input.as_str(), "y" | "Y") {
        return Err(CliError::Usage {
            message: "Operation cancelled".to_string(),
            metadata: None,
        });
    }
    Ok(())
}

// ── Dry-run preview ──

fn dry_run_preview(
    namespace: &str,
    app: &str,
    selections: &[EndpointSelection],
) -> Result<InvocationOutcome, CliError> {
    let color = style::is_stderr_enabled();
    write_stderr_line(&style::info(
        "Dry run — no security assessment will be requested",
        color,
    ));
    write_stderr_line(&format!("  Namespace: {namespace}"));
    write_stderr_line(&format!("  App:       {app}"));
    write_stderr_line(&format!("  Endpoints: {}", selections.len()));
    for selection in selections {
        let override_note = selection
            .permission_override
            .as_ref()
            .map(|p| format!(" (override: {} [{}])", p.resource, p.action))
            .unwrap_or_default();
        write_stderr_line(&format!("    {}{}", selection.operation_id, override_note));
    }
    Ok(InvocationOutcome::Complete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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

    #[derive(Default)]
    struct CapturingFrontend {
        last: Option<SecurityAssessmentRequestOutput>,
    }

    impl crate::frontend::Frontend for CapturingFrontend {
        fn render(&mut self, output: &CommandOutput) -> Result<(), CliError> {
            if let CommandOutput::SecurityAssessmentRequest(output) = output {
                self.last = Some(output.clone());
            }
            Ok(())
        }
        fn render_error(&mut self, _err: &CliError) {}
        fn render_warning(&mut self, _msg: &str, _reason: Option<&str>, _tip: Option<&str>) {}
        fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {}
        fn finish(self: Box<Self>) -> Result<(), CliError> {
            Ok(())
        }
    }

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

    fn real_request_matches(args: &[&str]) -> ArgMatches {
        let mut command = crate::invocation::builder::build_extend_command();
        let argv: Vec<&str> = ["extend", "security-assessment", "request"]
            .into_iter()
            .chain(args.iter().copied())
            .collect();
        let matches = command
            .try_get_matches_from_mut(argv)
            .expect("real command tree must accept these args");
        let (_, sa_matches) = matches
            .subcommand()
            .filter(|(name, _)| *name == "security-assessment")
            .expect("security-assessment subcommand must match");
        let (_, request_matches) = sa_matches
            .subcommand()
            .filter(|(name, _)| *name == "request")
            .expect("request subcommand must match");
        request_matches.clone()
    }

    async fn mount_discovery(server: &MockServer, app: &str, body: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path(format!(
                "/csm/v1/admin/namespaces/test-ns/pentestings/apps/{app}/endpoints"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    fn running_discovery_with(endpoints: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "isAppRunning": true,
            "hasAPISpec": true,
            "hasGRPCReflection": true,
            "maximumSelectableEndpoints": 10,
            "endpoints": endpoints,
        })
    }

    fn discovered_endpoint(operation_id: &str, method: &str, path: &str) -> serde_json::Value {
        serde_json::json!({
            "method": method,
            "path": path,
            "operationId": operation_id,
            "requireAuthentication": true,
            "permission": {"resource": "NAMESPACE:test-ns:USER", "action": "READ"}
        })
    }

    /// No auto-discovered permission — editable.
    fn editable_endpoint(operation_id: &str, method: &str, path: &str) -> serde_json::Value {
        serde_json::json!({
            "method": method,
            "path": path,
            "operationId": operation_id,
            "requireAuthentication": true,
        })
    }

    fn flags(overrides: GlobalFlags) -> GlobalFlags {
        GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..overrides
        }
    }

    fn panics_if_called() -> Result<String, CliError> {
        panic!("stdin reader should not be called")
    }

    #[tokio::test]
    async fn missing_app_is_usage_error() {
        let matches = clap::Command::new("request")
            .arg(clap::Arg::new("app").long("app"))
            .try_get_matches_from(["request"])
            .unwrap();
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        assert!(matches!(result, Err(CliError::Usage { .. })));
    }

    /// `--all-endpoints` and `--operation-ids` together must be rejected by
    /// clap — without this, `resolve_from_flags` would silently discard
    /// `--all-endpoints` and scope the request to only the listed ids.
    #[test]
    fn test_all_endpoints_and_operation_ids_conflict() {
        let mut command = crate::invocation::builder::build_extend_command();
        let argv = [
            "extend",
            "security-assessment",
            "request",
            "--app",
            "my-app",
            "--all-endpoints",
            "--operation-ids",
            "op-1",
        ];
        let result = command.try_get_matches_from_mut(argv);
        assert!(
            result.is_err(),
            "--all-endpoints and --operation-ids together must be rejected"
        );
    }

    #[tokio::test]
    async fn missing_namespace_is_usage_error() {
        let matches = real_request_matches(&["--app", "my-app"]);
        let flags = GlobalFlags::default();
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("--namespace")),
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn app_not_running_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            serde_json::json!({
                "isAppRunning": false,
                "hasAPISpec": true,
                "hasGRPCReflection": true,
                "maximumSelectableEndpoints": 10,
                "endpoints": []
            }),
        )
        .await;

        let matches = real_request_matches(&["--app", "my-app", "--all-endpoints"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("isn't running")),
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    // CSM sends `endpoints: null`, not `[]`, for a stopped app.
    #[tokio::test]
    #[serial_test::serial]
    async fn app_not_running_with_null_endpoints_is_usage_error_not_parse_error() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            serde_json::json!({
                "isAppRunning": false,
                "hasAPISpec": false,
                "hasGRPCReflection": false,
                "maximumSelectableEndpoints": 10,
                "endpoints": null
            }),
        )
        .await;

        let matches = real_request_matches(&["--app", "my-app", "--all-endpoints"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("isn't running")),
            other => panic!("expected a friendly Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn operation_ids_unknown_id_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1", "GET", "/users"
            )])),
        )
        .await;

        let matches = real_request_matches(&["--app", "my-app", "--operation-ids", "op-unknown"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("op-unknown")),
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    // An empty/blank `--operation-ids` value (e.g. from an empty shell
    // variable) must fail fast client-side, not silently submit an
    // engagement with zero endpoints.
    #[tokio::test]
    #[serial_test::serial]
    async fn operation_ids_empty_value_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1", "GET", "/users"
            )])),
        )
        .await;

        let matches = real_request_matches(&["--app", "my-app", "--operation-ids", " , ,"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("No endpoints")),
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    // `--operation-ids` must enforce the same `maximumSelectableEndpoints`
    // cap `--all-endpoints` does, instead of sailing through to a generic
    // CSM 400 after the mutating-endpoint confirmation prompt.
    #[tokio::test]
    #[serial_test::serial]
    async fn operation_ids_exceeding_cap_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            serde_json::json!({
                "isAppRunning": true,
                "hasAPISpec": true,
                "hasGRPCReflection": true,
                "maximumSelectableEndpoints": 1,
                "endpoints": [
                    discovered_endpoint("op-1", "GET", "/users"),
                    discovered_endpoint("op-2", "GET", "/health"),
                ]
            }),
        )
        .await;

        let matches = real_request_matches(&["--app", "my-app", "--operation-ids", "op-1,op-2"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("Too many endpoints")),
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    // A repeated id in `--operation-ids` must be rejected explicitly rather
    // than silently sending the same endpoint twice — matching how
    // `--permission` rejects a repeated operation id.
    #[tokio::test]
    #[serial_test::serial]
    async fn operation_ids_duplicate_id_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1", "GET", "/users"
            )])),
        )
        .await;

        let matches = real_request_matches(&["--app", "my-app", "--operation-ids", "op-1,op-1"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(message.contains("op-1"));
                assert!(message.contains("more than once"));
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn all_endpoints_exceeding_cap_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            serde_json::json!({
                "isAppRunning": true,
                "hasAPISpec": true,
                "hasGRPCReflection": true,
                "maximumSelectableEndpoints": 1,
                "endpoints": [
                    discovered_endpoint("op-1", "GET", "/users"),
                    discovered_endpoint("op-2", "GET", "/health"),
                ]
            }),
        )
        .await;

        let matches = real_request_matches(&["--app", "my-app", "--all-endpoints"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("Too many endpoints")),
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn permission_flag_targets_non_editable_row_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1", "GET", "/users"
            )])),
        )
        .await;

        let matches = real_request_matches(&[
            "--app",
            "my-app",
            "--all-endpoints",
            "--permission",
            "op-1=NAMESPACE:test-ns:USER [DELETE]",
        ]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("op-1")),
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn repeated_permission_flag_for_same_operation_id_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([editable_endpoint(
                "op-1", "GET", "/users"
            )])),
        )
        .await;

        let matches = real_request_matches(&[
            "--app",
            "my-app",
            "--all-endpoints",
            "--permission",
            "op-1=NAMESPACE:test-ns:USER [READ]",
            "--permission",
            "op-1=NAMESPACE:test-ns:USER [UPDATE]",
        ]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(message.contains("op-1"));
                assert!(message.contains("more than once"));
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn no_input_without_selection_flags_is_usage_error() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1", "GET", "/users"
            )])),
        )
        .await;

        let matches = real_request_matches(&["--app", "my-app"]);
        let flags = flags(GlobalFlags {
            is_no_input: true,
            ..Default::default()
        });
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(message.contains("interactive terminal"))
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn mutating_confirm_declined_cancels() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1",
                "DELETE",
                "/users/{id}"
            )])),
        )
        .await;

        let matches = real_request_matches(&["--app", "my-app", "--all-endpoints"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let mut reader = || Ok("n".to_string());
        let result = handle_with_reader(&matches, &flags, &mut frontend, &mut reader).await;
        match result {
            Err(CliError::Usage { message, .. }) => assert_eq!(message, "Operation cancelled"),
            other => panic!("expected cancellation, got {other:?}"),
        }
    }

    // A bare Enter (empty line) at the confirmation prompt must cancel
    // cleanly, not error — the crate-shared stdin reader used for
    // `--value-stdin`-style inputs errors on empty input, which is wrong here.
    #[tokio::test]
    #[serial_test::serial]
    async fn mutating_confirm_empty_input_cancels_not_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1",
                "DELETE",
                "/users/{id}"
            )])),
        )
        .await;

        let matches = real_request_matches(&["--app", "my-app", "--all-endpoints"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let mut reader = || Ok(String::new());
        let result = handle_with_reader(&matches, &flags, &mut frontend, &mut reader).await;
        match result {
            Err(CliError::Usage { message, .. }) => assert_eq!(message, "Operation cancelled"),
            other => panic!("expected cancellation, got {other:?}"),
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn mutating_confirm_accepted_creates_engagement() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1",
                "DELETE",
                "/users/{id}"
            )])),
        )
        .await;
        Mock::given(method("POST"))
            .and(path("/csm/v1/admin/namespaces/test-ns/pentestings"))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "engagementId": 7,
                "status": "SUBMITTED",
                "originalStatus": "queued",
                "targetApp": "my-app",
                "targetNamespace": "test-ns",
                "targetAppVersion": "v1"
            })))
            .mount(&server)
            .await;

        let matches = real_request_matches(&["--app", "my-app", "--all-endpoints"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = CapturingFrontend::default();
        let mut reader = || Ok("y".to_string());
        let result = handle_with_reader(&matches, &flags, &mut frontend, &mut reader).await;
        assert!(result.is_ok(), "expected success, got {result:?}");
        let output = frontend.last.expect("output must be rendered");
        assert_eq!(output.engagement_id, 7);
        assert_eq!(output.status, "SUBMITTED");
        assert_eq!(output.endpoint_count, 1);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn yes_flag_skips_mutating_confirm_without_prompting() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1",
                "DELETE",
                "/users/{id}"
            )])),
        )
        .await;
        Mock::given(method("POST"))
            .and(path("/csm/v1/admin/namespaces/test-ns/pentestings"))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "engagementId": 9,
                "status": "SUBMITTED",
                "originalStatus": "queued",
                "targetApp": "my-app",
                "targetNamespace": "test-ns",
                "targetAppVersion": "v1"
            })))
            .mount(&server)
            .await;

        let matches = real_request_matches(&["--app", "my-app", "--all-endpoints"]);
        let flags = flags(GlobalFlags {
            is_auto_confirmed: true,
            ..Default::default()
        });
        let mut frontend = CapturingFrontend::default();
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        assert!(result.is_ok(), "expected success, got {result:?}");
    }

    // Dry-run must short-circuit before the mutating-endpoint confirmation —
    // a DELETE endpoint selected under --dry-run must never touch the reader.
    #[tokio::test]
    #[serial_test::serial]
    async fn dry_run_with_mutating_endpoint_does_not_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1",
                "DELETE",
                "/users/{id}"
            )])),
        )
        .await;

        let matches = real_request_matches(&["--app", "my-app", "--all-endpoints"]);
        let flags = flags(GlobalFlags {
            is_dry_run: true,
            ..Default::default()
        });
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        assert!(
            result.is_ok(),
            "dry-run should succeed without prompting for confirmation: {result:?}"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn dry_run_does_not_call_create() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1", "GET", "/users"
            )])),
        )
        .await;

        let matches = real_request_matches(&["--app", "my-app", "--all-endpoints"]);
        let flags = flags(GlobalFlags {
            is_dry_run: true,
            ..Default::default()
        });
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        assert!(
            result.is_ok(),
            "dry-run should succeed without calling create: {result:?}"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn missing_permission_on_editable_endpoint_warns_but_does_not_block_submit() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([editable_endpoint(
                "op-1", "GET", "/users"
            )])),
        )
        .await;

        let matches = real_request_matches(&["--app", "my-app", "--all-endpoints"]);
        let flags = flags(GlobalFlags {
            is_dry_run: true,
            ..Default::default()
        });
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        assert!(
            result.is_ok(),
            "missing permission must not block submission: {result:?}"
        );
    }

    // ── `--wait` ──

    async fn mount_engagement_list(server: &MockServer, engagement_id: i64, status: &str) {
        Mock::given(method("GET"))
            .and(path("/csm/v1/admin/namespaces/test-ns/pentestings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "pentestings": [
                    {"engagementId": engagement_id, "targetApp": "my-app", "status": status}
                ]
            })))
            .mount(server)
            .await;
    }

    async fn mount_create(server: &MockServer, engagement_id: i64) {
        Mock::given(method("POST"))
            .and(path("/csm/v1/admin/namespaces/test-ns/pentestings"))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "engagementId": engagement_id,
                "status": "SUBMITTED",
                "originalStatus": "queued",
                "targetApp": "my-app",
                "targetNamespace": "test-ns",
                "targetAppVersion": "v1"
            })))
            .mount(server)
            .await;
    }

    #[test]
    fn wait_limit_without_wait_is_usage_error() {
        let mut command = crate::invocation::builder::build_extend_command();
        let argv = [
            "extend",
            "security-assessment",
            "request",
            "--app",
            "my-app",
            "--all-endpoints",
            "--wait-limit",
            "60",
        ];
        let result = command.try_get_matches_from_mut(argv);
        assert!(
            result.is_err(),
            "--wait-limit without --wait must be rejected"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn wait_flag_reports_completed_status() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1", "GET", "/users"
            )])),
        )
        .await;
        mount_create(&server, 7).await;
        mount_engagement_list(&server, 7, "COMPLETED").await;

        let matches = real_request_matches(&["--app", "my-app", "--all-endpoints", "--wait"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = CapturingFrontend::default();
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        assert!(result.is_ok(), "expected success, got {result:?}");
        let output = frontend.last.expect("output must be rendered");
        assert_eq!(output.status, "COMPLETED");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn wait_flag_engagement_failed_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1", "GET", "/users"
            )])),
        )
        .await;
        mount_create(&server, 8).await;
        mount_engagement_list(&server, 8, "FAILED").await;

        let matches = real_request_matches(&["--app", "my-app", "--all-endpoints", "--wait"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Api { message, .. }) => {
                assert!(message.contains("FAILED"));
                assert!(message.contains('8'));
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[test]
    fn next_poll_step_sleeps_a_full_interval_when_the_budget_allows() {
        assert_eq!(
            next_poll_step(Duration::from_secs(10), Duration::from_secs(30)),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn next_poll_step_never_sleeps_past_the_remaining_budget() {
        // The case the cap exists for: a limit that is not a multiple of the
        // interval. Without the cap this sleeps 10s and reports the timeout 5s
        // after the limit the caller asked for.
        assert_eq!(
            next_poll_step(Duration::from_secs(10), Duration::from_secs(5)),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn next_poll_step_at_exactly_the_interval_sleeps_the_interval() {
        assert_eq!(
            next_poll_step(Duration::from_secs(10), Duration::from_secs(10)),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn next_poll_step_with_no_budget_left_does_not_sleep() {
        // Unreachable through the loop, which returns the timeout before it
        // gets here, but it is the value the non-panicking subtraction yields
        // if an await is ever inserted between the check and this call.
        assert_eq!(
            next_poll_step(Duration::from_secs(10), Duration::ZERO),
            Duration::ZERO
        );
    }

    /// `--wait-limit 0` is rejected before any request is sent, so no
    /// engagement is created and then orphaned by a usage error. The mock
    /// server mounts nothing: reaching it at all would fail the test.
    #[tokio::test]
    #[serial_test::serial]
    async fn wait_limit_zero_is_rejected_before_any_request() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);

        let matches = real_request_matches(&[
            "--app",
            "my-app",
            "--all-endpoints",
            "--wait",
            "--wait-limit",
            "0",
        ]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert_eq!(message, "--wait-limit must be greater than 0");
            }
            other => panic!("expected a usage error, got {other:?}"),
        }
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "no request may be sent when the wait budget is rejected"
        );
    }

    // A still-non-terminal status must time out rather than poll forever.
    // `--wait-limit 1` keeps this fast: the sleep is capped to the remaining
    // budget, so it waits one second rather than a full `WAIT_POLL_INTERVAL`.
    #[tokio::test]
    #[serial_test::serial]
    async fn wait_flag_times_out_on_non_terminal_status() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_discovery(
            &server,
            "my-app",
            running_discovery_with(serde_json::json!([discovered_endpoint(
                "op-1", "GET", "/users"
            )])),
        )
        .await;
        mount_create(&server, 9).await;
        mount_engagement_list(&server, 9, "TESTING").await;

        let matches = real_request_matches(&[
            "--app",
            "my-app",
            "--all-endpoints",
            "--wait",
            "--wait-limit",
            "1",
        ]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(error @ CliError::Api { .. }) => {
                let CliError::Api { ref message, .. } = error else {
                    unreachable!()
                };
                assert!(message.contains("Timed out"));
                // Exit 6, not 3: the same machine-readable contract the app
                // lifecycle wait gives, so a caller can retry a timeout without
                // matching message text.
                assert_eq!(error.exit_code(), 6);
            }
            other => panic!("expected timeout Api error, got {other:?}"),
        }
    }

    // A cancelled token must stop the poll loop immediately without waiting
    // out `WAIT_POLL_INTERVAL`, and without treating the engagement as
    // terminal — exercised directly against `poll_until_terminal` since
    // driving a real Ctrl-C through `handle_with_reader` isn't practical in
    // a unit test.
    #[tokio::test]
    #[serial_test::serial]
    async fn poll_until_terminal_returns_immediately_when_cancelled() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        mount_engagement_list(&server, 10, "TESTING").await;

        let input = ags_runtime::runtime::execution::ResolutionInput {
            profile: None,
            namespace: Some("test-ns".to_string()),
            is_dry_run: false,
        };
        let http_client = ags_runtime::runtime::dispatch::http::build_http_client(None).unwrap();
        let context =
            ags_runtime::runtime::execution::ExecutionContext::resolve(&input, &http_client)
                .await
                .unwrap();
        let mut runtime = ags_runtime::runtime::Runtime::from_reqwest(context, http_client);

        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = poll_until_terminal(&mut runtime, "test-ns", 10, 3600, true, &cancel).await;
        match result {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("interrupted")),
            other => panic!("expected interrupted Usage error, got {other:?}"),
        }
    }
}
