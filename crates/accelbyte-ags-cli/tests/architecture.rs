//! Architecture-level tests that verify cross-module rules.
//!
//! These tests fail if a structural invariant is violated. They are the
//! safety net for refactors that established the rule but cannot otherwise
//! prevent regressions (e.g. a future commit adding an `eprintln!` back
//! into a runtime file).

use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Runtime-layer guard
// ---------------------------------------------------------------------------

/// Substrings that must not appear in any `.rs` file under the runtime crate.
///
/// The opening parenthesis is part of each macro pattern so that doc-comment
/// references like `` `eprintln!`-based `` do not trigger false positives.
const BANNED_SUBSTRINGS: &[&str] = &[
    "print!(",
    "println!(",
    "eprint!(",
    "eprintln!(",
    "indicatif::",
];

/// Directories whose `.rs` files must not contain any of the banned
/// substrings. Covers every module in the `ags-runtime` crate — runtime
/// business logic, the spec catalogue, and shared support utilities — so
/// that no part of the library layer can regress to direct stdout/stderr
/// writes. Paths are resolved relative to this crate's manifest dir.
const SCANNED_DIRS: &[&str] = &[
    "../ags-runtime/src/runtime",
    "../ags-runtime/src/catalogue",
    "../ags-runtime/src/support",
];

/// The runtime layer must not write to stdout, stderr, or import the
/// `indicatif` spinner crate. User-visible progress goes through
/// `protocol::event::ProgressSink`; user-visible warnings go through
/// `ProgressEvent::Message`. Any direct print/eprintln/indicatif usage in
/// the `ags-runtime` crate is an architectural regression.
#[test]
fn test_runtime_layer_has_no_user_facing_io() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    let mut files_scanned = 0usize;
    for scanned_dir in SCANNED_DIRS {
        let path = manifest_dir.join(scanned_dir);
        assert!(
            path.is_dir(),
            "scanned directory does not exist: {} — has the workspace layout changed?",
            path.display()
        );
        walk_rust_files(
            &path,
            BANNED_SUBSTRINGS,
            &mut violations,
            &mut files_scanned,
        );
    }
    assert!(
        files_scanned > 0,
        "no .rs files found under {SCANNED_DIRS:?} — the architecture guard would silently pass"
    );

    if !violations.is_empty() {
        panic!(
            "Runtime layer must not contain direct stdout/stderr writes or `indicatif` imports.\n\
             The runtime layer emits user-visible events through `ProgressSink` instead.\n\
             \n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }
}

// ---------------------------------------------------------------------------
// CLI binary-crate guard
// ---------------------------------------------------------------------------

/// Substrings that must not appear in CLI binary source files.
///
/// Print macros only — `indicatif::` is intentionally excluded because the
/// CLI/UI layer legitimately owns spinner and progress-bar rendering; only
/// the runtime library forbids direct `indicatif` usage.
const CLI_BANNED_SUBSTRINGS: &[&str] = &["print!(", "println!(", "eprint!(", "eprintln!("];

/// Directories under the CLI binary crate whose `.rs` files are scanned.
/// Resolved relative to `CARGO_MANIFEST_DIR` (i.e. `crates/accelbyte-ags-cli`),
/// so `"src"` covers the entire binary source tree but not `tests/`.
const CLI_SCANNED_DIRS: &[&str] = &["src"];

/// All terminal output in the CLI binary must go through the house sink
/// helpers (`write_stdout_line`, `write_stderr_line`, `write_stderr` in
/// `frontend/mod.rs`), never bare `println!` / `eprintln!`. This ensures
/// machine-readable and non-interactive modes can suppress or redirect
/// output consistently.
#[test]
fn test_cli_binary_output_goes_through_sink_helpers() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    let mut files_scanned = 0usize;
    for scanned_dir in CLI_SCANNED_DIRS {
        let path = manifest_dir.join(scanned_dir);
        assert!(
            path.is_dir(),
            "scanned directory does not exist: {} — has the workspace layout changed?",
            path.display()
        );
        walk_rust_files(
            &path,
            CLI_BANNED_SUBSTRINGS,
            &mut violations,
            &mut files_scanned,
        );
    }
    assert!(
        files_scanned > 0,
        "no .rs files found under {CLI_SCANNED_DIRS:?} — the architecture guard would silently pass"
    );

    if !violations.is_empty() {
        panic!(
            "CLI binary must not contain bare print macros (println!, eprintln!, etc.).\n\
             Route all output through the sink helpers in `frontend/mod.rs`:\n\
             \x20 • write_stdout_line  — structured/data output to stdout\n\
             \x20 • write_stderr_line  — UI chrome, hints, and diagnostics to stderr\n\
             \x20 • write_stderr       — partial-line stderr writes\n\
             \n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Recursively walk a directory, scanning every `.rs` file for the given
/// banned substrings.
fn walk_rust_files(
    dir: &Path,
    banned: &[&str],
    violations: &mut Vec<String>,
    files_scanned: &mut usize,
) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("failed to read directory {}: {e}", dir.display()));

    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("failed to read entry in {}: {e}", dir.display()));
        let path = entry.path();
        if path.is_dir() {
            walk_rust_files(&path, banned, violations, files_scanned);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            *files_scanned += 1;
            scan_file(&path, banned, violations);
        }
    }
}

/// Scan a single Rust source file for banned substrings.
///
/// Lines whose first non-whitespace characters are `//` are treated as
/// comments and skipped. This excludes doc comments (`///`, `//!`) and plain
/// line comments. Lines with mid-line `//` comments are scanned in full —
/// the false-positive rate on banned macros appearing only in inline
/// comments is negligible.
fn scan_file(path: &Path, banned: &[&str], violations: &mut Vec<String>) {
    let contents = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read file {}: {e}", path.display()));

    for (line_index, line) in contents.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        for substring in banned {
            if line.contains(substring) {
                violations.push(format!(
                    "{}:{}: contains banned substring `{substring}`",
                    path.display(),
                    line_index + 1,
                ));
            }
        }
    }
}
