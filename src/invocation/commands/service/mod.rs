//! Handle dynamic service commands: parse args, resolve auth, dispatch API calls.

pub(crate) mod clap_tree;
mod dispatch;
mod help;
mod parser;
mod request;

use crate::errors::CliError;
use crate::invocation::flags;
use crate::invocation::InvocationOutcome;
use crate::runtime::dispatch::http::build_http_client;

/// Dispatch a dynamic service command by loading its spec, parsing args, and executing the API call
pub(crate) async fn handle_service(
    service_arg: &str,
    service_args: &[String],
    flags: &flags::GlobalFlags,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let (selectors, stripped_args) = flags::pre_scan_leaf_selectors(service_args)?;
    let parsed =
        match parser::parse_service_args(service_arg, &stripped_args, &selectors, flags, frontend)?
        {
            parser::ParseServiceOutcome::Continue(parsed) => parsed,
            parser::ParseServiceOutcome::Exit(code) => return Ok(InvocationOutcome::Exit(code)),
            parser::ParseServiceOutcome::Complete => return Ok(InvocationOutcome::Complete),
        };

    let input = crate::runtime::execution::ResolutionInput {
        profile: flags.profile.clone(),
        namespace: flags.namespace.clone(),
        is_dry_run: flags.is_dry_run,
    };
    let http_client = build_http_client(flags.timeout)?;
    let context =
        crate::runtime::execution::ExecutionContext::resolve(&input, &http_client).await?;
    for warning in &context.access_token_warnings {
        frontend.render_warning(warning, None, None);
    }

    let mut runtime = crate::runtime::Runtime::from_reqwest(context.clone(), http_client.clone());

    let output =
        match dispatch::dispatch_command(&mut runtime, &parsed, flags, &context, frontend).await? {
            Some(output) => output,
            None => return Ok(InvocationOutcome::Complete),
        };

    frontend.render(&output, &crate::frontend::RenderOptions::from(flags))?;
    Ok(InvocationOutcome::Complete)
}
