//! Output contract for `ags auth token`.
//!
//! The command's contract is unusually strict because its output is consumed
//! by substitution, not read: stdout carries the token and nothing else, and
//! every other channel of the CLI's usual chrome must stay on stderr.

use crate::common::cli_helpers::ags_isolated;

// ── Channel routing ──

#[test]
fn test_auth_token_stdout_is_exactly_the_token() {
    let output = ags_isolated()
        .env("AGS_ACCESS_TOKEN", "contract-token")
        .args(["auth", "token"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "contract-token\n",
        "stdout must carry the token plus one trailing newline and nothing else"
    );
}

#[test]
fn test_auth_token_quiet_still_prints_the_token() {
    let output = ags_isolated()
        .env("AGS_ACCESS_TOKEN", "contract-token")
        .args(["--quiet", "auth", "token"])
        .output()
        .unwrap();

    assert!(output.status.success());
    // --quiet suppresses chrome, not the payload: a --quiet run that printed
    // nothing would make the flag silently break every caller.
    assert_eq!(String::from_utf8_lossy(&output.stdout), "contract-token\n");
}

#[test]
fn test_auth_token_failure_leaves_stdout_empty() {
    let output = ags_isolated()
        .env_remove("AGS_ACCESS_TOKEN")
        .env_remove("AGS_CLIENT_ID")
        .env_remove("AGS_CLIENT_SECRET")
        .args(["auth", "token"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "",
        "an error must not be captured by $(ags auth token)"
    );
    assert!(!output.stderr.is_empty(), "the error belongs on stderr");
}

// ── JSON mode ──

#[test]
fn test_auth_token_json_field_names_contract() {
    let output = ags_isolated()
        .env("AGS_ACCESS_TOKEN", "contract-token")
        .args(["auth", "token", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let obj = json.as_object().unwrap();

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["access_token", "expires_at", "source"],
        "the documented JSON contract is exactly these three fields"
    );
}

#[test]
fn test_auth_token_json_carries_no_human_guidance() {
    let output = ags_isolated()
        .env("AGS_ACCESS_TOKEN", "contract-token")
        .args(["auth", "token", "--format", "json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let obj = json.as_object().unwrap();
    for key in ["fix", "tip", "next", "warnings"] {
        assert!(
            !obj.contains_key(key),
            "JSON must not contain human guidance field '{key}'"
        );
    }
}
