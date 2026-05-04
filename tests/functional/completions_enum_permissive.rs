//! Verify that enum-typed flag values are tab-completable (via PossibleValue
//! metadata) but NOT rejected client-side by clap.
//!
//! Uses `platform item search --item-type`, which advertises a closed set of
//! possible values (APP, BUNDLE, CODE, …) in help text. A bogus value must
//! pass through clap parsing and reach the dispatch layer, where it will fail
//! for auth/network reasons — not with clap's "invalid value" message.

use assert_cmd::Command;

#[test]
fn test_enum_flag_accepts_unknown_value_past_parse() {
    let mut cmd = Command::cargo_bin("ags").unwrap();
    let output = cmd
        .args([
            "platform",
            "item",
            "search",
            "--namespace",
            "fake-ns",
            "--keyword",
            "fake-keyword",
            "--language",
            "en",
            "--item-type",
            "NOT_A_REAL_ENUM_VALUE_xyz",
        ])
        .env("AGS_PROFILE", "does-not-exist-test-profile")
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("possible values:")
            && !stderr.contains("invalid value")
            && !stderr.to_lowercase().contains("isn't a valid"),
        "expected permissive parser; got clap enum rejection:\n{stderr}"
    );
}
