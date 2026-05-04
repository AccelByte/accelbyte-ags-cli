use std::path::PathBuf;

/// Return the path to a test fixture file.
pub fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from("tests/fixtures").join(relative)
}
