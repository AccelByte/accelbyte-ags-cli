//! Update command handler: check for a newer release, with optional in-place
//! upgrade via `--install`.

use crate::errors::CliError;
use crate::frontend;
use crate::invocation::builder;
use crate::invocation::clap_helpers;
use crate::invocation::context::FrontendContext;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::handlers::update_install;
use crate::invocation::InvocationOutcome;
use ags_protocol::output::{CommandOutput, UpdateInstallAction, UpdateInstallOutput, UpdateOutput};
use ags_runtime::runtime::update_check;
use ags_runtime::runtime::update_check::install_method;

/// Handle the `ags update` command.
pub(crate) async fn handle_update(
    args: &[String],
    flags: &GlobalFlags,
    frontend: &mut dyn frontend::Frontend,
    ctx: &FrontendContext,
) -> Result<InvocationOutcome, CliError> {
    let mut command = builder::build_update_command();
    let argv = clap_helpers::build_argv("update", args);

    let m = match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
        Ok(m) => m,
        Err(error) => return clap_helpers::outcome_from_clap_error(error),
    };

    let install = m.get_flag("install");

    if install {
        handle_install(flags, frontend, ctx).await
    } else {
        handle_check(flags, frontend).await
    }
}

/// The existing `ags update` check flow — unchanged from the pre-install code.
async fn handle_check(
    flags: &GlobalFlags,
    frontend: &mut dyn frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let check_url = update_check::latest_release_url();
    let api_url = update_check::api_url();

    // Dry-run: render the request preview and return before any network or file
    // access. The preview is a GET of the release endpoint with the User-Agent
    // header and no body — the same shape `CommandOutput::DryRun` renders.
    if flags.is_dry_run {
        let dry_run = ags_protocol::result::DryRunResult {
            http_method: ags_protocol::catalogue::HttpMethod::Get,
            url: api_url,
            headers: vec![("User-Agent".to_string(), "accelbyte-ags-cli".to_string())],
            query: vec![],
            body: None,
        };
        frontend.render(&CommandOutput::DryRun(dry_run))?;
        return Ok(InvocationOutcome::Complete);
    }

    // Build the HTTP client with the same default timeout the API client uses.
    let timeout = std::time::Duration::from_secs(
        flags
            .timeout
            .unwrap_or(ags_runtime::runtime::dispatch::http::DEFAULT_TIMEOUT_SECS),
    );
    let client = update_check::build_client_with_timeout(timeout).ok_or_else(|| {
        CliError::Network {
            message: format!(
                "Could not check for updates: failed to build HTTP client (GET {api_url}). See {check_url}"
            ),
            metadata: None,
        }
    })?;

    let result = update_check::check_now(&client)
        .await
        .map_err(|e| CliError::Network {
            message: format!("Could not check for updates: {e} (GET {api_url}). See {check_url}"),
            metadata: None,
        })?;

    let (method, exe_path) = install_method::install_method();
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let upgrade_cmd = install_method::upgrade_command(method, os);
    let archive = install_method::download_archive(os, arch, cfg!(target_env = "musl"));

    let release_url = if result.is_newer {
        update_check::release_url(&result.latest)
    } else {
        update_check::latest_release_url().to_string()
    };

    let output = UpdateOutput {
        current: result.current,
        latest: result.latest,
        update_available: result.is_newer,
        install_method: method,
        binary_path: exe_path.to_string_lossy().to_string(),
        upgrade_command: upgrade_cmd,
        download_archive: archive,
        release_url,
    };

    frontend.render(&CommandOutput::Update(output))?;
    Ok(InvocationOutcome::Complete)
}

/// Refuse the install if the binary was installed with Homebrew.
fn refuse_homebrew(method: ags_protocol::output::InstallMethod) -> Result<(), CliError> {
    if method == ags_protocol::output::InstallMethod::Homebrew {
        return Err(CliError::Usage {
            message:
                "This copy was installed with Homebrew. Run: brew upgrade accelbyte/tap/ags-cli"
                    .to_string(),
            metadata: None,
        });
    }
    Ok(())
}

/// Attempt to restore the previous binary from `.old`.
///
/// On success returns `Ok(())`; on failure returns an `Err` message naming
/// the restore failure and the reinstall instruction. Callers combine the
/// original error (why the upgrade failed) with this result to decide the
/// final message. Never says "was restored" or "still in place" on failure.
fn restore_outcome(binary_path: &std::path::Path) -> Result<(), String> {
    update_install::restore_previous_binary(binary_path).map_err(|e| {
        format!(
            "Restoring the previous ags binary at {} failed: {e}. \
             Reinstall it with the command from ags update or from \
             https://github.com/AccelByte/accelbyte-ags-cli/releases/latest",
            binary_path.display()
        )
    })
}

/// Build the environment variable pairs for the installer subprocess.
///
/// Resolves the receipt prefix and binary directory, then delegates to
/// [`update_install::installer_env`]. Shared by the dry-run preview and
/// the live path so the two stay in sync.
fn installer_env_for(
    method: ags_protocol::output::InstallMethod,
    binary_path: &std::path::Path,
) -> Vec<(String, String)> {
    let receipt_prefix =
        install_method::receipt_path().and_then(|p| install_method::read_receipt_prefix(&p));
    let binary_dir = binary_path.parent().unwrap_or(std::path::Path::new("."));
    update_install::installer_env(method, receipt_prefix.as_deref(), binary_dir)
}

/// The `ags update --install` flow: confirm, preserve, download, run, verify.
///
/// From the rename (`preserve_previous_binary`) to the final cleanup, an
/// interrupt (Ctrl-C) restores the previous binary and reports the
/// cancelled outcome (exit 2), never success. A second Ctrl-C is the
/// force quit and may leave `.old` behind.
async fn handle_install(
    flags: &GlobalFlags,
    frontend: &mut dyn frontend::Frontend,
    ctx: &FrontendContext,
) -> Result<InvocationOutcome, CliError> {
    let (method, exe_path) = install_method::install_method();
    let binary_path = exe_path;
    let binary_path_display = binary_path.display().to_string();
    let previous_str = env!("CARGO_PKG_VERSION");

    let url_override = ags_runtime::runtime::config::update_installer_url_override();

    // The installer URL override names a script that gets executed, so
    // plain HTTP to a remote host is refused (loopback-only exception for
    // local test servers). This is stricter than AGS_UPDATE_CHECK_URL,
    // which only redirects a read.
    if let Some(ref base) = url_override {
        if !update_install::installer_url_override_allows_plain_http(base) {
            return Err(CliError::Usage {
                message:
                    "AGS_UPDATE_INSTALLER_URL must use https; plain http is accepted for loopback test servers only."
                        .to_string(),
                metadata: None,
            });
        }
    }

    let has_override = url_override.is_some();
    let installer_base =
        url_override.unwrap_or_else(|| update_install::INSTALLER_BASE_URL.to_string());
    let os = std::env::consts::OS;
    let url = update_install::installer_url(&installer_base, os);

    // `https_only` on the reqwest client: production traffic (no override)
    // always enforces HTTPS on the initial hop. When the override is set
    // the loopback check above already rejected plain HTTP to remote hosts,
    // so only loopback test servers reach reqwest with `https_only(false)`.
    let https_only = !has_override;

    // ── Dry-run path (before any network or file access) ──

    if flags.is_dry_run {
        refuse_homebrew(method)?;

        let env = installer_env_for(method, &binary_path);

        let output = UpdateInstallOutput {
            action: UpdateInstallAction::DryRun,
            binary_path: binary_path_display,
            install_method: method,
            installer_url: Some(url),
            latest: None,
            previous: previous_str.to_string(),
            installer_env: env,
        };

        frontend.render(&CommandOutput::UpdateInstall(output))?;
        return Ok(InvocationOutcome::Complete);
    }

    // ── Live path: check, confirm, install ──

    let check_url = update_check::latest_release_url();
    let api_url = update_check::api_url();

    let timeout = std::time::Duration::from_secs(
        flags
            .timeout
            .unwrap_or(ags_runtime::runtime::dispatch::http::DEFAULT_TIMEOUT_SECS),
    );
    let client = update_check::build_client_with_timeout(timeout).ok_or_else(|| {
        CliError::Network {
            message: format!(
                "Could not check for updates: failed to build HTTP client (GET {api_url}). See {check_url}"
            ),
            metadata: None,
        }
    })?;

    let result = update_check::check_now(&client)
        .await
        .map_err(|e| CliError::Network {
            message: format!("Could not check for updates: {e} (GET {api_url}). See {check_url}"),
            metadata: None,
        })?;

    // Already current — checked before refuse_homebrew so that a Homebrew
    // install that is already on the latest version gets a friendly
    // "already current" message (exit 0) rather than the "use brew upgrade"
    // refusal (exit 1).
    if !result.is_newer {
        let output = UpdateInstallOutput {
            action: UpdateInstallAction::AlreadyCurrent,
            binary_path: binary_path_display,
            install_method: method,
            installer_url: None,
            latest: Some(result.latest),
            previous: previous_str.to_string(),
            installer_env: vec![],
        };
        frontend.render(&CommandOutput::UpdateInstall(output))?;
        return Ok(InvocationOutcome::Complete);
    }

    refuse_homebrew(method)?;

    let previous = semver::Version::parse(previous_str).map_err(|e| {
        CliError::Internal(anyhow::anyhow!(
            "Could not parse current version {previous_str:?}: {e}"
        ))
    })?;
    let latest = semver::Version::parse(&result.latest).map_err(|e| {
        CliError::Internal(anyhow::anyhow!(
            "Could not parse latest version {:?}: {e}",
            result.latest
        ))
    })?;

    // Progress line on stderr.
    let color = frontend::style::is_stderr_enabled();
    let headline = format!(
        "{} ags {} is available (current: {})",
        frontend::style::text::SYMBOL_UPGRADE,
        result.latest,
        previous_str,
    );
    frontend::write_stderr_line(&frontend::style::apply_tone(
        &headline,
        frontend::style::Tone::Info,
        color,
    ));

    // Confirm.
    let confirm_prompt = format!(
        "Replace ags {} with {} at {}? [y/N] ",
        previous_str,
        result.latest,
        binary_path.display(),
    );
    let confirmation = crate::invocation::confirm::confirm_or_refuse(
        flags,
        ctx,
        &confirm_prompt,
        "Upgrading in place requires confirmation; pass --yes to confirm.",
        &mut crate::invocation::confirm::read_line_from_stdin,
    )?;
    if confirmation == crate::invocation::confirm::Confirmation::Declined {
        frontend::write_stderr_line("Cancelled.");
        return Ok(InvocationOutcome::Cancelled);
    }

    // ── Interrupt safety: own the signal path before the lock ──
    //
    // Declare ownership so the global Ctrl-C handler defers to the
    // cancellation-token shutdown instead of calling process::exit
    // (which skips destructors and would leak the InstallLock file).
    // This must precede `InstallLock::acquire` so no Ctrl-C in the
    // window between acquiring the lock and setting the flag can bypass
    // the lock's Drop.

    crate::invocation::declare_command_owns_interrupt_path();

    let cancel = tokio_util::sync::CancellationToken::new();
    let signal_cancel = cancel.clone();
    let signal_task = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_cancel.cancel();
        }
    });

    // Abort the watcher task on every return path (including `?`-propagated
    // errors) so it cannot outlive the handler.
    struct AbortOnDrop(tokio::task::JoinHandle<()>);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _signal_guard = AbortOnDrop(signal_task);

    // Lock.
    let cache_dir = ags_runtime::runtime::config::cache_dir().map_err(|e| {
        CliError::Internal(anyhow::anyhow!("Could not determine cache directory: {e}"))
    })?;
    let _lock = update_install::InstallLock::acquire_with_heartbeat(
        cache_dir.join(update_install::LOCK_FILE_NAME),
        update_install::LOCK_STALE_AFTER,
        update_install::LOCK_HEARTBEAT_EVERY,
    )
    .await?;

    // Preserve.
    let old_path = update_install::preserve_previous_binary(&binary_path).map_err(|e| {
        CliError::Internal(anyhow::anyhow!(
            "Could not preserve the current binary as {}: {e}. Nothing has changed.",
            update_install::previous_binary_path(&binary_path).display()
        ))
    })?;

    // Download.
    let tmp = tempfile::Builder::new()
        .prefix("ags-update-")
        .tempdir()
        .map_err(|e| {
            if let Err(restore_msg) = restore_outcome(&binary_path) {
                return CliError::Internal(anyhow::anyhow!(
                    "Could not create temporary directory: {e}. {restore_msg}"
                ));
            }
            CliError::Internal(anyhow::anyhow!(
                "Could not create temporary directory: {e}. The previous ags binary was restored at {binary_path_display}."
            ))
        })?;

    let installer_client = match update_install::build_installer_client(timeout, https_only) {
        Ok(c) => c,
        Err(original) => {
            if let Err(restore_msg) = restore_outcome(&binary_path) {
                return Err(CliError::Internal(anyhow::anyhow!(
                    "{}. {restore_msg}",
                    original.view().message
                )));
            }
            return Err(original);
        }
    };

    let installing_line = format!(
        "{} Installing with {}",
        frontend::style::text::SYMBOL_INFO,
        url,
    );
    frontend::write_stderr_line(&frontend::style::apply_tone(
        &installing_line,
        frontend::style::Tone::Info,
        color,
    ));

    // biased: poll the step branch first so a completed download wins
    // over a simultaneous Ctrl-C rather than discarding it at random.
    let script = tokio::select! {
        biased;
        result = update_install::download_installer(
            &installer_client,
            &url,
            tmp.path(),
            update_install::INSTALLER_MAX_BYTES,
        ) => {
            match result {
                Ok(path) => path,
                Err(original) => {
                    if let Err(restore_msg) = restore_outcome(&binary_path) {
                        return Err(CliError::Internal(anyhow::anyhow!(
                            "{}. {restore_msg}",
                            original.view().message
                        )));
                    }
                    return Err(original);
                }
            }
        }
        _ = cancel.cancelled() => {
            if let Err(restore_msg) = restore_outcome(&binary_path) {
                return Err(CliError::Internal(anyhow::anyhow!("Cancelled. {restore_msg}")));
            }
            frontend::write_stderr_line(&format!(
                "Cancelled. The previous ags binary was restored at {}.",
                binary_path.display()
            ));
            return Ok(InvocationOutcome::Cancelled);
        }
    };

    // Run.
    let env = installer_env_for(method, &binary_path);

    let installer_cmd = update_install::installer_command(&script, &env);
    // biased: poll the step branch first so a completed installer wins
    // over a simultaneous Ctrl-C rather than rolling back a success.
    let status = tokio::select! {
        biased;
        result = update_install::run_installer(installer_cmd) => {
            match result {
                Ok(s) => s,
                Err(original) => {
                    if let Err(restore_msg) = restore_outcome(&binary_path) {
                        return Err(CliError::Internal(anyhow::anyhow!(
                            "{}. {restore_msg}",
                            original.view().message
                        )));
                    }
                    return Err(original);
                }
            }
        }
        _ = cancel.cancelled() => {
            if let Err(restore_msg) = restore_outcome(&binary_path) {
                return Err(CliError::Internal(anyhow::anyhow!("Cancelled. {restore_msg}")));
            }
            frontend::write_stderr_line(&format!(
                "Cancelled. The previous ags binary was restored at {}.",
                binary_path.display()
            ));
            return Ok(InvocationOutcome::Cancelled);
        }
    };

    if !status.success() {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string());
        if let Err(restore_msg) = restore_outcome(&binary_path) {
            return Err(CliError::Internal(anyhow::anyhow!(
                "The installer exited with code {code}. {restore_msg}"
            )));
        }
        return Err(CliError::Internal(anyhow::anyhow!(
            "The installer exited with code {code}. The previous ags binary was restored at {binary_path_display}."
        )));
    }

    // Verify.
    let health_timeout = ags_runtime::runtime::config::update_health_timeout_override()
        .map(std::time::Duration::from_secs)
        .unwrap_or(update_install::HEALTH_CHECK_TIMEOUT);

    // biased: poll the step branch first so a verified upgrade wins
    // over a simultaneous Ctrl-C rather than rolling back a success.
    let installed_version = tokio::select! {
        biased;
        result = update_install::verify_installed_version(
            &binary_path, &previous, &latest, health_timeout,
        ) => {
            match result {
                Ok(v) => v,
                Err(original) => {
                    if let Err(restore_msg) = restore_outcome(&binary_path) {
                        return Err(CliError::Internal(anyhow::anyhow!(
                            "{}. {restore_msg}",
                            original.view().message
                        )));
                    }
                    return Err(CliError::Internal(anyhow::anyhow!(
                        "{} The previous ags binary was restored at {binary_path_display}.",
                        original.view().message
                    )));
                }
            }
        }
        _ = cancel.cancelled() => {
            if let Err(restore_msg) = restore_outcome(&binary_path) {
                return Err(CliError::Internal(anyhow::anyhow!("Cancelled. {restore_msg}")));
            }
            frontend::write_stderr_line(&format!(
                "Cancelled. The previous ags binary was restored at {}.",
                binary_path.display()
            ));
            return Ok(InvocationOutcome::Cancelled);
        }
    };

    // Cleanup.
    if !cfg!(windows) {
        let _ = std::fs::remove_file(&old_path);
    }

    let output = UpdateInstallOutput {
        action: UpdateInstallAction::Installed,
        binary_path: binary_path_display,
        install_method: method,
        installer_url: Some(url),
        latest: Some(installed_version.to_string()),
        previous: previous_str.to_string(),
        installer_env: vec![],
    };

    frontend.render(&CommandOutput::UpdateInstall(output))?;
    Ok(InvocationOutcome::Complete)
}
