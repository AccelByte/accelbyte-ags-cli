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

#[test]
fn test_full_command_tree_expands_registered_workflows_under_run() {
    // Shell completion must offer each registered workflow id as a
    // completable token after `ags workflow run`, and each workflow's own
    // `--<input>` flags under it. The completion tree therefore expands the
    // `run` subcommand into one subcommand per registered workflow.
    let root = builder::build_full_command();

    let workflow = root
        .get_subcommands()
        .find(|c| c.get_name() == "workflow")
        .expect("workflow subcommand");
    let run = workflow
        .get_subcommands()
        .find(|c| c.get_name() == "run")
        .expect("run subcommand");

    let run_subcommands: Vec<String> = run
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect();

    for id in [
        "competitive-multiplayer",
        "player-overview",
        "in-game-store",
        "season-pass",
    ] {
        assert!(
            run_subcommands.iter().any(|n| n == id),
            "expected '{id}' under `workflow run`, got {run_subcommands:?}"
        );
    }

    // The expanded subcommand carries the workflow's own `--<input>` flags.
    let season_pass = run
        .get_subcommands()
        .find(|c| c.get_name() == "season-pass")
        .expect("season-pass subcommand");
    assert!(
        season_pass
            .get_arguments()
            .any(|arg| arg.get_long().is_some()),
        "season-pass should carry its own --<input> flags"
    );
}
