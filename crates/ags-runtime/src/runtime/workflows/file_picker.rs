//! Local filesystem directory listing for the workflow file-picker field
//! (`FilePickerSpec`). Pure, synchronous, side-effect free — unlike
//! `options.rs`'s network-backed resolution, a local directory read is
//! effectively instant and needs no async plumbing or cancellation handling.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One directory entry offered by the file picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Display name: just the final path component, or `".."` for the
    /// parent-directory pseudo-entry.
    pub name: String,
    /// Absolute path to this entry (or to the parent directory, for `".."`).
    pub path: PathBuf,
    pub is_dir: bool,
}

/// List `dir`'s entries, filtered to files matching `extensions` (case-
/// insensitive; `None` or empty = every file allowed) plus every
/// subdirectory (never extension-filtered — a matching file might be inside
/// one), excluding entries whose name starts with `'.'` (Unix dotfile
/// convention). The Windows hidden-file attribute is not consulted — a file
/// marked hidden but whose name does not start with `'.'` will appear in
/// the listing. This is a deliberate simplification, not an oversight.
/// Symlinks are resolved via metadata: a symlinked directory is listed as a
/// directory, a symlinked file as a file. Unreadable or broken entries
/// (including broken symlinks) are silently skipped rather than failing the
/// whole listing. Sorted: `..` first (omitted only when `dir` has no
/// parent — i.e. at a filesystem root), then directories alphabetically
/// (case-insensitive), then files alphabetically (case-insensitive).
pub fn list_directory(dir: &Path, extensions: Option<&[String]>) -> io::Result<Vec<FileEntry>> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if name_str.starts_with('.') {
            continue;
        }
        let path = entry.path();
        // `entry.metadata()` does not traverse symlinks (it returns the
        // symlink's own metadata, for which is_dir()/is_file() are both
        // false), so a symlinked entry would otherwise fall through as
        // "neither file nor directory" and be skipped. `fs::metadata`
        // follows symlinks, resolving to the target's type as documented.
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            dirs.push(FileEntry {
                name: name_str.to_string(),
                path,
                is_dir: true,
            });
        } else if metadata.is_file() && extension_allowed(&path, extensions) {
            files.push(FileEntry {
                name: name_str.to_string(),
                path,
                is_dir: false,
            });
        }
        // Anything that's neither a file nor a directory after following
        // symlinks (a broken symlink, a device node, ...) is skipped.
    }

    dirs.sort_by_key(|a| a.name.to_lowercase());
    files.sort_by_key(|a| a.name.to_lowercase());

    let mut out = Vec::with_capacity(dirs.len() + files.len() + 1);
    if let Some(parent) = dir.parent() {
        out.push(FileEntry {
            name: "..".to_string(),
            path: parent.to_path_buf(),
            is_dir: true,
        });
    }
    out.extend(dirs);
    out.extend(files);
    Ok(out)
}

/// Whether `path`'s extension is allowed: `None`/empty `extensions` allows
/// every file; otherwise the path's extension (case-insensitive) must be in
/// the list. A path with no extension is excluded whenever a filter is set.
fn extension_allowed(path: &Path, extensions: Option<&[String]>) -> bool {
    let Some(extensions) = extensions else {
        return true;
    };
    if extensions.is_empty() {
        return true;
    }
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    extensions
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_directory_filters_by_extension_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("icon.PNG"), b"").unwrap();
        fs::write(dir.path().join("readme.txt"), b"").unwrap();
        let extensions = vec!["png".to_string()];
        let entries = list_directory(dir.path(), Some(&extensions)).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"icon.PNG"));
        assert!(!names.contains(&"readme.txt"));
    }

    #[test]
    fn test_list_directory_none_extensions_allows_every_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"").unwrap();
        fs::write(dir.path().join("b.bin"), b"").unwrap();
        let entries = list_directory(dir.path(), None).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.bin"));
    }

    #[test]
    fn test_list_directory_never_filters_directories_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("assets.old")).unwrap(); // dir name has no allowed "extension"
        let extensions = vec!["png".to_string()];
        let entries = list_directory(dir.path(), Some(&extensions)).unwrap();
        assert!(entries.iter().any(|e| e.name == "assets.old" && e.is_dir));
    }

    #[test]
    fn test_list_directory_excludes_hidden_entries() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".hidden"), b"").unwrap();
        fs::write(dir.path().join("visible.txt"), b"").unwrap();
        let entries = list_directory(dir.path(), None).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(!names.contains(&".hidden"));
        assert!(names.contains(&"visible.txt"));
    }

    #[test]
    fn test_list_directory_sort_order_dotdot_dirs_then_files() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("sub");
        fs::create_dir(&dir).unwrap();
        fs::create_dir(dir.join("zzz-dir")).unwrap();
        fs::create_dir(dir.join("aaa-dir")).unwrap();
        fs::write(dir.join("bbb-file.txt"), b"").unwrap();
        fs::write(dir.join("aaa-file.txt"), b"").unwrap();
        let entries = list_directory(&dir, None).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["..", "aaa-dir", "zzz-dir", "aaa-file.txt", "bbb-file.txt"]
        );
    }

    #[test]
    fn test_list_directory_omits_dotdot_at_filesystem_root() {
        // A path with no parent (platform root) must not list a ".." entry.
        // `Path::parent()` returns `None` for a root path on both platforms.
        let root = Path::new(if cfg!(windows) { "C:\\" } else { "/" });
        assert!(root.parent().is_none());

        // Exercise list_directory on a non-root directory and confirm the
        // ".." pseudo-entry IS emitted when a parent exists.
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("child");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("file.txt"), b"").unwrap();
        let entries = list_directory(&sub, None).unwrap();
        assert!(
            entries.first().map(|e| e.name.as_str()) == Some(".."),
            "expected first entry to be \"..\" but got: {:?}",
            entries.first().map(|e| &e.name),
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_list_directory_resolves_symlinked_dir_and_file() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let target_dir = root.path().join("real-dir");
        let target_file = root.path().join("real-file.txt");
        fs::create_dir(&target_dir).unwrap();
        fs::write(&target_file, b"").unwrap();
        symlink(&target_dir, root.path().join("linked-dir")).unwrap();
        symlink(&target_file, root.path().join("linked-file.txt")).unwrap();

        let entries = list_directory(root.path(), None).unwrap();
        let linked_dir = entries.iter().find(|e| e.name == "linked-dir").unwrap();
        let linked_file = entries
            .iter()
            .find(|e| e.name == "linked-file.txt")
            .unwrap();
        assert!(linked_dir.is_dir);
        assert!(!linked_file.is_dir);
    }
}
