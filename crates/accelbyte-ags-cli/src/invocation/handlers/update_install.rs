//! Install steps of `ags update --install`, each a small function with
//! injected inputs so the unit tests need no network and no real installer.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::errors::CliError;
use ags_protocol::output_views::InstallMethod;

/// Default installer base URL: the `releases/latest/download` path on the
/// GitHub releases page.
pub(crate) const INSTALLER_BASE_URL: &str =
    "https://github.com/AccelByte/accelbyte-ags-cli/releases/latest/download";

/// Maximum size in bytes of the downloaded installer script.
pub(crate) const INSTALLER_MAX_BYTES: u64 = 1024 * 1024;

/// Time limit for the post-install health check (`<binary> version`).
pub(crate) const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(15);

/// Number of attempts for the Windows remove-and-rename in
/// [`restore_previous_binary`]. Together with [`RESTORE_RETRY_DELAY`] the
/// total bound is about one second.
pub(crate) const RESTORE_RETRY_ATTEMPTS: u32 = 10;

/// Delay between retries in [`restore_previous_binary`] on Windows.
pub(crate) const RESTORE_RETRY_DELAY: Duration = Duration::from_millis(100);

/// A lock file older than this is treated as stale and replaced.
///
/// The holder refreshes the file every [`LOCK_HEARTBEAT_EVERY`], so a
/// file whose mtime is older than this duration has no live holder.
/// The one case this does not cover is a holder killed with SIGKILL or
/// a force quit (second Ctrl-C), which leaves the file until the
/// window expires.
pub(crate) const LOCK_STALE_AFTER: Duration = Duration::from_secs(600);

/// How often the lock holder rewrites the lock file to prove liveness.
/// Each write bumps the mtime, so `LOCK_STALE_AFTER` only fires when
/// no heartbeat has landed for the full window.
pub(crate) const LOCK_HEARTBEAT_EVERY: Duration = Duration::from_secs(60);

/// Name of the lock file in the cache directory.
pub(crate) const LOCK_FILE_NAME: &str = "update-install.lock";

/// The platform-specific installer script file name.
///
/// Returns `accelbyte-ags-cli-installer.ps1` on Windows, else `.sh`.
/// Used by both [`installer_url`] and [`download_installer`] so the
/// destination file name is derived from the platform, never from the URL.
pub(crate) fn installer_script_file_name(os: &str) -> &'static str {
    if os == "windows" {
        "accelbyte-ags-cli-installer.ps1"
    } else {
        "accelbyte-ags-cli-installer.sh"
    }
}

/// Build the full URL for the installer script on the given OS.
///
/// `base` is `INSTALLER_BASE_URL` or an override from
/// `AGS_UPDATE_INSTALLER_URL`. A trailing `/` on `base` is tolerated.
pub(crate) fn installer_url(base: &str, os: &str) -> String {
    let base = base.trim_end_matches('/');
    let script = installer_script_file_name(os);
    format!("{base}/{script}")
}

/// Build the environment variable pairs the installer receives.
///
/// For `Installer`, sets `CARGO_DIST_FORCE_INSTALL_DIR` to the receipt
/// prefix (or falls back to the binary directory for an unmanaged form
/// when the prefix is `None`). For `Manual`, sets
/// `ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL` to `binary_dir`. Both cases set
/// `ACCELBYTE_AGS_CLI_NO_MODIFY_PATH=1` — the single source of truth for
/// suppressing PATH modifications (see [`installer_command`] for how the
/// script is invoked). `Homebrew` returns an empty vector (the handler
/// refuses before this point).
pub(crate) fn installer_env(
    method: InstallMethod,
    receipt_prefix: Option<&Path>,
    binary_dir: &Path,
) -> Vec<(String, String)> {
    match method {
        InstallMethod::Installer => {
            let dir = receipt_prefix.unwrap_or(binary_dir);
            vec![
                (
                    "CARGO_DIST_FORCE_INSTALL_DIR".to_string(),
                    dir.display().to_string(),
                ),
                (
                    "ACCELBYTE_AGS_CLI_NO_MODIFY_PATH".to_string(),
                    "1".to_string(),
                ),
            ]
        }
        InstallMethod::Manual => {
            vec![
                (
                    "ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL".to_string(),
                    binary_dir.display().to_string(),
                ),
                (
                    "ACCELBYTE_AGS_CLI_NO_MODIFY_PATH".to_string(),
                    "1".to_string(),
                ),
            ]
        }
        InstallMethod::Homebrew => vec![],
    }
}

/// Returns `true` when the given URL is safe for the
/// `AGS_UPDATE_INSTALLER_URL` override: either `https` (any host) or
/// `http` on a loopback address (`127.0.0.1`, `::1`, `localhost`).
/// All other schemes or non-loopback `http` hosts return `false`.
///
/// This validation is stricter than the `AGS_UPDATE_CHECK_URL` hook
/// because the installer URL names a script that gets executed — a
/// plain-HTTP fetch to a remote host would let a network-position
/// attacker substitute an arbitrary script.
///
/// This check must never be loosened to accept arbitrary hostnames,
/// only exact loopback literals, because it gates whether an
/// executable script may be fetched over plain HTTP.
pub(crate) fn installer_url_override_allows_plain_http(url: &str) -> bool {
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return false,
    };
    match parsed.scheme() {
        "https" => true,
        "http" => match parsed.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            Some(url::Host::Domain(d)) => d == "localhost",
            None => false,
        },
        _ => false,
    }
}

/// Retry a fallible I/O operation up to `attempts` times, sleeping `delay`
/// between each attempt. Returns the first `Ok` immediately; on the final
/// failure returns that last error unchanged so the caller's message still
/// names the real OS error.
fn retry_transient_io<T>(
    attempts: u32,
    delay: Duration,
    mut op: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    let mut last_err = None;
    for _ in 0..attempts {
        match op() {
            Ok(val) => return Ok(val),
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(delay);
            }
        }
    }
    Err(last_err.expect("attempts must be >= 1"))
}

/// The path where the previous binary is preserved during an upgrade.
pub(crate) fn previous_binary_path(binary_path: &Path) -> PathBuf {
    let mut name = binary_path.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    binary_path.with_file_name(name)
}

/// Preserve the current binary as `<binary_path>.old`.
///
/// Renames the running binary out of the way so the installer can write a
/// new file at the original path. On all platforms the running process
/// keeps executing from the old inode; on Linux a `copy` would leave the
/// original path locked (`ETXTBSY`) against writes while the binary is
/// executing, so `rename` is used unconditionally.
/// Removes a stale `.old` first if one exists.
pub(crate) fn preserve_previous_binary(binary_path: &Path) -> std::io::Result<PathBuf> {
    let old = previous_binary_path(binary_path);
    // Remove a stale .old first if one exists.
    if old.exists() {
        let _ = std::fs::remove_file(&old);
    }
    std::fs::rename(binary_path, &old)?;
    Ok(old)
}

/// Restore the previous binary from `<binary_path>.old`.
///
/// On Windows, removes the new binary (if any) first because
/// `std::fs::rename` does not atomically replace on Windows. A step
/// cancelled by Ctrl-C drops a child that `kill_on_drop` has only begun
/// terminating, and Windows refuses to remove or replace a file whose
/// image or handle that dying process still holds. The remove and rename
/// each retry for about one second (bounded by [`RESTORE_RETRY_ATTEMPTS`]
/// and [`RESTORE_RETRY_DELAY`]) before reporting failure. On Unix,
/// `rename` atomically replaces the target if it exists on the same
/// filesystem and there is no sharing-violation concept, so no retry and
/// no explicit remove is needed.
pub(crate) fn restore_previous_binary(binary_path: &Path) -> std::io::Result<()> {
    let old = previous_binary_path(binary_path);
    if cfg!(windows) && binary_path.exists() {
        retry_transient_io(RESTORE_RETRY_ATTEMPTS, RESTORE_RETRY_DELAY, || {
            std::fs::remove_file(binary_path)
        })?;
    }
    if cfg!(windows) {
        retry_transient_io(RESTORE_RETRY_ATTEMPTS, RESTORE_RETRY_DELAY, || {
            std::fs::rename(&old, binary_path)
        })?;
    } else {
        std::fs::rename(&old, binary_path)?;
    }
    Ok(())
}

/// Remove `<current_exe>.old` if present; ignore every error; never print.
/// Called at the start of every `ags` invocation on Windows to clean up
/// the leftover from an in-place upgrade. The call site is gated by
/// `cfg!(windows)`; on other platforms `.old` is either removed by the
/// upgrade on success or kept as the user's rollback copy after a force
/// quit.
pub(crate) fn cleanup_previous_binary(current_exe: &Path) {
    let old = previous_binary_path(current_exe);
    let _ = std::fs::remove_file(old);
}

/// Download the installer script from `url` into `dir`.
///
/// Issues a GET request; a non-success status is `CliError::Network`.
/// Reads the body with `Response::chunk()` and stops with
/// `CliError::Network` as soon as the running total exceeds `max_bytes`.
/// Writes to `dir/<script-name>` where the script name is derived from the
/// running platform via [`installer_script_file_name`], never from the URL.
pub(crate) async fn download_installer(
    client: &reqwest::Client,
    url: &str,
    dir: &Path,
    max_bytes: u64,
) -> Result<PathBuf, CliError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| CliError::Network {
            message: format!("Failed to download installer from {url}: {e}"),
            metadata: None,
        })?;

    if !response.status().is_success() {
        return Err(CliError::Network {
            message: format!(
                "Failed to download installer from {url}: HTTP {}",
                response.status()
            ),
            metadata: None,
        });
    }

    // Derive the script file name from the running platform, not the URL,
    // so a redirect or override cannot change the destination name.
    let file_name = installer_script_file_name(std::env::consts::OS);
    let dest = dir.join(file_name);

    let mut total: u64 = 0;
    let mut body = Vec::new();
    let mut stream = response;
    while let Some(chunk) = stream.chunk().await.map_err(|e| CliError::Network {
        message: format!("Error reading installer body from {url}: {e}"),
        metadata: None,
    })? {
        total += chunk.len() as u64;
        if total > max_bytes {
            return Err(CliError::Network {
                message: format!("Installer script from {url} exceeds the {max_bytes}-byte limit"),
                metadata: None,
            });
        }
        body.extend_from_slice(&chunk);
    }

    std::fs::write(&dest, &body).map_err(|e| {
        CliError::Internal(anyhow::anyhow!(
            "Failed to write installer to {}: {e}",
            dest.display()
        ))
    })?;

    Ok(dest)
}

/// Build the reqwest client for installer downloads.
///
/// Three guards protect the download from scheme-downgrade attacks, each
/// covering a different hop:
///
/// 1. **Override validation** — when `AGS_UPDATE_INSTALLER_URL` is set,
///    [`installer_url_override_allows_plain_http`] rejects any non-loopback
///    plain-HTTP URL *before the client is ever constructed*. This is the
///    gate for the initial URL on the override path.
/// 2. **`https_only`** — when `true` (the production path, where there is
///    no override), reqwest refuses to send the initial request to a
///    non-HTTPS URL. On the override path `false` is passed because the
///    override has already been validated by guard 1.
/// 3. **Redirect policy** — the custom `Policy` refuses to follow any
///    redirect whose target scheme is not `https`, regardless of the
///    `https_only` setting. This covers every hop after the initial
///    request on every path.
///
/// Also configures `timeout` and the `User-Agent: accelbyte-ags-cli`
/// header.
pub(crate) fn build_installer_client(
    timeout: Duration,
    https_only: bool,
) -> Result<reqwest::Client, CliError> {
    use reqwest::redirect::Policy;

    let policy = Policy::custom(|attempt| {
        if attempt.url().scheme() == "https" {
            attempt.follow()
        } else {
            attempt.stop()
        }
    });

    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent("accelbyte-ags-cli")
        .https_only(https_only)
        .redirect(policy)
        .build()
        .map_err(|e| CliError::Network {
            message: format!("Failed to build HTTP client: {e}"),
            metadata: None,
        })
}

/// Build the `Command` to run the installer script.
///
/// On Windows: `powershell -NoProfile -ExecutionPolicy Bypass -File <script_path>`.
/// Elsewhere: `sh <script_path>`.
/// The script path is one argument; nothing downloaded is ever interpolated
/// into a shell string. The command inherits the environment and adds `env`;
/// see [`installer_env`] for the `ACCELBYTE_AGS_CLI_NO_MODIFY_PATH=1` variable
/// that governs PATH modification (the single source of truth — both shipped
/// installers read it). `stdout` piped, `stderr` inherited.
pub(crate) fn installer_command(
    script_path: &Path,
    env: &[(String, String)],
) -> std::process::Command {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("powershell");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
        c.arg(script_path);
        c
    } else {
        let mut c = std::process::Command::new("sh");
        c.arg(script_path);
        c
    };
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::inherit());
    // The installer never reads the user's terminal; a script that tries
    // to prompt gets end-of-file at once instead of blocking.
    cmd.stdin(std::process::Stdio::null());
    cmd
}

/// Run the installer subprocess.
///
/// Spawns the command, copies the child's stdout to this process's stderr
/// line by line as it arrives, and waits. A spawn failure is
/// `CliError::Internal` naming the program.
pub(crate) async fn run_installer(
    command: std::process::Command,
) -> Result<std::process::ExitStatus, CliError> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut child = tokio::process::Command::from(command)
        // On cancellation the future is dropped, which drops the Child.
        // kill_on_drop ensures the installer subprocess is killed rather
        // than orphaned (same as the version-probe child).
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            CliError::Internal(anyhow::anyhow!("Failed to spawn installer process: {e}"))
        })?;

    // Copy the child's stdout to this process's stderr line by line.
    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            crate::frontend::write_stderr_line(&line);
        }
    }

    let status = child.wait().await.map_err(|e| {
        CliError::Internal(anyhow::anyhow!("Failed to wait for installer process: {e}"))
    })?;
    Ok(status)
}

/// Parse the version from the output of `ags version`.
///
/// Expects a line like `ags 0.5.2 (workflow protocol 1.0.0)` and returns
/// the first semver token after `ags `. Used by unit tests; the production
/// path (`verify_installed_version`) inlines the same logic.
#[cfg(test)]
pub(crate) fn parse_reported_version(stdout: &str) -> Option<semver::Version> {
    let first_line = stdout.lines().next()?;
    let after_ags = first_line.strip_prefix("ags ")?;
    let token = after_ags.split_whitespace().next()?;
    token.parse().ok()
}

/// Accept the installed version if it is newer than `previous` and at
/// least `latest`.
///
/// Returns `Ok(version)` when `previous < reported` and
/// `reported >= latest`; `Err` with a message naming the reported text
/// and the two bounds otherwise.
pub(crate) fn accept_installed_version(
    reported: &str,
    previous: &semver::Version,
    latest: &semver::Version,
) -> Result<semver::Version, String> {
    let version: semver::Version = reported.parse().map_err(|_| {
        format!("The new binary reported {reported:?}, which is not a valid version")
    })?;
    if version <= *previous {
        return Err(format!(
            "The new binary reported {reported}, which is not newer than the previous version {previous}"
        ));
    }
    if version < *latest {
        return Err(format!(
            "The new binary reported {reported}, which is older than the latest release {latest}"
        ));
    }
    Ok(version)
}

/// Run `<binary_path> version` and verify the reported version.
///
/// Runs with `AGS_NO_UPDATE_CHECK=1` in its environment, both streams
/// piped, under `tokio::time::timeout(timeout, ...)`. On timeout kills
/// the child and returns `CliError::Internal`.
pub(crate) async fn verify_installed_version(
    binary_path: &Path,
    previous: &semver::Version,
    latest: &semver::Version,
    timeout: Duration,
) -> Result<semver::Version, CliError> {
    let child = tokio::process::Command::new(binary_path)
        .arg("version")
        .env("AGS_NO_UPDATE_CHECK", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // The version probe never needs the user's terminal.
        .stdin(std::process::Stdio::null())
        // On timeout the future is dropped, which drops the Child.
        // Without kill_on_drop the child process would be orphaned
        // (tokio does NOT kill on drop by default).
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            CliError::Internal(anyhow::anyhow!(
                "Failed to run {} version: {e}",
                binary_path.display()
            ))
        })?;

    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Err(CliError::Internal(anyhow::anyhow!(
                "Failed to wait for {} version: {e}",
                binary_path.display()
            )));
        }
        Err(_) => {
            // child is consumed by wait_with_output; kill_on_drop(true)
            // ensures the process is killed when the future is dropped.
            return Err(CliError::Internal(anyhow::anyhow!(
                "The new binary at {} did not answer `version` within {} seconds",
                binary_path.display(),
                timeout.as_secs()
            )));
        }
    };

    // The new binary must exit 0; a non-zero status means it is broken
    // even if it happens to print a valid version string.
    if !output.status.success() {
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string());
        return Err(CliError::Internal(anyhow::anyhow!(
            "The new binary at {} exited with status {code} on `version`",
            binary_path.display()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    accept_installed_version(
        stdout
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("ags "))
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or(""),
        previous,
        latest,
    )
    .map_err(|msg| CliError::Internal(anyhow::anyhow!("{msg}")))
}

/// Exclusive lock for `ags update --install`.
///
/// Created exclusively in the CLI's cache directory. A second invocation
/// while the lock exists and is younger than the stale threshold exits
/// with a usage error. A lock older than the threshold is treated as stale
/// and replaced. A background heartbeat task rewrites the lock file at
/// a configurable interval, bumping its mtime so the staleness test
/// doubles as a liveness test. `Drop` aborts the heartbeat task and
/// removes the file, ignoring errors.
#[derive(Debug)]
pub(crate) struct InstallLock {
    path: PathBuf,
    /// Aborting this handle stops the heartbeat task. The task is
    /// spawned by `acquire_with_heartbeat`; for the synchronous
    /// `acquire` (used in tests and by the stale-lock path), this is
    /// `None` and no heartbeat runs.
    _heartbeat: Option<tokio::task::AbortHandle>,
}

impl InstallLock {
    /// Acquire the lock. Creates `<cache_dir>/update-install.lock`
    /// exclusively; on `AlreadyExists`, if the file's modified time is
    /// older than `stale_after`, renames it atomically to a unique
    /// temporary name — exactly one concurrent process wins the rename;
    /// a loser's rename fails and it reports a concurrent install. The
    /// winner removes the renamed file (best effort) and tries
    /// `create_new` once more; a second `AlreadyExists` is also a usage
    /// error. Writes the process id into the file; this identifies the
    /// holder for a human inspecting the file and is what the heartbeat
    /// rewrites.
    pub(crate) fn acquire(
        cache_dir: &Path,
        stale_after: Duration,
    ) -> Result<InstallLock, CliError> {
        use std::fs::OpenOptions;
        use std::io::Write;

        let _ = std::fs::create_dir_all(cache_dir);
        let lock_path = cache_dir.join(LOCK_FILE_NAME);

        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut f) => {
                let _ = write!(f, "{}", std::process::id());
                Ok(InstallLock {
                    path: lock_path,
                    _heartbeat: None,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Check if the existing lock is stale.
                let is_stale = std::fs::metadata(&lock_path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > stale_after);

                if is_stale {
                    // Rename the stale file atomically; a concurrent process
                    // performing the same check will fail on rename and report
                    // a concurrent install rather than both claiming the lock.
                    let nanos = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos();
                    let stale_name =
                        format!("{}.stale-{}-{}", LOCK_FILE_NAME, std::process::id(), nanos,);
                    let stale_path = cache_dir.join(&stale_name);

                    if std::fs::rename(&lock_path, &stale_path).is_ok() {
                        // We won the rename; clean up (best effort).
                        let _ = std::fs::remove_file(&stale_path);
                        // Try once more.
                        match OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&lock_path)
                        {
                            Ok(mut f) => {
                                let _ = write!(f, "{}", std::process::id());
                                Ok(InstallLock {
                                    path: lock_path,
                                    _heartbeat: None,
                                })
                            }
                            Err(_) => Err(CliError::Usage {
                                message: format!(
                                    "Another ags update --install is running (lock: {}).",
                                    lock_path.display()
                                ),
                                metadata: None,
                            }),
                        }
                    } else {
                        // Another process renamed the stale lock first.
                        Err(CliError::Usage {
                            message: format!(
                                "Another ags update --install is running (lock: {}).",
                                lock_path.display()
                            ),
                            metadata: None,
                        })
                    }
                } else {
                    Err(CliError::Usage {
                        message: format!(
                            "Another ags update --install is running (lock: {}).",
                            lock_path.display()
                        ),
                        metadata: None,
                    })
                }
            }
            Err(e) => Err(CliError::Internal(anyhow::anyhow!(
                "Failed to create lock file {}: {e}",
                lock_path.display()
            ))),
        }
    }

    /// Acquire the lock and start a heartbeat task that rewrites the
    /// lock file every `every`, bumping its mtime so the staleness
    /// window in [`acquire`] doubles as a liveness test.
    ///
    /// `acquire` delegates to this with [`LOCK_HEARTBEAT_EVERY`] on the
    /// production path. Tests may pass a shorter interval.
    pub(crate) async fn acquire_with_heartbeat(
        lock_path: PathBuf,
        stale_after: Duration,
        every: Duration,
    ) -> Result<InstallLock, CliError> {
        let cache_dir = lock_path.parent().unwrap_or(std::path::Path::new("."));
        let mut lock = Self::acquire(cache_dir, stale_after)?;

        let heartbeat_path = lock.path.clone();
        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                // Rewrite the lock file with the current PID; this
                // bumps the mtime.  A failed write is benign — the next
                // beat will try again, and if the file is truly gone
                // the lock is already released.
                if std::fs::write(&heartbeat_path, format!("{}", std::process::id())).is_err() {
                    break;
                }
            }
        });

        lock._heartbeat = Some(handle.abort_handle());
        Ok(lock)
    }

    /// The path of the lock file.
    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        // Abort the heartbeat task before removing the file so a beat
        // in flight cannot recreate a file we just deleted.
        if let Some(handle) = self._heartbeat.take() {
            handle.abort();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[test]
    fn installer_url_picks_script_for_platform() {
        let base = "https://example.com/releases/latest/download";
        assert_eq!(
            installer_url(base, "windows"),
            format!("{base}/accelbyte-ags-cli-installer.ps1")
        );
        assert_eq!(
            installer_url(base, "macos"),
            format!("{base}/accelbyte-ags-cli-installer.sh")
        );
        assert_eq!(
            installer_url(base, "linux"),
            format!("{base}/accelbyte-ags-cli-installer.sh")
        );

        // Trailing slash on base is tolerated.
        let base_slash = "https://example.com/releases/latest/download/";
        assert_eq!(
            installer_url(base_slash, "windows"),
            format!("{base}/accelbyte-ags-cli-installer.ps1")
        );
    }

    #[test]
    fn installer_env_for_installer_method_points_at_receipt_prefix() {
        let prefix = Path::new("/home/user/.cargo");
        let binary_dir = Path::new("/home/user/.cargo/bin");
        let env = installer_env(InstallMethod::Installer, Some(prefix), binary_dir);

        let as_map: std::collections::HashMap<&str, &str> =
            env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

        assert_eq!(
            as_map.get("CARGO_DIST_FORCE_INSTALL_DIR"),
            Some(&"/home/user/.cargo"),
            "installer method must point CARGO_DIST_FORCE_INSTALL_DIR at the receipt prefix"
        );
        assert_eq!(as_map.get("ACCELBYTE_AGS_CLI_NO_MODIFY_PATH"), Some(&"1"));
        assert!(
            !as_map.contains_key("ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL"),
            "installer method must not set the unmanaged variable"
        );
    }

    #[test]
    fn installer_env_for_manual_method_is_unmanaged_flat() {
        let binary_dir = Path::new("/usr/local/bin");
        let env = installer_env(InstallMethod::Manual, None, binary_dir);

        let as_map: std::collections::HashMap<&str, &str> =
            env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

        assert_eq!(
            as_map.get("ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL"),
            Some(&"/usr/local/bin"),
            "manual method must set ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL to the binary directory"
        );
        assert_eq!(as_map.get("ACCELBYTE_AGS_CLI_NO_MODIFY_PATH"), Some(&"1"));
        assert!(
            !as_map.contains_key("CARGO_DIST_FORCE_INSTALL_DIR"),
            "manual method must not set the force-dir variable"
        );
    }

    #[test]
    fn preserve_and_restore_previous_binary_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("ags");
        let original_bytes = b"original-ags-binary-content";
        fs::write(&binary, original_bytes).unwrap();

        // Preserve: .old must exist after.
        let old_path = preserve_previous_binary(&binary).unwrap();
        assert!(old_path.exists(), ".old must exist after preserve");
        assert_eq!(
            old_path,
            previous_binary_path(&binary),
            "preserve must return the .old path"
        );

        // The code uses rename on all platforms (not copy), so the
        // original path is always removed.
        assert!(
            !binary.exists(),
            "rename must remove the original on every platform"
        );

        // Restore: original path must have the original bytes, no .old.
        restore_previous_binary(&binary).unwrap();
        assert_eq!(
            fs::read(&binary).unwrap(),
            original_bytes,
            "restored binary must have the original bytes"
        );
        assert!(!old_path.exists(), ".old must be gone after restore");
    }

    #[test]
    fn cleanup_previous_binary_removes_old_and_ignores_absence() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("ags");
        let old = dir.path().join("ags.old");
        fs::write(&old, b"stale").unwrap();

        cleanup_previous_binary(&binary);
        assert!(!old.exists(), ".old must be removed");

        // Calling again when absent must not error.
        cleanup_previous_binary(&binary);
    }

    #[test]
    fn verify_installed_version_parses_version_line() {
        assert_eq!(
            parse_reported_version("ags 0.5.2 (workflow protocol 1.0.0)"),
            Some(semver::Version::new(0, 5, 2))
        );
        assert_eq!(
            parse_reported_version("ags 1.0.0"),
            Some(semver::Version::new(1, 0, 0))
        );
        assert_eq!(parse_reported_version("not ags output"), None);
        assert_eq!(parse_reported_version("ags garbage"), None);
        assert_eq!(parse_reported_version(""), None);
    }

    #[test]
    fn installed_version_must_be_newer_than_previous_and_at_least_latest() {
        let previous = semver::Version::new(0, 5, 1);
        let latest = semver::Version::new(0, 5, 2);

        // 0.5.2 == latest, > previous -> accepted.
        assert!(accept_installed_version("0.5.2", &previous, &latest).is_ok());

        // 0.5.3 > latest, > previous -> accepted (a release between check and download).
        assert!(accept_installed_version("0.5.3", &previous, &latest).is_ok());

        // 0.5.1 == previous -> rejected (not newer).
        let err = accept_installed_version("0.5.1", &previous, &latest).unwrap_err();
        assert!(
            err.contains("0.5.1"),
            "error must name the rejected version: {err}"
        );

        // 0.5.0 < previous -> rejected.
        let err = accept_installed_version("0.5.0", &previous, &latest).unwrap_err();
        assert!(
            err.contains("0.5.0"),
            "error must name the rejected version: {err}"
        );

        // garbage -> rejected (unparseable).
        let err = accept_installed_version("garbage", &previous, &latest).unwrap_err();
        assert!(
            err.contains("garbage"),
            "error must name the rejected text: {err}"
        );
    }

    /// The synchronous `acquire` path: exclusivity, drop-release, and
    /// staleness replacement.  The heartbeat-based liveness invariant
    /// (a live holder refreshes the file so it never looks stale) is
    /// tested separately via `acquire_with_heartbeat` in the async tests.
    #[test]
    fn install_lock_is_exclusive_and_stale_after_ten_minutes() {
        let dir = tempfile::tempdir().unwrap();
        let stale_after = Duration::from_secs(600);

        // First acquire succeeds.
        let lock1 = InstallLock::acquire(dir.path(), stale_after).unwrap();
        assert!(lock1.path().exists(), "lock file must exist");

        // Second acquire while the first lives returns an error naming the path.
        let err = InstallLock::acquire(dir.path(), stale_after).unwrap_err();
        match &err {
            CliError::Usage { message, .. } => {
                assert!(
                    message.contains(&lock1.path().display().to_string()),
                    "error must name the lock path: {message}"
                );
            }
            other => panic!("expected Usage error, got {other:?}"),
        }

        // After drop, acquire succeeds.
        drop(lock1);
        let lock2 = InstallLock::acquire(dir.path(), stale_after).unwrap();
        drop(lock2);

        // A lock file older than stale_after is replaced.
        let lock_path = dir.path().join(LOCK_FILE_NAME);
        {
            let mut f = fs::File::create(&lock_path).unwrap();
            f.write_all(b"stale-pid").unwrap();
            // Set modified time to 11 minutes ago (filetime-free).
            let eleven_min_ago = std::time::SystemTime::now() - Duration::from_secs(660);
            f.set_modified(eleven_min_ago).unwrap();
        }

        let lock3 = InstallLock::acquire(dir.path(), stale_after).unwrap();
        assert!(lock3.path().exists(), "stale lock must be replaced");
        drop(lock3);

        // No stale-* temporary remains after the stale lock was replaced.
        let stale_remains: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{}.stale-", LOCK_FILE_NAME))
            })
            .collect();
        assert!(
            stale_remains.is_empty(),
            "stale-* temporary must be cleaned up; found: {:?}",
            stale_remains
                .iter()
                .map(|e| e.file_name())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn installer_url_override_accepts_https_anywhere_and_http_on_loopback_only() {
        // HTTPS anywhere: always accepted.
        assert!(
            super::installer_url_override_allows_plain_http("https://example.com/x"),
            "https to a remote host must be accepted"
        );

        // HTTP to loopback addresses: accepted.
        assert!(
            super::installer_url_override_allows_plain_http("http://127.0.0.1:8080/x"),
            "http to 127.0.0.1 must be accepted"
        );
        assert!(
            super::installer_url_override_allows_plain_http("http://localhost/x"),
            "http to localhost must be accepted"
        );
        assert!(
            super::installer_url_override_allows_plain_http("http://[::1]/x"),
            "http to [::1] must be accepted"
        );

        // HTTP to a remote host: rejected.
        assert!(
            !super::installer_url_override_allows_plain_http("http://example.com/x"),
            "http to a remote host must be rejected"
        );

        // Non-HTTP/HTTPS schemes: rejected.
        assert!(
            !super::installer_url_override_allows_plain_http("ftp://127.0.0.1/x"),
            "ftp scheme must be rejected even for loopback"
        );
    }

    #[test]
    fn installer_script_file_name_matches_installer_url() {
        for os in &["windows", "macos", "linux"] {
            let name = super::installer_script_file_name(os);
            let url = super::installer_url("https://example.com", os);
            assert!(
                url.ends_with(name),
                "installer_url for {os} must end with the script file name {name}; got {url}"
            );
        }
    }

    #[test]
    fn installer_base_url_is_https() {
        assert!(
            INSTALLER_BASE_URL.starts_with("https://"),
            "INSTALLER_BASE_URL must start with https://; got: {INSTALLER_BASE_URL}"
        );
    }

    /// The heartbeat task refreshes the lock file's mtime while the lock
    /// is held, so the mtime test in `acquire` becomes a liveness test.
    /// Red before the heartbeat exists because `acquire_with_heartbeat`
    /// is not yet defined.
    #[tokio::test]
    async fn install_lock_heartbeat_refreshes_the_lock_file_mtime() {
        let dir = tempfile::tempdir().unwrap();
        // A short heartbeat interval for the test — 50 ms is enough to
        // see several beats in a 300 ms window without being flaky.
        let heartbeat = Duration::from_millis(50);
        let stale_after = Duration::from_secs(600);

        let lock = InstallLock::acquire_with_heartbeat(
            dir.path().join(LOCK_FILE_NAME),
            stale_after,
            heartbeat,
        )
        .await
        .unwrap();

        let initial_mtime = fs::metadata(lock.path())
            .and_then(|m| m.modified())
            .unwrap();

        // Wait long enough for several heartbeats.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let later_mtime = fs::metadata(lock.path())
            .and_then(|m| m.modified())
            .unwrap();

        assert!(
            later_mtime > initial_mtime,
            "heartbeat must advance the lock file's mtime; \
             initial: {initial_mtime:?}, later: {later_mtime:?}"
        );

        drop(lock);
    }

    #[test]
    fn retry_transient_io_returns_on_first_success() {
        use std::cell::Cell;
        let count = Cell::new(0u32);
        let result = retry_transient_io(5, Duration::from_millis(1), || {
            count.set(count.get() + 1);
            Ok(42)
        });
        assert_eq!(result.unwrap(), 42);
        assert_eq!(
            count.get(),
            1,
            "must call the operation exactly once on immediate success"
        );
    }

    #[test]
    fn retry_transient_io_succeeds_after_transient_failures() {
        use std::cell::Cell;
        // Use a 1 ms delay instead of the production constant — the
        // production constants are asserted separately in
        // `restore_retry_bound_is_about_one_second`.
        let count = Cell::new(0u32);
        let result = retry_transient_io(5, Duration::from_millis(1), || {
            let n = count.get() + 1;
            count.set(n);
            if n < 3 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "sharing violation stub",
                ))
            } else {
                Ok("recovered")
            }
        });
        assert_eq!(result.unwrap(), "recovered");
        assert_eq!(
            count.get(),
            3,
            "must call the operation exactly three times"
        );
    }

    #[test]
    fn retry_transient_io_surfaces_the_last_error() {
        use std::cell::Cell;
        let count = Cell::new(0u32);
        let attempts = 4u32;
        let result: std::io::Result<()> =
            retry_transient_io(attempts, Duration::from_millis(1), || {
                count.set(count.get() + 1);
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "persistent sharing violation",
                ))
            });
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            count.get(),
            attempts,
            "must call the operation exactly {attempts} times"
        );
    }

    #[test]
    fn restore_retry_bound_is_about_one_second() {
        let total_ms = RESTORE_RETRY_ATTEMPTS as u128 * RESTORE_RETRY_DELAY.as_millis();
        assert!(
            (1000..=2000).contains(&total_ms),
            "the retry bound must be between 1000 ms and 2000 ms; got {total_ms} ms"
        );
    }

    /// A live holder whose heartbeat keeps the lock fresh must not have
    /// its lock stolen by a second acquire, even when the stale window
    /// is short.
    ///
    /// Timing: the stale window is 200 ms, the heartbeat is 50 ms, and
    /// the test waits 300 ms before the second acquire — well past the
    /// stale window, but the heartbeat keeps the mtime fresh.  These
    /// values are not flaky because the heartbeat (50 ms) fires several
    /// times inside the stale window (200 ms), and the tokio runtime's
    /// scheduling jitter on a healthy machine is well under 50 ms.
    #[tokio::test]
    async fn install_lock_is_not_stolen_from_a_live_holder() {
        let dir = tempfile::tempdir().unwrap();
        let heartbeat = Duration::from_millis(50);
        let stale_after = Duration::from_millis(200);

        let lock = InstallLock::acquire_with_heartbeat(
            dir.path().join(LOCK_FILE_NAME),
            stale_after,
            heartbeat,
        )
        .await
        .unwrap();

        // Wait past the stale window.
        tokio::time::sleep(Duration::from_millis(300)).await;

        // The second acquire must fail because the heartbeat kept the
        // lock fresh.
        let err = InstallLock::acquire(dir.path(), stale_after).unwrap_err();
        match &err {
            CliError::Usage { message, .. } => {
                assert!(
                    message.contains("Another ags update --install is running"),
                    "error must say another install is running: {message}"
                );
            }
            other => panic!("expected Usage error, got {other:?}"),
        }

        drop(lock);
    }

    /// Dropping the lock must abort the heartbeat task and remove the
    /// file, so a subsequent acquire succeeds immediately.  Red if
    /// `Drop` stops removing the file.
    #[tokio::test]
    async fn install_lock_released_on_drop_is_acquirable_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let heartbeat = Duration::from_millis(50);
        let stale_after = Duration::from_secs(600);

        let lock = InstallLock::acquire_with_heartbeat(
            dir.path().join(LOCK_FILE_NAME),
            stale_after,
            heartbeat,
        )
        .await
        .unwrap();

        // Let a few heartbeats fire.
        tokio::time::sleep(Duration::from_millis(150)).await;

        drop(lock);

        // The lock file must be gone.
        let lock_path = dir.path().join(LOCK_FILE_NAME);
        assert!(
            !lock_path.exists(),
            "lock file must be removed on drop; still present at {}",
            lock_path.display()
        );

        // A fresh acquire must succeed immediately.
        let lock2 = InstallLock::acquire(dir.path(), stale_after).unwrap();
        assert!(lock2.path().exists(), "re-acquired lock file must exist");
        drop(lock2);
    }
}
