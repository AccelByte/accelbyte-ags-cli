//! Source-path extraction: relocate a subdirectory of a cloned repository
//! to the destination root.
//!
//! After a template repository is cloned, the user-visible content may live
//! under a subdirectory (e.g. `templates/react`). This module moves the
//! contents of that subdirectory into the destination root, removing all
//! other repository contents (including `.git`).

use std::path::Path;

use crate::errors::CliError;

/// Move the contents of `source_path` (relative to `destination`) into
/// `destination`, removing all other files. The source path must be relative,
/// must not traverse outside `destination`, and must point to an existing
/// directory inside the cloned tree.
///
/// Uses an atomic staging pattern: entries are moved to a temporary sibling
/// directory, the original destination is cleared, then staged entries are
/// moved back. This avoids partial states if a filesystem operation fails
/// mid-way.
pub(super) fn extract_source_path(destination: &Path, source_path: &str) -> Result<(), CliError> {
    let trimmed = source_path.trim();
    if trimmed.is_empty() {
        return Err(usage_error(
            "Source path cannot be empty",
            "Provide a non-empty relative path inside the repository",
        ));
    }

    let clean = std::path::PathBuf::from(trimmed);
    if clean.is_absolute() {
        return Err(usage_error(
            &format!("Source path must be relative: {source_path}"),
            "Use a relative path like 'templates/react'",
        ));
    }

    // Reject traversal paths (`..` at the start or after normalization).
    let clean_str = clean.to_string_lossy();
    if clean_str == ".."
        || clean_str.starts_with(&format!("..{}", std::path::MAIN_SEPARATOR))
        || clean_str.starts_with("../")
    {
        return Err(usage_error(
            &format!("Source path cannot traverse outside destination: {source_path}"),
            "Use a path that stays within the cloned repository",
        ));
    }

    let destination_abs = std::fs::canonicalize(destination)
        .map_err(|e| CliError::Internal(anyhow::anyhow!("Failed to resolve destination: {e}")))?;

    let source_abs = destination_abs.join(&clean);
    let source_abs = std::fs::canonicalize(&source_abs).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            return usage_error(
                &format!("Source path does not exist in repository: {source_path}"),
                "Check that the path exists in the cloned repository",
            );
        }
        CliError::Internal(anyhow::anyhow!("Failed to resolve source path: {e}"))
    })?;

    // Double-check that the resolved source is inside the destination.
    if !source_abs.starts_with(&destination_abs) {
        return Err(usage_error(
            &format!("Source path cannot traverse outside destination: {source_path}"),
            "Use a path that stays within the cloned repository",
        ));
    }

    let source_meta = std::fs::metadata(&source_abs)
        .map_err(|e| CliError::Internal(anyhow::anyhow!("Failed to inspect source path: {e}")))?;
    if !source_meta.is_dir() {
        return Err(usage_error(
            &format!("Source path must be a directory: {source_path}"),
            "Point to a directory inside the cloned repository, not a file",
        ));
    }

    // Stage entries in a temporary sibling directory.
    let parent = destination_abs.parent().ok_or_else(|| {
        CliError::Internal(anyhow::anyhow!("Destination has no parent directory"))
    })?;
    let stage_dir = tempfile::tempdir_in(parent).map_err(|e| {
        CliError::Internal(anyhow::anyhow!("Failed to create staging directory: {e}"))
    })?;

    let entries = std::fs::read_dir(&source_abs).map_err(|e| {
        CliError::Internal(anyhow::anyhow!("Failed to read source path contents: {e}"))
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| {
            CliError::Internal(anyhow::anyhow!("Failed to read directory entry: {e}"))
        })?;
        let from = entry.path();
        let to = stage_dir.path().join(entry.file_name());
        std::fs::rename(&from, &to).map_err(|e| {
            CliError::Internal(anyhow::anyhow!(
                "Failed to stage '{}': {e}",
                entry.file_name().to_string_lossy()
            ))
        })?;
    }

    // Remove the entire destination tree and recreate it.
    std::fs::remove_dir_all(&destination_abs).map_err(|e| {
        CliError::Internal(anyhow::anyhow!("Failed to remove cloned repository: {e}"))
    })?;
    std::fs::create_dir_all(&destination_abs).map_err(|e| {
        CliError::Internal(anyhow::anyhow!(
            "Failed to recreate destination directory: {e}"
        ))
    })?;

    // Move staged entries back into the clean destination.
    let staged = std::fs::read_dir(stage_dir.path())
        .map_err(|e| CliError::Internal(anyhow::anyhow!("Failed to read staged contents: {e}")))?;
    for entry in staged {
        let entry = entry
            .map_err(|e| CliError::Internal(anyhow::anyhow!("Failed to read staged entry: {e}")))?;
        let from = entry.path();
        let to = destination_abs.join(entry.file_name());
        std::fs::rename(&from, &to).map_err(|e| {
            CliError::Internal(anyhow::anyhow!(
                "Failed to move staged entry '{}' to destination: {e}",
                entry.file_name().to_string_lossy()
            ))
        })?;
    }

    Ok(())
}

/// Build a usage error with a suggestion.
fn usage_error(message: &str, suggestion: &str) -> CliError {
    CliError::Usage {
        message: message.to_string(),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            suggestion,
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create a mock cloned repository with a subdirectory to extract.
    fn setup_repo_with_subpath(root: &Path) -> std::path::PathBuf {
        let destination = root.join("repo");
        fs::create_dir_all(destination.join(".git")).unwrap();
        fs::create_dir_all(destination.join("templates").join("react").join("src")).unwrap();
        fs::write(
            destination
                .join("templates")
                .join("react")
                .join("package.json"),
            "{}",
        )
        .unwrap();
        fs::write(
            destination
                .join("templates")
                .join("react")
                .join("src")
                .join("main.js"),
            "console.log('ok')",
        )
        .unwrap();
        fs::write(destination.join("README.md"), "root readme").unwrap();
        destination
    }

    #[test]
    fn test_extracts_selected_subpath_into_destination_root() {
        let root = TempDir::new().unwrap();
        let destination = setup_repo_with_subpath(root.path());

        extract_source_path(&destination, "templates/react").unwrap();

        assert!(destination.join("package.json").exists());
        assert!(destination.join("src").join("main.js").exists());
        assert!(!destination.join(".git").exists());
        assert!(!destination.join("templates").exists());
        assert!(!destination.join("README.md").exists());
    }

    #[test]
    fn test_returns_error_when_source_path_does_not_exist() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("repo");
        fs::create_dir_all(&destination).unwrap();

        let result = extract_source_path(&destination, "templates/react");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("does not exist"),
            "expected 'does not exist' in: {err_msg}"
        );
    }

    #[test]
    fn test_returns_error_when_source_path_points_to_file() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("repo");
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("templates"), "not a dir").unwrap();

        let result = extract_source_path(&destination, "templates");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("must be a directory"),
            "expected 'must be a directory' in: {err_msg}"
        );
    }

    #[test]
    fn test_returns_error_on_traversal_path() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("repo");
        fs::create_dir_all(&destination).unwrap();

        let result = extract_source_path(&destination, "../templates/react");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("cannot traverse outside destination"),
            "expected traversal error in: {err_msg}"
        );
    }

    #[test]
    fn test_returns_error_on_empty_source_path() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("repo");
        fs::create_dir_all(&destination).unwrap();

        let result = extract_source_path(&destination, "  ");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("cannot be empty"),
            "expected 'cannot be empty' in: {err_msg}"
        );
    }

    #[test]
    fn test_returns_error_on_absolute_source_path() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("repo");
        fs::create_dir_all(&destination).unwrap();

        // Use a platform-appropriate absolute path: on Windows, `/etc/passwd`
        // is not considered absolute by `Path::is_absolute()`, so we must use
        // a Windows-style path there.
        let abs_path = if cfg!(windows) {
            r"C:\Windows\System32"
        } else {
            "/etc/passwd"
        };

        let result = extract_source_path(&destination, abs_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("must be relative"),
            "expected 'must be relative' in: {err_msg}"
        );
    }
}
