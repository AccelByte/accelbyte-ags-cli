use assert_cmd::Command;

/// Base command builder for the ags binary.
pub fn ags() -> Command {
    Command::cargo_bin("ags").unwrap()
}

/// Command isolated from real credentials and config state.
/// Uses AGS_NO_KEYCHAIN=1 and a unique temp config directory per call
/// to prevent token/config bleed between tests.
pub fn ags_isolated() -> Command {
    let mut command = ags();
    let unique_dir = std::env::temp_dir()
        .join(format!("ags-test-{}", std::process::id()))
        .join(format!("{:?}", std::thread::current().id()));
    command
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_HOME", unique_dir);
    command
}

/// Command isolated from real credentials with a custom base URL.
/// Useful for tests that point at a wiremock server.
pub fn ags_with_base_url(base_url: &str) -> Command {
    let mut command = ags_isolated();
    command.env("AGS_BASE_URL", base_url);
    command
}
