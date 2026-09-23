//! CSM security-assessment API calls for `ags extend security-assessment request`.

use std::collections::BTreeMap;

use ags_protocol::catalogue::{OperationId, ServiceId};
use ags_protocol::error::RuntimeErrorKind;
use ags_protocol::event::{ProgressEvent, ProgressSink};
use ags_protocol::output::CommandOutput;
use ags_protocol::request::{CommandRequest, OutputFormat, PaginationHint, Verbosity};

use super::permission::ParsedPermission;
use crate::errors::CliError;
use crate::invocation::handlers::extend::csm_error::extract_csm_error_detail;
use ags_runtime::support::strings::strip_terminal_control_sequences;

/// No-op progress sink for internal API dispatch — matches
/// `remote_debug/debug_mode.rs`'s `SilentSink` (not reachable from this
/// module, since it's `pub(super)` there).
struct SilentSink;

impl ProgressSink for SilentSink {
    fn on_event(&mut self, _event: ProgressEvent) {}
}

// Encodes the same `get-app-endpoints` response shape as the generic
// `list-endpoints` shim's table shaper — see
// crates/ags-runtime/src/runtime/dispatch/shape_overrides/security_assessment_endpoints.rs.
// The two can't share a type across the `accelbyte-ags-cli` → `ags-runtime`
// dependency boundary; review both together if this response shape changes.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub(crate) struct Endpoint {
    pub(crate) method: String,
    pub(crate) path: String,
    #[serde(rename = "operationId")]
    pub(crate) operation_id: String,
    #[serde(rename = "requireAuthentication", default)]
    pub(crate) require_authentication: bool,
    #[serde(default)]
    pub(crate) permission: Option<EndpointPermission>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub(crate) struct EndpointPermission {
    pub(crate) resource: String,
    pub(crate) action: String,
}

/// HTTP methods that trigger the destructive-method confirmation gate.
/// `POST` is deliberately excluded, matching the Admin Portal. Single source
/// of truth for both `Endpoint::is_mutating` and
/// `confirm_mutating_endpoints`'s PUT → PATCH → DELETE warning ordering, so
/// the two can't drift apart.
pub(crate) const MUTATING_METHODS: [&str; 3] = ["PUT", "PATCH", "DELETE"];

impl Endpoint {
    /// Editable only when authenticated with no auto-discovered permission —
    /// matches the Admin Portal's editability rule.
    pub(crate) fn is_permission_editable(&self) -> bool {
        self.require_authentication && self.permission.is_none()
    }

    pub(crate) fn is_mutating(&self) -> bool {
        MUTATING_METHODS
            .iter()
            .any(|m| self.method.eq_ignore_ascii_case(m))
    }

    /// Strip terminal control sequences from every field the target app's
    /// own OpenAPI spec / gRPC reflection controls, before it ever reaches
    /// the terminal (plain-text prints in `mod.rs` or the ratatui checklist)
    /// — CONTRIBUTING.md's Security section requires this for all
    /// API-sourced display text.
    fn sanitize(&mut self) {
        self.method = strip_terminal_control_sequences(&self.method);
        self.path = strip_terminal_control_sequences(&self.path);
        self.operation_id = strip_terminal_control_sequences(&self.operation_id);
        if let Some(permission) = &mut self.permission {
            permission.resource = strip_terminal_control_sequences(&permission.resource);
            permission.action = strip_terminal_control_sequences(&permission.action);
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct EndpointInfoResult {
    #[serde(rename = "isAppRunning", default)]
    pub(crate) is_app_running: bool,
    #[serde(rename = "hasAPISpec", default)]
    pub(crate) has_api_spec: bool,
    #[serde(rename = "hasGRPCReflection", default)]
    pub(crate) has_grpc_reflection: bool,
    #[serde(rename = "maximumSelectableEndpoints", default)]
    pub(crate) maximum_selectable_endpoints: u32,
    // CSM returns `endpoints: null` (not an omitted key or `[]`) when
    // discovery is skipped, e.g. for a stopped app — plain `#[serde(default)]`
    // only covers a missing key, so an explicit `null` still needs handling.
    #[serde(default, deserialize_with = "null_or_missing_as_default")]
    pub(crate) endpoints: Vec<Endpoint>,
}

fn null_or_missing_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::de::DeserializeOwned,
{
    let value = <Option<T> as serde::Deserialize>::deserialize(deserializer)?;
    Ok(value.unwrap_or_default())
}

/// `csm/admin/security-assessment/v1/get-app-endpoints`, dispatched through
/// the generic catalogue-driven `run_command` pipeline (same pattern as
/// `remote_debug/debug_mode.rs`'s GET) rather than a hand-rolled URL — this
/// is a plain read with only a 404 special-case, so it doesn't need
/// `create_engagement`'s fine-grained raw-body error disambiguation.
pub(crate) async fn get_app_endpoints(
    runtime: &mut ags_runtime::runtime::Runtime,
    namespace: &str,
    app: &str,
) -> Result<EndpointInfoResult, CliError> {
    let mut path_params = BTreeMap::new();
    path_params.insert("namespace".to_string(), namespace.to_string());
    path_params.insert("appName".to_string(), app.to_string());

    let request = CommandRequest {
        service: ServiceId::new("csm"),
        operation_id: OperationId::new("csm/admin/security-assessment/v1/get-app-endpoints"),
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
        .map_err(|e| {
            // The pipeline's generic 404 message names the operation's
            // resource grouping ("security-assessment"), not the app —
            // restore the app-specific wording here. Every other status
            // keeps the pipeline's own classification, which is more
            // specific than our old one-size-fits-all fallback.
            if matches!(e.kind, RuntimeErrorKind::NotFound) {
                CliError::Api {
                    message: format!("App '{app}' not found in this namespace"),
                    metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                        "Check the app name and namespace",
                    ))),
                    category: crate::errors::ApiErrorCategory::NotFound,
                }
            } else {
                CliError::from(e)
            }
        })?;

    let raw_body = match output {
        CommandOutput::Service(api_output) => api_output.raw_body,
        _ => None,
    };
    let raw_body = raw_body.ok_or_else(|| CliError::Api {
        message: format!("empty endpoint discovery response for app '{app}'"),
        metadata: None,
        category: crate::errors::ApiErrorCategory::Upstream,
    })?;

    let mut discovery: EndpointInfoResult =
        serde_json::from_value(raw_body).map_err(|e| CliError::Api {
            message: format!("failed to parse endpoint discovery response for app '{app}': {e}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "This usually means CSM returned an unexpected response shape — retry, and \
                 report this if it persists",
            ))),
            category: crate::errors::ApiErrorCategory::Upstream,
        })?;
    for endpoint in &mut discovery.endpoints {
        endpoint.sanitize();
    }
    Ok(discovery)
}

/// `permission_override` is `None` unless the operator actually edited or
/// supplied one — only edited overrides are ever sent, matching the Admin
/// Portal's payload-minimization.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EndpointSelection {
    pub(crate) operation_id: String,
    pub(crate) permission_override: Option<ParsedPermission>,
}

#[derive(serde::Serialize)]
struct CreatePentestRequestBody {
    #[serde(rename = "appName")]
    app_name: String,
    endpoints: Vec<RequestEndpoint>,
}

#[derive(serde::Serialize)]
struct RequestEndpoint {
    #[serde(rename = "operationId")]
    operation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    permission: Option<RequestPermission>,
}

#[derive(serde::Serialize)]
struct RequestPermission {
    resource: String,
    action: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct Engagement {
    #[serde(rename = "engagementId")]
    pub(crate) engagement_id: i64,
    pub(crate) status: String,
}

/// `csm/admin/security-assessment/v1/create`.
pub(crate) async fn create_engagement(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    namespace: &str,
    app: &str,
    endpoints: &[EndpointSelection],
) -> Result<Engagement, CliError> {
    let encoded_ns = ags_runtime::support::strings::encode_url_path_segment(namespace, "namespace")
        .map_err(CliError::from)?;

    let url = format!(
        "{}/csm/v1/admin/namespaces/{}/pentestings",
        base_url.trim_end_matches('/'),
        encoded_ns
    );

    let body = CreatePentestRequestBody {
        app_name: app.to_string(),
        endpoints: endpoints
            .iter()
            .map(|e| RequestEndpoint {
                operation_id: e.operation_id.clone(),
                permission: e.permission_override.as_ref().map(|p| RequestPermission {
                    resource: p.resource.clone(),
                    action: p.action.clone(),
                }),
            })
            .collect(),
    };

    let response = client
        .post(&url)
        .bearer_auth(access_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| CliError::Network {
            message: format!("failed to request a security assessment for app '{app}': {e}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check your network connection and base URL",
            ))),
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(create_error(status, &body, app));
    }

    response
        .json::<Engagement>()
        .await
        .map_err(|e| CliError::Api {
            message: format!("failed to parse security assessment creation response: {e}"),
            metadata: None,
            category: crate::errors::ApiErrorCategory::Upstream,
        })
}

#[derive(Debug, Clone, serde::Deserialize)]
struct EngagementStatusEntry {
    #[serde(rename = "engagementId")]
    engagement_id: i64,
    #[serde(default)]
    status: String,
}

#[derive(Debug, serde::Deserialize)]
struct EngagementStatusListResponse {
    #[serde(default)]
    pentestings: Vec<EngagementStatusEntry>,
}

/// Poll a single engagement's live status via
/// `csm/admin/security-assessment/v1/list` — CSM has no get-by-id endpoint,
/// so `--wait` re-lists and filters client-side by `engagement_id`.
/// Deliberately duplicates the small id/status shape rather than reusing
/// `security_assessment_result::api::list_engagements`: that module's `api`
/// submodule is private to its parent, so it isn't reachable from here.
pub(crate) async fn get_engagement_status(
    runtime: &mut ags_runtime::runtime::Runtime,
    namespace: &str,
    engagement_id: i64,
) -> Result<String, CliError> {
    let mut path_params = BTreeMap::new();
    path_params.insert("namespace".to_string(), namespace.to_string());

    let request = CommandRequest {
        service: ServiceId::new("csm"),
        operation_id: OperationId::new("csm/admin/security-assessment/v1/list"),
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
    let output = runtime.run_command(&request, &mut sink).await?;

    let raw_body = match output {
        CommandOutput::Service(api_output) => api_output.raw_body,
        _ => None,
    };
    let raw_body = raw_body.ok_or_else(|| CliError::Api {
        message: format!(
            "empty security assessment list response while waiting on engagement #{engagement_id}"
        ),
        metadata: None,
        category: crate::errors::ApiErrorCategory::Upstream,
    })?;

    let response: EngagementStatusListResponse =
        serde_json::from_value(raw_body).map_err(|e| CliError::Api {
            message: format!("failed to parse security assessment list response: {e}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "This usually means CSM returned an unexpected response shape — retry, and \
                 report this if it persists",
            ))),
            category: crate::errors::ApiErrorCategory::Upstream,
        })?;

    let entry = response
        .pentestings
        .into_iter()
        .find(|e| e.engagement_id == engagement_id)
        .ok_or_else(|| CliError::Api {
            message: format!(
                "engagement #{engagement_id} no longer appears in the security assessment list"
            ),
            metadata: None,
            category: crate::errors::ApiErrorCategory::Upstream,
        })?;

    Ok(strip_terminal_control_sequences(&entry.status))
}

/// CSM's error envelope carries a generic `errorCode` for every pentest
/// failure, so disambiguation is status-first with a substring match on the
/// known sentinel messages for the 400s. Wording matches the Admin Portal's
/// own copy for the same failures.
fn create_error(status: reqwest::StatusCode, body: &str, app: &str) -> CliError {
    let lower = body.to_lowercase();
    let message = if status == reqwest::StatusCode::CONFLICT {
        format!("A security assessment is already running for '{app}'.")
    } else if status == reqwest::StatusCode::BAD_REQUEST {
        if lower.contains("extend app is not running") {
            format!("'{app}' isn't running.")
        } else if lower.contains("not present in the app's openapi spec") {
            "The app's API spec has changed since endpoints were discovered. Run 'list-endpoints' again and retry.".to_string()
        } else if lower.contains("exceeds the maximum allowed") {
            "Too many endpoints selected — check the discovery response's maximumSelectableEndpoints limit.".to_string()
        } else if lower.contains("exceed what the game admin role grants") {
            "One or more permissions exceed what the Game Admin role allows.".to_string()
        } else if lower.contains("not scoped to this namespace")
            || lower.contains("exceed the endpoint's discovered permission")
        {
            "One or more permission overrides are invalid.".to_string()
        } else {
            format!(
                "Couldn't submit the security assessment request.{}",
                extract_csm_error_detail(body)
            )
        }
    } else {
        format!(
            "Couldn't submit the security assessment request.{}",
            extract_csm_error_detail(body)
        )
    };

    CliError::Api {
        message,
        metadata: None,
        category: crate::errors::ApiErrorCategory::Upstream,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    fn client() -> reqwest::Client {
        reqwest::Client::new()
    }

    /// Isolates `AGS_HOME` for the lifetime of the guard so
    /// `Catalogue::get_or_load` (reached via `Runtime::run_command`) reads
    /// from a throwaway parsed-schema cache instead of whatever real cache
    /// happens to exist on the machine running the test — without this, a
    /// stale on-disk cache missing this PR's new operations makes these
    /// tests fail nondeterministically depending on developer machine state.
    struct AgsHomeGuard {
        original: Option<String>,
    }

    impl AgsHomeGuard {
        fn isolated(tmp: &tempfile::TempDir) -> Self {
            let original = std::env::var("AGS_HOME").ok();
            std::env::set_var("AGS_HOME", tmp.path());
            Self { original }
        }
    }

    impl Drop for AgsHomeGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(v) => std::env::set_var("AGS_HOME", v),
                None => std::env::remove_var("AGS_HOME"),
            }
        }
    }

    /// A `Runtime` wired to `base_url`, for `run_command`-based tests.
    /// Built directly (not via `ExecutionContext::resolve`) so these tests
    /// stay wiremock-only, with no env vars / `#[serial]` needed.
    fn test_runtime(base_url: &str) -> ags_runtime::runtime::Runtime {
        use ags_runtime::runtime::execution::{
            AccessTokenSource, BaseUrlSource, ExecutionContext, NamespaceSource, ProfileSource,
        };
        let context = ExecutionContext {
            profile: "default".to_string(),
            profile_source: ProfileSource::GlobalConfig,
            namespace: Some("ns1".to_string()),
            namespace_source: Some(NamespaceSource::Flag),
            base_url: base_url.to_string(),
            base_url_source: BaseUrlSource::Environment,
            access_token: "token".to_string(),
            access_token_source: AccessTokenSource::Environment,
            access_token_expiry: None,
            access_token_warnings: Vec::new(),
        };
        ags_runtime::runtime::Runtime::from_reqwest(context, reqwest::Client::new())
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn get_app_endpoints_parses_success_response() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = AgsHomeGuard::isolated(&tmp);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v1/admin/namespaces/ns1/pentestings/apps/my-app/endpoints",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "isAppRunning": true,
                "hasAPISpec": true,
                "hasGRPCReflection": false,
                "maximumSelectableEndpoints": 10,
                "endpoints": [
                    {
                        "method": "GET",
                        "path": "/users",
                        "operationId": "op-1",
                        "requireAuthentication": true,
                        "permission": {"resource": "NAMESPACE:ns1:USER", "action": "READ"}
                    },
                    {
                        "method": "DELETE",
                        "path": "/users/{id}",
                        "operationId": "op-2",
                        "requireAuthentication": true
                    },
                    {
                        "method": "GET",
                        "path": "/health",
                        "operationId": "op-3",
                        "requireAuthentication": false
                    }
                ]
            })))
            .mount(&server)
            .await;

        let mut runtime = test_runtime(&server.uri());
        let result = get_app_endpoints(&mut runtime, "ns1", "my-app")
            .await
            .unwrap();

        assert!(result.is_app_running);
        assert!(result.has_api_spec);
        assert!(!result.has_grpc_reflection);
        assert_eq!(result.maximum_selectable_endpoints, 10);
        assert_eq!(result.endpoints.len(), 3);
        assert!(!result.endpoints[0].is_permission_editable());
        assert!(result.endpoints[1].is_permission_editable());
        assert!(!result.endpoints[2].is_permission_editable());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn get_app_endpoints_maps_404_to_not_found_error() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = AgsHomeGuard::isolated(&tmp);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v1/admin/namespaces/ns1/pentestings/apps/missing/endpoints",
            ))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "errorCode": 20024,
                "errorMessage": "not found"
            })))
            .mount(&server)
            .await;

        let mut runtime = test_runtime(&server.uri());
        let err = get_app_endpoints(&mut runtime, "ns1", "missing")
            .await
            .unwrap_err();
        match err {
            CliError::Api {
                message,
                category: crate::errors::ApiErrorCategory::NotFound,
                ..
            } => assert_eq!(message, "App 'missing' not found in this namespace"),
            other => panic!("expected Api/NotFound error, got {other:?}"),
        }
    }

    // An app owner controls the OpenAPI spec / gRPC reflection CSM's
    // discovery response is built from — terminal escape sequences embedded
    // there must never reach the confirmation prompt or checklist verbatim.
    #[tokio::test]
    #[serial_test::serial]
    async fn get_app_endpoints_strips_terminal_control_sequences() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = AgsHomeGuard::isolated(&tmp);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v1/admin/namespaces/ns1/pentestings/apps/my-app/endpoints",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "isAppRunning": true,
                "hasAPISpec": true,
                "hasGRPCReflection": true,
                "maximumSelectableEndpoints": 10,
                "endpoints": [
                    {
                        "method": "GET",
                        "path": "/users\u{1b}]0;PWNED\u{7}",
                        "operationId": "op-1\u{1b}[31m",
                        "requireAuthentication": true,
                        "permission": {
                            "resource": "NAMESPACE:ns1:USER\u{1b}[0m",
                            "action": "READ\u{1b}[2A"
                        }
                    }
                ]
            })))
            .mount(&server)
            .await;

        let mut runtime = test_runtime(&server.uri());
        let result = get_app_endpoints(&mut runtime, "ns1", "my-app")
            .await
            .unwrap();

        let endpoint = &result.endpoints[0];
        assert_eq!(endpoint.path, "/users");
        assert_eq!(endpoint.operation_id, "op-1");
        let permission = endpoint.permission.as_ref().unwrap();
        assert_eq!(permission.resource, "NAMESPACE:ns1:USER");
        assert_eq!(permission.action, "READ");
    }

    // `endpoints: null` is CSM's actual shape for a stopped app — must not
    // fail deserialization (see `null_or_missing_as_default`).
    #[tokio::test]
    #[serial_test::serial]
    async fn get_app_endpoints_treats_null_endpoints_as_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = AgsHomeGuard::isolated(&tmp);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v1/admin/namespaces/ns1/pentestings/apps/stopped-app/endpoints",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "isAppRunning": false,
                "hasAPISpec": false,
                "hasGRPCReflection": false,
                "maximumSelectableEndpoints": 10,
                "endpoints": null
            })))
            .mount(&server)
            .await;

        let mut runtime = test_runtime(&server.uri());
        let result = get_app_endpoints(&mut runtime, "ns1", "stopped-app")
            .await
            .expect("null `endpoints` must not fail deserialization");
        assert!(!result.is_app_running);
        assert!(result.endpoints.is_empty());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn get_engagement_status_finds_matching_id() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = AgsHomeGuard::isolated(&tmp);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v1/admin/namespaces/ns1/pentestings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "pentestings": [
                    {"engagementId": 1, "targetApp": "my-app", "status": "COMPLETED"},
                    {"engagementId": 7, "targetApp": "my-app", "status": "TESTING"},
                ]
            })))
            .mount(&server)
            .await;

        let mut runtime = test_runtime(&server.uri());
        let status = get_engagement_status(&mut runtime, "ns1", 7).await.unwrap();
        assert_eq!(status, "TESTING");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn get_engagement_status_missing_id_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = AgsHomeGuard::isolated(&tmp);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v1/admin/namespaces/ns1/pentestings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "pentestings": []
            })))
            .mount(&server)
            .await;

        let mut runtime = test_runtime(&server.uri());
        let err = get_engagement_status(&mut runtime, "ns1", 99)
            .await
            .unwrap_err();
        match err {
            CliError::Api { message, .. } => assert!(message.contains("99")),
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_engagement_parses_success_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/csm/v1/admin/namespaces/ns1/pentestings"))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "engagementId": 42,
                "status": "SUBMITTED",
                "originalStatus": "queued",
                "targetApp": "my-app",
                "targetNamespace": "ns1",
                "targetAppVersion": "v1"
            })))
            .mount(&server)
            .await;

        let selections = vec![EndpointSelection {
            operation_id: "op-1".to_string(),
            permission_override: None,
        }];
        let engagement = create_engagement(
            &client(),
            &server.uri(),
            "token",
            "ns1",
            "my-app",
            &selections,
        )
        .await
        .unwrap();

        assert_eq!(engagement.engagement_id, 42);
        assert_eq!(engagement.status, "SUBMITTED");
    }

    #[tokio::test]
    async fn create_engagement_maps_409_to_active_session_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/csm/v1/admin/namespaces/ns1/pentestings"))
            .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
                "errorCode": 20024,
                "errorMessage": "an engagement is already active"
            })))
            .mount(&server)
            .await;

        let err = create_engagement(&client(), &server.uri(), "token", "ns1", "my-app", &[])
            .await
            .unwrap_err();
        match err {
            CliError::Api { message, .. } => {
                assert_eq!(
                    message,
                    "A security assessment is already running for 'my-app'."
                )
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_engagement_maps_app_not_running_400() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/csm/v1/admin/namespaces/ns1/pentestings"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "errorCode": 20024,
                "errorMessage": "extend app is not running"
            })))
            .mount(&server)
            .await;

        let err = create_engagement(&client(), &server.uri(), "token", "ns1", "my-app", &[])
            .await
            .unwrap_err();
        match err {
            CliError::Api { message, .. } => assert_eq!(message, "'my-app' isn't running."),
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_engagement_maps_spec_changed_400() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/csm/v1/admin/namespaces/ns1/pentestings"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "errorCode": 20024,
                "errorMessage": "operation is not present in the app's OpenAPI spec"
            })))
            .mount(&server)
            .await;

        let err = create_engagement(&client(), &server.uri(), "token", "ns1", "my-app", &[])
            .await
            .unwrap_err();
        match err {
            CliError::Api { message, .. } => assert!(message.contains("API spec has changed")),
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_engagement_maps_permission_invalid_400() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/csm/v1/admin/namespaces/ns1/pentestings"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "errorCode": 20024,
                "errorMessage": "permission is not scoped to this namespace"
            })))
            .mount(&server)
            .await;

        let err = create_engagement(&client(), &server.uri(), "token", "ns1", "my-app", &[])
            .await
            .unwrap_err();
        match err {
            CliError::Api { message, .. } => {
                assert_eq!(message, "One or more permission overrides are invalid.")
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_engagement_maps_unrecognised_400_to_generic() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/csm/v1/admin/namespaces/ns1/pentestings"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "errorCode": 20024,
                "errorMessage": "something else entirely"
            })))
            .mount(&server)
            .await;

        let err = create_engagement(&client(), &server.uri(), "token", "ns1", "my-app", &[])
            .await
            .unwrap_err();
        match err {
            CliError::Api { message, .. } => {
                assert!(message.starts_with("Couldn't submit the security assessment request."));
                assert!(message.contains("something else entirely"));
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    struct AssertRequestBody;

    impl Respond for AssertRequestBody {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            let endpoints = body["endpoints"].as_array().unwrap();
            assert_eq!(endpoints.len(), 2);
            assert_eq!(endpoints[0]["operationId"], "op-1");
            assert!(endpoints[0].get("permission").is_none());
            assert_eq!(endpoints[1]["operationId"], "op-2");
            assert_eq!(endpoints[1]["permission"]["resource"], "NAMESPACE:ns1:USER");
            assert_eq!(endpoints[1]["permission"]["action"], "DELETE");
            ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "engagementId": 1,
                "status": "SUBMITTED",
                "originalStatus": "queued",
                "targetApp": "my-app",
                "targetNamespace": "ns1",
                "targetAppVersion": "v1"
            }))
        }
    }

    #[tokio::test]
    async fn create_engagement_sends_operation_id_and_permission_override_only_when_present() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/csm/v1/admin/namespaces/ns1/pentestings"))
            .respond_with(AssertRequestBody)
            .mount(&server)
            .await;

        let selections = vec![
            EndpointSelection {
                operation_id: "op-1".to_string(),
                permission_override: None,
            },
            EndpointSelection {
                operation_id: "op-2".to_string(),
                permission_override: Some(ParsedPermission {
                    resource: "NAMESPACE:ns1:USER".to_string(),
                    action: "DELETE".to_string(),
                }),
            },
        ];

        create_engagement(
            &client(),
            &server.uri(),
            "token",
            "ns1",
            "my-app",
            &selections,
        )
        .await
        .unwrap();
    }
}
