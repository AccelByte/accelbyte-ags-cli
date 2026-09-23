//! CSM security-assessment API calls for `ags extend security-assessment result`.

use std::collections::BTreeMap;

use ags_protocol::catalogue::{OperationId, ServiceId};
use ags_protocol::error::RuntimeErrorKind;
use ags_protocol::event::{ProgressEvent, ProgressSink};
use ags_protocol::output::CommandOutput;
use ags_protocol::request::{CommandRequest, OutputFormat, PaginationHint, Verbosity};

use crate::errors::CliError;
use ags_runtime::support::strings::strip_terminal_control_sequences;
use futures_util::StreamExt;

/// No-op progress sink for internal API dispatch — matches
/// `security_assessment_request/api.rs`'s `SilentSink`.
struct SilentSink;

impl ProgressSink for SilentSink {
    fn on_event(&mut self, _event: ProgressEvent) {}
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub(crate) struct EngagementSummary {
    #[serde(rename = "engagementId")]
    pub(crate) engagement_id: i64,
    #[serde(rename = "targetApp")]
    pub(crate) target_app: String,
    #[serde(rename = "targetAppVersion", default)]
    pub(crate) target_app_version: Option<String>,
    #[serde(rename = "createdAt", default)]
    pub(crate) created_at: Option<String>,
    #[serde(default)]
    pub(crate) status: String,
}

impl EngagementSummary {
    /// Strip terminal control sequences from every field CSM (ultimately the
    /// target app's own metadata) controls, before it reaches the terminal
    /// via `pick_engagement_impl`'s picker listing — CONTRIBUTING.md's
    /// Security section requires this for all API-sourced display text.
    fn sanitize(&mut self) {
        self.target_app = strip_terminal_control_sequences(&self.target_app);
        if let Some(version) = &mut self.target_app_version {
            *version = strip_terminal_control_sequences(version);
        }
        if let Some(created_at) = &mut self.created_at {
            *created_at = strip_terminal_control_sequences(created_at);
        }
        self.status = strip_terminal_control_sequences(&self.status);
    }
}

/// The only status a report can be fetched for — `get-report` 404s until an
/// engagement reaches this status, so the picker only lists these.
const COMPLETED_STATUS: &str = "COMPLETED";

/// Format a `createdAt` for display, matching `ags extend
/// security-assessment list`'s "Requested At" column. Falls back to `"—"`
/// when absent — the shared formatter already falls back to the raw string
/// on parse failure.
pub(crate) fn format_created_at(raw: Option<&str>) -> String {
    match raw {
        Some(raw) => ags_runtime::support::time::format_rfc3339_human(raw),
        None => "—".to_string(),
    }
}

#[derive(Debug, serde::Deserialize)]
struct EngagementListResponse {
    #[serde(default)]
    pentestings: Vec<EngagementSummary>,
}

/// `csm/admin/security-assessment/v1/list`, filtered client-side to `app`
/// and `COMPLETED` status — the API has no server-side app filter, and a
/// report can only be fetched once an engagement has completed.
pub(crate) async fn list_engagements(
    runtime: &mut ags_runtime::runtime::Runtime,
    namespace: &str,
    app: &str,
) -> Result<Vec<EngagementSummary>, CliError> {
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
        message: format!("empty security assessment list response for app '{app}'"),
        metadata: None,
        category: crate::errors::ApiErrorCategory::Upstream,
    })?;

    let mut response: EngagementListResponse =
        serde_json::from_value(raw_body).map_err(|e| CliError::Api {
            message: format!("failed to parse security assessment list response: {e}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "This usually means CSM returned an unexpected response shape — retry, and \
                 report this if it persists",
            ))),
            category: crate::errors::ApiErrorCategory::Upstream,
        })?;
    for engagement in &mut response.pentestings {
        engagement.sanitize();
    }

    Ok(response
        .pentestings
        .into_iter()
        .filter(|e| e.target_app == app && e.status == COMPLETED_STATUS)
        .collect())
}

#[derive(Debug, serde::Deserialize)]
struct ReportResponse {
    url: String,
}

/// `csm/admin/security-assessment/v1/get-report`.
pub(crate) async fn get_report(
    runtime: &mut ags_runtime::runtime::Runtime,
    namespace: &str,
    engagement_id: &str,
    format: &str,
) -> Result<String, CliError> {
    let mut path_params = BTreeMap::new();
    path_params.insert("namespace".to_string(), namespace.to_string());
    path_params.insert("engagementId".to_string(), engagement_id.to_string());

    let mut query_params = BTreeMap::new();
    query_params.insert("format".to_string(), format.to_string());

    let request = CommandRequest {
        service: ServiceId::new("csm"),
        operation_id: OperationId::new("csm/admin/security-assessment/v1/get-report"),
        namespace: Some(namespace.to_string()),
        path_params,
        query_params,
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
            // The pipeline's generic 404 message names the operation's resource
            // grouping ("security-assessment"), not the engagement — restore
            // engagement-specific wording, including the COMPLETED-only
            // constraint the generic message can't know about.
            if matches!(e.kind, RuntimeErrorKind::NotFound) {
                CliError::Api {
                    message: format!(
                        "Engagement '{engagement_id}' not found, or its report isn't ready yet"
                    ),
                    metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                        "The report is only available once the engagement's status is COMPLETED — \
                     check 'ags extend security-assessment list'",
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
        message: format!("empty report response for engagement '{engagement_id}'"),
        metadata: None,
        category: crate::errors::ApiErrorCategory::Upstream,
    })?;

    let response: ReportResponse = serde_json::from_value(raw_body).map_err(|e| CliError::Api {
        message: format!("failed to parse report response: {e}"),
        metadata: None,
        category: crate::errors::ApiErrorCategory::Upstream,
    })?;

    Ok(response.url)
}

/// Hard cap on the in-memory report buffer. Pen-test reports (PDF/markdown,
/// covering a bounded set of endpoints) are expected to stay in the
/// tens-of-KB-to-low-MB range; 50 MiB is generous headroom while still
/// bounding the allocation against a pre-signed URL response whose size the
/// CLI does not control.
const MAX_REPORT_BYTES: u64 = 50 * 1024 * 1024;

fn report_too_large_error(limit: u64, detail: &str) -> CliError {
    CliError::Api {
        message: format!(
            "security assessment report exceeds the {}MB size limit ({detail})",
            limit / (1024 * 1024)
        ),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            "This is larger than any report this command expects — please report this if it \
             persists",
        ))),
        category: crate::errors::ApiErrorCategory::Upstream,
    }
}

/// GET the pre-signed report URL and return its raw bytes. No bearer token
/// is sent — the URL itself carries a temporary signed grant, mirroring the
/// Admin Portal's own `window.open(url)`.
pub(crate) async fn download_report_bytes(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<u8>, CliError> {
    download_report_bytes_capped(client, url, MAX_REPORT_BYTES).await
}

/// `limit`-parameterized so tests can exercise the size-cap logic (both the
/// `Content-Length` short-circuit and the streaming enforcement) without
/// transferring anything close to the real 50 MiB cap.
async fn download_report_bytes_capped(
    client: &reqwest::Client,
    url: &str,
    limit: u64,
) -> Result<Vec<u8>, CliError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| CliError::Network {
            message: format!("failed to download the security assessment report: {e}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check your network connection",
            ))),
        })?;

    let status = response.status();
    if !status.is_success() {
        return Err(CliError::Api {
            message: format!("report download failed with HTTP {}", status.as_u16()),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "The pre-signed report URL may have expired (1-hour validity) — re-run \
                 'ags extend security-assessment result' to fetch a fresh one",
            ))),
            category: crate::errors::ApiErrorCategory::Upstream,
        });
    }

    // Reject up front when the server tells us the size, before reading any
    // body at all — the streaming cap below still catches an absent/lying
    // `Content-Length` (e.g. chunked transfer).
    if let Some(len) = response.content_length() {
        if len > limit {
            return Err(report_too_large_error(
                limit,
                &format!("{len} bytes reported by Content-Length"),
            ));
        }
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| CliError::Network {
            message: format!("failed to read the security assessment report body: {e}"),
            metadata: None,
        })?;
        if bytes.len() as u64 + chunk.len() as u64 > limit {
            return Err(report_too_large_error(
                limit,
                "size limit exceeded while downloading",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A `Runtime` wired to `base_url`, for `run_command`-based tests. Built
    /// directly (not via `ExecutionContext::resolve`) so these tests stay
    /// wiremock-only. `run_command` still reaches `Catalogue::get_or_load`
    /// for operation resolution, which reads the on-disk parsed-schema
    /// cache — callers that exercise it need `AgsHomeGuard` below to avoid
    /// depending on the developer machine's real cache state.
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

    // The RFC 3339 parsing/formatting itself is covered by
    // `ags_runtime::support::time`'s own tests — only `format_created_at`'s
    // own `None` handling is unique to this module.
    #[test]
    fn format_created_at_falls_back_to_dash_when_absent() {
        assert_eq!(format_created_at(None), "—");
    }

    #[test]
    fn format_created_at_delegates_to_shared_formatter() {
        assert_eq!(
            format_created_at(Some("2026-08-10T10:17:56Z")),
            "Aug 10, 2026, 10:17:56 UTC"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn list_engagements_filters_by_target_app_and_completed_status() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = AgsHomeGuard::isolated(&tmp);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v1/admin/namespaces/ns1/pentestings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "pentestings": [
                    {
                        "engagementId": 1,
                        "targetApp": "my-app",
                        "targetAppVersion": "v1",
                        "createdAt": "2026-08-10T10:17:56Z",
                        "status": "COMPLETED",
                        "originalStatus": "completed",
                        "targetNamespace": "ns1"
                    },
                    {
                        "engagementId": 2,
                        "targetApp": "other-app",
                        "status": "COMPLETED",
                        "originalStatus": "completed",
                        "targetNamespace": "ns1"
                    },
                    {
                        "engagementId": 3,
                        "targetApp": "my-app",
                        "status": "RUNNING",
                        "originalStatus": "running",
                        "targetNamespace": "ns1"
                    }
                ]
            })))
            .mount(&server)
            .await;

        let mut runtime = test_runtime(&server.uri());
        let result = list_engagements(&mut runtime, "ns1", "my-app")
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].engagement_id, 1);
        assert_eq!(result[0].target_app_version.as_deref(), Some("v1"));
    }

    // `targetApp`/`createdAt`/`targetAppVersion`/`status` are CSM-reported
    // metadata ultimately controlled by the target app owner — terminal
    // escape sequences embedded there must never reach the engagement
    // picker verbatim.
    #[tokio::test]
    #[serial_test::serial]
    async fn list_engagements_strips_terminal_control_sequences() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = AgsHomeGuard::isolated(&tmp);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v1/admin/namespaces/ns1/pentestings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "pentestings": [
                    {
                        "engagementId": 1,
                        "targetApp": "my-app\u{1b}]0;PWNED\u{7}",
                        "targetAppVersion": "v1\u{1b}[31m",
                        "createdAt": "2026-08-10T10:17:56Z\u{1b}[0m",
                        "status": "COMPLETED\u{1b}[2A",
                        "originalStatus": "completed",
                        "targetNamespace": "ns1"
                    }
                ]
            })))
            .mount(&server)
            .await;

        let mut runtime = test_runtime(&server.uri());
        // `app` here must match the *sanitized* `targetApp` — the filter
        // compares against the cleaned value, not the raw wire value.
        let result = list_engagements(&mut runtime, "ns1", "my-app")
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].target_app, "my-app");
        assert_eq!(result[0].target_app_version.as_deref(), Some("v1"));
        // `status` still equals "COMPLETED" after stripping, so filtering by
        // status still works on the sanitized value.
        assert_eq!(result[0].status, "COMPLETED");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn get_report_parses_url() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = AgsHomeGuard::isolated(&tmp);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v1/admin/namespaces/ns1/pentestings/42/report"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/report.pdf"
            })))
            .mount(&server)
            .await;

        let mut runtime = test_runtime(&server.uri());
        let url = get_report(&mut runtime, "ns1", "42", "pdf").await.unwrap();
        assert_eq!(url, "https://example.com/report.pdf");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn get_report_maps_404_to_friendly_message() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = AgsHomeGuard::isolated(&tmp);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v1/admin/namespaces/ns1/pentestings/42/report"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "errorCode": 20024,
                "errorMessage": "not found"
            })))
            .mount(&server)
            .await;

        let mut runtime = test_runtime(&server.uri());
        let err = get_report(&mut runtime, "ns1", "42", "pdf")
            .await
            .unwrap_err();
        match err {
            CliError::Api {
                message,
                category: crate::errors::ApiErrorCategory::NotFound,
                ..
            } => assert!(message.contains("42")),
            other => panic!("expected Api/NotFound error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn download_report_bytes_returns_body_on_success() {
        let server = MockServer::start().await;
        let body = b"%PDF-1.4 fake report bytes";
        Mock::given(method("GET"))
            .and(path("/report.pdf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.as_slice()))
            .mount(&server)
            .await;

        let bytes = download_report_bytes(
            &reqwest::Client::new(),
            &format!("{}/report.pdf", server.uri()),
        )
        .await
        .unwrap();
        assert_eq!(bytes, body);
    }

    #[tokio::test]
    async fn download_report_bytes_maps_non_2xx_to_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/expired.pdf"))
            .respond_with(ResponseTemplate::new(403).set_body_string("Forbidden"))
            .mount(&server)
            .await;

        let err = download_report_bytes(
            &reqwest::Client::new(),
            &format!("{}/expired.pdf", server.uri()),
        )
        .await
        .unwrap_err();
        match err {
            CliError::Api { message, .. } => assert!(message.contains("403")),
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    // A body over the cap must be rejected rather than buffered in full.
    // wiremock always reports an accurate `Content-Length` for a static
    // mock body (hyper itself rejects a mismatched declared length), so
    // this exercises the `Content-Length` short-circuit — the more common
    // real-world path, since CSM's pre-signed URL responses report one.
    // The streaming accumulator below it is the same-shaped fallback for a
    // response with no reliable `Content-Length` (e.g. chunked transfer),
    // which isn't reproducible through a static wiremock body.
    #[tokio::test]
    async fn download_report_bytes_capped_rejects_body_over_limit() {
        let server = MockServer::start().await;
        let body = vec![b'x'; 200];
        Mock::given(method("GET"))
            .and(path("/large.pdf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;

        let err = download_report_bytes_capped(
            &reqwest::Client::new(),
            &format!("{}/large.pdf", server.uri()),
            100,
        )
        .await
        .unwrap_err();
        match err {
            CliError::Api { message, .. } => {
                assert!(message.contains("Content-Length"));
                assert!(message.contains("0MB")); // 100 / (1024*1024) == 0
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    // A body within the cap must still download successfully.
    #[tokio::test]
    async fn download_report_bytes_capped_allows_body_within_limit() {
        let server = MockServer::start().await;
        let body = b"within the limit";
        Mock::given(method("GET"))
            .and(path("/ok.pdf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.as_slice()))
            .mount(&server)
            .await;

        let bytes = download_report_bytes_capped(
            &reqwest::Client::new(),
            &format!("{}/ok.pdf", server.uri()),
            100,
        )
        .await
        .unwrap();
        assert_eq!(bytes, body);
    }
}
