//! Human-readable rendering for `ags ams upload`.

use crate::errors::CliError;
use crate::frontend::output::human::templates;
use crate::frontend::style;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::{
    AmsEntrypointKind, AmsUploadOutput, AmsUploadPlan, AmsUploadResult, AmsUploadView,
};

/// Render an upload result or dry-run plan as human-readable text.
///
/// The detail block is the command's result and goes to stdout, where
/// `--output` and a capturing script read it; the banner and the dry-run tip
/// are guidance and go to stderr. `is_stdout_first` is `false` so the banner
/// still prints before the block on a terminal.
pub(crate) fn render_ams_upload_output(
    output: &AmsUploadOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    // The two colour decisions are taken per stream, and the pairing is
    // load-bearing: stdout-bound text must ask `is_stdout_enabled` and
    // stderr-bound text `is_stderr_enabled`. They differ whenever a user
    // redirects exactly one stream, so do not collapse them into one call.
    Ok(render_with_colors(
        output,
        style::is_stdout_enabled(),
        style::is_stderr_enabled(),
    ))
}

/// Pure core of [`render_ams_upload_output`], with the per-stream colour
/// decisions passed in. Split out so a test can drive stdout and stderr with
/// different colour states — the atomics behind `style::is_std*_enabled` are
/// both false under a captured (non-TTY) test run, which would hide a swap of
/// the two.
fn render_with_colors(
    output: &AmsUploadOutput,
    stdout_color: bool,
    stderr_color: bool,
) -> RenderedOutput {
    let (stdout, stderr) = match &output.view {
        AmsUploadView::Uploaded(result) => (
            render_uploaded_details(result, stdout_color),
            style::success(
                &format!("Image \"{}\" uploaded", result.image_name),
                stderr_color,
            ),
        ),
        AmsUploadView::Planned(plan) => (
            render_plan_details(plan, stdout_color),
            format!(
                "{}\n{}",
                style::info(
                    &format!("Would upload image \"{}\"", plan.image_name),
                    stderr_color,
                ),
                templates::render_tip_text(
                    "Dry run: nothing was archived and no request was sent.",
                    stderr_color,
                )
            ),
        ),
    };
    RenderedOutput {
        stdout: Some(stdout),
        stderr: Some(stderr),
        is_stdout_first: false,
    }
}

/// Detail block for a completed upload.
fn render_uploaded_details(result: &AmsUploadResult, color: bool) -> String {
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
    templates::render_label_value_block_text(&rows, crate::frontend::style::Tone::Plain, color)
}

/// Detail block for a `--dry-run` plan.
fn render_plan_details(plan: &AmsUploadPlan, color: bool) -> String {
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

    templates::render_label_value_block_text(&rows, crate::frontend::style::Tone::Plain, color)
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

    /// The detail block is the command's result and belongs on stdout;
    /// the banner is chrome and belongs on stderr.
    #[test]
    fn test_uploaded_result_puts_details_on_stdout_and_banner_on_stderr() {
        let output = AmsUploadOutput {
            view: AmsUploadView::Uploaded(uploaded_result()),
        };

        let rendered = render_ams_upload_output(&output, &render_options()).unwrap();

        let stdout = rendered.stdout.expect("the detail block must go to stdout");
        let stderr = rendered.stderr.expect("the banner must go to stderr");
        assert!(stdout.contains("Image ID"), "{stdout}");
        assert!(stdout.contains("img-123"), "{stdout}");
        assert!(stderr.contains("uploaded"), "{stderr}");
        assert!(
            !stdout.contains("uploaded"),
            "the banner must not reach stdout:\n{stdout}"
        );
    }

    /// The dry-run plan is data; the banner and the tip are guidance.
    #[test]
    fn test_plan_puts_rows_on_stdout_and_banner_with_tip_on_stderr() {
        let output = AmsUploadOutput {
            view: AmsUploadView::Planned(plan()),
        };

        let rendered = render_ams_upload_output(&output, &render_options()).unwrap();

        let stdout = rendered.stdout.expect("the plan rows must go to stdout");
        let stderr = rendered.stderr.expect("the banner must go to stderr");
        assert!(stdout.contains("Directory"), "{stdout}");
        assert!(stdout.contains("Entrypoint"), "{stdout}");
        assert!(stdout.contains("Architecture"), "{stdout}");
        assert!(stderr.contains("Would upload image"), "{stderr}");
        assert!(
            stderr.contains("Dry run: nothing was archived and no request was sent."),
            "the tip is guidance and belongs with the banner:\n{stderr}"
        );
        assert!(
            !stdout.contains("Dry run:"),
            "the tip must not reach stdout:\n{stdout}"
        );
    }

    /// Stderr prints first so the banner still precedes the detail block.
    #[test]
    fn test_banner_prints_before_the_detail_block() {
        for view in [
            AmsUploadView::Uploaded(uploaded_result()),
            AmsUploadView::Planned(plan()),
        ] {
            let rendered =
                render_ams_upload_output(&AmsUploadOutput { view }, &render_options()).unwrap();
            assert!(
                !rendered.is_stdout_first,
                "stderr must print first so the banner leads"
            );
        }
    }

    /// The banner's colour must come from stderr, the stream it is written
    /// to — never from stdout's. The two decisions differ whenever a user
    /// redirects exactly one stream, so a swap of the sources is a real bug.
    /// Under a captured test run both atomics are false, which is why this
    /// drives the pure core with the two states set apart instead.
    #[test]
    fn test_the_banner_takes_its_color_from_stderr_not_stdout() {
        for view in [
            AmsUploadView::Uploaded(uploaded_result()),
            AmsUploadView::Planned(plan()),
        ] {
            let output = AmsUploadOutput { view };

            let stderr_colored = render_with_colors(&output, false, true);
            assert!(
                stderr_colored.stderr.as_deref().unwrap().contains('\x1b'),
                "the banner must be coloured when stderr has colour"
            );

            let stdout_colored = render_with_colors(&output, true, false);
            assert!(
                !stdout_colored.stderr.as_deref().unwrap().contains('\x1b'),
                "the banner must stay plain when only stdout has colour"
            );
        }
    }

    /// The detail block is `Tone::Plain`, so it carries no escape codes under
    /// either colour state. The stdout flag is still threaded through it so
    /// the block keeps reading its own stream's decision if it ever gains a
    /// tone.
    #[test]
    fn test_the_detail_block_is_plain_under_every_color_state() {
        for view in [
            AmsUploadView::Uploaded(uploaded_result()),
            AmsUploadView::Planned(plan()),
        ] {
            let output = AmsUploadOutput { view };
            for (stdout_color, stderr_color) in [(true, false), (false, true), (true, true)] {
                let rendered = render_with_colors(&output, stdout_color, stderr_color);
                assert!(
                    !rendered.stdout.as_deref().unwrap().contains('\x1b'),
                    "the detail block must carry no escape codes"
                );
            }
        }
    }

    fn render_options() -> RenderOptions {
        RenderOptions {
            verbosity: ags_protocol::request::Verbosity::Normal,
            is_page_all: false,
            output: None,
        }
    }

    fn uploaded_result() -> AmsUploadResult {
        AmsUploadResult {
            image_id: "img-123".to_string(),
            image_name: "my-image".to_string(),
            target_architecture: "linux-x86_64".to_string(),
            command: "./server".to_string(),
            file_count: 2,
            archive_bytes: 2048,
            part_count: 1,
            upload_base_url: "https://upload.example.test".to_string(),
            skipped_directory_symlinks: Vec::new(),
        }
    }

    fn plan() -> AmsUploadPlan {
        AmsUploadPlan {
            image_name: "my-image".to_string(),
            directory: "/tmp/build".to_string(),
            executable: "server".to_string(),
            command: "./server".to_string(),
            target_architecture: "linux-x86_64".to_string(),
            entrypoint_kind: AmsEntrypointKind::ElfBinary,
            file_count: 2,
            total_bytes: 2048,
            include_symbol_files: false,
            excluded_symbol_file_count: 1,
            skipped_directory_symlinks: Vec::new(),
            upload_base_url: None,
        }
    }
}
