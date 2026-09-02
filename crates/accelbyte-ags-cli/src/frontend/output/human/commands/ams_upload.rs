//! Human-readable rendering for `ags ams upload`.

use crate::errors::CliError;
use crate::frontend::output::human::templates;
use crate::frontend::style;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::{
    AmsEntrypointKind, AmsUploadOutput, AmsUploadPlan, AmsUploadResult, AmsUploadView,
};

/// Render an upload result or dry-run plan as human-readable text.
pub(crate) fn render_ams_upload_output(
    output: &AmsUploadOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let (stdout, stderr) = match &output.view {
        AmsUploadView::Uploaded(result) => (
            style::success(
                &format!("Image \"{}\" uploaded", result.image_name),
                style::is_stdout_enabled(),
            ),
            render_uploaded_details(result),
        ),
        AmsUploadView::Planned(plan) => (
            style::info(
                &format!("Would upload image \"{}\"", plan.image_name),
                style::is_stdout_enabled(),
            ),
            render_plan_details(plan),
        ),
    };
    Ok(RenderedOutput {
        stdout: Some(stdout),
        stderr: Some(stderr),
        is_stdout_first: true,
    })
}

/// Detail block for a completed upload.
fn render_uploaded_details(result: &AmsUploadResult) -> String {
    let rows = vec![
        ("Image ID", result.image_id.clone()),
        ("Architecture", result.target_architecture.clone()),
        ("Entrypoint", result.command.clone()),
        (
            "Archive",
            format!(
                "{} from {}",
                format_bytes(result.archive_bytes),
                pluralized_files(result.file_count)
            ),
        ),
        ("Upload host", result.upload_base_url.clone()),
    ];
    let mut rows: Vec<(&str, String)> = rows;
    if result.part_count > 1 {
        rows.push(("Parts", result.part_count.to_string()));
    }
    if let Some(skipped) = render_skipped_symlinks(&result.skipped_directory_symlinks) {
        rows.push(("Skipped", skipped));
    }
    templates::render_label_value_block_text(
        &rows,
        crate::frontend::style::Tone::Plain,
        style::is_stderr_enabled(),
    )
}

/// Detail block for a `--dry-run` plan.
fn render_plan_details(plan: &AmsUploadPlan) -> String {
    let entrypoint = match plan.entrypoint_kind {
        AmsEntrypointKind::ElfBinary => format!("{} (ELF binary)", plan.command),
        AmsEntrypointKind::ShellScript => format!("{} (shell script)", plan.command),
    };
    let mut rows: Vec<(&str, String)> = vec![
        ("Directory", plan.directory.clone()),
        ("Entrypoint", entrypoint),
        ("Architecture", plan.target_architecture.clone()),
        (
            "Contents",
            format!(
                "{} ({})",
                pluralized_files(plan.file_count),
                format_bytes(plan.total_bytes)
            ),
        ),
    ];
    if plan.excluded_symbol_file_count > 0 {
        rows.push((
            "Excluded",
            format!(
                "{} (pass --symbol-files to include)",
                pluralized_files(plan.excluded_symbol_file_count)
            ),
        ));
    }
    if let Some(skipped) = render_skipped_symlinks(&plan.skipped_directory_symlinks) {
        rows.push(("Skipped", skipped));
    }
    rows.push((
        "Upload host",
        plan.upload_base_url
            .clone()
            .unwrap_or_else(|| "resolved at upload time".to_string()),
    ));

    let block = templates::render_label_value_block_text(
        &rows,
        crate::frontend::style::Tone::Plain,
        style::is_stderr_enabled(),
    );
    format!(
        "{block}\n{}",
        templates::render_tip_text(
            "Dry run: nothing was archived and no request was sent.",
            style::is_stderr_enabled(),
        )
    )
}

/// Name the symlinked directories left out of the archive. Naming them, rather
/// than counting them, is what makes the omission actionable.
fn render_skipped_symlinks(paths: &[String]) -> Option<String> {
    (!paths.is_empty()).then(|| {
        format!(
            "{} (symlinked {}, not archived)",
            paths.join(", "),
            ags_runtime::support::strings::pluralize("directory", paths.len())
        )
    })
}

/// Render a byte count at a human scale.
fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let bytes = bytes as f64;
    if bytes < KIB {
        return format!("{bytes:.0} B");
    }
    if bytes < KIB * KIB {
        return format!("{:.1} KiB", bytes / KIB);
    }
    if bytes < KIB * KIB * KIB {
        return format!("{:.1} MiB", bytes / (KIB * KIB));
    }
    format!("{:.2} GiB", bytes / (KIB * KIB * KIB))
}

/// "1 file" / "12 files".
fn pluralized_files(count: usize) -> String {
    format!(
        "{count} {}",
        ags_runtime::support::strings::pluralize("file", count)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes_scales() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.00 GiB");
    }

    #[test]
    fn test_pluralized_files() {
        assert_eq!(pluralized_files(1), "1 file");
        assert_eq!(pluralized_files(0), "0 files");
        assert_eq!(pluralized_files(12), "12 files");
    }
}
