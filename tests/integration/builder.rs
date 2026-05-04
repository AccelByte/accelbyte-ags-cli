use ags::catalogue::Catalogue;
use ags::invocation::builder::build_service_command_tree;
use ags::invocation::flags::LeafSelectors;
use ags::protocol::catalogue::ServiceSchema;

fn parsed_iam() -> ServiceSchema {
    Catalogue::load_bundled("iam").expect("load IAM spec")
}

/// Simulate the cache round-trip: serialize → deserialize the schema.
fn cached_iam() -> ServiceSchema {
    let fresh = parsed_iam();
    let json = serde_json::to_string(&fresh).expect("serialize ServiceSchema");
    serde_json::from_str(&json).expect("deserialize ServiceSchema")
}

/// Users must be able to invoke `iam users list` so the dynamic command tree correctly maps OpenAPI paths to a resource/method hierarchy
#[test]
fn test_clap_tree_matches_users_list() {
    let service = parsed_iam();
    let service_command_tree = build_service_command_tree(&service, &LeafSelectors::default());

    let matches = service_command_tree
        .try_get_matches_from([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test-ns",
        ])
        .expect("should match 'iam users list --namespace test-ns'");

    let (resource, resource_matches) = matches.subcommand().expect("should have resource");
    assert_eq!(resource, "users");

    let (method, _) = resource_matches.subcommand().expect("should have method");
    assert_eq!(method, "list-users-with-accelbyte-account");
}

/// Operations with required path parameters must accept those params as flags so users can target specific resources
#[test]
fn test_clap_tree_matches_roles_get_with_param() {
    let service = parsed_iam();
    let service_command_tree = build_service_command_tree(&service, &LeafSelectors::default());

    let result =
        service_command_tree.try_get_matches_from(["iam", "roles", "get", "--role-id", "role-123"]);
    assert!(
        result.is_ok(),
        "should match 'iam roles get --role-id role-123': {:?}",
        result.err()
    );
}

/// Path parameter flags must use kebab-case so they follow standard CLI conventions
#[test]
fn test_clap_tree_path_param_is_kebab_case() {
    let service = parsed_iam();
    let service_command_tree = build_service_command_tree(&service, &LeafSelectors::default());

    let result = service_command_tree.try_get_matches_from([
        "iam",
        "users",
        "get",
        "--namespace",
        "test-ns",
        "--user-id",
        "abc123",
    ]);
    assert!(
        result.is_ok(),
        "should accept --user-id (kebab-case), got: {:?}",
        result.err()
    );
}

/// The old camelCase flag form must be rejected to prevent silent regressions where both forms co-exist
#[test]
fn test_clap_tree_camel_case_param_rejected() {
    let service = parsed_iam();
    let service_command_tree = build_service_command_tree(&service, &LeafSelectors::default());

    let result = service_command_tree.try_get_matches_from([
        "iam",
        "users",
        "get",
        "--namespace",
        "test-ns",
        "--userId",
        "abc123",
    ]);
    assert!(
        result.is_err(),
        "old camelCase flag --userId must be rejected; kebab-case --user-id is required"
    );
}

/// Typos and invalid resources must be rejected so users get Clap's "did you mean?" help instead of silent failures
#[test]
fn test_clap_tree_rejects_unknown_resource() {
    let service = parsed_iam();
    let service_command_tree = build_service_command_tree(&service, &LeafSelectors::default());

    let result = service_command_tree.try_get_matches_from(["iam", "nonexistent", "list"]);
    assert!(result.is_err(), "unknown resource should not match");
}

/// Invalid methods on valid resources must be rejected so users discover available operations via help
#[test]
fn test_clap_tree_rejects_unknown_method() {
    let service = parsed_iam();
    let service_command_tree = build_service_command_tree(&service, &LeafSelectors::default());

    let result = service_command_tree.try_get_matches_from(["iam", "users", "nonexistent"]);
    assert!(result.is_err(), "unknown method should not match");
}

/// Serialization round-trips must preserve the command tree exactly, ensuring cached specs behave identically to freshly parsed ones
#[test]
fn test_cached_clap_tree_matches_fresh() {
    let fresh = parsed_iam();
    let cached = cached_iam();

    let fresh_command_tree = build_service_command_tree(&fresh, &LeafSelectors::default());
    let cached_command_tree = build_service_command_tree(&cached, &LeafSelectors::default());

    // Compare all resource subcommand names
    let fresh_resources: Vec<&str> = fresh_command_tree
        .get_subcommands()
        .map(|command| command.get_name())
        .collect();
    let cached_resources: Vec<&str> = cached_command_tree
        .get_subcommands()
        .map(|command| command.get_name())
        .collect();
    assert_eq!(
        fresh_resources, cached_resources,
        "Resource names should match"
    );

    // Spot-check: users list should parse identically from both trees
    let fresh_match = fresh_command_tree
        .try_get_matches_from([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test-ns",
        ])
        .expect("fresh tree should match");
    let cached_match = cached_command_tree
        .try_get_matches_from([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test-ns",
        ])
        .expect("cached tree should match");

    let (fr, _) = fresh_match.subcommand().unwrap();
    let (cr, _) = cached_match.subcommand().unwrap();
    assert_eq!(fr, cr, "Resource name should match");
}

/// Every resource in the spec must appear as a subcommand so no API surface is silently dropped during tree construction
#[test]
fn test_clap_tree_has_all_resources() {
    let service = parsed_iam();
    let resource_names: Vec<String> = service
        .resources
        .iter()
        .map(|resource| resource.name.clone())
        .collect();
    let service_command_tree = build_service_command_tree(&service, &LeafSelectors::default());

    for name in &resource_names {
        let result = service_command_tree
            .clone()
            .try_get_matches_from(["iam", name, "--help"]);
        // --help causes an error exit but it's a DisplayHelp error, not InvalidSubcommand
        match result {
            Ok(_) => {}
            Err(e) => {
                assert_eq!(
                    e.kind(),
                    clap::error::ErrorKind::DisplayHelp,
                    "Resource '{name}' should be a valid subcommand, got: {e}"
                );
            }
        }
    }
}
