use crate::common::cli_helpers::ags;
use predicates::prelude::*;

#[test]
fn test_service_name_rejects_path_traversal() {
    ags()
        .args(["../../../etc/passwd", "--help"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown service"));
}

#[test]
fn test_service_name_rejects_absolute_path() {
    ags()
        .args(["/etc/passwd", "--help"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown service"));
}

#[test]
fn test_service_name_rejects_dot_dot_slash() {
    ags()
        .args(["..%2F..%2Fetc%2Fpasswd", "--help"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown service"));
}
