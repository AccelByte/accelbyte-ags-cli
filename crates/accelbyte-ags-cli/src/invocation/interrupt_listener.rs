//! Ctrl-C listener that is registered the moment it is created.
//!
//! `tokio::signal::ctrl_c()` is an `async fn`: it registers its listener only
//! when the returned future is first polled, and registering marks every
//! earlier Ctrl-C as already seen. A command that tells the user it is ready
//! and only then waits on `ctrl_c()` loses a Ctrl-C that lands in between.
//! Nothing else acts on it either, because the global interrupt handler
//! defers to a command that owns its interrupt path. Creating a fresh
//! `ctrl_c()` future on every pass of a `select!` loop has the same gap
//! between passes.
//!
//! [`InterruptListener::register`] registers synchronously. A command calls it
//! before it signals readiness and then waits on [`InterruptListener::recv`]
//! as often as it needs to.
//!
//! On Unix, the test in this file checks that a SIGINT sent after `register`
//! returns, and before the first `recv`, is seen. On Windows the synchronous
//! registration and the behaviour of `recv` rest only on reading tokio's
//! source (1.53.3, the locked version): in `signal/windows/sys.rs`, `fn new`
//! installs the console handler with `SetConsoleCtrlHandler` (once per
//! process) and subscribes the listener before `windows::ctrl_c()` returns;
//! in `signal/windows.rs`, `CtrlC::recv` waits on the `RxFuture` from
//! `signal/mod.rs`. There is no test for Windows, because a self-test would
//! have to call `GenerateConsoleCtrlEvent`, which sends Ctrl-C to every
//! process attached to the console, `cargo` included.

/// A registered Ctrl-C listener (SIGINT on Unix). A Ctrl-C delivered after
/// [`register`](Self::register) returns is seen by the next
/// [`recv`](Self::recv), whether or not `recv` was being polled when it
/// arrived.
pub(crate) struct InterruptListener {
    #[cfg(unix)]
    inner: tokio::signal::unix::Signal,
    #[cfg(windows)]
    inner: tokio::signal::windows::CtrlC,
}

impl InterruptListener {
    /// Register the listener now, before returning. Call it from inside a
    /// Tokio runtime with its signal driver enabled (`enable_all`).
    ///
    /// Fails when the operating system refuses the signal handler. A caller
    /// that relies on Ctrl-C to stop must treat that as fatal.
    pub(crate) fn register() -> std::io::Result<Self> {
        #[cfg(test)]
        {
            if FAIL_NEXT_REGISTRATION.with(|flag| flag.replace(false)) {
                return Err(std::io::Error::other(
                    "Ctrl-C registration failure requested by a test",
                ));
            }
        }

        #[cfg(unix)]
        let inner = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        #[cfg(windows)]
        let inner = tokio::signal::windows::ctrl_c()?;

        Ok(Self { inner })
    }

    /// Wait for the next Ctrl-C.
    ///
    /// Cancel safe: dropping this future, for example when another
    /// `tokio::select!` branch completes first, loses no Ctrl-C, so a loop
    /// can call it on every pass. Tokio documents this for the Unix
    /// `Signal::recv` in its "Cancel safety" section. The Windows
    /// `CtrlC::recv` has no such section, but it waits on the same
    /// watch-based `RxFuture` that `Signal::recv` waits on.
    pub(crate) async fn recv(&mut self) {
        // Tokio documents that neither platform's `recv` returns `None`; the
        // value carries nothing, so it is discarded.
        self.inner.recv().await;
    }
}

#[cfg(test)]
thread_local! {
    /// Set by [`fail_next_registration_on_this_thread`] and cleared by the
    /// next [`InterruptListener::register`] call on the same thread.
    static FAIL_NEXT_REGISTRATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Make the next [`InterruptListener::register`] call on this thread fail, so
/// a caller's registration-failure path can be tested. The flag is
/// thread-local, so a test running on another thread is unaffected, and the
/// one call that sees it clears it.
#[cfg(test)]
pub(crate) fn fail_next_registration_on_this_thread() {
    FAIL_NEXT_REGISTRATION.with(|flag| flag.set(true));
}

#[cfg(test)]
mod tests {
    /// A SIGINT sent after `register` returns, and already dispatched to
    /// every listener, is seen by the listener's first `recv`. This is the
    /// order a command that owns its interrupt path depends on: register,
    /// signal readiness, and only later start waiting.
    ///
    /// The witness is a second, independent listener. It makes SIGINT safe to
    /// raise in this process whatever `register` does, and waiting on it
    /// proves the signal was dispatched before `listener` is first polled. A
    /// listener that registers lazily, on its first poll, subscribes after
    /// that dispatch and never sees the signal, so this test times out.
    ///
    /// Serial: every SIGINT listener in this test process sees the raised
    /// signal, so the tests that run a tunnel loop are serial too.
    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::serial]
    async fn test_interrupt_listener_sees_a_signal_sent_before_its_first_poll() {
        let mut listener =
            super::InterruptListener::register().expect("registering the listener must succeed");
        let mut witness = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            .expect("registering the witness listener must succeed");

        // SAFETY: `raise` takes no pointers. Tokio's SIGINT handler is
        // installed above, so the signal is recorded instead of ending the
        // test process.
        let raised = unsafe { libc::raise(libc::SIGINT) };
        assert_eq!(raised, 0, "raise(SIGINT) must succeed");

        tokio::time::timeout(std::time::Duration::from_secs(10), witness.recv())
            .await
            .expect("the witness must see the raised SIGINT within 10 s");

        tokio::time::timeout(std::time::Duration::from_secs(10), listener.recv())
            .await
            .expect(
                "a SIGINT sent after register() and before the first recv() must reach recv() \
                 within 10 s",
            );
    }
}
