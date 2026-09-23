//! View and payload types carried by top-level command outputs.
//!
//! These types are intentionally separate from `protocol::result`:
//! `result` models shaped API response bodies, while this module models the
//! auxiliary payloads attached to command-level outputs such as auth, config,
//! profile, tracing, and render intent.

use crate::catalogue::OperationSchema;

/// How a command's output should be presented based on the operation's
/// mutation class and path shape.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
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
            crate::catalogue::MutationClass::Mutating => CommandIntent::Action,
            crate::catalogue::MutationClass::ReadOnly
            | crate::catalogue::MutationClass::Diagnostic => {
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
    pub workflow_protocol_version: String,
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
pub enum AuthView {
    /// Valid credentials found and tokens are usable
    Authenticated(AuthStatusData),
    /// Credentials exist but tokens are expired or incomplete
    RequiresAttention(AuthStatusData),
    /// A login flow just completed successfully
    LoginSuccess(AuthActionData),
    /// A token refresh completed (`ags auth refresh`).
    RefreshSuccess(AuthActionData),
    /// The resolved access token, emitted by `ags auth token`.
    Token(AuthTokenData),
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

/// The access token `ags auth token` resolved, with the provenance and expiry
/// a caller needs to decide whether to cache it.
///
/// The token itself is a secret, so this type deliberately withholds it from
/// the two paths that would leak it into a log or an unrelated payload:
/// `access_token` is skipped by `Serialize` and redacted by `Debug`. The one
/// command that may emit it writes it out explicitly through its renderer.
#[derive(Clone, serde::Serialize)]
pub struct AuthTokenData {
    /// The bearer token, without the `Bearer ` prefix.
    #[serde(skip)]
    pub access_token: String,
    /// Unix epoch seconds at which the token expires, matching the stored
    /// token's own `expires_at`. `None` when the source does not state an
    /// expiry (an `AGS_ACCESS_TOKEN` supplied by the caller).
    pub expires_at: Option<u64>,
    /// Where the token came from.
    pub source: AuthTokenSource,
    /// Non-fatal notes raised while resolving the token, for stderr.
    pub warnings: Vec<String>,
}

impl std::fmt::Debug for AuthTokenData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthTokenData")
            .field("access_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("source", &self.source)
            .field("warnings", &self.warnings)
            .finish()
    }
}

/// Where the token returned by `ags auth token` came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum AuthTokenSource {
    /// Supplied by the caller through `AGS_ACCESS_TOKEN`.
    Environment,
    /// A stored token that was still valid.
    Stored,
    /// Re-minted through the OAuth refresh-token grant.
    Refreshed,
    /// Newly minted through the client-credentials grant.
    ClientCredentials,
}

/// Outcome of a successful auth action, used in [`AuthActionData::status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum AuthActionStatus {
    /// A login flow just completed and a fresh token was stored.
    LoggedIn,
    /// Login was a no-op because valid credentials already exist.
    AlreadyAuthenticated,
    /// Login refreshed a stale session in place; no fresh OAuth ran.
    Refreshed,
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

/// Whether the env file was written or skipped.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupEnvStatus {
    /// The `.env.local` file was written (or overwritten with `--force`).
    Written,
    /// The `.env.local` file already existed and `--force` was not set.
    Skipped,
}

/// Outcome of an `app-ui setup-env` invocation, ready for rendering.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SetupEnvOutput {
    /// Whether the env file was written or skipped.
    pub status: SetupEnvStatus,
    /// The resolved path to the `.env.local` file.
    pub env_path: String,
}

/// Outcome of an `app-ui upload` invocation, ready for rendering.
///
/// Carries the CSM API response body so `--format json` can surface it
/// verbatim, plus metadata for the human-readable confirmation line.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AppUiUploadOutput {
    /// The App UI name that was uploaded.
    pub name: String,
    /// The build version label.
    pub version: String,
    /// Size of the uploaded archive in bytes.
    pub archive_bytes: u64,
    /// Raw JSON response body from the CSM upload endpoint.
    pub response: serde_json::Value,
}

/// Outcome of a `clone-template` invocation, ready for rendering.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CloneTemplateOutput {
    /// The human-readable template name that was cloned.
    pub template_name: String,
    /// The local destination path the template was cloned into.
    pub destination: String,
    /// The source sub-path extracted (if any).
    pub source_path: Option<String>,
}

/// Outcome of an `extend update-secret` upsert, ready for rendering.
///
/// Deliberately has NO `value` field: the real CSM `UpdateConfigurationV2Response`
/// echoes the plaintext secret value back, and this type must never carry it
/// forward into rendered output.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateSecretOutput {
    /// The secret's CSM config ID (existing or newly created).
    pub config_id: String,
    /// The secret's name (matches `--key`).
    pub config_name: String,
    /// Whether the secret's value is masked in the admin console.
    pub apply_mask: bool,
    /// The secret's description, if any.
    pub description: Option<String>,
    /// Whether this call created a new secret (`true`) or updated an
    /// existing one (`false`).
    pub created: bool,
}

/// Outcome of an `extend update-var` upsert, ready for rendering.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateVarOutput {
    /// The variable's CSM config ID (existing or newly created).
    pub config_id: String,
    /// The variable's name (matches `--key`).
    pub config_name: String,
    /// Whether the variable's value is masked in the admin console.
    pub apply_mask: bool,
    /// The variable's description, if any.
    pub description: Option<String>,
    /// Whether this call created a new variable (`true`) or updated an
    /// existing one (`false`).
    pub created: bool,
}

/// Outcome of an `extend security-assessment request` submission, ready for
/// rendering.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SecurityAssessmentRequestOutput {
    /// The game namespace the engagement was requested in.
    pub namespace: String,
    /// The Extend app name the engagement targets.
    pub app: String,
    /// The created engagement's numeric id (referenced by
    /// `security-assessment result`).
    pub engagement_id: i64,
    /// The engagement's normalized status, as returned by CSM (e.g.
    /// `"SUBMITTED"`).
    pub status: String,
    /// Number of endpoints included in the request.
    pub endpoint_count: usize,
}

/// Outcome of an `extend security-assessment result` report download, ready
/// for rendering.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SecurityAssessmentResultOutput {
    /// The game namespace the engagement belongs to.
    pub namespace: String,
    /// The Extend app name the engagement targets.
    pub app: String,
    /// The downloaded engagement's numeric id.
    pub engagement_id: i64,
    /// The report format requested (`"pdf"` or `"md"`).
    pub report_format: String,
    /// Local file path the report was written to.
    pub path: String,
    /// Number of bytes written.
    pub bytes_written: usize,
}

/// Complete result of a service API call, ready for rendering.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ApiOutput {
    pub operation: OperationSchema,
    pub resource_name: String,
    pub body: ApiBody,
    pub success: Option<ApiSuccess>,
    pub trace: Option<ExecutionTrace>,
    /// The raw JSON response body (post-pagination), retained for workflow
    /// output captures so they run against the real API field names rather than
    /// the presentation-shaped `body`. `None` for non-JSON / empty responses.
    /// Not part of the rendered output (`#[serde(skip)]`).
    #[serde(skip)]
    pub raw_body: Option<serde_json::Value>,
    /// Whether the scope entry that resolved this operation has more than one
    /// API version. Used by the human renderer to decide whether to show the
    /// API version label — the version is only informative when the user could
    /// have chosen a different one. Not part of the serialised output.
    #[serde(skip)]
    pub has_alternate_versions: bool,
}

/// The response body from an API call in its shaped or fallback form.
#[derive(Debug, Clone, serde::Serialize)]
pub enum ApiBody {
    /// A fully shaped, structured result ready for rendering.
    Shaped(Box<crate::result::CommandResult>),
    /// Plain text or non-JSON response
    Text(String),
    /// No response body (e.g. 204 No Content)
    Empty,
}

/// A one-line success message rendered to stderr after a mutating operation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ApiSuccess {
    pub summary: String,
    /// The API version the operation was dispatched against. Populated from
    /// `OperationSchema.api_version` so renderers can show which contract
    /// version was used without parsing the summary string.
    /// Not part of the serialised output.
    #[serde(skip)]
    pub api_version: crate::catalogue::ApiVersion,
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
pub enum ConfigView {
    /// Result of `ags config get` (all keys)
    GetAll {
        profile: String,
        entries: Vec<crate::config::ResolvedEntry>,
    },
    /// Result of `ags config get <key>`
    GetOne {
        key: String,
        value: Option<String>,
        source: crate::config::ConfigSource,
        /// True for the keychain-managed client secret: `value` is never
        /// carried and the entry cannot be written via `ags config set`.
        read_only: bool,
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

/// Wrapper for `ags ams upload` results carrying the view variant to render.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AmsUploadOutput {
    pub view: AmsUploadView,
}

/// The outcome of an AMS dedicated-server image upload.
#[derive(Debug, Clone, serde::Serialize)]
pub enum AmsUploadView {
    /// The image was archived, uploaded, and marked complete.
    Uploaded(AmsUploadResult),
    /// `--dry-run`: what would be uploaded, with no archive built and no
    /// API calls made.
    Planned(AmsUploadPlan),
}

/// How the entrypoint was classified during pre-flight validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum AmsEntrypointKind {
    /// A 64-bit little-endian ELF binary; the architecture was auto-detected.
    ElfBinary,
    /// A shell script (`.sh`); the architecture must be supplied explicitly.
    ShellScript,
}

/// Details of a completed image upload.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AmsUploadResult {
    pub image_id: String,
    pub image_name: String,
    /// Wire value, e.g. `linux-x86_64`.
    pub target_architecture: String,
    /// The entrypoint command recorded on the image, e.g. `./server`.
    pub command: String,
    pub file_count: usize,
    pub archive_bytes: u64,
    /// Number of multipart parts the archive was uploaded in; 1 for a
    /// single-shot presigned PUT.
    pub part_count: usize,
    pub upload_base_url: String,
    /// Symlinked directories left out of the archive, by archive-relative path.
    pub skipped_directory_symlinks: Vec<String>,
}

/// What a `--dry-run` upload would do.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AmsUploadPlan {
    pub image_name: String,
    pub directory: String,
    pub executable: String,
    pub command: String,
    pub target_architecture: String,
    pub entrypoint_kind: AmsEntrypointKind,
    pub file_count: usize,
    /// Total size of the files that would be archived, before compression.
    pub total_bytes: u64,
    pub include_symbol_files: bool,
    /// Excluded symbol files, when any were skipped.
    pub excluded_symbol_file_count: usize,
    /// Symlinked directories left out of the archive, by archive-relative path.
    pub skipped_directory_symlinks: Vec<String>,
    /// The explicit `--upload-url` override, when one was supplied. `None`
    /// means the host is discovered at upload time.
    pub upload_base_url: Option<String>,
}

/// Details of a raw body that was written to stdout or a file via `--output`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BinaryWrittenOutput {
    pub destination: BinaryWrittenDestination,
    pub bytes_written: usize,
    pub content_type: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum BinaryWrittenDestination {
    Stdout,
    File(std::path::PathBuf),
}

/// Outcome of `ags workflow add <path>`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkflowAddOutput {
    pub id: crate::workflow::WorkflowId,
    pub validated_only: bool,
    /// Path the file was installed to; `None` when `validated_only` is true.
    pub path: Option<std::path::PathBuf>,
}

/// Outcome of `ags workflow template`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkflowTemplateOutput {
    pub yaml: String,
    pub destination: BinaryWrittenDestination,
}

/// Outcome of `ags workflow remove <id>`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkflowRemoveOutput {
    pub id: crate::workflow::WorkflowId,
    pub path: std::path::PathBuf,
    /// True when a built-in workflow (Rust or bundled YAML) shares this id.
    /// The removed external file was already shadowed by it (never itself
    /// reachable via `workflow run`), so the built-in remains registered and
    /// unaffected — surfaced so the user doesn't mistake this for the
    /// workflow disappearing entirely.
    pub builtin_still_registered: bool,
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

/// How a captured workflow output value was sourced.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WorkflowOutputProvenance {
    /// Value was captured from a step response.
    Captured,
    /// No step produced a value for this output.
    Missing,
    /// Step was skipped so no value was available.
    Skipped,
}

/// A single resolved output item in the structured workflow output view.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkflowOutputItem {
    /// Alias name declared in the workflow definition.
    pub name: String,
    /// Optional grouping heading for display (e.g. "Moderation").
    pub section: Option<String>,
    /// Human-readable label override; falls back to `name` when absent.
    pub label: Option<String>,
    /// The resolved value (or `null` when `provenance` is `Missing`/`Skipped`).
    pub value: serde_json::Value,
    /// How this value was obtained.
    pub provenance: WorkflowOutputProvenance,
    /// For an array value, the object fields to show per item as a sub-list
    /// (first is the line label, the rest are detail). `None` renders a count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_fields: Option<Vec<String>>,
}

/// Structured output panel for a finished workflow run, grouping resolved
/// output items with section headings, labels, and provenance.
///
/// Human-only — not emitted in `--format json` output (which uses `outputs`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkflowOutputView {
    pub items: Vec<WorkflowOutputItem>,
}

/// Resolved (interpolated) completion attached to a finished workflow run.
/// Mirrors [`crate::workflow::WorkflowCompletion`] but holds final strings, not
/// templates. Reuses the authoring row types since the shape is identical.
/// Serialize-only, like the other output payload views — it is constructed
/// in-process by `resolve_completion` and never parsed back from the wire.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct WorkflowCompletionView {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub created: Vec<crate::workflow::CompletionResource>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub next_steps: Vec<crate::workflow::CompletionStep>,
}

/// How the current copy of the CLI was installed, for the update command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallMethod {
    /// Installed via the shell or PowerShell installer script.
    Installer,
    /// Installed via Homebrew.
    Homebrew,
    /// Binary placed manually or by an unknown mechanism.
    Manual,
}

impl std::fmt::Display for InstallMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Installer => write!(f, "installer"),
            Self::Homebrew => write!(f, "homebrew"),
            Self::Manual => write!(f, "manual"),
        }
    }
}

/// Outcome of `ags update`, ready for rendering.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateOutput {
    /// The version of the running binary.
    pub current: String,
    /// The latest version available on GitHub.
    pub latest: String,
    /// Whether the latest version is newer than the current one.
    pub update_available: bool,
    /// How this copy of the CLI was installed.
    pub install_method: InstallMethod,
    /// Path to the running binary.
    pub binary_path: String,
    /// The exact command to run to upgrade, or `None` for a manual install.
    pub upgrade_command: Option<String>,
    /// The platform-specific download archive name.
    pub download_archive: String,
    /// The release notes page for the latest version.
    pub release_url: String,
}

/// Action taken by `ags update --install`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateInstallAction {
    /// The installer ran and a newer binary was verified in place.
    Installed,
    /// The running version is already the latest; nothing was downloaded.
    AlreadyCurrent,
    /// Dry-run preview; no request was sent and no file was changed.
    DryRun,
}

impl std::fmt::Display for UpdateInstallAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Installed => write!(f, "installed"),
            Self::AlreadyCurrent => write!(f, "already_current"),
            Self::DryRun => write!(f, "dry_run"),
        }
    }
}

/// Outcome of `ags update --install`, ready for rendering.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateInstallOutput {
    /// The action that was taken.
    pub action: UpdateInstallAction,
    /// Path to the binary that was (or would be) replaced.
    pub binary_path: String,
    /// How this copy of the CLI was installed.
    pub install_method: InstallMethod,
    /// The URL the installer script was downloaded from, or `None` when
    /// nothing was downloaded (already current or dry-run).
    pub installer_url: Option<String>,
    /// The latest version available, or `None` under dry-run (no request).
    pub latest: Option<String>,
    /// The version of the binary before the upgrade.
    pub previous: String,
    /// The environment variable pairs the installer would receive (dry-run
    /// renderer only). Not serialised — the JSON contract is the six fields
    /// above.
    #[serde(skip)]
    pub installer_env: Vec<(String, String)>,
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

    /// The token is a secret: neither the derived `Serialize` nor the derived
    /// `Debug` may carry it, so an incidental `to_value` or `{:?}` on a command
    /// output cannot leak it. The one command allowed to emit it writes it out
    /// explicitly in its own renderer.
    #[test]
    fn test_auth_token_view_withholds_the_token_from_serde_and_debug() {
        let view = AuthView::Token(AuthTokenData {
            access_token: "super-secret-token".to_string(),
            expires_at: Some(1_800_000_000),
            source: AuthTokenSource::Stored,
            warnings: vec![],
        });

        let serialised = serde_json::to_string(&view).expect("AuthView must serialise");
        assert!(
            !serialised.contains("super-secret-token"),
            "Serialize must not carry the token: {serialised}"
        );
        assert!(
            serialised.contains("1800000000"),
            "the non-secret fields must still serialise: {serialised}"
        );

        let debugged = format!("{view:?}");
        assert!(
            !debugged.contains("super-secret-token"),
            "Debug must not carry the token: {debugged}"
        );
        assert!(
            debugged.contains("<redacted>"),
            "Debug must say the field was withheld: {debugged}"
        );
    }

    #[test]
    fn test_workflow_output_view_round_trips() {
        let view = WorkflowOutputView {
            items: vec![WorkflowOutputItem {
                name: "ban_active".into(),
                section: Some("Moderation".into()),
                label: Some("Active bans".into()),
                value: serde_json::json!(0),
                provenance: WorkflowOutputProvenance::Captured,
                item_fields: None,
            }],
        };
        let s = serde_json::to_string(&view).unwrap();
        let back: WorkflowOutputView = serde_json::from_str(&s).unwrap();
        assert_eq!(back.items[0].provenance, WorkflowOutputProvenance::Captured);
    }

    #[test]
    fn test_workflow_completion_view_serialises() {
        let v = WorkflowCompletionView {
            created: vec![crate::workflow::CompletionResource {
                label: "Match pool".into(),
                value: "ranked-pool".into(),
            }],
            next_steps: vec![crate::workflow::CompletionStep {
                description: "Inspect the match pool".into(),
                command: "ags matchmaking match-pools get --namespace dev --pool ranked-pool"
                    .into(),
            }],
        };
        let json = serde_json::to_string(&v).expect("WorkflowCompletionView must serialise");
        assert!(json.contains("ranked-pool"));
        assert!(json.contains("match-pools get"));
    }

    #[test]
    fn update_install_output_serialises_six_keys() {
        let output = UpdateInstallOutput {
            action: UpdateInstallAction::Installed,
            binary_path: "/usr/local/bin/ags".to_string(),
            install_method: InstallMethod::Installer,
            installer_url: Some("https://example.com/installer.sh".to_string()),
            latest: Some("0.5.2".to_string()),
            previous: "0.5.1".to_string(),
            installer_env: vec![("FOO".to_string(), "bar".to_string())],
        };
        let value = serde_json::to_value(&output).expect("must serialise");
        let obj = value.as_object().expect("must be an object");
        assert_eq!(
            obj.len(),
            6,
            "expected exactly 6 keys, got: {:?}",
            obj.keys().collect::<Vec<_>>()
        );
        // Keys must be in alphabetical order (serde_json::Value::Object uses
        // a BTreeMap).
        let keys: Vec<&String> = obj.keys().collect();
        assert_eq!(
            keys,
            &[
                "action",
                "binary_path",
                "install_method",
                "installer_url",
                "latest",
                "previous"
            ]
        );
        // installer_env must NOT appear.
        assert!(!obj.contains_key("installer_env"));
    }
}
