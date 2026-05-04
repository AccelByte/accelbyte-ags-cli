use ags::catalogue::Catalogue;
use std::collections::HashSet;

/// The full spec-loading pipeline (include_bytes + gzip + parse) must produce core IAM resources, catching regressions in bundling or parsing
#[test]
fn test_parse_bundled_iam_spec() {
    let service = Catalogue::load_bundled("iam").expect("Failed to load IAM spec");

    assert_eq!(service.name, "iam");
    assert!(!service.resources.is_empty());

    let resource_names: HashSet<&str> = service.resources.iter().map(|r| r.name.as_str()).collect();

    for expected in ["users", "roles", "bans", "oauth2"] {
        assert!(
            resource_names.contains(expected),
            "Missing '{expected}' resource"
        );
    }
}
