//! Handler for the `ags describe` command.
//!
//! Produces a JSON introspection envelope for each describable entity
//! (root catalogue, service, resource, method) or an error envelope when a
//! named entity cannot be resolved. Both outcomes route through
//! `Frontend::render` via `CommandOutput::Describe`.

mod envelope;

use serde_json::Value;

use crate::errors::CliError;
use crate::invocation::builder;
use crate::invocation::clap_helpers;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use ags_protocol::output::CommandOutput;
use ags_protocol::workflow::WorkflowId;
use ags_runtime::catalogue::Catalogue;
use ags_runtime::runtime::workflows::registry;

/// Result of resolving a describe query.
/// `Ok(value)` — a success envelope to render.
/// `Err(value)` — an error envelope; render and exit 1.
type Outcome = Result<Value, Value>;

/// Handle `ags describe [service] [resource] [method]`.
pub(crate) fn handle_describe(
    args: &[String],
    _flags: &GlobalFlags,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let mut command = builder::build_describe_command();
    let argv = clap_helpers::build_argv("describe", args);

    let matches = match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
        Ok(m) => m,
        Err(error) => return clap_helpers::outcome_from_clap_error(error),
    };

    let service = matches.get_one::<String>("service").map(|s| s.as_str());
    let resource = matches.get_one::<String>("resource").map(|s| s.as_str());
    let method = matches.get_one::<String>("method").map(|s| s.as_str());

    let outcome = match (service, resource, method) {
        (None, _, _) => Ok(describe_root()),
        (Some("workflow"), None, _) => Ok(describe_workflow_catalogue()),
        (Some("workflow"), Some(id), None) => describe_workflow_detail(id),
        (Some("workflow"), Some(id), Some(extra)) => Err(invalid_workflow_path_error(id, extra)),
        (Some(s), None, _) => describe_service(s),
        (Some(s), Some(r), None) => describe_resource(s, r),
        (Some(s), Some(r), Some(m)) => describe_method(s, r, m),
    };

    let (envelope_value, is_error) = match outcome {
        Ok(value) => (value, false),
        Err(value) => (value, true),
    };

    frontend.render(&CommandOutput::Describe(
        ags_protocol::output::DescribeOutput {
            envelope: envelope_value,
        },
    ))?;

    if is_error {
        return Ok(InvocationOutcome::Exit(1));
    }
    Ok(InvocationOutcome::Complete)
}

/// Build the top-level catalogue describe envelope listing every service.
fn describe_root() -> Value {
    let mut children: Vec<envelope::CatalogueChild> = Vec::new();

    for service in Catalogue::service_ids() {
        let display = Catalogue::display_name_or_panic(service);
        let desc = Catalogue::service_description(service);
        children.push(envelope::CatalogueChild {
            node_type: "service",
            name: display.to_string(),
            path: vec![display.to_string()],
            summary: desc.to_string(),
        });
    }

    children.push(envelope::CatalogueChild {
        node_type: "workflow-catalogue",
        name: "workflow".to_string(),
        path: vec!["workflow".to_string()],
        summary: "Registered multi-step workflows".to_string(),
    });

    // Sort alphabetically by name, consistent with every other catalogue
    // builder in this module. Services already arrive in declaration order
    // (alphabetical) and `workflow` sorts last, so this is order-preserving
    // today — it makes the contract explicit rather than relying on the
    // manifest's declaration order.
    children.sort_by(|a, b| a.name.cmp(&b.name));

    envelope::to_value(&envelope::DescribeEnvelope {
        schema_version: "1",
        kind: envelope::DescribeKind::Catalogue,
        path: vec![],
        generated_by: envelope::generator_info(),
        data: envelope::CatalogueData {
            node_type: "root",
            name: "ags".to_string(),
            summary: "AccelByte Gaming Services CLI".to_string(),
            children,
        },
    })
}

/// Build the service describe envelope listing the resources under one service.
fn describe_service(service_arg: &str) -> Outcome {
    let internal = resolve_service(service_arg)?;
    let display = Catalogue::display_name(internal).unwrap_or(internal);
    let (definition, _) =
        Catalogue::load_uncached(internal).map_err(|e| clierror_envelope(CliError::from(e)))?;

    let mut children: Vec<envelope::CatalogueChild> = definition
        .resources
        .iter()
        .map(|resource| {
            let summary = Catalogue::resource_description(internal, &resource.name)
                .unwrap_or("")
                .to_string();
            envelope::CatalogueChild {
                node_type: "resource",
                name: resource.name.clone(),
                path: vec![display.to_string(), resource.name.clone()],
                summary,
            }
        })
        .collect();
    children.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(envelope::to_value(&envelope::DescribeEnvelope {
        schema_version: "1",
        kind: envelope::DescribeKind::Catalogue,
        path: vec![display.to_string()],
        generated_by: envelope::generator_info(),
        data: envelope::CatalogueData {
            node_type: "service",
            name: display.to_string(),
            summary: Catalogue::service_description(internal).to_string(),
            children,
        },
    }))
}

/// Build the resource describe envelope listing the methods under one resource.
fn describe_resource(service_arg: &str, resource_arg: &str) -> Outcome {
    let internal = resolve_service(service_arg)?;
    let display = Catalogue::display_name(internal).unwrap_or(internal);
    let (definition, _) =
        Catalogue::load_uncached(internal).map_err(|e| clierror_envelope(CliError::from(e)))?;

    let resource = definition
        .resources
        .iter()
        .find(|resource| resource.name == resource_arg)
        .ok_or_else(|| {
            let candidates: Vec<&str> = definition
                .resources
                .iter()
                .map(|resource| resource.name.as_str())
                .collect();
            let suggestions = envelope::find_suggestions(resource_arg, &candidates);
            error_envelope(
                vec![display.to_string(), resource_arg.to_string()],
                "unknown_resource",
                format!("Unknown resource '{resource_arg}' in service '{display}'"),
                suggestions,
            )
        })?;

    let summary = Catalogue::resource_description(internal, &resource.name)
        .unwrap_or("")
        .to_string();

    // Describe the default contract per method. Methods with no default scope (ambiguous
    // multi-scope, no admin) are intentionally omitted — they require --api-scope to invoke
    // and are invisible here. A future requires_scope_flag field in the envelope would surface
    // them.
    let default_ops: Vec<&ags_protocol::catalogue::OperationSchema> = resource
        .methods
        .iter()
        .filter_map(|m| m.default_operation())
        .collect();
    let mut children: Vec<envelope::CatalogueChild> = default_ops
        .iter()
        .map(|operation| envelope::CatalogueChild {
            node_type: "method",
            name: operation.name.clone(),
            path: vec![
                display.to_string(),
                resource.name.clone(),
                operation.name.clone(),
            ],
            summary: operation.summary.clone(),
        })
        .collect();
    children.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(envelope::to_value(&envelope::DescribeEnvelope {
        schema_version: "1",
        kind: envelope::DescribeKind::Catalogue,
        path: vec![display.to_string(), resource.name.clone()],
        generated_by: envelope::generator_info(),
        data: envelope::CatalogueData {
            node_type: "resource",
            name: resource.name.clone(),
            summary,
            children,
        },
    }))
}

/// Build the method describe envelope: the scope-and-version contract matrix for one method.
fn describe_method(service_arg: &str, resource_arg: &str, method_arg: &str) -> Outcome {
    let internal = resolve_service(service_arg)?;
    let display = Catalogue::display_name(internal).unwrap_or(internal);
    let (definition, _) =
        Catalogue::load_uncached(internal).map_err(|e| clierror_envelope(CliError::from(e)))?;

    let resource = definition
        .resources
        .iter()
        .find(|resource| resource.name == resource_arg)
        .ok_or_else(|| {
            let candidates: Vec<&str> = definition
                .resources
                .iter()
                .map(|resource| resource.name.as_str())
                .collect();
            let suggestions = envelope::find_suggestions(resource_arg, &candidates);
            error_envelope(
                vec![display.to_string(), resource_arg.to_string()],
                "unknown_resource",
                format!("Unknown resource '{resource_arg}' in service '{display}'"),
                suggestions,
            )
        })?;

    // Expose the full scope/version contract matrix for the method. Deprecated
    // contracts are excluded upstream by the parser, so every entry here is
    // callable. Consumers pick a specific contract via --api-scope/--api-version.
    let method = resource
        .methods
        .iter()
        .find(|candidate| candidate.name == method_arg)
        .ok_or_else(|| {
            let candidates: Vec<&str> = resource
                .methods
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect();
            let suggestions = envelope::find_suggestions(method_arg, &candidates);
            error_envelope(
                vec![
                    display.to_string(),
                    resource_arg.to_string(),
                    method_arg.to_string(),
                ],
                "unknown_method",
                format!(
                    "Unknown method '{method_arg}' in resource '{resource_arg}' of service '{display}'"
                ),
                suggestions,
            )
        })?;

    let matrix = envelope::build_method_matrix(display, &resource.name, method);

    Ok(envelope::to_value(&envelope::DescribeEnvelope {
        schema_version: "1",
        kind: envelope::DescribeKind::Command,
        path: vec![
            display.to_string(),
            resource.name.clone(),
            method.name.clone(),
        ],
        generated_by: envelope::generator_info(),
        data: matrix,
    }))
}

/// `describe workflow` — the registered-workflow catalogue.
fn describe_workflow_catalogue() -> Value {
    let mut children: Vec<envelope::CatalogueChild> = registry()
        .entries()
        .into_iter()
        .map(|(id, name)| envelope::CatalogueChild {
            node_type: "workflow",
            name: id.clone(),
            path: vec!["workflow".to_string(), id],
            summary: name,
        })
        .collect();
    // `entries()` yields workflows in BTreeMap key order (ascending by id, which
    // equals our `name`), so this sort is currently a no-op — but it documents
    // the alphabetical catalogue contract and guards it if `entries()` is ever
    // changed to return a different order.
    children.sort_by(|a, b| a.name.cmp(&b.name));

    envelope::to_value(&envelope::DescribeEnvelope {
        schema_version: "1",
        kind: envelope::DescribeKind::Catalogue,
        path: vec!["workflow".to_string()],
        generated_by: envelope::generator_info(),
        data: envelope::CatalogueData {
            node_type: "workflow-catalogue",
            name: "workflow".to_string(),
            summary: "Registered multi-step workflows".to_string(),
            children,
        },
    })
}

/// `describe workflow <id> <extra>` — workflows have no sub-levels. The
/// three-slot describe grammar parses the extra token, so this returns a JSON
/// error envelope (not a Clap usage error), keeping describe JSON-only. The
/// offending path is echoed back, like describe's other error envelopes.
fn invalid_workflow_path_error(id: &str, extra: &str) -> Value {
    envelope::to_value(&envelope::DescribeEnvelope {
        schema_version: "1",
        kind: envelope::DescribeKind::Error,
        path: vec!["workflow".to_string(), id.to_string(), extra.to_string()],
        generated_by: envelope::generator_info(),
        data: envelope::DescribeErrorData {
            code: "invalid_workflow_path".to_string(),
            message: format!(
                "Workflows have no sub-levels; '{extra}' is not valid under 'workflow {id}'. Use 'ags describe workflow {id}'."
            ),
            suggestions: vec![],
        },
    })
}

/// `describe workflow <id>` — one workflow's metadata, inputs, and steps.
fn describe_workflow_detail(id: &str) -> Outcome {
    let workflow_registry = registry();
    match workflow_registry.resolve(&WorkflowId::new(id)) {
        Some(workflow) => Ok(envelope::to_value(&envelope::DescribeEnvelope {
            schema_version: "1",
            kind: envelope::DescribeKind::Workflow,
            path: vec!["workflow".to_string(), id.to_string()],
            generated_by: envelope::generator_info(),
            data: envelope::build_workflow_detail(workflow.definition()),
        })),
        None => {
            // Registry ids borrow from the `'static` registry, so collect the
            // `&str` slice directly — no intermediate owned `Vec<String>`.
            let id_refs: Vec<&str> = workflow_registry.ids().map(|i| i.as_str()).collect();
            let suggestions = envelope::find_suggestions(id, &id_refs);
            Err(envelope::to_value(&envelope::DescribeEnvelope {
                schema_version: "1",
                kind: envelope::DescribeKind::Error,
                path: vec!["workflow".to_string(), id.to_string()],
                generated_by: envelope::generator_info(),
                data: envelope::DescribeErrorData {
                    code: "unknown_workflow".to_string(),
                    message: format!("Unknown workflow: '{id}'"),
                    suggestions,
                },
            }))
        }
    }
}

// ── Helpers ──

/// Resolve a user-supplied service name (display or internal) to its internal id, or build an unknown-service error envelope.
fn resolve_service(service_arg: &str) -> Result<&'static str, Value> {
    Catalogue::internal_name(service_arg).ok_or_else(|| {
        let candidates: Vec<&str> = Catalogue::service_ids()
            .map(Catalogue::display_name_or_panic)
            .collect();
        let suggestions = envelope::find_suggestions(service_arg, &candidates);
        error_envelope(
            vec![service_arg.to_string()],
            "unknown_service",
            format!("Unknown service: '{service_arg}'"),
            suggestions,
        )
    })
}

/// Build an error envelope `Value` to return as the command output.
fn error_envelope(
    path: Vec<String>,
    code: &str,
    message: String,
    suggestions: Vec<String>,
) -> Value {
    envelope::to_value(&envelope::DescribeEnvelope {
        schema_version: "1",
        kind: envelope::DescribeKind::Error,
        path,
        generated_by: envelope::generator_info(),
        data: envelope::DescribeErrorData {
            code: code.to_string(),
            message,
            suggestions,
        },
    })
}

/// Convert a fatal `CliError` (e.g. catalogue load failure) into an error envelope
/// so `ags describe` always emits machine-readable output even in the failure case.
/// Preserves ErrorView metadata (reason/detail/suggestion/tip) so consumers keep
/// the same guidance they would receive from a service command.
fn clierror_envelope(err: CliError) -> Value {
    let view = err.view();
    let mut value = error_envelope(vec![], "internal", view.message, vec![]);
    if let Some(data) = value.get_mut("data").and_then(|v| v.as_object_mut()) {
        if let Some(reason) = view.reason {
            data.insert("reason".to_string(), Value::String(reason));
        }
        if let Some(detail) = view.detail {
            data.insert("detail".to_string(), Value::String(detail));
        }
        if let Some(suggestion) = view.suggestion {
            data.insert("suggestion".to_string(), Value::String(suggestion));
        }
        if let Some(tip) = view.tip {
            data.insert("tip".to_string(), Value::String(tip));
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_describe_root_includes_workflow_child() {
        let v = describe_root();
        let children = v["data"]["children"].as_array().unwrap();
        assert!(
            children
                .iter()
                .any(|c| c["name"] == "workflow" && c["node_type"] == "workflow-catalogue"),
            "root catalogue must list a workflow node"
        );
    }

    #[test]
    fn test_describe_workflow_catalogue_lists_workflows() {
        let v = describe_workflow_catalogue();
        assert_eq!(v["kind"], "catalogue");
        assert_eq!(v["path"], serde_json::json!(["workflow"]));
        assert_eq!(v["data"]["node_type"], "workflow-catalogue");
        assert_eq!(v["data"]["name"], "workflow");
        let children = v["data"]["children"].as_array().unwrap();
        assert!(children
            .iter()
            .any(|c| { c["name"] == "competitive-multiplayer" && c["node_type"] == "workflow" }));
    }

    #[test]
    fn test_describe_workflow_detail_known_id() {
        let v = describe_workflow_detail("competitive-multiplayer").unwrap();
        assert_eq!(v["kind"], "workflow");
        assert_eq!(
            v["path"],
            serde_json::json!(["workflow", "competitive-multiplayer"])
        );
        assert_eq!(v["data"]["id"], "competitive-multiplayer");
        assert!(v["data"]["inputs"].as_array().is_some());
    }

    #[test]
    fn test_describe_workflow_detail_unknown_id_suggests() {
        let v = describe_workflow_detail("competitive-multiplayerx").unwrap_err();
        assert_eq!(v["kind"], "error");
        assert_eq!(v["data"]["code"], "unknown_workflow");
        let suggestions = v["data"]["suggestions"].as_array().unwrap();
        assert!(
            suggestions.iter().any(|s| s == "competitive-multiplayer"),
            "substring match should suggest the real id: {suggestions:?}"
        );
    }

    #[test]
    fn test_invalid_workflow_path_error_echoes_path() {
        let v = invalid_workflow_path_error("competitive-multiplayer", "x");
        assert_eq!(v["kind"], "error");
        assert_eq!(v["data"]["code"], "invalid_workflow_path");
        assert_eq!(
            v["path"],
            serde_json::json!(["workflow", "competitive-multiplayer", "x"])
        );
    }
}
