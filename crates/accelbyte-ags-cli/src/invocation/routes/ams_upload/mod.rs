//! `ags ams upload` — the hand-written resource under the generated `ams`
//! service.
//!
//! Structurally this is the `auth` path, not the service path: the work is not
//! a catalogued API operation, so there is no schema to synthesise a workflow
//! from. The route owns its frontend and run lifecycle, reports progress
//! through `ProgressSink`, and returns a `CommandOutput` the normal renderers
//! handle.

use clap::ArgMatches;

use ags_protocol::output::{AmsUploadOutput, CommandOutput};
use ags_runtime::runtime::ams_upload::{TargetArchitecture, UploadRequest};

use crate::errors::CliError;
use crate::invocation::builder;
use crate::invocation::clap_helpers;
use crate::invocation::context::{FrontendContext, PhaseBackend};
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;

/// The token that selects this route under the `ams` service.
pub(crate) const UPLOAD_RESOURCE: &str = "upload";

/// Whether a set of `ams` service args is really an `ams upload` invocation.
pub(crate) fn is_ams_upload(service_args: &[String]) -> bool {
    service_args
        .first()
        .is_some_and(|first| first == UPLOAD_RESOURCE)
}

/// Run `ags ams upload`, owning frontend construction and the run lifecycle.
///
/// `service_args` is the full `["upload", …]` slice as typed.
pub(crate) async fn route_ams_upload(
    service_args: &[String],
    flags: &GlobalFlags,
    backend: PhaseBackend,
    options: crate::frontend::RenderOptions,
    frontend_context: &FrontendContext,
) -> Result<InvocationOutcome, CliError> {
    let mut command = builder::build_ams_upload_command();
    let argv = clap_helpers::build_argv(UPLOAD_RESOURCE, &service_args[1..]);

    // Parse before any frontend exists, so help and usage errors emit no run
    // lifecycle — the same ordering the auth path uses.
    let matches = match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
        Ok(matches) => matches,
        Err(error) => match clap_helpers::outcome_from_clap_error(error) {
            Ok(outcome) => return Ok(outcome),
            Err(usage_error) => {
                let exit_code = usage_error.exit_code();
                let mut frontend = crate::frontend::frontend_for_surface(
                    frontend_context.pre_surface_backend(),
                    options.clone(),
                )?;
                frontend.render_error(&usage_error);
                let _ = frontend.finish();
                return Ok(InvocationOutcome::Exit(exit_code));
            }
        },
    };

    let request = build_upload_request(&matches, flags.verbosity.is_verbose());

    // `--dry-run` is entirely local — no host discovery, no archive, no calls —
    // so it must not run the auth prologue either. It stays usable before a
    // first login, which is the whole point of the flag.
    if flags.is_dry_run {
        let mut frontend = self_owned_frontend(backend, options, frontend_context)?;
        frontend.on_event(&crate::frontend::FrontendEvent::RunStarted {
            workflow_banner: None,
        });
        let runtime = ags_runtime::runtime::Runtime::from_reqwest(
            ags_runtime::runtime::execution::ExecutionContext::default(),
            ags_runtime::runtime::dispatch::http::build_http_client(flags.timeout)?,
        );
        let result = runtime
            .ams_upload_dry_run(&request)
            .map(|view| CommandOutput::AmsUpload(AmsUploadOutput { view }))
            .map_err(CliError::from);
        return Ok(finish_upload_run(frontend, result));
    }

    let (context, http_client) = super::run_prologue(flags, frontend_context, &options).await?;
    let mut frontend = self_owned_frontend(backend, options, frontend_context)?;
    frontend.on_event(&crate::frontend::FrontendEvent::RunStarted {
        workflow_banner: None,
    });

    let runtime = ags_runtime::runtime::Runtime::from_reqwest(context, http_client);
    let result = {
        let mut sink = crate::frontend::FrontendSink::new(frontend.as_mut());
        runtime
            .ams_upload(&request, &mut sink)
            .await
            .map(|view| CommandOutput::AmsUpload(AmsUploadOutput { view }))
            .map_err(CliError::from)
    };
    Ok(finish_upload_run(frontend, result))
}

/// Build the presentation surface for an upload run.
///
/// Upload is single-shot and gathers no input, so it takes the plain/JSON
/// surfaces rather than acquiring the terminal for a form.
fn self_owned_frontend(
    backend: PhaseBackend,
    options: crate::frontend::RenderOptions,
    frontend_context: &FrontendContext,
) -> Result<Box<dyn crate::frontend::Frontend>, CliError> {
    let frontend = crate::frontend::frontend_for_surface(backend, options)?;
    crate::invocation::register_reporter_if_plain(frontend_context);
    crate::invocation::try_emit_first_run_hint(frontend_context, false);
    Ok(frontend)
}

/// Close out an upload run: emit `RunFinished`, render the result or error,
/// then tear the frontend down. Mirrors the auth path's `finish_auth_run`.
fn finish_upload_run(
    mut frontend: Box<dyn crate::frontend::Frontend>,
    result: Result<CommandOutput, CliError>,
) -> InvocationOutcome {
    let outcome = match &result {
        Ok(_) => crate::frontend::RunOutcome::Success,
        Err(_) => crate::frontend::RunOutcome::Failed,
    };
    frontend.on_event(&crate::frontend::FrontendEvent::RunFinished { outcome });
    match result {
        Ok(output) => {
            if let Err(error) = frontend.render(&output) {
                let exit_code = error.exit_code();
                frontend.render_error(&error);
                let _ = frontend.finish();
                return InvocationOutcome::Exit(exit_code);
            }
            let _ = frontend.finish();
            InvocationOutcome::Complete
        }
        Err(error) => {
            let exit_code = error.exit_code();
            frontend.render_error(&error);
            let _ = frontend.finish();
            InvocationOutcome::Exit(exit_code)
        }
    }
}

/// Translate parsed flags into the runtime's upload request.
fn build_upload_request(matches: &ArgMatches, is_verbose: bool) -> UploadRequest {
    UploadRequest {
        directory: matches
            .get_one::<String>("path")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(".")),
        executable: matches
            .get_one::<String>("executable")
            .cloned()
            .unwrap_or_default(),
        image_name: matches
            .get_one::<String>("image-name")
            .cloned()
            .unwrap_or_default(),
        target_architecture: matches.get_one::<String>("target-arch").map(|value| {
            TargetArchitecture::parse(value)
                .expect("clap value_parser already restricted the value")
        }),
        include_symbol_files: matches.get_flag("symbol-files"),
        skip_script_validation: matches.get_flag("skip-script-validation"),
        upload_url_override: matches.get_one::<String>("upload-url").cloned(),
        part_concurrency: matches
            .get_one::<u16>("part-concurrency")
            .copied()
            .unwrap_or(ags_runtime::runtime::ams_upload::DEFAULT_PART_CONCURRENCY as u16)
            as usize,
        is_verbose,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse `args` through the real `ams upload` clap command.
    fn parse(args: &[&str]) -> Result<ArgMatches, clap::Error> {
        let argv = std::iter::once(UPLOAD_RESOURCE).chain(args.iter().copied());
        builder::build_ams_upload_command().try_get_matches_from(argv)
    }

    #[test]
    fn test_is_ams_upload_matches_only_the_upload_token() {
        assert!(is_ams_upload(&["upload".to_string()]));
        assert!(is_ams_upload(&[
            "upload".to_string(),
            "--image-name".to_string()
        ]));
        assert!(!is_ams_upload(&["images".to_string(), "list".to_string()]));
        assert!(!is_ams_upload(&[]));
    }

    #[test]
    fn test_executable_and_image_name_are_required() {
        assert!(parse(&["--image-name", "my-image"]).is_err());
        assert!(parse(&["--executable", "./server"]).is_err());
        assert!(parse(&["--executable", "./server", "--image-name", "my-image"]).is_ok());
    }

    #[test]
    fn test_defaults_match_the_documented_behaviour() {
        let matches = parse(&["--executable", "./server", "--image-name", "my-image"]).unwrap();
        let request = build_upload_request(&matches, false);
        assert_eq!(request.directory, std::path::PathBuf::from("."));
        assert_eq!(request.image_name, "my-image");
        assert_eq!(request.executable, "./server");
        assert!(request.target_architecture.is_none());
        assert!(!request.include_symbol_files);
        assert!(!request.skip_script_validation);
        assert!(request.upload_url_override.is_none());
        assert_eq!(
            request.part_concurrency,
            ags_runtime::runtime::ams_upload::DEFAULT_PART_CONCURRENCY
        );
    }

    #[test]
    fn test_target_arch_is_restricted_to_ams_values() {
        assert!(parse(&[
            "--executable",
            "./start.sh",
            "--image-name",
            "my-image",
            "--target-arch",
            "linux-amd64"
        ])
        .is_err());
        let matches = parse(&[
            "--executable",
            "./start.sh",
            "--image-name",
            "my-image",
            "--target-arch",
            "linux-arm_64",
        ])
        .unwrap();
        assert_eq!(
            build_upload_request(&matches, false).target_architecture,
            Some(TargetArchitecture::LinuxArm64)
        );
    }

    /// The old `ams` CLI took credentials as flags. They are gone, so a
    /// migrated script fails loudly rather than silently ignoring a secret.
    #[test]
    fn test_legacy_credential_flags_are_rejected() {
        for flag in ["--clientId", "-c", "--secret", "-s", "--hostURL", "-H"] {
            assert!(
                parse(&["--executable", "./server", "--image-name", "img", flag, "x"]).is_err(),
                "{flag} must not be accepted"
            );
        }
    }
}
