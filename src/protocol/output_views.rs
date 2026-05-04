//! View and payload types carried by top-level command outputs.
//!
//! These types are intentionally separate from `protocol::result`:
//! `result` models shaped API response bodies, while this module models the
//! auxiliary payloads attached to command-level outputs such as auth, config,
//! profile, tracing, and render intent.

use crate::protocol::catalogue::OperationSchema;

/// How a command's output should be presented based on the operation's
/// mutation class and path shape.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum CommandIntent {
    /// Non-read-only operation (`MutationClass::Mutating`).
    Action,
    /// Read-only single-resource lookup (path ends with `{id}`).
    Inspect,
    /// Read-only collection lookup (path does not end with `{id}`).
    List,
}

impl CommandIntent {
    /// Derive the intent from an operation's mutation class and path shape.
    pub fn from_operation(operation: &OperationSchema) -> Self {
        match operation.mutation_class {
            crate::protocol::catalogue::MutationClass::Mutating => CommandIntent::Action,
            crate::protocol::catalogue::MutationClass::ReadOnly
            | crate::protocol::catalogue::MutationClass::Diagnostic => {
                if operation.path_template.ends_with('}') {
                    CommandIntent::Inspect
                } else {
                    CommandIntent::List
                }
            }
        }
    }
}

/// A single label-value pair for display in inspect or success views.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FieldEntry {
    pub label: String,
    pub value: String,
}

/// A named group of fields rendered as a subsection in inspect views.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Section {
    pub heading: String,
    pub fields: Vec<FieldEntry>,
}

/// Whether `refresh-specs` was invoked for a single service or all services.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum RefreshMode {
    Single,
    All,
}

/// Outcome of a `refresh-specs` invocation, ready for rendering.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RefreshSpecsOutput {
    pub mode: RefreshMode,
    pub succeeded: Vec<String>,
    pub failed: Vec<(String, String)>,
    #[serde(skip)]
    pub duration: std::time::Duration,
}

/// The CLI version string for display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VersionOutput {
    pub version: String,
}

/// JSON request-body template emitted by `--skeleton`. The body itself is
/// schema-derived and intentionally opaque to the frontend.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkeletonOutput {
    pub body: serde_json::Value,
}

/// JSON introspection envelope emitted by `ags describe`. The envelope is
/// constructed upstream from typed catalogue data; the frontend renders it
/// as-is.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DescribeOutput {
    pub envelope: serde_json::Value,
}

/// Output from `ags completions <shell>`: a shell-completion script with
/// an optional stderr hint emitted when the shell was auto-detected.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompletionsOutput {
    pub script: String,
    pub hint: Option<String>,
}

/// Wrapper for auth command results carrying the view variant to render.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AuthOutput {
    pub view: AuthView,
}

/// The specific auth state to present to the user.
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub enum AuthView {
    /// Valid credentials found and tokens are usable
    Authenticated(AuthStatusData),
    /// Credentials exist but tokens are expired or incomplete
    RequiresAttention(AuthStatusData),
    /// A login flow just completed successfully
    LoginSuccess(AuthActionData),
    /// Stored credentials were cleared for one profile
    LogoutSuccess(LogoutData),
    /// Stored credentials were cleared for all profiles
    LogoutAllSuccess(LogoutAllData),
    /// No credentials found at all
    NotAuthenticated {
        next_step: Option<String>,
        tip: Option<String>,
    },
}

/// Snapshot of current auth state for the status display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AuthStatusData {
    pub source: AuthSource,
    pub base_url: Option<String>,
    pub login_type: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Presence,
    pub access_token: TokenState,
    pub refresh_token: TokenState,
    pub namespace: Option<String>,
    pub next_step: Option<String>,
}

/// Outcome of a successful auth action, used in [`AuthActionData::status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub enum AuthActionStatus {
    /// A login flow just completed and a fresh token was stored.
    LoggedIn,
    /// Login was a no-op because valid credentials already exist.
    AlreadyAuthenticated,
}

/// Data from a successful login action for the confirmation display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AuthActionData {
    pub status: AuthActionStatus,
    pub base_url: Option<String>,
    pub login_type: Option<String>,
    pub client_id: Option<String>,
    pub token_expires_in_secs: Option<u64>,
    pub tip: Option<String>,
}

/// Credential clearing results for the logout confirmation display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LogoutData {
    pub client_id: Presence,
    pub client_secret: Presence,
    pub access_token: Presence,
    pub refresh_token: Presence,
}

/// Results of clearing credentials from all profiles.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LogoutAllData {
    pub profiles_cleared: Vec<String>,
}

/// Where the active credentials originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub enum AuthSource {
    /// From `AGS_ACCESS_TOKEN` environment variable
    EnvironmentAccessToken,
    /// From `AGS_CLIENT_ID` / `AGS_CLIENT_SECRET` environment variables
    EnvironmentClientCredentials,
    /// From the OS keychain
    Stored,
}

/// Whether a credential item exists in storage, without exposing its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub enum Presence {
    /// Credential is present in storage
    Stored,
    /// Credential was just removed
    Cleared,
    /// Credential was not found
    Missing,
    /// Storage could not be queried
    Unknown,
}

/// Lifecycle state of an OAuth token for display purposes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub enum TokenState {
    /// Token exists and has not expired
    Valid { expires_in_secs: Option<u64> },
    /// Token exists but has expired
    Expired,
    /// No token found in storage
    Missing,
    /// Token exists but expiry is not known
    Present,
    /// Storage could not be queried
    Unknown,
}

/// Complete result of a service API call, ready for rendering.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ApiOutput {
    pub operation: OperationSchema,
    pub resource_name: String,
    pub body: ApiBody,
    pub success: Option<ApiSuccess>,
    pub trace: Option<ExecutionTrace>,
}

/// The response body from an API call in its shaped or fallback form.
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub enum ApiBody {
    /// A fully shaped, structured result ready for rendering.
    Shaped(Box<crate::protocol::result::CommandResult>),
    /// Plain text or non-JSON response
    Text(String),
    /// No response body (e.g. 204 No Content)
    Empty,
}

/// A one-line success message rendered to stderr after a mutating operation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ApiSuccess {
    pub summary: String,
}

/// Verbose request/response details shown on stderr when `--verbose` is set.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExecutionTrace {
    pub resolution: Option<ResolutionTrace>,
    pub request: RequestTrace,
    pub response: Option<ResponseTrace>,
}

/// How each config value was resolved for verbose output.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolutionTrace {
    pub spec_source: String,
    pub profile: (String, String),
    pub base_url: (String, String),
    pub namespace: Option<(String, String)>,
    pub token_source: String,
    pub token_expiry: Option<String>,
}

/// The outbound HTTP request details for verbose trace output.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RequestTrace {
    pub http_method: String,
    pub url: String,
    pub query_params: Vec<(String, String)>,
    pub has_auth_header: bool,
    pub body_size: Option<usize>,
}

/// The HTTP response status for verbose trace output.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResponseTrace {
    pub status: u16,
    pub reason: Option<String>,
    pub body_size: Option<usize>,
}

/// Wrapper for config command results.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigOutput {
    pub view: ConfigView,
}

/// The specific config operation result to present.
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub enum ConfigView {
    /// Result of `ags config get` (all keys)
    GetAll {
        profile: String,
        entries: Vec<crate::protocol::config::ResolvedEntry>,
    },
    /// Result of `ags config get <key>`
    GetOne {
        key: String,
        value: Option<String>,
        source: crate::protocol::config::ConfigSource,
    },
    /// Result of `ags config set`
    Set { key: String, value: String },
    /// Result of `ags config unset`
    Unset { key: String },
}

/// Wrapper for profile command results.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProfileOutput {
    pub view: ProfileView,
}

/// The specific profile state to present.
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub enum ProfileView {
    /// Result of `ags profile list`
    List {
        profiles: Vec<ProfileSummary>,
        active: Option<String>,
    },
    /// Result of `ags profile create`
    Created { name: String },
    /// Result of `ags profile use`
    Switched { name: String },
    /// Result of `ags profile show` when no active profile is set
    NoActiveProfile,
    /// Result of `ags profile show`
    Show {
        name: String,
        is_active: bool,
        config: ProfileShowData,
    },
    /// Result of `ags profile delete`
    Deleted {
        name: String,
        warnings: Vec<OperationWarning>,
        tips: Vec<String>,
    },
    /// Result of `ags profile rename`
    Renamed {
        old: String,
        new: String,
        warnings: Vec<OperationWarning>,
    },
}

/// Summary of a single profile for the list view.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProfileSummary {
    pub name: String,
    pub is_active: bool,
}

/// Profile configuration fields for the show view.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProfileShowData {
    pub base_url: Option<String>,
    pub client_id: Option<String>,
    pub namespace: Option<String>,
    pub grant_type: Option<String>,
    pub has_secret: bool,
    pub has_token: bool,
}

/// Details of a raw body that was written to stdout or a file via `--output`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BinaryWrittenOutput {
    pub destination: BinaryWrittenDestination,
    pub bytes_written: usize,
    pub content_type: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub enum BinaryWrittenDestination {
    Stdout,
    File(std::path::PathBuf),
}

/// A non-fatal issue encountered during a multi-step operation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OperationWarning {
    /// What went wrong
    pub message: String,
    /// Platform-level error detail (if applicable)
    pub reason: Option<String>,
    /// How to recover
    pub fix: String,
}

#[cfg(test)]
mod serialize_smoke_tests {
    use super::*;

    /// Top-level view types must derive `Serialize` so the JSON frontend can
    /// emit them via `serde_json::to_value`. This smoke test exercises the
    /// derive path; it does NOT lock down JSON shape — that's a future-task
    /// concern when the JSON frontend switches over.
    #[test]
    fn test_auth_view_authenticated_serialises() {
        let view = AuthView::Authenticated(AuthStatusData {
            source: AuthSource::Stored,
            base_url: Some("https://demo.accelbyte.io".to_string()),
            login_type: Some("authorization-code".to_string()),
            client_id: Some("abc123".to_string()),
            client_secret: Presence::Stored,
            access_token: TokenState::Valid {
                expires_in_secs: Some(3600),
            },
            refresh_token: TokenState::Valid {
                expires_in_secs: Some(604800),
            },
            namespace: Some("accelbyte".to_string()),
            next_step: None,
        });
        let _ = serde_json::to_value(&view).expect("AuthView must serialise");
    }
}
