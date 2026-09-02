//! Runtime startup cleanup for stale temp files across AGS-owned state.

use std::fs;

use crate::runtime::config;
use crate::support::file_system::{TEMP_FILE_PREFIX, TEMP_FILE_STALE_AGE};

/// Remove stale `ags-ams-upload-*` staging directories from the system temp
/// directory.
///
/// The upload pipeline's staging directory is `Drop`-cleaned on both the
/// success and the error path, but `Drop` does not run when the process is
/// terminated by a signal — a Ctrl-C'd upload leaves its archive behind, and an
/// image archive is large enough to matter. This sweep is the backstop.
pub(crate) fn cleanup_stale_upload_dirs() {
    let temp_dir = std::env::temp_dir();
    let Ok(entries) = fs::read_dir(&temp_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(crate::runtime::ams_upload::TEMP_DIR_PREFIX)
        {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let is_stale = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= TEMP_FILE_STALE_AGE);
        if is_stale {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Remove stale `.ags-tmp-*` files from config, cache, and profile directories.
pub(crate) fn cleanup_stale_temp_files() {
    if let Ok(config_dir) = config::config_dir() {
        crate::support::file_system::cleanup_stale_temp_files(
            &config_dir,
            TEMP_FILE_PREFIX,
            TEMP_FILE_STALE_AGE,
        );
    }

    if let Ok(cache_dir) = config::cache_dir() {
        crate::support::file_system::cleanup_stale_temp_files(
            &cache_dir,
            TEMP_FILE_PREFIX,
            TEMP_FILE_STALE_AGE,
        );
    }

    if let Ok(profiles_dir) = config::profiles_dir() {
        let Ok(entries) = fs::read_dir(&profiles_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let profile_path = entry.path();
            if profile_path.is_dir() {
                crate::support::file_system::cleanup_stale_temp_files(
                    &profile_path,
                    TEMP_FILE_PREFIX,
                    TEMP_FILE_STALE_AGE,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::{set_file_mtime, FileTime};
    use std::time::Duration;

    /// Mark a temp file old enough to be treated as stale.
    fn mark_stale(path: &std::path::Path) {
        let stale = std::time::SystemTime::now() - TEMP_FILE_STALE_AGE - Duration::from_secs(1);
        set_file_mtime(path, FileTime::from_system_time(stale)).unwrap();
    }

    /// Cleanup removes stale temp files from individual profile directories.
    #[test]
    #[serial_test::serial]
    fn test_cleanup_removes_stale_temp_files_from_profile_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, tmp.path());
        let profile = tmp.path().join("profiles").join("default");
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join(".ags-tmp-abc123"), "stale").unwrap();
        mark_stale(&profile.join(".ags-tmp-abc123"));
        fs::write(profile.join("config.json"), "{}").unwrap();

        cleanup_stale_temp_files();

        std::env::remove_var(config::ENV_HOME);
        assert!(!profile.join(".ags-tmp-abc123").exists());
        assert!(profile.join("config.json").exists());
    }

    /// Cleanup sweeps every profile directory under the profiles root.
    #[test]
    #[serial_test::serial]
    fn test_cleanup_handles_multiple_profiles() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, tmp.path());
        for name in &["default", "work"] {
            let dir = tmp.path().join("profiles").join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(".ags-tmp-stale"), "stale").unwrap();
            mark_stale(&dir.join(".ags-tmp-stale"));
        }

        cleanup_stale_temp_files();

        std::env::remove_var(config::ENV_HOME);
        for name in &["default", "work"] {
            let dir = tmp.path().join("profiles").join(name);
            assert!(!dir.join(".ags-tmp-stale").exists());
        }
    }

    /// Cleanup is a no-op when the profiles directory does not exist.
    #[test]
    #[serial_test::serial]
    fn test_cleanup_is_silent_when_profiles_dir_absent() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, tmp.path());
        cleanup_stale_temp_files();
        std::env::remove_var(config::ENV_HOME);
    }

    /// Cleanup removes stale temp files from the global config directory.
    #[test]
    #[serial_test::serial]
    fn test_cleanup_removes_stale_temp_files_from_config_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, tmp.path());
        fs::write(tmp.path().join(".ags-tmp-global"), "stale").unwrap();
        mark_stale(&tmp.path().join(".ags-tmp-global"));
        fs::write(tmp.path().join("config.json"), "{}").unwrap();

        cleanup_stale_temp_files();

        std::env::remove_var(config::ENV_HOME);
        assert!(!tmp.path().join(".ags-tmp-global").exists());
        assert!(tmp.path().join("config.json").exists());
    }

    /// Cleanup removes stale temp files from the spec cache directory.
    #[test]
    #[serial_test::serial]
    fn test_cleanup_removes_stale_temp_files_from_cache_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, tmp.path());
        let cache = tmp.path().join("cache");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join(".ags-tmp-abc"), "stale").unwrap();
        mark_stale(&cache.join(".ags-tmp-abc"));
        fs::write(cache.join("iam.json"), "{}").unwrap();

        cleanup_stale_temp_files();

        std::env::remove_var(config::ENV_HOME);
        assert!(!cache.join(".ags-tmp-abc").exists());
        assert!(cache.join("iam.json").exists());
    }

    /// Cleanup preserves recent temp files that may belong to active writers.
    #[test]
    #[serial_test::serial]
    fn test_cleanup_preserves_recent_temp_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, tmp.path());
        let profile = tmp.path().join("profiles").join("default");
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join(".ags-tmp-live"), "recent").unwrap();

        cleanup_stale_temp_files();

        std::env::remove_var(config::ENV_HOME);
        assert!(profile.join(".ags-tmp-live").exists());
    }
}

#[cfg(test)]
mod upload_dir_tests {
    use super::*;
    use filetime::{set_file_mtime, FileTime};
    use std::time::Duration;

    /// The variables `std::env::temp_dir()` actually reads: `TMPDIR` on unix,
    /// but `GetTempPath2`'s `TMP`/`TEMP` on Windows, where `TMPDIR` is ignored.
    #[cfg(not(windows))]
    const TEMP_DIR_VARS: &[&str] = &["TMPDIR"];
    #[cfg(windows)]
    const TEMP_DIR_VARS: &[&str] = &["TMP", "TEMP"];

    /// Age a directory past the stale threshold.
    fn mark_stale(path: &std::path::Path) {
        let stale = std::time::SystemTime::now() - TEMP_FILE_STALE_AGE - Duration::from_secs(1);
        set_file_mtime(path, FileTime::from_system_time(stale)).unwrap();
    }

    /// A staging directory left behind by a signal-killed upload is reclaimed
    /// once it is stale; a fresh one (a concurrent upload) is left alone.
    #[test]
    #[serial_test::serial]
    fn test_stale_upload_dirs_are_removed_and_fresh_ones_kept() {
        let temp_root = tempfile::tempdir().unwrap();
        let originals: Vec<_> = TEMP_DIR_VARS
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        for key in TEMP_DIR_VARS {
            std::env::set_var(key, temp_root.path());
        }

        let stale = temp_root.path().join(format!(
            "{}stale",
            crate::runtime::ams_upload::TEMP_DIR_PREFIX
        ));
        let fresh = temp_root.path().join(format!(
            "{}fresh",
            crate::runtime::ams_upload::TEMP_DIR_PREFIX
        ));
        let unrelated = temp_root.path().join("someone-elses-dir");
        for dir in [&stale, &fresh, &unrelated] {
            std::fs::create_dir(dir).unwrap();
            std::fs::write(dir.join("image.tar.gz"), b"archive").unwrap();
        }
        mark_stale(&stale);
        mark_stale(&unrelated);

        cleanup_stale_upload_dirs();

        assert!(
            !stale.exists(),
            "a stale staging directory must be reclaimed"
        );
        assert!(
            fresh.exists(),
            "a fresh staging directory is an active upload"
        );
        assert!(unrelated.exists(), "only our own prefix is swept");

        for (key, original) in originals {
            match original {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}
