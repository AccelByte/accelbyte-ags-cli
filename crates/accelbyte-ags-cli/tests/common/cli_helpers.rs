use assert_cmd::Command;

/// Base command builder for the ags binary.
///
/// Sets `AGS_NO_UPDATE_CHECK=1` so the test suite is hermetic: no test
/// spawns a background child that calls `api.github.com`. Both
/// `ags_isolated()` and `ags_with_base_url()` delegate here, so the
/// suppression covers all three builders.
pub fn ags() -> Command {
    let mut command = Command::cargo_bin("ags").unwrap();
    command.env("AGS_NO_UPDATE_CHECK", "1");
    command
}

/// Command builder with update-check suppression deliberately cleared.
///
/// Both `AGS_NO_UPDATE_CHECK` and `CI` are removed from the child's
/// environment so the `__update-check` arm actually runs its fetch.
/// Use this ONLY in the functional tests that exercise the update-check
/// path against a mock server — every other test should use [`ags()`],
/// which sets `AGS_NO_UPDATE_CHECK=1` to prevent live GitHub calls.
pub fn ags_with_update_check_enabled() -> Command {
    let mut command = Command::cargo_bin("ags").unwrap();
    command.env_remove("AGS_NO_UPDATE_CHECK");
    command.env_remove("CI");
    command
}

/// Command isolated from real credentials and config state.
///
/// Uses `AGS_NO_KEYCHAIN=1` and a unique temp directory as `AGS_HOME` to
/// prevent token/config bleed between tests. The directory is keyed by the
/// current thread's **name**, which the Rust test harness sets to the
/// fully-qualified test path (e.g. `module::submodule::test_name`), so
/// each test gets its own directory. Multiple calls to `ags_isolated()`
/// within the same test deliberately share state — they resolve to the
/// same `AGS_HOME`.
///
/// Thread *IDs* (`ThreadId`) are unsuitable here because the harness's
/// thread pool reuses them across sequential tests, causing unrelated
/// tests to share state and producing flaky failures.
///
/// The `::` separators in the thread name are replaced with `--` to
/// produce a valid path segment on all platforms (`:` is forbidden in
/// Windows filenames).
pub fn ags_isolated() -> Command {
    let mut command = ags();
    let test_name = std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .replace("::", "--");
    let unique_dir = std::env::temp_dir()
        .join(format!("ags-test-{}", std::process::id()))
        .join(test_name);
    command
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_HOME", unique_dir);
    command
}

/// Command isolated from real credentials with a custom base URL.
/// Useful for tests that point at a wiremock server.
pub fn ags_with_base_url(base_url: &str) -> Command {
    let mut command = ags_isolated();
    command.env("AGS_BASE_URL", base_url);
    command
}

/// Place the built `ags` binary into `dir` and return its path and a Command.
///
/// For tests that must observe `current_exe()` at a path they control.
/// The returned command has `AGS_NO_UPDATE_CHECK=1` and `AGS_NO_KEYCHAIN=1`
/// pre-set.
///
/// On Unix, uses a hard link instead of a copy. The debug binary is ~270 MB;
/// `std::fs::copy` holds a write fd on the destination for the duration of
/// the copy. When parallel tests fork (via `Command::output`/`spawn`), the
/// child inherits the write fd. The forked child's `exec` closes it via
/// `O_CLOEXEC`, but our `exec` can race ahead and hit `ETXTBSY`. A hard
/// link creates a directory entry without any write fd, eliminating the
/// race. Falls back to copy if the link fails (e.g. cross-filesystem).
pub fn ags_copied_to(dir: &std::path::Path) -> (std::path::PathBuf, Command) {
    let original = assert_cmd::cargo::cargo_bin("ags");
    let binary_name = original.file_name().unwrap();
    let dest = dir.join(binary_name);
    #[cfg(unix)]
    {
        if std::fs::hard_link(&original, &dest).is_err() {
            std::fs::copy(&original, &dest).expect("failed to copy ags binary");
        }
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).ok();
    }
    #[cfg(not(unix))]
    {
        std::fs::copy(&original, &dest).expect("failed to copy ags binary");
    }
    let mut cmd = Command::new(&dest);
    cmd.env("AGS_NO_UPDATE_CHECK", "1");
    cmd.env("AGS_NO_KEYCHAIN", "1");
    (dest, cmd)
}
