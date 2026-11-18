//! Architecture-level tests that verify cross-module rules.
//!
//! These tests fail if a structural invariant is violated. They are the
//! safety net for refactors that established the rule but cannot otherwise
//! prevent regressions (e.g. a future commit adding an `eprintln!` back
//! into a runtime file).

use std::fs;
use std::path::{Path, PathBuf};

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
// Tunnel Ctrl-C listener guard
// ---------------------------------------------------------------------------

/// The `ags extend tunnel` handler directory, relative to this crate's
/// manifest dir. Every `.rs` file under it is scanned.
const TUNNEL_DIR: &str = "src/invocation/handlers/extend/tunnel";

/// The file in [`TUNNEL_DIR`] that holds the tunnel's accept loop.
const TUNNEL_LOOP_FILE: &str = "bridge.rs";

/// The call that registers the tunnel's Ctrl-C listener before it listens.
const INTERRUPT_REGISTER_CALL: &str = "InterruptListener::register(";

/// The tunnel must stop on a Ctrl-C that arrives the moment it reports that
/// it is listening. It once printed its listening line before any Ctrl-C
/// listener existed: `tokio::signal::ctrl_c()` registers only when it is
/// first polled, so a SIGINT landing in that gap was lost, the tunnel kept
/// running, and a functional test waited until the CI job time limit. The
/// fix registers an `InterruptListener` before the listening line and waits
/// on `interrupt.recv()`.
///
/// That race cannot be produced on demand, so no behaviour test fails if a
/// refactor puts the lazy future back. This source check is the only CI
/// guard against it, and it runs on the Windows build too, where the SIGINT
/// functional tests do not. It also requires the loop file to register the
/// listener, so moving the loop to another file fails here instead of
/// passing on a scan that no longer covers it.
#[test]
fn test_tunnel_loop_has_no_lazy_ctrl_c() {
    let tunnel_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(TUNNEL_DIR);
    assert!(
        tunnel_dir.is_dir(),
        "tunnel directory does not exist: {}; has the handler moved?",
        tunnel_dir.display()
    );
    let mut files = Vec::new();
    collect_rust_files(&tunnel_dir, &mut files);
    assert!(
        !files.is_empty(),
        "no .rs files found under {}: the tunnel Ctrl-C guard would silently pass",
        tunnel_dir.display()
    );

    let loop_file = tunnel_dir.join(TUNNEL_LOOP_FILE);
    assert!(
        code_lines(&read_source(&loop_file))
            .any(|(_, line)| line.contains(INTERRUPT_REGISTER_CALL)),
        "{} has no `{INTERRUPT_REGISTER_CALL}` call in code. The tunnel loop must register its \
         Ctrl-C listener before it reports that it is listening; if the loop moved to another \
         file, point this guard at that file.",
        loop_file.display()
    );

    let mut violations = Vec::new();
    for path in &files {
        for line in lazy_ctrl_c_calls(&read_source(path)) {
            violations.push(format!("{}:{line}: calls `ctrl_c(`", path.display()));
        }
    }
    if !violations.is_empty() {
        panic!(
            "The tunnel must not wait on tokio's `ctrl_c()`: it registers only when first polled, \
             so a Ctrl-C sent right after the listening line is lost and the tunnel keeps running.\n\
             Wait on the `InterruptListener` registered before the listening line \
             (`interrupt.recv()`) instead.\n\
             \n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }
}

/// The matcher behind the tunnel Ctrl-C guard finds a call to `ctrl_c`
/// however its path is spelled, and ignores a `use` with no call, longer
/// identifiers that contain `ctrl_c`, and comment and doc lines. A matcher
/// that missed a spelling would let the guard pass with the lazy future back
/// in the loop.
#[test]
fn test_lazy_ctrl_c_matcher_finds_every_spelling() {
    let source = [
        "use tokio::signal::ctrl_c;",
        "async fn run(key: Key) {",
        "    tokio::signal::ctrl_c().await.ok();",
        "    signal::ctrl_c().await.ok();",
        "    ctrl_c().await.ok();",
        "    if is_ctrl_c(key) {}",
        "    test_ctrl_c_registration();",
        "    // tokio::signal::ctrl_c()",
        "    /// ctrl_c()",
        "}",
    ]
    .join("\n");

    assert_eq!(lazy_ctrl_c_calls(&source), vec![3, 4, 5]);
}

/// The 1-based line numbers of the code lines in `text` that call `ctrl_c`:
/// the identifier `ctrl_c` immediately followed by `(`, however the path to
/// it is spelled (`tokio::signal::ctrl_c()`, `signal::ctrl_c()`, or a bare
/// `ctrl_c()` after a `use`). A longer identifier that ends in `ctrl_c`, such
/// as `is_ctrl_c(`, is not a call to it. Comment lines are skipped as in
/// [`code_lines`]; a line is reported once however many calls it holds.
fn lazy_ctrl_c_calls(text: &str) -> Vec<usize> {
    const NAME: &str = "ctrl_c";
    code_lines(text)
        .filter(|(_, line)| {
            line.match_indices(NAME).any(|(at, _)| {
                let tail_of_longer_identifier = line[..at]
                    .chars()
                    .next_back()
                    .is_some_and(is_identifier_char);
                line[at + NAME.len()..].starts_with('(') && !tail_of_longer_identifier
            })
        })
        .map(|(line_number, _)| line_number)
        .collect()
}

// ---------------------------------------------------------------------------
// Tunnel SIGINT test wait guard
// ---------------------------------------------------------------------------

/// The functional tests that send SIGINT to `ags extend tunnel`, relative to
/// this crate's manifest dir.
const TUNNEL_SIGINT_TESTS: &str = "tests/functional/extend_tunnel.rs";

/// The fewest SIGINT sends the scan must find in [`TUNNEL_SIGINT_TESTS`]. A
/// floor rather than an exact count, so a new SIGINT test needs no edit here.
const MIN_SIGINT_SENDS: usize = 3;

/// The call that sends a signal to a child process.
const KILL_CALL: &str = "libc::kill(";

/// The bounded wait every SIGINT test must reach first.
const BOUNDED_WAIT_CALL: &str = "wait_with_deadline(";

/// A wait that blocks for as long as the child runs.
const UNBOUNDED_WAIT_CALL: &str = ".wait()";

/// Every functional test that sends SIGINT to a tunnel must wait for it to
/// exit with `wait_with_deadline`, which kills the tunnel and fails the test
/// when it has not exited in time. A plain `child.wait()` blocks for as long
/// as the tunnel runs, so a tunnel that missed the signal would hang the CI
/// job until its time limit instead of failing one test. The lost Ctrl-C
/// those tests probe cannot be produced on demand, so a hang is the shape a
/// regression takes, and only this source check keeps every SIGINT test on a
/// bounded wait.
///
/// It also requires `wait_with_deadline` itself to poll with `try_wait` and
/// to `.kill()` the child: a helper that only called `child.wait()` would
/// satisfy the first rule and still hang.
#[test]
fn test_tunnel_sigint_tests_wait_with_a_deadline() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(TUNNEL_SIGINT_TESTS);
    let text = read_source(&path);

    let sends = code_lines(&text)
        .filter(|(_, line)| sigint_send_at(line).is_some())
        .count();
    assert!(
        sends >= MIN_SIGINT_SENDS,
        "found {sends} `{KILL_CALL}` lines sending SIGINT in {}, expected at least \
         {MIN_SIGINT_SENDS}: the SIGINT wait guard would pass on a scan that no longer covers \
         the tunnel's SIGINT tests",
        path.display()
    );

    let helper = function_body(&text, "wait_with_deadline")
        .unwrap_or_else(|| panic!("`fn wait_with_deadline` not found in {}", path.display()));
    assert!(
        helper.contains("try_wait") && helper.contains(".kill()"),
        "`wait_with_deadline` in {} must poll with `try_wait` and `.kill()` the child at the \
         deadline; a helper that blocks in `child.wait()` hangs CI just the same. Its body:\n\
         {helper}",
        path.display()
    );

    let violations: Vec<String> = unbounded_sigint_waits(&text)
        .into_iter()
        .map(|violation| format!("{}:{violation}", path.display()))
        .collect();
    if !violations.is_empty() {
        panic!(
            "Every tunnel test that sends SIGINT must wait for the exit with \
             `{BOUNDED_WAIT_CALL}`, so a tunnel that misses the signal fails the test instead of \
             hanging CI until the job time limit.\n\
             \n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }
}

/// The matcher behind the SIGINT wait guard accepts a SIGINT followed by
/// `wait_with_deadline(`, even with a later `.wait()` for cleanup; flags a
/// SIGINT whose first wait is `.wait()`, or that reaches the next `fn` or
/// the end of the text without `wait_with_deadline(`; and ignores kills that
/// send no SIGINT. Commented-out code counts for neither.
#[test]
fn test_sigint_wait_matcher_flags_unbounded_waits() {
    let no_violation: Vec<&str> = Vec::new();

    let bounded = [
        "fn bounded() {",
        "    unsafe {",
        "        libc::kill(pid, libc::SIGINT);",
        "    }",
        "    let status = wait_with_deadline(&mut child, \"run\");",
        "}",
    ]
    .join("\n");
    assert_eq!(
        unbounded_sigint_waits(&bounded),
        no_violation,
        "SIGINT then wait_with_deadline"
    );

    let unbounded = [
        "fn unbounded() {",
        "    unsafe {",
        "        libc::kill(pid, libc::SIGINT);",
        "    }",
        "    // let status = wait_with_deadline(&mut child, \"run\");",
        "    let status = child.wait().expect(\"wait\");",
        "}",
    ]
    .join("\n");
    assert_eq!(
        unbounded_sigint_waits(&unbounded),
        vec![
            "3: SIGINT sent here is followed by an unbounded `.wait()` on line 6 before any \
             `wait_with_deadline(`"
        ],
        "SIGINT then child.wait()"
    );

    let no_wait_before_next_fn = [
        "fn no_wait() {",
        "    unsafe { libc::kill(pid, libc::SIGINT); }",
        "}",
        "",
        "pub(crate) async fn next_test() {",
        "    let status = wait_with_deadline(&mut child, \"later\");",
        "}",
    ]
    .join("\n");
    assert_eq!(
        unbounded_sigint_waits(&no_wait_before_next_fn),
        vec![
            "2: SIGINT sent here is not followed by `wait_with_deadline(` before the next `fn` \
             or the end of the file"
        ],
        "SIGINT with no wait before the next fn"
    );

    let no_wait_before_end = "    unsafe { libc::kill(pid, libc::SIGINT); }\n}";
    assert_eq!(
        unbounded_sigint_waits(no_wait_before_end),
        vec![
            "1: SIGINT sent here is not followed by `wait_with_deadline(` before the next `fn` \
             or the end of the file"
        ],
        "SIGINT with no wait before the end of the text"
    );

    let bounded_then_cleanup = [
        "fn bounded_then_cleanup() {",
        "    unsafe { libc::kill(pid, libc::SIGINT); }",
        "    let status = wait_with_deadline(&mut child, \"run\");",
        "    let _ = upstream.kill();",
        "    let _ = upstream.wait();",
        "}",
    ]
    .join("\n");
    assert_eq!(
        unbounded_sigint_waits(&bounded_then_cleanup),
        no_violation,
        "SIGINT then wait_with_deadline, then a cleanup wait"
    );

    let no_sigint = [
        "fn cleanup() {",
        "    let _ = child.kill();",
        "    let _ = child.wait();",
        "    unsafe { libc::kill(pid, libc::SIGTERM); }",
        "    let _ = child.wait();",
        "    // unsafe { libc::kill(pid, libc::SIGINT); }",
        "    let _ = child.wait();",
        "}",
    ]
    .join("\n");
    assert_eq!(
        unbounded_sigint_waits(&no_sigint),
        no_violation,
        "kills that send no SIGINT"
    );
}

/// The byte offset of the `libc::kill(` call on a line that sends SIGINT
/// with it, or `None` for any other line.
fn sigint_send_at(line: &str) -> Option<usize> {
    line.find(KILL_CALL).filter(|_| line.contains("SIGINT"))
}

/// One description per SIGINT send in `text` that is not followed by a
/// bounded wait, each starting with the 1-based line number of its
/// `libc::kill(` call. The code after that call, up to the next line that
/// starts a function (see [`declared_function`]) or the end of the text,
/// must reach `wait_with_deadline(` before any `.wait()`. Comment lines are
/// skipped as in [`code_lines`].
fn unbounded_sigint_waits(text: &str) -> Vec<String> {
    let lines: Vec<(usize, &str)> = code_lines(text).collect();
    let mut violations = Vec::new();
    for (index, &(kill_line, line)) in lines.iter().enumerate() {
        let Some(at) = sigint_send_at(line) else {
            continue;
        };
        let rest_of_function = lines[index + 1..]
            .iter()
            .copied()
            .take_while(|(_, line)| declared_function(line).is_none());
        let first = std::iter::once((kill_line, &line[at..]))
            .chain(rest_of_function)
            .find_map(|(line_number, code)| first_wait(code).map(|wait| (line_number, wait)));
        match first {
            Some((_, Wait::Bounded)) => {}
            Some((wait_line, Wait::Unbounded)) => violations.push(format!(
                "{kill_line}: SIGINT sent here is followed by an unbounded \
                 `{UNBOUNDED_WAIT_CALL}` on line {wait_line} before any `{BOUNDED_WAIT_CALL}`"
            )),
            None => violations.push(format!(
                "{kill_line}: SIGINT sent here is not followed by `{BOUNDED_WAIT_CALL}` before \
                 the next `fn` or the end of the file"
            )),
        }
    }
    violations
}

/// The first wait call in a piece of code.
#[derive(Debug, PartialEq)]
enum Wait {
    Bounded,
    Unbounded,
}

/// Which wait call comes first in `code`, or `None` when it has neither.
fn first_wait(code: &str) -> Option<Wait> {
    match (code.find(BOUNDED_WAIT_CALL), code.find(UNBOUNDED_WAIT_CALL)) {
        (Some(bounded), Some(unbounded)) if unbounded < bounded => Some(Wait::Unbounded),
        (Some(_), _) => Some(Wait::Bounded),
        (None, Some(_)) => Some(Wait::Unbounded),
        (None, None) => None,
    }
}

/// The name of the function a line declares, when the line starts one: `fn `
/// after optional whitespace and qualifiers such as `pub`, `pub(crate)` or
/// `async`. `None` for any other line.
fn declared_function(line: &str) -> Option<&str> {
    const QUALIFIERS: &[&str] = &[
        "pub",
        "pub(crate)",
        "pub(super)",
        "async",
        "const",
        "unsafe",
    ];
    let mut rest = line.trim_start();
    loop {
        if let Some(after_fn) = rest.strip_prefix("fn ") {
            let name = after_fn.trim_start();
            let end = name
                .find(|c: char| !is_identifier_char(c))
                .unwrap_or(name.len());
            return Some(&name[..end]);
        }
        let qualifier = QUALIFIERS.iter().find(|qualifier| {
            rest.strip_prefix(**qualifier)
                .is_some_and(|after| after.starts_with(char::is_whitespace))
        })?;
        rest = rest[qualifier.len()..].trim_start();
    }
}

/// The code lines of `fn <name>` in `text`, from its signature to the line
/// that closes its body, or `None` when no such function is declared or its
/// braces never balance. Braces are counted character by character, so a
/// string literal in the body must not hold an unbalanced brace. Comment
/// lines are skipped as in [`code_lines`].
fn function_body(text: &str, name: &str) -> Option<String> {
    let mut body = String::new();
    let mut depth = 0i32;
    let mut opened = false;
    for (_, line) in code_lines(text).skip_while(|(_, line)| declared_function(line) != Some(name))
    {
        body.push_str(line);
        body.push('\n');
        for c in line.chars() {
            match c {
                '{' => {
                    depth += 1;
                    opened = true;
                }
                '}' => depth -= 1,
                _ => {}
            }
        }
        if opened && depth <= 0 {
            return Some(body);
        }
    }
    None
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
    let mut files = Vec::new();
    collect_rust_files(dir, &mut files);
    *files_scanned += files.len();
    for path in &files {
        scan_file(path, banned, violations);
    }
}

/// Append every `.rs` file under `dir`, recursively, to `files`.
fn collect_rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("failed to read directory {}: {e}", dir.display()));

    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("failed to read entry in {}: {e}", dir.display()));
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
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
    let contents = read_source(path);

    for (line_number, line) in code_lines(&contents) {
        for substring in banned {
            if line.contains(substring) {
                violations.push(format!(
                    "{}:{line_number}: contains banned substring `{substring}`",
                    path.display(),
                ));
            }
        }
    }
}

/// Read a source file, failing the test with its path if it cannot be read.
fn read_source(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read file {}: {e}", path.display()))
}

/// The code lines of `text`, each with its 1-based line number. Lines whose
/// first non-whitespace characters are `//` are comments and are skipped,
/// which covers doc comments (`///`, `//!`) and plain line comments. A line
/// with code before a `//` comment is kept whole.
fn code_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line))
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
}

/// Whether `c` can be part of a Rust identifier.
fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
