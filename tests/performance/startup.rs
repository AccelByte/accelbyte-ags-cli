use assert_cmd::Command;

#[test]
#[ignore]
fn test_version_startup_under_2s() {
    let start = std::time::Instant::now();
    let output = Command::cargo_bin("ags")
        .unwrap()
        .arg("--version")
        .output()
        .unwrap();
    let elapsed = start.elapsed();

    assert!(output.status.success());
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "ags --version took {elapsed:?}, expected < 2s"
    );
}

#[test]
#[ignore]
fn test_help_startup_under_1s() {
    let start = std::time::Instant::now();
    let output = Command::cargo_bin("ags")
        .unwrap()
        .arg("--help")
        .output()
        .unwrap();
    let elapsed = start.elapsed();

    assert!(output.status.success());
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "ags --help took {elapsed:?}, expected < 1s"
    );
}
