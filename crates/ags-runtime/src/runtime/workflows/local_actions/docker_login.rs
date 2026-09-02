//! `docker-login` local action: authenticates the local Docker CLI
//! against a container registry using `docker login --password-stdin`.
//!
//! The secret (password / token) is transported exclusively via stdin
//! and never appears in argv, log output, or error messages.

use std::ffi::OsString;
use std::time::Duration;

use ags_protocol::error::{RuntimeError, RuntimeErrorKind};
use ags_protocol::event::ProgressSink;
use async_trait::async_trait;
use serde_json::{Map, Value};

use super::{LocalAction, LocalActionInput};

/// Cap on docker stderr bytes propagated into user-facing error strings
/// so a misbehaving registry proxy returning a large error page cannot
/// blow up the UI.
const DOCKER_STDERR_DISPLAY_LIMIT: usize = 512;

/// Maximum wall-clock time to wait for `docker login` to complete.
///
/// `docker login --password-stdin` performs a short credential exchange
/// with the registry (DNS + TCP + TLS + one HTTP roundtrip). Under normal
/// conditions this finishes in under 5 seconds. 30 seconds accommodates
/// slow networks and overloaded registries while still catching a truly
/// hung process (e.g. the registry never sends a response).
const DOCKER_LOGIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum credential blob size that the Windows credential store
/// (`wincred`) can persist, in bytes. This is the Windows platform
/// constant `CRED_MAX_CREDENTIAL_BLOB_SIZE`. It applies to the secret
/// alone, not the entire credential record. A secret of exactly 2,560
/// bytes stores successfully; 2,561 bytes fails. Established by
/// bisection on a Windows machine against the `wincred` helper, `docker
/// login`, and the Go predecessor's own login routine.
const WINCRED_MAX_CREDENTIAL_BLOB_SIZE: usize = 2560;

/// The `docker-login` action handler.
pub struct DockerLoginAction;

#[async_trait(?Send)]
impl LocalAction for DockerLoginAction {
    fn inputs(&self) -> Vec<LocalActionInput> {
        vec![
            LocalActionInput {
                name: "registry",
                required: true,
                description: "Container registry hostname to authenticate against.",
            },
            LocalActionInput {
                name: "username",
                required: true,
                description: "Registry username.",
            },
            LocalActionInput {
                name: "password",
                required: true,
                description: "Registry password or token (transported via stdin, never argv).",
            },
        ]
    }

    async fn run(
        &self,
        _runtime: &crate::runtime::Runtime,
        inputs: &Map<String, Value>,
        _sink: &mut dyn ProgressSink,
        dry_run: bool,
    ) -> Result<Value, RuntimeError> {
        let registry = super::required_string(inputs, "registry", "docker-login")?;
        let username = super::required_string(inputs, "username", "docker-login")?;

        if dry_run {
            return Ok(serde_json::json!({
                "registry": registry,
                "username": username,
                "login": "dry-run"
            }));
        }

        let password = super::required_string(inputs, "password", "docker-login")?;
        run_docker_login(&registry, &username, &password)
    }
}

/// Build the argument list for a `docker login` invocation.
///
/// The password is intentionally absent from the returned argv — it is
/// transported via the subprocess's stdin using `--password-stdin`.
/// Callers must pipe the password into the child process's stdin.
pub fn build_docker_login_args(registry: &str, username: &str) -> Vec<OsString> {
    vec![
        "login".into(),
        "--username".into(),
        username.into(),
        "--password-stdin".into(),
        registry.into(),
    ]
}

/// RAII guard that ensures a spawned child process is reaped (killed +
/// waited) when dropped without being consumed via `into_child()`.
///
/// This prevents zombie processes when an early `?` return occurs
/// between `spawn()` and `wait_with_output()`. On the success path,
/// `into_child()` disarms the guard so the caller can call
/// `wait_with_output()` without double-waiting.
struct ChildGuard(Option<std::process::Child>);

impl ChildGuard {
    fn new(child: std::process::Child) -> Self {
        Self(Some(child))
    }

    /// Borrow the child mutably (e.g. to call `stdin.take()`).
    fn as_mut(&mut self) -> &mut std::process::Child {
        self.0.as_mut().expect("ChildGuard: child already consumed")
    }

    /// Consume the guard and return the child for `wait_with_output()`.
    /// Disarms the drop-reap so the caller owns the lifecycle.
    fn into_child(mut self) -> std::process::Child {
        self.0.take().expect("ChildGuard: child already consumed")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            // Ignore errors: the child may have already exited, making
            // kill() fail — that is not an error worth surfacing.
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Map a [`WaitError`][crate::support::process::WaitError] from the
/// `docker login` child process into a [`RuntimeError`].
///
/// The credentials are available at the production call site (captured in
/// the closure scope). This function receives them so tests can prove they
/// never appear in the emitted error message.
///
/// `_registry`, `_username`, and `_password` are deliberately accepted but
/// unused — they are a guardrail seam so the credential-leak test drives
/// the production function through the same parameter path a live
/// invocation uses. Clippy's `unused_variables` lint plus
/// `test_timeout_error_does_not_leak_password` together ensure a future
/// edit that wires a secret into the message is caught immediately.
fn map_docker_wait_error(
    err: crate::support::process::WaitError,
    _registry: &str,
    _username: &str,
    _password: &str,
) -> RuntimeError {
    match err {
        crate::support::process::WaitError::TimedOut(d) => RuntimeError {
            kind: RuntimeErrorKind::Network,
            message: format!(
                "docker-login: docker login timed out after {}s",
                d.as_secs()
            ),
            details: None,
            hint: Some(
                "The container registry did not respond in time. \
                 Check your network connection and verify the registry URL is reachable."
                    .to_string(),
            ),
            trace: None,
        },
        crate::support::process::WaitError::Wait(e) => RuntimeError {
            kind: RuntimeErrorKind::Internal,
            message: format!("docker-login: failed to wait for docker process: {e}"),
            details: None,
            hint: None,
            trace: None,
        },
    }
}

/// Execute `docker login --password-stdin`, piping the password through
/// stdin. Maps spawn and exit errors per the error-mapping table:
///
/// - Missing `docker` binary → `NotFound`
/// - Non-zero exit → `Internal` carrying the action name and stderr
fn run_docker_login(
    registry: &str,
    username: &str,
    password: &str,
) -> Result<serde_json::Value, RuntimeError> {
    let args = build_docker_login_args(registry, username);

    let mut cmd = std::process::Command::new("docker");
    cmd.args(&args);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let child = cmd.spawn().map_err(map_docker_spawn_error)?;
    // ChildGuard ensures kill() + wait() on every early-return path
    // between spawn() and wait_with_output(), preventing zombie
    // processes even when the executor retries the step.
    let mut guard = ChildGuard::new(child);

    // Start drain threads BEFORE writing to stdin to prevent
    // pipe-buffer deadlock: if docker writes to stdout/stderr before
    // consuming all of stdin, the output pipe fills and docker blocks;
    // meanwhile our write_all blocks too if stdin is full — creating a
    // deadlock that wait_with_timeout's own doc comment says it exists
    // to prevent. spawn_drains takes the stdout/stderr pipes and
    // starts background readers immediately.
    let drains = crate::support::process::spawn_drains(guard.as_mut());

    // Pipe the password through stdin — it never touches argv or logs.
    let mut stdin = guard.as_mut().stdin.take().ok_or_else(|| RuntimeError {
        kind: RuntimeErrorKind::Internal,
        message: "docker-login: docker child process has no stdin pipe".into(),
        details: None,
        hint: None,
        trace: None,
    })?;
    {
        use std::io::Write;
        // Write errors are propagated as Internal; the password itself
        // is not included in the error message.
        stdin
            .write_all(password.as_bytes())
            .map_err(|e| RuntimeError {
                kind: RuntimeErrorKind::Internal,
                message: format!("docker-login: failed to write to docker stdin: {e}"),
                details: None,
                hint: None,
                trace: None,
            })?;
    }
    // Drop stdin to close the pipe so docker can proceed.
    drop(stdin);

    // Disarm the guard — wait_with_drains() takes ownership and handles
    // kill + reap on the timeout path, so no zombie is left behind.
    let child = guard.into_child();
    let output = crate::support::process::wait_with_drains(child, DOCKER_LOGIN_TIMEOUT, drains)
        .map_err(|e| map_docker_wait_error(e, registry, username, password))?;

    if !output.status.success() {
        let stderr = sanitize_docker_stderr(&output.stderr);
        let hint = credential_store_size_hint(password.len(), cfg!(windows), registry, username)
            .unwrap_or_else(|| {
                "Check that the registry URL, username, and credentials are correct.".to_string()
            });
        return Err(RuntimeError {
            kind: RuntimeErrorKind::Internal,
            message: format!("docker-login: docker login failed: {stderr}"),
            details: None,
            hint: Some(hint),
            trace: None,
        });
    }

    // docker-login has no meaningful output for downstream steps; return
    // Null so captures are a no-op when none are declared.
    Ok(serde_json::Value::Null)
}

/// Map a `docker` spawn error to the appropriate `RuntimeError`.
///
/// `NotFound` means the `docker` binary is missing — a user-correctable
/// condition. Other I/O errors are internal failures.
fn map_docker_spawn_error(e: std::io::Error) -> RuntimeError {
    if e.kind() == std::io::ErrorKind::NotFound {
        RuntimeError {
            kind: RuntimeErrorKind::NotFound,
            message: "docker is not installed or not found on PATH".to_string(),
            details: None,
            hint: Some(
                "Install Docker from https://docs.docker.com/get-docker/ and ensure it is on your PATH."
                    .to_string(),
            ),
            trace: None,
        }
    } else {
        RuntimeError {
            kind: RuntimeErrorKind::Internal,
            message: format!("docker-login: failed to run docker: {e}"),
            details: None,
            hint: None,
            trace: None,
        }
    }
}

/// Sanitize docker stderr for safe terminal display: convert to UTF-8
/// lossily (docker may emit locale-encoded messages), strip terminal
/// control sequences (the registry is untrusted), and truncate to a
/// display-safe length on a character boundary.
fn sanitize_docker_stderr(raw: &[u8]) -> String {
    let lossy = String::from_utf8_lossy(raw);
    let cleaned = crate::support::strings::strip_terminal_control_sequences(lossy.trim());
    crate::support::strings::truncate_display_text(&cleaned, DOCKER_STDERR_DISPLAY_LIMIT)
}

/// Return a targeted error hint when a `docker login` non-zero exit is
/// caused by the Windows credential store's size cap, or `None` if the
/// composite signal does not match.
///
/// The detection is locale-independent: it checks whether the secret
/// length exceeds `WINCRED_MAX_CREDENTIAL_BLOB_SIZE` on a Windows
/// target, rather than matching Docker's (localised) error text.
///
/// # Secret safety
///
/// This function receives only the secret's byte length (`usize`), not
/// the secret itself. The secret therefore cannot appear in the returned
/// hint — the guarantee is enforced by the type signature.
///
/// `is_windows` is a parameter (rather than `cfg!(windows)` inline) so
/// every branch is reachable from tests running on any platform.
fn credential_store_size_hint(
    secret_len: usize,
    is_windows: bool,
    registry: &str,
    username: &str,
) -> Option<String> {
    if !is_windows || secret_len <= WINCRED_MAX_CREDENTIAL_BLOB_SIZE {
        return None;
    }

    Some(format!(
        "The token is {secret_len} bytes, which exceeds the 2,560-byte \
         limit of the Windows credential store. This is the most likely \
         cause of the login failure.\n\
         \n\
         Workaround: log in from WSL2, where Docker keeps credentials \
         in a plain config file with no size limit:\n\
         \n      ags extend docker-login --namespace <ns> --app <app> \
         --print --print-format token \\\n        \
         | docker login --username {username} --password-stdin \
         {registry}\n\
         \n\
         If the workaround does not resolve the failure, verify the \
         registry URL, username, and that the token has not expired.\n\
         \n\
         Reference: https://docs.accelbyte.io/gaming-services/modules/\
         foundations/extend/sdk-and-tools/extend-helper-cli/\
         #docker-login-fails"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // Test Plan case 5: argv builder produces the expected shape
    // ---------------------------------------------------------------
    #[test]
    fn test_build_args_produces_correct_argv() {
        let args = build_docker_login_args("registry.example.com", "myuser");
        let strs: Vec<&str> = args.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            strs,
            vec![
                "login",
                "--username",
                "myuser",
                "--password-stdin",
                "registry.example.com",
            ]
        );
    }

    // ---------------------------------------------------------------
    // Test Plan case 6: argv builder handles registries with ports
    // ---------------------------------------------------------------
    #[test]
    fn test_build_args_handles_registry_with_port() {
        let args = build_docker_login_args("registry.example.com:5000", "user");
        let strs: Vec<&str> = args.iter().map(|a| a.to_str().unwrap()).collect();
        assert!(strs.contains(&"registry.example.com:5000"));
        assert!(strs.contains(&"--password-stdin"));
    }

    // ---------------------------------------------------------------
    // Test Plan case 7: argv builder handles username with special chars
    // ---------------------------------------------------------------
    #[test]
    fn test_build_args_handles_special_chars_in_username() {
        let args = build_docker_login_args("reg.io", "user@domain.com");
        let strs: Vec<&str> = args.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(strs[2], "user@domain.com");
    }

    // ---------------------------------------------------------------
    // Test Plan case 9: token NEVER appears in any argv element
    // ---------------------------------------------------------------
    //
    // Strengthened assertion: each element of the argv is checked
    // individually — a joined-string check passes when the token is
    // its own element. Passwords include shell-special characters.
    #[test]
    fn test_password_never_appears_in_argv() {
        let passwords = [
            "secret-password",
            "p@$$w0rd!",
            "pass word with spaces",
            "$(whoami)",
            "`echo pwned`",
            "'; DROP TABLE users; --",
            "pass\nwith\nnewlines",
            // Shell-special characters that could be misinterpreted.
            "tok$en{with}[brackets]&pipes|here",
            // A token long enough to exceed typical buffer display limits.
            &"a".repeat(10000),
        ];
        for password in &passwords {
            let args = build_docker_login_args("reg.io", "user");
            // Assert over EVERY element individually — a joined-string
            // check would pass if the token happened to be its own element.
            for (i, arg) in args.iter().enumerate() {
                let s = arg.to_string_lossy();
                // Empty string is excluded: contains("") is always true.
                if !password.is_empty() {
                    assert!(
                        !s.contains(password),
                        "password must never appear in argv element [{i}]: \
                         found '{password}' in '{s}'"
                    );
                }
                assert_ne!(
                    s.as_ref(),
                    *password,
                    "password must never equal argv element [{i}]"
                );
            }
        }
    }

    /// Strengthened case 9: assert the exact argv shape is fixed at 5
    /// elements with known positions, regardless of the password value.
    /// Even when the password text matches a legitimate argv element,
    /// no extra element is added.
    #[test]
    fn test_password_absent_from_every_argv_element_comprehensive() {
        let tricky_passwords = [
            "login",            // matches the docker subcommand
            "--username",       // matches a flag
            "--password-stdin", // matches another flag
            "reg.io",           // matches the registry
            "user",             // matches the username
        ];
        for password in &tricky_passwords {
            let args = build_docker_login_args("reg.io", "user");
            // The password is never an argument to docker — it goes via stdin.
            // The argv list is fixed at 5 elements with known positions.
            assert_eq!(args.len(), 5, "argv must have exactly 5 elements");
            assert_eq!(args[0].to_str().unwrap(), "login");
            assert_eq!(args[1].to_str().unwrap(), "--username");
            assert_eq!(args[2].to_str().unwrap(), "user");
            assert_eq!(args[3].to_str().unwrap(), "--password-stdin");
            assert_eq!(args[4].to_str().unwrap(), "reg.io");
            // No sixth element was added for the password.
            let _ = password; // used only to drive the loop
        }
    }

    // ---------------------------------------------------------------
    // Error mapping: docker not found → NotFound
    // ---------------------------------------------------------------
    #[test]
    fn test_map_spawn_not_found_returns_not_found_error() {
        let err = map_docker_spawn_error(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not found",
        ));
        assert_eq!(err.kind, RuntimeErrorKind::NotFound);
        assert!(err.message.contains("not installed"));
    }

    #[test]
    fn test_map_spawn_other_error_returns_internal() {
        let err = map_docker_spawn_error(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        ));
        assert_eq!(err.kind, RuntimeErrorKind::Internal);
    }

    // ---------------------------------------------------------------
    // Stderr sanitization
    // ---------------------------------------------------------------
    #[test]
    fn test_sanitize_stderr_truncates_long_output() {
        let long = "x".repeat(1000);
        let sanitized = sanitize_docker_stderr(long.as_bytes());
        // `truncate_display_text` appends "... (truncated)" when it shortens.
        assert!(
            sanitized.len() <= DOCKER_STDERR_DISPLAY_LIMIT + "... (truncated)".len(),
            "sanitized length {} exceeds limit {} + suffix {}",
            sanitized.len(),
            DOCKER_STDERR_DISPLAY_LIMIT,
            "... (truncated)".len()
        );
        assert!(
            sanitized.ends_with("... (truncated)"),
            "truncated output must end with the standard suffix, got: {sanitized}"
        );
    }

    #[test]
    fn test_sanitize_stderr_trims_whitespace() {
        let raw = b"  error message  \n";
        let sanitized = sanitize_docker_stderr(raw);
        assert_eq!(sanitized, "error message");
    }

    // ---------------------------------------------------------------
    // Regression: multi-byte character straddling the byte limit must
    // not panic. The old implementation used `&trimmed[..512]` which
    // is a raw byte index — if byte 512 falls inside a multi-byte
    // character, Rust panics with "byte index is not a char boundary".
    // ---------------------------------------------------------------
    #[test]
    fn test_sanitize_stderr_handles_multibyte_at_boundary() {
        // 511 ASCII bytes followed by 'é' (U+00E9, 2-byte UTF-8: 0xC3 0xA9).
        // Byte 511 = 0xC3 (first byte of 'é'), byte 512 = 0xA9 (second byte).
        // The old `&trimmed[..512]` would split inside 'é' and panic.
        let mut input = "a".repeat(511);
        input.push('é');
        input.push_str(&"b".repeat(100));
        assert!(input.len() > DOCKER_STDERR_DISPLAY_LIMIT);

        let result = sanitize_docker_stderr(input.as_bytes());

        // Must produce valid UTF-8 without panicking.
        assert!(result.len() <= DOCKER_STDERR_DISPLAY_LIMIT + "... (truncated)".len());
        // The truncation point must back off to byte 511 (before the 'é'),
        // so the result starts with 511 'a's — not a partial character.
        assert!(result.starts_with(&"a".repeat(511)));
        assert!(result.ends_with("... (truncated)"));
    }

    // ---------------------------------------------------------------
    // ANSI CSI and OSC escape sequences must be stripped — the registry
    // is untrusted and could inject terminal control sequences.
    // ---------------------------------------------------------------
    #[test]
    fn test_sanitize_stderr_strips_terminal_escape_sequences() {
        // CSI color sequence + OSC title-set sequence.
        let mut raw = Vec::new();
        raw.extend_from_slice(b"\x1b]0;evil title\x07"); // OSC title set
        raw.extend_from_slice(b"\x1b[31m"); // CSI red color
        raw.extend_from_slice(b"error: bad credentials");
        raw.extend_from_slice(b"\x1b[0m"); // CSI reset

        let result = sanitize_docker_stderr(&raw);
        assert_eq!(result, "error: bad credentials");
        assert!(!result.contains('\x1b'), "ESC must be stripped");
        assert!(!result.contains('\x07'), "BEL must be stripped");
    }

    // ---------------------------------------------------------------
    // Stripping runs before truncation: a string that exceeds the byte
    // limit in raw form but falls under the limit after escape removal
    // must survive intact without truncation.
    // ---------------------------------------------------------------
    #[test]
    fn test_sanitize_stderr_strips_then_truncates_in_correct_order() {
        let visible = "x".repeat(500);
        // Pad the raw bytes past 512 using escape sequences that
        // contribute zero visible characters.
        let padding = "\x1b[31m".repeat(20); // 20 * 5 = 100 escape bytes
        let raw = format!("{padding}{visible}");
        assert!(
            raw.len() > DOCKER_STDERR_DISPLAY_LIMIT,
            "raw must exceed the display limit to prove ordering"
        );

        let result = sanitize_docker_stderr(raw.as_bytes());
        // After stripping, only the 500-char visible text remains —
        // which is under the limit and must not be truncated.
        assert_eq!(result, visible);
    }

    // ---------------------------------------------------------------
    // Test Plan case 7 (SUBSTITUTED): the Spec's case 7 assumed a
    // Docker Hub default with no registry. The EHS credentials
    // response always supplies a registry and the action requires it,
    // so that path is unreachable. Registry stays mandatory — assert
    // a missing registry is rejected with a clear Validation error.
    // ---------------------------------------------------------------
    #[test]
    fn test_missing_registry_rejected_with_validation_error() {
        // The validation happens via `required_string` in the parent module.
        // Calling it directly avoids needing a full Runtime instance for a
        // pure-input-validation test.
        let inputs = serde_json::Map::new();
        let err = crate::runtime::workflows::local_actions::required_string(
            &inputs,
            "registry",
            "docker-login",
        )
        .unwrap_err();
        assert_eq!(
            err.kind,
            RuntimeErrorKind::Validation,
            "missing registry must be Validation, not another kind: {err:?}"
        );
        assert!(
            err.message.contains("registry"),
            "error message must name the missing field: {}",
            err.message
        );
    }

    // ---------------------------------------------------------------
    // ChildGuard: RAII zombie-prevention guard
    // ---------------------------------------------------------------

    #[test]
    fn test_child_guard_into_child_returns_usable_child() {
        // `cargo --version` is available on every platform running `cargo test`.
        let child = std::process::Command::new("cargo")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("cargo must be available in the test environment");

        let guard = ChildGuard::new(child);
        // into_child disarms the guard; the caller owns the child.
        let child = guard.into_child();
        let output = child.wait_with_output().expect("wait must succeed");
        assert!(output.status.success(), "cargo --version must exit 0");
    }

    #[test]
    fn test_child_guard_drop_reaps_child_without_panic() {
        let child = std::process::Command::new("cargo")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("cargo must be available in the test environment");

        // Dropping the guard reaps the child (kill + wait). This must
        // not panic regardless of whether the child has already exited
        // or is still running — kill() on an exited child fails
        // harmlessly and the subsequent wait() succeeds.
        drop(ChildGuard::new(child));
    }

    // ---------------------------------------------------------------
    // Timeout constant sanity
    // ---------------------------------------------------------------

    #[test]
    fn test_docker_login_timeout_is_reasonable() {
        // The constant must be long enough for a slow registry handshake
        // but short enough to catch a truly hung process.
        assert!(
            DOCKER_LOGIN_TIMEOUT >= std::time::Duration::from_secs(10),
            "timeout too short for real-world registries"
        );
        assert!(
            DOCKER_LOGIN_TIMEOUT <= std::time::Duration::from_secs(120),
            "timeout too long — a hung docker login should not block for minutes"
        );
    }

    // ---------------------------------------------------------------
    // Timeout error message must never contain the password/token.
    // The test calls the production `map_docker_wait_error` function
    // with real passwords flowing through the same parameter path as
    // a live invocation. If the mapping ever interpolates a credential
    // into the error, this test catches it.
    // ---------------------------------------------------------------

    #[test]
    fn test_timeout_error_does_not_leak_password() {
        let passwords = [
            "super-secret-token",
            "p@$$w0rd!",
            "$(whoami)",
            &"a".repeat(1000),
        ];
        for password in &passwords {
            // Call the production error-mapping function with the
            // password supplied through the same route a real
            // invocation uses.
            let err = map_docker_wait_error(
                crate::support::process::WaitError::TimedOut(std::time::Duration::from_secs(30)),
                "registry.example.com",
                "testuser",
                password,
            );

            let full_text = format!("{} {:?} {:?}", err.message, err.hint, err.details);
            // The password never appears anywhere in the error.
            if !password.is_empty() {
                assert!(
                    !full_text.contains(password),
                    "timeout error must not contain password '{password}', \
                     found in: {full_text}"
                );
            }
        }
    }

    // ---------------------------------------------------------------
    // Timeout error maps to the correct RuntimeErrorKind (Network, not
    // Internal) so the CLI can map it to the right exit code.
    // ---------------------------------------------------------------

    #[test]
    fn test_wait_error_maps_to_internal_kind() {
        let io_err = std::io::Error::other("waitpid failed");
        let runtime_err = map_docker_wait_error(
            crate::support::process::WaitError::Wait(io_err),
            "registry.example.com",
            "testuser",
            "testpassword",
        );
        assert_eq!(runtime_err.kind, RuntimeErrorKind::Internal);
        assert!(
            runtime_err.message.contains("failed to wait"),
            "message must describe the wait failure: {}",
            runtime_err.message
        );
    }

    #[test]
    fn test_timeout_error_maps_to_network_kind() {
        // Call the production error-mapping function.
        let runtime_err = map_docker_wait_error(
            crate::support::process::WaitError::TimedOut(std::time::Duration::from_secs(30)),
            "registry.example.com",
            "testuser",
            "testpassword",
        );
        assert_eq!(runtime_err.kind, RuntimeErrorKind::Network);
        assert!(runtime_err.message.contains("timed out"));
        assert!(runtime_err.message.contains("30s"));
    }

    // ---------------------------------------------------------------
    // FIX 3: CI-running wiring test — DockerLoginAction::run rejects
    // a missing registry via its call to required_string. Validation
    // happens before any subprocess spawn, so no Docker binary needed.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_run_rejects_missing_registry_with_validation_error() {
        struct NoopSink;
        impl ags_protocol::event::ProgressSink for NoopSink {
            fn on_event(&mut self, _event: ags_protocol::event::ProgressEvent) {}
        }

        struct NeverClient;
        #[async_trait::async_trait]
        impl crate::runtime::dispatch::http::HttpClient for NeverClient {
            async fn send(
                &self,
                _: crate::runtime::dispatch::http::HttpRequest,
            ) -> Result<crate::runtime::dispatch::http::HttpResponse, RuntimeError> {
                unreachable!("validation test must not dispatch HTTP")
            }
        }

        let runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext::default(),
            Box::new(NeverClient),
            reqwest::Client::new(),
        );

        // Provide username and password but NOT registry.
        let mut inputs = serde_json::Map::new();
        inputs.insert("username".to_string(), serde_json::json!("testuser"));
        inputs.insert("password".to_string(), serde_json::json!("testpass"));

        let mut sink = NoopSink;
        let err = DockerLoginAction
            .run(&runtime, &inputs, &mut sink, false)
            .await
            .expect_err("missing registry must fail");

        assert_eq!(
            err.kind,
            RuntimeErrorKind::Validation,
            "error kind must be Validation: {err:?}"
        );
        assert!(
            err.message.contains("registry"),
            "error message must name the missing field: {}",
            err.message
        );
    }

    // ---------------------------------------------------------------
    // Windows credential-store size-cap detection
    //
    // The pure detection function `credential_store_size_hint` returns
    // a targeted hint when the composite signal fires (non-zero exit +
    // secret exceeds the Windows `CRED_MAX_CREDENTIAL_BLOB_SIZE` cap +
    // target is Windows). The platform is a parameter, so every test
    // runs unconditionally on every platform — no `#[cfg]` gate.
    // ---------------------------------------------------------------

    #[test]
    fn test_credential_hint_fires_for_oversized_secret_on_windows() {
        // A secret longer than the 2560-byte cap on a Windows target
        // must return the targeted hint.
        let result = credential_store_size_hint(3000, true, "reg.example.com", "user");
        assert!(
            result.is_some(),
            "must return a targeted hint when secret exceeds the cap on Windows"
        );
    }

    #[test]
    fn test_credential_hint_none_for_oversized_secret_on_non_windows() {
        // The same oversized secret on a non-Windows target must return
        // None, so Linux and macOS users see the generic hint.
        let result = credential_store_size_hint(3000, false, "reg.example.com", "user");
        assert!(
            result.is_none(),
            "must return None on non-Windows even when secret exceeds the cap"
        );
    }

    #[test]
    fn test_credential_hint_none_for_short_secret_on_windows() {
        // A short secret on Windows must not fire — a genuine wrong
        // password must not be misreported as a size problem.
        let result = credential_store_size_hint(100, true, "reg.example.com", "user");
        assert!(
            result.is_none(),
            "must return None for a short secret even on Windows"
        );
    }

    #[test]
    fn test_credential_hint_boundary_2560_does_not_fire() {
        // Exactly at the cap: 2560 bytes stores successfully, so no hint.
        let result = credential_store_size_hint(2560, true, "reg.example.com", "user");
        assert!(
            result.is_none(),
            "2560-byte secret is at the cap and must not fire"
        );
    }

    #[test]
    fn test_credential_hint_boundary_2561_fires() {
        // One byte over the cap: 2561 bytes fails to store.
        let result = credential_store_size_hint(2561, true, "reg.example.com", "user");
        assert!(
            result.is_some(),
            "2561-byte secret exceeds the cap and must fire on Windows"
        );
    }

    #[test]
    fn test_credential_hint_contains_wsl2_command_and_doc_link() {
        let hint = credential_store_size_hint(3000, true, "reg.example.com", "testuser")
            .expect("must return a hint for oversized secret on Windows");

        // The hint must contain the WSL2 workaround command shape with
        // interpolated registry and username.
        assert!(
            hint.contains("--print --print-format token"),
            "hint must contain the --print --print-format token flags: {hint}"
        );
        assert!(
            hint.contains("docker login"),
            "hint must contain the docker login command: {hint}"
        );
        assert!(
            hint.contains("--password-stdin"),
            "hint must contain --password-stdin: {hint}"
        );
        assert!(
            hint.contains("reg.example.com"),
            "hint must interpolate the registry: {hint}"
        );
        assert!(
            hint.contains("testuser"),
            "hint must interpolate the username: {hint}"
        );

        // The hint must contain the documentation link, verbatim.
        assert!(
            hint.contains(
                "https://docs.accelbyte.io/gaming-services/modules/foundations/extend/\
                 sdk-and-tools/extend-helper-cli/#docker-login-fails"
            ),
            "hint must contain the documentation link: {hint}"
        );
    }

    #[test]
    fn test_credential_hint_reports_secret_length() {
        // The function takes `secret_len: usize`, not the secret itself,
        // so the secret can never physically reach the hint text — the
        // guarantee lives in the type signature. This test verifies the
        // complementary property: the hint DOES report the numeric length
        // so the user can see how large the token is. A plausible edit
        // removing the `{secret_len}` interpolation would break this.
        let hint = credential_store_size_hint(3000, true, "reg.example.com", "user")
            .expect("must return a hint for oversized secret on Windows");
        assert!(
            hint.contains("3000"),
            "hint must report the secret length (3000): {hint}"
        );

        // A different length must also appear — proving the interpolation
        // is dynamic, not a hardcoded string.
        let hint2 = credential_store_size_hint(4096, true, "reg.example.com", "user")
            .expect("must return a hint for oversized secret on Windows");
        assert!(
            hint2.contains("4096"),
            "hint must report the secret length (4096): {hint2}"
        );
    }

    #[test]
    fn test_credential_hint_command_block_is_indented() {
        // Pin the indentation of the copy-paste command block so a
        // Rust line-continuation refactor cannot silently flatten it
        // back to column 0.
        let hint = credential_store_size_hint(3000, true, "reg.example.com", "testuser")
            .expect("must return a hint for oversized secret on Windows");

        let ags_line = hint
            .lines()
            .find(|l| l.contains("ags extend docker-login"))
            .expect("hint must contain the ags extend docker-login command");
        assert!(
            ags_line.starts_with(' '),
            "the ags command line must be indented, got: {ags_line:?}"
        );

        let docker_line = hint
            .lines()
            .find(|l| l.contains("| docker login"))
            .expect("hint must contain the | docker login pipe");
        assert!(
            docker_line.starts_with(' '),
            "the docker login pipe line must be indented, got: {docker_line:?}"
        );
    }

    #[test]
    fn test_credential_hint_explains_windows_limit() {
        // The hint must identify the Windows credential-store size limit
        // as the most likely cause and state the exact byte cap.
        let hint = credential_store_size_hint(3000, true, "reg.example.com", "user")
            .expect("must return a hint for oversized secret on Windows");
        assert!(
            hint.contains("Windows"),
            "hint must mention Windows: {hint}"
        );
        assert!(
            hint.contains("2,560"),
            "hint must state the exact byte limit: {hint}"
        );
        assert!(
            hint.contains("most likely cause"),
            "hint must identify the size limit as the most likely cause: {hint}"
        );
    }

    #[test]
    fn test_credential_hint_carries_fallback_and_does_not_claim_credentials_fine() {
        let hint = credential_store_size_hint(3000, true, "reg.example.com", "user")
            .expect("must return a hint for oversized secret on Windows");

        // The hint must NOT assert that the credentials are fine — the
        // length signal alone cannot distinguish a size-cap failure from
        // a wrong-credential failure where the token happens to be oversized.
        assert!(
            !hint.contains("not a problem with your credentials"),
            "hint must not claim credentials are fine: {hint}"
        );

        // The hint must carry fallback guidance for the case where the
        // size-cap workaround does not resolve the failure.
        assert!(
            hint.contains("verify the registry URL"),
            "hint must mention verifying the registry URL: {hint}"
        );
        assert!(
            hint.contains("token has not expired"),
            "hint must mention token expiry: {hint}"
        );
    }

    #[test]
    fn test_non_matching_failure_preserves_generic_hint() {
        // When the composite signal does not match (e.g. short secret,
        // wrong password), the function returns None. The call site must
        // then use the unchanged generic hint. This test verifies that
        // None is returned for the common non-matching cases, proving
        // the generic hint path is preserved.
        let cases = [
            // Short secret on Windows (wrong password)
            (100, true),
            // Short secret on Linux
            (100, false),
            // At the cap exactly on Windows
            (2560, true),
            // At the cap exactly on non-Windows
            (2560, false),
            // Oversized on non-Windows
            (3000, false),
        ];
        for (len, is_windows) in cases {
            let result = credential_store_size_hint(len, is_windows, "reg.io", "user");
            assert!(
                result.is_none(),
                "must return None for secret_len={len}, is_windows={is_windows}"
            );
        }
    }

    // ---------------------------------------------------------------
    // Test Plan case 18: end-to-end (environment-gated, NOT coverage)
    // ---------------------------------------------------------------
    #[tokio::test]
    #[ignore] // Requires Docker installed; does NOT count as CI coverage.
    async fn test_end_to_end_docker_login_with_real_binary() {
        // This test only runs when Docker is available. It is NOT
        // coverage of the pure logic — that is covered by the argv
        // builder and error mapping tests above.
        let mut inputs = serde_json::Map::new();
        inputs.insert(
            "registry".to_string(),
            serde_json::json!("registry.example.com"),
        );
        inputs.insert("username".to_string(), serde_json::json!("testuser"));
        inputs.insert("password".to_string(), serde_json::json!("testpassword"));

        struct NoopSink;
        impl ags_protocol::event::ProgressSink for NoopSink {
            fn on_event(&mut self, _event: ags_protocol::event::ProgressEvent) {}
        }

        struct NeverClient;
        #[async_trait::async_trait]
        impl crate::runtime::dispatch::http::HttpClient for NeverClient {
            async fn send(
                &self,
                _: crate::runtime::dispatch::http::HttpRequest,
            ) -> Result<crate::runtime::dispatch::http::HttpResponse, RuntimeError> {
                unreachable!("docker-login e2e does not dispatch HTTP")
            }
        }

        let runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext::default(),
            Box::new(NeverClient),
            reqwest::Client::new(),
        );
        let mut sink = NoopSink;
        // We expect this to fail (bad credentials), but it should
        // produce an Internal error, not a NotFound error.
        let result = DockerLoginAction
            .run(&runtime, &inputs, &mut sink, false)
            .await;
        match result {
            Err(e) => {
                assert_ne!(
                    e.kind,
                    RuntimeErrorKind::NotFound,
                    "Docker should be found on PATH for this test"
                );
            }
            Ok(_) => {
                // Unexpected success — the test credentials happened
                // to work. Not a failure of the test.
            }
        }
    }
}
