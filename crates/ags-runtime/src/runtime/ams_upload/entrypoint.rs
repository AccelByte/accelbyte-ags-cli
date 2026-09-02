//! Pre-flight validation of the image name, upload directory, and entrypoint.
//!
//! This is the half of the port that needs no wire contract: it decides
//! whether AMS will be able to run what we are about to ship, and it encodes
//! three field bugs found through armada-cli — a case-mismatched executable
//! that only fails once it reaches Linux, a binary built for the wrong class or
//! machine, and a launch script saved with Windows line endings.

use std::io::Read;
use std::path::{Path, PathBuf};

use ags_protocol::output::AmsEntrypointKind;

use super::elf;
use super::errors::AmsUploadError;
use super::TargetArchitecture;

/// Inclusive image-name length bounds enforced by AMS (`api.ImageNameMinLength`
/// / `ImageNameMaxLength`). Length is the only rule — there is no character-set
/// or pattern constraint.
const IMAGE_NAME_MIN_LENGTH: usize = 3;
const IMAGE_NAME_MAX_LENGTH: usize = 128;

/// A validated entrypoint: how it will be launched and which architecture the
/// image is tagged with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEntrypoint {
    pub kind: AmsEntrypointKind,
    pub architecture: TargetArchitecture,
    /// The launch command recorded on the image, always relative to the
    /// archive root, e.g. `./server` or `./bin/start.sh`.
    pub command: String,
}

/// Reject an image name AMS would refuse. Byte length, matching the Go check.
pub fn validate_image_name(name: &str) -> Result<(), AmsUploadError> {
    if name.len() < IMAGE_NAME_MIN_LENGTH {
        return Err(AmsUploadError::ImageNameTooShort {
            min: IMAGE_NAME_MIN_LENGTH,
        });
    }
    if name.len() > IMAGE_NAME_MAX_LENGTH {
        return Err(AmsUploadError::ImageNameTooLong {
            max: IMAGE_NAME_MAX_LENGTH,
        });
    }
    Ok(())
}

/// Reject an upload directory that is missing, not a directory, or empty.
pub fn validate_directory(directory: &Path) -> Result<(), AmsUploadError> {
    let metadata = std::fs::metadata(directory)
        .map_err(|_| AmsUploadError::DirectoryMissing(directory.to_path_buf()))?;
    if !metadata.is_dir() {
        return Err(AmsUploadError::DirectoryNotADirectory(
            directory.to_path_buf(),
        ));
    }
    let mut entries = std::fs::read_dir(directory)
        .map_err(|e| AmsUploadError::io(format!("Failed to read {}", directory.display()), e))?;
    if entries.next().is_none() {
        return Err(AmsUploadError::DirectoryEmpty(directory.to_path_buf()));
    }
    Ok(())
}

/// Validate the entrypoint inside `directory` and settle on a target
/// architecture.
///
/// A `.sh` entrypoint cannot be probed, so `declared_architecture` is required
/// there; an ELF entrypoint auto-detects and only cross-checks a declared
/// value.
pub fn resolve_entrypoint(
    directory: &Path,
    executable: &str,
    declared_architecture: Option<TargetArchitecture>,
    skip_script_validation: bool,
) -> Result<ResolvedEntrypoint, AmsUploadError> {
    let command = relative_launch_command(executable)?;
    // Resolve the file from the *normalised* command, not the raw flag, so a
    // Windows-style `bin\server` looks up the same file it records as
    // `./bin/server` — on Linux the raw form is one filename containing a
    // backslash.
    let executable_path = directory.join(command.trim_start_matches("./"));
    check_filename_case(&executable_path)?;

    if has_shell_script_extension(&executable_path) {
        let architecture =
            declared_architecture.ok_or(AmsUploadError::ShellScriptNeedsTargetArchitecture)?;
        if !skip_script_validation {
            validate_shell_script(&executable_path)?;
        }
        return Ok(ResolvedEntrypoint {
            kind: AmsEntrypointKind::ShellScript,
            architecture,
            command,
        });
    }

    let detected = detect_binary_architecture(&executable_path)?
        .ok_or(AmsUploadError::ExecutableNotAcceptedBinary)?;
    if let Some(requested) = declared_architecture {
        if requested != detected {
            return Err(AmsUploadError::ArchitectureMismatch {
                requested: requested.wire_value().to_string(),
                detected: detected.wire_value().to_string(),
            });
        }
    }
    Ok(ResolvedEntrypoint {
        kind: AmsEntrypointKind::ElfBinary,
        architecture: detected,
        command,
    })
}

/// Reject a shell script that AMS's Linux hosts would fail to execute: CR or
/// CRLF line endings, or a missing `#!` line.
///
/// The shebang may sit on any line, not only the first — armada-cli accepts a
/// script with leading comment lines, and its fixtures pin that behaviour.
pub fn validate_shell_script(path: &Path) -> Result<(), AmsUploadError> {
    if !has_shell_script_extension(path) {
        return Err(AmsUploadError::ShellScriptInvalid {
            path: path.to_path_buf(),
            reason: "file extension is not .sh".to_string(),
        });
    }

    let file = std::fs::File::open(path)
        .map_err(|e| AmsUploadError::io(format!("Failed to open {}", path.display()), e))?;
    let mut reader = std::io::BufReader::new(file);
    let mut buffer = [0u8; 8192];

    let mut line = 1usize;
    let mut column = 0usize;
    let mut previous_byte = 0u8;
    let mut has_shebang = false;

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| AmsUploadError::io(format!("Failed to read {}", path.display()), e))?;
        if read == 0 {
            break;
        }
        for &byte in &buffer[..read] {
            column += 1;
            if byte == b'\r' {
                return Err(AmsUploadError::ShellScriptInvalid {
                    path: path.to_path_buf(),
                    reason: format!(
                        "CR or CRLF line ending at line {line}, column {column}; expected LF"
                    ),
                });
            }
            if byte == b'\n' {
                column = 0;
                line += 1;
                continue;
            }
            if column == 2 && previous_byte == b'#' && byte == b'!' {
                has_shebang = true;
            }
            previous_byte = byte;
        }
    }

    if !has_shebang {
        return Err(AmsUploadError::ShellScriptInvalid {
            path: path.to_path_buf(),
            reason: "no shebang (#!) line found".to_string(),
        });
    }
    Ok(())
}

/// Confirm the executable exists as a file whose on-disk name matches
/// `path`'s final component byte-for-byte.
///
/// Stat alone is not enough: Windows and macOS filesystems are case-insensitive
/// and happily resolve `Server` to `server`, which then fails on the
/// case-sensitive Linux host that runs the image (ref JA-645).
fn check_filename_case(path: &Path) -> Result<(), AmsUploadError> {
    let metadata = std::fs::metadata(path)
        .map_err(|_| AmsUploadError::ExecutableMissing(path.to_path_buf()))?;
    if metadata.is_dir() {
        return Err(AmsUploadError::ExecutableIsDirectory(path.to_path_buf()));
    }

    let file_name = path
        .file_name()
        .ok_or_else(|| AmsUploadError::ExecutableMissing(path.to_path_buf()))?;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    let parent = parent.unwrap_or_else(|| Path::new("."));
    let entries = std::fs::read_dir(parent)
        .map_err(|e| AmsUploadError::io(format!("Failed to read {}", parent.display()), e))?;

    for entry in entries.flatten() {
        if entry.file_name() == file_name {
            return Ok(());
        }
    }
    Err(AmsUploadError::ExecutableWrongCase(path.to_path_buf()))
}

/// Read just enough of the file to run the ELF accept matrix over it.
fn detect_binary_architecture(path: &Path) -> Result<Option<TargetArchitecture>, AmsUploadError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| AmsUploadError::io(format!("Failed to open {}", path.display()), e))?;
    let mut header = vec![0u8; elf::header_prefix_len()];
    let mut filled = 0usize;
    while filled < header.len() {
        let read = file
            .read(&mut header[filled..])
            .map_err(|e| AmsUploadError::io(format!("Failed to read {}", path.display()), e))?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    header.truncate(filled);
    Ok(elf::detect_architecture(&header))
}

/// Normalise the user-supplied executable into the `./<path>` launch command
/// AMS records on the image.
fn relative_launch_command(executable: &str) -> Result<String, AmsUploadError> {
    let normalised = executable.replace('\\', "/");
    if normalised.trim().is_empty() {
        return Err(AmsUploadError::ExecutableMissing(PathBuf::from(executable)));
    }
    if normalised.starts_with('/') {
        return Err(AmsUploadError::ExecutableOutsideDirectory(
            executable.to_string(),
        ));
    }

    let mut segments: Vec<&str> = Vec::new();
    for segment in normalised.split('/') {
        match segment {
            "" | "." => continue,
            ".." => {
                if segments.pop().is_none() {
                    return Err(AmsUploadError::ExecutableOutsideDirectory(
                        executable.to_string(),
                    ));
                }
            }
            other => segments.push(other),
        }
    }
    if segments.is_empty() {
        return Err(AmsUploadError::ExecutableMissing(PathBuf::from(executable)));
    }
    Ok(format!("./{}", segments.join("/")))
}

/// Whether the path's extension is exactly `.sh`.
fn has_shell_script_extension(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "sh")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write `contents` to `name` inside `directory` and return its path.
    fn write_file(directory: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let path = directory.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(contents).unwrap();
        path
    }

    /// A 20-byte little-endian 64-bit ELF header for the given `e_machine`.
    fn elf_bytes(machine: u16) -> Vec<u8> {
        let mut header = vec![0u8; 20];
        header[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        header[4] = 2;
        header[5] = 1;
        header[6] = 1;
        header[18..20].copy_from_slice(&machine.to_le_bytes());
        header
    }

    #[test]
    fn test_image_name_length_bounds() {
        assert!(validate_image_name("ab").is_err());
        assert!(validate_image_name("abc").is_ok());
        assert!(validate_image_name(&"a".repeat(128)).is_ok());
        assert!(validate_image_name(&"a".repeat(129)).is_err());
    }

    #[test]
    fn test_directory_must_exist_and_be_non_empty() {
        let temp = tempfile::tempdir().unwrap();
        assert!(matches!(
            validate_directory(&temp.path().join("nope")),
            Err(AmsUploadError::DirectoryMissing(_))
        ));
        assert!(matches!(
            validate_directory(temp.path()),
            Err(AmsUploadError::DirectoryEmpty(_))
        ));
        write_file(temp.path(), "server", &elf_bytes(62));
        assert!(validate_directory(temp.path()).is_ok());
    }

    #[test]
    fn test_elf_entrypoint_detects_architecture() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path(), "server", &elf_bytes(183));
        let resolved = resolve_entrypoint(temp.path(), "server", None, false).unwrap();
        assert_eq!(resolved.kind, AmsEntrypointKind::ElfBinary);
        assert_eq!(resolved.architecture, TargetArchitecture::LinuxArm64);
        assert_eq!(resolved.command, "./server");
    }

    #[test]
    fn test_elf_entrypoint_rejects_conflicting_target_arch() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path(), "server", &elf_bytes(62));
        let error = resolve_entrypoint(
            temp.path(),
            "server",
            Some(TargetArchitecture::LinuxArm64),
            false,
        )
        .unwrap_err();
        assert!(matches!(error, AmsUploadError::ArchitectureMismatch { .. }));
    }

    #[test]
    fn test_non_elf_entrypoint_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        write_file(
            temp.path(),
            "server",
            b"MZ\x90\x00 not an ELF binary at all",
        );
        assert!(matches!(
            resolve_entrypoint(temp.path(), "server", None, false),
            Err(AmsUploadError::ExecutableNotAcceptedBinary)
        ));
    }

    #[test]
    fn test_missing_entrypoint_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path(), "other", b"x");
        assert!(matches!(
            resolve_entrypoint(temp.path(), "server", None, false),
            Err(AmsUploadError::ExecutableMissing(_))
        ));
    }

    #[test]
    fn test_directory_entrypoint_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("server")).unwrap();
        assert!(matches!(
            resolve_entrypoint(temp.path(), "server", None, false),
            Err(AmsUploadError::ExecutableIsDirectory(_))
        ));
    }

    #[test]
    fn test_shell_script_requires_target_architecture() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path(), "start.sh", b"#!/bin/bash\necho hi\n");
        assert!(matches!(
            resolve_entrypoint(temp.path(), "start.sh", None, false),
            Err(AmsUploadError::ShellScriptNeedsTargetArchitecture)
        ));
        let resolved = resolve_entrypoint(
            temp.path(),
            "start.sh",
            Some(TargetArchitecture::LinuxX86_64),
            false,
        )
        .unwrap();
        assert_eq!(resolved.kind, AmsEntrypointKind::ShellScript);
        assert_eq!(resolved.command, "./start.sh");
    }

    #[test]
    fn test_shell_script_validation_can_be_skipped() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path(), "start.sh", b"echo no shebang\n");
        assert!(resolve_entrypoint(
            temp.path(),
            "start.sh",
            Some(TargetArchitecture::LinuxX86_64),
            false
        )
        .is_err());
        assert!(resolve_entrypoint(
            temp.path(),
            "start.sh",
            Some(TargetArchitecture::LinuxX86_64),
            true
        )
        .is_ok());
    }

    // Fixtures ported from armada-cli `pkg/internal/entity/testdata`.
    #[test]
    fn test_shell_script_fixtures() {
        let temp = tempfile::tempdir().unwrap();
        let cases: &[(&str, &[u8], bool)] = &[
            ("test_valid.sh", b"#!/bin/bash\necho \"Hello, World!\"", true),
            (
                "test_valid_comments.sh",
                b"#this file has some comments\n#before the script\n#!/bin/bash\necho \"Hello, World!\"",
                true,
            ),
            (
                "test_invalid_windows_line_endings.sh",
                b"#!/bin/bash\r\necho \"Hello, World!\"\n",
                false,
            ),
            (
                "test_invalid_missing_shebang.sh",
                b"# test file\necho \"Hello, World!\"",
                false,
            ),
            (
                "test_invalid_missing_shebang_commented.sh",
                b"##!/bin/bash\necho \"Hello, World!\"",
                false,
            ),
            ("test_empty.sh", b"", false),
        ];
        for (name, contents, is_valid) in cases {
            let path = write_file(temp.path(), name, contents);
            assert_eq!(
                validate_shell_script(&path).is_ok(),
                *is_valid,
                "{name} should be {}",
                if *is_valid { "accepted" } else { "rejected" }
            );
        }

        let readme = write_file(temp.path(), "README.md", b"#!/bin/bash\n");
        assert!(
            validate_shell_script(&readme).is_err(),
            "a non-.sh file is rejected regardless of contents"
        );
    }

    /// armada's own tests pass the entrypoint as `./<name>`, so that form must
    /// resolve to the same file as the bare name.
    #[test]
    fn test_dot_slash_entrypoint_resolves() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path(), "server", &elf_bytes(62));
        let resolved = resolve_entrypoint(temp.path(), "./server", None, false).unwrap();
        assert_eq!(resolved.command, "./server");
    }

    /// A Windows-style path must resolve the file it records, not a single
    /// filename containing a backslash.
    #[test]
    fn test_backslash_entrypoint_resolves_the_recorded_file() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path(), "bin/server", &elf_bytes(62));
        let resolved = resolve_entrypoint(temp.path(), "bin\\server", None, false).unwrap();
        assert_eq!(resolved.command, "./bin/server");
    }

    #[test]
    fn test_launch_command_normalisation() {
        assert_eq!(relative_launch_command("server").unwrap(), "./server");
        assert_eq!(relative_launch_command("./server").unwrap(), "./server");
        assert_eq!(
            relative_launch_command("bin/./server").unwrap(),
            "./bin/server"
        );
        assert_eq!(
            relative_launch_command("bin\\server").unwrap(),
            "./bin/server"
        );
        assert!(matches!(
            relative_launch_command("/usr/bin/server"),
            Err(AmsUploadError::ExecutableOutsideDirectory(_))
        ));
        assert!(matches!(
            relative_launch_command("../server"),
            Err(AmsUploadError::ExecutableOutsideDirectory(_))
        ));
    }

    /// The case check must reject a name that only resolves because the host
    /// filesystem is case-insensitive. On a case-sensitive filesystem the stat
    /// itself fails first, which is the same refusal by a different route.
    #[test]
    fn test_wrong_case_entrypoint_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        write_file(temp.path(), "server", &elf_bytes(62));
        let error = resolve_entrypoint(temp.path(), "Server", None, false).unwrap_err();
        assert!(
            matches!(
                error,
                AmsUploadError::ExecutableWrongCase(_) | AmsUploadError::ExecutableMissing(_)
            ),
            "unexpected error: {error:?}"
        );
    }
}
