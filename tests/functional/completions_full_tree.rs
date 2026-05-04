//! The tree returned by `build_full_command` must contain every registered
//! service as a populated subcommand with its resources.

use ags::invocation::builder;

#[test]
fn test_full_command_tree_has_populated_services() {
    let root = builder::build_full_command();

    let service_names: Vec<String> = root
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect();

    assert!(
        service_names.iter().any(|n| n == "iam"),
        "expected iam in subcommands: {service_names:?}"
    );

    let iam = root
        .get_subcommands()
        .find(|c| c.get_name() == "iam")
        .expect("iam service");
    let resource_count = iam.get_subcommands().count();
    assert!(
        resource_count > 0,
        "iam should have populated resources, got {resource_count}"
    );
}
