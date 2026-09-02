//! Bundled built-in workflows authored as YAML instead of Rust literals.
//! Spike: proves `include_str!` -> `serde_yaml_ng::from_str` ->
//! `register_builtins` -> `compile_workflow` end-to-end for one workflow,
//! alongside its Rust-literal original. Sibling to `builtins/`, not nested
//! under it — like `external`, this loads/registers workflows rather than
//! being one itself, so it doesn't fit `builtins/`'s one-struct-per-workflow
//! convention.

#[cfg(test)]
use ags_protocol::error::RuntimeError;
use ags_protocol::workflow::WorkflowDefinition;

#[cfg(test)]
use crate::runtime::config;
use crate::runtime::workflows::{WorkflowRegistry, YamlWorkflow};

/// Bundled built-in workflows authored as YAML. One entry today — the
/// near-term spike proving the pipeline — not a general mechanism yet. No
/// separate id key: each YAML's own `id:` field is the only source of
/// truth, since that's also what `WorkflowRegistry::register` keys on.
pub(crate) static BUNDLED_YAML_WORKFLOWS: &[&str] = &[
    include_str!("../../../workflows/competitive-multiplayer-yaml-poc.yaml"),
    include_str!("../../../workflows/docker-login.yaml"),
];

/// Parse every bundled YAML workflow and return the one whose `id:` field
/// matches. A parse failure on *any* entry surfaces immediately rather than
/// being skipped — every entry here is a compile-time-bundled, developer-
/// controlled file, so a malformed one is a bug regardless of which id was
/// being searched for.
///
/// Test-only: `register_bundled_yaml_workflows` parses entries directly
/// instead of looking them up by id, so this exists solely as a by-id lookup
/// helper for tests.
#[cfg(test)]
pub(crate) fn load_bundled_yaml_workflow(id: &str) -> Result<WorkflowDefinition, RuntimeError> {
    for yaml in BUNDLED_YAML_WORKFLOWS {
        let definition: WorkflowDefinition = serde_yaml_ng::from_str(yaml).map_err(|e| {
            config::internal_error(format!("Failed to parse bundled workflow YAML: {e}"))
        })?;
        if definition.id.as_str() == id {
            return Ok(definition);
        }
    }
    Err(config::internal_error(format!(
        "No bundled YAML workflow for id '{id}'"
    )))
}

/// Parse and register every entry in `BUNDLED_YAML_WORKFLOWS`. Panics on a
/// parse failure — acceptable for a single bundled, developer-controlled
/// file; graceful surfacing (`LoadedWorkflows`/`invalid`) is out of scope
/// for this spike and only matters once workflows come from user-supplied
/// disk files.
pub(crate) fn register_bundled_yaml_workflows(registry: &mut WorkflowRegistry) {
    for yaml in BUNDLED_YAML_WORKFLOWS {
        let definition: WorkflowDefinition = serde_yaml_ng::from_str(yaml)
            .unwrap_or_else(|e| panic!("bundled workflow failed to parse: {e}"));
        registry.register(Box::new(YamlWorkflow::new(definition)));
    }
}

#[cfg(test)]
mod tests {
    use ags_protocol::workflow::WorkflowDefinition;

    const MINIMAL_YAML: &str = r#"
id: parse-probe
name: Parse probe
steps:
  - id: only-step
    description: Exercises every binding wire form.
    operation: {service: iam, operation: iam/admin/users/v3/get}
    inputs:
      - {field: namespace, source: {from: "workflow/namespace"}}
      - {field: setBy, source: {const: "SERVER"}}
      - {field: name, source: {format: "{resourcePrefix}-thing"}}
      - field: "data.matching_rule[0].reference"
        source: {const: 200}
inputs:
  - name: namespace
    description: probe
    required: true
    schema: {type: string}
  - name: resourcePrefix
    description: probe
    required: false
    default: probe
    schema: {type: string}
"#;

    #[test]
    fn test_serde_yaml_ng_parses_every_binding_wire_form() {
        let def: WorkflowDefinition = serde_yaml_ng::from_str(MINIMAL_YAML).expect("must parse");
        assert_eq!(def.id.as_str(), "parse-probe");
        assert_eq!(def.steps.len(), 1);
        assert_eq!(def.steps[0].inputs.len(), 4);
    }

    use super::{load_bundled_yaml_workflow, BUNDLED_YAML_WORKFLOWS};
    use ags_protocol::workflow::WorkflowId;

    /// The bundled YAML table must contain at least the two known entries
    /// and each must be a member. A floor plus membership check (not an
    /// exact count) so a new workflow does not break this test.
    #[test]
    fn test_bundled_yaml_workflows_table_contains_known_entries() {
        assert!(
            BUNDLED_YAML_WORKFLOWS.len() >= 2,
            "expected at least 2 bundled YAML workflows, got {}",
            BUNDLED_YAML_WORKFLOWS.len()
        );
        // Each known workflow must appear by parsing its id.
        let ids: Vec<String> = BUNDLED_YAML_WORKFLOWS
            .iter()
            .map(|yaml| {
                let def: WorkflowDefinition =
                    serde_yaml_ng::from_str(yaml).expect("bundled YAML must parse");
                def.id.as_str().to_string()
            })
            .collect();
        assert!(
            ids.contains(&"competitive-multiplayer-yaml-poc".to_string()),
            "competitive-multiplayer-yaml-poc must be bundled; got {ids:?}"
        );
        assert!(
            ids.contains(&"docker-login".to_string()),
            "docker-login must be bundled; got {ids:?}"
        );
    }

    #[test]
    fn test_load_bundled_yaml_workflow_parses_competitive_multiplayer_poc() {
        let def = load_bundled_yaml_workflow("competitive-multiplayer-yaml-poc")
            .expect("bundled YAML must parse");
        assert_eq!(def.id, WorkflowId::new("competitive-multiplayer-yaml-poc"));
        assert_eq!(def.steps.len(), 7);
        assert_eq!(def.inputs.len(), 10);
        assert!(def.completion.is_some());
    }

    #[test]
    fn test_load_bundled_yaml_workflow_parses_docker_login() {
        let def = load_bundled_yaml_workflow("docker-login").expect("bundled YAML must parse");
        assert_eq!(def.id, WorkflowId::new("docker-login"));
        assert_eq!(def.steps.len(), 2);
        assert_eq!(def.inputs.len(), 2);
    }

    #[test]
    fn test_bundled_docker_login_workflow_is_registered() {
        let resolved = registry().resolve(&WorkflowId::new("docker-login"));
        assert!(
            resolved.is_some(),
            "docker-login workflow must be in the process-wide registry"
        );
        assert_eq!(resolved.unwrap().definition().steps.len(), 2);
    }

    #[test]
    fn test_bundled_docker_login_workflow_compiles_against_bundled_catalogue() {
        let definition = load_bundled_yaml_workflow("docker-login").unwrap();
        let mut catalogue = Catalogue::new();
        let compiled = compile_workflow(&definition, &mut catalogue)
            .expect("docker-login YAML must compile against the bundled catalogue");
        assert_eq!(compiled.steps.len(), 2);
    }

    #[test]
    fn test_bundled_docker_login_workflow_no_required_field_left_unbound() {
        let definition = load_bundled_yaml_workflow("docker-login").unwrap();
        let mut catalogue = Catalogue::new();
        let compiled = compile_workflow(&definition, &mut catalogue).unwrap();
        for step in &compiled.steps {
            let unbound: Vec<&str> = step
                .auto_derived
                .iter()
                .filter(|f| f.required)
                .map(|f| f.field.as_str())
                .collect();
            assert!(
                unbound.is_empty(),
                "step '{}' has unbound required fields: {unbound:?}",
                step.id
            );
        }
    }

    #[test]
    fn test_load_bundled_yaml_workflow_unknown_id_errors() {
        let err = load_bundled_yaml_workflow("does-not-exist").unwrap_err();
        assert!(err.to_string().contains("does-not-exist"));
    }

    use crate::runtime::workflows::registry;

    #[test]
    fn test_bundled_yaml_workflow_is_registered() {
        let resolved = registry().resolve(&WorkflowId::new("competitive-multiplayer-yaml-poc"));
        assert!(
            resolved.is_some(),
            "yaml-poc workflow must be in the process-wide registry"
        );
        assert_eq!(
            resolved.unwrap().definition().steps.len(),
            7,
            "must have the same 7 steps as the Rust original"
        );
    }

    use crate::catalogue::Catalogue;
    use crate::runtime::workflows::compile::compile_workflow;

    #[test]
    fn test_bundled_yaml_workflow_compiles_against_bundled_catalogue() {
        let definition = load_bundled_yaml_workflow("competitive-multiplayer-yaml-poc").unwrap();
        let mut catalogue = Catalogue::new();
        let compiled = compile_workflow(&definition, &mut catalogue)
            .expect("bundled YAML workflow must compile against the bundled catalogue");
        assert_eq!(compiled.steps.len(), 7);
    }

    #[test]
    fn test_bundled_yaml_workflow_no_required_field_left_unbound() {
        let definition = load_bundled_yaml_workflow("competitive-multiplayer-yaml-poc").unwrap();
        let mut catalogue = Catalogue::new();
        let compiled = compile_workflow(&definition, &mut catalogue).unwrap();
        for step in &compiled.steps {
            let unbound: Vec<&str> = step
                .auto_derived
                .iter()
                .filter(|f| f.required)
                .map(|f| f.field.as_str())
                .collect();
            assert!(
                unbound.is_empty(),
                "step '{}' has unbound required fields: {unbound:?}",
                step.id
            );
        }
    }
}
