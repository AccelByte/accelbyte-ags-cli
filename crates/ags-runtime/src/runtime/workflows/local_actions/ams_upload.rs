//! Local action that uploads a dedicated-server build to AMS.
//!
//! Wraps the same pipeline `ags ams upload` runs, so a workflow can produce an
//! image instead of requiring one to exist beforehand.

use ags_protocol::error::RuntimeError;
use ags_protocol::event::ProgressSink;
use ags_protocol::output::AmsUploadView;
use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::runtime::ams_upload::{TargetArchitecture, UploadRequest, DEFAULT_PART_CONCURRENCY};
use crate::runtime::Runtime;

use super::{required_string, LocalAction, LocalActionInput};

/// Id a workflow definition names to reach this action.
pub(super) const ID: &str = "ams/upload-image";

pub(super) struct AmsUploadStep;

#[async_trait(?Send)]
impl LocalAction for AmsUploadStep {
    fn inputs(&self) -> Vec<LocalActionInput> {
        vec![
            LocalActionInput {
                name: "path",
                required: true,
                description: "Directory holding the built dedicated server.",
            },
            LocalActionInput {
                name: "executable",
                required: true,
                description: "Entrypoint to run, relative to the directory.",
            },
            LocalActionInput {
                name: "imageName",
                required: true,
                description: "Name of the AMS image to create.",
            },
            LocalActionInput {
                name: "targetArchitecture",
                required: false,
                description:
                    "linux-x86_64 or linux-arm_64. Required for a shell-script entrypoint.",
            },
        ]
    }

    async fn run(
        &self,
        runtime: &Runtime,
        inputs: &Map<String, Value>,
        sink: &mut dyn ProgressSink,
        dry_run: bool,
    ) -> Result<Value, RuntimeError> {
        let request = UploadRequest {
            directory: required_string(inputs, "path", ID)?.into(),
            executable: required_string(inputs, "executable", ID)?,
            image_name: required_string(inputs, "imageName", ID)?,
            target_architecture: inputs
                .get("targetArchitecture")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(|value| {
                    TargetArchitecture::parse(value).ok_or_else(|| RuntimeError {
                        kind: ags_protocol::error::RuntimeErrorKind::Validation,
                        message: format!("'{value}' is not a target architecture AMS accepts"),
                        details: None,
                        hint: Some(format!("Use {}.", TargetArchitecture::all().join(" or "))),
                        trace: None,
                    })
                })
                .transpose()?,
            include_symbol_files: false,
            skip_script_validation: false,
            upload_url_override: None,
            part_concurrency: DEFAULT_PART_CONCURRENCY,
            is_verbose: false,
        };

        // A dry run validates the build directory and entrypoint locally but
        // builds no archive and makes no call, so a workflow preview stays
        // free of side effects while still catching a broken entrypoint.
        let view = if dry_run {
            runtime.ams_upload_dry_run(&request)?
        } else {
            runtime.ams_upload(&request, sink).await?
        };

        // `skipped_directory_symlinks` is carried through because a silently
        // truncated image is the failure this reports; the live path also warns
        // through the sink, but a dry run has only what it returns here.
        Ok(match view {
            AmsUploadView::Uploaded(result) => uploaded_envelope(&result),
            AmsUploadView::Planned(plan) => planned_envelope(&plan),
        })
    }
}

/// Build the snake_case JSON envelope for a completed upload.
fn uploaded_envelope(result: &ags_protocol::output::AmsUploadResult) -> Value {
    json!({
        "image_id": result.image_id,
        "image_name": result.image_name,
        "target_architecture": result.target_architecture,
        "command": result.command,
        "archive_bytes": result.archive_bytes,
        "part_count": result.part_count,
        "skipped_directory_symlinks": result.skipped_directory_symlinks,
    })
}

/// Build the snake_case JSON envelope for a dry-run plan. The image id is
/// only known after a real upload, so the preview carries a placeholder so
/// downstream steps still resolve their bindings.
fn planned_envelope(plan: &ags_protocol::output::AmsUploadPlan) -> Value {
    json!({
        "image_id": "<image-id>",
        "image_name": plan.image_name,
        "target_architecture": plan.target_architecture,
        "command": plan.command,
        "archive_bytes": plan.total_bytes,
        "part_count": 1,
        "skipped_directory_symlinks": plan.skipped_directory_symlinks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every input the workflow binds must be declared, or `compile_workflow`
    /// will reject the binding as unknown.
    #[test]
    fn test_declares_the_inputs_the_pipeline_needs() {
        let declared: Vec<&str> = AmsUploadStep.inputs().iter().map(|i| i.name).collect();
        assert_eq!(
            declared,
            vec!["path", "executable", "imageName", "targetArchitecture"]
        );
        let required: Vec<&str> = AmsUploadStep
            .inputs()
            .iter()
            .filter(|i| i.required)
            .map(|i| i.name)
            .collect();
        assert_eq!(required, vec!["path", "executable", "imageName"]);
    }

    /// The action's JSON envelope uses snake_case keys per RULE-35.
    /// This test catches a reintroduction of camelCase keys by asserting the
    /// exact key set — a new or renamed key that breaks the convention fails.
    #[test]
    fn test_planned_envelope_keys_are_snake_case() {
        let plan = ags_protocol::output::AmsUploadPlan {
            image_name: "img".into(),
            directory: "/tmp".into(),
            executable: "./srv".into(),
            command: "./srv".into(),
            target_architecture: "linux-x86_64".into(),
            entrypoint_kind: ags_protocol::output::AmsEntrypointKind::ElfBinary,
            file_count: 1,
            total_bytes: 100,
            include_symbol_files: false,
            excluded_symbol_file_count: 0,
            skipped_directory_symlinks: vec![],
            upload_base_url: None,
        };
        let envelope = planned_envelope(&plan);
        let keys: std::collections::BTreeSet<String> = envelope
            .as_object()
            .expect("envelope is an object")
            .keys()
            .cloned()
            .collect();
        let expected: std::collections::BTreeSet<String> = [
            "image_id",
            "image_name",
            "target_architecture",
            "command",
            "archive_bytes",
            "part_count",
            "skipped_directory_symlinks",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(keys, expected, "envelope keys must be snake_case");
    }
}
