//! Resolving which AMS upload host a build should be shipped to.
//!
//! The AGS platform answers `GET /ams/v1/upload-url` — catalogued as
//! `ams/public/info/v1/get-upload-url` — with the environment's AMS upload
//! host. armada-cli discarded any failure here and fell through to production,
//! so a typo in the platform host silently shipped a build to prod. Discovery
//! failure is fatal instead.

use reqwest::Client;

use super::errors::AmsUploadError;

/// Path on the AGS platform that reports the AMS upload host.
const UPLOAD_URL_PATH: &str = "/ams/v1/upload-url";

/// Resolve the AMS upload base URL for the platform at `base_url`.
///
/// An explicit `override_url` short-circuits discovery entirely, which is the
/// escape hatch for environments whose platform host cannot answer.
pub(crate) async fn resolve_upload_base_url(
    client: &Client,
    base_url: &str,
    access_token: &str,
    override_url: Option<&str>,
) -> Result<String, AmsUploadError> {
    if let Some(override_url) = override_url {
        return normalise_upload_url(override_url);
    }

    let endpoint = format!("{}{UPLOAD_URL_PATH}", base_url.trim_end_matches('/'));
    let response = client
        .get(&endpoint)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| AmsUploadError::UploadHostUnresolved {
            reason: format!("{endpoint} could not be reached: {error}"),
        })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(AmsUploadError::UploadHostUnresolved {
            reason: format!("{endpoint} answered HTTP {}", status.as_u16()),
        });
    }

    let candidate = extract_url(&body).ok_or_else(|| AmsUploadError::UploadHostUnresolved {
        reason: format!("{endpoint} did not return a URL"),
    })?;
    normalise_upload_url(&candidate)
}

/// Read the upload host out of a `get-upload-url` response body.
///
/// The endpoint answers with a bare URL string rather than a JSON object, so
/// the body is accepted in three shapes: raw text, a quoted JSON string, and a
/// `{"url": …}` object — the last two guard against the endpoint being
/// normalised later.
fn extract_url(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(url) = value.as_str() {
            return Some(url.to_string());
        }
        if let Some(url) = value.get("url").and_then(serde_json::Value::as_str) {
            return Some(url.to_string());
        }
        return None;
    }
    Some(trimmed.to_string())
}

/// Validate an upload host and strip any trailing slash.
fn normalise_upload_url(url: &str) -> Result<String, AmsUploadError> {
    let trimmed = url.trim().trim_end_matches('/');
    let parsed = url::Url::parse(trimmed)
        .map_err(|_| AmsUploadError::UploadHostInvalid(url.trim().to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") || !parsed.has_host() {
        return Err(AmsUploadError::UploadHostInvalid(url.trim().to_string()));
    }
    Ok(trimmed.to_string())
}

/// The host of an AGS platform base URL, used as `ams-source-environment`.
///
/// Falls back to the raw input when it does not parse, so a malformed
/// configured base URL surfaces as an auth failure rather than here.
pub(crate) fn source_environment(base_url: &str) -> String {
    url::Url::parse(base_url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_else(|| base_url.trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_url_accepts_a_bare_string_body() {
        assert_eq!(
            extract_url("https://prod.ams.accelbyte.io\n").as_deref(),
            Some("https://prod.ams.accelbyte.io")
        );
    }

    #[test]
    fn test_extract_url_accepts_json_shapes() {
        assert_eq!(
            extract_url("\"https://dev.ams.accelbyte.io\"").as_deref(),
            Some("https://dev.ams.accelbyte.io")
        );
        assert_eq!(
            extract_url(r#"{"url":"https://dev.ams.accelbyte.io"}"#).as_deref(),
            Some("https://dev.ams.accelbyte.io")
        );
        assert_eq!(extract_url(r#"{"other":1}"#), None);
        assert_eq!(extract_url("   "), None);
    }

    #[test]
    fn test_normalise_rejects_non_http_urls() {
        assert!(normalise_upload_url("ftp://ams.example.com").is_err());
        assert!(normalise_upload_url("prod.ams.accelbyte.io").is_err());
        assert_eq!(
            normalise_upload_url("https://prod.ams.accelbyte.io/").unwrap(),
            "https://prod.ams.accelbyte.io"
        );
    }

    #[test]
    fn test_source_environment_is_the_platform_host() {
        assert_eq!(
            source_environment("https://demo.accelbyte.io"),
            "demo.accelbyte.io"
        );
        assert_eq!(
            source_environment("https://demo.accelbyte.io/"),
            "demo.accelbyte.io"
        );
    }
}
