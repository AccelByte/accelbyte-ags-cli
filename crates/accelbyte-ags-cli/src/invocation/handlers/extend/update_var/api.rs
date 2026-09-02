//! CSM variable API calls for `ags extend update-var`.
//!
//! Plain `reqwest`-based functions that accept `client`/`base_url`/
//! `access_token` as parameters (rather than resolving them internally),
//! matching the pattern in `app_ui/setup_env.rs`'s `fetch_app_ui_record_paged`
//! — this keeps the functions cheap to test against a `wiremock::MockServer`.

use crate::errors::CliError;
use crate::invocation::handlers::extend::csm_error::extract_csm_error_detail;

/// A CSM app configuration variable, as returned by `GetListOfVariablesV2`,
/// `SaveVariableV2`, and `UpdateVariableV2`.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub(crate) struct VariableRecord {
    #[serde(rename = "configId")]
    pub(crate) config_id: String,
    #[serde(rename = "configName")]
    pub(crate) config_name: String,
    #[serde(rename = "applyMask", default)]
    pub(crate) apply_mask: bool,
    #[serde(default)]
    pub(crate) description: Option<String>,
}

#[derive(serde::Deserialize)]
struct ListVariablesResponse {
    #[serde(default)]
    data: Vec<VariableRecord>,
}

/// Page size used when walking `GetListOfVariablesV2`.
const PAGE_LIMIT: u64 = 100;

/// Maximum number of pages to walk before giving up. Prevents infinite
/// loops when a misbehaving server always returns a full page.
const MAX_PAGES: u64 = 50;

/// `GetListOfVariablesV2` — page through variables looking for `target_key`.
///
/// Stops early as soon as the page containing `target_key` is fetched.
/// If the key is genuinely absent (a short page was seen), returns all
/// accumulated records so the caller can treat `.find() → None` as "not
/// found." If the page cap is exhausted without a short page or a match,
/// returns an error — the list may be truncated and the result unreliable.
pub(crate) async fn list_variables(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    namespace: &str,
    app: &str,
    target_key: &str,
) -> Result<Vec<VariableRecord>, CliError> {
    list_variables_paged(
        client,
        base_url,
        access_token,
        namespace,
        app,
        target_key,
        PAGE_LIMIT,
        MAX_PAGES,
    )
    .await
}

/// Inner implementation of [`list_variables`] that accepts pagination
/// parameters. Production callers use the constants via the wrapper; tests
/// can pass smaller values to keep mock data manageable.
#[allow(clippy::too_many_arguments)]
async fn list_variables_paged(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    namespace: &str,
    app: &str,
    target_key: &str,
    page_limit: u64,
    max_pages: u64,
) -> Result<Vec<VariableRecord>, CliError> {
    // Encode path segments per the CONTRIBUTING.md convention: never
    // interpolate user input into URL paths without encoding.
    let encoded_ns = ags_runtime::support::strings::encode_url_path_segment(namespace, "namespace")
        .map_err(CliError::from)?;
    let encoded_app = ags_runtime::support::strings::encode_url_path_segment(app, "app")
        .map_err(CliError::from)?;

    let url = format!(
        "{}/csm/v2/admin/namespaces/{}/apps/{}/variables",
        base_url.trim_end_matches('/'),
        encoded_ns,
        encoded_app
    );

    let mut records = Vec::new();
    let mut offset: u64 = 0;
    let mut saw_short_page = false;

    for _ in 0..max_pages {
        let response = client
            .get(&url)
            .query(&[
                ("limit", &page_limit.to_string()),
                ("offset", &offset.to_string()),
            ])
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| CliError::Network {
                message: format!("failed to list CSM variables: {e}"),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Check your network connection and base URL",
                ))),
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail = extract_csm_error_detail(&body);
            return Err(CliError::Api {
                message: format!(
                    "CSM GetListOfVariablesV2 returned HTTP {status} for namespace '{namespace}' app '{app}'{detail}"
                ),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Check the namespace, app name, and your permissions",
                ))),
                category: crate::errors::ApiErrorCategory::Upstream,
            });
        }

        let body: ListVariablesResponse = response.json().await.map_err(|e| CliError::Api {
            message: format!("failed to parse CSM variables list response: {e}"),
            metadata: None,
            category: crate::errors::ApiErrorCategory::Upstream,
        })?;

        let page_len = body.data.len() as u64;
        let key_found = body.data.iter().any(|r| r.config_name == target_key);
        records.extend(body.data);

        if key_found {
            return Ok(records);
        }

        if page_len < page_limit {
            saw_short_page = true;
            break;
        }
        offset += page_len;
    }

    if saw_short_page {
        // Key genuinely absent — all records returned; caller's .find() → None.
        Ok(records)
    } else {
        // Cap exhaustion: every page was full and the key was never seen.
        Err(CliError::Api {
            message: format!(
                "variable '{target_key}' was not found within the first {offset} records \
                 in namespace '{namespace}' app '{app}'"
            ),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "The app may contain more variables than were searched. \
                 Check the variable name or contact your administrator.",
            ))),
            category: crate::errors::ApiErrorCategory::Upstream,
        })
    }
}

#[derive(serde::Serialize)]
struct SaveVariableRequest<'a> {
    #[serde(rename = "configName")]
    config_name: &'a str,
    value: &'a str,
    #[serde(rename = "applyMask")]
    apply_mask: bool,
    description: Option<&'a str>,
    /// Required by `apimodel.SaveConfigurationV2Request`. The legacy Go CLI
    /// hardcodes this to `"plaintext"` on create and never sends it on
    /// update (see `extend-helper-cli/internal/cmd/update_var.go`).
    source: &'a str,
}

#[derive(serde::Serialize)]
struct UpdateVariableRequest<'a> {
    value: &'a str,
    #[serde(rename = "applyMask")]
    apply_mask: bool,
    description: Option<&'a str>,
}

/// `SaveVariableV2` — create a new config variable.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_variable(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    namespace: &str,
    app: &str,
    key: &str,
    value: &str,
    apply_mask: bool,
    description: Option<&str>,
) -> Result<VariableRecord, CliError> {
    let encoded_ns = ags_runtime::support::strings::encode_url_path_segment(namespace, "namespace")
        .map_err(CliError::from)?;
    let encoded_app = ags_runtime::support::strings::encode_url_path_segment(app, "app")
        .map_err(CliError::from)?;

    let url = format!(
        "{}/csm/v2/admin/namespaces/{}/apps/{}/variables",
        base_url.trim_end_matches('/'),
        encoded_ns,
        encoded_app
    );
    let response = client
        .post(&url)
        .bearer_auth(access_token)
        .json(&SaveVariableRequest {
            config_name: key,
            value,
            apply_mask,
            description,
            source: "plaintext",
        })
        .send()
        .await
        .map_err(|e| CliError::Network {
            message: format!("failed to create CSM variable '{key}': {e}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check your network connection and base URL",
            ))),
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let detail = extract_csm_error_detail(&body);
        return Err(CliError::Api {
            message: format!(
                "CSM SaveVariableV2 returned HTTP {status} for variable '{key}' in namespace '{namespace}'{detail}"
            ),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check the namespace, app name, and your permissions",
            ))),
            category: crate::errors::ApiErrorCategory::Upstream,
        });
    }

    response.json().await.map_err(|e| CliError::Api {
        message: format!("failed to parse CSM SaveVariableV2 response: {e}"),
        metadata: None,
        category: crate::errors::ApiErrorCategory::Upstream,
    })
}

/// `UpdateVariableV2` — update an existing config variable by `configId`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_variable(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    namespace: &str,
    app: &str,
    config_id: &str,
    value: &str,
    apply_mask: bool,
    description: Option<&str>,
) -> Result<VariableRecord, CliError> {
    let encoded_ns = ags_runtime::support::strings::encode_url_path_segment(namespace, "namespace")
        .map_err(CliError::from)?;
    let encoded_app = ags_runtime::support::strings::encode_url_path_segment(app, "app")
        .map_err(CliError::from)?;
    let encoded_id = ags_runtime::support::strings::encode_url_path_segment(config_id, "config-id")
        .map_err(CliError::from)?;

    let url = format!(
        "{}/csm/v2/admin/namespaces/{}/apps/{}/variables/{}",
        base_url.trim_end_matches('/'),
        encoded_ns,
        encoded_app,
        encoded_id
    );
    let response = client
        .put(&url)
        .bearer_auth(access_token)
        .json(&UpdateVariableRequest {
            value,
            apply_mask,
            description,
        })
        .send()
        .await
        .map_err(|e| CliError::Network {
            message: format!("failed to update CSM variable '{config_id}': {e}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check your network connection and base URL",
            ))),
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let detail = extract_csm_error_detail(&body);
        return Err(CliError::Api {
            message: format!(
                "CSM UpdateVariableV2 returned HTTP {status} for variable '{config_id}' in namespace '{namespace}'{detail}"
            ),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check the namespace, app name, and your permissions",
            ))),
            category: crate::errors::ApiErrorCategory::Upstream,
        });
    }

    response.json().await.map_err(|e| CliError::Api {
        message: format!("failed to parse CSM UpdateVariableV2 response: {e}"),
        metadata: None,
        category: crate::errors::ApiErrorCategory::Upstream,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_list_variables_returns_data() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v2/admin/namespaces/test-ns/apps/my-app/variables"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"configId": "id-1", "configName": "MY_KEY", "applyMask": false, "description": "old desc"}
                ]
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = list_variables(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
        )
        .await;

        let records = result.expect("list_variables should succeed");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].config_id, "id-1");
        assert_eq!(records[0].config_name, "MY_KEY");
        assert!(!records[0].apply_mask);
        assert_eq!(records[0].description.as_deref(), Some("old desc"));
    }

    #[tokio::test]
    async fn test_list_variables_walks_pages_to_find_second_page_record() {
        use wiremock::matchers::query_param;

        let server = MockServer::start().await;
        // Page 1: full page (page_limit records), none matching the target key.
        Mock::given(method("GET"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .and(query_param("limit", "2"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"configId": "id-1", "configName": "OTHER_1", "applyMask": false, "description": null},
                    {"configId": "id-2", "configName": "OTHER_2", "applyMask": false, "description": null}
                ]
            })))
            .mount(&server)
            .await;
        // Page 2: short page containing the target key.
        Mock::given(method("GET"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .and(query_param("limit", "2"))
            .and(query_param("offset", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"configId": "id-3", "configName": "MY_KEY", "applyMask": true, "description": "found me"}
                ]
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = list_variables_paged(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
            2,
            10,
        )
        .await;

        let records = result.expect("list_variables_paged should succeed");
        assert_eq!(records.len(), 3);
        assert!(
            records
                .iter()
                .any(|r| r.config_name == "MY_KEY" && r.config_id == "id-3"),
            "expected the second page's record to be reached: {records:?}"
        );
    }

    #[tokio::test]
    async fn test_list_variables_non_200_returns_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = list_variables(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
        )
        .await;

        let err = result.expect_err("a 403 response must produce an error");
        assert!(matches!(err, CliError::Api { .. }));
        assert!(err.to_string().contains("403"));
    }

    #[tokio::test]
    async fn test_create_variable_sends_expected_body_and_returns_record() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .and(body_json(serde_json::json!({
                "configName": "MY_KEY",
                "value": "new-value",
                "applyMask": true,
                "description": "desc",
                "source": "plaintext"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "new-id",
                "configName": "MY_KEY",
                "applyMask": true,
                "description": "desc"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = create_variable(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
            "new-value",
            true,
            Some("desc"),
        )
        .await;

        let record = result.expect("create_variable should succeed");
        assert_eq!(record.config_id, "new-id");
    }

    #[tokio::test]
    async fn test_update_variable_sends_expected_body_and_returns_record() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables/id-1",
            ))
            .and(body_json(serde_json::json!({
                "value": "updated-value",
                "applyMask": false,
                "description": "old desc"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id-1",
                "configName": "MY_KEY",
                "applyMask": false,
                "description": "old desc"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = update_variable(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "id-1",
            "updated-value",
            false,
            Some("old desc"),
        )
        .await;

        let record = result.expect("update_variable should succeed");
        assert_eq!(record.config_id, "id-1");
    }

    #[tokio::test]
    async fn test_create_variable_non_200_returns_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .respond_with(ResponseTemplate::new(409))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = create_variable(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
            "new-value",
            false,
            None,
        )
        .await;

        let err = result.expect_err("a 409 response must produce an error");
        assert!(matches!(err, CliError::Api { .. }));
        assert!(err.to_string().contains("409"));
    }

    #[tokio::test]
    async fn test_update_variable_non_200_returns_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables/id-1",
            ))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = update_variable(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "id-1",
            "updated-value",
            false,
            None,
        )
        .await;

        let err = result.expect_err("a 500 response must produce an error");
        assert!(matches!(err, CliError::Api { .. }));
        assert!(err.to_string().contains("500"));
    }

    // ── CSM error detail extraction (errorCode / errorMessage from body) ──

    /// A non-2xx response whose body carries `errorCode` and `errorMessage`
    /// must include those fields in the error message, matching the behaviour
    /// of the sibling `update_secret/api.rs`. Without body parsing, the user
    /// cannot see WHY the request was rejected.
    #[tokio::test]
    async fn test_list_variables_non_200_includes_csm_error_detail() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "errorCode": 20003,
                "errorMessage": "insufficient permissions to list variables"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = list_variables(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
        )
        .await;

        let err = result.expect_err("a 403 response must produce an error");
        assert!(matches!(err, CliError::Api { .. }));
        let msg = err.to_string();
        assert!(msg.contains("403"), "must include HTTP status: {msg}");
        assert!(
            msg.contains("insufficient permissions to list variables"),
            "must include CSM errorMessage: {msg}"
        );
    }

    #[tokio::test]
    async fn test_create_variable_non_200_includes_csm_error_detail() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
                "errorCode": 20004,
                "errorMessage": "variable MY_KEY already exists"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = create_variable(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
            "new-value",
            false,
            None,
        )
        .await;

        let err = result.expect_err("a 409 response must produce an error");
        assert!(matches!(err, CliError::Api { .. }));
        let msg = err.to_string();
        assert!(msg.contains("409"), "must include HTTP status: {msg}");
        assert!(
            msg.contains("variable MY_KEY already exists"),
            "must include CSM errorMessage: {msg}"
        );
    }

    #[tokio::test]
    async fn test_update_variable_non_200_includes_csm_error_detail() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables/id-1",
            ))
            .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
                "errorCode": 20005,
                "errorMessage": "internal error updating variable id-1"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = update_variable(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "id-1",
            "updated-value",
            false,
            None,
        )
        .await;

        let err = result.expect_err("a 500 response must produce an error");
        assert!(matches!(err, CliError::Api { .. }));
        let msg = err.to_string();
        assert!(msg.contains("500"), "must include HTTP status: {msg}");
        assert!(
            msg.contains("internal error updating variable id-1"),
            "must include CSM errorMessage: {msg}"
        );
    }

    // ── Pagination: early-exit when key found, cap-exhaustion error ──

    /// When the target key is on the first page, page 2 must never be
    /// requested. A mock that returns 500 on page 2 proves the request is
    /// never made — if it were, the function would return a network/api error
    /// instead of Ok.
    #[tokio::test]
    async fn test_list_variables_stops_paging_when_key_on_first_page() {
        use wiremock::matchers::query_param;

        let server = MockServer::start().await;
        // Page 1: full page (2 records), one matching the target key.
        Mock::given(method("GET"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .and(query_param("limit", "2"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"configId": "id-1", "configName": "OTHER", "applyMask": false, "description": null},
                    {"configId": "id-2", "configName": "MY_KEY", "applyMask": false, "description": null}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;
        // Page 2: if reached, fails the test with a 500 error.
        Mock::given(method("GET"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables",
            ))
            .and(query_param("offset", "2"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = list_variables_paged(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
            2,
            10,
        )
        .await;

        let records = result.expect("should stop after finding key on page 1");
        assert!(
            records.iter().any(|r| r.config_name == "MY_KEY"),
            "result must contain the target key: {records:?}"
        );
        server.verify().await;
    }

    /// When every page is full and the key is never found, the function must
    /// return an error mentioning the record count rather than returning a
    /// truncated list that the caller would treat as "key absent."
    #[tokio::test]
    async fn test_list_variables_cap_exhaustion_returns_error() {
        use wiremock::matchers::{method, path, query_param};

        let server = MockServer::start().await;
        let page_limit: u64 = 2;
        let max_pages: u64 = 3;

        // All 3 pages are full (2 records each), none matching the target key.
        for page in 0..max_pages {
            let offset = page * page_limit;
            Mock::given(method("GET"))
                .and(path(
                    "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables",
                ))
                .and(query_param("limit", page_limit.to_string()))
                .and(query_param("offset", offset.to_string()))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "data": [
                        {"configId": format!("id-{}", offset), "configName": format!("OTHER_{}", offset), "applyMask": false, "description": null},
                        {"configId": format!("id-{}", offset + 1), "configName": format!("OTHER_{}", offset + 1), "applyMask": false, "description": null}
                    ]
                })))
                .mount(&server)
                .await;
        }

        let client = reqwest::Client::new();
        let result = list_variables_paged(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
            page_limit,
            max_pages,
        )
        .await;

        let err = result.expect_err("should return an error for cap exhaustion");
        let msg = err.to_string();
        let expected_count = max_pages * page_limit;
        assert!(
            msg.contains(&format!("within the first {expected_count} records")),
            "cap error must mention record count ({expected_count}): {msg}"
        );
    }

    // ── Path segment encoding ──

    /// A namespace containing a `/` must be percent-encoded in the request
    /// path, not split into additional path segments. The mock expects the
    /// encoded path; if the code interpolates the raw value the mock will
    /// not match and the function will return a network/connection error.
    ///
    /// Contract: CONTRIBUTING.md path-parameter rule — all user-supplied
    /// path segments go through `encode_url_path_segment`.
    #[tokio::test]
    async fn test_list_variables_encodes_namespace_in_url() {
        let server = MockServer::start().await;
        // Namespace with a slash: must appear as `ns%2Fevil` in the path.
        Mock::given(method("GET"))
            .and(path(
                "/csm/v2/admin/namespaces/ns%2Fevil/apps/my-app/variables",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"configId": "id-1", "configName": "MY_KEY", "applyMask": false}
                ]
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = list_variables(
            &client,
            &server.uri(),
            "test-token",
            "ns/evil",
            "my-app",
            "MY_KEY",
        )
        .await;

        let records = result.expect("namespace with '/' must be encoded, not split");
        assert_eq!(records.len(), 1);
    }

    /// An app name containing a `/` must be percent-encoded in the request
    /// path on the create endpoint.
    ///
    /// Contract: CONTRIBUTING.md path-parameter rule.
    #[tokio::test]
    async fn test_create_variable_encodes_app_in_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/app%2Fslash/variables",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "new-id",
                "configName": "MY_KEY",
                "applyMask": false
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = create_variable(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "app/slash",
            "MY_KEY",
            "val",
            false,
            None,
        )
        .await;

        result.expect("app with '/' must be encoded in the create URL");
    }

    /// A config_id containing a `/` must be percent-encoded on the update
    /// path rather than creating an extra path segment.
    ///
    /// Contract: CONTRIBUTING.md path-parameter rule.
    #[tokio::test]
    async fn test_update_variable_encodes_config_id_in_url() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v2/admin/namespaces/test-ns/apps/my-app/variables/id%2Fslash",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id/slash",
                "configName": "MY_KEY",
                "applyMask": false
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = update_variable(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "id/slash",
            "val",
            false,
            None,
        )
        .await;

        result.expect("config_id with '/' must be encoded in the update URL");
    }
}
