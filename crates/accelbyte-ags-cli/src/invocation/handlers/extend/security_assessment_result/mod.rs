//! Handler for `ags extend security-assessment result`.
//!
//! Lists an Extend app's security-assessment sessions, lets the operator
//! pick one by index (or supply `--engagement-id` to skip the picker),
//! fetches a pre-signed download URL for the completed report from CSM, and
//! writes it to disk.

mod api;

use clap::ArgMatches;

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::{write_stderr, write_stderr_line, Frontend};
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use ags_protocol::output::{CommandOutput, SecurityAssessmentResultOutput};

use self::api::EngagementSummary;

/// Report formats accepted by `--report-format`, matching the `format` query
/// parameter `csm/admin/security-assessment/v1/get-report` accepts.
const VALID_REPORT_FORMATS: &[&str] = &["pdf", "md"];

fn read_line_from_stdin() -> Result<String, CliError> {
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| CliError::Usage {
            message: format!("Failed to read input: {e}"),
            metadata: None,
        })?;
    Ok(input.trim().to_string())
}

pub(crate) async fn handle_security_assessment_result(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    frontend: &mut dyn Frontend,
) -> Result<InvocationOutcome, CliError> {
    handle_with_reader(matches, flags, frontend, &mut read_line_from_stdin).await
}

async fn handle_with_reader(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    frontend: &mut dyn Frontend,
    read: &mut dyn FnMut() -> Result<String, CliError>,
) -> Result<InvocationOutcome, CliError> {
    let app = matches
        .get_one::<String>("app")
        .ok_or_else(|| CliError::Usage {
            message: "--app is required".to_string(),
            metadata: None,
        })?
        .clone();
    let namespace = flags.namespace.clone().ok_or_else(|| CliError::Usage {
        message: "--namespace is required for extend security-assessment result".to_string(),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            "Supply --namespace <ns> or set a default via 'ags config set namespace <ns>'",
        ))),
    })?;

    let report_format = matches
        .get_one::<String>("report-format")
        .map(String::as_str)
        .unwrap_or("pdf")
        .to_string();
    if !VALID_REPORT_FORMATS.contains(&report_format.as_str()) {
        return Err(CliError::Usage {
            message: "--report-format must be 'pdf' or 'md'".to_string(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Supported values: pdf, md",
            ))),
        });
    }

    let report_output = matches.get_one::<String>("report-output").cloned();

    let engagement_id_flag = matches.get_one::<String>("engagement-id").cloned();
    if let Some(id) = &engagement_id_flag {
        let is_digits_only = !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit());
        if !is_digits_only || id.parse::<i64>().is_err() {
            return Err(CliError::Usage {
                message: "--engagement-id must be numeric (digits only)".to_string(),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Pass the numeric id shown by 'ags extend security-assessment list', \
                     e.g. --engagement-id 12345",
                ))),
            });
        }
    }

    let http_client = ags_runtime::runtime::dispatch::http::build_http_client(flags.timeout)?;
    let input = ags_runtime::runtime::execution::ResolutionInput {
        profile: flags.profile.clone(),
        namespace: Some(namespace.clone()),
        is_dry_run: false,
    };
    let context =
        ags_runtime::runtime::execution::ExecutionContext::resolve(&input, &http_client).await?;
    let resolved_namespace = context.namespace.clone().unwrap_or(namespace);
    // The final report download hits a pre-signed, unauthenticated URL
    // outside CSM's normal bearer-token surface — kept aside before
    // `context`/`http_client` are moved into the `Runtime` used for the
    // catalogue-driven `list`/`get-report` calls below. Same trick
    // `security_assessment_request` uses for `create_engagement`.
    let download_client = http_client.clone();

    let mut runtime = ags_runtime::runtime::Runtime::from_reqwest(context, http_client);

    let engagement_id = match engagement_id_flag {
        Some(id) => id,
        None => {
            let engagements =
                api::list_engagements(&mut runtime, &resolved_namespace, &app).await?;
            let chosen = pick_engagement_impl(&app, &engagements, flags, read)?;
            chosen.engagement_id.to_string()
        }
    };

    let output_path = report_output.unwrap_or_else(|| {
        format!(
            "{}-{engagement_id}-report.{report_format}",
            sanitize_filename_segment(&app)
        )
    });

    if flags.is_dry_run {
        return dry_run_preview(&resolved_namespace, &app, &engagement_id, &output_path);
    }

    let url = api::get_report(
        &mut runtime,
        &resolved_namespace,
        &engagement_id,
        &report_format,
    )
    .await?;
    let bytes = api::download_report_bytes(&download_client, &url).await?;

    write_report_file(std::path::Path::new(&output_path), &bytes)
        .map_err(crate::frontend::map_output_sink_error_to_cli_error)?;

    let output = SecurityAssessmentResultOutput {
        namespace: resolved_namespace,
        app,
        // Always parseable at this point: either validated above (`--engagement-id`)
        // or derived from `EngagementSummary.engagement_id: i64` via the picker.
        engagement_id: engagement_id.parse().map_err(|_| CliError::Usage {
            message: format!("Engagement id '{engagement_id}' is not representable as a number"),
            metadata: None,
        })?,
        report_format,
        path: output_path,
        bytes_written: bytes.len(),
    };
    frontend.render(&CommandOutput::SecurityAssessmentResult(output))?;

    Ok(InvocationOutcome::Complete)
}

/// Replace path separators in a value destined for the default output
/// filename, so a `--app` containing `/` (or `\` on Windows) can't redirect
/// the write outside the current directory.
fn sanitize_filename_segment(value: &str) -> String {
    value.replace(['/', '\\'], "_")
}

/// Write the downloaded report to `path` with `0600` permissions on Unix.
/// Deliberately bypasses `OutputSink::File` (plain `std::fs::write`, default
/// umask) and writes directly here instead of changing that shared type —
/// it also backs the global `--output` flag, and a pen-test report listing
/// confirmed weaknesses in a running production app shouldn't be
/// world/group-readable by default the way other sink-written files are.
fn write_report_file(
    path: &std::path::Path,
    bytes: &[u8],
) -> Result<(), ags_runtime::support::output_sink::OutputSinkError> {
    use ags_runtime::support::output_sink::OutputSinkError;

    // A nested fn rather than an immediately-called closure: on Windows the
    // `cfg(not(unix))` arm carries no `?`, so clippy's redundant_closure_call
    // fires there and `-D warnings` fails the build. Linux never sees it,
    // because the `?` in the unix arm suppresses the lint.
    fn write_with_owner_only_mode(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)?;
            std::io::Write::write_all(&mut file, bytes)
        }
        #[cfg(not(unix))]
        {
            std::fs::write(path, bytes)
        }
    }

    let write_result: std::io::Result<()> = write_with_owner_only_mode(path, bytes);

    write_result.map_err(anyhow::Error::from).map_err(|error| {
        OutputSinkError::Internal(error.context(format!("Cannot write to '{}'", path.display())))
    })
}

// ── Engagement picker ──

/// Prompt the operator to pick one session by index. Auto-selects when
/// there's exactly one match — same single-choice convention as
/// `clone_template::pick_one_impl`.
fn pick_engagement_impl(
    app: &str,
    engagements: &[EngagementSummary],
    flags: &GlobalFlags,
    read: &mut dyn FnMut() -> Result<String, CliError>,
) -> Result<EngagementSummary, CliError> {
    if engagements.is_empty() {
        return Err(CliError::Usage {
            message: format!("No completed security assessment sessions found for app '{app}'"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Run 'ags extend security-assessment request' to start one, or check \
                 'ags extend security-assessment list' if one is already running",
            ))),
        });
    }
    if engagements.len() == 1 {
        return Ok(engagements[0].clone());
    }
    if flags.is_no_input {
        return Err(CliError::Usage {
            message: "Multiple security assessment sessions available but interactive input is \
                      disabled"
                .to_string(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Use --engagement-id <id> to select one non-interactively",
            ))),
        });
    }

    write_stderr_line("Select a security assessment session:");
    for (i, engagement) in engagements.iter().enumerate() {
        write_stderr_line(&format!(
            "  [{}] id: {} · {} · {} · {} ({})",
            i + 1,
            engagement.engagement_id,
            engagement.target_app,
            api::format_created_at(engagement.created_at.as_deref()),
            engagement.target_app_version.as_deref().unwrap_or("—"),
            engagement.status,
        ));
    }
    write_stderr(&format!("Enter number (1-{}): ", engagements.len()));

    let input = read()?;
    let index: usize = input.trim().parse().map_err(|_| CliError::Usage {
        message: format!("Invalid selection: '{input}'"),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            format!("Enter a number between 1 and {}", engagements.len()),
        ))),
    })?;
    if index < 1 || index > engagements.len() {
        return Err(CliError::Usage {
            message: format!("Selection out of range: {index}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                format!("Enter a number between 1 and {}", engagements.len()),
            ))),
        });
    }
    Ok(engagements[index - 1].clone())
}

// ── Dry-run preview ──

fn dry_run_preview(
    namespace: &str,
    app: &str,
    engagement_id: &str,
    output_path: &str,
) -> Result<InvocationOutcome, CliError> {
    let color = style::is_stderr_enabled();
    write_stderr_line(&style::info(
        "Dry run — no report will be downloaded",
        color,
    ));
    write_stderr_line(&format!("  Namespace:    {namespace}"));
    write_stderr_line(&format!("  App:          {app}"));
    write_stderr_line(&format!("  Engagement:   {engagement_id}"));
    write_stderr_line(&format!("  Report path:  {output_path}"));
    Ok(InvocationOutcome::Complete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct NullFrontend;

    impl crate::frontend::Frontend for NullFrontend {
        fn render(&mut self, _output: &CommandOutput) -> Result<(), CliError> {
            Ok(())
        }
        fn render_error(&mut self, _err: &CliError) {}
        fn render_warning(&mut self, _msg: &str, _reason: Option<&str>, _tip: Option<&str>) {}
        fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {}
        fn finish(self: Box<Self>) -> Result<(), CliError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct CapturingFrontend {
        last: Option<SecurityAssessmentResultOutput>,
    }

    impl crate::frontend::Frontend for CapturingFrontend {
        fn render(&mut self, output: &CommandOutput) -> Result<(), CliError> {
            if let CommandOutput::SecurityAssessmentResult(output) = output {
                self.last = Some(output.clone());
            }
            Ok(())
        }
        fn render_error(&mut self, _err: &CliError) {}
        fn render_warning(&mut self, _msg: &str, _reason: Option<&str>, _tip: Option<&str>) {}
        fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {}
        fn finish(self: Box<Self>) -> Result<(), CliError> {
            Ok(())
        }
    }

    struct TempEnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl TempEnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            use std::env;
            let original = env::var(key).ok();
            env::set_var(key, value);
            Self { key, original }
        }
    }

    impl Drop for TempEnvGuard {
        fn drop(&mut self) {
            use std::env;
            match &self.original {
                Some(val) => env::set_var(self.key, val),
                None => env::remove_var(self.key),
            }
        }
    }

    fn isolated_runtime_env(
        tmp: &tempfile::TempDir,
        server: &wiremock::MockServer,
    ) -> [TempEnvGuard; 4] {
        [
            TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap()),
            TempEnvGuard::set("AGS_NO_KEYCHAIN", "1"),
            TempEnvGuard::set("AGS_ACCESS_TOKEN", "fake-test-token"),
            TempEnvGuard::set("AGS_BASE_URL", &server.uri()),
        ]
    }

    fn real_result_matches(args: &[&str]) -> ArgMatches {
        let mut command = crate::invocation::builder::build_extend_command();
        let argv: Vec<&str> = ["extend", "security-assessment", "result"]
            .into_iter()
            .chain(args.iter().copied())
            .collect();
        let matches = command
            .try_get_matches_from_mut(argv)
            .expect("real command tree must accept these args");
        let (_, sa_matches) = matches
            .subcommand()
            .filter(|(name, _)| *name == "security-assessment")
            .expect("security-assessment subcommand must match");
        let (_, result_matches) = sa_matches
            .subcommand()
            .filter(|(name, _)| *name == "result")
            .expect("result subcommand must match");
        result_matches.clone()
    }

    fn flags(overrides: GlobalFlags) -> GlobalFlags {
        GlobalFlags {
            namespace: Some("test-ns".to_string()),
            ..overrides
        }
    }

    fn panics_if_called() -> Result<String, CliError> {
        panic!("stdin reader should not be called")
    }

    #[tokio::test]
    async fn missing_app_is_usage_error() {
        let matches = clap::Command::new("result")
            .arg(clap::Arg::new("app").long("app"))
            .try_get_matches_from(["result"])
            .unwrap();
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        assert!(matches!(result, Err(CliError::Usage { .. })));
    }

    #[tokio::test]
    async fn missing_namespace_is_usage_error() {
        let matches = real_result_matches(&["--app", "my-app"]);
        let flags = GlobalFlags::default();
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("--namespace")),
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_report_format_is_rejected_before_any_network_call() {
        let matches = real_result_matches(&["--app", "my-app", "--report-format", "xml"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(message.contains("--report-format"))
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_engagement_id_is_rejected_before_any_network_call() {
        let matches = real_result_matches(&["--app", "my-app", "--engagement-id", "not-a-number"]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(message.contains("--engagement-id"))
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    // A digits-only value that overflows i64 must be rejected up front rather
    // than silently coerced to 0 in the rendered output.
    #[tokio::test]
    async fn engagement_id_overflowing_i64_is_rejected_before_any_network_call() {
        let matches = real_result_matches(&[
            "--app",
            "my-app",
            "--engagement-id",
            "999999999999999999999",
        ]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(message.contains("--engagement-id"))
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[test]
    fn sanitize_filename_segment_strips_path_separators() {
        assert_eq!(sanitize_filename_segment("my-app"), "my-app");
        assert_eq!(
            sanitize_filename_segment("../../etc/passwd"),
            ".._.._etc_passwd"
        );
        assert_eq!(sanitize_filename_segment(r"..\windows"), ".._windows");
    }

    // A pen-test report lists confirmed weaknesses in a running production
    // app — it must not be left world/group-readable via the process umask.
    #[cfg(unix)]
    #[test]
    fn write_report_file_sets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("report.pdf");
        write_report_file(&path, b"%PDF-1.4").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.4");
    }

    #[test]
    fn pick_engagement_auto_selects_single_match() {
        let engagements = vec![EngagementSummary {
            engagement_id: 1,
            target_app: "my-app".to_string(),
            target_app_version: Some("v1".to_string()),
            created_at: Some("2026-08-10T10:17:56Z".to_string()),
            status: "COMPLETED".to_string(),
        }];
        let flags = flags(GlobalFlags::default());
        let chosen =
            pick_engagement_impl("my-app", &engagements, &flags, &mut panics_if_called).unwrap();
        assert_eq!(chosen.engagement_id, 1);
    }

    #[test]
    fn pick_engagement_empty_list_is_usage_error() {
        let flags = flags(GlobalFlags::default());
        let result = pick_engagement_impl("my-app", &[], &flags, &mut panics_if_called);
        match result {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("my-app")),
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[test]
    fn pick_engagement_no_input_with_multiple_matches_is_usage_error() {
        let engagements = vec![
            EngagementSummary {
                engagement_id: 1,
                target_app: "my-app".to_string(),
                target_app_version: None,
                created_at: None,
                status: "COMPLETED".to_string(),
            },
            EngagementSummary {
                engagement_id: 2,
                target_app: "my-app".to_string(),
                target_app_version: None,
                created_at: None,
                status: "COMPLETED".to_string(),
            },
        ];
        let flags = flags(GlobalFlags {
            is_no_input: true,
            ..Default::default()
        });
        let result = pick_engagement_impl("my-app", &engagements, &flags, &mut panics_if_called);
        match result {
            Err(CliError::Usage { message, .. }) => {
                assert!(message.contains("interactive input is disabled"))
            }
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[test]
    fn pick_engagement_reads_selection_by_index() {
        let engagements = vec![
            EngagementSummary {
                engagement_id: 1,
                target_app: "my-app".to_string(),
                target_app_version: None,
                created_at: None,
                status: "COMPLETED".to_string(),
            },
            EngagementSummary {
                engagement_id: 2,
                target_app: "my-app".to_string(),
                target_app_version: None,
                created_at: None,
                status: "COMPLETED".to_string(),
            },
        ];
        let flags = flags(GlobalFlags::default());
        let mut reader = || Ok("2".to_string());
        let chosen = pick_engagement_impl("my-app", &engagements, &flags, &mut reader).unwrap();
        assert_eq!(chosen.engagement_id, 2);
    }

    #[test]
    fn pick_engagement_out_of_range_selection_is_usage_error() {
        let engagements = vec![
            EngagementSummary {
                engagement_id: 1,
                target_app: "my-app".to_string(),
                target_app_version: None,
                created_at: None,
                status: "COMPLETED".to_string(),
            },
            EngagementSummary {
                engagement_id: 2,
                target_app: "my-app".to_string(),
                target_app_version: None,
                created_at: None,
                status: "COMPLETED".to_string(),
            },
        ];
        let flags = flags(GlobalFlags::default());
        let mut reader = || Ok("5".to_string());
        let result = pick_engagement_impl("my-app", &engagements, &flags, &mut reader);
        match result {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("out of range")),
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn engagement_id_flag_skips_the_picker_and_downloads() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        Mock::given(method("GET"))
            .and(path(
                "/csm/v1/admin/namespaces/test-ns/pentestings/42/report",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": format!("{}/report.pdf", server.uri())
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/report.pdf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"%PDF-1.4".as_slice()))
            .mount(&server)
            .await;

        let dest = tmp.path().join("out.pdf");
        let matches = real_result_matches(&[
            "--app",
            "my-app",
            "--engagement-id",
            "42",
            "--report-output",
            dest.to_str().unwrap(),
        ]);
        let flags = flags(GlobalFlags::default());
        let mut frontend = CapturingFrontend::default();
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        assert!(result.is_ok(), "expected success, got {result:?}");
        let output = frontend.last.expect("output must be rendered");
        assert_eq!(output.engagement_id, 42);
        assert_eq!(output.path, dest.to_str().unwrap());
        assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-1.4");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn dry_run_does_not_call_get_report_or_download() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let _guards = isolated_runtime_env(&tmp, &server);
        // No mocks mounted for get-report or the download URL — a real
        // call would fail the test via wiremock's "no matching mock" panic.

        let matches = real_result_matches(&["--app", "my-app", "--engagement-id", "42"]);
        let flags = flags(GlobalFlags {
            is_dry_run: true,
            ..Default::default()
        });
        let mut frontend = NullFrontend;
        let result =
            handle_with_reader(&matches, &flags, &mut frontend, &mut panics_if_called).await;
        assert!(
            result.is_ok(),
            "dry-run should succeed without calling get-report or downloading: {result:?}"
        );
    }
}
