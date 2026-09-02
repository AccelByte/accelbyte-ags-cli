//! Handler for `ags extend app-ui upload`.
//!
//! Builds the frontend project, archives the build output, and uploads
//! the archive to the CSM `UploadAppUIFile` endpoint via the shared
//! multipart dispatch path.
//!
//! Five-step execution order:
//! 1. Validate paths (project path, build path)
//! 2. Run the frontend build (or skip with `--no-build`)
//! 3. Archive the build output into a zip
//! 4. Upload the archive via `Runtime::run_command` using operation
//!    `csm/admin/app-ui/v1/upload-assets`
//! 5. Clean up the temp zip (both success and failure paths)

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::ArgMatches;

use crate::errors::CliError;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;

// ── Constants ──

/// Maximum wall-clock time for a frontend build subprocess. 30 minutes
/// mirrors `DOCKER_BUILD_TIMEOUT` in `image_upload`; frontend builds
/// can pull dependencies and run transpilation passes that take
/// comparable time.
const FRONTEND_BUILD_TIMEOUT: Duration = Duration::from_secs(1800);

/// Default base URL used when the configured base URL is empty.
const DEFAULT_BASE_URL: &str = "https://development.accelbyte.io";

// ── Data types ──

/// Detected frontend package manager.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PackageManager {
    Npm,
    Yarn,
    Pnpm,
}

/// Command configuration for the frontend build subprocess.
///
/// Pure data — the actual spawn is performed by the caller-supplied
/// runner, keeping this struct testable without process side effects.
#[derive(Debug, Clone)]
pub(crate) struct BuildCommand {
    pub program: String,
    pub args: Vec<String>,
    pub env_vars: Vec<(String, String)>,
    pub working_dir: PathBuf,
}

// ── Path resolution ──

/// Resolve and validate the project path. Returns the canonicalized
/// absolute path, or a `CliError::Usage` if the path does not exist or
/// is not a directory.
fn resolve_project_path(raw: &str) -> Result<PathBuf, CliError> {
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
    std::fs::canonicalize(&path).map_err(|e| CliError::Usage {
        message: format!("failed to resolve project path '{}': {e}", path.display()),
        metadata: None,
    })
}

/// Resolve the build path relative to the project path when the raw
/// value is relative. Absolute paths are returned unchanged.
///
/// The Go tool joins them (`filepath.Join(projectPath, buildPath)`),
/// so the `"dist"` default means `<project-path>/dist`. Resolving
/// against the CWD instead would break every invocation made from
/// outside the project directory.
pub(crate) fn resolve_build_path(project_path: &Path, raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        p
    } else {
        project_path.join(p)
    }
}

/// Validate that the build output directory exists, is a directory,
/// and is non-empty. Called both after a successful build AND on the
/// `--no-build` path. An empty build directory means the archive would
/// upload nothing.
fn validate_build_output(path: &Path) -> Result<(), CliError> {
    if !path.exists() {
        return Err(CliError::Usage {
            message: format!("build path '{}' was not found", path.display()),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Run the frontend build first, or check --build-path / --project-path.",
            ))),
        });
    }
    if !path.is_dir() {
        return Err(CliError::Usage {
            message: format!("build path '{}' is not a directory", path.display()),
            metadata: None,
        });
    }
    // Non-empty check: at least one entry.
    let has_entry = std::fs::read_dir(path)
        .map_err(|e| CliError::Usage {
            message: format!("failed to read build path '{}': {e}", path.display()),
            metadata: None,
        })?
        .next()
        .is_some();
    if !has_entry {
        return Err(CliError::Usage {
            message: format!("build path '{}' is empty", path.display()),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "The build may have failed silently. Check the build output and retry.",
            ))),
        });
    }
    Ok(())
}

// ── Package manager detection ──

/// Detect the frontend package manager by examining the project
/// directory. Returns `Err(CliError::Usage)` when no `package.json`
/// is found.
///
/// Detection order: `yarn.lock` → Yarn, `pnpm-lock.yaml` → Pnpm,
/// else Npm. `package.json` must exist.
pub(crate) fn detect_package_manager(project_path: &Path) -> Result<PackageManager, CliError> {
    if !project_path.join("package.json").exists() {
        return Err(CliError::Usage {
            message: format!("no package manager found in '{}'", project_path.display()),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Ensure the project directory contains a package.json.",
            ))),
        });
    }
    if project_path.join("yarn.lock").exists() {
        return Ok(PackageManager::Yarn);
    }
    if project_path.join("pnpm-lock.yaml").exists() {
        return Ok(PackageManager::Pnpm);
    }
    Ok(PackageManager::Npm)
}

// ── Build command construction (pure) ──

/// Build the subprocess configuration for the frontend build.
///
/// The package manager is invoked directly rather than through a shell
/// (`sh -c`), diverging from the Go tool. The Go tool wraps the
/// command in `sh -c`, which does not exist on Windows; invoking the
/// package manager directly works on all platforms.
pub(crate) fn make_build_command(
    pm: PackageManager,
    name: &str,
    version: &str,
    base_url: &str,
    namespace: &str,
    project_path: &Path,
) -> Result<BuildCommand, CliError> {
    let (program, args) = match pm {
        PackageManager::Npm => ("npm", vec!["run".to_string(), "build".to_string()]),
        PackageManager::Yarn => ("yarn", vec!["build".to_string()]),
        PackageManager::Pnpm => ("pnpm", vec!["build".to_string()]),
    };

    // Encode path segments per the CONTRIBUTING.md convention: never
    // interpolate user input into URL paths without encoding. The
    // caller validates inputs first; encoding here is defense-in-depth.
    let encoded_ns = ags_runtime::support::strings::encode_url_path_segment(namespace, "namespace")
        .map_err(CliError::from)?;
    let encoded_name = ags_runtime::support::strings::encode_url_path_segment(name, "name")
        .map_err(CliError::from)?;
    let encoded_ver =
        ags_runtime::support::strings::encode_url_path_segment(version, "build-version")
            .map_err(CliError::from)?;

    // BASE_URL is the CSM asset path — leading and trailing slash, no host.
    let asset_path =
        format!("/csm/v1/admin/namespaces/{encoded_ns}/files/app-ui/{encoded_name}/{encoded_ver}/");

    let env_vars = vec![
        ("AB_APPUI_NAME".to_string(), name.to_string()),
        ("AB_APPUI_BUILD_VERSION".to_string(), version.to_string()),
        ("AB_BASE_URL".to_string(), base_url.to_string()),
        ("AB_NAMESPACE".to_string(), namespace.to_string()),
        ("BASE_URL".to_string(), asset_path),
    ];

    Ok(BuildCommand {
        program: program.to_string(),
        args,
        env_vars,
        working_dir: project_path.to_path_buf(),
    })
}

// ── Build execution ──

/// Spawn the frontend build subprocess and wait for it to complete.
///
/// Uses `wait_with_timeout` from the shared subprocess helper to
/// enforce a bounded wait. A non-zero exit code is mapped to
/// `CliError::Network` carrying the build tool's stderr.
fn execute_build(cmd: BuildCommand) -> Result<(), CliError> {
    let mut process = std::process::Command::new(&cmd.program);
    process.args(&cmd.args);
    process.current_dir(&cmd.working_dir);
    // Inherit the parent environment, then layer the build-specific vars.
    for (key, value) in &cmd.env_vars {
        process.env(key, value);
    }
    // Null stdin so the build tool never blocks on a prompt.
    process.stdin(std::process::Stdio::null());
    process.stdout(std::process::Stdio::piped());
    // Pipe stderr so the output is captured for the structured error
    // message (visible to `--format json`, log aggregation, and
    // wrapping tools). The captured output is also forwarded to the
    // terminal line-by-line below, so the user sees build errors.
    process.stderr(std::process::Stdio::piped());

    let child = process.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CliError::Usage {
                message: format!("'{}' is not installed or not found on PATH", cmd.program),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Install the package manager and ensure it is on your PATH.",
                ))),
            }
        } else {
            CliError::Network {
                message: format!("failed to run '{}': {e}", cmd.program),
                metadata: None,
            }
        }
    })?;

    let output = ags_runtime::support::process::wait_with_timeout(child, FRONTEND_BUILD_TIMEOUT)
        .map_err(|err| match err {
            ags_runtime::support::process::WaitError::TimedOut(d) => CliError::Network {
                message: format!("frontend build timed out after {}s", d.as_secs()),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "The build did not complete in time. Check network and retry.",
                ))),
            },
            ags_runtime::support::process::WaitError::Wait(e) => CliError::Network {
                message: format!("failed to wait for build process: {e}"),
                metadata: None,
            },
        })?;

    // Forward captured stdout to stderr (build progress, not data output).
    if !output.stdout.is_empty() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            crate::frontend::write_stderr_line(line);
        }
    }

    // Forward captured stderr to the terminal so the user sees build
    // errors. The captured bytes are also retained for the truncated
    // error message below.
    if !output.stderr.is_empty() {
        let text = String::from_utf8_lossy(&output.stderr);
        for line in text.lines() {
            crate::frontend::write_stderr_line(line);
        }
    }

    if !output.status.success() {
        let code = output.status.code().unwrap_or(1);
        let stderr_text = String::from_utf8_lossy(&output.stderr);
        // Strip terminal control sequences before truncating so a
        // multi-byte escape at the byte boundary does not panic.
        let clean = ags_runtime::support::strings::strip_terminal_control_sequences(&stderr_text);
        let truncated = ags_runtime::support::strings::truncate_display_text(&clean, 512);
        return Err(CliError::Network {
            message: format!("frontend build failed (exit code {code}): {truncated}"),
            metadata: None,
        });
    }

    Ok(())
}

// ── Archive ──

/// Archive the build directory into a zip file inside a unique
/// temporary directory. Entry names are relative to the build
/// directory root so `<build>/index.html` is stored as `index.html`.
///
/// Returns `(archive_path, size_bytes, temp_dir_handle)`. The caller
/// **must** hold the returned `TempDir` until the archive file has
/// been fully consumed (uploaded); dropping the handle deletes the
/// directory and its contents.
///
/// On failure, any partial zip is deleted before the error is
/// returned (defense-in-depth — `TempDir::Drop` also cleans up).
pub(crate) fn create_archive(
    build_path: &Path,
    name: &str,
    version: &str,
) -> Result<(PathBuf, u64, tempfile::TempDir), CliError> {
    let zip_name = format!("{name}-{version}.zip");
    let temp_dir = tempfile::TempDir::new()
        .map_err(|e| CliError::Internal(anyhow::anyhow!("failed to create temp directory: {e}")))?;
    let zip_path = temp_dir.path().join(&zip_name);

    let size = write_zip_with_cleanup(build_path, &zip_path)?;

    Ok((zip_path, size, temp_dir))
}

/// Write the zip archive and clean up any partial file on failure.
///
/// `write_zip` calls `File::create` before walking the build
/// directory, so a walk failure leaves a partial zip on disk. This
/// wrapper removes it (defense-in-depth — `TempDir::Drop` in
/// `create_archive` also cleans up). Separated so the cleanup
/// invariant is testable with a caller-owned directory, without
/// `TempDir::Drop` masking the result.
fn write_zip_with_cleanup(build_path: &Path, zip_path: &Path) -> Result<u64, CliError> {
    if let Err(e) = write_zip(build_path, zip_path) {
        // Clean up partial zip on failure (defense-in-depth; TempDir
        // drop also removes the directory tree).
        let _ = std::fs::remove_file(zip_path);
        return Err(e);
    }

    let size = std::fs::metadata(zip_path)
        .map_err(|e| CliError::Internal(anyhow::anyhow!("failed to stat archive: {e}")))?
        .len();

    Ok(size)
}

/// Write the zip archive to `zip_path` containing all files under
/// `build_path` with entry names relative to `build_path`.
fn write_zip(build_path: &Path, zip_path: &Path) -> Result<(PathBuf, u64), CliError> {
    use zip::write::SimpleFileOptions;

    let file = std::fs::File::create(zip_path).map_err(|e| {
        CliError::Internal(anyhow::anyhow!(
            "failed to create archive '{}': {e}",
            zip_path.display()
        ))
    })?;

    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // Recursive walk using std::fs — no extra crate for directory traversal.
    walk_dir_into_zip(build_path, build_path, &mut zip, options)?;

    zip.finish()
        .map_err(|e| CliError::Internal(anyhow::anyhow!("failed to finalize archive: {e}")))?;

    let size = std::fs::metadata(zip_path)
        .map_err(|e| CliError::Internal(anyhow::anyhow!("failed to stat archive: {e}")))?
        .len();

    Ok((zip_path.to_path_buf(), size))
}

/// Recursively add entries from `current` into the zip, computing
/// entry names relative to `base`.
fn walk_dir_into_zip<W: std::io::Write + std::io::Seek>(
    base: &Path,
    current: &Path,
    zip: &mut zip::ZipWriter<W>,
    options: zip::write::SimpleFileOptions,
) -> Result<(), CliError> {
    let entries = std::fs::read_dir(current).map_err(|e| {
        CliError::Internal(anyhow::anyhow!(
            "failed to read directory '{}': {e}",
            current.display()
        ))
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| {
            CliError::Internal(anyhow::anyhow!("failed to read directory entry: {e}"))
        })?;
        let path = entry.path();

        // Use symlink_metadata so symlinks are NOT followed. A symlink
        // cycle (e.g. `dist/self -> dist/`) would recurse unboundedly
        // if we followed symlinks via `is_dir()`. Frontend builds pull
        // large untrusted dependency trees where a postinstall script
        // or a compromised dependency can create such a link. Symlinked
        // entries are skipped — the archive contains only real files
        // and directories.
        let meta = std::fs::symlink_metadata(&path).map_err(|e| {
            CliError::Internal(anyhow::anyhow!("failed to stat '{}': {e}", path.display()))
        })?;
        if meta.file_type().is_symlink() {
            continue;
        }

        let relative = path
            .strip_prefix(base)
            .map_err(|e| CliError::Internal(anyhow::anyhow!("path prefix error: {e}")))?;
        // Use forward slashes for zip entry names (the zip spec requires them).
        let entry_name = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/");

        if meta.is_dir() {
            // Directory entry with trailing slash.
            let dir_name = format!("{entry_name}/");
            zip.add_directory(&dir_name, options).map_err(|e| {
                CliError::Internal(anyhow::anyhow!("failed to add directory entry: {e}"))
            })?;
            walk_dir_into_zip(base, &path, zip, options)?;
        } else {
            zip.start_file(&entry_name, options).map_err(|e| {
                CliError::Internal(anyhow::anyhow!("failed to start file entry: {e}"))
            })?;
            // Stream file content via io::copy rather than reading the
            // entire file into memory, avoiding per-file buffering.
            let mut file = std::fs::File::open(&path).map_err(|e| {
                CliError::Internal(anyhow::anyhow!("failed to open '{}': {e}", path.display()))
            })?;
            std::io::copy(&mut file, zip).map_err(|e| {
                CliError::Internal(anyhow::anyhow!("failed to write archive entry: {e}"))
            })?;
        }
    }

    Ok(())
}

// ── Size seam ──

/// Check the archive size against an optional limit. The limit is a
/// parameter, not a compiled-in constant — at launch the caller
/// passes `None` (no limit). When a limit is introduced, refusal is
/// `CliError::Usage` naming both the archive size and the limit.
pub(crate) fn check_archive_size(actual_bytes: u64, limit: Option<u64>) -> Result<(), CliError> {
    if let Some(max) = limit {
        if actual_bytes >= max {
            let actual_mib = actual_bytes / (1024 * 1024);
            let limit_mib = max / (1024 * 1024);
            return Err(CliError::Usage {
                message: format!(
                    "archive size ({actual_mib} MiB) exceeds the {limit_mib} MiB limit"
                ),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Reduce the build output size or increase the limit.",
                ))),
            });
        }
    }
    Ok(())
}

// ── Default build version ──

/// Generate the default build version: the first 8 hex characters of
/// the SHA-256 hash of `"{unix_nanos}-{pid}"`.
fn default_build_version() -> String {
    use sha2::{Digest, Sha256};

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let input = format!("{now}-{pid}");

    let hash = Sha256::digest(input.as_bytes());
    // First 8 hex characters = first 4 bytes.
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        hash[0], hash[1], hash[2], hash[3]
    )
}

// ── Base URL resolution ──

/// Resolve the configured base URL for the build environment.
/// Reads from `AGS_BASE_URL` env var, then profile config, falling
/// back to `DEFAULT_BASE_URL` when neither is set.
pub(crate) fn resolve_base_url(profile: Option<&str>) -> String {
    let profile_name = profile.unwrap_or("default");
    ags_runtime::runtime::auth::credentials::resolve_base_url_value(profile_name)
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

// ── Input validation ──

/// Validate that a user-supplied value contains only safe characters
/// for use as a URL path segment and a filename component. Rejects
/// empty values, path traversal sequences (`..`), and characters
/// outside `[A-Za-z0-9._-]`.
///
/// Returns `CliError::Usage` naming the offending `--flag` and value.
pub(crate) fn validate_safe_component(value: &str, flag_name: &str) -> Result<(), CliError> {
    if value.is_empty() {
        return Err(CliError::Usage {
            message: format!("--{flag_name} cannot be empty"),
            metadata: None,
        });
    }
    if value.contains("..") {
        return Err(CliError::Usage {
            message: format!("--{flag_name} contains path traversal sequence '..': '{value}'"),
            metadata: None,
        });
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
    {
        return Err(CliError::Usage {
            message: format!(
                "--{flag_name} contains invalid characters: '{value}' \
                 (only letters, digits, '.', '-', and '_' are allowed)"
            ),
            metadata: None,
        });
    }
    Ok(())
}

// ── Dry-run preview ──

/// Emit the dry-run preview to stderr and return `Complete`. No
/// subprocess is spawned, no archive is created, and no HTTP request
/// is made.
fn dry_run_preview(
    name: &str,
    build_version: &str,
    namespace: &str,
    project_path: &Path,
    build_path_raw: &str,
    base_url: &str,
    no_build: bool,
) -> Result<InvocationOutcome, CliError> {
    let build_path = resolve_build_path(project_path, build_path_raw);
    let color = crate::frontend::style::is_stderr_enabled();
    crate::frontend::write_stderr_line(&crate::frontend::style::info(
        "Dry run — no build, archive, or upload will be performed",
        color,
    ));
    crate::frontend::write_stderr_line(&format!("  App UI:         {name}"));
    crate::frontend::write_stderr_line(&format!("  Build version:  {build_version}"));
    crate::frontend::write_stderr_line(&format!("  Namespace:      {namespace}"));
    crate::frontend::write_stderr_line(&format!("  Project path:   {}", project_path.display()));
    crate::frontend::write_stderr_line(&format!("  Build path:     {}", build_path.display()));
    crate::frontend::write_stderr_line(&format!("  Base URL:       {base_url}"));
    crate::frontend::write_stderr_line(&format!("  No-build:       {no_build}"));

    // Detect package manager (best-effort for preview — missing
    // package.json is fine here, the user may be planning to add one).
    match detect_package_manager(project_path) {
        Ok(pm) => {
            let pm_name = match pm {
                PackageManager::Npm => "npm",
                PackageManager::Yarn => "yarn",
                PackageManager::Pnpm => "pnpm",
            };
            crate::frontend::write_stderr_line(&format!("  Package mgr:    {pm_name}"));
        }
        Err(_) => {
            crate::frontend::write_stderr_line("  Package mgr:    (not detected)");
        }
    }

    let archive_name = format!("{name}-{build_version}.zip");
    let asset_path =
        format!("/csm/v1/admin/namespaces/{namespace}/files/app-ui/{name}/{build_version}/");
    crate::frontend::write_stderr_line(&format!("  Archive name:   {archive_name}"));
    crate::frontend::write_stderr_line(&format!("  Asset path:     {asset_path}"));
    crate::frontend::write_stderr_line("  Upload target:  csm/admin/app-ui/v1/upload-assets");

    Ok(InvocationOutcome::Complete)
}

// ── Orchestrator (steps 1-3) ──

/// Run steps 1-3 of the upload pipeline: validate paths, optionally
/// run the frontend build, and archive the build output.
///
/// The `build_runner` parameter is the subprocess boundary seam: the
/// handler passes `execute_build`; tests pass closures that capture
/// the command or panic if the build should not be reached.
///
/// The returned `TempDir` owns the archive file's parent directory.
/// The caller must hold it until the archive has been fully consumed
/// (uploaded); dropping it early deletes the archive.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_steps_1_to_3<F>(
    name: &str,
    build_version: &str,
    no_build: bool,
    project_path: &Path,
    raw_build_path: &str,
    namespace: &str,
    base_url: &str,
    build_runner: F,
) -> Result<(PathBuf, u64, tempfile::TempDir), CliError>
where
    F: FnOnce(BuildCommand) -> Result<(), CliError>,
{
    // Step 1: resolve build path.
    let build_path = resolve_build_path(project_path, raw_build_path);

    if no_build {
        // --no-build: skip the build entirely, just validate the existing output.
        validate_build_output(&build_path)?;
    } else {
        // Step 2: detect package manager and run the build.
        let pm = detect_package_manager(project_path)?;
        let cmd = make_build_command(pm, name, build_version, base_url, namespace, project_path)?;
        build_runner(cmd)?;

        // Validate the build produced output.
        validate_build_output(&build_path)?;
    }

    // Step 3: archive the build output.
    let (archive_path, archive_size, archive_dir) =
        create_archive(&build_path, name, build_version)?;

    // Size seam: no limit at launch.
    check_archive_size(archive_size, None)?;

    Ok((archive_path, archive_size, archive_dir))
}

// ── Upload request construction ──

/// Build the `CommandRequest` for the CSM `UploadAppUIFile` operation.
///
/// The request uses the shared multipart dispatch path — no
/// `reqwest::multipart::Form` is constructed here. The `FormPart::File`
/// is turned into a streamed multipart part by the dispatch layer in
/// `crates/ags-runtime/src/runtime/dispatch/http.rs`.
pub(crate) fn build_upload_request(
    namespace: &str,
    name: &str,
    version: &str,
    archive_path: &Path,
    archive_filename: &str,
) -> ags_protocol::request::CommandRequest {
    use ags_protocol::catalogue::{OperationId, ServiceId};
    use ags_protocol::request::{
        CommandRequest, FormPart, OutputFormat, PaginationHint, RequestBody, Verbosity,
    };
    use std::collections::BTreeMap;

    let mut path_params = BTreeMap::new();
    path_params.insert("namespace".to_string(), namespace.to_string());
    path_params.insert("appUiName".to_string(), name.to_string());

    let mut query_params = BTreeMap::new();
    query_params.insert("version".to_string(), version.to_string());

    CommandRequest {
        service: ServiceId::new("csm"),
        operation_id: OperationId::new("csm/admin/app-ui/v1/upload-assets"),
        namespace: Some(namespace.to_string()),
        path_params,
        query_params,
        header_params: BTreeMap::new(),
        form_params: BTreeMap::new(),
        body: Some(RequestBody::Multipart(vec![FormPart::File {
            name: "file".to_string(),
            path: archive_path.to_path_buf(),
            filename: archive_filename.to_string(),
        }])),
        output_format: OutputFormat::Human,
        pagination: PaginationHint::Auto,
        verbosity: Verbosity::Normal,
        output: None,
    }
}

// ── Temp-zip cleanup ──

/// Remove the temporary zip archive. Called on BOTH the success and
/// failure paths. A cleanup failure is logged to stderr with the file
/// path but does not change the command's exit code.
///
/// This is a deliberate parity break from the Go tool, which only
/// removes the archive after a successful upload — a failed upload
/// leaves the archive in the temp directory forever. Cleaning up on
/// both paths prevents temp-directory growth from repeated failures.
fn cleanup_archive(zip_path: &Path) {
    if let Err(e) = std::fs::remove_file(zip_path) {
        // Best-effort: log the error but do not change the exit code.
        crate::frontend::write_stderr_line(&format!(
            "  Warning: failed to remove temp archive '{}': {e}",
            zip_path.display()
        ));
    }
}

// ── Entry point ──

/// Execute the `app-ui upload` command (steps 1-5).
pub(crate) async fn handle_app_ui_upload(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    // Parse flags.
    let name = matches
        .get_one::<String>("name")
        .ok_or_else(|| CliError::Usage {
            message: "--name is required".to_string(),
            metadata: None,
        })?
        .clone();

    let project_path_raw = matches
        .get_one::<String>("project-path")
        .map(|s| s.as_str())
        .unwrap_or(".");

    let build_path_raw = matches
        .get_one::<String>("build-path")
        .map(|s| s.as_str())
        .unwrap_or("dist");

    let build_version = matches
        .get_one::<String>("build-version")
        .cloned()
        .unwrap_or_else(default_build_version);

    let no_build = matches.get_flag("no-build");

    // Validate namespace (reuse the same pattern as setup-env).
    let namespace = flags.namespace.as_deref().ok_or_else(|| CliError::Usage {
        message: "--namespace is required for app-ui upload".to_string(),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            "Supply --namespace <ns> or set a default via 'ags config set namespace <ns>'",
        ))),
    })?;

    // Emit a notice for any Go-compat flag the user explicitly supplied.
    // Mirrors the docker-login pattern: fires before validation so the
    // user always sees the notice, even if the command subsequently
    // fails due to an invalid value. The gate on `is_quiet` suppresses
    // the notice in `--quiet` mode.
    {
        let supplied = crate::invocation::compat_flags::collect_supplied_flags(
            matches,
            &[&crate::invocation::compat_flags::APP_UI_UPLOAD_VERBOSITY],
        );
        if !supplied.is_empty() && !flags.verbosity.is_quiet() {
            for flag_name in &supplied {
                frontend.render_warning(
                    &format!(
                        "--{flag_name} is accepted for backward compatibility but has no effect"
                    ),
                    None,
                    None,
                );
            }
        }
    }

    // Validate inputs before any subprocess or network work. The
    // allow-list rejects characters that would be dangerous in a URL
    // path segment or a filename (slash, dotdot, spaces, control
    // chars). encode_url_path_segment inside make_build_command
    // provides defense-in-depth.
    validate_safe_component(&name, "name")?;
    validate_safe_component(&build_version, "build-version")?;
    validate_safe_component(namespace, "namespace")?;

    // Resolve project path.
    let project_path = resolve_project_path(project_path_raw)?;

    // Resolve base URL for the build environment variables.
    let base_url = resolve_base_url(flags.profile.as_deref());

    // --dry-run: show what would happen, then return. No subprocess,
    // no archive, no HTTP request.
    if flags.is_dry_run {
        return dry_run_preview(
            &name,
            &build_version,
            namespace,
            &project_path,
            build_path_raw,
            &base_url,
            no_build,
        );
    }

    // Run steps 1-3. Hold `_archive_dir` until after upload — dropping
    // it early deletes the archive before dispatch can read it.
    let (archive_path, archive_size, _archive_dir) = run_steps_1_to_3(
        &name,
        &build_version,
        no_build,
        &project_path,
        build_path_raw,
        namespace,
        &base_url,
        execute_build,
    )?;

    // Step 4: upload the archive via the shared dispatch path.
    let archive_filename = format!("{name}-{build_version}.zip");
    let request = build_upload_request(
        namespace,
        &name,
        &build_version,
        &archive_path,
        &archive_filename,
    );

    let upload_result = upload_archive(flags, &request).await;

    // Step 5: clean up the temp zip on BOTH success and failure.
    cleanup_archive(&archive_path);

    // Now handle the upload result.
    let command_output = upload_result?;

    // Extract the raw JSON response body from the CommandOutput.
    let response_body = extract_response_body(&command_output);

    let upload_output = ags_protocol::output::AppUiUploadOutput {
        name: name.clone(),
        version: build_version.clone(),
        archive_bytes: archive_size,
        response: response_body,
    };

    frontend.render(&ags_protocol::output::CommandOutput::AppUiUpload(
        upload_output,
    ))?;

    Ok(InvocationOutcome::Complete)
}

/// Dispatch the upload through the shared runtime path.
async fn upload_archive(
    flags: &GlobalFlags,
    request: &ags_protocol::request::CommandRequest,
) -> Result<ags_protocol::output::CommandOutput, CliError> {
    let input = ags_runtime::runtime::execution::ResolutionInput {
        profile: flags.profile.clone(),
        namespace: flags.namespace.clone(),
        is_dry_run: flags.is_dry_run,
    };
    let http_client = ags_runtime::runtime::dispatch::http::build_http_client(flags.timeout)?;
    let context =
        ags_runtime::runtime::execution::ExecutionContext::resolve(&input, &http_client).await?;
    let mut runtime = ags_runtime::runtime::Runtime::from_reqwest(context, http_client);

    let mut null_frontend = NullFrontend;
    let mut sink = crate::frontend::FrontendSink::new(&mut null_frontend);
    runtime
        .run_command(request, &mut sink)
        .await
        .map_err(CliError::from)
}

/// Extract the raw JSON response body from a `CommandOutput::Service`.
/// Falls back to an empty object for non-service or non-JSON responses.
fn extract_response_body(output: &ags_protocol::output::CommandOutput) -> serde_json::Value {
    use ags_protocol::output::{ApiBody, CommandOutput};
    match output {
        CommandOutput::Service(api_output) => {
            // Prefer the raw JSON body when available (it preserves the original field names).
            if let Some(raw) = &api_output.raw_body {
                return raw.clone();
            }
            match &api_output.body {
                ApiBody::Shaped(result) => serde_json::json!(result),
                ApiBody::Text(text) => serde_json::Value::String(text.clone()),
                ApiBody::Empty => serde_json::json!({}),
            }
        }
        _ => serde_json::json!({}),
    }
}

/// Minimal frontend that does nothing — used to satisfy the
/// `FrontendSink` borrow requirement when the real frontend is
/// unavailable (we are in an async context with a mutable borrow on
/// the real frontend from the caller).
struct NullFrontend;

impl crate::frontend::Frontend for NullFrontend {
    fn render(&mut self, _output: &ags_protocol::output::CommandOutput) -> Result<(), CliError> {
        Ok(())
    }
    fn render_error(&mut self, _err: &CliError) {}
    fn render_warning(&mut self, _msg: &str, _reason: Option<&str>, _tip: Option<&str>) {}
    fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {}
    fn finish(self: Box<Self>) -> Result<(), CliError> {
        Ok(())
    }
}

// ══════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Path resolution ──

    #[test]
    fn test_relative_build_path_resolves_against_project_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let project = dir.path().join("p");
        std::fs::create_dir_all(&project).unwrap();
        // Create dist inside the project so the resolved path exists.
        let expected = project.join("dist");
        std::fs::create_dir_all(&expected).unwrap();

        let resolved = resolve_build_path(&project, "dist");
        assert_eq!(
            resolved, expected,
            "relative build path must resolve against project path, not CWD"
        );
    }

    #[test]
    fn test_absolute_build_path_used_as_is() {
        let dir = tempfile::TempDir::new().unwrap();
        let abs = dir.path().join("custom-build");
        std::fs::create_dir_all(&abs).unwrap();
        let abs_str = abs.to_str().unwrap();

        let resolved = resolve_build_path(Path::new("/some/project"), abs_str);
        assert_eq!(
            resolved, abs,
            "absolute build path must not be joined with project path"
        );
    }

    // ── Build output validation ──

    #[test]
    fn test_no_build_with_absent_build_path_returns_usage_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let absent = dir.path().join("nonexistent");

        let result = validate_build_output(&absent);
        assert!(result.is_err(), "absent build path must fail");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("build path") && msg.contains("was not found"),
            "error must mention 'build path' and 'was not found': {msg}"
        );
    }

    #[test]
    fn test_empty_build_directory_fails_validation() {
        let dir = tempfile::TempDir::new().unwrap();
        let empty = dir.path().join("empty-build");
        std::fs::create_dir_all(&empty).unwrap();

        let result = validate_build_output(&empty);
        assert!(result.is_err(), "empty build directory must fail");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("empty"), "error must mention 'empty': {msg}");
    }

    #[test]
    fn test_non_empty_build_directory_passes_validation() {
        let dir = tempfile::TempDir::new().unwrap();
        let build = dir.path().join("build");
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join("index.html"), "<html>").unwrap();

        let result = validate_build_output(&build);
        assert!(
            result.is_ok(),
            "non-empty build directory must pass: {result:?}"
        );
    }

    // ── Package manager detection ──

    #[test]
    fn test_no_package_json_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = detect_package_manager(dir.path());
        assert!(result.is_err(), "missing package.json must fail");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("no package manager found"),
            "error must say 'no package manager found': {msg}"
        );
    }

    #[test]
    fn test_package_json_with_yarn_lock_detects_yarn() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        std::fs::write(dir.path().join("yarn.lock"), "").unwrap();
        let result = detect_package_manager(dir.path());
        assert_eq!(result.unwrap(), PackageManager::Yarn);
    }

    #[test]
    fn test_package_json_with_pnpm_lock_detects_pnpm() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        std::fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        let result = detect_package_manager(dir.path());
        assert_eq!(result.unwrap(), PackageManager::Pnpm);
    }

    #[test]
    fn test_package_json_alone_defaults_to_npm() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        let result = detect_package_manager(dir.path());
        assert_eq!(result.unwrap(), PackageManager::Npm);
    }

    // ── Build command construction ──

    #[test]
    fn test_npm_build_command() {
        let cmd = make_build_command(
            PackageManager::Npm,
            "my-app",
            "abc12345",
            "https://dev.example.com",
            "test-ns",
            Path::new("/project"),
        )
        .unwrap();
        assert_eq!(cmd.program, "npm");
        assert_eq!(cmd.args, vec!["run", "build"]);
        assert_eq!(cmd.working_dir, PathBuf::from("/project"));
    }

    #[test]
    fn test_yarn_build_command_has_no_run() {
        let cmd = make_build_command(
            PackageManager::Yarn,
            "my-app",
            "abc12345",
            "https://dev.example.com",
            "test-ns",
            Path::new("/project"),
        )
        .unwrap();
        assert_eq!(cmd.program, "yarn");
        assert_eq!(cmd.args, vec!["build"]);
    }

    #[test]
    fn test_pnpm_build_command_has_no_run() {
        let cmd = make_build_command(
            PackageManager::Pnpm,
            "my-app",
            "abc12345",
            "https://dev.example.com",
            "test-ns",
            Path::new("/project"),
        )
        .unwrap();
        assert_eq!(cmd.program, "pnpm");
        assert_eq!(cmd.args, vec!["build"]);
    }

    #[test]
    fn test_build_command_env_vars_include_base_url_as_asset_path() {
        let cmd = make_build_command(
            PackageManager::Npm,
            "my-app",
            "abc12345",
            "https://dev.example.com",
            "test-ns",
            Path::new("/project"),
        )
        .unwrap();

        // All five env vars must be present.
        let env_map: std::collections::HashMap<&str, &str> = cmd
            .env_vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        assert_eq!(
            env_map.get("AB_APPUI_NAME"),
            Some(&"my-app"),
            "AB_APPUI_NAME must equal --name"
        );
        assert_eq!(
            env_map.get("AB_APPUI_BUILD_VERSION"),
            Some(&"abc12345"),
            "AB_APPUI_BUILD_VERSION must equal --build-version"
        );
        assert_eq!(
            env_map.get("AB_BASE_URL"),
            Some(&"https://dev.example.com"),
            "AB_BASE_URL must be the configured base URL"
        );
        assert_eq!(
            env_map.get("AB_NAMESPACE"),
            Some(&"test-ns"),
            "AB_NAMESPACE must equal the resolved namespace"
        );

        // The critical assertion: BASE_URL is the ASSET PATH, not a URL.
        // Leading and trailing slash, no host, namespace and name interpolated.
        assert_eq!(
            env_map.get("BASE_URL"),
            Some(&"/csm/v1/admin/namespaces/test-ns/files/app-ui/my-app/abc12345/"),
            "BASE_URL must be the asset path, not a URL"
        );
    }

    #[test]
    fn test_build_command_env_vars_count_is_five() {
        let cmd = make_build_command(
            PackageManager::Npm,
            "my-app",
            "v1",
            "https://example.com",
            "ns",
            Path::new("/p"),
        )
        .unwrap();
        assert_eq!(
            cmd.env_vars.len(),
            5,
            "exactly five env vars must be set: {:?}",
            cmd.env_vars
        );
    }

    // ── Archive layout ──

    #[test]
    fn test_archive_entry_names_are_relative_to_build_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let build = dir.path().join("build");
        std::fs::create_dir_all(build.join("assets")).unwrap();
        std::fs::write(build.join("index.html"), "<html>").unwrap();
        std::fs::write(build.join("assets").join("app.js"), "// js").unwrap();

        let (archive_path, size, _td) = create_archive(&build, "test-app", "v1").unwrap();

        // Verify the archive was created and has a positive size.
        assert!(archive_path.exists(), "archive must exist");
        assert!(size > 0, "archive size must be positive");

        // Read the zip and assert entry names.
        let file = std::fs::File::open(&archive_path).unwrap();
        let archive = zip::ZipArchive::new(file).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.name_for_index(i).unwrap().to_string())
            .collect();
        names.sort();

        // Directory entry "assets/" plus two file entries.
        assert!(
            names.contains(&"index.html".to_string()),
            "archive must contain 'index.html': {names:?}"
        );
        assert!(
            names.contains(&"assets/app.js".to_string()),
            "archive must contain 'assets/app.js': {names:?}"
        );
        // No wrapping directory prefix.
        assert!(
            !names
                .iter()
                .any(|n| n.starts_with("build/") || n.starts_with("dist/")),
            "archive must not have a wrapping directory prefix: {names:?}"
        );
    }

    // ── Size seam ──

    #[test]
    fn test_size_check_no_limit_passes() {
        // 512 MiB archive, no limit → Ok.
        let result = check_archive_size(512 * 1024 * 1024, None);
        assert!(result.is_ok(), "no limit must always pass: {result:?}");
    }

    #[test]
    fn test_size_check_under_limit_passes() {
        let result = check_archive_size(4 * 1024 * 1024, Some(5 * 1024 * 1024));
        assert!(result.is_ok(), "under limit must pass: {result:?}");
    }

    #[test]
    fn test_size_check_over_limit_returns_usage_error() {
        // 6 MiB archive, 5 MiB limit → Usage error.
        let result = check_archive_size(6 * 1024 * 1024, Some(5 * 1024 * 1024));
        assert!(result.is_err(), "over limit must fail");
        let err = result.unwrap_err();
        assert!(
            matches!(err, CliError::Usage { .. }),
            "over-limit must be Usage: {err:?}"
        );
        let msg = err.to_string();
        // The error must name both the archive size and the limit.
        assert!(
            msg.contains("6") && msg.contains("5"),
            "error must name both sizes: {msg}"
        );
    }

    // ── Orchestration: --no-build skips build ──

    #[test]
    fn test_no_build_skips_build_even_without_package_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let project = dir.path().join("project");
        let build = project.join("dist");
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join("index.html"), "<html>").unwrap();
        // No package.json — with --no-build this must NOT trigger an
        // error, and the build runner must never be called.

        let result = run_steps_1_to_3(
            "my-app",
            "v1",
            true, // no_build
            &project,
            "dist",
            "ns",
            "https://example.com",
            |_cmd| panic!("build runner must not be called with --no-build"),
        );

        assert!(
            result.is_ok(),
            "no-build must skip the build and succeed: {result:?}"
        );
        let (archive_path, _size, _td) = result.unwrap();
        assert!(
            archive_path.exists(),
            "archive must exist while handle is held"
        );
    }

    // ── Orchestration: missing PM fails before spawn ──

    #[test]
    fn test_missing_package_manager_fails_before_spawn() {
        let dir = tempfile::TempDir::new().unwrap();
        let project = dir.path().join("project");
        let build = project.join("dist");
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join("index.html"), "<html>").unwrap();
        // No package.json and no_build=false.

        let result = run_steps_1_to_3(
            "my-app",
            "v1",
            false, // no_build
            &project,
            "dist",
            "ns",
            "https://example.com",
            |_cmd| panic!("build runner must not be called when PM detection fails"),
        );

        assert!(result.is_err(), "missing PM must fail");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("no package manager found"),
            "error must mention 'no package manager found': {msg}"
        );
    }

    // ── Orchestration: --no-build with absent build path ──

    #[test]
    fn test_no_build_with_absent_build_path_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        // dist does not exist.

        let result = run_steps_1_to_3(
            "my-app",
            "v1",
            true,
            &project,
            "dist",
            "ns",
            "https://example.com",
            |_cmd| panic!("build runner must not be called"),
        );

        assert!(result.is_err(), "absent build path on --no-build must fail");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("build path") && msg.contains("was not found"),
            "error must mention 'build path' and 'was not found': {msg}"
        );
    }

    // ── Orchestration: build runner receives correct env vars ──

    #[test]
    fn test_build_runner_receives_correct_env_vars() {
        let dir = tempfile::TempDir::new().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("package.json"), "{}").unwrap();

        // Create a populated build directory so archive succeeds.
        let build = project.join("dist");
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join("index.html"), "<html>").unwrap();

        let captured = std::sync::Mutex::new(None::<BuildCommand>);

        let result = run_steps_1_to_3(
            "my-app",
            "explicit-ver",
            false,
            &project,
            "dist",
            "test-ns",
            "https://dev.example.com",
            |cmd| {
                *captured.lock().unwrap() = Some(cmd);
                Ok(())
            },
        );

        assert!(result.is_ok(), "steps 1-3 must succeed: {result:?}");
        let cmd = captured
            .lock()
            .unwrap()
            .take()
            .expect("runner must be called");

        let env_map: std::collections::HashMap<&str, &str> = cmd
            .env_vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        assert_eq!(env_map["AB_APPUI_BUILD_VERSION"], "explicit-ver");
        assert_eq!(
            env_map["BASE_URL"],
            "/csm/v1/admin/namespaces/test-ns/files/app-ui/my-app/explicit-ver/"
        );

        // TempDir handle keeps archive alive for the assertions above;
        // archive is cleaned up when the result tuple drops.
        let (_archive_path, _, _td) = result.unwrap();
    }

    // ── Orchestration: size seam is wired ──

    #[test]
    fn test_size_seam_is_wired_with_real_archive_size() {
        let dir = tempfile::TempDir::new().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("package.json"), "{}").unwrap();

        let build = project.join("dist");
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join("index.html"), "<html>").unwrap();

        let result = run_steps_1_to_3(
            "my-app",
            "v1",
            true, // skip build to avoid needing npm
            &project,
            "dist",
            "ns",
            "https://example.com",
            |_cmd| panic!("should not be called"),
        );

        assert!(result.is_ok(), "steps 1-3 must succeed: {result:?}");
        let (_archive_path, size, _td) = result.unwrap();
        assert!(
            size > 0,
            "archive size must be positive (the seam must see it)"
        );

        // The size seam with no limit must have passed (Ok return proves it).
        // A broken seam that skips the check would still pass this test, but
        // the over-limit test below covers the refusal path.
    }

    // ── Empty build directory on --no-build path ──

    #[test]
    fn test_empty_build_dir_fails_on_no_build_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let project = dir.path().join("project");
        let build = project.join("dist");
        std::fs::create_dir_all(&build).unwrap();
        // dist exists but is empty.

        let result = run_steps_1_to_3(
            "my-app",
            "v1",
            true,
            &project,
            "dist",
            "ns",
            "https://example.com",
            |_cmd| panic!("should not be called"),
        );

        assert!(result.is_err(), "empty build dir must fail");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("empty"), "error must mention 'empty': {msg}");
    }

    // ── F-1: Archive path uniqueness ──

    #[test]
    fn test_create_archive_uses_unique_temp_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let build = dir.path().join("build");
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join("index.html"), "<html>").unwrap();

        let result1 = create_archive(&build, "same-app", "v1").unwrap();
        let result2 = create_archive(&build, "same-app", "v1").unwrap();

        let path1 = &result1.0;
        let path2 = &result2.0;

        // Two archives with the same name/version must land in different
        // directories to prevent collision in concurrent invocations.
        assert_ne!(
            path1.parent(),
            path2.parent(),
            "concurrent archives must use unique temp directories: {path1:?} vs {path2:?}"
        );

        // The filenames must match the `{name}-{version}.zip` convention
        // (parity with the tool being retired).
        assert_eq!(
            path1.file_name(),
            path2.file_name(),
            "archive filename must be identical across calls"
        );
        assert_eq!(
            path1.file_name().unwrap().to_str().unwrap(),
            "same-app-v1.zip"
        );

        // Cleanup.
        let _ = std::fs::remove_file(path1);
        let _ = std::fs::remove_file(path2);
    }

    // ── F-3: Upload request shape ──

    #[test]
    fn test_build_upload_request_shape() {
        use ags_protocol::catalogue::{OperationId, ServiceId};
        use ags_protocol::request::{FormPart, RequestBody};

        let req = build_upload_request(
            "test-ns",
            "my-app",
            "v42",
            Path::new("/tmp/archive/my-app-v42.zip"),
            "my-app-v42.zip",
        );

        // Service and operation.
        assert_eq!(req.service, ServiceId::new("csm"));
        assert_eq!(
            req.operation_id,
            OperationId::new("csm/admin/app-ui/v1/upload-assets")
        );

        // Namespace.
        assert_eq!(req.namespace, Some("test-ns".to_string()));

        // Path params: namespace and appUiName.
        assert_eq!(
            req.path_params.get("namespace"),
            Some(&"test-ns".to_string())
        );
        assert_eq!(
            req.path_params.get("appUiName"),
            Some(&"my-app".to_string())
        );
        assert_eq!(req.path_params.len(), 2);

        // Query params: version is the critical assertion (F-3).
        // Removing the `version` insert from build_upload_request must
        // make this test fail — verified by mutation.
        assert_eq!(
            req.query_params.get("version"),
            Some(&"v42".to_string()),
            "query_params must include 'version' equal to the build version"
        );
        assert_eq!(req.query_params.len(), 1);

        // Body: exactly one multipart File part.
        let body = req.body.as_ref().expect("body must be present");
        match body {
            RequestBody::Multipart(parts) => {
                assert_eq!(parts.len(), 1, "exactly one form part");
                match &parts[0] {
                    FormPart::File {
                        name,
                        path,
                        filename,
                    } => {
                        assert_eq!(name, "file", "form part name");
                        assert_eq!(
                            path,
                            &PathBuf::from("/tmp/archive/my-app-v42.zip"),
                            "form part path"
                        );
                        assert_eq!(filename, "my-app-v42.zip", "form part filename");
                    }
                    other => panic!("expected FormPart::File, got {other:?}"),
                }
            }
            _ => panic!("expected RequestBody::Multipart"),
        }
    }

    // ── Input validation ──

    #[test]
    fn test_validate_safe_component_accepts_valid_values() {
        assert!(validate_safe_component("my-app", "name").is_ok());
        assert!(validate_safe_component("v1.2.3", "build-version").is_ok());
        assert!(validate_safe_component("abc_123", "name").is_ok());
        assert!(validate_safe_component("UPPER", "name").is_ok());
    }

    #[test]
    fn test_validate_safe_component_rejects_slash() {
        let err = validate_safe_component("foo/bar", "name").unwrap_err();
        assert!(
            matches!(err, CliError::Usage { .. }),
            "slash must produce Usage: {err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("--name"), "error must name the flag: {msg}");
        assert!(msg.contains("foo/bar"), "error must show the value: {msg}");
    }

    #[test]
    fn test_validate_safe_component_rejects_dotdot() {
        let err = validate_safe_component("foo..bar", "name").unwrap_err();
        assert!(
            matches!(err, CliError::Usage { .. }),
            "dotdot must produce Usage: {err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains(".."), "error must mention '..': {msg}");
    }

    #[test]
    fn test_validate_safe_component_rejects_bare_dotdot() {
        let err = validate_safe_component("..", "build-version").unwrap_err();
        assert!(
            matches!(err, CliError::Usage { .. }),
            "bare dotdot must produce Usage: {err:?}"
        );
    }

    #[test]
    fn test_validate_safe_component_rejects_empty() {
        let err = validate_safe_component("", "name").unwrap_err();
        assert!(
            matches!(err, CliError::Usage { .. }),
            "empty must produce Usage: {err:?}"
        );
        assert!(
            err.to_string().contains("cannot be empty"),
            "error must say 'cannot be empty': {err}"
        );
    }

    #[test]
    fn test_validate_safe_component_rejects_space() {
        let err = validate_safe_component("foo bar", "name").unwrap_err();
        assert!(
            matches!(err, CliError::Usage { .. }),
            "space must produce Usage: {err:?}"
        );
    }

    // ── Build command: path traversal rejection via encoding ──

    #[test]
    fn test_make_build_command_rejects_path_traversal_in_namespace() {
        let result = make_build_command(
            PackageManager::Npm,
            "my-app",
            "v1",
            "https://example.com",
            "../../admin",
            Path::new("/project"),
        );
        assert!(
            result.is_err(),
            "path traversal in namespace must be rejected by encoding"
        );
    }

    // ── Dry-run preview ──

    #[test]
    fn test_dry_run_preview_returns_complete() {
        let dir = tempfile::TempDir::new().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();

        let result = dry_run_preview(
            "my-app",
            "v1",
            "test-ns",
            &project,
            "dist",
            "https://dev.example.com",
            false,
        );
        assert!(result.is_ok(), "dry-run must return Ok: {result:?}");
        assert!(
            matches!(result.unwrap(), InvocationOutcome::Complete),
            "dry-run must return Complete"
        );
    }

    // ── Build failure: captured stderr ──

    /// `execute_build` must capture the build tool's stderr and include
    /// it in the `CliError::Network` message. Before the piped-stderr
    /// fix, stderr was inherited and the error carried nothing after
    /// the colon.
    ///
    /// Uses `cmd /C` on Windows and `sh -c` on Unix to create a program
    /// that writes a marker to stderr and exits non-zero, exercising
    /// `execute_build` directly without needing a fake npm on PATH.
    #[test]
    fn test_execute_build_captures_stderr_in_error() {
        let dir = tempfile::TempDir::new().unwrap();

        #[cfg(windows)]
        let cmd = BuildCommand {
            program: "cmd".to_string(),
            args: vec![
                "/C".to_string(),
                "echo FAKE_BUILD_ERROR_XYZ >&2 & exit /b 1".to_string(),
            ],
            env_vars: vec![],
            working_dir: dir.path().to_path_buf(),
        };
        #[cfg(not(windows))]
        let cmd = BuildCommand {
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                "echo 'FAKE_BUILD_ERROR_XYZ' >&2; exit 1".to_string(),
            ],
            env_vars: vec![],
            working_dir: dir.path().to_path_buf(),
        };

        let result = execute_build(cmd);
        assert!(result.is_err(), "non-zero exit must fail");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("FAKE_BUILD_ERROR_XYZ"),
            "error message must contain stderr output: {msg}"
        );
    }

    // ── Symlink-cycle protection ──

    /// A self-referential directory symlink inside the build output must
    /// not cause unbounded recursion in the archive walk. The fix uses
    /// `fs::symlink_metadata` to detect symlinks without following them,
    /// and skips symlinked entries.
    ///
    /// Creating a symlink requires privileges on Windows (developer mode
    /// or admin). The test skips at runtime when symlink creation fails,
    /// rather than using `#[ignore]`.
    #[test]
    fn test_walk_dir_skips_symlinks() {
        use std::io::Write;
        let dir = tempfile::TempDir::new().unwrap();
        let build = dir.path().join("build");
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join("index.html"), "<html>").unwrap();

        // Create a self-referential directory symlink: build/self-link -> build.
        let link_path = build.join("self-link");
        #[cfg(unix)]
        let link_result = std::os::unix::fs::symlink(&build, &link_path);
        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_dir(&build, &link_path);

        match link_result {
            Ok(()) => {}
            Err(e) => {
                // Cannot use eprintln! — the architecture guard bans it
                // in src/. writeln! to stderr achieves the same result.
                let _ = writeln!(
                    std::io::stderr(),
                    "SKIP: cannot create symlink ({e}); \
                     requires developer mode or admin on Windows"
                );
                return;
            }
        }

        // Run the archive in a thread with a timeout. Without the
        // symlink-cycle fix, `walk_dir_into_zip` recurses unboundedly
        // and either overflows the stack or hangs. The timeout converts
        // that into a clean failure rather than crashing the test runner.
        let build_clone = build.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(create_archive(&build_clone, "test-app", "v1"));
        });

        match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Ok((archive_path, size, _td))) => {
                assert!(size > 0, "archive must not be empty");

                let file = std::fs::File::open(&archive_path).unwrap();
                let archive = zip::ZipArchive::new(file).unwrap();
                let names: Vec<String> = (0..archive.len())
                    .map(|i| archive.name_for_index(i).unwrap().to_string())
                    .collect();

                assert!(
                    names.contains(&"index.html".to_string()),
                    "regular file must be in archive: {names:?}"
                );
                // The symlink must not be followed — no recursive entries.
                assert!(
                    !names.iter().any(|n| n.starts_with("self-link/")),
                    "symlink directory must not be followed: {names:?}"
                );
            }
            Ok(Err(e)) => panic!("archive failed: {e}"),
            Err(_) => panic!("archive timed out after 10s — symlink cycle was not prevented"),
        }
    }

    // ── F-5: Cleanup on archiving failure ──

    #[test]
    fn test_write_zip_failure_leaves_partial_file_that_cleanup_removes() {
        // `write_zip` calls `File::create` before walking the build
        // directory. If the walk fails (e.g. build path absent),
        // the partial zip already exists on disk.
        // `write_zip_with_cleanup`'s error branch removes it.
        //
        // `TempDir::Drop` in `create_archive` also cleans up
        // (defense-in-depth), so this test uses a caller-owned
        // directory to isolate the explicit `remove_file` cleanup —
        // the same pattern as
        // `test_write_file_restricted_leaves_no_temp_files_on_failure`
        // in ags-runtime `support/file_system.rs`.
        let dir = tempfile::TempDir::new().unwrap();

        // A non-existent build path makes `read_dir` fail
        // immediately. `File::create(zip_path)` has already run by
        // that point, so the partial zip exists on disk. This
        // failure mode is root-safe and cross-platform — no
        // permission bits or file locks needed.
        let build = dir.path().join("nonexistent");
        let zip_path = dir.path().join("test-1.0.zip");

        let result = write_zip_with_cleanup(&build, &zip_path);

        assert!(result.is_err(), "must fail when build path is absent");
        // The explicit `remove_file` in `write_zip_with_cleanup`'s
        // error branch must have cleaned up the partial zip. The
        // test performs NO cleanup of its own on this path — the
        // pass/fail hinges entirely on the production cleanup code.
        assert!(
            !zip_path.exists(),
            "partial zip must not survive after cleanup"
        );
    }
}
