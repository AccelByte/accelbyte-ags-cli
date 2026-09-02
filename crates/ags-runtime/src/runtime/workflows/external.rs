//! Loading of user-supplied workflow YAML files from disk (`workflows_dir()`),
//! as opposed to `bundled`'s compiled-in YAML. Unlike a bundled
//! file, a malformed or colliding user file must never crash the process —
//! failures are skipped silently, matching the existing best-effort
//! swallow-and-fall-back convention used elsewhere for optional disk state
//! (e.g. `catalogue::repository::try_cache_parsed_schema`).

use std::path::{Path, PathBuf};

use ags_protocol::error::RuntimeError;
use ags_protocol::workflow::{WorkflowDefinition, WorkflowId};

use super::{WorkflowRegistry, YamlWorkflow};
use crate::runtime::config;

/// Parse one external workflow YAML file from disk.
pub(crate) fn load_external_workflow_file(path: &Path) -> Result<WorkflowDefinition, RuntimeError> {
    let yaml = std::fs::read_to_string(path)
        .map_err(|e| config::internal_error(format!("Failed to read '{}': {e}", path.display())))?;
    serde_yaml_ng::from_str(&yaml)
        .map_err(|e| config::internal_error(format!("Failed to parse '{}': {e}", path.display())))
}

/// Scan `workflows_dir()` for `*.yaml`/`*.yml` files and register every one
/// that parses successfully and whose id doesn't already exist in
/// `registry`. Malformed files and id collisions are skipped silently —
/// there is no channel to surface a warning at this point (called from
/// `registry()`'s `OnceLock` init, before any frontend exists), and a stray
/// broken file must never take down the whole CLI.
pub(crate) fn load_external_workflows(registry: &mut WorkflowRegistry) {
    let Ok(dir) = config::workflows_dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_yaml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"))
            .unwrap_or(false);
        if !is_yaml {
            continue;
        }
        let Ok(definition) = load_external_workflow_file(&path) else {
            continue;
        };
        // A missing `workflow_protocol_version` is treated as "authored
        // before versioning existed" rather than grounds to unregister —
        // `ags workflow add` has only required the field since it shipped,
        // so every workflow installed before then is legacy, not broken.
        // The file still registers; `ags workflow run` surfaces this case
        // via its protocol-version warning, same as a declared-but-mismatched
        // version.
        if registry.resolve(&definition.id).is_some() {
            continue;
        }
        // The single insertion point for a user-installed workflow: always
        // `register_external`, never `register`, so its origin can never be
        // mistaken for bundled (see `WorkflowOrigin`).
        registry.register_external(Box::new(YamlWorkflow::new(definition)));
    }
}

/// Scan `workflows_dir()` for the `*.yaml`/`*.yml` file whose parsed `id:`
/// field matches `id` (case-insensitively — Windows/macOS filesystems are
/// case-insensitive, matching the collision check in `facade::workflow::
/// workflow_add`). Matches by parsed content, not filename, since a
/// hand-copied file's filename need not match its `id:` field (the same
/// reason `load_external_workflows` above matches by content). Used by
/// `ags workflow remove <id>` to locate the file to delete; returns `Ok(None)`
/// both when `workflows_dir()` doesn't exist and when no file matches —
/// callers distinguish "no external file" from "unreadable directory" no
/// differently, mirroring `load_external_workflows`'s own best-effort stance.
pub(crate) fn find_external_workflow_by_id(
    id: &WorkflowId,
) -> Result<Option<PathBuf>, RuntimeError> {
    let dir = config::workflows_dir()?;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(None);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_yaml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"))
            .unwrap_or(false);
        if !is_yaml {
            continue;
        }
        let Ok(definition) = load_external_workflow_file(&path) else {
            continue;
        };
        if definition.id.as_str().eq_ignore_ascii_case(id.as_str()) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::workflow::WorkflowId;
    use std::io::Write;

    struct HomeEnvGuard;
    impl Drop for HomeEnvGuard {
        fn drop(&mut self) {
            std::env::remove_var(crate::runtime::config::ENV_HOME);
        }
    }

    const VALID_YAML: &str = r#"
id: external-probe
name: External probe
workflow_protocol_version: "0.1.0"
steps:
  - id: only-step
    description: probe
    operation: {service: iam, operation: iam/admin/users/v3/get}
    inputs:
      - {field: namespace, source: {from: "workflow/namespace"}}
inputs:
  - name: namespace
    description: probe
    required: true
    schema: {type: string}
"#;

    #[test]
    fn test_load_external_workflow_file_parses_valid_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("probe.yaml");
        std::fs::write(&path, VALID_YAML).unwrap();
        let def = load_external_workflow_file(&path).unwrap();
        assert_eq!(def.id, WorkflowId::new("external-probe"));
    }

    #[test]
    fn test_load_external_workflow_file_errors_on_malformed_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.yaml");
        std::fs::write(&path, "id: [this is not valid").unwrap();
        assert!(load_external_workflow_file(&path).is_err());
    }

    #[test]
    #[serial_test::serial]
    fn test_load_external_workflows_registers_valid_file() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, home.path());
        let workflows_dir = home.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        let mut f = std::fs::File::create(workflows_dir.join("probe.yaml")).unwrap();
        f.write_all(VALID_YAML.as_bytes()).unwrap();

        let mut registry = WorkflowRegistry::new();
        load_external_workflows(&mut registry);
        assert!(registry
            .resolve(&WorkflowId::new("external-probe"))
            .is_some());
    }

    #[test]
    #[serial_test::serial]
    /// A workflow loaded from disk is registered with `WorkflowOrigin::External`,
    /// never `Bundled` — this is what gates it to `value: None` telemetry.
    fn test_load_external_workflows_registers_with_external_origin() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, home.path());
        let workflows_dir = home.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::write(workflows_dir.join("probe.yaml"), VALID_YAML).unwrap();

        let mut registry = WorkflowRegistry::new();
        load_external_workflows(&mut registry);
        let id = WorkflowId::new("external-probe");
        assert!(!registry.is_bundled(&id));
        assert_eq!(
            registry.origin(&id),
            Some(crate::runtime::workflows::WorkflowOrigin::External)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_load_external_workflows_skips_malformed_file_without_panic() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, home.path());
        let workflows_dir = home.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::write(workflows_dir.join("broken.yaml"), "not: [valid").unwrap();

        let mut registry = WorkflowRegistry::new();
        load_external_workflows(&mut registry); // must not panic
        assert!(registry.ids().next().is_none());
    }

    #[test]
    #[serial_test::serial]
    fn test_load_external_workflows_registers_file_missing_workflow_protocol_version() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, home.path());
        let workflows_dir = home.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::write(
            workflows_dir.join("no-version.yaml"),
            r#"
id: external-no-version-probe
name: External no-version probe
steps:
  - id: only-step
    description: probe
    operation: {service: iam, operation: iam/admin/users/v3/get}
    inputs:
      - {field: namespace, source: {from: "workflow/namespace"}}
inputs:
  - name: namespace
    description: probe
    required: true
    schema: {type: string}
"#,
        )
        .unwrap();

        let mut registry = WorkflowRegistry::new();
        load_external_workflows(&mut registry); // must not panic
        assert!(
            registry
                .resolve(&WorkflowId::new("external-no-version-probe"))
                .is_some(),
            "a file missing workflow_protocol_version must register as legacy, not disappear"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_load_external_workflows_skips_id_collision() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, home.path());
        let workflows_dir = home.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::write(workflows_dir.join("dup.yaml"), VALID_YAML).unwrap();

        let mut registry = WorkflowRegistry::new();
        // Pre-populate the id so the loader must skip the file, not overwrite.
        let existing = load_external_workflow_file(&workflows_dir.join("dup.yaml")).unwrap();
        registry.register(Box::new(YamlWorkflow::new(existing)));
        load_external_workflows(&mut registry);
        // Still exactly one entry — the file was skipped, not double-registered.
        assert_eq!(registry.ids().count(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn test_load_external_workflows_ignores_non_yaml_files() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, home.path());
        let workflows_dir = home.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::write(workflows_dir.join("readme.txt"), "not a workflow").unwrap();

        let mut registry = WorkflowRegistry::new();
        load_external_workflows(&mut registry); // must not attempt to parse the .txt file
        assert!(registry.ids().next().is_none());
    }

    #[test]
    #[serial_test::serial]
    fn test_load_external_workflows_missing_dir_returns_silently() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, home.path());
        // Deliberately do not create the `workflows` subdirectory.

        let mut registry = WorkflowRegistry::new();
        load_external_workflows(&mut registry); // must not panic
        assert!(registry.ids().next().is_none());
    }

    #[test]
    #[serial_test::serial]
    fn test_find_external_workflow_by_id_matches_by_content_not_filename() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, home.path());
        let workflows_dir = home.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        // Filename deliberately does not match the `id:` field.
        std::fs::write(workflows_dir.join("unrelated-name.yaml"), VALID_YAML).unwrap();

        let found = find_external_workflow_by_id(&WorkflowId::new("external-probe"))
            .unwrap()
            .expect("must find the file by its id field");
        assert_eq!(found, workflows_dir.join("unrelated-name.yaml"));
    }

    #[test]
    #[serial_test::serial]
    fn test_find_external_workflow_by_id_matches_case_insensitively() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, home.path());
        let workflows_dir = home.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::write(workflows_dir.join("probe.yaml"), VALID_YAML).unwrap();

        let found = find_external_workflow_by_id(&WorkflowId::new("External-Probe")).unwrap();
        assert!(found.is_some(), "id match must be case-insensitive");
    }

    #[test]
    #[serial_test::serial]
    fn test_find_external_workflow_by_id_returns_none_when_absent() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, home.path());
        let workflows_dir = home.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::write(workflows_dir.join("probe.yaml"), VALID_YAML).unwrap();

        let found = find_external_workflow_by_id(&WorkflowId::new("does-not-exist")).unwrap();
        assert!(found.is_none());
    }

    #[test]
    #[serial_test::serial]
    fn test_find_external_workflow_by_id_missing_dir_returns_none() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, home.path());
        // Deliberately do not create the `workflows` subdirectory.

        let found = find_external_workflow_by_id(&WorkflowId::new("external-probe")).unwrap();
        assert!(found.is_none());
    }

    #[test]
    #[serial_test::serial]
    fn test_find_external_workflow_by_id_skips_malformed_file_without_panic() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, home.path());
        let workflows_dir = home.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::write(workflows_dir.join("broken.yaml"), "not: [valid").unwrap();

        let found = find_external_workflow_by_id(&WorkflowId::new("external-probe")).unwrap(); // must not panic
        assert!(found.is_none());
    }
}
