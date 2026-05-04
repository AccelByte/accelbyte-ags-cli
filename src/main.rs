mod catalogue;
mod errors;
mod frontend;
mod invocation;
mod protocol;
mod runtime;
mod support;

/// Process entry point: reset SIGPIPE, build the Tokio runtime, and drive `invocation::run`.
fn main() {
    // Reset SIGPIPE before the Tokio runtime is constructed so the invariant
    // "no other threads exist" is guaranteed — the #[tokio::main] expansion
    // runs reset_sigpipe inside the runtime body, which technically violates
    // the SAFETY comment even though it is safe in practice today.
    reset_sigpipe();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(error) => {
            crate::frontend::write_stderr_line(&format!("ags: failed to start runtime: {error}"));
            std::process::exit(5);
        }
    };

    runtime.block_on(async {
        if let Err(e) = invocation::run().await {
            // Bare-stderr escape hatch: reached only when the Frontend could not
            // be initialised (e.g. `frontend.enter()` failed), so no trait-based
            // rendering is available. All other errors are rendered inside `run`
            // via `Frontend::render_error`.
            crate::frontend::write_stderr_line(&format!("ags: {e}"));
            std::process::exit(e.exit_code());
        }
    });
}

/// Restore default SIGPIPE handling on Unix so piping into `head`, `less`,
/// etc. terminates cleanly (exit 141) instead of panicking from a broken
/// pipe in `println!` / `eprintln!`.
fn reset_sigpipe() {
    #[cfg(unix)]
    // SAFETY: called from the synchronous `main` before the Tokio runtime is
    // constructed, so no other threads exist. Signal disposition is
    // process-wide and inherited by every thread spawned later.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}
