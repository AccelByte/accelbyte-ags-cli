//! JSON rendering for `ags ams upload`.

use crate::errors::CliError;
use crate::frontend::output::json::format_json;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::{AmsEntrypointKind, AmsUploadOutput, AmsUploadView};

/// Render an upload result or dry-run plan as JSON.
pub(crate) fn render_ams_upload_output(
    output: &AmsUploadOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let value = match &output.view {
        AmsUploadView::Uploaded(result) => serde_json::json!({
            "status": "uploaded",
            "image_id": result.image_id,
            "image_name": result.image_name,
            "target_architecture": result.target_architecture,
            "command": result.command,
            "file_count": result.file_count,
            "archive_bytes": result.archive_bytes,
            "part_count": result.part_count,
            "upload_base_url": result.upload_base_url,
            "skipped_directory_symlinks": result.skipped_directory_symlinks,
        }),
        AmsUploadView::Planned(plan) => serde_json::json!({
            "status": "dry_run",
            "image_name": plan.image_name,
            "directory": plan.directory,
            "executable": plan.executable,
            "command": plan.command,
            "target_architecture": plan.target_architecture,
            "entrypoint_kind": match plan.entrypoint_kind {
                AmsEntrypointKind::ElfBinary => "elf_binary",
                AmsEntrypointKind::ShellScript => "shell_script",
            },
            "file_count": plan.file_count,
            "total_bytes": plan.total_bytes,
            "include_symbol_files": plan.include_symbol_files,
            "excluded_symbol_file_count": plan.excluded_symbol_file_count,
            "skipped_directory_symlinks": plan.skipped_directory_symlinks,
            "upload_base_url": plan.upload_base_url,
        }),
    };
    Ok(RenderedOutput {
        stdout: Some(format_json(&value)?),
        stderr: None,
        is_stdout_first: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::output::{AmsUploadPlan, AmsUploadResult};

    #[test]
    fn test_uploaded_envelope_carries_the_image_id() {
        let output = AmsUploadOutput {
            view: AmsUploadView::Uploaded(AmsUploadResult {
                image_id: "img-123".into(),
                image_name: "my-image".into(),
                target_architecture: "linux-x86_64".into(),
                command: "./server".into(),
                file_count: 4,
                archive_bytes: 2048,
                part_count: 1,
                upload_base_url: "https://dev.ams.accelbyte.io".into(),
                skipped_directory_symlinks: vec!["assets".into()],
            }),
        };
        let rendered = render_ams_upload_output(&output, &RenderOptions::default()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["status"], "uploaded");
        assert_eq!(json["image_id"], "img-123");
        assert_eq!(json["part_count"], 1);
        assert_eq!(json["skipped_directory_symlinks"][0], "assets");
        assert!(
            rendered.stderr.is_none(),
            "JSON output keeps stdout as the only payload"
        );
    }

    #[test]
    fn test_dry_run_envelope_reports_an_unresolved_host_as_null() {
        let output = AmsUploadOutput {
            view: AmsUploadView::Planned(AmsUploadPlan {
                image_name: "my-image".into(),
                directory: "./build".into(),
                executable: "server".into(),
                command: "./server".into(),
                target_architecture: "linux-arm_64".into(),
                entrypoint_kind: AmsEntrypointKind::ElfBinary,
                file_count: 2,
                total_bytes: 10,
                include_symbol_files: false,
                excluded_symbol_file_count: 1,
                skipped_directory_symlinks: Vec::new(),
                upload_base_url: None,
            }),
        };
        let rendered = render_ams_upload_output(&output, &RenderOptions::default()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["status"], "dry_run");
        assert_eq!(json["entrypoint_kind"], "elf_binary");
        assert!(json["upload_base_url"].is_null());
    }

    /// The JSON envelope for `ags ams upload` must use snake_case keys
    /// exclusively (RULE-35). This test asserts on the complete key set
    /// of each arm so that a later edit introducing a camelCase key is
    /// caught automatically.
    #[test]
    fn test_uploaded_envelope_uses_only_snake_case_keys() {
        let output = AmsUploadOutput {
            view: AmsUploadView::Uploaded(AmsUploadResult {
                image_id: "img-1".into(),
                image_name: "n".into(),
                target_architecture: "linux-x86_64".into(),
                command: "./s".into(),
                file_count: 1,
                archive_bytes: 64,
                part_count: 1,
                upload_base_url: "https://example.com".into(),
                skipped_directory_symlinks: Vec::new(),
            }),
        };
        let rendered = render_ams_upload_output(&output, &RenderOptions::default()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        let keys: std::collections::BTreeSet<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        let expected: std::collections::BTreeSet<&str> = [
            "status",
            "image_id",
            "image_name",
            "target_architecture",
            "command",
            "file_count",
            "archive_bytes",
            "part_count",
            "upload_base_url",
            "skipped_directory_symlinks",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            keys, expected,
            "uploaded envelope must have exactly these snake_case keys"
        );
        // No key may contain an uppercase letter (camelCase canary).
        for key in json.as_object().unwrap().keys() {
            assert!(
                key.chars().all(|c| !c.is_ascii_uppercase()),
                "key {key:?} contains uppercase — RULE-35 requires snake_case"
            );
        }
    }

    #[test]
    fn test_planned_envelope_uses_only_snake_case_keys() {
        let output = AmsUploadOutput {
            view: AmsUploadView::Planned(AmsUploadPlan {
                image_name: "n".into(),
                directory: ".".into(),
                executable: "s".into(),
                command: "./s".into(),
                target_architecture: "linux-x86_64".into(),
                entrypoint_kind: AmsEntrypointKind::ElfBinary,
                file_count: 1,
                total_bytes: 10,
                include_symbol_files: false,
                excluded_symbol_file_count: 0,
                skipped_directory_symlinks: Vec::new(),
                upload_base_url: None,
            }),
        };
        let rendered = render_ams_upload_output(&output, &RenderOptions::default()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        let keys: std::collections::BTreeSet<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        let expected: std::collections::BTreeSet<&str> = [
            "status",
            "image_name",
            "directory",
            "executable",
            "command",
            "target_architecture",
            "entrypoint_kind",
            "file_count",
            "total_bytes",
            "include_symbol_files",
            "excluded_symbol_file_count",
            "skipped_directory_symlinks",
            "upload_base_url",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            keys, expected,
            "planned envelope must have exactly these snake_case keys"
        );
        for key in json.as_object().unwrap().keys() {
            assert!(
                key.chars().all(|c| !c.is_ascii_uppercase()),
                "key {key:?} contains uppercase — RULE-35 requires snake_case"
            );
        }
    }
}
