use crate::catalogue::Catalogue;
use crate::errors::CliError;
use crate::protocol::catalogue::OperationSchema;

/// Display contextual help for a service, resource, or method subcommand.
pub(super) fn print_service_help(
    mut service_command: clap::Command,
    service_name: &str,
    service_args: &[String],
) -> Result<(), CliError> {
    if !service_args.is_empty() && service_args[0] != "--help" && service_args[0] != "-h" {
        let resource_name = &service_args[0];
        if let Some(resource_subcommand) = service_command.find_subcommand_mut(resource_name) {
            if service_args.len() >= 2 && service_args[1] != "--help" && service_args[1] != "-h" {
                let method_name = &service_args[1];
                if let Some(method_subcommand) =
                    resource_subcommand.find_subcommand_mut(method_name)
                {
                    let _ = method_subcommand.print_long_help();
                    return Ok(());
                }

                let resource_args: Vec<&str> = service_args
                    .iter()
                    .skip(1)
                    .map(|arg| arg.as_str())
                    .collect();
                if let Err(error) = resource_subcommand.try_get_matches_from_mut(
                    std::iter::once(resource_name.as_str()).chain(resource_args),
                ) {
                    return Err(CliError::Usage {
                        message: crate::invocation::clap_helpers::strip_clap_prefix(
                            &error.to_string(),
                        ),
                        metadata: None,
                    });
                }
            }
            let _ = resource_subcommand.print_help();
            return Ok(());
        }

        let args: Vec<&str> = std::iter::once(service_name)
            .chain(service_args.iter().map(|arg| arg.as_str()))
            .collect();
        let error = service_command
            .try_get_matches_from_mut(args)
            .err()
            .map(|clap_error| {
                crate::invocation::clap_helpers::strip_clap_prefix(&clap_error.to_string())
            })
            .unwrap_or_else(|| {
                format!("Unknown resource '{resource_name}' in service '{service_name}'")
            });
        return Err(CliError::Usage {
            message: error,
            metadata: None,
        });
    }

    let _ = service_command.print_help();
    Ok(())
}

/// Build a JSON request body template for the given operation.
pub(super) fn build_skeleton_output(
    operation: &OperationSchema,
) -> Result<serde_json::Value, CliError> {
    if operation.request_body.is_none() {
        return Err(CliError::Usage {
            message: format!("Operation '{}' has no request body", operation.name),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Only operations that accept --json have a skeleton to generate",
            ))),
        });
    }

    Ok(Catalogue::build_body_skeleton(operation))
}
