//! The file-upload stop-gap guard must reject multipart (`type: file`,
//! `multipart/form-data`) commands cleanly — before gathering inputs and
//! before auth — in both normal and `--dry-run` modes. Regression for a panic
//! in request assembly (`resolve.rs` used to `unreachable!()` on form-data).

use crate::common::cli_helpers::ags_isolated;
use predicates::prelude::*;

/// A real file-upload command (csm app-ui upload-assets) is rejected with the
/// Admin-Portal message and does not panic.
#[test]
fn test_file_upload_command_rejected_with_admin_portal_hint() {
    ags_isolated()
        .args(["csm", "app-ui", "upload-assets", "--namespace", "dev"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("multipart/form-data"))
        .stderr(predicate::str::contains("Admin Portal"))
        .stderr(predicate::str::contains("panic").not())
        .stderr(predicate::str::contains("Gathering").not());
}

/// `--dry-run` is also blocked for a file-upload command — the gather/resolve
/// phase that previously panicked runs before the dry-run branch, so previewing
/// is not possible; the guard fires first with the same clean error.
#[test]
fn test_file_upload_command_rejected_in_dry_run() {
    ags_isolated()
        .args([
            "csm",
            "app-ui",
            "upload-assets",
            "--namespace",
            "dev",
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not yet support"))
        .stderr(predicate::str::contains("Admin Portal"))
        .stderr(predicate::str::contains("panic").not());
}
