//! Handler for `ags extend app-ui setup-env`.
//!
//! Reads the App UI record from the CSM `ListAppUI` endpoint, extracts
//! the four VITE_AB_* environment variables, and writes (or upserts) a
//! `.env.local` file in the project directory. Preserves comments, blank
//! lines, and unmanaged keys — only the four managed keys are touched.

use std::path::{Path, PathBuf};

use clap::ArgMatches;

use crate::errors::CliError;
use crate::frontend::{write_stderr_line, Frontend};
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use ags_protocol::output::{CommandOutput, SetupEnvOutput, SetupEnvStatus};

// ── Managed keys ──

/// The four environment keys written by this command, in the fixed order
/// matching the Go source. The order determines the append sequence for
/// keys not already present in the template.
const MANAGED_KEYS: [&str; 4] = [
    "VITE_AB_REDIRECT_URI",
    "VITE_AB_BASE_URL",
    "VITE_AB_NAMESPACE",
    "VITE_AB_CLIENT_ID",
];

const ENV_LOCAL_FILENAME: &str = ".env.local";
const ENV_EXAMPLE_FILENAME: &str = ".env.example";

// ── CSM response types (deserialization only) ──

#[derive(serde::Deserialize)]
struct ListAppUiResponse {
    #[serde(default)]
    data: Vec<AppUiRecord>,
}

#[derive(Debug, serde::Deserialize)]
struct AppUiRecord {
    #[serde(default)]
    name: String,
    #[serde(rename = "publicIamClient")]
    public_iam_client: Option<IamClient>,
}

#[derive(Debug, serde::Deserialize)]
struct IamClient {
    #[serde(rename = "clientId", default)]
    client_id: String,
    #[serde(rename = "redirectUriList", default)]
    redirect_uri_list: Vec<String>,
}

// ── Entry point ──

/// Execute the `app-ui setup-env` command.
///
/// Validates inputs and the fast-exit guard before authenticating, so
/// path errors and the skip path never trigger a network round-trip.
pub(crate) async fn handle_app_ui_setup_env(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    frontend: &mut dyn Frontend,
) -> Result<InvocationOutcome, CliError> {
    let name = matches
        .get_one::<String>("name")
        .ok_or_else(|| CliError::Usage {
            message: "--name is required".to_string(),
            metadata: None,
        })?;

    let project_path_raw = matches
        .get_one::<String>("project-path")
        .map(|s| s.as_str())
        .unwrap_or(".");

    let force = matches.get_flag("force");

    // Step 1: validate project path (before auth).
    let project_path = validate_project_path(project_path_raw)?;

    // Step 2: validate namespace is supplied.
    let namespace = validate_required_namespace(flags.namespace.as_deref())?;

    let env_local_path = project_path.join(ENV_LOCAL_FILENAME);

    // Step 3: fast-exit guard — skip if .env.local exists and --force is not set.
    if let Some(output) = fast_exit_guard(&env_local_path, force, flags.is_auto_confirmed) {
        frontend.render(&CommandOutput::SetupEnv(output))?;
        return Ok(InvocationOutcome::Complete);
    }

    // Step 4: dry-run — no auth, no network, no write.
    if flags.is_dry_run {
        return dry_run_preview(&env_local_path, name, namespace);
    }

    // Step 5: auth — resolve execution context and build HTTP client.
    let input = ags_runtime::runtime::execution::ResolutionInput {
        profile: flags.profile.clone(),
        namespace: flags.namespace.clone(),
        is_dry_run: false,
    };
    let http_client = ags_runtime::runtime::dispatch::http::build_http_client(flags.timeout)?;
    let context =
        ags_runtime::runtime::execution::ExecutionContext::resolve(&input, &http_client).await?;

    let resolved_namespace = context.namespace.as_deref().unwrap_or(namespace);
    let base_url = &context.base_url;

    // Step 6: call CSM ListAppUI and match by name.
    let record = fetch_app_ui_record(
        &http_client,
        base_url,
        &context.access_token,
        resolved_namespace,
        name,
    )
    .await?;

    // Step 7: extract env values from the record.
    let values = extract_env_values(&record, base_url, resolved_namespace, name)?;

    // Step 8: load template and upsert.
    let env_example_path = project_path.join(ENV_EXAMPLE_FILENAME);
    let template = load_template(&env_example_path)?;
    let content = upsert_env_values(&template, &values);

    // Step 9: atomic write.
    atomic_write_env_file(&env_local_path, &content)?;

    frontend.render(&CommandOutput::SetupEnv(SetupEnvOutput {
        status: SetupEnvStatus::Written,
        env_path: env_local_path.display().to_string(),
    }))?;

    Ok(InvocationOutcome::Complete)
}

// ── Path validation ──

/// Resolve and validate the project path. Returns the canonicalized path
/// or a `CliError::Usage` if the path does not exist or is not a directory.
fn validate_project_path(raw: &str) -> Result<PathBuf, CliError> {
    let path = PathBuf::from(raw);
    if !path.exists() {
        return Err(CliError::Usage {
            message: format!("project path '{}' was not found", path.display()),
            metadata: None,
        });
    }
    if !path.is_dir() {
        return Err(CliError::Usage {
            message: format!("'{}' is not a directory", path.display()),
            metadata: None,
        });
    }
    // Canonicalize to resolve relative paths and symlinks.
    std::fs::canonicalize(&path).map_err(|e| CliError::Usage {
        message: format!("failed to resolve project path '{}': {e}", path.display()),
        metadata: None,
    })
}

// ── Namespace validation ──

/// Validate that a namespace was supplied. Returns the namespace string
/// or a `CliError::Usage` explaining the requirement.
fn validate_required_namespace(namespace: Option<&str>) -> Result<&str, CliError> {
    namespace.ok_or_else(|| CliError::Usage {
        message: "--namespace is required for app-ui setup-env".to_string(),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            "Supply --namespace <ns> or set a default via 'ags config set namespace <ns>'",
        ))),
    })
}

// ── Fast-exit guard ──

/// Return `Some(SetupEnvOutput)` with status `Skipped` when `.env.local`
/// already exists and neither `--force` nor `--yes` overrides the guard.
/// The caller is responsible for rendering and returning early.
fn fast_exit_guard(
    env_local_path: &Path,
    force: bool,
    is_auto_confirmed: bool,
) -> Option<SetupEnvOutput> {
    if env_local_path.exists() && !force && !is_auto_confirmed {
        Some(SetupEnvOutput {
            status: SetupEnvStatus::Skipped,
            env_path: env_local_path.display().to_string(),
        })
    } else {
        None
    }
}

// ── Dry-run preview ──

fn dry_run_preview(
    env_path: &Path,
    name: &str,
    namespace: &str,
) -> Result<InvocationOutcome, CliError> {
    let color = crate::frontend::style::is_stderr_enabled();
    write_stderr_line(&crate::frontend::style::info(
        "Dry run — no files will be written",
        color,
    ));
    write_stderr_line(&format!("  App UI:    {name}"));
    write_stderr_line(&format!("  Namespace: {namespace}"));
    write_stderr_line(&format!("  Env path:  {}", env_path.display()));
    write_stderr_line(&format!("  Keys:      {}", MANAGED_KEYS.join(", ")));
    Ok(InvocationOutcome::Complete)
}

// ── CSM API call ──

/// Page size used when walking the ListAppUI endpoint.
const PAGE_LIMIT: u64 = 100;

/// Maximum number of pages to walk before giving up. Prevents infinite
/// loops when a misbehaving server always returns a full page.
const MAX_PAGES: u64 = 50;

/// Fetch the App UI record from CSM by paging through the ListAppUI
/// endpoint and matching by exact name equality (case-sensitive). The
/// `name` query parameter is passed to the server as a hint but the
/// authoritative match is client-side (case-sensitive). Pages are walked
/// until the exact match is found, a short page signals the end of
/// results, or the page cap is reached.
async fn fetch_app_ui_record(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    namespace: &str,
    name: &str,
) -> Result<AppUiRecord, CliError> {
    fetch_app_ui_record_paged(
        client,
        base_url,
        access_token,
        namespace,
        name,
        PAGE_LIMIT,
        MAX_PAGES,
    )
    .await
}

/// Inner implementation of [`fetch_app_ui_record`] that accepts pagination
/// parameters. Production callers use the constants via the wrapper; tests
/// can pass smaller values to keep mock data manageable.
async fn fetch_app_ui_record_paged(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    namespace: &str,
    name: &str,
    page_limit: u64,
    max_pages: u64,
) -> Result<AppUiRecord, CliError> {
    let url = format!(
        "{}/csm/v1/admin/namespaces/{}/app-ui",
        base_url.trim_end_matches('/'),
        namespace
    );

    let mut offset: u64 = 0;
    let mut saw_short_page = false;

    for _ in 0..max_pages {
        let response = client
            .get(&url)
            .query(&[
                ("name", name),
                ("limit", &page_limit.to_string()),
                ("offset", &offset.to_string()),
            ])
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| CliError::Network {
                message: format!("failed to query app UI list: {e}"),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Check your network connection and base URL",
                ))),
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(CliError::Api {
                message: format!(
                    "CSM ListAppUI returned HTTP {status} for namespace '{namespace}'"
                ),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Check the namespace and your permissions",
                ))),
                category: crate::errors::ApiErrorCategory::Upstream,
            });
        }

        let body: ListAppUiResponse = response.json().await.map_err(|e| CliError::Api {
            message: format!("failed to parse CSM ListAppUI response: {e}"),
            metadata: None,
            category: crate::errors::ApiErrorCategory::Upstream,
        })?;

        let page_len = body.data.len() as u64;

        // Client-side exact name match (case-sensitive), matching Go behaviour.
        if let Some(record) = body.data.into_iter().find(|r| r.name == name) {
            return Ok(record);
        }

        // A short page means the server has no more results.
        if page_len < page_limit {
            saw_short_page = true;
            break;
        }

        offset += page_len;
    }

    if saw_short_page {
        Err(CliError::Usage {
            message: format!("app UI '{name}' was not found in namespace '{namespace}'"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check the app UI name with 'ags csm app-ui list'",
            ))),
        })
    } else {
        Err(CliError::Usage {
            message: format!(
                "app UI '{name}' was not found within the first {offset} records \
                 in namespace '{namespace}'"
            ),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "The namespace may contain more records than were searched. \
                 Try a more specific name.",
            ))),
        })
    }
}

// ── Field validation and extraction ──

/// Extract the four VITE_AB_* values from a CSM App UI record. Validates
/// that the IAM client, client ID, and redirect URI list are present and
/// non-empty before any file is touched.
fn extract_env_values(
    record: &AppUiRecord,
    base_url: &str,
    namespace: &str,
    name: &str,
) -> Result<Vec<(String, String)>, CliError> {
    let iam = record
        .public_iam_client
        .as_ref()
        .ok_or_else(|| CliError::Api {
            message: format!("app UI '{name}' has no IAM client configured"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Configure the public IAM client for this App UI in the admin console",
            ))),
            category: crate::errors::ApiErrorCategory::Upstream,
        })?;

    if iam.client_id.is_empty() {
        return Err(CliError::Api {
            message: format!("app UI '{name}' IAM client has no client ID"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Configure the client ID for this App UI's IAM client",
            ))),
            category: crate::errors::ApiErrorCategory::Upstream,
        });
    }

    if iam.redirect_uri_list.is_empty() {
        return Err(CliError::Api {
            message: format!("app UI '{name}' IAM client has no redirect URI"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Configure at least one redirect URI for this App UI's IAM client",
            ))),
            category: crate::errors::ApiErrorCategory::Upstream,
        });
    }

    let redirect_uri = &iam.redirect_uri_list[0];

    Ok(vec![
        ("VITE_AB_REDIRECT_URI".to_string(), redirect_uri.clone()),
        ("VITE_AB_BASE_URL".to_string(), base_url.to_string()),
        ("VITE_AB_NAMESPACE".to_string(), namespace.to_string()),
        ("VITE_AB_CLIENT_ID".to_string(), iam.client_id.clone()),
    ])
}

// ── Template loading ──

/// Load the `.env.example` file as a template base. Returns an empty
/// string if the file does not exist. Returns `CliError::Internal` on
/// I/O errors other than not-found.
fn load_template(path: &Path) -> Result<String, CliError> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(CliError::Internal(anyhow::anyhow!(
            "failed to read '{}': {e}",
            path.display()
        ))),
    }
}

// ── Env-file upsert (pure logic) ──

/// Replace a single key's value in the content string if the key is found.
/// Returns `(new_content, was_replaced)`. The regex matches lines with
/// optional leading whitespace and an optional `export` prefix, replacing
/// the entire line with the bare `KEY=VALUE` form (no prefix). This
/// preserves the line's ordinal position and matches Go behaviour.
fn replace_env_key(content: &str, key: &str, value: &str) -> (String, bool) {
    let pattern = format!(
        r"(?m)^[ \t]*(?:export[ \t]+)?{}[ \t]*=.*$",
        regex::escape(key)
    );
    let re = regex::Regex::new(&pattern).expect("managed key pattern must compile");
    if !re.is_match(content) {
        return (content.to_string(), false);
    }
    let replacement = format!("{key}={value}");
    (
        re.replace_all(content, replacement.as_str()).to_string(),
        true,
    )
}

/// Upsert the managed keys into the content string. For each key, if it
/// is found in the content, replace in-place; otherwise append at the end.
/// Ensures the result ends with a trailing newline.
fn upsert_env_values(content: &str, values: &[(String, String)]) -> String {
    let mut result = content.to_string();

    for (key, value) in values {
        let (updated, replaced) = replace_env_key(&result, key, value);
        if replaced {
            result = updated;
        } else {
            // Append. Ensure the previous content ends with a newline
            // before appending so the new key starts on its own line.
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(&format!("{key}={value}\n"));
        }
    }

    // Ensure trailing newline.
    if !result.ends_with('\n') {
        result.push('\n');
    }

    result
}

// ── Atomic write ──

/// Write content to the target path atomically: write to a temporary file
/// in the same directory, then rename into place. On rename failure the
/// temporary file is removed. Permissions are 0o644 (world-readable for
/// build tools), matching the Go source.
fn atomic_write_env_file(target: &Path, content: &str) -> Result<(), CliError> {
    use std::io::Write;

    let dir = target.parent().ok_or_else(|| {
        CliError::Internal(anyhow::anyhow!(
            "cannot determine parent directory of '{}'",
            target.display()
        ))
    })?;

    let mut tmp = tempfile::Builder::new()
        .prefix(".ags-tmp-")
        .tempfile_in(dir)
        .map_err(|e| {
            CliError::Internal(anyhow::anyhow!(
                "failed to create temporary file in '{}': {e}",
                dir.display()
            ))
        })?;

    tmp.write_all(content.as_bytes())
        .map_err(|e| CliError::Internal(anyhow::anyhow!("failed to write temporary file: {e}")))?;

    tmp.as_file()
        .sync_all()
        .map_err(|e| CliError::Internal(anyhow::anyhow!("failed to sync temporary file: {e}")))?;

    // Set permissions to 0o644 on Unix (world-readable for build tools).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o644);
        tmp.as_file().set_permissions(perms).map_err(|e| {
            CliError::Internal(anyhow::anyhow!(
                "failed to set permissions on temporary file: {e}"
            ))
        })?;
    }

    tmp.persist(target).map_err(|e| {
        // persist() consumes the NamedTempFile; on failure the temp file
        // is automatically cleaned up by the PersistError's Drop impl.
        CliError::Internal(anyhow::anyhow!(
            "failed to rename temporary file to '{}': {}",
            target.display(),
            e.error
        ))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Path validation ──

    #[test]
    fn test_validate_project_path_nonexistent_errors() {
        let result = validate_project_path("/nonexistent/path/that/does/not/exist");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("was not found"),
            "expected 'was not found' in: {msg}"
        );
    }

    #[test]
    fn test_validate_project_path_file_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("a-file.txt");
        std::fs::write(&file, "content").unwrap();
        let result = validate_project_path(file.to_str().unwrap());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("is not a directory"),
            "expected 'is not a directory' in: {msg}"
        );
    }

    #[test]
    fn test_validate_project_path_valid_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = validate_project_path(dir.path().to_str().unwrap());
        assert!(result.is_ok());
    }

    // ── Fast-exit guard ──

    #[test]
    fn test_fast_exit_guard_skips_when_env_local_exists_and_no_force() {
        let dir = tempfile::TempDir::new().unwrap();
        let env_local = dir.path().join(ENV_LOCAL_FILENAME);
        std::fs::write(&env_local, "EXISTING=keep_me\n").unwrap();

        let result = fast_exit_guard(&env_local, false, false);
        assert!(
            result.is_some(),
            "guard must fire when .env.local exists without --force"
        );
        assert_eq!(result.unwrap().status, SetupEnvStatus::Skipped);
    }

    #[test]
    fn test_fast_exit_guard_proceeds_when_force_is_set() {
        let dir = tempfile::TempDir::new().unwrap();
        let env_local = dir.path().join(ENV_LOCAL_FILENAME);
        std::fs::write(&env_local, "EXISTING=content\n").unwrap();

        let result = fast_exit_guard(&env_local, true, false);
        assert!(result.is_none(), "guard must not fire when --force is set");
    }

    #[test]
    fn test_fast_exit_guard_proceeds_when_file_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let env_local = dir.path().join(ENV_LOCAL_FILENAME);

        let result = fast_exit_guard(&env_local, false, false);
        assert!(
            result.is_none(),
            "guard must not fire when .env.local does not exist"
        );
    }

    // ── replace_env_key ──

    #[test]
    fn test_replace_env_key_replaces_existing_key() {
        let content = "VITE_AB_CLIENT_ID=old_value\nOTHER=keep\n";
        let (result, replaced) = replace_env_key(content, "VITE_AB_CLIENT_ID", "new_value");
        assert!(replaced);
        assert!(result.contains("VITE_AB_CLIENT_ID=new_value"));
        assert!(result.contains("OTHER=keep"));
    }

    #[test]
    fn test_replace_env_key_missing_key_returns_unchanged() {
        let content = "OTHER=keep\n";
        let (result, replaced) = replace_env_key(content, "VITE_AB_CLIENT_ID", "new");
        assert!(!replaced);
        assert_eq!(result, content);
    }

    #[test]
    fn test_replace_env_key_strips_export_prefix() {
        let content = "export VITE_AB_CLIENT_ID=old\n";
        let (result, replaced) = replace_env_key(content, "VITE_AB_CLIENT_ID", "new");
        assert!(replaced);
        assert_eq!(result, "VITE_AB_CLIENT_ID=new\n");
    }

    #[test]
    fn test_replace_env_key_strips_leading_whitespace() {
        let content = "  VITE_AB_CLIENT_ID=old\n";
        let (result, replaced) = replace_env_key(content, "VITE_AB_CLIENT_ID", "new");
        assert!(replaced);
        assert_eq!(result, "VITE_AB_CLIENT_ID=new\n");
    }

    #[test]
    fn test_replace_env_key_strips_export_with_whitespace() {
        let content = "  export VITE_AB_CLIENT_ID = old_val\n";
        let (result, replaced) = replace_env_key(content, "VITE_AB_CLIENT_ID", "new");
        assert!(replaced);
        assert_eq!(result, "VITE_AB_CLIENT_ID=new\n");
    }

    #[test]
    fn test_replace_env_key_preserves_line_position() {
        let content = "LINE1=a\nVITE_AB_CLIENT_ID=old\nLINE3=c\n";
        let (result, replaced) = replace_env_key(content, "VITE_AB_CLIENT_ID", "new");
        assert!(replaced);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "LINE1=a");
        assert_eq!(lines[1], "VITE_AB_CLIENT_ID=new");
        assert_eq!(lines[2], "LINE3=c");
    }

    #[test]
    fn test_replace_env_key_value_with_equals_sign() {
        let content = "VITE_AB_BASE_URL=https://old.example.com\n";
        let (result, replaced) =
            replace_env_key(content, "VITE_AB_BASE_URL", "https://new.example.com?a=1");
        assert!(replaced);
        assert_eq!(result, "VITE_AB_BASE_URL=https://new.example.com?a=1\n");
    }

    #[test]
    fn test_replace_env_key_value_with_spaces() {
        let content = "VITE_AB_BASE_URL=old\n";
        let (result, replaced) =
            replace_env_key(content, "VITE_AB_BASE_URL", "has spaces in value");
        assert!(replaced);
        assert_eq!(result, "VITE_AB_BASE_URL=has spaces in value\n");
    }

    // ── upsert_env_values ──

    #[test]
    fn test_upsert_new_file_creates_all_four_keys() {
        let values = vec![
            (
                "VITE_AB_REDIRECT_URI".to_string(),
                "http://localhost:3000".to_string(),
            ),
            (
                "VITE_AB_BASE_URL".to_string(),
                "https://api.example.com".to_string(),
            ),
            ("VITE_AB_NAMESPACE".to_string(), "test-ns".to_string()),
            ("VITE_AB_CLIENT_ID".to_string(), "client123".to_string()),
        ];
        let result = upsert_env_values("", &values);
        assert!(result.contains("VITE_AB_REDIRECT_URI=http://localhost:3000"));
        assert!(result.contains("VITE_AB_BASE_URL=https://api.example.com"));
        assert!(result.contains("VITE_AB_NAMESPACE=test-ns"));
        assert!(result.contains("VITE_AB_CLIENT_ID=client123"));
        assert!(result.ends_with('\n'));
    }

    #[test]
    fn test_upsert_replaces_existing_keys_in_place() {
        let template = "VITE_AB_CLIENT_ID=old_value\nUNRELATED=keep_me\n";
        let values = vec![
            (
                "VITE_AB_REDIRECT_URI".to_string(),
                "http://localhost:3000".to_string(),
            ),
            (
                "VITE_AB_BASE_URL".to_string(),
                "https://api.example.com".to_string(),
            ),
            ("VITE_AB_NAMESPACE".to_string(), "test-ns".to_string()),
            ("VITE_AB_CLIENT_ID".to_string(), "new_client".to_string()),
        ];
        let result = upsert_env_values(template, &values);

        // Existing key replaced in-place.
        assert!(result.contains("VITE_AB_CLIENT_ID=new_client"));
        assert!(!result.contains("old_value"));

        // Unrelated key preserved.
        assert!(result.contains("UNRELATED=keep_me"));

        // Missing keys appended.
        assert!(result.contains("VITE_AB_REDIRECT_URI=http://localhost:3000"));
    }

    #[test]
    fn test_upsert_preserves_comments_and_blank_lines() {
        let template = "# This is a comment\n\nVITE_AB_CLIENT_ID=old\n# Another comment\n";
        let values = vec![("VITE_AB_CLIENT_ID".to_string(), "new".to_string())];
        let result = upsert_env_values(template, &values);
        assert!(result.contains("# This is a comment"));
        assert!(result.contains("# Another comment"));
        assert!(result.contains("VITE_AB_CLIENT_ID=new"));

        // Verify blank line is preserved (the template has a blank line between
        // the comment and the key).
        let lines: Vec<&str> = result.lines().collect();
        assert!(
            lines.iter().any(|l| l.is_empty()),
            "blank lines must survive: {result:?}"
        );
    }

    #[test]
    fn test_upsert_unrelated_key_survives() {
        let template = "CUSTOM_KEY=my_value\nVITE_AB_CLIENT_ID=old\n";
        let values = vec![("VITE_AB_CLIENT_ID".to_string(), "new".to_string())];
        let result = upsert_env_values(template, &values);
        assert!(result.contains("CUSTOM_KEY=my_value"));
        assert!(result.contains("VITE_AB_CLIENT_ID=new"));
    }

    #[test]
    fn test_upsert_appends_missing_keys_from_template_without_vite_keys() {
        let template = "OTHER_KEY=value\n";
        let values = vec![
            (
                "VITE_AB_REDIRECT_URI".to_string(),
                "http://localhost:3000".to_string(),
            ),
            (
                "VITE_AB_BASE_URL".to_string(),
                "https://api.example.com".to_string(),
            ),
            ("VITE_AB_NAMESPACE".to_string(), "ns".to_string()),
            ("VITE_AB_CLIENT_ID".to_string(), "cid".to_string()),
        ];
        let result = upsert_env_values(template, &values);

        // All original content preserved.
        assert!(result.contains("OTHER_KEY=value"));

        // All four VITE keys present.
        for key in &MANAGED_KEYS {
            assert!(
                result.contains(key),
                "result must contain {key}: {result:?}"
            );
        }
    }

    #[test]
    fn test_upsert_trailing_newline_ensured() {
        let values = vec![("KEY".to_string(), "val".to_string())];
        let result = upsert_env_values("EXISTING=no_newline", &values);
        assert!(
            result.ends_with('\n'),
            "result must end with newline: {result:?}"
        );
    }

    // ── extract_env_values ──

    #[test]
    fn test_extract_env_values_success() {
        let record = AppUiRecord {
            name: "my-app".to_string(),
            public_iam_client: Some(IamClient {
                client_id: "client123".to_string(),
                redirect_uri_list: vec!["http://localhost:3000".to_string()],
            }),
        };
        let values =
            extract_env_values(&record, "https://api.example.com", "test-ns", "my-app").unwrap();
        assert_eq!(values.len(), 4);
        assert_eq!(
            values[0],
            (
                "VITE_AB_REDIRECT_URI".to_string(),
                "http://localhost:3000".to_string()
            )
        );
        assert_eq!(
            values[1],
            (
                "VITE_AB_BASE_URL".to_string(),
                "https://api.example.com".to_string()
            )
        );
        assert_eq!(
            values[2],
            ("VITE_AB_NAMESPACE".to_string(), "test-ns".to_string())
        );
        assert_eq!(
            values[3],
            ("VITE_AB_CLIENT_ID".to_string(), "client123".to_string())
        );
    }

    #[test]
    fn test_extract_env_values_no_iam_client_errors() {
        let record = AppUiRecord {
            name: "my-app".to_string(),
            public_iam_client: None,
        };
        let result = extract_env_values(&record, "https://api.example.com", "ns", "my-app");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, CliError::Api { .. }));
        assert!(
            err.to_string().contains("has no IAM client configured"),
            "expected 'has no IAM client configured' in: {err}"
        );
    }

    #[test]
    fn test_extract_env_values_empty_client_id_errors() {
        let record = AppUiRecord {
            name: "my-app".to_string(),
            public_iam_client: Some(IamClient {
                client_id: String::new(),
                redirect_uri_list: vec!["http://localhost:3000".to_string()],
            }),
        };
        let result = extract_env_values(&record, "https://api.example.com", "ns", "my-app");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, CliError::Api { .. }));
        assert!(
            err.to_string().contains("has no client ID"),
            "expected 'has no client ID' in: {err}"
        );
    }

    #[test]
    fn test_extract_env_values_empty_redirect_uri_list_errors() {
        let record = AppUiRecord {
            name: "my-app".to_string(),
            public_iam_client: Some(IamClient {
                client_id: "client123".to_string(),
                redirect_uri_list: vec![],
            }),
        };
        let result = extract_env_values(&record, "https://api.example.com", "ns", "my-app");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, CliError::Api { .. }));
        assert!(
            err.to_string().contains("has no redirect URI"),
            "expected 'has no redirect URI' in: {err}"
        );
    }

    // ── Client-side exact-name matching (exercised inline by the page loop) ──

    #[test]
    fn test_app_ui_not_found_in_records() {
        let records = vec![AppUiRecord {
            name: "other-app".to_string(),
            public_iam_client: None,
        }];
        // The page loop uses `.find(|r| r.name == name)` for client-side matching.
        let found = records.into_iter().find(|r| r.name == "my-app");
        assert!(found.is_none(), "no record should match a different name");
    }

    #[test]
    fn test_app_ui_found_by_exact_name() {
        let records = vec![
            AppUiRecord {
                name: "other-app".to_string(),
                public_iam_client: None,
            },
            AppUiRecord {
                name: "my-app".to_string(),
                public_iam_client: Some(IamClient {
                    client_id: "cid".to_string(),
                    redirect_uri_list: vec!["http://localhost".to_string()],
                }),
            },
        ];
        let found = records.into_iter().find(|r| r.name == "my-app");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "my-app");
    }

    // ── Pagination (wiremock-backed) ──

    /// The exact-name record sits on the second page. The lookup must walk
    /// past the first full page and find it on the second.
    #[tokio::test]
    async fn test_pagination_walks_to_record_on_second_page() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let page_limit: u64 = 2;
        let max_pages: u64 = 3;

        // Page 1 (offset=0): full page, no match.
        Mock::given(method("GET"))
            .and(path("/csm/v1/admin/namespaces/test-ns/app-ui"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"name": "other-1"},
                    {"name": "other-2"}
                ]
            })))
            .mount(&server)
            .await;

        // Page 2 (offset=2): contains the target record.
        Mock::given(method("GET"))
            .and(path("/csm/v1/admin/namespaces/test-ns/app-ui"))
            .and(query_param("offset", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {
                        "name": "target-app",
                        "publicIamClient": {
                            "clientId": "cid",
                            "redirectUriList": ["http://localhost"]
                        }
                    }
                ]
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = fetch_app_ui_record_paged(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "target-app",
            page_limit,
            max_pages,
        )
        .await;

        let record = result.expect("should find the record on the second page");
        assert_eq!(record.name, "target-app");
    }

    /// A short page (fewer than `page_limit` items) with no match terminates
    /// the loop and produces the true-miss message (no record-count language).
    #[tokio::test]
    async fn test_pagination_short_page_produces_true_miss_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let page_limit: u64 = 2;
        let max_pages: u64 = 3;

        // Single short page (1 record < page_limit of 2), no match.
        Mock::given(method("GET"))
            .and(path("/csm/v1/admin/namespaces/test-ns/app-ui"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"name": "other-app"}
                ]
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = fetch_app_ui_record_paged(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "missing-app",
            page_limit,
            max_pages,
        )
        .await;

        let err = result.expect_err("should return an error for a true miss");
        let msg = err.to_string();
        assert!(
            msg.contains("was not found in namespace"),
            "true-miss must say 'was not found in namespace': {msg}"
        );
        // Must NOT contain the cap-exhaustion language.
        assert!(
            !msg.contains("within the first"),
            "true-miss must NOT mention record count: {msg}"
        );
    }

    /// Cap exhaustion (every page is full, no match) produces a distinct
    /// error message that mentions the number of records searched.
    #[tokio::test]
    async fn test_pagination_cap_exhaustion_produces_distinct_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let page_limit: u64 = 2;
        let max_pages: u64 = 3;

        // Every page returns exactly page_limit records, no match.
        Mock::given(method("GET"))
            .and(path("/csm/v1/admin/namespaces/test-ns/app-ui"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"name": "other-1"},
                    {"name": "other-2"}
                ]
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = fetch_app_ui_record_paged(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "missing-app",
            page_limit,
            max_pages,
        )
        .await;

        let err = result.expect_err("should return an error for cap exhaustion");
        let msg = err.to_string();

        // Must mention the record count to distinguish from a true miss.
        let expected_count = max_pages * page_limit; // 6
        assert!(
            msg.contains(&format!("within the first {expected_count} records")),
            "cap error must mention record count ({expected_count}): {msg}"
        );

        // Confirm the two messages differ: the true-miss message must NOT
        // appear as a substring of the cap-exhaustion message's core phrase.
        let true_miss_msg = "app UI 'missing-app' was not found in namespace 'test-ns'".to_string();
        assert_ne!(
            msg, true_miss_msg,
            "cap error must differ from the true-miss message"
        );
    }

    /// A failed HTTP call (non-2xx) from the CSM ListAppUI endpoint produces
    /// a `CliError::Api` with the HTTP status and namespace in the message.
    #[tokio::test]
    async fn test_fetch_app_ui_record_failed_call_produces_api_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/csm/v1/admin/namespaces/test-ns/app-ui"))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "errorCode": 20013,
                "errorMessage": "insufficient permissions"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = fetch_app_ui_record_paged(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            10,
            3,
        )
        .await;

        let err = result.expect_err("a 403 response should produce an error");
        assert!(
            matches!(err, CliError::Api { .. }),
            "expected Api variant, got: {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("403"),
            "error must mention the HTTP status: {msg}"
        );
        assert!(
            msg.contains("test-ns"),
            "error must mention the namespace: {msg}"
        );
    }

    // ── load_template ──

    #[test]
    fn test_load_template_missing_file_returns_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.txt");
        let result = load_template(&path).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_load_template_existing_file_returns_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(".env.example");
        std::fs::write(&path, "KEY=value\n").unwrap();
        let result = load_template(&path).unwrap();
        assert_eq!(result, "KEY=value\n");
    }

    // ── atomic_write_env_file ──

    #[test]
    fn test_atomic_write_creates_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let target = dir.path().join(ENV_LOCAL_FILENAME);
        atomic_write_env_file(&target, "KEY=value\n").unwrap();
        assert!(target.exists());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "KEY=value\n");
    }

    #[test]
    fn test_atomic_write_overwrites_existing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let target = dir.path().join(ENV_LOCAL_FILENAME);
        std::fs::write(&target, "OLD=content\n").unwrap();
        atomic_write_env_file(&target, "NEW=content\n").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "NEW=content\n");
    }

    #[test]
    fn test_atomic_write_no_tmp_file_left_behind() {
        let dir = tempfile::TempDir::new().unwrap();
        let target = dir.path().join(ENV_LOCAL_FILENAME);
        atomic_write_env_file(&target, "KEY=value\n").unwrap();

        // No .ags-tmp-* files should remain in the directory.
        let tmp_files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".ags-tmp-"))
            .collect();
        assert!(
            tmp_files.is_empty(),
            "no temporary files should remain after successful write: {tmp_files:?}"
        );
    }

    // ── Dry-run ──

    #[test]
    fn test_dry_run_writes_nothing() {
        let dir = tempfile::TempDir::new().unwrap();
        let env_path = dir.path().join(ENV_LOCAL_FILENAME);
        let result = dry_run_preview(&env_path, "my-app", "test-ns");
        assert!(result.is_ok());
        assert!(!env_path.exists(), "dry-run must not create the env file");
    }

    // ── Namespace validation ──

    #[test]
    fn test_namespace_required() {
        let err = validate_required_namespace(None);
        assert!(err.is_err());
        let cli_err = err.unwrap_err();
        assert!(
            matches!(cli_err, CliError::Usage { .. }),
            "expected Usage variant, got: {cli_err:?}"
        );
        assert_eq!(
            cli_err.exit_code(),
            1,
            "namespace-required error must exit with code 1"
        );
        assert!(
            cli_err.to_string().contains("--namespace is required"),
            "error must mention --namespace: {cli_err}"
        );
    }

    #[test]
    fn test_namespace_present_passes_validation() {
        let result = validate_required_namespace(Some("my-ns"));
        assert_eq!(result.unwrap(), "my-ns");
    }

    // ── Integration-style pure-logic test ──

    #[test]
    fn test_full_upsert_from_template_with_existing_keys() {
        let template = "\
# App UI Environment\n\
\n\
VITE_AB_BASE_URL=https://old.example.com\n\
VITE_AB_CLIENT_ID=old_client\n\
CUSTOM_PORT=3000\n\
";
        let record = AppUiRecord {
            name: "test-app".to_string(),
            public_iam_client: Some(IamClient {
                client_id: "new_client_id".to_string(),
                redirect_uri_list: vec!["http://localhost:8080".to_string()],
            }),
        };
        let values = extract_env_values(
            &record,
            "https://new.api.example.com",
            "production",
            "test-app",
        )
        .unwrap();
        let result = upsert_env_values(template, &values);

        // Replaced keys have new values.
        assert!(result.contains("VITE_AB_BASE_URL=https://new.api.example.com"));
        assert!(result.contains("VITE_AB_CLIENT_ID=new_client_id"));

        // Appended keys are present.
        assert!(result.contains("VITE_AB_REDIRECT_URI=http://localhost:8080"));
        assert!(result.contains("VITE_AB_NAMESPACE=production"));

        // Unrelated keys and comments are preserved.
        assert!(result.contains("CUSTOM_PORT=3000"));
        assert!(result.contains("# App UI Environment"));

        // Old values are gone.
        assert!(!result.contains("old_client"));
        assert!(!result.contains("https://old.example.com"));

        // Trailing newline.
        assert!(result.ends_with('\n'));
    }

    #[test]
    fn test_no_input_does_not_hang() {
        // The command never prompts for input: namespace is validated
        // up-front and all interactive confirmation goes through the
        // fast-exit guard. A missing namespace yields a Usage error
        // immediately, never a prompt.
        let err = validate_required_namespace(None);
        assert!(
            matches!(err, Err(CliError::Usage { .. })),
            "missing namespace must yield Usage error, not a prompt"
        );

        // A present namespace passes validation without prompting.
        let ok = validate_required_namespace(Some("test-ns"));
        assert_eq!(ok.unwrap(), "test-ns");
    }
}
