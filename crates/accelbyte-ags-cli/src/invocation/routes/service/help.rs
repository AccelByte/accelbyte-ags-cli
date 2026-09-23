use crate::errors::CliError;
use crate::invocation::flags::LeafSelectors;
use crate::invocation::handlers::extend::service_shims::ShimPresentation;
use ags_protocol::catalogue::{OperationSchema, ServiceSchema};
use ags_runtime::catalogue::Catalogue;

/// Display contextual help for a service, resource, or method subcommand.
///
/// When `shim` is `Some`, the caller arrived through an `extend` migration
/// shortcut and the method-level help page is rewritten to show the
/// shortcut's own identity (summary, display address, canonical block)
/// instead of the canonical CSM page.
pub(super) fn print_service_help(
    mut service_command: clap::Command,
    service_name: &str,
    service_args: &[String],
    shim: Option<&ShimPresentation>,
    service_schema: &ServiceSchema,
    selectors: &LeafSelectors,
) -> Result<(), CliError> {
    if !service_args.is_empty() && service_args[0] != "--help" && service_args[0] != "-h" {
        let resource_name = &service_args[0];
        if let Some(resource_subcommand) = service_command.find_subcommand_mut(resource_name) {
            if service_args.len() >= 2 && service_args[1] != "--help" && service_args[1] != "-h" {
                let method_name = &service_args[1];
                if let Some(method_subcommand) =
                    resource_subcommand.find_subcommand_mut(method_name)
                {
                    if let Some(shim) = shim {
                        apply_shim_overrides(
                            method_subcommand,
                            shim,
                            service_schema,
                            resource_name,
                            method_name,
                            selectors,
                        );
                    }
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

/// Apply shim-specific overrides to a method subcommand before printing help.
///
/// Three passages are replaced; everything else (options, schema, contract
/// block) is kept identical to the canonical page:
///
/// 1. **Header** (`long_about`): the shim summary is prepended as the first
///    line, and a canonical-command block is inserted before the contract
///    block.
/// 2. **Usage: line** (`bin_name`): uses the display address.
/// 3. **Example: block** (`after_help`): rebuilt from scratch with the
///    display address as the command prefix, using the same selector-resolved
///    operation that `build_service_command_tree` chose for the page's options
///    and contract block.
fn apply_shim_overrides(
    cmd: &mut clap::Command,
    shim: &ShimPresentation,
    service_schema: &ServiceSchema,
    resource_name: &str,
    method_name: &str,
    selectors: &LeafSelectors,
) {
    // --- 1. Override long_about (header) ---
    let existing_long_about = cmd
        .get_long_about()
        .map(|s| s.to_string())
        .unwrap_or_default();

    let canonical_block = format!(
        "Canonical command: {}\n\
         This shortcut forwards to it. Both addresses are supported.",
        shim.canonical_address,
    );

    // Insert the summary as the first line and the canonical block before
    // the "Default contract:" block. The contract block is always the last
    // structured passage in long_about (placed there by `clap_tree.rs`).
    let new_long_about = if let Some(split_pos) = existing_long_about.find("\n\nDefault contract:")
    {
        let (before_contract, contract_and_rest) = existing_long_about.split_at(split_pos);
        format!(
            "{}\n\n{}\n\n{}{}",
            shim.summary, before_contract, canonical_block, contract_and_rest,
        )
    } else {
        // Fallback: no contract block found (shouldn't happen in practice).
        format!(
            "{}\n\n{}\n\n{}",
            shim.summary, existing_long_about, canonical_block,
        )
    };

    // --- 2. Apply bin_name, long_about, and (conditionally) after_help ---
    // `Command` setters take `self` by value; take the value out once, chain
    // the unconditional setters, conditionally append `after_help`, and write
    // back once.
    let mut taken = std::mem::replace(cmd, clap::Command::new("__placeholder__"));
    taken = taken
        .bin_name(&shim.display_address)
        .long_about(new_long_about);

    // --- 3. Rebuild the Example: block with the display address prefix ---
    let operation =
        resolve_operation_for_help(service_schema, resource_name, method_name, selectors);
    if let Some(operation) = operation {
        let after_help = super::clap_tree::build_operation_example_with_prefix(
            &shim.display_address,
            &operation.parameters,
            operation.request_body.is_some(),
        );
        if !after_help.is_empty() {
            taken = taken.after_help(after_help);
        }
    }

    // --- 4. Surface the shim-only --wait flags on wait-capable shortcuts ---
    // The canonical CSM operation has no wait flags; they are consumed by the
    // shim scanner before dispatch. Adding them here lists them in the Options
    // section of the shortcut's help page alongside --app/--json/--api-version.
    if shim.wait_capable {
        taken = crate::invocation::handlers::extend::service_shims::add_wait_flag_args(taken);
    }

    *cmd = taken;
}

/// Resolve the operation for help-page rendering.
///
/// Mirrors `build_service_command_tree`: tries the selector-aware `resolve()`
/// path first so the Example block tracks whichever `--api-scope` /
/// `--api-version` the user selected. Falls back to
/// `fallback_default_contract` when resolution fails (e.g. the selectors are
/// absent or invalid) so `--help` never breaks entirely.
fn resolve_operation_for_help(
    service_schema: &ServiceSchema,
    resource_name: &str,
    method_name: &str,
    selectors: &LeafSelectors,
) -> Option<OperationSchema> {
    let resource = service_schema
        .resources
        .iter()
        .find(|r| r.name == resource_name)?;

    // Accept the canonical name or a former name (same matching as parser.rs).
    let method = resource.methods.iter().find(|m| {
        m.name == method_name
            || ags_runtime::catalogue::former_method_names(
                &service_schema.name,
                &resource.name,
                &m.name,
            )
            .contains(&method_name)
    })?;

    let command_path = format!(
        "ags {} {} {}",
        service_schema.name, resource_name, method_name
    );
    if let Ok(resolved) = crate::invocation::resolve::resolve(
        &command_path,
        method,
        selectors.api_scope.as_deref(),
        selectors.api_version.as_deref(),
    ) {
        return Some(resolved.operation);
    }

    super::clap_tree::fallback_default_contract(method).map(|(op, _, _)| op.clone())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invocation::flags::LeafSelectors;
    use ags_protocol::catalogue::{
        ApiVersion, HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema,
        ParameterLocation, ParameterSchema, ScopeEntry, ValueType,
    };

    /// Build a minimal `OperationSchema` with the given parameters.
    fn op_with_params(scope: &str, version: u32, params: Vec<(&str, bool)>) -> OperationSchema {
        OperationSchema {
            id: OperationId::new(format!("svc/{scope}/res/v{version}/get")),
            name: "create".to_string(),
            summary: "".to_string(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: format!("/svc/v{version}/{scope}/res"),
            parameters: params
                .into_iter()
                .map(|(name, required)| ParameterSchema {
                    name: name.to_string(),
                    description: None,
                    required,
                    value_type: ValueType::String,
                    location: ParameterLocation::Query,
                    is_file: false,
                    default: None,
                })
                .collect(),
            request_body: None,
            response: None,
            permissions: vec![],
            scope: scope.to_string(),
            api_version: ApiVersion(version),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// Build a synthetic `ServiceSchema` with a single method that has two
    /// versions whose parameter sets differ — the v2 operation carries
    /// `alpha` while v5 carries `alpha` and `beta`.
    fn synthetic_multi_version_schema() -> ServiceSchema {
        ServiceSchema {
            name: "test-svc".to_string(),
            description: "Test service".to_string(),
            resources: vec![ags_protocol::catalogue::ResourceSchema {
                name: "widgets".to_string(),
                description: "Widgets".to_string(),
                methods: vec![MethodSchema {
                    name: "create".to_string(),
                    summary: "Create a widget".to_string(),
                    default_scope: Some("admin".to_string()),
                    scopes: vec![ScopeEntry {
                        scope: "admin".to_string(),
                        default_version: ApiVersion(5),
                        contracts: vec![
                            op_with_params("admin", 2, vec![("alpha", true)]),
                            op_with_params("admin", 5, vec![("alpha", true), ("beta", false)]),
                        ],
                    }],
                }],
            }],
        }
    }

    /// With no selectors, `resolve_operation_for_help` returns the default
    /// contract (v5), which has two parameters.
    #[test]
    fn resolve_operation_defaults_to_latest_version() {
        let schema = synthetic_multi_version_schema();
        let sel = LeafSelectors::default();
        let op = resolve_operation_for_help(&schema, "widgets", "create", &sel).unwrap();
        assert_eq!(
            op.api_version,
            ApiVersion(5),
            "default resolution must pick v5"
        );
        assert_eq!(op.parameters.len(), 2, "v5 has two parameters");
    }

    /// When `--api-version v2` is selected, `resolve_operation_for_help`
    /// returns the v2 operation (one parameter), not the default v5.
    #[test]
    fn resolve_operation_honours_api_version_selector() {
        let schema = synthetic_multi_version_schema();
        let sel = LeafSelectors {
            api_scope: None,
            api_version: Some("v2".to_string()),
        };
        let op = resolve_operation_for_help(&schema, "widgets", "create", &sel).unwrap();
        assert_eq!(op.api_version, ApiVersion(2), "selector must pick v2");
        assert_eq!(op.parameters.len(), 1, "v2 has one parameter");
    }

    /// An invalid selector falls back to the default contract rather than
    /// returning `None`, so the Example block is always rendered.
    #[test]
    fn resolve_operation_falls_back_on_invalid_selector() {
        let schema = synthetic_multi_version_schema();
        let sel = LeafSelectors {
            api_scope: None,
            api_version: Some("v99".to_string()),
        };
        let op = resolve_operation_for_help(&schema, "widgets", "create", &sel).unwrap();
        assert_eq!(
            op.api_version,
            ApiVersion(5),
            "invalid selector must fall back to the default contract"
        );
    }
}
