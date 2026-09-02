//! Enumerating the upload directory and packing it into a single `tar.gz`.

use std::path::{Path, PathBuf};

use flate2::write::GzEncoder;
use flate2::Compression;

use super::errors::AmsUploadError;

/// Extensions treated as debug-symbol files and excluded unless the caller asks
/// for them. Matched exactly, mirroring armada-cli's list.
const SYMBOL_FILE_EXTENSIONS: &[&str] = &["PDB", "SYM", "debug", "pdb", "sym"];

/// One file selected for the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// Absolute (or caller-relative) path on disk.
    pub path: PathBuf,
    /// Path inside the archive, always `/`-separated and relative to the
    /// upload directory.
    pub archive_path: String,
    pub size_bytes: u64,
}

/// The files that will be archived, plus what was left out.
#[derive(Debug, Clone, Default)]
pub struct ArchiveManifest {
    pub entries: Vec<ArchiveEntry>,
    pub total_bytes: u64,
    pub excluded_symbol_file_count: usize,
    /// Archive-relative paths of symlinked directories that were left out.
    /// Named rather than counted so the warning points at the link to fix.
    pub skipped_directory_symlinks: Vec<String>,
}

/// Walk `directory` and select the files to archive.
///
/// Symlinks to directories are skipped rather than followed, so a
/// self-referential link cannot turn the walk into an infinite recursion; they
/// are reported in `skipped_directory_symlinks` so the omission is visible
/// instead of silently shrinking the image. Entries are returned in sorted
/// archive order so the same directory always produces the same archive layout.
pub fn collect_entries(
    directory: &Path,
    include_symbol_files: bool,
) -> Result<ArchiveManifest, AmsUploadError> {
    let mut manifest = ArchiveManifest::default();
    walk(directory, directory, include_symbol_files, &mut manifest)?;
    manifest
        .entries
        .sort_by(|left, right| left.archive_path.cmp(&right.archive_path));
    manifest.skipped_directory_symlinks.sort();
    manifest.total_bytes = manifest.entries.iter().map(|entry| entry.size_bytes).sum();
    Ok(manifest)
}

/// Write `entries` into a gzipped tar at `destination` and return its size.
///
/// Files are streamed into the encoder one at a time, so peak memory is a
/// buffer rather than the archive.
pub fn build_archive(entries: &[ArchiveEntry], destination: &Path) -> Result<u64, AmsUploadError> {
    let file = std::fs::File::create(destination).map_err(|e| {
        AmsUploadError::io(format!("Failed to create {}", destination.display()), e)
    })?;
    let encoder = GzEncoder::new(std::io::BufWriter::new(file), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.follow_symlinks(true);

    for entry in entries {
        let mut source = std::fs::File::open(&entry.path).map_err(|e| {
            AmsUploadError::io(format!("Failed to open {}", entry.path.display()), e)
        })?;
        builder
            .append_file(&entry.archive_path, &mut source)
            .map_err(|e| {
                AmsUploadError::io(format!("Failed to archive {}", entry.path.display()), e)
            })?;
    }

    let encoder = builder
        .into_inner()
        .map_err(|e| AmsUploadError::io("Failed to finalise the archive", e))?;
    let writer = encoder
        .finish()
        .map_err(|e| AmsUploadError::io("Failed to compress the archive", e))?;
    drop(writer);

    let size = std::fs::metadata(destination)
        .map_err(|e| AmsUploadError::io(format!("Failed to stat {}", destination.display()), e))?
        .len();
    Ok(size)
}

/// Recursively add the files under `current` to `manifest`.
fn walk(
    root: &Path,
    current: &Path,
    include_symbol_files: bool,
    manifest: &mut ArchiveManifest,
) -> Result<(), AmsUploadError> {
    let entries = std::fs::read_dir(current)
        .map_err(|e| AmsUploadError::io(format!("Failed to read {}", current.display()), e))?;

    for entry in entries {
        let entry = entry
            .map_err(|e| AmsUploadError::io(format!("Failed to read {}", current.display()), e))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| AmsUploadError::io(format!("Failed to inspect {}", path.display()), e))?;

        if file_type.is_dir() {
            walk(root, &path, include_symbol_files, manifest)?;
            continue;
        }
        if !include_symbol_files && is_symbol_file(&path) {
            manifest.excluded_symbol_file_count += 1;
            continue;
        }
        // Symlinks are archived as the file they point at, matching the Go
        // implementation; a dangling link is a hard error rather than a
        // silently truncated image.
        let metadata = std::fs::metadata(&path)
            .map_err(|e| AmsUploadError::io(format!("Failed to stat {}", path.display()), e))?;
        if metadata.is_dir() {
            manifest
                .skipped_directory_symlinks
                .push(archive_path(root, &path)?);
            continue;
        }
        manifest.entries.push(ArchiveEntry {
            archive_path: archive_path(root, &path)?,
            path,
            size_bytes: metadata.len(),
        });
    }
    Ok(())
}

/// Express `path` relative to `root` using `/` separators.
fn archive_path(root: &Path, path: &Path) -> Result<String, AmsUploadError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        AmsUploadError::io(
            format!("{} is outside the upload directory", path.display()),
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path escapes the root"),
        )
    })?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

/// Whether the path's extension marks it as a debug-symbol file.
fn is_symbol_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| SYMBOL_FILE_EXTENSIONS.contains(&extension))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Create `name` (with any parent directories) under `directory`.
    fn write_file(directory: &Path, name: &str, contents: &[u8]) {
        let path = directory.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(&path)
            .unwrap()
            .write_all(contents)
            .unwrap();
    }

    /// Build a directory tree that mirrors armada-cli's `testdata/a1` fixture.
    fn sample_tree() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path(), "foo.txt", b"foo");
        write_file(temp.path(), "bar.exe", b"bar");
        write_file(temp.path(), "noLink.7z", b"7z");
        write_file(temp.path(), "b2/bar.exe", b"nested");
        write_file(temp.path(), "b2/c2/foo.txt", b"deep");
        temp
    }

    #[test]
    fn test_collect_entries_is_recursive_and_sorted() {
        let temp = sample_tree();
        let manifest = collect_entries(temp.path(), false).unwrap();
        let paths: Vec<_> = manifest
            .entries
            .iter()
            .map(|entry| entry.archive_path.as_str())
            .collect();
        assert_eq!(
            paths,
            vec![
                "b2/bar.exe",
                "b2/c2/foo.txt",
                "bar.exe",
                "foo.txt",
                "noLink.7z"
            ]
        );
        assert_eq!(manifest.total_bytes, 3 + 3 + 2 + 6 + 4);
    }

    /// AMS launches the image with `./<entrypoint>`, so an archive that loses
    /// the executable bit produces an image that cannot start.
    #[cfg(unix)]
    #[test]
    fn test_build_archive_preserves_the_executable_bit() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path(), "server", b"#!/bin/sh\n");
        write_file(temp.path(), "data.txt", b"data");
        std::fs::set_permissions(
            temp.path().join("server"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        let output = tempfile::tempdir().unwrap();
        let archive_path = output.path().join("image.tar.gz");
        let manifest = collect_entries(temp.path(), false).unwrap();
        build_archive(&manifest.entries, &archive_path).unwrap();

        let decoder = flate2::read::GzDecoder::new(std::fs::File::open(&archive_path).unwrap());
        let modes: std::collections::BTreeMap<String, u32> = tar::Archive::new(decoder)
            .entries()
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.path().unwrap().to_string_lossy().into_owned(),
                    entry.header().mode().unwrap(),
                )
            })
            .collect();
        assert_eq!(modes["server"] & 0o777, 0o755);
        assert_eq!(
            modes["data.txt"] & 0o111,
            0,
            "data files stay non-executable"
        );
    }

    #[test]
    fn test_symbol_files_are_excluded_unless_requested() {
        let temp = sample_tree();
        // The uppercase case is a separate basename because `client.PDB` and
        // `server.pdb` would be the same file on a case-insensitive filesystem.
        for name in ["server.pdb", "client.PDB", "server.sym", "server.debug"] {
            write_file(temp.path(), name, b"symbols");
        }
        let excluded = collect_entries(temp.path(), false).unwrap();
        assert_eq!(excluded.entries.len(), 5);
        assert_eq!(excluded.excluded_symbol_file_count, 4);

        let included = collect_entries(temp.path(), true).unwrap();
        assert_eq!(included.entries.len(), 9);
        assert_eq!(included.excluded_symbol_file_count, 0);
    }

    #[test]
    fn test_symbol_extension_match_is_exact() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path(), "server.symbols", b"not a symbol file");
        write_file(temp.path(), "server.Pdb", b"not a symbol file either");
        let manifest = collect_entries(temp.path(), false).unwrap();
        assert_eq!(manifest.entries.len(), 2);
        assert_eq!(manifest.excluded_symbol_file_count, 0);
    }

    /// A symlinked subdirectory is left out — following it risks recursing
    /// forever — but it must be named, or the image ships short of whatever it
    /// held with nothing to point at.
    #[cfg(unix)]
    #[test]
    fn test_skipped_directory_symlinks_are_named() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path(), "server", b"binary");
        let outside = tempfile::tempdir().unwrap();
        write_file(outside.path(), "asset.pak", b"assets");
        std::os::unix::fs::symlink(outside.path(), temp.path().join("assets")).unwrap();

        let manifest = collect_entries(temp.path(), false).unwrap();
        assert_eq!(manifest.skipped_directory_symlinks, vec!["assets"]);
        assert_eq!(manifest.entries.len(), 1, "only the real file is archived");
    }

    /// Symlinked *files* stay followed: build systems link shared artifacts into
    /// the tree, and the fleet needs the bytes, not a link it cannot resolve.
    #[cfg(unix)]
    #[test]
    fn test_symlinked_file_is_archived_as_its_target() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write_file(outside.path(), "libshared.so", b"shared library");
        std::os::unix::fs::symlink(
            outside.path().join("libshared.so"),
            temp.path().join("libshared.so"),
        )
        .unwrap();

        let manifest = collect_entries(temp.path(), false).unwrap();
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].archive_path, "libshared.so");
        assert_eq!(manifest.total_bytes, 14);
    }

    #[test]
    fn test_build_archive_round_trips_the_tree() {
        let temp = sample_tree();
        let output = tempfile::tempdir().unwrap();
        let archive_path = output.path().join("image.tar.gz");
        let manifest = collect_entries(temp.path(), false).unwrap();
        let size = build_archive(&manifest.entries, &archive_path).unwrap();
        assert!(size > 0);

        let decoder = flate2::read::GzDecoder::new(std::fs::File::open(&archive_path).unwrap());
        let mut archive = tar::Archive::new(decoder);
        let mut names: Vec<String> = archive
            .entries()
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "b2/bar.exe",
                "b2/c2/foo.txt",
                "bar.exe",
                "foo.txt",
                "noLink.7z"
            ]
        );
    }
}
