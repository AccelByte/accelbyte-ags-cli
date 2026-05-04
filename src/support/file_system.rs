//! Shared filesystem helpers for restricted writes, advisory locks, and temp-file cleanup.

use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use fs4::FileExt;

/// Create a directory tree with 0700 permissions on Unix, using `DirBuilder::mode`
/// for atomic creation (no TOCTOU window). On non-Unix platforms, falls back to
/// `create_dir_all` with umask-derived permissions.
pub(crate) fn create_dir_restricted(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)
}

/// Filename prefix for temp files created by `write_file_restricted`.
/// `cleanup_stale_temp_files` matches the same prefix when sweeping leftovers.
pub(crate) const TEMP_FILE_PREFIX: &str = ".ags-tmp-";

/// Age threshold beyond which a leftover temp file is considered stale and
/// safe to delete. Long enough to outlast any in-flight write that might
/// briefly leave the temp file visible on disk.
pub(crate) const TEMP_FILE_STALE_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Write `data` to `path` with 0600 permissions (Unix), replacing any existing
/// content so the destination is never partially written.
pub(crate) fn write_file_restricted(path: &Path, data: &str) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("No parent directory for '{}'", path.display()),
        )
    })?;

    let mut builder = tempfile::Builder::new();
    builder.prefix(TEMP_FILE_PREFIX);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o600));
    }
    let mut tmp = builder.tempfile_in(dir)?;

    tmp.write_all(data.as_bytes())?;
    tmp.as_file().sync_all()?;

    tmp.persist(path).map_err(|e| {
        let error = std::io::Error::new(
            e.error.kind(),
            format!(
                "Failed to rename '{}' to '{}': {}",
                e.file.path().display(),
                path.display(),
                e.error
            ),
        );
        drop(e.file);
        error
    })?;

    Ok(())
}

/// RAII guard for an exclusive advisory file lock.
/// The lock is released when this value is dropped.
pub struct FileLock {
    _file: std::fs::File,
}

// FileLock crosses async/blocking task boundaries via `tokio::task::spawn_blocking`
// (see `runtime::auth::session::acquire_token_lock`), which requires the returned
// value to be Send. Assert it here so any future change that introduces a non-Send
// member fails locally rather than at the distant call site.
const _: () = {
    /// Compile-time `Send` assertion — instantiating it forces the bound check.
    const fn assert_send<T: Send>() {}
    assert_send::<FileLock>();
};

impl FileLock {
    /// Acquire an exclusive advisory lock on `path`, creating the file and its
    /// parent directory if needed. When the lock is already held by another
    /// process, the process-global contention reporter fires once before we block.
    pub fn acquire(path: &Path, name: &str) -> Result<Self, io::Error> {
        if let Some(dir) = path.parent() {
            create_dir_restricted(dir).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("Failed to create lock directory '{}': {e}", dir.display()),
                )
            })?;
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("Failed to open lock file '{}': {e}", path.display()),
                )
            })?;

        match file.try_lock_exclusive() {
            Ok(()) => {}
            Err(e) if is_lock_contended(&e) => {
                crate::support::report_lock_contention(name);
                file.lock_exclusive().map_err(|e| {
                    io::Error::new(
                        e.kind(),
                        format!("Failed to acquire lock '{}': {e}", path.display()),
                    )
                })?;
            }
            Err(e) => {
                return Err(io::Error::new(
                    e.kind(),
                    format!("Failed to acquire lock '{}': {e}", path.display()),
                ));
            }
        }

        Ok(FileLock { _file: file })
    }
}

/// Return whether an error from `try_lock_exclusive` means another process
/// currently holds the lock rather than a real I/O failure.
///
/// Cross-platform behaviour (`fs4` 0.8): on Unix, `lock_contended_error()`
/// returns `EWOULDBLOCK` (kind `WouldBlock`); on Windows, it returns
/// `ERROR_LOCK_VIOLATION` (raw OS code 33, kind `Other`). Both platforms
/// have a known sentinel value, and our strict-equality match against
/// `fs4::lock_contended_error()` recognises whichever value the host
/// produces. This invariant cannot be pinned by a single-host Rust test
/// because the reference value is host-determined; verified by reading
/// `fs4`'s `windows.rs` and `unix/mod.rs` sources.
fn is_lock_contended(error: &io::Error) -> bool {
    let reference = fs4::lock_contended_error();
    if error.kind() == reference.kind() {
        return true;
    }
    // Only compare raw OS codes when both sides are Some — otherwise None == None
    // would misclassify unrelated errors (custom errors, some Windows I/O paths)
    // as contention and cause an indefinite block on lock_exclusive().
    match (error.raw_os_error(), reference.raw_os_error()) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

impl Drop for FileLock {
    /// Release the advisory lock when the guard is dropped.
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self._file);
    }
}

/// Remove stale temp files from a directory, ignoring all read/remove errors.
pub(crate) fn cleanup_stale_temp_files(dir: &Path, prefix: &str, stale_age: Duration) {
    let Ok(files) = fs::read_dir(dir) else { return };
    for file in files.flatten() {
        if !file.file_name().to_string_lossy().starts_with(prefix) {
            continue;
        }
        let Ok(metadata) = file.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let Ok(age) = modified.elapsed() else {
            continue;
        };
        if age >= stale_age {
            let _ = fs::remove_file(file.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::{set_file_mtime, FileTime};
    use std::sync::{Arc, Barrier};
    use std::thread;

    /// Mark a file as stale so cleanup should remove it.
    fn mark_stale(path: &Path) {
        let stale = std::time::SystemTime::now() - TEMP_FILE_STALE_AGE - Duration::from_secs(1);
        set_file_mtime(path, FileTime::from_system_time(stale)).unwrap();
    }

    /// Restricted writes create the destination file with the expected content.
    #[test]
    fn test_write_file_restricted_creates_file_with_correct_content() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        write_file_restricted(&path, r#"{"key":"value"}"#).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"key":"value"}"#
        );
    }

    /// Restricted writes replace any existing content atomically.
    #[test]
    fn test_write_file_restricted_overwrites_existing_content() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        write_file_restricted(&path, "old").unwrap();
        write_file_restricted(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    }

    /// Restricted writes leave no temp files behind after success.
    #[test]
    fn test_write_file_restricted_leaves_no_temp_files_in_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        write_file_restricted(&path, "data").unwrap();
        let leftover: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(TEMP_FILE_PREFIX)
            })
            .collect();
        assert!(leftover.is_empty());
        assert!(path.exists());
    }

    /// Restricted writes fail when the destination parent does not exist.
    #[test]
    fn test_write_file_restricted_fails_for_nonexistent_parent() {
        let path = std::path::Path::new("/definitely/does/not/exist/config.json");
        assert!(write_file_restricted(path, "data").is_err());
    }

    #[cfg(unix)]
    /// Restricted writes preserve 0600 permissions on Unix.
    #[test]
    fn test_write_file_restricted_sets_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        write_file_restricted(&path, "data").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    /// Overwriting an existing restricted file preserves 0600 — the temp+rename
    /// strategy must not regress to umask-derived permissions on the second write.
    #[test]
    fn test_write_file_restricted_overwrite_preserves_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        write_file_restricted(&path, "old").unwrap();
        write_file_restricted(&path, "new").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    /// Failed restricted writes do not leave temp files behind.
    #[test]
    fn test_write_file_restricted_leaves_no_temp_files_on_failure() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o500)).unwrap();

        let path = tmp.path().join("config.json");
        let result = write_file_restricted(&path, "data");

        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();

        assert!(result.is_err());
        let entries: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(entries.is_empty());
    }

    /// Restricted directory creation creates the full directory tree.
    #[test]
    fn test_create_dir_restricted_creates_directory_recursively() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a").join("b").join("c");
        create_dir_restricted(&dir).unwrap();
        assert!(dir.is_dir());
    }

    /// Restricted directory creation is idempotent.
    #[test]
    fn test_create_dir_restricted_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("restricted");
        create_dir_restricted(&dir).unwrap();
        create_dir_restricted(&dir).unwrap();
        assert!(dir.is_dir());
    }

    #[cfg(unix)]
    /// Restricted directory creation preserves 0700 permissions on Unix.
    #[test]
    fn test_create_dir_restricted_sets_0700_on_new_dir() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("restricted");
        create_dir_restricted(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    /// Temp-file cleanup removes stale files but preserves non-temp files.
    #[test]
    fn test_cleanup_stale_temp_files_removes_only_stale_matching_files() {
        let tmp = tempfile::tempdir().unwrap();
        let stale = tmp.path().join(".ags-tmp-stale");
        let live = tmp.path().join(".ags-tmp-live");
        let other = tmp.path().join("config.json");
        fs::write(&stale, "stale").unwrap();
        fs::write(&live, "live").unwrap();
        fs::write(&other, "{}").unwrap();
        mark_stale(&stale);

        cleanup_stale_temp_files(tmp.path(), TEMP_FILE_PREFIX, TEMP_FILE_STALE_AGE);

        assert!(!stale.exists());
        assert!(live.exists());
        assert!(other.exists());
    }

    /// A lock file is created and the lock is successfully acquired with no contention.
    #[test]
    fn test_acquire_with_no_contention() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".test.lock");
        let _lock = FileLock::acquire(&path, "test resource").unwrap();
        assert!(path.exists());
    }

    /// The lock is released when the guard is dropped and a second acquisition succeeds.
    #[test]
    fn test_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".test.lock");
        {
            let _lock = FileLock::acquire(&path, "test resource").unwrap();
        }
        let _lock2 = FileLock::acquire(&path, "test resource").unwrap();
    }

    /// The parent directory of the lock file is created with 0700 permissions.
    #[cfg(unix)]
    #[test]
    fn test_acquire_creates_parent_directory_with_0700_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let lock_path = tmp.path().join("subdir").join(".test.lock");
        let _lock = FileLock::acquire(&lock_path, "test resource").unwrap();
        let mode = std::fs::metadata(lock_path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn test_contended_error_is_classified_but_unrelated_errors_are_not() {
        let contended = fs4::lock_contended_error();
        assert!(is_lock_contended(&contended));

        let unrelated = io::Error::new(io::ErrorKind::PermissionDenied, "nope");
        assert!(!is_lock_contended(&unrelated));
    }

    /// Two threads competing for the same lock both complete without a lost update.
    #[test]
    fn test_two_threads_both_complete_without_data_loss() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = Arc::new(dir.path().join(".test.lock"));
        let data_path = Arc::new(dir.path().join("counter.txt"));
        std::fs::write(data_path.as_ref(), "0").unwrap();
        let barrier = Arc::new(Barrier::new(2));

        let lock_path2 = Arc::clone(&lock_path);
        let data_path2 = Arc::clone(&data_path);
        let barrier2 = Arc::clone(&barrier);
        let handle = thread::spawn(move || {
            barrier2.wait();
            let _lock = FileLock::acquire(&lock_path2, "test resource").unwrap();
            let n: u64 = std::fs::read_to_string(data_path2.as_ref())
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            thread::sleep(Duration::from_millis(10));
            std::fs::write(data_path2.as_ref(), (n + 1).to_string()).unwrap();
        });

        barrier.wait();
        let lock = FileLock::acquire(&lock_path, "test resource").unwrap();
        let n: u64 = std::fs::read_to_string(data_path.as_ref())
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        thread::sleep(Duration::from_millis(10));
        std::fs::write(data_path.as_ref(), (n + 1).to_string()).unwrap();
        drop(lock);

        handle.join().unwrap();
        let final_val = std::fs::read_to_string(data_path.as_ref()).unwrap();
        assert_eq!(final_val.trim(), "2");
    }
}
