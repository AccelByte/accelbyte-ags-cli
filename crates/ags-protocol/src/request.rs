//! Request types — how a command is invoked.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// OAuth2 grant type used to obtain access tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GrantType {
    /// Browser-driven OAuth2 authorization code + PKCE flow.
    AuthorizationCode,
    /// Headless client-credentials grant.
    ClientCredentials,
}

impl GrantType {
    /// OAuth2 wire-protocol parameter value used in HTTP form bodies.
    /// (`authorization_code`, `client_credentials` — note the underscores.)
    #[allow(dead_code)]
    pub fn as_oauth_param(self) -> &'static str {
        match self {
            GrantType::AuthorizationCode => "authorization_code",
            GrantType::ClientCredentials => "client_credentials",
        }
    }

    /// User-facing kebab-case label used in config files, render output, and
    /// the `--grant` flag. Matches the serde encoding.
    pub fn as_kebab(self) -> &'static str {
        match self {
            GrantType::AuthorizationCode => "authorization-code",
            GrantType::ClientCredentials => "client-credentials",
        }
    }
}

impl std::fmt::Display for GrantType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_kebab())
    }
}

impl std::str::FromStr for GrantType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "authorization-code" => Ok(GrantType::AuthorizationCode),
            "client-credentials" => Ok(GrantType::ClientCredentials),
            other => Err(format!(
                "unknown grant type '{other}' (expected 'authorization-code' or 'client-credentials')"
            )),
        }
    }
}

/// User-facing output verbosity. Exactly one state is active at a time —
/// replaces the previous `is_quiet`/`is_verbose` boolean pair, which made
/// the illegal "quiet AND verbose" combination representable.
///
/// When both `--quiet` and `--verbose` are passed, the last flag on the
/// command line wins (last-writer-wins), which is more predictable than
/// the previous behaviour where the renderer checked `is_quiet` first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verbosity {
    /// Suppress non-essential output (progress, status lines).
    Quiet,
    /// Default verbosity — render the result, omit trace.
    #[default]
    Normal,
    /// Render the result plus resolution trace and request/response details.
    Verbose,
}

impl Verbosity {
    /// Whether progress output and trailing detail lines should be suppressed.
    pub fn is_quiet(self) -> bool {
        matches!(self, Verbosity::Quiet)
    }

    /// Whether trace output should be rendered.
    pub fn is_verbose(self) -> bool {
        matches!(self, Verbosity::Verbose)
    }
}

/// Where the rendered or raw response body should go. `None` (the absent
/// case) means "use the configured frontend"; `Some(Stdout)` means raw bytes
/// to stdout (was `--output -`); `Some(File(path))` means write to disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "path")]
pub enum OutputDestination {
    /// Write raw bytes to stdout (was `--output -`).
    Stdout,
    /// Write to a file path.
    File(std::path::PathBuf),
}

impl std::str::FromStr for OutputDestination {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(if s == "-" {
            OutputDestination::Stdout
        } else {
            OutputDestination::File(std::path::PathBuf::from(s))
        })
    }
}

/// The outbound request body: either a JSON value or a multipart/form-data
/// payload. An operation's OpenAPI schema determines which — `in: body` and
/// `in: formData` are mutually exclusive on a single OAS2 operation, so a
/// request is never both.
///
/// `Serialize`/`Deserialize` are hand-written rather than derived with a
/// `#[serde(tag = "kind", content = "value")]` enum tag: that tag would wrap
/// `Json`'s payload in `{"kind": "json", "value": {...}}`, changing the
/// on-the-wire shape of `--dry-run --format json` / `workflow run --format
/// json` output for every command with a JSON body, not just the new
/// multipart ones — a shape change `cli-reference.md` §10.1 requires a major
/// bump for. Instead, `Json(v)` serialises as the bare `v` (matching
/// pre-multipart behaviour exactly) and only `Multipart` gets the
/// `{"kind": "multipart", "value": [...]}` envelope, since that shape is new
/// either way.
#[derive(Debug, Clone, PartialEq)]
pub enum RequestBody {
    /// A JSON request body.
    Json(serde_json::Value),
    /// A multipart/form-data request body, one part per formData parameter.
    Multipart(Vec<FormPart>),
}

impl Serialize for RequestBody {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            RequestBody::Json(value) => value.serialize(serializer),
            RequestBody::Multipart(parts) => {
                #[derive(Serialize)]
                struct Envelope<'a> {
                    kind: &'static str,
                    value: &'a Vec<FormPart>,
                }
                Envelope {
                    kind: "multipart",
                    value: parts,
                }
                .serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for RequestBody {
    /// A bare JSON value deserialises as `Json`; an object shaped like
    /// `{"kind": "multipart", "value": [...]}` deserialises as `Multipart`.
    /// This is a heuristic, not a fully unambiguous tag: a genuine JSON body
    /// that happens to be an object with a top-level `"kind": "multipart"`
    /// field would be misread as `Multipart`. Accepted deliberately to keep
    /// `Json`'s wire shape unchanged — no real AccelByte request body uses a
    /// `kind` field for this purpose, and every current caller only
    /// deserialises `RequestBody` back in round-trip tests.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if let serde_json::Value::Object(map) = &value {
            if map.get("kind").and_then(serde_json::Value::as_str) == Some("multipart") {
                let parts = map.get("value").cloned().unwrap_or(serde_json::Value::Null);
                let parts: Vec<FormPart> =
                    serde_json::from_value(parts).map_err(serde::de::Error::custom)?;
                return Ok(RequestBody::Multipart(parts));
            }
        }
        Ok(RequestBody::Json(value))
    }
}

/// One part of a multipart/form-data request body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FormPart {
    /// A plain text form field.
    Text {
        /// The formData parameter name.
        name: String,
        /// The field's resolved string value.
        value: String,
    },
    /// A file form field, read from local disk at send time.
    File {
        /// The formData parameter name.
        name: String,
        /// Local filesystem path to read the file from.
        path: std::path::PathBuf,
        /// Filename reported to the server (the path's basename).
        filename: String,
    },
}

/// A single invocation of a command, keyed on `operation_id` rather than CLI display name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRequest {
    pub service: crate::catalogue::ServiceId,
    pub operation_id: crate::catalogue::OperationId,
    pub namespace: Option<String>,
    pub path_params: BTreeMap<String, String>,
    pub query_params: BTreeMap<String, String>,
    pub header_params: BTreeMap<String, String>,
    /// Raw string values of `formData` CLI flags, keyed by parameter name.
    /// Populated by `build_command_request` for a single service command;
    /// read by `cli_flags_matching_workflow_inputs` to seed the executor's
    /// `workflow_supplied` map. Not used by the final dispatched request —
    /// the real multipart body is assembled later from resolved workflow
    /// inputs, not from this map directly.
    #[serde(default)]
    pub form_params: BTreeMap<String, String>,
    pub body: Option<RequestBody>,
    pub output_format: OutputFormat,
    pub pagination: PaginationHint,
    pub verbosity: Verbosity,
    /// Where the response body should go. `None` means "use the formatter";
    /// `Some(Stdout)` writes to stdout; `Some(File)` writes to disk.
    pub output: Option<OutputDestination>,
}

/// How the caller wants results rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "human" => Ok(OutputFormat::Human),
            "json" => Ok(OutputFormat::Json),
            _ => Err(format!(
                "unknown --format value '{s}' (expected: human, json)"
            )),
        }
    }
}

/// How the runtime should handle multi-page responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum PaginationHint {
    /// Respect server defaults (single page of whatever size the API returns).
    Auto,
    /// Only fetch the first page.
    FirstPageOnly,
    /// Paginate to exhaustion.
    All,
    /// Stop after N items.
    Limit(u64),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialise `value` to JSON, parse it back, and assert equality — the contract test for protocol types.
    fn round_trip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let parsed: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, &parsed);
    }

    #[test]
    fn test_command_request_minimal_round_trip() {
        round_trip(&CommandRequest {
            service: crate::catalogue::ServiceId::new("iam"),
            operation_id: crate::catalogue::OperationId::new("AdminGetUserByUserIDV3"),
            namespace: None,
            path_params: BTreeMap::new(),
            query_params: BTreeMap::new(),
            header_params: BTreeMap::new(),
            form_params: BTreeMap::new(),
            body: None,
            output_format: OutputFormat::Human,
            pagination: PaginationHint::Auto,
            verbosity: Verbosity::Normal,
            output: None,
        });
    }

    #[test]
    fn test_command_request_full_round_trip() {
        let mut path = BTreeMap::new();
        path.insert("namespace".to_string(), "accelbyte".to_string());
        path.insert("userId".to_string(), "abc123".to_string());
        let mut query = BTreeMap::new();
        query.insert("limit".to_string(), "100".to_string());
        let mut headers = BTreeMap::new();
        headers.insert("X-Trace".to_string(), "1".to_string());

        round_trip(&CommandRequest {
            service: crate::catalogue::ServiceId::new("iam"),
            operation_id: crate::catalogue::OperationId::new("AdminUpdateUserV3"),
            namespace: Some("accelbyte".to_string()),
            path_params: path,
            query_params: query,
            header_params: headers,
            form_params: BTreeMap::new(),
            body: Some(RequestBody::Json(
                serde_json::json!({"displayName": "Alice"}),
            )),
            output_format: OutputFormat::Json,
            pagination: PaginationHint::Limit(500),
            verbosity: Verbosity::Verbose,
            output: Some(OutputDestination::File(std::path::PathBuf::from(
                "/tmp/out.json",
            ))),
        });
    }

    #[test]
    fn test_verbosity_all_variants_round_trip() {
        for value in [Verbosity::Quiet, Verbosity::Normal, Verbosity::Verbose] {
            round_trip(&value);
        }
    }

    #[test]
    fn test_output_format_all_variants_round_trip() {
        for value in [OutputFormat::Human, OutputFormat::Json] {
            round_trip(&value);
        }
    }

    #[test]
    fn test_pagination_hint_all_variants_round_trip() {
        round_trip(&PaginationHint::Auto);
        round_trip(&PaginationHint::FirstPageOnly);
        round_trip(&PaginationHint::All);
        round_trip(&PaginationHint::Limit(0));
        round_trip(&PaginationHint::Limit(1));
        round_trip(&PaginationHint::Limit(u64::MAX));
    }

    #[test]
    fn test_output_format_rejects_removed_tui_alias() {
        use std::str::FromStr;
        let err = OutputFormat::from_str("tui").unwrap_err();
        assert!(
            err.contains("unknown --format value 'tui'"),
            "tui must be rejected after the legacy alias removal: {err}"
        );
    }

    /// `RequestBody::Json` round-trips through serialize/deserialize.
    #[test]
    fn test_request_body_json_round_trip() {
        round_trip(&RequestBody::Json(serde_json::json!({"a": 1})));
    }

    /// `RequestBody::Json` serialises as the bare value — no `{"kind", "value"}`
    /// envelope — so `--dry-run --format json` output for JSON-bodied commands
    /// keeps the same shape it had before multipart support existed.
    #[test]
    fn test_request_body_json_serializes_as_bare_value() {
        let body = RequestBody::Json(serde_json::json!({"statCode": "mmr"}));
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json, serde_json::json!({"statCode": "mmr"}));
    }

    /// `RequestBody::Multipart` serialises with a `{"kind": "multipart", "value": [...]}`
    /// envelope — the new shape is fine to introduce since no prior release ever
    /// emitted a multipart body.
    #[test]
    fn test_request_body_multipart_serializes_with_kind_envelope() {
        let body = RequestBody::Multipart(vec![FormPart::Text {
            name: "strategy".to_string(),
            value: "REPLACE".to_string(),
        }]);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "multipart",
                "value": [{"kind": "text", "name": "strategy", "value": "REPLACE"}]
            })
        );
    }

    /// `RequestBody::Multipart` with both a text and a file part round-trips.
    #[test]
    fn test_request_body_multipart_round_trip() {
        round_trip(&RequestBody::Multipart(vec![
            FormPart::Text {
                name: "strategy".to_string(),
                value: "REPLACE".to_string(),
            },
            FormPart::File {
                name: "file".to_string(),
                path: std::path::PathBuf::from("/tmp/asset.png"),
                filename: "asset.png".to_string(),
            },
        ]));
    }
}
