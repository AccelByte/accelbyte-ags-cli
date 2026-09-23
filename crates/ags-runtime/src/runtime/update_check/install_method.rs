//! Install method detection for the update command.
//!
//! Determines how the current copy of the CLI was installed by examining the
//! path of the running executable and the installer receipt, when present.
//! The detection is a pure function of its inputs so it is fully unit-testable
//! with plain, non-existent paths.

use std::path::{Path, PathBuf};

use ags_protocol::output_views::InstallMethod;

/// Path to the installer receipt, if the platform-conventional location
/// resolves.
///
/// The receipt is written by both installer scripts as
/// `accelbyte-ags-cli-receipt.json` in the receipt home:
/// `%LOCALAPPDATA%\accelbyte-ags-cli` on Windows, otherwise
/// `$XDG_CONFIG_HOME/accelbyte-ags-cli` when `XDG_CONFIG_HOME` is set,
/// otherwise `~/.config/accelbyte-ags-cli`.
pub fn receipt_path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var("LOCALAPPDATA").ok().map(PathBuf::from)
    } else {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
    };
    base.map(|b| {
        b.join("accelbyte-ags-cli")
            .join("accelbyte-ags-cli-receipt.json")
    })
}

/// Parse only the `install_prefix` string field from the receipt JSON at
/// `path`. Any I/O or parse failure yields `None`.
pub fn read_receipt_prefix(path: &Path) -> Option<PathBuf> {
    let data = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&data).ok()?;
    value
        .get("install_prefix")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
}

/// Normalise a path to a string with forward slashes, for cross-platform
/// comparison. Canonicalises when the path exists on disk; otherwise falls
/// back to the `to_string_lossy` representation.
fn normalise(path: &Path) -> String {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    resolved.to_string_lossy().replace('\\', "/")
}

/// Detect how this copy of the CLI was installed. Pure function of the
/// executable path and the optional receipt prefix.
///
/// Decision table (evaluated in order):
/// 1. Any path component of `exe` is `Cellar`, or `exe` starts with a known
///    Homebrew prefix → `Homebrew`.
/// 2. `receipt_prefix` is present and the parent directory of `exe` equals
///    `receipt_prefix` or `receipt_prefix/bin` → `Installer`.
/// 3. Otherwise → `Manual`.
pub fn detect_install_method(exe: &Path, receipt_prefix: Option<&Path>) -> InstallMethod {
    let exe_str = normalise(exe);

    // 1. Homebrew check: any path component is "Cellar", or known prefixes.
    //    The prefix checks catch symlinked binaries (e.g. /opt/homebrew/bin/ags)
    //    whose paths lack a "Cellar" component and are not canonicalized on disk.
    let is_homebrew = exe_str.split('/').any(|c| c == "Cellar")
        || exe_str.starts_with("/opt/homebrew/")
        || exe_str.starts_with("/home/linuxbrew/.linuxbrew/");
    if is_homebrew {
        return InstallMethod::Homebrew;
    }

    // 2. Receipt prefix check: parent of exe matches prefix or prefix/bin.
    if let Some(prefix) = receipt_prefix {
        let prefix_str = normalise(prefix);
        let prefix_trimmed = prefix_str.trim_end_matches('/');
        let prefix_bin = format!("{prefix_trimmed}/bin");

        if let Some(slash_pos) = exe_str.rfind('/') {
            let parent = &exe_str[..slash_pos];
            if parent == prefix_trimmed || parent == prefix_bin {
                return InstallMethod::Installer;
            }
        }
    }

    // 3. Otherwise manual.
    InstallMethod::Manual
}

/// Read `std::env::current_exe()` and the receipt, returning the detected
/// method and the binary path.
pub fn install_method() -> (InstallMethod, PathBuf) {
    // Fall back to a sentinel so the human renderer can detect the unknown
    // case and avoid printing a misleading path in the manual instruction.
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("<unknown>"));
    let prefix = receipt_path().and_then(|p| read_receipt_prefix(&p));
    let method = detect_install_method(&exe, prefix.as_deref());
    (method, exe)
}

/// The exact upgrade command for a given install method and OS, or `None`
/// for a manual install. `os` is `std::env::consts::OS` or a test value.
pub fn upgrade_command(method: InstallMethod, os: &str) -> Option<String> {
    match method {
        InstallMethod::Installer => {
            if os == "windows" {
                Some(r#"powershell -ExecutionPolicy Bypass -c "irm https://github.com/AccelByte/accelbyte-ags-cli/releases/latest/download/accelbyte-ags-cli-installer.ps1 | iex""#.to_string())
            } else {
                Some("curl --proto '=https' --tlsv1.2 -LsSf https://github.com/AccelByte/accelbyte-ags-cli/releases/latest/download/accelbyte-ags-cli-installer.sh | sh".to_string())
            }
        }
        InstallMethod::Homebrew => Some("brew upgrade accelbyte/tap/ags-cli".to_string()),
        InstallMethod::Manual => None,
    }
}

/// The download archive name for a given OS, architecture, and C library.
///
/// Returns the platform-specific archive name from the `dist-workspace.toml`
/// target matrix. On Linux, `is_musl` selects the `unknown-linux-musl` triple
/// instead of `unknown-linux-gnu`. Other OSes ignore the flag. Unknown
/// combinations yield `accelbyte-ags-cli-<arch>-<os>` with no extension.
pub fn download_archive(os: &str, arch: &str, is_musl: bool) -> String {
    match (os, arch) {
        ("windows", "x86_64") => "accelbyte-ags-cli-x86_64-pc-windows-msvc.zip".to_string(),
        ("macos", a) => format!("accelbyte-ags-cli-{a}-apple-darwin.tar.xz"),
        ("linux", a @ ("x86_64" | "aarch64")) => {
            let env = if is_musl { "musl" } else { "gnu" };
            format!("accelbyte-ags-cli-{a}-unknown-linux-{env}.tar.xz")
        }
        (o, a) => format!("accelbyte-ags-cli-{a}-{o}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_install_method_prefers_homebrew_cellar_path() {
        // A Cellar path yields homebrew even when a receipt prefix matches.
        let exe = Path::new("/opt/homebrew/Cellar/ags-cli/0.5.0/bin/ags");
        let receipt_prefix = Some(Path::new("/home/user/.cargo"));
        assert_eq!(
            detect_install_method(exe, receipt_prefix),
            InstallMethod::Homebrew,
        );
    }

    #[test]
    fn detect_install_method_matches_homebrew_prefix_without_cellar() {
        // Symlinked Homebrew binaries (e.g. /opt/homebrew/bin/ags) have no
        // "Cellar" component; the prefix check must still detect them.
        let exe = Path::new("/opt/homebrew/bin/ags");
        assert_eq!(detect_install_method(exe, None), InstallMethod::Homebrew,);

        // Linuxbrew equivalent.
        let exe_linux = Path::new("/home/linuxbrew/.linuxbrew/bin/ags");
        assert_eq!(
            detect_install_method(exe_linux, None),
            InstallMethod::Homebrew,
        );
    }

    #[test]
    fn detect_install_method_matches_receipt_prefix_bin() {
        // exe under <prefix>/bin with a receipt yields installer.
        let exe = Path::new("/home/user/.cargo/bin/ags");
        let receipt_prefix = Some(Path::new("/home/user/.cargo"));
        assert_eq!(
            detect_install_method(exe, receipt_prefix),
            InstallMethod::Installer,
        );

        // Also works with Windows-style separators (normalization).
        let exe_win = Path::new(r"C:\Users\me\.cargo\bin\ags.exe");
        let prefix_win = Path::new(r"C:\Users\me\.cargo");
        assert_eq!(
            detect_install_method(exe_win, Some(prefix_win)),
            InstallMethod::Installer,
        );
    }

    #[test]
    fn detect_install_method_matches_receipt_prefix_flat() {
        // exe directly under <prefix> yields installer.
        let exe = Path::new("/home/user/.cargo/ags");
        let receipt_prefix = Some(Path::new("/home/user/.cargo"));
        assert_eq!(
            detect_install_method(exe, receipt_prefix),
            InstallMethod::Installer,
        );
    }

    #[test]
    fn detect_install_method_ignores_receipt_for_relocated_binary() {
        // Receipt present, exe elsewhere, yields manual.
        let exe = Path::new("/usr/local/bin/ags");
        let receipt_prefix = Some(Path::new("/home/user/.cargo"));
        assert_eq!(
            detect_install_method(exe, receipt_prefix),
            InstallMethod::Manual,
        );
    }

    #[test]
    fn detect_install_method_without_receipt_is_manual() {
        // No receipt, plain path, yields manual.
        let exe = Path::new("/usr/local/bin/ags");
        assert_eq!(detect_install_method(exe, None), InstallMethod::Manual);
    }

    #[test]
    fn upgrade_command_per_method_and_os() {
        // Installer on non-Windows produces the shell one-liner.
        let unix_cmd = upgrade_command(InstallMethod::Installer, "linux").unwrap();
        assert_eq!(
            unix_cmd,
            "curl --proto '=https' --tlsv1.2 -LsSf https://github.com/AccelByte/accelbyte-ags-cli/releases/latest/download/accelbyte-ags-cli-installer.sh | sh"
        );
        let macos_cmd = upgrade_command(InstallMethod::Installer, "macos").unwrap();
        assert_eq!(macos_cmd, unix_cmd);

        // Installer on Windows produces the PowerShell one-liner.
        let win_cmd = upgrade_command(InstallMethod::Installer, "windows").unwrap();
        assert_eq!(
            win_cmd,
            r#"powershell -ExecutionPolicy Bypass -c "irm https://github.com/AccelByte/accelbyte-ags-cli/releases/latest/download/accelbyte-ags-cli-installer.ps1 | iex""#
        );

        // Homebrew produces the brew upgrade command (OS-independent).
        assert_eq!(
            upgrade_command(InstallMethod::Homebrew, "macos").as_deref(),
            Some("brew upgrade accelbyte/tap/ags-cli"),
        );
        assert_eq!(
            upgrade_command(InstallMethod::Homebrew, "linux").as_deref(),
            Some("brew upgrade accelbyte/tap/ags-cli"),
        );

        // Manual returns None.
        assert!(upgrade_command(InstallMethod::Manual, "linux").is_none());
        assert!(upgrade_command(InstallMethod::Manual, "windows").is_none());
    }

    #[test]
    fn download_archive_name_covers_the_seven_targets() {
        // Windows x86_64 (is_musl ignored).
        assert_eq!(
            download_archive("windows", "x86_64", false),
            "accelbyte-ags-cli-x86_64-pc-windows-msvc.zip"
        );
        // macOS (is_musl ignored).
        assert_eq!(
            download_archive("macos", "x86_64", false),
            "accelbyte-ags-cli-x86_64-apple-darwin.tar.xz"
        );
        assert_eq!(
            download_archive("macos", "aarch64", false),
            "accelbyte-ags-cli-aarch64-apple-darwin.tar.xz"
        );
        // Linux glibc.
        assert_eq!(
            download_archive("linux", "x86_64", false),
            "accelbyte-ags-cli-x86_64-unknown-linux-gnu.tar.xz"
        );
        assert_eq!(
            download_archive("linux", "aarch64", false),
            "accelbyte-ags-cli-aarch64-unknown-linux-gnu.tar.xz"
        );
        // Linux musl.
        assert_eq!(
            download_archive("linux", "x86_64", true),
            "accelbyte-ags-cli-x86_64-unknown-linux-musl.tar.xz"
        );
        assert_eq!(
            download_archive("linux", "aarch64", true),
            "accelbyte-ags-cli-aarch64-unknown-linux-musl.tar.xz"
        );
        // Fallback: no extension for unknown combos.
        assert_eq!(
            download_archive("freebsd", "x86_64", false),
            "accelbyte-ags-cli-x86_64-freebsd"
        );
        assert_eq!(
            download_archive("linux", "riscv64", false),
            "accelbyte-ags-cli-riscv64-linux"
        );
    }
}
