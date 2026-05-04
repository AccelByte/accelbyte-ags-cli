use ags::runtime::config::ProfileConfig;

// ── Config file permissions (Unix only) ──

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn test_config_dir_has_0700_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("AGS_HOME", tmp.path().to_str().unwrap());

    let config = ProfileConfig {
        base_url: Some("https://example.com".to_string()),
        client_id: Some("test".to_string()),
        ..Default::default()
    };
    config.save("default").unwrap();

    let profile_dir = tmp.path().join("profiles").join("default");
    let dir_meta = std::fs::metadata(&profile_dir).unwrap();
    let mode = dir_meta.permissions().mode() & 0o777;

    std::env::remove_var("AGS_HOME");

    assert_eq!(
        mode, 0o700,
        "Profile directory should have 0700 permissions, got {mode:o}"
    );
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn test_config_file_has_0600_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("AGS_HOME", tmp.path().to_str().unwrap());

    let config = ProfileConfig {
        base_url: Some("https://example.com".to_string()),
        client_id: Some("test".to_string()),
        ..Default::default()
    };
    config.save("default").unwrap();

    let file_path = tmp
        .path()
        .join("profiles")
        .join("default")
        .join("config.json");
    let file_meta = std::fs::metadata(&file_path).unwrap();
    let mode = file_meta.permissions().mode() & 0o777;

    std::env::remove_var("AGS_HOME");

    assert_eq!(
        mode, 0o600,
        "Config file should have 0600 permissions, got {mode:o}"
    );
}
