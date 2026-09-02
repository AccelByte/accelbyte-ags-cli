//! Handler for `ags extend image-upload`.
//!
//! Builds and pushes a container image to the Extend registry via the
//! `docker` (or `podman`) binary. The eight-step execution order is:
//!
//! 1. Verify `docker` is on PATH (detect podman)
//! 2. `--dry-run` short-circuits here
//! 3. EHS credential fetch (only when `--login`)
//! 4. `docker login --password-stdin` (only when `--login`)
//! 5. CSM app read for `appRepoUrl` (always)
//! 6. Duplicate-tag pre-check (only when `--login`)
//! 7. Build the command list
//! 8. Execute with retry

use std::time::Duration;

use crate::errors::CliError;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;

/// Maximum wall-clock time to wait for a single `docker build/push`
/// invocation. Container builds can be long-running; 30 minutes is
/// generous while still catching a truly hung process.
const DOCKER_BUILD_TIMEOUT: Duration = Duration::from_secs(1800);

/// Cap on docker stderr bytes propagated into user-facing error strings.
const DOCKER_STDERR_DISPLAY_LIMIT: usize = 512;

/// Upper bound on the backoff delay in seconds. Prevents absurdly long
/// waits from exponential blowup while still allowing generous retry
/// schedules. Ten minutes is generous for any interactive CLI workflow.
const MAX_BACKOFF_DELAY_SECS: f64 = 600.0;

// ── Data types ──

/// Parsed parameters for the image-upload command.
#[derive(Debug, Clone)]
pub(crate) struct ImageUploadParams {
    pub app: String,
    pub image_tag: String,
    pub dockerfile: String,
    pub platforms: Vec<String>,
    pub work_dir: Option<String>,
    pub login: bool,
    pub retry_limit: u32,
    pub retry_interval: f64,
    pub retry_rate: f64,
}

/// A resolved command to execute (one or two depending on podman vs docker).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImageCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Whether the resolved `docker` binary is actually podman.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DockerFlavor {
    Docker,
    Podman,
}

/// Result of splitting an `appRepoUrl` into registry host and repository path.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RegistryRepo {
    pub registry: String,
    pub repo: String,
}

/// Captures the step-execution plan for the image-upload pipeline.
///
/// Each field indicates whether the corresponding step should execute.
/// The plan is determined solely by the `--login` flag; separating plan
/// construction from execution makes the gating logic testable without
/// Docker or network access.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UploadPlan {
    /// Step 3: fetch EHS credentials from the Extend service.
    pub fetch_credentials: bool,
    /// Step 4: run `docker login --password-stdin`.
    pub docker_login: bool,
    /// Step 5: if true, reuse the runtime created in step 3 for the
    /// CSM app-repo-url fetch rather than creating a new one.
    pub reuse_runtime: bool,
    /// Step 6: HEAD-check whether the tag already exists.
    pub check_tag_exists: bool,
}

impl UploadPlan {
    /// Build a plan from the image-upload parameters.
    ///
    /// `--login` gates steps 3, 4, and 6. When login is active, step 5
    /// reuses the runtime created in step 3 to avoid a redundant auth
    /// resolution.
    pub fn from_params(params: &ImageUploadParams) -> Self {
        Self {
            fetch_credentials: params.login,
            docker_login: params.login,
            reuse_runtime: params.login,
            check_tag_exists: params.login,
        }
    }
}

// ── Pure functions ──

/// Calculate exponential backoff delay in seconds.
///
/// Formula mirrors Go's `CalculateBackoff`: `interval * rate^attempts`.
/// `attempts` is the number of failures so far, so the first retry
/// (after one failure) uses `interval * rate^1`.
pub(crate) fn calculate_backoff(attempts: u32, interval: f64, rate: f64) -> f64 {
    interval * rate.powi(attempts as i32)
}

/// Clamp a backoff delay to a safe range for `Duration::from_secs_f64`.
///
/// Returns a finite non-negative value bounded by [`MAX_BACKOFF_DELAY_SECS`].
/// Defends against negative values (from misconfigured input), NaN, and
/// infinity that would panic `Duration::from_secs_f64`.
pub(crate) fn clamp_backoff_delay(raw: f64) -> f64 {
    if raw.is_nan() || raw < 0.0 {
        0.0
    } else if raw > MAX_BACKOFF_DELAY_SECS {
        // Catches both finite values above the cap and positive infinity.
        MAX_BACKOFF_DELAY_SECS
    } else {
        raw
    }
}

/// Check a Docker image tag against the OCI tag charset rule.
///
/// Docker tags follow `[A-Za-z0-9_][A-Za-z0-9._-]{0,127}`. This is
/// stricter than a generic URL path segment; slashes, query strings,
/// and path traversal (`..`) are all invalid and could redirect
/// authenticated OCI registry requests if allowed through.
///
/// Returns `Err(reason)` with a human-readable message on violation.
/// This is the single implementation of the charset rule, shared by
/// the clap `value_parser` in `builder.rs` and the defense-in-depth
/// guard in [`validate_image_tag`].
pub(crate) fn check_docker_tag(tag: &str) -> Result<(), String> {
    if tag.is_empty() {
        return Err("cannot be empty".to_string());
    }
    let bytes = tag.as_bytes();
    if bytes.len() > 128 {
        return Err("must be at most 128 characters".to_string());
    }
    let first = bytes[0];
    if !first.is_ascii_alphanumeric() && first != b'_' {
        return Err(format!(
            "must start with an alphanumeric character or underscore, got '{}'",
            first as char
        ));
    }
    for &b in &bytes[1..] {
        if !b.is_ascii_alphanumeric() && b != b'_' && b != b'.' && b != b'-' {
            return Err(format!(
                "contains invalid character '{}'; \
                 allowed: [A-Za-z0-9._-]",
                b as char
            ));
        }
    }
    Ok(())
}

/// Validate a Docker image tag against the OCI tag charset.
///
/// Defense-in-depth: the clap `value_parser` validates at the CLI
/// edge; this function guards the URL construction site in
/// [`check_tag_exists_with_base`]. Both delegate to [`check_docker_tag`].
pub(crate) fn validate_image_tag(tag: &str) -> Result<(), CliError> {
    check_docker_tag(tag).map_err(|reason| {
        let metadata = if tag.is_empty() {
            Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Provide a non-empty value for --image-tag.",
            )))
        } else {
            None
        };
        CliError::Usage {
            message: format!("image tag {reason}"),
            metadata,
        }
    })
}

/// Build the list of commands to execute for a build-and-push.
///
/// Podman cannot use `buildx build --push`, so it falls back to
/// separate `build` + `push` commands. Docker uses a single
/// `buildx build --push` invocation.
pub(crate) fn make_image_cmds(
    flavor: DockerFlavor,
    app_repo_url: &str,
    image_tag: &str,
    dockerfile: &str,
    platforms: &[String],
    work_dir: &str,
) -> Vec<ImageCommand> {
    let full_tag = format!("{app_repo_url}:{image_tag}");
    let platform_csv = platforms.join(",");

    match flavor {
        DockerFlavor::Podman => {
            let build = ImageCommand {
                program: "docker".to_string(),
                args: vec![
                    "build".to_string(),
                    "--tag".to_string(),
                    full_tag.clone(),
                    "-f".to_string(),
                    dockerfile.to_string(),
                    "--platform".to_string(),
                    platform_csv,
                    work_dir.to_string(),
                ],
            };
            let push = ImageCommand {
                program: "docker".to_string(),
                args: vec!["push".to_string(), full_tag],
            };
            vec![build, push]
        }
        DockerFlavor::Docker => {
            let cmd = ImageCommand {
                program: "docker".to_string(),
                args: vec![
                    "buildx".to_string(),
                    "build".to_string(),
                    "--tag".to_string(),
                    full_tag,
                    "-f".to_string(),
                    dockerfile.to_string(),
                    "--platform".to_string(),
                    platform_csv,
                    "--push".to_string(),
                    work_dir.to_string(),
                ],
            };
            vec![cmd]
        }
    }
}

/// Split an `appRepoUrl` (e.g. `registry.example.com/ns/repo`) into
/// the registry host and the repository path. The first `/` is the
/// split point; if there is no `/`, the entire string is treated as the
/// registry and the repo is empty (which the OCI tag check will skip).
pub(crate) fn split_registry_repo(app_repo_url: &str) -> RegistryRepo {
    match app_repo_url.find('/') {
        Some(idx) => RegistryRepo {
            registry: app_repo_url[..idx].to_string(),
            repo: app_repo_url[idx + 1..].to_string(),
        },
        None => RegistryRepo {
            registry: app_repo_url.to_string(),
            repo: String::new(),
        },
    }
}

/// Format the dry-run preview lines showing the command(s) that would
/// be executed. Returns the complete multi-line string to emit.
pub(crate) fn format_dry_run_lines(flavor: DockerFlavor, params: &ImageUploadParams) -> String {
    let cmds = make_image_cmds(
        flavor,
        "<appRepoUrl>",
        &params.image_tag,
        &params.dockerfile,
        &params.platforms,
        params.work_dir.as_deref().unwrap_or("."),
    );
    let mut lines = Vec::new();
    for cmd in &cmds {
        let full = std::iter::once(cmd.program.as_str())
            .chain(cmd.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!("  {full}"));
    }
    lines.join("\n")
}

/// Map a `docker` spawn error to the appropriate `CliError`.
///
/// `NotFound` means the binary is missing. Other I/O errors are
/// internal failures.
pub(crate) fn map_docker_spawn_error(e: std::io::Error) -> CliError {
    if e.kind() == std::io::ErrorKind::NotFound {
        CliError::Usage {
            message: "docker is not installed or not found on PATH".to_string(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Install Docker from https://docs.docker.com/get-docker/ and ensure it is on your PATH.",
            ))),
        }
    } else {
        CliError::Network {
            message: format!("image-upload: failed to run docker: {e}"),
            metadata: None,
        }
    }
}

/// Map a [`WaitError`] from a docker subprocess into a `CliError`.
pub(crate) fn map_docker_wait_error(err: ags_runtime::support::process::WaitError) -> CliError {
    match err {
        ags_runtime::support::process::WaitError::TimedOut(d) => CliError::Network {
            message: format!(
                "image-upload: docker command timed out after {}s",
                d.as_secs()
            ),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "The build or push did not complete in time. Check your network and retry.",
            ))),
        },
        ags_runtime::support::process::WaitError::Wait(e) => CliError::Network {
            message: format!("image-upload: failed to wait for docker process: {e}"),
            metadata: None,
        },
    }
}

/// Sanitize Docker stderr for display: strip control sequences and
/// truncate to the display limit.
fn sanitize_docker_stderr(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let cleaned = ags_runtime::support::strings::strip_terminal_control_sequences(&text);
    ags_runtime::support::strings::truncate_display_text(&cleaned, DOCKER_STDERR_DISPLAY_LIMIT)
}

// ── Docker introspection ──

/// Check that `docker` is on PATH. Returns the detected flavor
/// (Docker vs Podman). This is a local-only check — no network calls.
///
/// Detection strategy mirrors Go's `detectPodman()`:
/// 1. Run `docker --version` — if it fails with `NotFound`, docker is
///    not installed.
/// 2. Check the version output for "podman" (covers podman aliased as
///    `docker` and podman-docker packages).
pub(crate) fn check_docker_available() -> Result<DockerFlavor, CliError> {
    let output = std::process::Command::new("docker")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CliError::Usage {
                    message: "docker is not installed or not found on PATH".to_string(),
                    metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                        "Install Docker from https://docs.docker.com/get-docker/ \
                         and ensure it is on your PATH.",
                    ))),
                }
            } else {
                CliError::Network {
                    message: format!("image-upload: failed to probe docker: {e}"),
                    metadata: None,
                }
            }
        })?;

    let version_text = String::from_utf8_lossy(&output.stdout).to_lowercase();
    if version_text.contains("podman") {
        return Ok(DockerFlavor::Podman);
    }

    Ok(DockerFlavor::Docker)
}

// ── Dry-run preview ──

/// Emit the dry-run preview to stderr and return `Complete`.
pub(crate) fn dry_run_preview(
    flavor: DockerFlavor,
    params: &ImageUploadParams,
) -> Result<InvocationOutcome, CliError> {
    let color = crate::frontend::style::is_stderr_enabled();
    crate::frontend::write_stderr_line(&crate::frontend::style::info(
        "Dry run — no image will be built or pushed",
        color,
    ));
    crate::frontend::write_stderr_line(&format!("  App:        {}", params.app));
    crate::frontend::write_stderr_line(&format!("  Image tag:  {}", params.image_tag));
    crate::frontend::write_stderr_line(&format!("  Dockerfile: {}", params.dockerfile));
    crate::frontend::write_stderr_line(&format!("  Platforms:  {}", params.platforms.join(",")));
    crate::frontend::write_stderr_line(&format!(
        "  Work dir:   {}",
        params.work_dir.as_deref().unwrap_or(".")
    ));
    crate::frontend::write_stderr_line(&format!("  Login:      {}", params.login));
    crate::frontend::write_stderr_line(&format!(
        "  Flavor:     {}",
        match flavor {
            DockerFlavor::Docker => "docker",
            DockerFlavor::Podman => "podman",
        }
    ));
    crate::frontend::write_stderr_line("");
    crate::frontend::write_stderr_line("  Commands that would be executed:");
    let cmd_lines = format_dry_run_lines(flavor, params);
    crate::frontend::write_stderr_line(&cmd_lines);

    Ok(InvocationOutcome::Complete)
}

// ── OCI duplicate-tag pre-check ──

/// Check whether a tag already exists in the OCI registry.
///
/// Sends `HEAD https://{registry}/v2/{repo}/manifests/{tag}` with
/// Basic Auth. Only HTTP 200 is treated as "tag exists" and returns
/// `Err`. All other outcomes (network errors, non-200 status codes)
/// are swallowed — the check is advisory and must not make the command
/// *less* reliable than the tool it replaces.
///
/// When the check cannot be performed, a single stderr line is emitted
/// so a silently-skipped guard becomes visible.
pub(crate) async fn check_tag_exists_or_skip(
    registry_repo: &RegistryRepo,
    tag: &str,
    username: &str,
    token: &str,
) -> Result<(), CliError> {
    let base = format!("https://{}", registry_repo.registry);
    check_tag_exists_with_base(registry_repo, tag, username, token, &base).await
}

/// Inner implementation with injectable base URL for testability.
/// Production callers use [`check_tag_exists_or_skip`] which hardcodes
/// HTTPS. Tests pass the wiremock server URI (plain HTTP).
async fn check_tag_exists_with_base(
    registry_repo: &RegistryRepo,
    tag: &str,
    username: &str,
    token: &str,
    base: &str,
) -> Result<(), CliError> {
    if registry_repo.repo.is_empty() {
        // Cannot construct a valid manifest URL without a repo path.
        return Ok(());
    }

    // Defense-in-depth: reject tags that would alter the URL path.
    // The primary guard is the clap value_parser, but this function may
    // be called from future code paths that bypass the CLI edge.
    validate_image_tag(tag)?;

    let url = format!("{}/v2/{}/manifests/{}", base, registry_repo.repo, tag);

    let client = reqwest::Client::new();
    let result = client
        .head(&url)
        .basic_auth(username, Some(token))
        .header(
            "Accept",
            "application/vnd.oci.image.manifest.v1+json, \
             application/vnd.docker.distribution.manifest.v2+json",
        )
        .send()
        .await;

    match result {
        Ok(response) if response.status().as_u16() == 200 => {
            return Err(CliError::Usage {
                message: format!("image tag '{tag}' already exists in the registry"),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Use a different --image-tag value, or delete the existing image first.",
                ))),
            });
        }
        Ok(_) => {
            // Non-200 status (e.g. 404, 401): tag does not exist or
            // cannot be checked. Proceed.
        }
        Err(_e) => {
            // Transport error: DNS failure, timeout, etc. Swallow per
            // Go parity, but emit a notice so the skipped check is visible.
            crate::frontend::write_stderr_line(
                "  Note: could not verify tag uniqueness (network error); proceeding.",
            );
        }
    }
    Ok(())
}

// ── Command execution with retry ──

/// Execute a single docker command, capturing output.
///
/// Docker's stdout is piped and forwarded to the CLI's stderr after
/// completion (build progress should not pollute stdout). Docker's
/// stderr is inherited for real-time progress output.
pub(crate) fn run_single_command(cmd: &ImageCommand) -> Result<(), CliError> {
    let mut process = std::process::Command::new(&cmd.program);
    process.args(&cmd.args);
    process.stdin(std::process::Stdio::null());
    // Pipe stdout so we can forward to stderr; inherit stderr for
    // real-time build progress.
    process.stdout(std::process::Stdio::piped());
    process.stderr(std::process::Stdio::inherit());

    let child = process.spawn().map_err(map_docker_spawn_error)?;

    let output = ags_runtime::support::process::wait_with_timeout(child, DOCKER_BUILD_TIMEOUT)
        .map_err(map_docker_wait_error)?;

    // Forward captured stdout to stderr (build output, not data output).
    if !output.stdout.is_empty() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            crate::frontend::write_stderr_line(line);
        }
    }

    if !output.status.success() {
        let code = output.status.code().unwrap_or(1);
        let stderr_text = sanitize_docker_stderr(&output.stderr);
        return Err(CliError::Network {
            message: format!(
                "image-upload: docker command failed (exit code {code}): {stderr_text}"
            ),
            metadata: None,
        });
    }

    Ok(())
}

/// Execute all image commands with retry logic.
///
/// The `executor` closure is injectable for testing: production callers
/// pass `run_single_command`, tests pass a closure that counts calls
/// or simulates failures.
pub(crate) fn execute_with_retry<F>(
    params: &ImageUploadParams,
    app_repo_url: &str,
    flavor: DockerFlavor,
    executor: F,
) -> Result<(), CliError>
where
    F: Fn(&ImageCommand) -> Result<(), CliError>,
{
    let max_attempts = params.retry_limit + 1;
    let mut attempts: u32 = 0;

    loop {
        // Re-build the command list on every attempt — a `Command`
        // object is consumed by a single run.
        let cmds = make_image_cmds(
            flavor,
            app_repo_url,
            &params.image_tag,
            &params.dockerfile,
            &params.platforms,
            params.work_dir.as_deref().unwrap_or("."),
        );

        let mut last_error: Option<CliError> = None;
        let mut all_ok = true;

        for cmd in &cmds {
            match executor(cmd) {
                Ok(()) => {}
                Err(e) => {
                    last_error = Some(e);
                    all_ok = false;
                    break;
                }
            }
        }

        if all_ok {
            return Ok(());
        }

        attempts += 1;
        if attempts >= max_attempts {
            return Err(last_error.unwrap_or_else(|| {
                CliError::Internal(anyhow::anyhow!(
                    "image-upload: retry loop exhausted with no recorded error"
                ))
            }));
        }

        // Backoff: first retry uses `interval * rate^1`.
        // Clamp to a safe range — negative, NaN, or infinite values from
        // misconfigured flags would panic `Duration::from_secs_f64`.
        let delay = clamp_backoff_delay(calculate_backoff(
            attempts,
            params.retry_interval,
            params.retry_rate,
        ));
        crate::frontend::write_stderr_line(&format!(
            "  Retrying in {delay:.1}s (attempt {}/{max_attempts})...",
            attempts + 1
        ));
        std::thread::sleep(Duration::from_secs_f64(delay));
    }
}

// ── Main orchestrator ──

/// Execute the image-upload command.
///
/// Follows the eight-step execution order defined in the spec. Step
/// gating is driven by [`UploadPlan`], which determines which optional
/// steps run based on `--login`.
pub(crate) async fn handle_image_upload(
    params: &ImageUploadParams,
    flags: &GlobalFlags,
) -> Result<InvocationOutcome, CliError> {
    let plan = UploadPlan::from_params(params);

    // Step 1: docker on PATH (detect podman).
    let flavor = check_docker_available()?;

    // Step 2: --dry-run short-circuits.
    if flags.is_dry_run {
        return dry_run_preview(flavor, params);
    }

    // Step 3: EHS credential fetch (gated by plan).
    let credentials = if plan.fetch_credentials {
        let input = ags_runtime::runtime::execution::ResolutionInput {
            profile: flags.profile.clone(),
            namespace: flags.namespace.clone(),
            is_dry_run: flags.is_dry_run,
        };
        let http_client = ags_runtime::runtime::dispatch::http::build_http_client(flags.timeout)?;
        let context =
            ags_runtime::runtime::execution::ExecutionContext::resolve(&input, &http_client)
                .await?;
        let runtime = ags_runtime::runtime::Runtime::from_reqwest(context, http_client);
        let namespace = resolve_namespace(flags)?;
        let creds = runtime
            .fetch_docker_credentials(&namespace, &params.app)
            .await?;
        Some((runtime, creds))
    } else {
        None
    };

    // Step 4: docker login (gated by plan).
    if plan.docker_login {
        if let Some((ref runtime, ref creds)) = credentials {
            let inputs_map: serde_json::Map<String, serde_json::Value> =
                crate::invocation::routes::extend_docker_login::credentials_to_inputs(
                    &creds.registry_url,
                    &creds.username,
                    &creds.token,
                )
                .into_iter()
                .collect();
            use ags_runtime::runtime::workflows::local_actions::docker_login::DockerLoginAction;
            use ags_runtime::runtime::workflows::local_actions::LocalAction;
            struct NoopSink;
            impl ags_protocol::event::ProgressSink for NoopSink {
                fn on_event(&mut self, _event: ags_protocol::event::ProgressEvent) {}
            }
            let mut sink = NoopSink;
            // Always a real run: handle_image_upload returns via dry_run_preview()
            // before this point when flags.is_dry_run is set.
            DockerLoginAction
                .run(runtime, &inputs_map, &mut sink, false)
                .await
                .map_err(CliError::from)?;
        }
    }

    // Step 5: CSM app read for appRepoUrl (always).
    // When plan.reuse_runtime is true, the runtime from step 3 is
    // reused to avoid a redundant auth resolution round-trip.
    let app_repo_url = {
        let namespace = resolve_namespace(flags)?;
        if plan.reuse_runtime {
            if let Some((ref runtime, _)) = credentials {
                runtime.fetch_app_repo_url(&namespace, &params.app).await?
            } else {
                // plan.reuse_runtime is true but credentials are None —
                // should not happen when plan is derived from params, but
                // handle gracefully by creating a new runtime.
                build_and_fetch_app_repo_url(flags, &namespace, &params.app).await?
            }
        } else {
            build_and_fetch_app_repo_url(flags, &namespace, &params.app).await?
        }
    };

    // Step 6: Duplicate-tag pre-check (gated by plan).
    if plan.check_tag_exists {
        if let Some((_, ref creds)) = credentials {
            let registry_repo = split_registry_repo(&app_repo_url);
            check_tag_exists_or_skip(
                &registry_repo,
                &params.image_tag,
                &creds.username,
                &creds.token,
            )
            .await?;
        }
    }

    // Step 7 + 8: Build command list and execute with retry.
    execute_with_retry(params, &app_repo_url, flavor, run_single_command)?;

    Ok(InvocationOutcome::Complete)
}

/// Build a fresh runtime and fetch the app repo URL.
///
/// Extracted from the no-login branch of step 5 for testability
/// and to avoid duplicating the auth resolution logic.
async fn build_and_fetch_app_repo_url(
    flags: &GlobalFlags,
    namespace: &str,
    app: &str,
) -> Result<String, CliError> {
    let input = ags_runtime::runtime::execution::ResolutionInput {
        profile: flags.profile.clone(),
        namespace: flags.namespace.clone(),
        is_dry_run: flags.is_dry_run,
    };
    let http_client = ags_runtime::runtime::dispatch::http::build_http_client(flags.timeout)?;
    let context =
        ags_runtime::runtime::execution::ExecutionContext::resolve(&input, &http_client).await?;
    let runtime = ags_runtime::runtime::Runtime::from_reqwest(context, http_client);
    runtime
        .fetch_app_repo_url(namespace, app)
        .await
        .map_err(CliError::from)
}

/// Resolve the namespace from flag, env, or profile config.
fn resolve_namespace(flags: &GlobalFlags) -> Result<String, CliError> {
    ags_runtime::runtime::execution::resolve_namespace(
        flags.namespace.as_deref(),
        flags.profile.as_deref(),
    )
    .map(|(namespace, _source)| namespace)
    .ok_or_else(|| CliError::Usage {
        message: "--namespace is required for image-upload".to_string(),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            "Supply --namespace <ns>, set AGS_NAMESPACE, or run 'ags config set namespace <ns>'",
        ))),
    })
}

// ══════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── T-IMU-01: calculate_backoff ──

    #[test]
    fn test_calculate_backoff_first_retry() {
        // First retry: attempts=1, interval=1.0, rate=2.0 → 1.0 * 2.0^1 = 2.0
        let delay = calculate_backoff(1, 1.0, 2.0);
        assert!(
            (delay - 2.0).abs() < f64::EPSILON,
            "first retry delay must be 2.0, got {delay}"
        );
    }

    #[test]
    fn test_calculate_backoff_second_retry() {
        // Second retry: attempts=2, interval=1.0, rate=2.0 → 1.0 * 2.0^2 = 4.0
        let delay = calculate_backoff(2, 1.0, 2.0);
        assert!(
            (delay - 4.0).abs() < f64::EPSILON,
            "second retry delay must be 4.0, got {delay}"
        );
    }

    #[test]
    fn test_calculate_backoff_custom_interval_and_rate() {
        // attempts=3, interval=0.5, rate=3.0 → 0.5 * 3.0^3 = 13.5
        let delay = calculate_backoff(3, 0.5, 3.0);
        assert!(
            (delay - 13.5).abs() < f64::EPSILON,
            "custom backoff must be 13.5, got {delay}"
        );
    }

    #[test]
    fn test_calculate_backoff_zero_attempts() {
        // attempts=0 → interval * rate^0 = interval * 1 = interval
        let delay = calculate_backoff(0, 1.0, 2.0);
        assert!(
            (delay - 1.0).abs() < f64::EPSILON,
            "zero attempts must return interval, got {delay}"
        );
    }

    // ── T-IMU-02: make_image_cmds ──

    #[test]
    fn test_make_image_cmds_docker_produces_single_buildx_push() {
        let cmds = make_image_cmds(
            DockerFlavor::Docker,
            "registry.example.com/repo",
            "v1.0",
            "Dockerfile",
            &["linux/amd64".to_string()],
            ".",
        );
        assert_eq!(cmds.len(), 1, "docker must produce exactly 1 command");
        let cmd = &cmds[0];
        assert!(
            cmd.args.contains(&"buildx".to_string()),
            "docker command must use buildx: {cmd:?}"
        );
        assert!(
            cmd.args.contains(&"--push".to_string()),
            "docker command must include --push: {cmd:?}"
        );
        assert!(
            cmd.args
                .contains(&"registry.example.com/repo:v1.0".to_string()),
            "docker command must include full tag: {cmd:?}"
        );
    }

    #[test]
    fn test_make_image_cmds_podman_produces_build_then_push() {
        let cmds = make_image_cmds(
            DockerFlavor::Podman,
            "registry.example.com/repo",
            "v1.0",
            "Dockerfile",
            &["linux/amd64".to_string()],
            ".",
        );
        assert_eq!(cmds.len(), 2, "podman must produce exactly 2 commands");
        assert_eq!(
            cmds[0].args[0], "build",
            "first podman command must be build"
        );
        assert_eq!(
            cmds[1].args[0], "push",
            "second podman command must be push"
        );
        assert!(
            !cmds[0].args.contains(&"--push".to_string()),
            "podman build must not include --push"
        );
    }

    #[test]
    fn test_make_image_cmds_multiple_platforms() {
        let cmds = make_image_cmds(
            DockerFlavor::Docker,
            "registry.example.com/repo",
            "v1.0",
            "Dockerfile",
            &["linux/amd64".to_string(), "linux/arm64".to_string()],
            ".",
        );
        // The platform CSV should appear in the args.
        let platform_arg_idx = cmds[0]
            .args
            .iter()
            .position(|a| a == "--platform")
            .expect("--platform must appear");
        let platform_val = &cmds[0].args[platform_arg_idx + 1];
        assert_eq!(
            platform_val, "linux/amd64,linux/arm64",
            "platforms must be comma-separated"
        );
    }

    // ── T-IMU-03: split_registry_repo ──

    #[test]
    fn test_split_registry_repo_normal_url() {
        let rr = split_registry_repo("registry.example.com/ns/repo");
        assert_eq!(rr.registry, "registry.example.com");
        assert_eq!(rr.repo, "ns/repo");
    }

    #[test]
    fn test_split_registry_repo_no_slash() {
        let rr = split_registry_repo("registry.example.com");
        assert_eq!(rr.registry, "registry.example.com");
        assert_eq!(rr.repo, "");
    }

    #[test]
    fn test_split_registry_repo_single_segment_repo() {
        let rr = split_registry_repo("registry.example.com/repo");
        assert_eq!(rr.registry, "registry.example.com");
        assert_eq!(rr.repo, "repo");
    }

    // ── T-IMU-04: format_dry_run_lines ──

    #[test]
    fn test_format_dry_run_lines_uses_placeholder() {
        let params = ImageUploadParams {
            app: "myapp".to_string(),
            image_tag: "v1.0".to_string(),
            dockerfile: "Dockerfile".to_string(),
            platforms: vec!["linux/amd64".to_string()],
            work_dir: None,
            login: false,
            retry_limit: 0,
            retry_interval: 1.0,
            retry_rate: 2.0,
        };
        let output = format_dry_run_lines(DockerFlavor::Docker, &params);
        assert!(
            output.contains("<appRepoUrl>"),
            "dry-run must use placeholder: {output}"
        );
        assert!(
            !output.contains("registry."),
            "dry-run must not contain real registry URLs: {output}"
        );
    }

    #[test]
    fn test_format_dry_run_lines_podman_shows_two_commands() {
        let params = ImageUploadParams {
            app: "myapp".to_string(),
            image_tag: "v1.0".to_string(),
            dockerfile: "Dockerfile".to_string(),
            platforms: vec!["linux/amd64".to_string()],
            work_dir: None,
            login: false,
            retry_limit: 0,
            retry_interval: 1.0,
            retry_rate: 2.0,
        };
        let output = format_dry_run_lines(DockerFlavor::Podman, &params);
        // Should contain both "build" and "push" lines.
        assert!(
            output.contains("build") && output.contains("push"),
            "podman dry-run must show build and push: {output}"
        );
    }

    // ── T-IMU-05: map_docker_spawn_error ──

    #[test]
    fn test_map_docker_spawn_error_not_found_is_usage() {
        let err = map_docker_spawn_error(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not found",
        ));
        assert!(
            matches!(err, CliError::Usage { .. }),
            "NotFound must map to Usage: {err:?}"
        );
    }

    #[test]
    fn test_map_docker_spawn_error_other_is_network() {
        let err = map_docker_spawn_error(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        ));
        assert!(
            matches!(err, CliError::Network { .. }),
            "PermissionDenied must map to Network: {err:?}"
        );
    }

    // ── T-IMU-06: map_docker_wait_error ──

    #[test]
    fn test_map_docker_wait_error_timeout_is_network() {
        let err = map_docker_wait_error(ags_runtime::support::process::WaitError::TimedOut(
            Duration::from_secs(30),
        ));
        assert!(
            matches!(err, CliError::Network { .. }),
            "TimedOut must map to Network: {err:?}"
        );
        if let CliError::Network { message, .. } = &err {
            assert!(
                message.contains("30"),
                "timeout message must include duration: {message}"
            );
        }
    }

    #[test]
    fn test_map_docker_wait_error_wait_is_network() {
        let err = map_docker_wait_error(ags_runtime::support::process::WaitError::Wait(
            std::io::Error::other("waitpid failed"),
        ));
        assert!(
            matches!(err, CliError::Network { .. }),
            "Wait must map to Network: {err:?}"
        );
    }

    // ── T-IMU-08: execute_with_retry (injectable executor) ──

    #[test]
    fn test_execute_with_retry_no_retries_on_success() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let call_count = AtomicU32::new(0);
        let params = ImageUploadParams {
            app: "myapp".to_string(),
            image_tag: "v1.0".to_string(),
            dockerfile: "Dockerfile".to_string(),
            platforms: vec!["linux/amd64".to_string()],
            work_dir: None,
            login: false,
            retry_limit: 3,
            retry_interval: 0.0, // zero delay for tests
            retry_rate: 1.0,
        };

        let result = execute_with_retry(
            &params,
            "registry.example.com/repo",
            DockerFlavor::Docker,
            |_cmd| {
                call_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        );

        assert!(result.is_ok(), "success path must not error");
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "success must execute exactly 1 command (no retries)"
        );
    }

    #[test]
    fn test_execute_with_retry_retries_on_failure() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let call_count = AtomicU32::new(0);
        let params = ImageUploadParams {
            app: "myapp".to_string(),
            image_tag: "v1.0".to_string(),
            dockerfile: "Dockerfile".to_string(),
            platforms: vec!["linux/amd64".to_string()],
            work_dir: None,
            login: false,
            retry_limit: 2, // max 3 attempts total
            retry_interval: 0.0,
            retry_rate: 1.0,
        };

        let result = execute_with_retry(
            &params,
            "registry.example.com/repo",
            DockerFlavor::Docker,
            |_cmd| {
                let n = call_count.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    // Fail first two attempts
                    Err(CliError::Network {
                        message: "simulated failure".to_string(),
                        metadata: None,
                    })
                } else {
                    Ok(())
                }
            },
        );

        assert!(result.is_ok(), "must succeed on third attempt");
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            3,
            "must have attempted 3 times (1 initial + 2 retries)"
        );
    }

    #[test]
    fn test_execute_with_retry_exhausts_retries() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let call_count = AtomicU32::new(0);
        let params = ImageUploadParams {
            app: "myapp".to_string(),
            image_tag: "v1.0".to_string(),
            dockerfile: "Dockerfile".to_string(),
            platforms: vec!["linux/amd64".to_string()],
            work_dir: None,
            login: false,
            retry_limit: 1, // max 2 attempts total
            retry_interval: 0.0,
            retry_rate: 1.0,
        };

        let result = execute_with_retry(
            &params,
            "registry.example.com/repo",
            DockerFlavor::Docker,
            |_cmd| {
                call_count.fetch_add(1, Ordering::SeqCst);
                Err(CliError::Network {
                    message: "always fails".to_string(),
                    metadata: None,
                })
            },
        );

        assert!(result.is_err(), "must error after exhausting retries");
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            2,
            "must have attempted exactly 2 times (1 initial + 1 retry)"
        );
    }

    #[test]
    fn test_execute_with_retry_zero_retry_limit_means_no_retries() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let call_count = AtomicU32::new(0);
        let params = ImageUploadParams {
            app: "myapp".to_string(),
            image_tag: "v1.0".to_string(),
            dockerfile: "Dockerfile".to_string(),
            platforms: vec!["linux/amd64".to_string()],
            work_dir: None,
            login: false,
            retry_limit: 0,
            retry_interval: 0.0,
            retry_rate: 1.0,
        };

        let result = execute_with_retry(
            &params,
            "registry.example.com/repo",
            DockerFlavor::Docker,
            |_cmd| {
                call_count.fetch_add(1, Ordering::SeqCst);
                Err(CliError::Network {
                    message: "fails".to_string(),
                    metadata: None,
                })
            },
        );

        assert!(
            result.is_err(),
            "must error on first failure with retry_limit=0"
        );
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "must have attempted exactly 1 time"
        );
    }

    // ── T-IMU-09: podman two-command execution ──

    #[test]
    fn test_execute_with_retry_podman_executes_both_commands() {
        use std::sync::Mutex;

        let executed_commands: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let params = ImageUploadParams {
            app: "myapp".to_string(),
            image_tag: "v1.0".to_string(),
            dockerfile: "Dockerfile".to_string(),
            platforms: vec!["linux/amd64".to_string()],
            work_dir: None,
            login: false,
            retry_limit: 0,
            retry_interval: 0.0,
            retry_rate: 1.0,
        };

        let result = execute_with_retry(
            &params,
            "registry.example.com/repo",
            DockerFlavor::Podman,
            |cmd| {
                executed_commands.lock().unwrap().push(cmd.args[0].clone());
                Ok(())
            },
        );

        assert!(result.is_ok());
        let cmds = executed_commands.lock().unwrap();
        assert_eq!(cmds.len(), 2, "podman must execute 2 commands");
        assert_eq!(cmds[0], "build");
        assert_eq!(cmds[1], "push");
    }

    // ── T-IMU-10: podman build failure skips push ──

    #[test]
    fn test_execute_with_retry_podman_build_failure_skips_push() {
        use std::sync::Mutex;

        let executed_commands: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let params = ImageUploadParams {
            app: "myapp".to_string(),
            image_tag: "v1.0".to_string(),
            dockerfile: "Dockerfile".to_string(),
            platforms: vec!["linux/amd64".to_string()],
            work_dir: None,
            login: false,
            retry_limit: 0,
            retry_interval: 0.0,
            retry_rate: 1.0,
        };

        let result = execute_with_retry(
            &params,
            "registry.example.com/repo",
            DockerFlavor::Podman,
            |cmd| {
                let verb = cmd.args[0].clone();
                executed_commands.lock().unwrap().push(verb.clone());
                if verb == "build" {
                    Err(CliError::Network {
                        message: "build failed".to_string(),
                        metadata: None,
                    })
                } else {
                    Ok(())
                }
            },
        );

        assert!(result.is_err());
        let cmds = executed_commands.lock().unwrap();
        assert_eq!(cmds.len(), 1, "push must be skipped after build failure");
        assert_eq!(cmds[0], "build");
    }

    // ── Extra: backoff sequence test ──

    #[test]
    fn test_backoff_sequence_matches_go_formula() {
        // Go's CalculateBackoff: interval * math.Pow(rate, float64(attempts))
        // With interval=1.0, rate=2.0:
        //   attempts=0 → 1.0 (initial, no delay)
        //   attempts=1 → 2.0
        //   attempts=2 → 4.0
        //   attempts=3 → 8.0
        let expected = [1.0, 2.0, 4.0, 8.0];
        for (i, &exp) in expected.iter().enumerate() {
            let got = calculate_backoff(i as u32, 1.0, 2.0);
            assert!(
                (got - exp).abs() < f64::EPSILON,
                "backoff({i}) must be {exp}, got {got}"
            );
        }
    }

    // ── T-IMU-11: guard test — CSM operation address resolves ──

    #[test]
    fn test_csm_get_app_v2_resolves_in_bundled_catalogue() {
        use ags_runtime::catalogue::Catalogue;

        let csm_schema = Catalogue::load_bundled("csm").expect("bundled CSM spec must load");

        // The operation we call is `GET /csm/v2/admin/namespaces/{namespace}/apps/{app}`
        // which has x-operationId `csm/admin/apps/v2/get`. Verify it resolves
        // to a resource+method in the live catalogue.
        let target_resource = "apps";
        let target_method = "get";

        let resource = csm_schema
            .resources
            .iter()
            .find(|r| r.name == target_resource);
        assert!(
            resource.is_some(),
            "CSM schema must have a '{target_resource}' resource; \
             available: {:?}",
            csm_schema
                .resources
                .iter()
                .map(|r| &r.name)
                .collect::<Vec<_>>()
        );

        let resource = resource.unwrap();
        let method = resource.methods.iter().find(|m| m.name == target_method);
        assert!(
            method.is_some(),
            "CSM '{target_resource}' must have a '{target_method}' method; \
             available: {:?}",
            resource.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );

        // Verify that the method's default operation has a matching
        // operation ID containing the expected address segments.
        let method = method.unwrap();
        let default_op = method.default_operation();
        assert!(
            default_op.is_some(),
            "CSM {target_resource}/{target_method} must have a default operation"
        );
        let op = default_op.unwrap();
        let op_id = op.id.as_str();
        assert!(
            op_id.contains(target_resource) && op_id.contains(target_method),
            "operation ID must reference {target_resource}/{target_method}; got: {op_id}"
        );
    }

    // ── T-IMU-12a: clamp_backoff_delay ──

    #[test]
    fn test_clamp_backoff_delay_negative_becomes_zero() {
        assert!(
            (clamp_backoff_delay(-5.0)).abs() < f64::EPSILON,
            "negative delay must clamp to 0.0"
        );
    }

    #[test]
    fn test_clamp_backoff_delay_nan_becomes_zero() {
        assert!(
            (clamp_backoff_delay(f64::NAN)).abs() < f64::EPSILON,
            "NaN delay must clamp to 0.0"
        );
    }

    #[test]
    fn test_clamp_backoff_delay_infinite_becomes_cap() {
        let clamped = clamp_backoff_delay(f64::INFINITY);
        assert!(
            (clamped - MAX_BACKOFF_DELAY_SECS).abs() < f64::EPSILON,
            "infinite delay must clamp to cap: {clamped}"
        );
    }

    #[test]
    fn test_clamp_backoff_delay_normal_passes_through() {
        let clamped = clamp_backoff_delay(3.5);
        assert!(
            (clamped - 3.5).abs() < f64::EPSILON,
            "normal delay must pass through unchanged: {clamped}"
        );
    }

    #[test]
    fn test_clamp_backoff_delay_zero_passes_through() {
        let clamped = clamp_backoff_delay(0.0);
        assert!(
            clamped.abs() < f64::EPSILON,
            "zero delay must pass through: {clamped}"
        );
    }

    // ── T-IMU-12b: validate_image_tag ──

    #[test]
    fn test_validate_image_tag_accepts_simple_tag() {
        assert!(validate_image_tag("v1.0").is_ok());
    }

    #[test]
    fn test_validate_image_tag_accepts_complex_tag() {
        assert!(validate_image_tag("my_tag-v2.3.1").is_ok());
    }

    #[test]
    fn test_validate_image_tag_accepts_underscore_start() {
        assert!(validate_image_tag("_internal").is_ok());
    }

    #[test]
    fn test_validate_image_tag_accepts_max_length() {
        let tag = "a".repeat(128);
        assert!(validate_image_tag(&tag).is_ok());
    }

    #[test]
    fn test_validate_image_tag_rejects_empty() {
        let err = validate_image_tag("").unwrap_err();
        assert!(matches!(err, CliError::Usage { .. }));
    }

    #[test]
    fn test_validate_image_tag_rejects_slash() {
        let err = validate_image_tag("v1/evil").unwrap_err();
        assert!(matches!(err, CliError::Usage { .. }));
    }

    #[test]
    fn test_validate_image_tag_rejects_traversal() {
        let err = validate_image_tag("../../admin").unwrap_err();
        assert!(matches!(err, CliError::Usage { .. }));
    }

    #[test]
    fn test_validate_image_tag_rejects_query() {
        let err = validate_image_tag("v1?foo=bar").unwrap_err();
        assert!(matches!(err, CliError::Usage { .. }));
    }

    #[test]
    fn test_validate_image_tag_rejects_too_long() {
        let long = "a".repeat(129);
        let err = validate_image_tag(&long).unwrap_err();
        assert!(matches!(err, CliError::Usage { .. }));
    }

    #[test]
    fn test_validate_image_tag_rejects_leading_dot() {
        let err = validate_image_tag(".v1").unwrap_err();
        assert!(matches!(err, CliError::Usage { .. }));
    }

    #[test]
    fn test_validate_image_tag_rejects_leading_hyphen() {
        let err = validate_image_tag("-v1").unwrap_err();
        assert!(matches!(err, CliError::Usage { .. }));
    }

    // ── T-IMU-12c: UploadPlan gating ──

    #[test]
    fn test_upload_plan_login_true_enables_all_login_steps() {
        let params = ImageUploadParams {
            app: "myapp".to_string(),
            image_tag: "v1.0".to_string(),
            dockerfile: "Dockerfile".to_string(),
            platforms: vec!["linux/amd64".to_string()],
            work_dir: None,
            login: true,
            retry_limit: 0,
            retry_interval: 1.0,
            retry_rate: 2.0,
        };
        let plan = UploadPlan::from_params(&params);
        assert!(
            plan.fetch_credentials,
            "login=true must enable fetch_credentials"
        );
        assert!(plan.docker_login, "login=true must enable docker_login");
        assert!(plan.reuse_runtime, "login=true must enable reuse_runtime");
        assert!(
            plan.check_tag_exists,
            "login=true must enable check_tag_exists"
        );
    }

    #[test]
    fn test_upload_plan_login_false_disables_all_login_steps() {
        let params = ImageUploadParams {
            app: "myapp".to_string(),
            image_tag: "v1.0".to_string(),
            dockerfile: "Dockerfile".to_string(),
            platforms: vec!["linux/amd64".to_string()],
            work_dir: None,
            login: false,
            retry_limit: 0,
            retry_interval: 1.0,
            retry_rate: 2.0,
        };
        let plan = UploadPlan::from_params(&params);
        assert!(
            !plan.fetch_credentials,
            "login=false must disable fetch_credentials"
        );
        assert!(!plan.docker_login, "login=false must disable docker_login");
        assert!(
            !plan.reuse_runtime,
            "login=false must disable reuse_runtime"
        );
        assert!(
            !plan.check_tag_exists,
            "login=false must disable check_tag_exists"
        );
    }

    #[test]
    fn test_upload_plan_reuse_runtime_matches_login() {
        // The key property tested here: when login=true, the plan says
        // to reuse the runtime from step 3 in step 5 rather than
        // creating a new one. This prevents a redundant auth
        // resolution round-trip.
        let login_plan = UploadPlan::from_params(&ImageUploadParams {
            app: "a".to_string(),
            image_tag: "t".to_string(),
            dockerfile: "D".to_string(),
            platforms: vec![],
            work_dir: None,
            login: true,
            retry_limit: 0,
            retry_interval: 0.0,
            retry_rate: 0.0,
        });
        let no_login_plan = UploadPlan::from_params(&ImageUploadParams {
            app: "a".to_string(),
            image_tag: "t".to_string(),
            dockerfile: "D".to_string(),
            platforms: vec![],
            work_dir: None,
            login: false,
            retry_limit: 0,
            retry_interval: 0.0,
            retry_rate: 0.0,
        });
        assert!(login_plan.reuse_runtime);
        assert!(!no_login_plan.reuse_runtime);
    }

    // ── T-IMU-12d: check_tag_exists_with_base ──

    /// HTTP 200 means the tag already exists — the function must return a
    /// `CliError::Usage` error containing the tag name so the user can
    /// choose a different value.
    #[tokio::test]
    async fn test_check_tag_200_returns_usage_error() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("HEAD"))
            .and(wiremock::matchers::path("/v2/ns/repo/manifests/v1.0"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let rr = RegistryRepo {
            registry: "unused".to_string(),
            repo: "ns/repo".to_string(),
        };
        let result = check_tag_exists_with_base(&rr, "v1.0", "user", "tok", &server.uri()).await;

        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(
                    message.contains("already exists"),
                    "error must mention 'already exists': {message}"
                );
                assert!(
                    message.contains("v1.0"),
                    "error must echo the tag name: {message}"
                );
            }
            other => panic!("HTTP 200 must return Usage error, got: {other:?}"),
        }
    }

    /// Non-200 HTTP status (e.g. 404) means the tag does not exist or the
    /// check cannot be performed — the function returns `Ok(())` and the
    /// upload proceeds.
    #[tokio::test]
    async fn test_check_tag_404_returns_ok() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("HEAD"))
            .and(wiremock::matchers::path("/v2/ns/repo/manifests/v1.0"))
            .respond_with(wiremock::ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let rr = RegistryRepo {
            registry: "unused".to_string(),
            repo: "ns/repo".to_string(),
        };
        let result = check_tag_exists_with_base(&rr, "v1.0", "user", "tok", &server.uri()).await;

        assert!(result.is_ok(), "HTTP 404 must return Ok: {result:?}");
    }

    /// A transport error (DNS failure, connection refused, timeout) is
    /// swallowed — the function returns `Ok(())` and emits a notice on
    /// stderr. The check is advisory and must not block the upload.
    #[tokio::test]
    async fn test_check_tag_transport_error_returns_ok() {
        // Use port 1 (privileged, never open) for an immediate
        // connection-refused. Avoids the port-reuse race that occurs
        // when a dropped WireMock server's port is reassigned to a
        // concurrent test's server.
        let uri = "http://127.0.0.1:1".to_string();

        let rr = RegistryRepo {
            registry: "unused".to_string(),
            repo: "ns/repo".to_string(),
        };
        let result = check_tag_exists_with_base(&rr, "v1.0", "user", "tok", &uri).await;

        assert!(
            result.is_ok(),
            "transport error must return Ok (proceed): {result:?}"
        );
    }

    /// When `RegistryRepo.repo` is empty, the function returns `Ok(())`
    /// immediately without making any HTTP call. This covers the edge case
    /// where `split_registry_repo` returns an empty repo path.
    #[tokio::test]
    async fn test_check_tag_empty_repo_returns_ok_immediately() {
        let rr = RegistryRepo {
            registry: "registry.example.com".to_string(),
            repo: String::new(),
        };
        let result =
            check_tag_exists_with_base(&rr, "v1.0", "user", "tok", "http://should-not-be-called")
                .await;

        assert!(
            result.is_ok(),
            "empty repo must return Ok without HTTP call: {result:?}"
        );
    }

    // ── T-IMU-13: backoff delay panic safety ──

    /// `execute_with_retry` must not panic when `retry_interval` is negative.
    ///
    /// `Duration::from_secs_f64` panics on negative, NaN, or infinite input.
    /// The backoff delay must be clamped to a safe range before constructing
    /// the `Duration`. Before the fix, this test panics at the `sleep` call.
    ///
    /// Contract: `execute_with_retry` clamps backoff delay to a finite
    /// non-negative value (RULE-08 — correct error variant, not a panic).
    #[test]
    fn test_execute_with_retry_negative_interval_does_not_panic() {
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let params = ImageUploadParams {
            app: "myapp".to_string(),
            image_tag: "v1.0".to_string(),
            dockerfile: "Dockerfile".to_string(),
            platforms: vec!["linux/amd64".to_string()],
            work_dir: None,
            login: false,
            retry_limit: 1, // must retry once to hit the sleep path
            retry_interval: -1.0,
            retry_rate: 2.0,
        };

        let result = catch_unwind(AssertUnwindSafe(|| {
            execute_with_retry(
                &params,
                "registry.example.com/repo",
                DockerFlavor::Docker,
                |_cmd| {
                    Err(CliError::Network {
                        message: "simulated failure".to_string(),
                        metadata: None,
                    })
                },
            )
        }));

        assert!(
            result.is_ok(),
            "execute_with_retry must not panic with negative interval; \
             the delay must be clamped to a safe value"
        );
        // The inner result should be Err (retries exhausted), not a panic.
        assert!(result.unwrap().is_err());
    }

    /// `execute_with_retry` must not panic when `retry_rate` is NaN.
    ///
    /// `calculate_backoff(1, 1.0, NaN)` produces NaN, which panics in
    /// `Duration::from_secs_f64`. The backoff must be clamped.
    #[test]
    fn test_execute_with_retry_nan_rate_does_not_panic() {
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let params = ImageUploadParams {
            app: "myapp".to_string(),
            image_tag: "v1.0".to_string(),
            dockerfile: "Dockerfile".to_string(),
            platforms: vec!["linux/amd64".to_string()],
            work_dir: None,
            login: false,
            retry_limit: 1,
            retry_interval: 1.0,
            retry_rate: f64::NAN,
        };

        let result = catch_unwind(AssertUnwindSafe(|| {
            execute_with_retry(
                &params,
                "registry.example.com/repo",
                DockerFlavor::Docker,
                |_cmd| {
                    Err(CliError::Network {
                        message: "simulated failure".to_string(),
                        metadata: None,
                    })
                },
            )
        }));

        assert!(
            result.is_ok(),
            "execute_with_retry must not panic with NaN rate"
        );
    }
}
