//! Top-level command routes: auth, service, builtin, and workflow.

pub mod auth;
pub mod builtin;
pub mod service;
pub mod workflow;

use ags_runtime::runtime::execution::{ExecutionContext, ResolutionInput};
use reqwest::Client;

use crate::errors::CliError;
use crate::frontend::RenderOptions;
use crate::invocation::context::FrontendContext;
use crate::invocation::flags::GlobalFlags;

/// Shared runtime prologue for the service and workflow routes: resolve auth and
/// base URL, then render any access-token warnings on a fresh pre-surface
/// frontend so they appear before any owned phase surface exists. Runs with NO
/// phase surfaces, so a resolution failure `?`-propagates as `Err` for the
/// caller's fresh human frontend to render. Returns the resolved context and
/// HTTP client for the caller to build a `Runtime` from.
pub(crate) async fn run_prologue(
    flags: &GlobalFlags,
    frontend_context: &FrontendContext,
    render_options: &RenderOptions,
) -> Result<(ExecutionContext, Client), CliError> {
    let input = ResolutionInput {
        profile: flags.profile.clone(),
        namespace: flags.namespace.clone(),
        is_dry_run: flags.is_dry_run,
    };
    let http_client = ags_runtime::runtime::dispatch::http::build_http_client(flags.timeout)?;
    let context = ExecutionContext::resolve(&input, &http_client).await?;
    if !context.access_token_warnings.is_empty() {
        let mut warning_frontend = crate::frontend::frontend_for_surface(
            frontend_context.pre_surface_backend(),
            render_options.clone(),
        )?;
        for warning in &context.access_token_warnings {
            warning_frontend.render_warning(warning, None, None);
        }
        let _ = warning_frontend.finish();
    }
    Ok((context, http_client))
}
