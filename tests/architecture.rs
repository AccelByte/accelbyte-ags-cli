//! Architecture-level tests that verify cross-module rules.
//!
//! These tests fail if a structural invariant is violated. They are the
//! safety net for refactors that established the rule but cannot otherwise
//! prevent regressions (e.g. a future commit adding an `eprintln!` back
//! into a runtime file).

use std::fs;
use std::path::Path;

/// Substrings that must not appear in any `.rs` file under `src/runtime/`.
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

/// Directory whose `.rs` files must not contain any of the banned substrings.
const SCANNED_DIR: &str = "src/runtime";

/// The runtime layer must not write to stdout, stderr, or import the
/// `indicatif` spinner crate. User-visible progress goes through
/// `protocol::event::ProgressSink`; user-visible warnings go through
/// `ProgressEvent::Message`. Any direct print/eprintln/indicatif usage in
/// `src/runtime/` is an architectural regression.
#[test]
fn test_runtime_layer_has_no_user_facing_io() {
    let mut violations = Vec::new();
    walk_rust_files(Path::new(SCANNED_DIR), &mut violations);

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

/// Recursively walk a directory, scanning every `.rs` file.
fn walk_rust_files(dir: &Path, violations: &mut Vec<String>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("failed to read directory {}: {e}", dir.display()));

    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("failed to read entry in {}: {e}", dir.display()));
        let path = entry.path();
        if path.is_dir() {
            walk_rust_files(&path, violations);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            scan_file(&path, violations);
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
fn scan_file(path: &Path, violations: &mut Vec<String>) {
    let contents = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read file {}: {e}", path.display()));

    for (line_index, line) in contents.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        for banned in BANNED_SUBSTRINGS {
            if line.contains(banned) {
                violations.push(format!(
                    "{}:{}: contains banned substring `{banned}`",
                    path.display(),
                    line_index + 1,
                ));
            }
        }
    }
}

/// Per CONTRIBUTING.md, every `#[test]` and `#[tokio::test]` function must
/// be named `test_<scenario>` so test output groups them visibly.
#[test]
fn test_test_functions_use_test_prefix() {
    let mut violations = Vec::new();
    walk_rust_files_for_test_naming(Path::new("src"), &mut violations);
    walk_rust_files_for_test_naming(Path::new("tests"), &mut violations);

    if !violations.is_empty() {
        panic!(
            "#[test] / #[tokio::test] functions must be named `test_<scenario>`.\n\
             \n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }
}

/// Per CONTRIBUTING.md, every function (public or private, including
/// test-helper functions) must have a `///` documentation comment.
///
/// Two categories are excluded:
/// - `#[test]` and `#[tokio::test]` functions, exempt by CONTRIBUTING.md
///   because the function name describes the scenario.
/// - Trait-impl methods (functions inside `impl <Trait> for <Type>` blocks),
///   because they inherit documentation from the trait definition.
#[test]
fn test_functions_have_documentation_headers() {
    let mut violations = Vec::new();
    walk_rust_files_for_documentation_headers(Path::new("src"), &mut violations);

    if !violations.is_empty() {
        panic!(
            "Functions must have a `///` documentation comment.\n\
             \n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }
}

/// Recursively walk `directory`, collecting `#[test]` / `#[tokio::test]`
/// functions whose name does not start with `test_`.
fn walk_rust_files_for_test_naming(directory: &Path, violations: &mut Vec<String>) {
    visit_rust_files(directory, &mut |path| scan_test_naming(path, violations));
}

/// Recursively walk `directory`, collecting functions missing a
/// documentation comment (excluding `#[test]` functions and trait-impl
/// methods).
fn walk_rust_files_for_documentation_headers(directory: &Path, violations: &mut Vec<String>) {
    visit_rust_files(directory, &mut |path| {
        scan_documentation_headers(path, violations)
    });
}

/// Generic recursive-walk helper: invokes `visit` on every `.rs` file
/// reachable from `directory`.
fn visit_rust_files(directory: &Path, visit: &mut dyn FnMut(&Path)) {
    if !directory.exists() {
        return;
    }
    let entries = fs::read_dir(directory).unwrap_or_else(|error| {
        panic!("failed to read directory {}: {error}", directory.display())
    });
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("failed to read entry in {}: {error}", directory.display())
        });
        let path = entry.path();
        if path.is_dir() {
            visit_rust_files(&path, visit);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            visit(&path);
        }
    }
}

/// Read `path` and append any `#[test]` / `#[tokio::test]` function not
/// named `test_<…>` to `violations`.
fn scan_test_naming(path: &Path, violations: &mut Vec<String>) {
    let contents = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read file {}: {error}", path.display()));
    let lines: Vec<&str> = contents.lines().collect();

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !(trimmed.starts_with("#[test]") || trimmed.starts_with("#[tokio::test]")) {
            continue;
        }
        let Some(function_name) = next_function_name(&lines, index + 1) else {
            continue;
        };
        if !function_name.starts_with("test_") {
            violations.push(format!(
                "{}:{}: #[test] function `{function_name}` does not start with `test_`",
                path.display(),
                index + 1,
            ));
        }
    }
}

/// Read `path` and append every function declaration missing a preceding
/// `///` documentation comment to `violations`.
///
/// Tracks brace depth so that functions inside `impl <Trait> for <Type>`
/// blocks can be skipped — those inherit documentation from the trait.
/// Functions annotated with `#[test]` or `#[tokio::test]` are also
/// skipped per CONTRIBUTING.md.
fn scan_documentation_headers(path: &Path, violations: &mut Vec<String>) {
    let contents = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read file {}: {error}", path.display()));
    let lines: Vec<&str> = contents.lines().collect();

    let mut brace_depth: usize = 0;
    let mut trait_impl_depths: Vec<usize> = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();

        if is_function_declaration(trimmed)
            && trait_impl_depths.is_empty()
            && !has_test_attribute_above(&lines, index)
            && !has_documentation_comment_above(&lines, index)
        {
            let function_name = parse_function_name(trimmed).unwrap_or("<unknown>");
            violations.push(format!(
                "{}:{}: function `{function_name}` missing `///` documentation comment",
                path.display(),
                index + 1,
            ));
        }

        if is_trait_impl_opening(trimmed) && line.contains('{') {
            trait_impl_depths.push(brace_depth + 1);
        }

        for character in line.chars() {
            match character {
                '{' => brace_depth = brace_depth.saturating_add(1),
                '}' => {
                    brace_depth = brace_depth.saturating_sub(1);
                    while let Some(&top) = trait_impl_depths.last() {
                        if brace_depth < top {
                            trait_impl_depths.pop();
                        } else {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Whether `trimmed` (a line with leading whitespace stripped) opens any
/// function declaration, regardless of visibility or `async`/`unsafe`/`const`
/// qualifiers.
fn is_function_declaration(trimmed: &str) -> bool {
    for prefix in [
        "fn ",
        "async fn ",
        "unsafe fn ",
        "const fn ",
        "pub fn ",
        "pub async fn ",
        "pub unsafe fn ",
        "pub const fn ",
        "pub(crate) fn ",
        "pub(crate) async fn ",
        "pub(crate) unsafe fn ",
        "pub(crate) const fn ",
        "pub(super) fn ",
        "pub(super) async fn ",
        "pub(super) unsafe fn ",
        "pub(super) const fn ",
    ] {
        if trimmed.starts_with(prefix) {
            return true;
        }
    }
    false
}

/// Whether `trimmed` opens an `impl <Trait> for <Type>` block. Multi-line
/// impl headers without ` for ` on the opening line are not detected — none
/// appear in this codebase today.
fn is_trait_impl_opening(trimmed: &str) -> bool {
    if !(trimmed.starts_with("impl ") || trimmed.starts_with("impl<")) {
        return false;
    }
    trimmed.contains(" for ")
}

/// Walk backwards from `function_index` and report whether any of the
/// immediately preceding attribute lines is a `#[test]` or `#[tokio::test]`
/// annotation.
fn has_test_attribute_above(lines: &[&str], function_index: usize) -> bool {
    let mut cursor = function_index;
    while cursor > 0 {
        cursor -= 1;
        let trimmed = lines[cursor].trim_start();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("#[test]") || trimmed.starts_with("#[tokio::test]") {
            return true;
        }
        if trimmed.starts_with("#[") || trimmed.starts_with("#![") {
            continue;
        }
        return false;
    }
    false
}

/// Walk backwards from `function_index` and report whether the nearest
/// non-attribute, non-blank line is a `///` documentation comment.
fn has_documentation_comment_above(lines: &[&str], function_index: usize) -> bool {
    let mut cursor = function_index;
    while cursor > 0 {
        cursor -= 1;
        let trimmed = lines[cursor].trim_start();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("#[") || trimmed.starts_with("#![") {
            continue;
        }
        return trimmed.starts_with("///");
    }
    false
}

/// Extract the function name following the `fn ` keyword on `trimmed`.
fn parse_function_name(trimmed: &str) -> Option<&str> {
    let after_keyword = trimmed.split("fn ").nth(1)?;
    let end = after_keyword
        .find(|character: char| !(character.is_alphanumeric() || character == '_'))
        .unwrap_or(after_keyword.len());
    Some(&after_keyword[..end])
}

/// Starting at `start`, scan forward for the first line containing `fn `
/// and return the function name. Returns `None` if no `fn` appears within
/// a small look-ahead window — `#[test]` annotations sit immediately above
/// their function, so scanning more than a few lines is unnecessary.
fn next_function_name<'a>(lines: &[&'a str], start: usize) -> Option<&'a str> {
    for line in lines.iter().skip(start).take(10) {
        let trimmed = line.trim_start();
        if !trimmed.contains("fn ") {
            continue;
        }
        return parse_function_name(trimmed);
    }
    None
}
