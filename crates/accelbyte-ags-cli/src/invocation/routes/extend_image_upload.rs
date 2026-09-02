//! `ags extend image-upload` — build and push a container image to the
//! Extend registry.
//!
//! Self-owning route: `--help` runs before any auth or network call.
//! The handler is an imperative eight-step sequence (not workflow-backed).

use crate::errors::CliError;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::handlers::extend::image_upload::{handle_image_upload, ImageUploadParams};
use crate::invocation::InvocationOutcome;

/// Route `ags extend image-upload [flags]`.
///
/// `args` is `remaining[2..]` — everything after `["extend", "image-upload"]`.
pub(crate) async fn route_extend_image_upload(
    args: &[String],
    flags: &mut GlobalFlags,
    _render_options: crate::frontend::RenderOptions,
    frontend_context: &crate::invocation::context::FrontendContext,
) -> Result<InvocationOutcome, CliError> {
    crate::invocation::router::parse_page_limit(flags)?;

    // `--help` is handled before any auth / network call.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return render_image_upload_help();
    }

    // Parse flags via the clap command tree.
    let mut command = crate::invocation::builder::build_extend_command();
    let argv: Vec<String> = std::iter::once("extend".to_string())
        .chain(std::iter::once("image-upload".to_string()))
        .chain(args.iter().cloned())
        .collect();
    let matches = command
        .try_get_matches_from_mut(argv.iter().map(String::as_str))
        .map_err(|error| CliError::Usage {
            message: crate::invocation::clap_helpers::strip_clap_prefix(&error.to_string()),
            metadata: None,
        })?;
    let (_, image_upload_matches) = matches
        .subcommand()
        .and_then(|(name, sub)| {
            if name == "image-upload" {
                Some((name, sub))
            } else {
                None
            }
        })
        .ok_or_else(|| CliError::Usage {
            message: "Expected image-upload subcommand".to_string(),
            metadata: None,
        })?;

    // Emit the first-run onboarding hint and register the lock-contention
    // reporter for plain surfaces, mirroring the docker-login route.
    let is_meta = false;
    crate::invocation::try_emit_first_run_hint(frontend_context, is_meta);
    crate::invocation::register_reporter_if_plain(frontend_context);

    // Extract parameters.
    let app = image_upload_matches
        .get_one::<String>("app")
        .ok_or_else(|| CliError::Usage {
            message: "--app is required for image-upload".to_string(),
            metadata: None,
        })?
        .clone();

    let image_tag = image_upload_matches
        .get_one::<String>("image-tag")
        .ok_or_else(|| CliError::Usage {
            message: "--image-tag is required for image-upload".to_string(),
            metadata: None,
        })?
        .clone();

    let dockerfile = image_upload_matches
        .get_one::<String>("dockerfile")
        .cloned()
        .unwrap_or_else(|| "Dockerfile".to_string());

    let platforms: Vec<String> = image_upload_matches
        .get_many::<String>("platform")
        .map(|vals| vals.cloned().collect())
        .unwrap_or_else(|| vec!["linux/amd64".to_string()]);

    let work_dir = image_upload_matches.get_one::<String>("work-dir").cloned();

    let login = image_upload_matches.get_flag("login");

    let retry_limit = image_upload_matches
        .get_one::<u32>("retry-limit")
        .copied()
        .unwrap_or(0);

    let retry_interval = image_upload_matches
        .get_one::<f64>("retry-interval")
        .copied()
        .unwrap_or(1.0);

    let retry_rate = image_upload_matches
        .get_one::<f64>("retry-rate")
        .copied()
        .unwrap_or(2.0);

    let params = ImageUploadParams {
        app,
        image_tag,
        dockerfile,
        platforms,
        work_dir,
        login,
        retry_limit,
        retry_interval,
        retry_rate,
    };

    handle_image_upload(&params, flags).await
}

/// Render `--help` for `image-upload` by building the clap command and
/// letting it print its own help. Returns `Complete` since no I/O
/// beyond stdout occurs.
fn render_image_upload_help() -> Result<InvocationOutcome, CliError> {
    let mut command = crate::invocation::builder::build_extend_command();
    // Navigate to the image-upload subcommand.
    let help = command
        .find_subcommand_mut("image-upload")
        .map(|sub| sub.render_help().to_string())
        .unwrap_or_else(|| command.render_help().to_string());

    crate::frontend::write_stdout_line(&help)?;
    Ok(InvocationOutcome::Complete)
}
