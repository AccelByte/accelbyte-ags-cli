use super::{help, request};
use crate::errors::CliError;
use crate::invocation::builder;
use crate::invocation::context::FrontendContext;
use crate::invocation::flags;
use ags_protocol::catalogue::{ResourceSchema, ServiceSchema};
use ags_protocol::event::{ProgressEvent, ProgressSink};
use ags_runtime::catalogue::{Catalogue, ServiceId, SpecSource};

/// Result of parsing and validating CLI args before auth/dispatch.
pub(super) struct ParsedServiceCommand {
    pub(super) service_id: ServiceId,
    pub(super) command_request: ags_protocol::request::CommandRequest,
    pub(super) spec_source: SpecSource,
    /// The resolved `ServiceSchema`, carried so `route_service` can
    /// synthesise the 1-step workflow without re-loading the catalogue
    /// (which would also conflict with `compile_workflow`'s `&mut Catalogue`).
    pub(super) service_schema: ServiceSchema,
    /// The clean CLI command path, e.g. `iam users create` (service display
    /// name + resource + method). Shown as the fullscreen frame title instead
    /// of the synthesised workflow's slash-path operation id.
    pub(super) command_path: String,
}

/// `Continue` carries `Box<ParsedServiceCommand>` so the enum stays small —
/// the parsed-command struct is far larger than the other variants.
pub(super) enum ParseServiceOutcome {
    Continue(Box<ParsedServiceCommand>),
    Complete,
    Exit(i32),
}

/// Parse and validate CLI args, load spec, and build a `CommandRequest`.
///
/// Runs entirely pre-surface: where it must render (cold-cache spec-loading
/// progress, `--skeleton` output) it constructs a fresh pre-surface frontend
/// itself, so a `--ui=fullscreen` command never acquires the terminal here.
pub(super) fn parse_service_args(
    service_arg: &str,
    service_args: &[String],
    selectors: &flags::LeafSelectors,
    flags: &flags::GlobalFlags,
    frontend_context: &FrontendContext,
    render_options: crate::frontend::RenderOptions,
    shim_presentation: Option<
        &crate::invocation::handlers::extend::service_shims::ShimPresentation,
    >,
) -> Result<ParseServiceOutcome, CliError> {
    let protocol_output_format = frontend_context.protocol_output_format();
    let service_id = Catalogue::find_id(service_arg).ok_or_else(|| {
        let services = Catalogue::service_ids()
            .map(Catalogue::display_name_or_panic)
            .map(|service| format!("  {service}"))
            .collect::<Vec<_>>()
            .join("\n");
        CliError::Usage {
            message: format!("Unknown service: '{service_arg}'\n\nValid services:\n{services}"),
            metadata: None,
        }
    })?;

    let internal = service_id.as_str();

    let cache_exists = Catalogue::has_cached(internal);
    let (service_schema, spec_source) = {
        // Cold-cache spec-loading progress stays on the pre-surface path —
        // intentionally not on the owned `progress_frontend`. A fresh
        // pre-surface frontend backs the spinner (noop for JSON).
        if cache_exists {
            Catalogue::load_uncached(internal)?
        } else {
            let mut progress = crate::frontend::frontend_for_surface(
                frontend_context.pre_surface_backend(),
                render_options.clone(),
            )?;
            let mut sink = crate::frontend::FrontendSink::new(progress.as_mut());
            sink.on_event(ProgressEvent::Started {
                message: "Preparing specs...".to_string(),
            });
            let result = Catalogue::load_uncached(internal)?;
            sink.on_event(ProgressEvent::Finished);
            let _ = progress.finish();
            result
        }
    };

    let service_command = builder::build_service_command_tree(&service_schema, selectors);

    let has_help_flag = service_args
        .iter()
        .any(|arg| arg == "--help" || arg == "-h");
    let positional_args: Vec<_> = service_args
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    let needs_help = !flags.is_skeleton
        && (service_args.is_empty() || has_help_flag || positional_args.len() <= 1);

    if needs_help {
        help::print_service_help(
            service_command,
            internal,
            service_args,
            shim_presentation,
            &service_schema,
            selectors,
        )?;
        if has_help_flag {
            return Ok(ParseServiceOutcome::Complete);
        }
        return Ok(ParseServiceOutcome::Exit(1));
    }

    if flags.is_skeleton {
        return handle_skeleton_outcome(
            &service_schema,
            internal,
            &positional_args,
            selectors,
            frontend_context,
            render_options,
        );
    }

    early_resolve_selected_method(&service_schema, internal, &positional_args, selectors)?;

    let namespace_resolution = ags_runtime::runtime::execution::resolve_namespace(
        flags.namespace.as_deref(),
        flags.profile.as_deref(),
    );
    let effective_args = inject_namespace_if_needed(
        service_args,
        &namespace_resolution,
        &service_schema,
        internal,
        &positional_args,
        selectors,
    )?;

    let display_name = Catalogue::display_name(internal).unwrap_or(internal);
    let matches = service_command
        .try_get_matches_from(
            std::iter::once(display_name).chain(effective_args.iter().map(|arg| arg.as_str())),
        )
        .map_err(|error| CliError::Usage {
            message: crate::invocation::clap_helpers::strip_clap_prefix(&error.to_string()),
            metadata: None,
        })?;

    let (resource_name, resource_matches) =
        matches.subcommand().ok_or_else(|| CliError::Usage {
            message: "No resource specified".to_string(),
            metadata: None,
        })?;

    let (method_name, method_matches) =
        resource_matches
            .subcommand()
            .ok_or_else(|| CliError::Usage {
                message: "No method specified".to_string(),
                metadata: None,
            })?;

    let resource_schema = service_schema
        .resources
        .iter()
        .find(|resource| resource.name == resource_name)
        .ok_or_else(|| CliError::Usage {
            message: format!("Unknown resource: {resource_name}"),
            metadata: None,
        })?;

    let resolved = resolve_method(internal, resource_schema, method_name, selectors)?;
    let body = match request::resolve_request_body(
        &resolved.operation,
        method_matches,
        flags.is_no_input,
    )? {
        request::BodyResolution::Explicit(value) => Some(value),
        // A missing body is no longer pre-gathered here: the workflow
        // executor's `gather_workflow_inputs` collects the missing body
        // fields per-input (line-by-line on stdin in human mode).
        request::BodyResolution::NeedsGathering => None,
        request::BodyResolution::None => None,
    };
    let command_request = request::build_command_request(
        &resolved.operation,
        method_matches,
        flags,
        protocol_output_format,
        service_id.clone(),
        body,
    )?;

    // The clean CLI command path the user typed (display name + resource +
    // method), e.g. `iam users create` — used for the fullscreen frame title.
    let command_path = format!("{display_name} {resource_name} {method_name}");

    Ok(ParseServiceOutcome::Continue(Box::new(
        ParsedServiceCommand {
            service_id,
            command_request,
            spec_source,
            service_schema,
            command_path,
        },
    )))
}

/// Resolve a method name within a resource, threading `LeafSelectors` through
/// the central `invocation::resolve::resolve` function. Returns a
/// `ResolvedContract` or bubbles the spec-shaped `ResolutionError` up as
/// `CliError` via the existing `From` impl.
fn resolve_method(
    service_name: &str,
    resource: &ResourceSchema,
    method_name: &str,
    selectors: &flags::LeafSelectors,
) -> Result<crate::invocation::resolve::ResolvedContract, CliError> {
    let method = resource
        .methods
        .iter()
        .find(|method| {
            // Accept the canonical name (the usual case — Clap reports the
            // canonical subcommand even when a hidden alias was typed) or a
            // former name on the raw-arg paths (`--skeleton`, early resolve).
            method.name == method_name
                || ags_runtime::catalogue::former_method_names(
                    service_name,
                    &resource.name,
                    &method.name,
                )
                .contains(&method_name)
        })
        .ok_or_else(|| CliError::Usage {
            message: format!("Unknown method: {method_name}"),
            metadata: None,
        })?;
    let command = format!("ags {service_name} {} {method_name}", resource.name);
    crate::invocation::resolve::resolve(
        &command,
        method,
        selectors.api_scope.as_deref(),
        selectors.api_version.as_deref(),
    )
    .map_err(Into::into)
}

/// Handle `--skeleton` by resolving the target operation and rendering its
/// request-body template. Renders through a fresh pre-surface frontend —
/// `render_options` is threaded so `--output` is honoured.
fn handle_skeleton_outcome(
    service_schema: &ServiceSchema,
    service_name: &str,
    positional_args: &[&String],
    selectors: &flags::LeafSelectors,
    frontend_context: &FrontendContext,
    render_options: crate::frontend::RenderOptions,
) -> Result<ParseServiceOutcome, CliError> {
    let resource_name = positional_args
        .first()
        .map(|resource| resource.as_str())
        .ok_or_else(|| CliError::Usage {
            message: "--skeleton requires a full command path: ags <service> <resource> <method> --skeleton".to_string(),
            metadata: None,
        })?;
    let method_name = positional_args
        .get(1)
        .map(|method| method.as_str())
        .ok_or_else(|| CliError::Usage {
            message: "--skeleton requires a full command path: ags <service> <resource> <method> --skeleton".to_string(),
            metadata: None,
        })?;
    let resource_schema = service_schema
        .resources
        .iter()
        .find(|resource| resource.name == resource_name)
        .ok_or_else(|| CliError::Usage {
            message: format!("Unknown resource: {resource_name}"),
            metadata: None,
        })?;
    let resolved = resolve_method(service_name, resource_schema, method_name, selectors)?;
    let skeleton = help::build_skeleton_output(&resolved.operation)?;
    let mut frontend = crate::frontend::frontend_for_surface(
        frontend_context.pre_surface_backend(),
        render_options,
    )?;
    frontend.render(&ags_protocol::output::CommandOutput::Skeleton(
        ags_protocol::output::SkeletonOutput { body: skeleton },
    ))?;
    let _ = frontend.finish();
    Ok(ParseServiceOutcome::Complete)
}

/// Surface scope/version errors before Clap's arg validation fires.
///
/// Without this, `inject_namespace_if_needed` silently swallows a failed
/// resolve (because it needs a fallback for help paths), and Clap then
/// errors with "missing required --namespace" instead of the real
/// `UnsupportedScope` / `UnsupportedVersion` message.
fn early_resolve_selected_method(
    service_schema: &ServiceSchema,
    service_name: &str,
    positional_args: &[&String],
    selectors: &flags::LeafSelectors,
) -> Result<(), CliError> {
    if positional_args.len() >= 2 {
        if let Some(resource_schema) = service_schema
            .resources
            .iter()
            .find(|resource| resource.name == *positional_args[0])
        {
            let method_name = positional_args[1].as_str();
            if resource_schema.methods.iter().any(|method| {
                method.name == method_name
                    || ags_runtime::catalogue::former_method_names(
                        service_name,
                        &resource_schema.name,
                        &method.name,
                    )
                    .contains(&method_name)
            }) {
                resolve_method(service_name, resource_schema, method_name, selectors)?;
            }
        }
    }
    Ok(())
}

/// Auto-inject `--namespace` when the user has a default namespace configured
/// (via flag or environment variable) but did not explicitly pass one. Only injects if the
/// target operation's parameter list actually includes a `namespace` parameter.
fn inject_namespace_if_needed(
    service_args: &[String],
    namespace_resolution: &Option<(String, ags_runtime::runtime::execution::NamespaceSource)>,
    service_schema: &ServiceSchema,
    service_name: &str,
    positional_args: &[&String],
    selectors: &flags::LeafSelectors,
) -> Result<Vec<String>, CliError> {
    let mut effective_args: Vec<String> = service_args.to_vec();

    if let Some((namespace, _source)) = namespace_resolution {
        if !effective_args
            .iter()
            .any(|arg| arg == "--namespace" || arg.starts_with("--namespace="))
        {
            let operation_accepts_namespace = positional_args.len() >= 2
                && service_schema
                    .resources
                    .iter()
                    .find(|resource| resource.name == *positional_args[0])
                    .and_then(|resource| {
                        resolve_method(service_name, resource, positional_args[1], selectors).ok()
                    })
                    .is_some_and(|resolved| {
                        resolved
                            .operation
                            .parameters
                            .iter()
                            .any(|parameter| parameter.name == "namespace")
                    });

            if operation_accepts_namespace {
                effective_args.push("--namespace".to_string());
                effective_args.push(namespace.clone());
            }
        }
    }

    Ok(effective_args)
}
