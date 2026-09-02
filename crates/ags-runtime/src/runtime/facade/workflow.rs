//! Facade for `ags workflow add`/`ags workflow template` — pure filesystem
//! operations, no HTTP/auth involved. Constructed as `impl Runtime` methods
//! (even though `self` goes unused) to mirror `facade/profile.rs`'s CRUD
//! shape, which does the same for its own filesystem-only operations.

use std::path::Path;

use ags_protocol::error::{RuntimeError, RuntimeErrorKind};
use ags_protocol::output_views::{
    BinaryWrittenDestination, WorkflowAddOutput, WorkflowRemoveOutput, WorkflowTemplateOutput,
};
use ags_protocol::workflow::{WorkflowDefinition, WorkflowId};

use crate::catalogue::Catalogue;
use crate::runtime::config;
use crate::runtime::workflows::compile::compile_workflow;
use crate::runtime::workflows::{find_external_workflow_file, registry};

impl crate::runtime::Runtime {
    /// Validate a workflow YAML file and, unless `validate_only`, install it
    /// into `workflows_dir()` under `<id>.yaml`.
    pub fn workflow_add(
        &self,
        path: &Path,
        validate_only: bool,
    ) -> Result<WorkflowAddOutput, RuntimeError> {
        let yaml = std::fs::read_to_string(path).map_err(|e| RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message: format!("Cannot read '{}': {e}", path.display()),
            details: None,
            hint: None,
            trace: None,
        })?;
        let definition: WorkflowDefinition =
            serde_yaml_ng::from_str(&yaml).map_err(|e| RuntimeError {
                kind: RuntimeErrorKind::Validation,
                message: format!("'{}' is not a valid workflow YAML: {e}", path.display()),
                details: None,
                hint: None,
                trace: None,
            })?;

        match definition.workflow_protocol_version.as_deref() {
            None => {
                return Err(RuntimeError {
                    kind: RuntimeErrorKind::Validation,
                    message: format!(
                        "'{}' is missing a required `workflow_protocol_version` field.",
                        path.display()
                    ),
                    details: None,
                    hint: Some(
                        "Declare `workflow_protocol_version: \"<version>\"`, or run `ags \
                         workflow template` for a starter file with one already filled in."
                            .to_string(),
                    ),
                    trace: None,
                });
            }
            Some(declared) if semver::Version::parse(declared).is_err() => {
                return Err(RuntimeError {
                    kind: RuntimeErrorKind::Validation,
                    message: format!(
                        "'{}' has an invalid `workflow_protocol_version`: '{declared}' is not a \
                         valid semver version.",
                        path.display()
                    ),
                    details: None,
                    hint: Some(
                        "Declare `workflow_protocol_version` as a semver string (e.g. \"1.0.0\"), \
                         or run `ags workflow template` for a starter file with one already \
                         filled in."
                            .to_string(),
                    ),
                    trace: None,
                });
            }
            Some(_) => {}
        }

        validate_workflow_id(definition.id.as_str())?;

        let mut catalogue = Catalogue::new();
        compile_workflow(&definition, &mut catalogue)?;

        // Case-insensitive, not just exact-match: on case-insensitive
        // filesystems (Windows/macOS default) `Foo.yaml` and `foo.yaml` are
        // the same file, so installing `Foo` after `foo` already exists
        // would otherwise silently overwrite it.
        if let Some(existing) = registry().ids().find(|existing| {
            existing
                .as_str()
                .eq_ignore_ascii_case(definition.id.as_str())
        }) {
            let message = if existing.as_str() == definition.id.as_str() {
                format!(
                    "A workflow with id '{}' is already registered.",
                    definition.id.as_str()
                )
            } else {
                format!(
                    "A workflow with id '{}' is already registered as '{}'; ids are compared case-insensitively.",
                    definition.id.as_str(),
                    existing.as_str()
                )
            };
            return Err(RuntimeError {
                kind: RuntimeErrorKind::Validation,
                message,
                details: None,
                hint: Some(
                    "Choose a different id, or run `ags workflow remove <id>` to remove the \
                     existing workflow first (built-in workflows cannot be removed this way)."
                        .to_string(),
                ),
                trace: None,
            });
        }

        let installed_path = if validate_only {
            None
        } else {
            let dir = config::workflows_dir()?;
            crate::support::file_system::create_dir_restricted(&dir).map_err(|e| {
                config::internal_error(format!("Cannot create workflows directory: {e}"))
            })?;
            let target = dir.join(format!("{}.yaml", definition.id.as_str()));
            // Defense-in-depth: `validate_workflow_id` above should already make
            // this unreachable, but a future regression there (or a change to
            // the join logic) must not silently escape `workflows_dir()`.
            if target.parent() != Some(dir.as_path()) {
                return Err(config::internal_error(format!(
                    "Refusing to write workflow file outside the workflows directory: '{}'",
                    target.display()
                )));
            }
            crate::support::file_system::write_file_restricted(&target, &yaml).map_err(|e| {
                config::internal_error(format!("Failed to write '{}': {e}", target.display()))
            })?;
            Some(target)
        };

        Ok(WorkflowAddOutput {
            id: definition.id,
            validated_only: validate_only,
            path: installed_path,
        })
    }

    /// Emit a starter workflow YAML skeleton (unvalidated), optionally
    /// writing it to `output` instead of returning it for stdout.
    pub fn workflow_template(
        &self,
        output: Option<&Path>,
    ) -> Result<WorkflowTemplateOutput, RuntimeError> {
        let yaml = WORKFLOW_TEMPLATE_SKELETON.replacen(
            "{{WORKFLOW_PROTOCOL_VERSION}}",
            ags_protocol::workflow::WORKFLOW_PROTOCOL_VERSION,
            1,
        );
        let destination = match output {
            None => BinaryWrittenDestination::Stdout,
            Some(path) => {
                crate::support::file_system::write_file_restricted(path, &yaml).map_err(|e| {
                    config::internal_error(format!("Failed to write '{}': {e}", path.display()))
                })?;
                BinaryWrittenDestination::File(path.to_path_buf())
            }
        };
        Ok(WorkflowTemplateOutput { yaml, destination })
    }

    /// Remove a previously-installed external workflow YAML file from
    /// `workflows_dir()`. Rejects an id that belongs only to a built-in
    /// workflow (Rust or bundled YAML) with a clear error rather than
    /// silently no-op-ing.
    pub fn workflow_remove(&self, id: &str) -> Result<WorkflowRemoveOutput, RuntimeError> {
        let workflow_id = WorkflowId::new(id);
        match find_external_workflow_file(&workflow_id)? {
            Some(path) => {
                std::fs::remove_file(&path).map_err(|e| {
                    config::internal_error(format!("Failed to remove '{}': {e}", path.display()))
                })?;
                // Deliberately check `registry()` only AFTER deleting the file: this
                // is the first access of the process-wide `OnceLock` in this
                // codepath (no runtime prologue precedes `workflow remove`, same as
                // `workflow_add`/`workflow_template`), so its one-time external-file
                // scan reflects the post-deletion directory state. A hit here can
                // therefore only mean a genuine built-in shares this id — the
                // just-deleted external file was already shadowed by it and never
                // itself reached the registry (see `external.rs`'s
                // first-registered-wins skip). Do not reorder this after the
                // deletion below, or a plain (non-colliding) external removal could
                // be misreported as "still a built-in".
                let builtin_still_registered = registry().resolve(&workflow_id).is_some();
                Ok(WorkflowRemoveOutput {
                    id: workflow_id,
                    path,
                    builtin_still_registered,
                })
            }
            None => {
                let is_builtin = registry()
                    .ids()
                    .any(|existing| existing.as_str().eq_ignore_ascii_case(id));
                if is_builtin {
                    Err(RuntimeError {
                        kind: RuntimeErrorKind::Validation,
                        message: format!("'{id}' is a built-in workflow and cannot be removed."),
                        details: None,
                        hint: Some(
                            "Only workflows installed via `ags workflow add` can be removed."
                                .to_string(),
                        ),
                        trace: None,
                    })
                } else {
                    Err(RuntimeError {
                        kind: RuntimeErrorKind::Validation,
                        message: format!("Unknown workflow: '{id}'."),
                        details: None,
                        hint: Some(
                            "Run `ags workflow list` to see available workflows.".to_string(),
                        ),
                        trace: None,
                    })
                }
            }
        }
    }
}

/// Validate a workflow id before it is ever used to derive a filesystem path.
/// `WorkflowId` (`ags-protocol`) is a dumb newtype with no validation of its
/// own by design — crate-layering keeps `ags-protocol` free of behaviour — so
/// this check lives here, at the one call site (`workflow_add`) that turns an
/// author-supplied id into a filename (`<id>.yaml` under `workflows_dir()`).
///
/// Rejects empty ids and anything outside a conservative allowlist
/// (alphanumeric, `.`, `_`, `-`). This blocks path traversal (`../../x`),
/// absolute paths (`C:/...`, `/etc/...`), and embedded separators (`/`, `\`)
/// — every existing built-in id (e.g. `competitive-multiplayer`) already
/// satisfies this allowlist, so it costs nothing for legitimate authors.
///
/// Also rejects Windows-reserved device names, which are reserved regardless
/// of extension — Windows treats the segment before the first `.` as the
/// device name, so an id like `con.foo` would still fail to create on disk
/// as `con.foo.yaml`. Not a security issue (no traversal/overwrite risk),
/// just a confusing dead end this catches early with a clear message.
fn validate_workflow_id(id: &str) -> Result<(), RuntimeError> {
    let allowlisted = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !allowlisted {
        return Err(RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message: format!("Invalid workflow id '{id}'."),
            details: None,
            hint: Some(
                "Workflow ids may only contain letters, digits, '.', '_', and '-' \
                 (e.g. 'competitive-multiplayer')."
                    .to_string(),
            ),
            trace: None,
        });
    }

    let device_name = id.split('.').next().unwrap_or(id);
    if WINDOWS_RESERVED_DEVICE_NAMES
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(device_name))
    {
        return Err(RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message: format!("Invalid workflow id '{id}': reserved by Windows."),
            details: None,
            hint: Some(
                "Workflow ids may not use Windows-reserved device names (CON, PRN, AUX, NUL, \
                 COM1-9, LPT1-9), even as a prefix before '.'."
                    .to_string(),
            ),
            trace: None,
        });
    }

    Ok(())
}

/// Reserved regardless of case or extension on Windows.
const WINDOWS_RESERVED_DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Starter skeleton for `ags workflow template`, mirroring the annotated
/// schema an author needs (placeholder id/name, one step, one input). Lives
/// as a standalone file (not a Rust literal) so it can be edited with real
/// YAML tooling; deliberately not under `workflows/` alongside the bundled
/// workflows, since it isn't itself a registered workflow.
const WORKFLOW_TEMPLATE_SKELETON: &str = include_str!("../../../templates/workflow.yaml");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::execution::ExecutionContext;
    use crate::runtime::Runtime;
    use std::path::PathBuf;

    struct HomeEnvGuard;
    impl Drop for HomeEnvGuard {
        fn drop(&mut self) {
            std::env::remove_var(config::ENV_HOME);
        }
    }

    fn test_runtime() -> Runtime {
        Runtime::from_reqwest(ExecutionContext::default(), reqwest::Client::new())
    }

    const VALID_YAML: &str = r#"
id: facade-add-probe
name: Facade add probe
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
    #[serial_test::serial]
    fn test_workflow_add_writes_file_and_returns_path() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());
        let src = home.path().join("draft.yaml");
        std::fs::write(&src, VALID_YAML).unwrap();

        let output = test_runtime().workflow_add(&src, false).unwrap();
        assert!(!output.validated_only);
        let installed = output
            .path
            .expect("path must be Some when not validate_only");
        assert_eq!(
            installed,
            home.path().join("workflows/facade-add-probe.yaml")
        );
        assert!(installed.exists());
    }

    #[test]
    #[cfg(unix)]
    #[serial_test::serial]
    fn test_workflow_add_creates_workflows_dir_with_0700_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());
        let src = home.path().join("draft.yaml");
        std::fs::write(&src, VALID_YAML).unwrap();

        let output = test_runtime().workflow_add(&src, false).unwrap();
        let dir = output.path.unwrap().parent().unwrap().to_path_buf();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_add_validate_only_does_not_write() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());
        let src = home.path().join("draft.yaml");
        std::fs::write(&src, VALID_YAML).unwrap();

        let output = test_runtime().workflow_add(&src, true).unwrap();
        assert!(output.validated_only);
        assert!(output.path.is_none());
        assert!(!home.path().join("workflows/facade-add-probe.yaml").exists());
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_add_rejects_malformed_yaml() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());
        let src = home.path().join("broken.yaml");
        std::fs::write(&src, "not: [valid").unwrap();

        let err = test_runtime().workflow_add(&src, false).unwrap_err();
        assert_eq!(err.kind, RuntimeErrorKind::Validation);
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_add_rejects_missing_workflow_protocol_version() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());
        let src = home.path().join("no-version.yaml");
        std::fs::write(
            &src,
            r#"
id: facade-no-version-probe
name: No version probe
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

        let err = test_runtime().workflow_add(&src, false).unwrap_err();
        assert_eq!(err.kind, RuntimeErrorKind::Validation);
        assert!(
            err.message.contains("workflow_protocol_version"),
            "got: {}",
            err.message
        );
        assert!(
            !home.path().join("workflows").exists(),
            "must not write anything to disk when workflow_protocol_version is missing"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_add_rejects_unparsable_workflow_protocol_version() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());
        let src = home.path().join("bad-version.yaml");
        std::fs::write(
            &src,
            r#"
id: facade-bad-version-probe
name: Bad version probe
workflow_protocol_version: "banana"
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

        let err = test_runtime().workflow_add(&src, false).unwrap_err();
        assert_eq!(err.kind, RuntimeErrorKind::Validation);
        assert!(
            err.message.contains("not a valid semver version"),
            "got: {}",
            err.message
        );
        assert!(
            !home.path().join("workflows").exists(),
            "must not write anything to disk when workflow_protocol_version is invalid"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_add_rejects_id_collision_with_builtin() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());
        let src = home.path().join("collide.yaml");
        // Same id as the built-in Rust workflow, valid enough to compile.
        std::fs::write(
            &src,
            r#"
id: competitive-multiplayer
name: Colliding workflow
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
"#,
        )
        .unwrap();

        let err = test_runtime().workflow_add(&src, false).unwrap_err();
        assert_eq!(err.kind, RuntimeErrorKind::Validation);
        assert!(err.message.contains("already registered"));
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_add_rejects_id_collision_with_builtin_case_insensitive() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());
        let src = home.path().join("collide-case.yaml");
        // Same id as the built-in Rust workflow, differing only in case.
        std::fs::write(
            &src,
            r#"
id: Competitive-Multiplayer
name: Colliding workflow (case-insensitive)
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
"#,
        )
        .unwrap();

        let err = test_runtime().workflow_add(&src, false).unwrap_err();
        assert_eq!(err.kind, RuntimeErrorKind::Validation);
        assert!(err.message.contains("already registered"));
        assert!(err.message.contains("case-insensitively"));
    }

    /// YAML with a given `id:` value, otherwise identical to `VALID_YAML`.
    fn yaml_with_id(id: &str) -> String {
        format!(
            r#"
id: "{id}"
name: Malicious id probe
workflow_protocol_version: "0.1.0"
steps:
  - id: only-step
    description: probe
    operation: {{service: iam, operation: iam/admin/users/v3/get}}
    inputs:
      - {{field: namespace, source: {{from: "workflow/namespace"}}}}
inputs:
  - name: namespace
    description: probe
    required: true
    schema: {{type: string}}
"#
        )
    }

    /// Asserts `id` is rejected with a `Validation` error before any write,
    /// and that `workflows_dir()` (under the isolated, empty `AGS_HOME`) was
    /// never even created — i.e. the id never reached the filesystem-join
    /// step. Callers add id-specific escape-path checks on top of this.
    fn assert_id_rejected_with_no_write(home: &std::path::Path, id: &str) {
        let src = home.join("draft.yaml");
        std::fs::write(&src, yaml_with_id(id)).unwrap();

        let err = test_runtime().workflow_add(&src, false).unwrap_err();
        assert_eq!(
            err.kind,
            RuntimeErrorKind::Validation,
            "id '{id}' must be rejected as Validation, got: {err:?}"
        );
        assert!(
            !home.join("workflows").exists(),
            "id '{id}' must not cause workflows_dir() to be created"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_add_rejects_id_with_path_traversal() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());

        assert_id_rejected_with_no_write(home.path(), "../../victim/pwned");

        // Confirm the traversal target really wasn't created one level up.
        let escaped = home
            .path()
            .parent()
            .expect("tempdir has a parent")
            .join("victim");
        assert!(
            !escaped.exists(),
            "path-traversal id must not have escaped the isolated AGS_HOME"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_add_rejects_id_with_path_separators() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());

        assert_id_rejected_with_no_write(home.path(), "sub/evil");
        assert_id_rejected_with_no_write(home.path(), "sub\\evil");
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_add_rejects_absolute_path_id() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());

        // Windows-style and POSIX-style absolute paths — both contain
        // characters ('/', ':', or a leading '/') outside the id allowlist.
        let windows_style = "C:/ags-cli-test-no-such-dir/precious";
        let posix_style = "/ags-cli-test-no-such-dir/precious";
        assert_id_rejected_with_no_write(home.path(), windows_style);
        assert_id_rejected_with_no_write(home.path(), posix_style);

        assert!(
            !std::path::Path::new(&format!("{windows_style}.yaml")).exists(),
            "absolute-path id must not have overwritten a file at the literal path"
        );
        assert!(
            !std::path::Path::new(&format!("{posix_style}.yaml")).exists(),
            "absolute-path id must not have overwritten a file at the literal path"
        );
    }

    #[test]
    fn test_validate_workflow_id_rejects_empty() {
        let err = validate_workflow_id("").unwrap_err();
        assert_eq!(err.kind, RuntimeErrorKind::Validation);
    }

    #[test]
    fn test_validate_workflow_id_accepts_existing_builtin_ids() {
        // Every existing builtin id must remain valid under the new allowlist.
        for id in [
            "competitive-multiplayer",
            "competitive-multiplayer-yaml-poc",
            "player-overview",
            "in-game-store",
            "season-pass",
        ] {
            assert!(
                validate_workflow_id(id).is_ok(),
                "builtin id '{id}' must stay valid"
            );
        }
    }

    #[test]
    fn test_validate_workflow_id_rejects_windows_reserved_names() {
        for id in ["CON", "con", "NUL", "com1", "COM9", "lpt3.foo", "Aux"] {
            assert!(
                validate_workflow_id(id).is_err(),
                "reserved id '{id}' must be rejected"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_add_rejects_compile_failure() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());
        let src = home.path().join("bad-op.yaml");
        std::fs::write(
            &src,
            r#"
id: facade-bad-op-probe
name: Bad operation probe
steps:
  - id: only-step
    description: probe
    operation: {service: iam, operation: iam/admin/does-not-exist/v1/nope}
    inputs: []
inputs: []
"#,
        )
        .unwrap();

        assert!(test_runtime().workflow_add(&src, false).is_err());
    }

    #[test]
    fn test_workflow_template_returns_yaml_to_stdout_destination() {
        let output = test_runtime().workflow_template(None).unwrap();
        assert!(matches!(
            output.destination,
            BinaryWrittenDestination::Stdout
        ));
        assert!(output.yaml.contains("id: my-workflow"));
        assert!(
            output.yaml.contains(&format!(
                "workflow_protocol_version: \"{}\"",
                ags_protocol::workflow::WORKFLOW_PROTOCOL_VERSION
            )),
            "template must stamp this CLI build's workflow protocol version: {}",
            output.yaml
        );
        assert!(
            !output.yaml.contains("{{WORKFLOW_PROTOCOL_VERSION}}"),
            "the placeholder must be fully substituted: {}",
            output.yaml
        );
    }

    #[test]
    fn test_workflow_template_writes_to_output_path() {
        let dir = tempfile::tempdir().unwrap();
        let path: PathBuf = dir.path().join("skeleton.yaml");
        let output = test_runtime().workflow_template(Some(&path)).unwrap();
        assert!(matches!(
            output.destination,
            BinaryWrittenDestination::File(_)
        ));
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), output.yaml);
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_remove_deletes_installed_file() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());
        let src = home.path().join("draft.yaml");
        std::fs::write(&src, yaml_with_id("facade-remove-probe")).unwrap();
        test_runtime().workflow_add(&src, false).unwrap();
        let installed = home.path().join("workflows/facade-remove-probe.yaml");
        assert!(installed.exists());

        let output = test_runtime()
            .workflow_remove("facade-remove-probe")
            .unwrap();
        assert_eq!(output.id.as_str(), "facade-remove-probe");
        assert_eq!(output.path, installed);
        assert!(!output.builtin_still_registered);
        assert!(!installed.exists());
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_remove_matches_case_insensitively() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());
        let src = home.path().join("draft.yaml");
        std::fs::write(&src, yaml_with_id("facade-remove-case-probe")).unwrap();
        test_runtime().workflow_add(&src, false).unwrap();

        let output = test_runtime()
            .workflow_remove("Facade-Remove-Case-Probe")
            .unwrap();
        assert!(!output.path.exists());
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_remove_rejects_builtin_id() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());

        let err = test_runtime()
            .workflow_remove("competitive-multiplayer")
            .unwrap_err();
        assert_eq!(err.kind, RuntimeErrorKind::Validation);
        assert!(err.message.contains("built-in"), "got: {}", err.message);
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_remove_rejects_unknown_id() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());

        let err = test_runtime()
            .workflow_remove("no-such-workflow")
            .unwrap_err();
        assert_eq!(err.kind, RuntimeErrorKind::Validation);
        assert!(
            err.message.contains("Unknown workflow"),
            "got: {}",
            err.message
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_workflow_remove_reports_shadowed_builtin() {
        let _guard = HomeEnvGuard;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(config::ENV_HOME, home.path());
        let workflows_dir = home.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        // Same id as the real built-in Rust workflow — this file was never
        // actually loaded into the registry (builtins register first, so
        // `external::load_external_workflows` silently skips the collision),
        // but it still exists on disk and must still be removable.
        std::fs::write(
            workflows_dir.join("shadowed.yaml"),
            yaml_with_id("competitive-multiplayer"),
        )
        .unwrap();

        let output = test_runtime()
            .workflow_remove("competitive-multiplayer")
            .unwrap();
        assert!(output.builtin_still_registered);
        assert!(!workflows_dir.join("shadowed.yaml").exists());
    }
}
