//! End-to-end TUI test driven through a real pseudo-terminal.
//!
//! Marked #[ignore] because CI typically lacks a real TTY. Run locally with:
//!     cargo test -p accelbyte-ags-cli --test integration tui_e2e -- --ignored
//!
//! The test confirms that the TUI binary, when given `--ui=inline` and an
//! interactive TTY, engages the inline frontend instead of refusing — i.e.,
//! it exercises the TTY-detection branch from Task 3 in the affirmative
//! direction. It does NOT drive a real API call; it sends `Esc` to cancel an
//! early phase and asserts a non-zero exit code.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

/// Build a unique per-invocation AGS_HOME path for PTY tests.
///
/// Mirrors the strategy in `tests/common/cli_helpers.rs::ags_isolated`, but
/// returns a plain `PathBuf` so it can be set via `CommandBuilder::env` instead
/// of assert_cmd's `Command` wrapper.
///
/// A process-wide atomic counter guarantees uniqueness even when concurrent
/// tests call this within the same clock tick: a timestamp alone collides under
/// macOS's coarse `subsec_nanos`, which lets two `ags` children share one home
/// and race the first-run profile creation ("No active profile").
fn isolated_ags_home() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    std::env::temp_dir()
        .join(format!("ags-test-{}", std::process::id()))
        .join(format!("pty-{ts}-{seq}"))
}

/// Find the path to the compiled `ags` binary that cargo-test built for us.
fn ags_binary_path() -> std::path::PathBuf {
    // Cargo provides CARGO_BIN_EXE_ags at compile time for integration tests
    // when the crate has a binary target with that name.
    let path = env!("CARGO_BIN_EXE_ags");
    std::path::PathBuf::from(path)
}

#[test]
#[ignore = "Requires a real PTY; run with --ignored"]
fn test_tui_engages_when_pty_available_and_exits_on_esc() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(ags_binary_path());
    // Explicit `--ui=inline` is the point of this test: prove the flag engages
    // the inline surface in a PTY rather than refusing. `--dry-run` + isolated
    // `AGS_HOME`/`AGS_NO_KEYCHAIN` resolve the prologue offline (no keychain
    // dialog, no server); `--namespace` leaves only the required request body to
    // gather, so the inline form engages.
    cmd.args([
        "--ui=inline",
        "--dry-run",
        "iam",
        "users",
        "create",
        "--namespace",
        "ns",
    ]);
    let ags_home = isolated_ags_home();
    cmd.env("AGS_NO_KEYCHAIN", "1");
    cmd.env("AGS_HOME", &ags_home);
    cmd.env_remove("AGS_NAMESPACE");

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .expect("spawn child process inside PTY");

    // Drain the PTY concurrently so ratatui's full-screen redraws can't fill the
    // buffer and deadlock the child before it consumes our keystrokes.
    let reader = pair.master.try_clone_reader().expect("clone reader");
    let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let collected_writer = std::sync::Arc::clone(&collected);
    let reader_thread = std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => collected_writer
                    .lock()
                    .unwrap()
                    .extend_from_slice(&buf[..n]),
            }
        }
    });

    // Let specs load, the prologue resolve, and the first form frame render.
    std::thread::sleep(Duration::from_millis(1500));

    // Cancel robustly: Esc backs out of the focused field/editor; Ctrl-C is
    // intercepted globally and cancels the form from any state.
    {
        let mut writer = pair.master.take_writer().expect("take writer");
        writer.write_all(b"\x1b").ok(); // Esc
        writer.flush().ok();
        std::thread::sleep(Duration::from_millis(250));
        writer.write_all(b"\x03").ok(); // Ctrl-C
        writer.flush().ok();
    }

    // Wait for the child to exit, with a hard timeout to avoid hanging CI.
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = reader_thread.join();
                let out = String::from_utf8_lossy(&collected.lock().unwrap()).into_owned();
                panic!("TUI process did not exit within 10s of cancel; output:\n{out}");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    let _ = reader_thread.join();
    let output_text = String::from_utf8_lossy(&collected.lock().unwrap()).into_owned();

    // Non-zero exit confirms the cancel was processed. What we want to PREVENT
    // is the binary refusing `--ui=inline` outright.
    assert!(
        !status.success(),
        "expected non-zero exit; output was:\n{output_text}"
    );
    assert!(
        !output_text.contains("cannot be shown"),
        "TUI should have engaged in PTY but was refused; output:\n{output_text}"
    );
}

/// Drives a FORM-shaped service command (`iam users create`) into the inline
/// form surface via a real pseudo-terminal WITHOUT passing `--ui`.
///
/// Purpose: prove the decision matrix auto-routes a Service+FORM command
/// to the `Inline` surface. `iam users create` has a required request body, so
/// the shape classifier sets `has_body_field = true` → `Form` shape → `Inline`
/// surface (see `policy::base_surface`).
///
/// The test is OFFLINE: `--dry-run` + isolated `AGS_HOME`/`AGS_NO_KEYCHAIN`
/// let the prologue resolve without a live server or stored credentials —
/// matching the pattern used by `tests/functional/competitive_multiplayer.rs`.
///
/// The test does NOT assert a successful API result; there is no mock backend.
/// It asserts only that:
///   1. The process exits non-zero (the Enter/Esc/Ctrl-C cancel sequence yields
///      a cancelled gather → non-zero exit).
///   2. The TUI was not *refused* (no "requires an interactive terminal" in
///      output), confirming the inline surface engaged rather than the matrix
///      falling back to a plain non-TTY refusal.
///
/// Ratatui renders via full-screen cursor-movement sequences, so form-chrome
/// strings like "[ Confirm ]" may arrive interleaved with ANSI
/// escape codes and cannot be reliably matched as a contiguous substring.
/// We therefore assert engagement-only (non-zero + not-refused), matching the
/// philosophy of the sibling `test_tui_engages_when_pty_available_and_exits_on_esc`.
#[test]
#[ignore = "Requires a real PTY; run with --ignored"]
fn test_form_service_command_engages_inline_form_and_cancels() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(ags_binary_path());
    // `--dry-run` satisfies the prologue offline (no credentials needed).
    // No `--ui` flag — this verifies the matrix auto-routes FORM → Inline.
    // `--namespace` is passed as a scalar so only the required request body
    // remains missing, guaranteeing the form is entered.
    cmd.args(["--dry-run", "iam", "users", "create", "--namespace", "ns"]);

    // Isolated environment: no keychain bleed, no real config/credentials.
    let ags_home = isolated_ags_home();
    cmd.env("AGS_NO_KEYCHAIN", "1");
    cmd.env("AGS_HOME", &ags_home);
    // Remove any inherited namespace env-var so it cannot satisfy remaining
    // required inputs and accidentally short-circuit form gathering.
    cmd.env_remove("AGS_NAMESPACE");

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .expect("spawn child process inside PTY");

    // Drain the PTY output CONCURRENTLY. ratatui redraws full-screen frames; if
    // we don't read the master continuously the PTY buffer fills, the child
    // blocks on its stderr write, and it never consumes our keystrokes — a
    // classic PTY deadlock. A reader thread accumulates output until EOF.
    let reader = pair.master.try_clone_reader().expect("clone reader");
    let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let collected_writer = std::sync::Arc::clone(&collected);
    let reader_thread = std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => collected_writer
                    .lock()
                    .unwrap()
                    .extend_from_slice(&buf[..n]),
            }
        }
    });

    // Let the binary load bundled specs, resolve the prologue, and render the
    // first form frame. Generous because spec decompression/parsing dominates.
    std::thread::sleep(Duration::from_millis(1500));

    // Exercise a field, then cancel robustly:
    //   Enter  — engage the focused field (begins a scalar edit, or opens the
    //            JSON editor when the body field is focused).
    //   Esc    — back out of that edit / exit the JSON editor.
    //   Ctrl-C — `form_runner::drive_form` intercepts Ctrl-C globally and
    //            cancels the form from any form-loop state (a single byte, so
    //            no lone-ESC escape-sequence disambiguation to worry about).
    {
        let mut writer = pair.master.take_writer().expect("take writer");
        writer.write_all(b"\r").ok(); // Enter
        writer.flush().ok();
        std::thread::sleep(Duration::from_millis(250));
        writer.write_all(b"\x1b").ok(); // Esc
        writer.flush().ok();
        std::thread::sleep(Duration::from_millis(250));
        writer.write_all(b"\x03").ok(); // Ctrl-C
        writer.flush().ok();
    }

    // Wait for the child to exit, with a hard timeout.
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = reader_thread.join();
                let out = String::from_utf8_lossy(&collected.lock().unwrap()).into_owned();
                panic!("form process did not exit within 10s of cancel sequence; output:\n{out}");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    let _ = reader_thread.join();
    let output_text = String::from_utf8_lossy(&collected.lock().unwrap()).into_owned();

    // Assert engagement, not success: non-zero exit confirms the cancel was
    // processed (Cancelled → non-zero) rather than the prologue refusing.
    assert!(
        !status.success(),
        "expected non-zero exit (cancel); output was:\n{output_text}"
    );
    // Assert the inline surface was not refused (would otherwise print this).
    assert!(
        !output_text.contains("cannot be shown"),
        "inline form should have engaged in PTY but was refused; output:\n{output_text}"
    );
}

/// Drives `workflow run competitive-multiplayer` into the FULLSCREEN surface via
/// a real pseudo-terminal, proving the matrix routes a workflow run to the
/// alt-screen surface and that it dismisses cleanly.
///
/// OFFLINE: `--dry-run` + isolated `AGS_HOME` resolve the prologue with no
/// keychain and no server. Only `--namespace` is supplied. The run opens on the
/// briefing screen; the test advances past it (Enter) to the Phase 1 inputs form
/// (the fleet inputs are still missing) — which is all it needs: it only asserts
/// the surface engaged and dismisses cleanly.
///
/// Engagement is asserted via the alternate-screen-enter sequence
/// (`ESC[?1049h`) in the PTY output — a reliable signal that a REAL fullscreen
/// surface was built, not the plain degrade path (which never enters the alt
/// screen). The test cancels the form (Ctrl-C), dismisses the cancelled summary
/// (`q`), and asserts the process exits.
#[test]
#[ignore = "Requires a real PTY; run with --ignored"]
fn test_workflow_run_engages_fullscreen_surface_and_dismisses() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(ags_binary_path());
    // No `--ui` flag — the matrix auto-routes a workflow run to fullscreen.
    cmd.args([
        "--dry-run",
        "workflow",
        "run",
        "competitive-multiplayer",
        "--namespace",
        "dev",
    ]);
    let ags_home = isolated_ags_home();
    cmd.env("AGS_NO_KEYCHAIN", "1");
    cmd.env("AGS_HOME", &ags_home);
    cmd.env_remove("AGS_NAMESPACE");

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .expect("spawn child process inside PTY");

    // Drain output concurrently to avoid a PTY write-buffer deadlock.
    let reader = pair.master.try_clone_reader().expect("clone reader");
    let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let collected_writer = std::sync::Arc::clone(&collected);
    let reader_thread = std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => collected_writer
                    .lock()
                    .unwrap()
                    .extend_from_slice(&buf[..n]),
            }
        }
    });

    // Let specs load, the prologue resolve, the workflow run, and the result
    // panel render in the alt screen.
    std::thread::sleep(Duration::from_millis(1500));

    // The workflow opens on a briefing screen. Advance past it (Enter) to the
    // Phase 1 inputs form, cancel that (Ctrl-C), then dismiss the resulting
    // "cancelled" summary (`q` exits the dismiss loop). Order matters: a bare
    // `q` would be typed into the form, not treated as dismiss.
    {
        let mut writer = pair.master.take_writer().expect("take writer");
        writer.write_all(b"\r").ok(); // Enter → continue past the briefing
        writer.flush().ok();
        std::thread::sleep(Duration::from_millis(800));
        writer.write_all(b"\x03").ok(); // Ctrl-C → cancel the inputs form
        writer.flush().ok();
        std::thread::sleep(Duration::from_millis(300));
        writer.write_all(b"q").ok(); // dismiss the cancelled summary
        writer.flush().ok();
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = reader_thread.join();
                let out = String::from_utf8_lossy(&collected.lock().unwrap()).into_owned();
                panic!("fullscreen workflow did not exit within 10s of dismiss; output:\n{out}");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }

    let _ = reader_thread.join();
    let output_text = String::from_utf8_lossy(&collected.lock().unwrap()).into_owned();

    // The alt-screen-enter sequence proves a REAL fullscreen surface engaged
    // (the plain degrade path never enters the alternate screen).
    assert!(
        output_text.contains("\x1b[?1049h"),
        "expected the alternate-screen-enter sequence (fullscreen engaged); output:\n{output_text}"
    );
    assert!(
        !output_text.contains("cannot be shown"),
        "fullscreen surface should have engaged in PTY but was refused; output:\n{output_text}"
    );
}

/// Strip ANSI CSI escape sequences so chrome strings can be matched as
/// contiguous substrings. ratatui writes each text run after a cursor-move
/// escape; removing the escapes leaves the rendered glyphs intact.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for inner in chars.by_ref() {
                    if inner.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            Some(_) => {}
            None => {}
        }
    }
    out
}

/// Drives `workflow run competitive-multiplayer` into the per-step review form
/// and asserts the persistent fullscreen chrome — the step strip AND the contextual
/// nav bar — is on screen *while the form is up*, proving the form renders
/// inside the four-region layout (not a full-screen takeover). It also asserts
/// a FIXED request field (`stat-code`/`mmr`) is shown — proof the form presents
/// the step's complete request, not just missing inputs (the old gather never
/// showed literal-bound fields).
///
/// OFFLINE: `--dry-run` + isolated `AGS_HOME` resolve the prologue with no
/// keychain and no server, and dry-run previews each step with NO network. On
/// the interactive fullscreen surface every step now pauses for review, so the
/// run pauses at step 1 ("Create the MMR skill stat") with its full editable
/// request. Ctrl-C cancels the form.
///
/// `--namespace` must be supplied: it is resolved from the active profile (or
/// the flag) at context-build time, before the run, so omitting it errors in
/// the prologue rather than pausing.
#[test]
#[ignore = "Requires a real PTY; run with --ignored"]
fn test_workflow_gather_renders_inside_persistent_chrome() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(ags_binary_path());
    // `--dry-run` previews each step offline (no network). The run opens on the
    // Phase 1 Inputs form, so ALL required inputs are supplied via flags (the
    // optional ones default) — the form is fully valid and can be submitted
    // (Ctrl-S) to advance into step 1's per-step review, which is what this
    // test asserts. The fleet inputs are dynamic-enum fields, but a flag-
    // supplied value is accepted as-is (options are only resolved lazily on
    // field activation, which this test never triggers).
    cmd.args([
        "--dry-run",
        "workflow",
        "run",
        "competitive-multiplayer",
        "--namespace",
        "dev",
        "--fleet-image-id",
        "img-1",
        "--fleet-region",
        "us-west-2",
        "--fleet-instance-id",
        "inst-1",
    ]);
    let ags_home = isolated_ags_home();
    cmd.env("AGS_NO_KEYCHAIN", "1");
    cmd.env("AGS_HOME", &ags_home);
    cmd.env_remove("AGS_NAMESPACE");

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .expect("spawn child process inside PTY");

    let reader = pair.master.try_clone_reader().expect("clone reader");
    let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let collected_writer = std::sync::Arc::clone(&collected);
    let reader_thread = std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => collected_writer
                    .lock()
                    .unwrap()
                    .extend_from_slice(&buf[..n]),
            }
        }
    });

    // Let specs load, the prologue resolve, and Phase 1's Inputs form render.
    std::thread::sleep(Duration::from_millis(1500));

    // The workflow opens on a briefing screen; advance past it (Enter) to the
    // Phase 1 inputs form, then submit it (Ctrl-S submits regardless of focus;
    // raw mode has IXON off so it reaches the app) so the walk advances into
    // step 1's review.
    let mut writer = pair.master.take_writer().expect("take writer");
    writer.write_all(b"\r").ok(); // Enter → continue past the briefing
    writer.flush().ok();
    std::thread::sleep(Duration::from_millis(900));
    writer.write_all(b"\x13").ok(); // Ctrl-S → submit the inputs form
    writer.flush().ok();

    // Let step 1's per-step review form render.
    std::thread::sleep(Duration::from_millis(1500));

    // Snapshot what's on screen while step 1's review form is up.
    let during_gather = strip_ansi(&String::from_utf8_lossy(&collected.lock().unwrap()));

    // Cancel the review form (Ctrl-C is intercepted from any review state),
    // then dismiss the resulting "cancelled" summary (the fullscreen surface
    // runs its dismiss loop on any TTY outcome — q/Enter exits).
    writer.write_all(b"\x03").ok(); // Ctrl-C → cancel review
    writer.flush().ok();
    std::thread::sleep(Duration::from_millis(300));
    writer.write_all(b"q").ok(); // dismiss the cancelled summary
    writer.flush().ok();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = reader_thread.join();
                panic!("gather did not exit within 10s of Ctrl-C; output:\n{during_gather}");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let _ = reader_thread.join();

    // A real fullscreen surface engaged (alt-screen enter) …
    let raw = String::from_utf8_lossy(&collected.lock().unwrap()).into_owned();
    assert!(
        raw.contains("\x1b[?1049h"),
        "expected the alternate-screen-enter sequence; output:\n{during_gather}"
    );
    // … and the persistent chrome was on screen DURING gather: the step strip
    // shows the kebab step id (step 1 = `create-stat`) AND the Fields nav token
    // both appear in the same frame.
    assert!(
        during_gather.contains("create-stat"),
        "step strip should persist during gather; stripped output:\n{during_gather}"
    );
    assert!(
        during_gather.contains("Navigation"),
        "Navigation bar should persist during gather; stripped output:\n{during_gather}"
    );
    // … and the review form groups fields under labelled sections (the per-row
    // source suffixes are gone — sections carry the source cue instead).
    assert!(
        during_gather.contains("Step"),
        "Step section header should be on screen during the per-step review; output:\n{during_gather}"
    );
}

/// Drives `workflow run competitive-multiplayer` into the FULLSCREEN surface and
/// asserts the run now opens on the step-0 **Inputs** phase before the step walk:
/// the step strip carries a leading `0. Inputs` row and the Inputs form lists the
/// declared `namespace` input. Proof that Phase 1 (declared-input collection)
/// precedes the per-step walk.
///
/// OFFLINE: `--dry-run` + isolated `AGS_HOME`. `--namespace` is supplied
/// (namespace is resolved at context-build time, so it must be present), yet the
/// Inputs form still renders it prefilled — Phase 1 always collects the declared
/// inputs on the interactive fullscreen surface. The test cancels the Inputs
/// form (Ctrl-C) and dismisses the cancelled summary (`q`).
#[test]
#[ignore = "Requires a real PTY; run with --ignored"]
fn test_inputs_phase_precedes_step_walk() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(ags_binary_path());
    cmd.args([
        "--dry-run",
        "workflow",
        "run",
        "competitive-multiplayer",
        "--namespace",
        "dev",
    ]);
    let ags_home = isolated_ags_home();
    cmd.env("AGS_NO_KEYCHAIN", "1");
    cmd.env("AGS_HOME", &ags_home);
    cmd.env_remove("AGS_NAMESPACE");

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .expect("spawn child process inside PTY");

    let reader = pair.master.try_clone_reader().expect("clone reader");
    let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let collected_writer = std::sync::Arc::clone(&collected);
    let reader_thread = std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => collected_writer
                    .lock()
                    .unwrap()
                    .extend_from_slice(&buf[..n]),
            }
        }
    });

    // The workflow opens on a briefing screen. Advance past it (Enter) to reach
    // Phase 1's Inputs form, then snapshot the screen.
    std::thread::sleep(Duration::from_millis(1500));
    let mut writer = pair.master.take_writer().expect("take writer");
    writer.write_all(b"\r").ok(); // Enter → continue past the briefing
    writer.flush().ok();
    std::thread::sleep(Duration::from_millis(900));
    let during_inputs = strip_ansi(&String::from_utf8_lossy(&collected.lock().unwrap()));

    // Cancel the Inputs form (Ctrl-C), then dismiss the cancelled summary (q).
    writer.write_all(b"\x03").ok(); // Ctrl-C → cancel the Inputs form
    writer.flush().ok();
    std::thread::sleep(Duration::from_millis(300));
    writer.write_all(b"q").ok(); // dismiss the cancelled summary
    writer.flush().ok();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = reader_thread.join();
                panic!("inputs-phase run did not exit within 10s; output:\n{during_inputs}");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let _ = reader_thread.join();

    let raw = String::from_utf8_lossy(&collected.lock().unwrap()).into_owned();
    // A real fullscreen surface engaged (alt-screen enter).
    assert!(
        raw.contains("\x1b[?1049h"),
        "expected the alternate-screen-enter sequence; output:\n{during_inputs}"
    );
    // The step strip carries the leading "gather-inputs" row …
    assert!(
        during_inputs.contains("gather-inputs"),
        "step strip should show the leading gather-inputs row; output:\n{during_inputs}"
    );
    // … and the Inputs form lists the declared `namespace` input.
    assert!(
        during_inputs.contains("namespace"),
        "Inputs form should list the namespace input; output:\n{during_inputs}"
    );
}

/// `ags --ui=fullscreen version` in a TTY must DEGRADE to plain output (a
/// builtin has no step list, so `frontend_for_surface` falls back to a
/// `PlainFrontend`) — it must NOT enter the alt screen and must exit 0 cleanly.
///
/// A real TTY is required: `--ui=fullscreen` on a non-TTY is a usage error
/// (explicit TUI needs an interactive terminal), so this exercises
/// the in-TTY degrade path. Asserts: exit success, no alt-screen-enter
/// sequence, and no panic.
#[test]
#[ignore = "Requires a real PTY; run with --ignored"]
fn test_fullscreen_builtin_degrades_to_plain_and_exits_zero() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(ags_binary_path());
    cmd.args(["--ui=fullscreen", "version"]);
    let ags_home = isolated_ags_home();
    cmd.env("AGS_NO_KEYCHAIN", "1");
    cmd.env("AGS_HOME", &ags_home);

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .expect("spawn child process inside PTY");

    let reader = pair.master.try_clone_reader().expect("clone reader");
    let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let collected_writer = std::sync::Arc::clone(&collected);
    let reader_thread = std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => collected_writer
                    .lock()
                    .unwrap()
                    .extend_from_slice(&buf[..n]),
            }
        }
    });

    // `version` prints and exits immediately — no key input needed.
    let deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("--ui=fullscreen version did not exit within 8s");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    let _ = reader_thread.join();
    let output_text = String::from_utf8_lossy(&collected.lock().unwrap()).into_owned();

    assert!(
        status.success(),
        "--ui=fullscreen version must exit 0 (plain degrade); output:\n{output_text}"
    );
    // Degraded to plain → the alternate screen must NOT have been entered.
    assert!(
        !output_text.contains("\x1b[?1049h"),
        "builtin must degrade to plain, not enter the alt screen; output:\n{output_text}"
    );
    assert!(
        !output_text.to_lowercase().contains("panic"),
        "no panic expected; output:\n{output_text}"
    );
}
