//! The AMS image-upload wire contract, reconstructed from `armada-core-api`
//! v0.56.2. Every field is lowerCamelCase and every path hangs off
//! `{uploadBaseUrl}/upload/v1`.

use ags_protocol::output_views::{ExecutionTrace, RequestTrace, ResponseTrace};
use reqwest::{Client, Method};
use serde_json::{json, Value};

use super::errors::AmsUploadError;

/// Image format AMS is told to expect. Only `tgz` is produced by this CLI.
const IMAGE_FORMAT: &str = "tgz";

/// Operation labels for each AMS upload API call, shared with `errors.rs` so a
/// 403's stage-specific guidance can match on them without duplicating the
/// literal strings.
pub(crate) mod operation {
    pub(crate) const CREATE_IMAGE: &str = "Creating the image";
    pub(crate) const PRESIGN_URL: &str = "Requesting an upload URL";
    pub(crate) const INITIATE_MULTIPART: &str = "Starting the multipart upload";
    pub(crate) const PRESIGN_PART: &str = "Requesting an upload URL for a part";
    pub(crate) const FINALIZE_MULTIPART: &str = "Finalizing the multipart upload";
    pub(crate) const COMPLETE: &str = "Completing the upload";
}

const HEADER_SOURCE_ENVIRONMENT: &str = "ams-source-environment";
const HEADER_CLI_VERSION: &str = "ams-cli-version";

/// Maximum bytes of an error response body echoed back to the user. Kept short
/// because the actionable guidance is rendered below the message, and a proxy's
/// HTML error page would otherwise push it off screen.
const MAX_ERROR_BODY_LEN: usize = 200;

/// Authenticated client for the AMS upload endpoints.
pub(crate) struct UploadApi {
    client: Client,
    /// `{uploadBaseUrl}/upload/v1`, with no trailing slash.
    base: String,
    access_token: String,
    /// Host of the AGS platform this upload originates from, sent as
    /// `ams-source-environment` so AMS can attribute the image.
    source_environment: String,
    /// Whether to attach request/response detail to failures, so `--verbose`
    /// is not silent when an upload is refused.
    is_verbose: bool,
}

impl UploadApi {
    /// Build a client for `upload_base_url`, tagging requests as coming from
    /// `source_environment` (the AGS platform host).
    pub(crate) fn new(
        client: Client,
        upload_base_url: &str,
        access_token: impl Into<String>,
        source_environment: impl Into<String>,
        is_verbose: bool,
    ) -> Self {
        Self {
            client,
            base: format!("{}/upload/v1", upload_base_url.trim_end_matches('/')),
            access_token: access_token.into(),
            source_environment: source_environment.into(),
            is_verbose,
        }
    }

    /// Register a new image and return its id. AMS answers `201 Created`.
    pub(crate) async fn create_image(
        &self,
        name: &str,
        target_architecture: &str,
    ) -> Result<String, AmsUploadError> {
        let body = json!({
            "name": name,
            // armada never populates tags; AMS requires the key to be present.
            "tags": [],
            "format": IMAGE_FORMAT,
            "targetArchitecture": target_architecture,
        });
        let value = self
            .send(operation::CREATE_IMAGE, Method::POST, "/images", body, 201)
            .await?;
        string_field(&value, "id", operation::CREATE_IMAGE)
    }

    /// Get a pre-signed URL for a single-shot upload of `file_path`.
    pub(crate) async fn presign_url(
        &self,
        image_id: &str,
        file_path: &str,
    ) -> Result<String, AmsUploadError> {
        let body = json!({ "imageId": image_id, "filePath": file_path });
        let value = self
            .send(
                operation::PRESIGN_URL,
                Method::POST,
                "/pre-sign-url",
                body,
                200,
            )
            .await?;
        string_field(&value, "url", operation::PRESIGN_URL)
    }

    /// Start a multipart upload and return its upload id.
    pub(crate) async fn initiate_multipart(
        &self,
        image_id: &str,
        file_path: &str,
    ) -> Result<String, AmsUploadError> {
        let body = json!({ "imageId": image_id, "filePath": file_path });
        let value = self
            .send(
                operation::INITIATE_MULTIPART,
                Method::POST,
                "/multi-part",
                body,
                200,
            )
            .await?;
        string_field(&value, "uploadId", operation::INITIATE_MULTIPART)
    }

    /// Get a pre-signed URL for one part. `part_number` is 1-based.
    pub(crate) async fn presign_part(
        &self,
        upload_id: &str,
        image_id: &str,
        file_path: &str,
        part_number: usize,
    ) -> Result<String, AmsUploadError> {
        let body = json!({
            "imageId": image_id,
            "filePath": file_path,
            "partNo": part_number,
        });
        let value = self
            .send(
                operation::PRESIGN_PART,
                Method::PUT,
                &format!("/multi-part/{upload_id}"),
                body,
                200,
            )
            .await?;
        string_field(&value, "url", operation::PRESIGN_PART)
    }

    /// Close a multipart upload. `etags` must be ordered by part number.
    pub(crate) async fn finalize_multipart(
        &self,
        upload_id: &str,
        image_id: &str,
        file_path: &str,
        etags: &[String],
    ) -> Result<(), AmsUploadError> {
        let body = json!({
            "imageId": image_id,
            "filePath": file_path,
            "parts": etags,
        });
        self.send(
            operation::FINALIZE_MULTIPART,
            Method::PUT,
            &format!("/multi-part/{upload_id}/finalize"),
            body,
            200,
        )
        .await?;
        Ok(())
    }

    /// Mark the image as fully uploaded and record its entrypoint command.
    pub(crate) async fn complete(
        &self,
        image_id: &str,
        image_size_bytes: u64,
        command: &str,
    ) -> Result<(), AmsUploadError> {
        let body = json!({
            "imageId": image_id,
            "imageSizeBytes": image_size_bytes,
            "command": command,
        });
        self.send(operation::COMPLETE, Method::PUT, "/complete", body, 200)
            .await?;
        Ok(())
    }

    /// Issue one AMS upload API call and decode its JSON body.
    async fn send(
        &self,
        operation: &'static str,
        method: Method,
        path: &str,
        body: Value,
        expected_status: u16,
    ) -> Result<Value, AmsUploadError> {
        let url = format!("{}{path}", self.base);
        let request_size = serde_json::to_string(&body).map(|s| s.len()).ok();
        let response = self
            .client
            .request(method.clone(), &url)
            .bearer_auth(&self.access_token)
            .header(HEADER_SOURCE_ENVIRONMENT, &self.source_environment)
            .header(HEADER_CLI_VERSION, super::CLI_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|error| AmsUploadError::ApiCallFailed {
                operation,
                reason: error.to_string(),
                status: None,
                trace: self.trace_for(&method, &url, request_size, None, None),
            })?;

        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        if status != expected_status {
            return Err(AmsUploadError::ApiCallFailed {
                operation,
                reason: describe_error_body(status, &text),
                status: Some(status),
                trace: self.trace_for(&method, &url, request_size, Some(status), Some(text.len())),
            });
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// Build the verbose request/response trace for one call, or `None` when
    /// `--verbose` was not requested.
    ///
    /// Only AMS API calls are traced. Pre-signed storage URLs are deliberately
    /// never put in a trace: the signature travels in the query string, so
    /// printing one would leak a usable upload credential into logs.
    fn trace_for(
        &self,
        method: &Method,
        url: &str,
        request_size: Option<usize>,
        status: Option<u16>,
        response_size: Option<usize>,
    ) -> Option<Box<ExecutionTrace>> {
        if !self.is_verbose {
            return None;
        }
        Some(Box::new(ExecutionTrace {
            resolution: None,
            request: RequestTrace {
                http_method: method.as_str().to_string(),
                url: url.to_string(),
                query_params: Vec::new(),
                has_auth_header: true,
                body_size: request_size,
            },
            response: status.map(|status| ResponseTrace {
                status,
                reason: None,
                body_size: response_size,
            }),
        }))
    }
}

/// Pull a required string field out of a decoded response body.
fn string_field(
    value: &Value,
    field: &str,
    operation: &'static str,
) -> Result<String, AmsUploadError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|found| !found.is_empty())
        .map(str::to_string)
        .ok_or(AmsUploadError::ApiCallFailed {
            operation,
            reason: format!("the response did not contain a '{field}' value"),
            status: None,
            trace: None,
        })
}

/// Turn an AMS error response into a single readable sentence.
///
/// AccelByte services answer with `{"errorCode":…,"errorMessage":…}`. Anything
/// else — most often a proxy's multi-line HTML error page, which sits in front
/// of AMS and answers before the service does — is flattened to one line and
/// truncated, so it cannot swamp the guidance printed beneath it.
fn describe_error_body(status: u16, body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        let message = value
            .get("errorMessage")
            .or_else(|| value.get("message"))
            .and_then(Value::as_str);
        if let Some(message) = message {
            return match value.get("errorCode").and_then(Value::as_i64) {
                Some(code) => format!("HTTP {status} ({code}) {message}"),
                None => format!("HTTP {status} {message}"),
            };
        }
    }
    let flattened = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = crate::support::strings::truncate_display_text(&flattened, MAX_ERROR_BODY_LEN);
    if trimmed.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status} {trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_describe_error_body_prefers_accelbyte_shape() {
        let body = r#"{"errorCode":1234,"errorMessage":"image name already in use"}"#;
        assert_eq!(
            describe_error_body(409, body),
            "HTTP 409 (1234) image name already in use"
        );
    }

    #[test]
    fn test_describe_error_body_falls_back_to_raw_text() {
        assert_eq!(
            describe_error_body(502, "  <html>Bad Gateway</html>  "),
            "HTTP 502 <html>Bad Gateway</html>"
        );
        assert_eq!(describe_error_body(500, ""), "HTTP 500");
    }

    /// A proxy's HTML error page must collapse to one line — left as-is it
    /// pushes the actionable guidance rendered beneath it off the screen.
    #[test]
    fn test_describe_error_body_flattens_multiline_html() {
        let nginx = "<html>\n<head><title>403 Forbidden</title></head>\n<body>\n\
                     <center><h1>403 Forbidden</h1></center>\n</body>\n</html>\n";
        let described = describe_error_body(403, nginx);
        assert!(!described.contains('\n'), "must be one line: {described}");
        assert!(described.starts_with("HTTP 403 "));
        assert!(described.len() <= MAX_ERROR_BODY_LEN + 32, "{described}");
    }

    #[test]
    fn test_string_field_rejects_missing_or_empty_values() {
        let value = serde_json::json!({ "id": "", "other": 3 });
        assert!(string_field(&value, "id", "Creating the image").is_err());
        assert!(string_field(&value, "missing", "Creating the image").is_err());
        assert_eq!(
            string_field(
                &serde_json::json!({ "id": "img-1" }),
                "id",
                "Creating the image"
            )
            .unwrap(),
            "img-1"
        );
    }

    #[test]
    fn test_base_path_is_normalised() {
        let api = UploadApi::new(
            Client::new(),
            "https://prod.ams.accelbyte.io/",
            "token",
            "demo.accelbyte.io",
            false,
        );
        assert_eq!(api.base, "https://prod.ams.accelbyte.io/upload/v1");
    }
}
