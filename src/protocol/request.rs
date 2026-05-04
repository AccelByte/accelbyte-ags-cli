//! Request types — how a command is invoked.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// OAuth2 grant type used to obtain access tokens.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum, zeroize::Zeroize,
)]
#[serde(rename_all = "kebab-case")]
#[clap(rename_all = "kebab-case")]
#[non_exhaustive]
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

/// User-facing output verbosity. Exactly one state is active at a time —
/// replaces the previous `is_quiet`/`is_verbose` boolean pair, which made
/// the illegal "quiet AND verbose" combination representable.
///
/// When both `--quiet` and `--verbose` are passed, the last flag on the
/// command line wins (last-writer-wins), which is more predictable than
/// the previous behaviour where the renderer checked `is_quiet` first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
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
#[non_exhaustive]
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

/// A single invocation of a command, keyed on `operation_id` rather than CLI display name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRequest {
    pub service: crate::catalogue::ServiceId,
    pub operation_id: crate::protocol::catalogue::OperationId,
    pub namespace: Option<String>,
    pub path_params: BTreeMap<String, String>,
    pub query_params: BTreeMap<String, String>,
    pub header_params: BTreeMap<String, String>,
    pub body: Option<serde_json::Value>,
    pub output_format: OutputFormat,
    pub pagination: PaginationHint,
    pub verbosity: Verbosity,
    /// Where the response body should go. `None` means "use the formatter";
    /// `Some(Stdout)` writes to stdout; `Some(File)` writes to disk.
    pub output: Option<OutputDestination>,
}

/// How the caller wants results rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
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
            service: crate::catalogue::Catalogue::find_id("iam").expect("iam in manifest"),
            operation_id: crate::protocol::catalogue::OperationId::new("AdminGetUserByUserIDV3"),
            namespace: None,
            path_params: BTreeMap::new(),
            query_params: BTreeMap::new(),
            header_params: BTreeMap::new(),
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
            service: crate::catalogue::Catalogue::find_id("iam").expect("iam in manifest"),
            operation_id: crate::protocol::catalogue::OperationId::new("AdminUpdateUserV3"),
            namespace: Some("accelbyte".to_string()),
            path_params: path,
            query_params: query,
            header_params: headers,
            body: Some(serde_json::json!({"displayName": "Alice"})),
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
}
