//! Child-process helpers: bounded waits that prevent a hung subprocess from
//! blocking the CLI indefinitely.

use std::process::{Child, Output};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Error returned when [`wait_with_timeout`] cannot produce normal `Output`.
#[derive(Debug)]
pub enum WaitError {
    /// The child did not exit within the allowed duration. The child has
    /// already been killed and reaped before this variant is returned.
    TimedOut(Duration),
    /// The OS `try_wait` or `wait` call failed, or a reader thread
    /// panicked while draining the child's output.
    Wait(std::io::Error),
}

/// Grace period for collecting reader-thread output after the child exits.
///
/// Once the child has exited, a healthy reader thread finishes in
/// microseconds — it reads until EOF, and the pipe closes when the
/// child exits. A longer delay means a grandchild inherited the pipe
/// write end and is keeping it open, so joining that thread would block
/// indefinitely — the same unbounded hang that the timeout and error
/// paths detach for. Three seconds is generous enough for slow I/O
/// finalization while still preventing a hang.
const JOIN_GRACE: Duration = Duration::from_secs(3);

/// Handles for drain threads spawned by [`spawn_drains`].
///
/// Pass this to [`wait_with_drains`] after performing any pre-wait I/O
/// (e.g. writing to the child's stdin). Dropping the handles without
/// calling [`wait_with_drains`] detaches the drain threads, which is
/// safe — they own their data and touch no shared mutable state.
pub struct DrainHandles {
    stdout: Option<DrainPair>,
    stderr: Option<DrainPair>,
}

/// A drain thread paired with a channel receiver for its output.
struct DrainPair {
    rx: mpsc::Receiver<Vec<u8>>,
    /// Kept solely so it can be explicitly detached (dropped) on the
    /// timeout / error paths. On the success path we collect output
    /// via `recv_timeout` on the channel instead of joining.
    _thread: std::thread::JoinHandle<()>,
}

/// Spawn background threads that drain stdout and stderr from `child`.
///
/// Each thread reads from its pipe using [`read_bounded`] and sends the
/// buffer through an [`mpsc`] channel. The caller collects the output
/// via the channel receivers in [`DrainHandles`] rather than joining
/// the threads, which avoids an unbounded join when a grandchild holds
/// the pipe write end open.
///
/// Call this **before** writing to the child's stdin if the child may
/// produce output before consuming all of its input — otherwise a
/// pipe-buffer deadlock is possible (the child blocks writing to full
/// stdout while the parent blocks writing to full stdin, and nobody is
/// draining either pipe).
pub fn spawn_drains(child: &mut Child) -> DrainHandles {
    DrainHandles {
        stdout: child.stdout.take().map(spawn_one_drain),
        stderr: child.stderr.take().map(spawn_one_drain),
    }
}

fn spawn_one_drain(reader: impl std::io::Read + Send + 'static) -> DrainPair {
    let (tx, rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let buf = read_bounded(reader);
        // If the receiver has been dropped (e.g. the wait timed out and
        // the caller moved on) the send fails harmlessly.
        let _ = tx.send(buf);
    });
    DrainPair {
        rx,
        _thread: thread,
    }
}

/// Collect output from a drain pair, bounded by [`JOIN_GRACE`].
///
/// Returns the buffer on success, or an error message if the reader
/// thread panicked (sender dropped without sending).
fn collect_drain(pair: Option<DrainPair>) -> Result<Vec<u8>, &'static str> {
    let Some(pair) = pair else {
        return Ok(Vec::new());
    };
    match pair.rx.recv_timeout(JOIN_GRACE) {
        Ok(buf) => Ok(buf),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // A grandchild holds the pipe open — same scenario the
            // timeout path detaches for. Return empty rather than
            // blocking. The thread is detached when `_thread` is
            // dropped with the `DrainPair`.
            Ok(Vec::new())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            // The sender was dropped without sending — the reader
            // thread panicked. Surface this so a future bug in
            // `read_bounded` is reported as a panic rather than
            // silently producing "the child had no output".
            Err("reader thread panicked")
        }
    }
}

/// Wait for `child` to exit, enforcing a maximum wall-clock duration.
///
/// stdout and stderr are drained on background threads to prevent pipe-buffer
/// deadlocks (a child that fills the OS pipe buffer blocks until the reader
/// drains it; polling `try_wait` without reading would never converge).
///
/// On timeout the child is killed (`SIGKILL` / `TerminateProcess`) and reaped
/// before returning `Err(WaitError::TimedOut(..))`, so no zombie is left
/// behind.
///
/// On success the returned [`Output`] is identical to what
/// `Child::wait_with_output` would have produced, except that stdout and
/// stderr are each capped at [`MAX_STREAM_BYTES`].
pub fn wait_with_timeout(mut child: Child, timeout: Duration) -> Result<Output, WaitError> {
    let drains = spawn_drains(&mut child);
    wait_with_drains(child, timeout, drains)
}

/// Like [`wait_with_timeout`], but accepts pre-spawned [`DrainHandles`].
///
/// Use this when you need the drain threads running **before** additional
/// I/O with the child (e.g. writing to stdin). Call [`spawn_drains`]
/// first, perform the I/O, then pass the handles here.
pub fn wait_with_drains(
    mut child: Child,
    timeout: Duration,
    drains: DrainHandles,
) -> Result<Output, WaitError> {
    let start = Instant::now();
    let poll_interval = Duration::from_millis(50);

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    // Deadline exceeded — kill and reap so no zombie remains.
                    let _ = child.kill();
                    let _ = child.wait();
                    // Deliberately detach (drop) the drain threads rather
                    // than joining them. `Child::kill()` terminates only the
                    // direct child pid — it does NOT kill grandchildren (e.g.
                    // `git-remote-https` spawned by `git clone`, or helper
                    // processes spawned by `docker login`). A surviving
                    // grandchild inherits the stdout/stderr write ends of the
                    // pipe, so the read end never reaches EOF while that
                    // grandchild is alive. `read_bounded` blocks in `read`
                    // waiting for EOF, and an unbounded `join()` would block
                    // with it — converting the bounded timeout into the
                    // unbounded wait this function exists to prevent.
                    //
                    // Dropping the `DrainHandles` detaches the threads: they
                    // will finish on their own once the pipe finally closes
                    // (when the grandchild exits or is reaped by the OS).
                    // This is safe — the threads own their data, touch no
                    // shared mutable state, and the pipe read-end they hold
                    // is independent of any resource the caller uses after
                    // this return.
                    drop(drains);
                    return Err(WaitError::TimedOut(timeout));
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => {
                // OS error from try_wait — kill defensively and report.
                let _ = child.kill();
                let _ = child.wait();
                // Detach drain threads — same rationale as the TimedOut
                // path above: a grandchild may hold the pipe open, and
                // joining would block indefinitely.
                drop(drains);
                return Err(WaitError::Wait(e));
            }
        }
    };

    // Child has exited; collect output from the drain threads with a
    // bounded grace period. If a reader thread does not deliver within
    // JOIN_GRACE (because a grandchild holds the pipe open), that
    // stream comes back empty — partial data beats a hang, the same
    // trade MAX_STREAM_BYTES already makes.
    //
    // If a reader thread panicked (sender dropped without sending), the
    // panic is surfaced as a WaitError::Wait rather than silently
    // returning an empty buffer, so a future bug in `read_bounded` is
    // reported instead of manifesting as "the child produced no output".
    let stdout = collect_drain(drains.stdout)
        .map_err(|msg| WaitError::Wait(std::io::Error::other(format!("stdout {msg}"))))?;
    let stderr = collect_drain(drains.stderr)
        .map_err(|msg| WaitError::Wait(std::io::Error::other(format!("stderr {msg}"))))?;

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Maximum bytes retained per stream (stdout or stderr).
///
/// 8 MiB is generous for the output of `git clone` progress or
/// `docker login` credential exchange — both call sites already truncate
/// stderr to 512 bytes for display. The cap prevents a remote that streams
/// endlessly (e.g. a malicious git server) from growing CLI memory for the
/// full timeout window.
const MAX_STREAM_BYTES: usize = 8 * 1024 * 1024;

/// Read from `reader` into a buffer capped at `MAX_STREAM_BYTES`.
///
/// After the cap is reached the function continues reading and discarding
/// bytes until EOF. Stopping the read would re-introduce the pipe-buffer
/// deadlock that this helper exists to prevent: the child would block
/// trying to write to a full pipe and never exit, defeating the timeout.
fn read_bounded(mut reader: impl std::io::Read) -> Vec<u8> {
    read_bounded_inner(&mut reader, MAX_STREAM_BYTES)
}

/// Inner implementation with an injectable cap for testing.
fn read_bounded_inner(reader: &mut dyn std::io::Read, cap: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];

    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break, // EOF
            Ok(n) => {
                let remaining = cap.saturating_sub(buf.len());
                if remaining > 0 {
                    let keep = n.min(remaining);
                    buf.extend_from_slice(&chunk[..keep]);
                }
                // Bytes past the cap are silently discarded — the pipe is
                // still drained so the child can continue writing and exit.
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    // ---------------------------------------------------------------
    // A child that exits quickly succeeds within any reasonable timeout.
    // ---------------------------------------------------------------
    #[test]
    fn test_child_within_deadline_returns_output() {
        let child = Command::new("cargo")
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("cargo must be available in the test environment");

        let result = wait_with_timeout(child, Duration::from_secs(30));
        let output = result.expect("cargo --version must finish within 30s");
        assert!(output.status.success());
        assert!(
            !output.stdout.is_empty(),
            "cargo --version should produce stdout"
        );
    }

    // ---------------------------------------------------------------
    // A child that exceeds its deadline is killed and a TimedOut error
    // is returned. We use a long-sleeping process as the stalled child.
    // ---------------------------------------------------------------
    #[test]
    fn test_child_exceeding_deadline_returns_timed_out() {
        // Spawn a process that sleeps far longer than the timeout.
        let child = long_sleep_child();

        let start = Instant::now();
        let result = wait_with_timeout(child, Duration::from_millis(200));
        let elapsed = start.elapsed();

        assert!(
            matches!(result, Err(WaitError::TimedOut(_))),
            "expected TimedOut, got: {result:?}"
        );
        // The function should return promptly after the deadline, not after
        // the sleep process would have exited naturally.
        assert!(
            elapsed < Duration::from_secs(5),
            "wait_with_timeout should return near the deadline, took {elapsed:?}"
        );
    }

    // ---------------------------------------------------------------
    // After a timeout, the child process must no longer be running.
    // We verify by attempting to wait on its PID (which would fail or
    // return immediately because the process was already reaped).
    // ---------------------------------------------------------------
    #[test]
    fn test_child_is_reaped_after_timeout() {
        let child = long_sleep_child();
        let pid = child.id();

        let result = wait_with_timeout(child, Duration::from_millis(200));
        assert!(matches!(result, Err(WaitError::TimedOut(_))));

        // The child was killed and reaped inside wait_with_timeout.
        // On all platforms, the PID should no longer refer to a running
        // process owned by us. We verify by checking that the OS does
        // not report it as alive.
        assert_process_not_running(pid);
    }

    // ---------------------------------------------------------------
    // A child that finishes within the deadline is entirely unaffected:
    // its exit code, stdout, and stderr are faithfully returned.
    // ---------------------------------------------------------------
    #[test]
    fn test_successful_child_output_preserved() {
        // `cargo --version` prints something like "cargo 1.xx.x (...)"
        let child = Command::new("cargo")
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("cargo must be available");

        let output = wait_with_timeout(child, Duration::from_secs(30)).unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("cargo"),
            "stdout should contain 'cargo', got: {stdout}"
        );
    }

    // ---------------------------------------------------------------
    // The TimedOut variant carries the configured duration so callers
    // can include it in the error message.
    // ---------------------------------------------------------------
    #[test]
    fn test_timed_out_carries_configured_duration() {
        let child = long_sleep_child();
        let timeout = Duration::from_millis(150);

        let result = wait_with_timeout(child, timeout);
        match result {
            Err(WaitError::TimedOut(d)) => assert_eq!(d, timeout),
            other => panic!("expected TimedOut, got: {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // The bounded reader retains at most `cap` bytes and discards the
    // rest, preventing unbounded memory growth from a verbose or
    // adversarial subprocess.
    // ---------------------------------------------------------------
    #[test]
    fn test_bounded_reader_caps_output() {
        let cap: usize = 256;
        // Feed more than `cap` bytes through the bounded reader.
        let input_data = vec![0xABu8; cap * 4];
        let cursor = std::io::Cursor::new(input_data.clone());

        let result = read_bounded_inner(&mut cursor.clone(), cap);

        assert_eq!(
            result.len(),
            cap,
            "retained buffer must be exactly the cap ({cap}), got {}",
            result.len()
        );
        // The retained prefix must be the first `cap` bytes of the input.
        assert_eq!(
            &result[..],
            &input_data[..cap],
            "retained bytes must be the first {cap} bytes of input"
        );
    }

    #[test]
    fn test_bounded_reader_under_cap_returns_all() {
        let cap: usize = 1024;
        let input_data = vec![0x42u8; 100];
        let cursor = std::io::Cursor::new(input_data.clone());

        let result = read_bounded_inner(&mut cursor.clone(), cap);

        assert_eq!(
            result.len(),
            input_data.len(),
            "input under cap must be returned in full"
        );
        assert_eq!(&result[..], &input_data[..]);
    }

    #[test]
    fn test_bounded_reader_subprocess_cap_holds() {
        // Spawn a real child that emits more than the test cap and prove
        // the returned stdout is capped. We use a small cap here so the
        // test completes quickly without generating 8 MiB of output.
        let test_cap: usize = 512;

        // Generate `test_cap * 4` bytes of output via a child process.
        #[cfg(windows)]
        let mut child = {
            // PowerShell: emit a string of 'A' repeated N times.
            let count = test_cap * 4;
            Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!("[Console]::Out.Write('A' * {count})"),
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("powershell must be available on Windows")
        };
        #[cfg(not(windows))]
        let mut child = {
            let count = test_cap * 4;
            Command::new("dd")
                .args([
                    "if=/dev/zero",
                    &format!("bs={count}"),
                    "count=1",
                    "status=none",
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("dd must be available on Unix")
        };

        // Take the stdout pipe and read through the bounded reader using
        // the test cap instead of the production constant.
        let mut stdout_pipe = child.stdout.take().expect("stdout must be piped");
        let result = read_bounded_inner(&mut stdout_pipe, test_cap);

        // Drop the pipe handle so the child can detect EOF if it is
        // still writing, then reap the child to avoid zombie processes.
        drop(stdout_pipe);
        let _ = child.wait();

        assert!(
            result.len() <= test_cap,
            "stdout must be capped at {test_cap} bytes, got {}",
            result.len()
        );
        // The child should have written more than cap, so the output is
        // exactly at the cap — not short-circuited at a smaller size.
        assert_eq!(
            result.len(),
            test_cap,
            "child wrote more than cap, so retained output should be exactly {test_cap}"
        );
    }

    // ---------------------------------------------------------------
    // The drain-past-cap behaviour prevents pipe-buffer deadlock even
    // when the child writes far more than the OS pipe buffer (typically
    // 64 KiB). This test uses a 256 KiB child write with a small cap
    // to prove the reader drains the entire pipe without hanging and
    // retains exactly `cap` bytes.
    // ---------------------------------------------------------------
    #[test]
    fn test_bounded_reader_drains_above_os_pipe_buffer() {
        // 256 KiB — well above the ~64 KiB typical OS pipe buffer.
        let child_output_bytes: usize = 256 * 1024;
        // Cap well below the pipe buffer so the reader MUST drain past
        // it to avoid deadlocking the child.
        let test_cap: usize = 4096;

        #[cfg(windows)]
        let mut child = {
            Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!("[Console]::Out.Write('A' * {child_output_bytes})"),
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("powershell must be available on Windows")
        };
        #[cfg(not(windows))]
        let mut child = {
            Command::new("dd")
                .args([
                    "if=/dev/zero",
                    &format!("bs={child_output_bytes}"),
                    "count=1",
                    "status=none",
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("dd must be available on Unix")
        };

        let mut stdout_pipe = child.stdout.take().expect("stdout must be piped");
        let result = read_bounded_inner(&mut stdout_pipe, test_cap);

        // Wait for the child to exit BEFORE dropping the pipe handle.
        // This ordering is load-bearing for the drain proof:
        //
        // Correct (drain past cap): the reader consumed all 256 KiB
        //   (retaining only `test_cap`) and hit EOF when the child
        //   finished writing and exited. `wait()` returns immediately.
        //
        // Broken (break at cap, no drain): the reader stopped after
        //   `test_cap` bytes. The child is blocked on a full pipe
        //   (~64 KiB OS buffer) whose read end is still open (held by
        //   `stdout_pipe`). `wait()` hangs because the child cannot
        //   exit while blocked on write → test hangs.
        let _ = child.wait();
        drop(stdout_pipe);

        assert_eq!(
            result.len(),
            test_cap,
            "retained buffer must be exactly the cap ({test_cap}), got {}; \
             the reader must drain past the OS pipe buffer and retain the first cap bytes",
            result.len()
        );
    }

    // ---------------------------------------------------------------
    // F11: The bounded success-path join returns empty rather than
    // blocking indefinitely when a reader thread does not deliver
    // within JOIN_GRACE (e.g. a grandchild holds the pipe open).
    // ---------------------------------------------------------------
    #[test]
    fn test_bounded_join_returns_when_reader_stalls() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let thread = std::thread::spawn(move || {
            // Simulate a reader blocked on a grandchild-held pipe:
            // sleep far longer than JOIN_GRACE, then try to send.
            std::thread::sleep(Duration::from_secs(120));
            let _ = tx.send(vec![1, 2, 3]);
        });
        let pair = super::DrainPair {
            rx,
            _thread: thread,
        };

        let start = Instant::now();
        let result = super::collect_drain(Some(pair));
        let elapsed = start.elapsed();

        assert!(result.is_ok(), "grace timeout must return Ok, not Err");
        assert!(
            result.unwrap().is_empty(),
            "timed-out drain must return empty"
        );
        // collect_drain must return near JOIN_GRACE, not after 120s.
        assert!(
            elapsed < Duration::from_secs(10),
            "collect_drain should return near JOIN_GRACE ({:?}), took {elapsed:?}",
            super::JOIN_GRACE
        );
    }

    // ---------------------------------------------------------------
    // F13: A panicking reader thread is surfaced as an error rather
    // than silently returning an empty buffer. Without this, a future
    // bug in `read_bounded` manifests as "the child produced no
    // output" and misdirects debugging.
    //
    // Note: this test produces a panic message on stderr from the
    // spawned thread. That is expected — the test verifies the panic
    // is surfaced through the channel, not suppressed.
    // ---------------------------------------------------------------
    #[test]
    fn test_reader_thread_panic_surfaces_as_error() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let thread = std::thread::spawn(move || {
            // Move tx into the thread so it is dropped on panic unwind,
            // disconnecting the channel.
            let _tx = tx;
            panic!("simulated reader-thread panic");
        });
        // Give the thread time to panic and drop the sender.
        std::thread::sleep(Duration::from_millis(200));

        let pair = super::DrainPair {
            rx,
            _thread: thread,
        };
        let result = super::collect_drain(Some(pair));

        assert!(
            result.is_err(),
            "panicked reader must produce Err, not Ok; got: {result:?}"
        );
        assert!(
            result.unwrap_err().contains("panic"),
            "error message must mention panic"
        );
    }

    // ---------------------------------------------------------------
    // F12: spawn_drains takes stdout and stderr from the child,
    // confirming the drain threads are started before the caller
    // performs any stdin I/O. After spawn_drains the child's stdout
    // and stderr are None (pipes moved to drain threads).
    // ---------------------------------------------------------------
    #[test]
    fn test_spawn_drains_takes_stdout_and_stderr() {
        let mut child = Command::new("cargo")
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("cargo must be available");

        assert!(
            child.stdout.is_some(),
            "stdout must be piped before spawn_drains"
        );
        assert!(
            child.stderr.is_some(),
            "stderr must be piped before spawn_drains"
        );

        let drains = spawn_drains(&mut child);

        assert!(child.stdout.is_none(), "spawn_drains must take stdout");
        assert!(child.stderr.is_none(), "spawn_drains must take stderr");

        // Clean up: wait for the child using the drains.
        let result = wait_with_drains(child, Duration::from_secs(10), drains);
        assert!(result.is_ok(), "cargo --version must succeed");
    }

    // ---------------------------------------------------------------
    // F12: Pre-started drains prevent pipe-buffer deadlock when the
    // child writes to stdout before consuming stdin. The child writes
    // 128 KiB to stdout (exceeding the ~64 KiB OS pipe buffer), then
    // reads one byte from stdin. Without drains the child blocks on
    // full stdout and never reads stdin, causing a timeout. With
    // drains active the output is consumed, the child proceeds to
    // read stdin, and the test completes within the deadline.
    // ---------------------------------------------------------------
    #[test]
    fn test_drains_before_stdin_write_prevents_deadlock() {
        #[cfg(windows)]
        let mut child = {
            Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    "[Console]::Out.Write('A' * 131072); $null = [Console]::In.Read()",
                ])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("powershell must be available on Windows")
        };
        #[cfg(not(windows))]
        let mut child = {
            Command::new("sh")
                .args([
                    "-c",
                    "dd if=/dev/zero bs=131072 count=1 status=none; head -c 1 > /dev/null",
                ])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("sh must be available on Unix")
        };

        // Start drains BEFORE writing to stdin — the key ordering.
        let drains = spawn_drains(&mut child);

        // Write one byte to stdin so the child's stdin-read unblocks.
        {
            use std::io::Write;
            let mut stdin = child.stdin.take().expect("stdin must be piped");
            stdin.write_all(b"x").expect("stdin write must succeed");
            drop(stdin);
        }

        let result = wait_with_drains(child, Duration::from_secs(10), drains);
        assert!(
            result.is_ok(),
            "must complete without deadlock; got: {result:?}"
        );
    }

    // ── helpers ──

    /// Spawn a child process that sleeps for a long time, suitable for
    /// timeout testing. Uses `ping` on Windows and `sleep` on Unix —
    /// both are universally available without extra dependencies.
    fn long_sleep_child() -> Child {
        #[cfg(windows)]
        {
            // `ping -n 60 127.0.0.1` blocks for ~60 seconds on Windows.
            Command::new("ping")
                .args(["-n", "60", "127.0.0.1"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("ping must be available on Windows")
        }
        #[cfg(not(windows))]
        {
            Command::new("sleep")
                .arg("60")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("sleep must be available on Unix")
        }
    }

    /// Assert that a process with the given PID is no longer running.
    /// Uses OS-specific external commands rather than libc so the test
    /// runs without a platform-specific crate dependency.
    fn assert_process_not_running(pid: u32) {
        #[cfg(windows)]
        {
            let output = Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/NH"])
                .output()
                .expect("tasklist must be available on Windows");
            let text = String::from_utf8_lossy(&output.stdout);
            // tasklist prints "INFO: No tasks are running..." when the PID
            // is absent. When the process is alive, the output contains its
            // image name on a non-INFO line.
            assert!(
                text.contains("INFO:") || !text.contains(&pid.to_string()),
                "process {pid} should not be running, tasklist output: {text}"
            );
        }
        #[cfg(not(windows))]
        {
            // `kill -0 <pid>` exits 0 if the process exists, non-zero otherwise.
            let status = Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("kill must be available on Unix");
            assert!(
                !status.success(),
                "process {pid} should not be running (kill -0 returned success)"
            );
        }
    }
}
