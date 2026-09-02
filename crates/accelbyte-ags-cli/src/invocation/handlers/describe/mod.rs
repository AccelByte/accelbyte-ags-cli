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
use crate::invocation::handlers::extend::service_shims::{self, SHIMS};
use crate::invocation::InvocationOutcome;
use ags_protocol::output::CommandOutput;
use ags_protocol::workflow::WorkflowId;
use ags_runtime::catalogue::Catalogue;
use ags_runtime::runtime::workflows::registry;

/// Result of resolving a describe query.
/// `Ok(value)` — a success envelope to render.
/// `Err(value)` — an error envelope; render and exit 1.
type Outcome = Result<Value, Value>;

use crate::invocation::routes::ams_upload::UPLOAD_RESOURCE as UPLOAD_COMMAND;
use crate::invocation::routes::service::clap_tree::AMS_SERVICE_NAME;

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
        (Some("extend"), None, _) => Ok(describe_extend()),
        (Some("extend"), Some(sub), None) => describe_extend_command(sub),
        (Some("extend"), Some(sub), Some(extra)) => describe_extend_subgroup_entry(sub, extra),
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
            alias_of: None,
        });
    }

    children.push(envelope::CatalogueChild {
        node_type: "command-group",
        name: "extend".to_string(),
        path: vec!["extend".to_string()],
        summary: "Extend platform tooling".to_string(),
        alias_of: None,
    });

    children.push(envelope::CatalogueChild {
        node_type: "workflow-catalogue",
        name: "workflow".to_string(),
        path: vec!["workflow".to_string()],
        summary: "Registered multi-step workflows".to_string(),
        alias_of: None,
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
            alias_of: None,
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
                alias_of: None,
            }
        })
        .collect();

    if internal == AMS_SERVICE_NAME {
        children.push(envelope::CatalogueChild {
            node_type: "command",
            name: UPLOAD_COMMAND.to_string(),
            path: vec![display.to_string(), UPLOAD_COMMAND.to_string()],
            summary: ams_upload_summary(),
            alias_of: None,
        });
    }
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
            alias_of: None,
        },
    }))
}

/// Build the describe envelope for `ags ams upload`.
///
/// The hand-written command has no OpenAPI operation, so its flags are read
/// off the Clap command itself rather than a schema — one source of truth
/// shared with `--help` and shell completions.
fn describe_ams_upload(display: &str) -> Value {
    let command = builder::build_ams_upload_command();
    let parameters = command
        .get_arguments()
        .filter(|argument| argument.get_long().is_some())
        .map(|argument| envelope::InputParameter {
            name: argument.get_long().unwrap_or_default().to_string(),
            location: "flag".to_string(),
            required: argument.is_required_set(),
            parameter_type: if argument.get_action().takes_values() {
                "string".to_string()
            } else {
                "boolean".to_string()
            },
            description: argument
                .get_help()
                .map(|help| help.to_string())
                .unwrap_or_default(),
            enum_values: {
                let values: Vec<String> = argument
                    .get_possible_values()
                    .iter()
                    .map(|value| value.get_name().to_string())
                    .collect();
                (!values.is_empty()).then_some(values)
            },
        })
        .collect();

    envelope::to_value(&envelope::DescribeEnvelope {
        schema_version: "1",
        kind: envelope::DescribeKind::Command,
        path: vec![display.to_string(), UPLOAD_COMMAND.to_string()],
        generated_by: envelope::generator_info(),
        data: envelope::NativeCommandData {
            command: format!("ags {display} {UPLOAD_COMMAND}"),
            summary: ams_upload_summary(),
            description: command
                .get_long_about()
                .map(|about| about.to_string())
                .unwrap_or_default(),
            parameters,
        },
    })
}

/// One-line summary of `ags ams upload`, taken from the Clap command.
fn ams_upload_summary() -> String {
    builder::build_ams_upload_command()
        .get_about()
        .map(|about| about.to_string())
        .unwrap_or_default()
}

/// Build the resource describe envelope listing the methods under one resource.
fn describe_resource(service_arg: &str, resource_arg: &str) -> Outcome {
    let internal = resolve_service(service_arg)?;
    let display = Catalogue::display_name(internal).unwrap_or(internal);
    if internal == AMS_SERVICE_NAME && resource_arg == UPLOAD_COMMAND {
        return Ok(describe_ams_upload(display));
    }
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
            alias_of: None,
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
            alias_of: None,
        },
    }))
}

/// Build the method describe envelope: the scope-and-version contract matrix for one method.
fn describe_method(service_arg: &str, resource_arg: &str, method_arg: &str) -> Outcome {
    let internal = resolve_service(service_arg)?;
    let display = Catalogue::display_name(internal).unwrap_or(internal);
    // `ams upload` is a leaf command, not a resource, so it has no methods to
    // descend into — say that rather than "unknown resource 'upload'".
    if internal == AMS_SERVICE_NAME && resource_arg == UPLOAD_COMMAND {
        return Err(error_envelope(
            vec![
                display.to_string(),
                UPLOAD_COMMAND.to_string(),
                method_arg.to_string(),
            ],
            "unknown_method",
            format!("'{display} {UPLOAD_COMMAND}' is a command and has no methods"),
            vec![format!("ags describe {display} {UPLOAD_COMMAND}")],
        ));
    }
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
            alias_of: None,
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
            alias_of: None,
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

/// Build the `describe extend` catalogue listing all extend subcommands.
///
/// Canonical children are derived from the real clap tree (non-hidden
/// subcommands that are NOT top-level shim entries) so adding a new
/// extend command requires zero edits here. Top-level shim names are
/// excluded because they are re-added as alias nodes via
/// `shim_alias_children`; without the exclusion each shim would appear
/// twice — once as `node_type: "command"` and once as `node_type: "alias"`.
/// Parent groups (`app-ui`, `remote-debug`) are NOT excluded: they are
/// real navigable command groups, not shims.
fn describe_extend() -> Value {
    let cmd = builder::build_extend_command();
    let shim_top_level: std::collections::BTreeSet<&str> = SHIMS
        .iter()
        .filter_map(|s| {
            if s.parent.is_none() {
                Some(s.name)
            } else {
                None
            }
        })
        .collect();
    let mut children: Vec<envelope::CatalogueChild> = cmd
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set())
        .filter(|sub| !shim_top_level.contains(sub.get_name()))
        .map(|sub| envelope::CatalogueChild {
            node_type: "command",
            name: sub.get_name().to_string(),
            path: vec!["extend".to_string(), sub.get_name().to_string()],
            summary: sub.get_about().map(|a| a.to_string()).unwrap_or_default(),
            alias_of: None,
        })
        .collect();

    // Add alias children for each migration shortcut, derived from the
    // same registration table that drives routing and help text.
    children.extend(shim_alias_children());

    envelope::to_value(&envelope::DescribeEnvelope {
        schema_version: "1",
        kind: envelope::DescribeKind::Catalogue,
        path: vec!["extend".to_string()],
        generated_by: envelope::generator_info(),
        data: envelope::CatalogueData {
            node_type: "command-group",
            name: "extend".to_string(),
            summary: "Extend platform tooling".to_string(),
            children,
            alias_of: None,
        },
    })
}

/// Build an alias-typed `CatalogueChild` for a single migration shortcut.
///
/// Each entry carries `node_type: "alias"`, the extend-relative path,
/// a summary naming the canonical address, and `alias_of` pointing at
/// the canonical service path. Driven from the SHIMS table: no second list.
///
/// The `name` field is always the bare relative name (`shim.name`), matching
/// the last element of the child's `path`. This is the same invariant every
/// other catalogue node observes — a downstream walker can trust
/// `name == path.last()` unconditionally.
fn shim_alias_child(shim: &service_shims::ExtendShim) -> envelope::CatalogueChild {
    let path = service_shims::shim_path_segments(shim);
    let canonical = vec![
        shim.service.to_string(),
        shim.resource.to_string(),
        shim.method.to_string(),
    ];
    envelope::CatalogueChild {
        node_type: "alias",
        name: shim.name.to_string(),
        path: std::iter::once("extend".to_string()).chain(path).collect(),
        summary: format!(
            "Migration shortcut for ags {} {} {}",
            shim.service, shim.resource, shim.method
        ),
        alias_of: Some(canonical),
    }
}

/// Build alias-typed `CatalogueChild` entries for all migration shortcuts.
fn shim_alias_children() -> Vec<envelope::CatalogueChild> {
    SHIMS.iter().map(shim_alias_child).collect()
}

/// Build a full describe envelope for a single alias node. Used by
/// `describe_extend_command` when the user asks about a specific shim name.
fn shim_alias_envelope(shim: &service_shims::ExtendShim) -> Value {
    let child = shim_alias_child(shim);
    envelope::to_value(&envelope::DescribeEnvelope {
        schema_version: "1",
        kind: envelope::DescribeKind::Command,
        path: child.path.clone(),
        generated_by: envelope::generator_info(),
        data: envelope::CatalogueData {
            node_type: "alias",
            name: child.name,
            summary: child.summary,
            children: vec![],
            alias_of: child.alias_of,
        },
    })
}

/// Describe a single extend subcommand.
///
/// Canonical commands are matched from the real clap tree (non-hidden
/// subcommands). Shim names and parent groups are matched from the SHIMS
/// table. Adding a new canonical command or shim requires zero edits here.
fn describe_extend_command(sub: &str) -> Outcome {
    // Build the extend command tree ONCE and derive both canonical names
    // and the about-text lookup from the same instance. Top-level shim
    // names are excluded: they are visible clap subcommands but have
    // their own alias-typed describe path (below).
    let cmd = builder::build_extend_command();
    let shim_top_level: std::collections::BTreeSet<&str> = SHIMS
        .iter()
        .filter_map(|s| {
            if s.parent.is_none() {
                Some(s.name)
            } else {
                None
            }
        })
        .collect();
    let canonical_names: Vec<String> = cmd
        .get_subcommands()
        .filter(|c| !c.is_hide_set())
        .filter(|c| !shim_top_level.contains(c.get_name()))
        .map(|c| c.get_name().to_string())
        .collect();

    // Canonical (non-hidden) extend subcommands — matched from the derived list.
    // When the canonical subcommand has visible children of its own (e.g.
    // `app-ui` with `setup-env`) or shim aliases underneath, return a
    // catalogue listing all children. Leaf commands return the bare command
    // envelope with no children.
    if canonical_names.contains(&sub.to_string()) {
        let subcmd = cmd.find_subcommand(sub);
        let summary = subcmd
            .and_then(|c| c.get_about().map(|a| a.to_string()))
            .unwrap_or_default();

        // Collect canonical (non-hidden, non-shim) children of this subcommand.
        let shim_child_names: std::collections::BTreeSet<&str> = SHIMS
            .iter()
            .filter(|s| s.parent == Some(sub))
            .map(|s| s.name)
            .collect();
        let mut children: Vec<envelope::CatalogueChild> = subcmd
            .map(|c| {
                c.get_subcommands()
                    .filter(|sc| !sc.is_hide_set())
                    .filter(|sc| !shim_child_names.contains(sc.get_name()))
                    .map(|sc| envelope::CatalogueChild {
                        node_type: "command",
                        name: sc.get_name().to_string(),
                        path: vec![
                            "extend".to_string(),
                            sub.to_string(),
                            sc.get_name().to_string(),
                        ],
                        summary: sc.get_about().map(|a| a.to_string()).unwrap_or_default(),
                        alias_of: None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Also include any shim aliases under this group.
        children.extend(
            SHIMS
                .iter()
                .filter(|s| s.parent == Some(sub))
                .map(shim_alias_child),
        );

        if children.is_empty() {
            return Ok(envelope::to_value(&envelope::DescribeEnvelope {
                schema_version: "1",
                kind: envelope::DescribeKind::Command,
                path: vec!["extend".to_string(), sub.to_string()],
                generated_by: envelope::generator_info(),
                data: envelope::CatalogueData {
                    node_type: "command",
                    name: sub.to_string(),
                    summary,
                    children: vec![],
                    alias_of: None,
                },
            }));
        }

        return Ok(envelope::to_value(&envelope::DescribeEnvelope {
            schema_version: "1",
            kind: envelope::DescribeKind::Catalogue,
            path: vec!["extend".to_string(), sub.to_string()],
            generated_by: envelope::generator_info(),
            data: envelope::CatalogueData {
                node_type: "command-group",
                name: sub.to_string(),
                summary,
                children,
                alias_of: None,
            },
        }));
    }

    // Top-level shim name — return the alias envelope matching what
    // `describe extend` already emits for this entry.
    if let Some(shim) = SHIMS.iter().find(|s| s.parent.is_none() && s.name == sub) {
        return Ok(shim_alias_envelope(shim));
    }

    // Shim parent group (hidden, no canonical counterpart) — return a
    // catalogue of the group's children.
    let group_members: Vec<_> = SHIMS.iter().filter(|s| s.parent == Some(sub)).collect();
    if !group_members.is_empty() {
        let children: Vec<envelope::CatalogueChild> =
            group_members.iter().map(|s| shim_alias_child(s)).collect();
        return Ok(envelope::to_value(&envelope::DescribeEnvelope {
            schema_version: "1",
            kind: envelope::DescribeKind::Catalogue,
            path: vec!["extend".to_string(), sub.to_string()],
            generated_by: envelope::generator_info(),
            data: envelope::CatalogueData {
                node_type: "command-group",
                name: sub.to_string(),
                summary: format!("{sub} commands"),
                children,
                alias_of: None,
            },
        }));
    }

    // Unknown — suggest from all known extend subcommand names (canonical,
    // top-level shims, and parent groups).
    let shim_names: Vec<String> = SHIMS
        .iter()
        .filter(|s| s.parent.is_none())
        .map(|s| s.name.to_string())
        .collect();
    let parent_names: Vec<String> = service_shims::parent_group_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    let all_names: Vec<String> = canonical_names
        .into_iter()
        .chain(shim_names)
        .chain(parent_names)
        .collect();
    let all_refs: Vec<&str> = all_names.iter().map(String::as_str).collect();
    let suggestions = envelope::find_suggestions(sub, &all_refs);
    Err(error_envelope(
        vec!["extend".to_string(), sub.to_string()],
        "unknown_command",
        format!("Unknown command '{sub}' under 'extend'"),
        suggestions,
    ))
}

/// Resolve a three-token extend describe path: `describe extend <parent> <name>`.
///
/// Returns the alias envelope when `(parent, name)` matches a subgroup entry
/// in the shim table. Also checks for canonical (non-hidden) subcommands
/// under the parent group (e.g. `setup-env` under `app-ui`). Falls back to
/// the standard error for genuinely unknown paths.
fn describe_extend_subgroup_entry(parent: &str, name: &str) -> Outcome {
    // Shim alias match.
    if let Some(shim) = SHIMS
        .iter()
        .find(|s| s.parent == Some(parent) && s.name == name)
    {
        return Ok(shim_alias_envelope(shim));
    }

    // Canonical subcommand match — check the clap tree for a non-hidden
    // subcommand under the parent group.
    let cmd = builder::build_extend_command();
    if let Some(parent_cmd) = cmd.find_subcommand(parent) {
        if let Some(sub) = parent_cmd
            .get_subcommands()
            .find(|c| c.get_name() == name && !c.is_hide_set())
        {
            let summary = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
            return Ok(envelope::to_value(&envelope::DescribeEnvelope {
                schema_version: "1",
                kind: envelope::DescribeKind::Command,
                path: vec!["extend".to_string(), parent.to_string(), name.to_string()],
                generated_by: envelope::generator_info(),
                data: envelope::CatalogueData {
                    node_type: "command",
                    name: name.to_string(),
                    summary,
                    children: vec![],
                    alias_of: None,
                },
            }));
        }
    }

    Err(invalid_extend_path_error(parent, name))
}

/// Error envelope for a genuinely unknown three-token extend describe path.
fn invalid_extend_path_error(sub: &str, extra: &str) -> Value {
    envelope::to_value(&envelope::DescribeEnvelope {
        schema_version: "1",
        kind: envelope::DescribeKind::Error,
        path: vec!["extend".to_string(), sub.to_string(), extra.to_string()],
        generated_by: envelope::generator_info(),
        data: envelope::DescribeErrorData {
            code: "invalid_extend_path".to_string(),
            message: format!(
                "'{extra}' is not a valid name under 'extend {sub}'. \
                 Use 'ags describe extend'."
            ),
            suggestions: vec![],
        },
    })
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
    fn test_describe_root_includes_extend_child() {
        let v = describe_root();
        let children = v["data"]["children"].as_array().unwrap();
        assert!(
            children
                .iter()
                .any(|c| c["name"] == "extend" && c["node_type"] == "command-group"),
            "root catalogue must list an extend node"
        );
    }

    #[test]
    fn test_describe_extend_lists_clone_template() {
        let v = describe_extend();
        assert_eq!(v["kind"], "catalogue");
        assert_eq!(v["data"]["node_type"], "command-group");
        assert_eq!(v["data"]["name"], "extend");
        let children = v["data"]["children"].as_array().unwrap();
        assert!(children
            .iter()
            .any(|c| c["name"] == "clone-template" && c["node_type"] == "command"));
    }

    #[test]
    fn test_describe_extend_command_known() {
        let v = describe_extend_command("clone-template").unwrap();
        assert_eq!(v["kind"], "command");
        assert_eq!(v["data"]["name"], "clone-template");
    }

    #[test]
    fn test_describe_extend_command_unknown_suggests() {
        let v = describe_extend_command("clone-templatex").unwrap_err();
        assert_eq!(v["kind"], "error");
        assert_eq!(v["data"]["code"], "unknown_command");
        let suggestions = v["data"]["suggestions"].as_array().unwrap();
        assert!(
            suggestions.iter().any(|s| s == "clone-template"),
            "should suggest clone-template: {suggestions:?}"
        );
    }

    #[test]
    fn test_invalid_extend_path_error_echoes_path() {
        let v = invalid_extend_path_error("clone-template", "extra");
        assert_eq!(v["kind"], "error");
        assert_eq!(v["data"]["code"], "invalid_extend_path");
        assert_eq!(
            v["path"],
            serde_json::json!(["extend", "clone-template", "extra"])
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

    // ── Extend migration shortcut alias nodes (Test Plan 5, 6) ──

    /// Test Plan case 5: every shim in the registration table produces an
    /// alias-typed child in `describe extend` with the correct shape, and
    /// `name` equals the bare relative name (the last element of `path`).
    #[test]
    fn test_describe_extend_lists_alias_child_per_shim() {
        let v = describe_extend();
        let children = v["data"]["children"].as_array().unwrap();

        for shim in SHIMS {
            let alias = children
                .iter()
                .find(|c| c["name"].as_str() == Some(shim.name))
                .unwrap_or_else(|| panic!("describe extend must list alias child '{}'", shim.name));

            assert_eq!(
                alias["node_type"], "alias",
                "shim '{}' must have node_type 'alias'",
                shim.name
            );

            // name must equal the bare relative name from the registration table.
            assert_eq!(
                alias["name"], shim.name,
                "shim '{}': name must be the bare relative name",
                shim.name
            );

            let expected_path: Vec<String> = std::iter::once("extend".to_string())
                .chain(service_shims::shim_path_segments(shim))
                .collect();
            assert_eq!(
                alias["path"],
                serde_json::json!(expected_path),
                "shim '{}': path mismatch",
                shim.name
            );

            let alias_of = alias["alias_of"]
                .as_array()
                .unwrap_or_else(|| panic!("shim '{}' must have alias_of", shim.name));
            assert_eq!(
                alias_of,
                &vec![
                    serde_json::json!(shim.service),
                    serde_json::json!(shim.resource),
                    serde_json::json!(shim.method),
                ],
                "shim '{}': alias_of must be the canonical service triple",
                shim.name
            );
        }
    }

    /// Test Plan case 6: no `node_type: "command"` child shares a name with
    /// any shim. This protects downstream catalogue walkers from ambiguity
    /// between alias nodes and real command nodes.
    #[test]
    fn test_describe_extend_no_command_child_shadows_shim() {
        let v = describe_extend();
        let children = v["data"]["children"].as_array().unwrap();

        let command_names: Vec<&str> = children
            .iter()
            .filter(|c| c["node_type"].as_str() == Some("command"))
            .filter_map(|c| c["name"].as_str())
            .collect();

        for shim in SHIMS {
            assert!(
                !command_names.contains(&shim.name),
                "command child '{}' collides with shim alias — \
                 downstream catalogue walkers would see duplicate names \
                 with different node_types",
                shim.name
            );
        }
    }

    /// Test Plan case 7 (inline half): `describe csm apps` is unaffected
    /// by the alias additions — no `alias_of` field appears in any child,
    /// and all children have a non-alias node_type.
    #[test]
    fn test_describe_csm_apps_unchanged_no_alias_of() {
        let v = describe_resource("csm", "apps").unwrap();
        assert_eq!(v["kind"], "catalogue");
        assert_eq!(v["data"]["node_type"], "resource");
        assert_eq!(v["data"]["name"], "apps");

        let children = v["data"]["children"].as_array().unwrap();
        assert!(!children.is_empty(), "csm apps must have method children");

        for child in children {
            assert_eq!(
                child["node_type"], "method",
                "csm apps children must all be methods, got {:?}",
                child["node_type"]
            );
            assert!(
                child.get("alias_of").is_none(),
                "csm apps child '{}' must not have alias_of — \
                 the field should be omitted, not null",
                child["name"]
            );
        }
    }

    /// Non-alias nodes omit the `alias_of` field entirely (not null).
    #[test]
    fn test_non_alias_child_omits_alias_of_field() {
        let v = describe_root();
        let children = v["data"]["children"].as_array().unwrap();

        for child in children {
            if child["node_type"].as_str() != Some("alias") {
                assert!(
                    child.get("alias_of").is_none(),
                    "non-alias child '{}' (type {:?}) must omit alias_of entirely",
                    child["name"],
                    child["node_type"]
                );
            }
        }
    }

    // ── Describe extend shim alias (Finding 4 fix) ──

    /// `describe extend <shim-name>` returns the alias envelope matching
    /// `describe extend`'s catalogue entry for the same shim. Parameterised
    /// over the full SHIMS table so adding a shim requires zero edits here.
    #[test]
    fn test_describe_extend_shim_returns_alias_envelope() {
        for shim in SHIMS {
            if shim.parent.is_some() {
                continue; // subgroup shims are addressed via their parent
            }
            let v = describe_extend_command(shim.name)
                .expect("top-level shim must return Ok, not unknown_command");
            assert_eq!(
                v["kind"], "command",
                "shim '{}': kind must be 'command'",
                shim.name
            );
            assert_eq!(
                v["data"]["node_type"], "alias",
                "shim '{}': node_type must be 'alias'",
                shim.name
            );
            let alias_of = v["data"]["alias_of"]
                .as_array()
                .unwrap_or_else(|| panic!("shim '{}' must have alias_of", shim.name));
            assert_eq!(
                alias_of,
                &vec![
                    serde_json::json!(shim.service),
                    serde_json::json!(shim.resource),
                    serde_json::json!(shim.method),
                ],
                "shim '{}': alias_of must match the canonical service triple",
                shim.name
            );
        }
    }

    /// `describe extend <unknown>` returns `unknown_command`.
    #[test]
    fn test_describe_extend_unknown_returns_error() {
        let v = describe_extend_command("no-such-command").unwrap_err();
        assert_eq!(v["kind"], "error");
        assert_eq!(v["data"]["code"], "unknown_command");
        // Suggestions may be empty when the query has no substring overlap.
        assert!(v["data"]["suggestions"].as_array().is_some());
    }

    /// Suggestions include both canonical and shim names when the query
    /// matches by substring. Uses "app" which overlaps multiple shim names.
    #[test]
    fn test_describe_extend_unknown_suggests_from_full_set() {
        let v = describe_extend_command("app").unwrap_err();
        assert_eq!(v["data"]["code"], "unknown_command");
        let suggestions = v["data"]["suggestions"].as_array().unwrap();
        assert!(
            !suggestions.is_empty(),
            "a query matching shim names by substring must produce suggestions; got none"
        );
        // Verify shim names appear (not just canonical commands).
        assert!(
            suggestions.iter().any(|s| {
                let name = s.as_str().unwrap_or("");
                name.contains("app")
            }),
            "suggestions must include shim names containing 'app': {suggestions:?}"
        );
    }

    /// `describe extend <parent-group>` returns a catalogue of the group's
    /// children, not an `unknown_command` error. Each child's `name` is the
    /// bare relative name from the shim table, not the combined display name.
    #[test]
    fn test_describe_extend_parent_group_returns_catalogue() {
        let parents = service_shims::parent_group_names();
        assert!(
            !parents.is_empty(),
            "shim table must have at least one parent group for this test"
        );
        for parent in parents {
            let v = describe_extend_command(parent)
                .expect("parent group must return Ok, not unknown_command");
            assert_eq!(
                v["kind"], "catalogue",
                "parent group '{parent}': kind must be 'catalogue'"
            );
            assert_eq!(
                v["data"]["node_type"], "command-group",
                "parent group '{parent}': node_type must be 'command-group'"
            );
            let children = v["data"]["children"].as_array().unwrap();
            assert!(
                !children.is_empty(),
                "parent group '{parent}' must have at least one child"
            );

            // Build expected children from the shim table for this parent.
            let expected_children: Vec<&service_shims::ExtendShim> =
                SHIMS.iter().filter(|s| s.parent == Some(parent)).collect();

            for shim in &expected_children {
                let child = children
                    .iter()
                    .find(|c| c["name"].as_str() == Some(shim.name))
                    .unwrap_or_else(|| {
                        panic!("parent group '{parent}' must list child '{}'", shim.name)
                    });
                assert_eq!(
                    child["node_type"], "alias",
                    "parent group '{parent}': child '{}' must be an alias",
                    shim.name
                );
                // The name must be the bare relative name from the shim table,
                // not the combined "<parent> <name>" display form.
                assert_eq!(
                    child["name"], shim.name,
                    "parent group '{parent}': child name must be the bare \
                     relative name from the shim table"
                );
            }
        }
    }

    /// Every alias child emitted by any describe path satisfies the invariant
    /// `name == path.last()`. This is the contract downstream catalogue
    /// walkers rely on: a child's name is always its bare relative position
    /// in the tree, never a combined display form that embeds parent context.
    /// Parameterised over the shim table so adding a shim requires zero edits.
    #[test]
    fn test_alias_child_name_equals_path_last_invariant() {
        // 1. Flat `describe extend` listing — every alias child.
        let flat = describe_extend();
        let flat_children = flat["data"]["children"].as_array().unwrap();
        for child in flat_children {
            if child["node_type"].as_str() == Some("alias") {
                let name = child["name"].as_str().unwrap();
                let path = child["path"].as_array().unwrap();
                let last = path.last().unwrap().as_str().unwrap();
                assert_eq!(
                    name, last,
                    "flat listing: alias child name '{name}' must equal \
                     path.last() '{last}' (path: {path:?})"
                );
            }
        }

        // 2. Nested parent-group listings — every alias child.
        let parents = service_shims::parent_group_names();
        for parent in &parents {
            let v = describe_extend_command(parent).unwrap();
            let children = v["data"]["children"].as_array().unwrap();
            for child in children {
                if child["node_type"].as_str() == Some("alias") {
                    let name = child["name"].as_str().unwrap();
                    let path = child["path"].as_array().unwrap();
                    let last = path.last().unwrap().as_str().unwrap();
                    assert_eq!(
                        name, last,
                        "parent group '{parent}': alias child name '{name}' \
                         must equal path.last() '{last}' (path: {path:?})"
                    );
                }
            }
        }

        // 3. Individual alias envelopes for top-level shims.
        for shim in SHIMS {
            if shim.parent.is_some() {
                continue;
            }
            let v = describe_extend_command(shim.name).unwrap();
            if v["data"]["node_type"].as_str() == Some("alias") {
                let name = v["data"]["name"].as_str().unwrap();
                let path = v["path"].as_array().unwrap();
                let last = path.last().unwrap().as_str().unwrap();
                assert_eq!(
                    name, last,
                    "alias envelope for '{}': name '{name}' must equal \
                     path.last() '{last}' (path: {path:?})",
                    shim.name
                );
            }
        }
    }

    // ── Describe extend subgroup three-token paths ──

    /// `describe extend <parent> <name>` returns the alias envelope for a
    /// known subgroup shim. Parameterised over every subgroup entry in the
    /// SHIMS table so adding a shim requires zero edits here.
    #[test]
    fn test_describe_extend_subgroup_shim_returns_alias_envelope() {
        for shim in SHIMS {
            let Some(parent) = shim.parent else { continue };
            let v = describe_extend_subgroup_entry(parent, shim.name)
                .expect("subgroup shim must return Ok, not an error");
            assert_eq!(
                v["kind"],
                "command",
                "shim '{parent}/{name}': kind must be 'command'",
                name = shim.name
            );
            assert_eq!(
                v["data"]["node_type"],
                "alias",
                "shim '{parent}/{name}': node_type must be 'alias'",
                name = shim.name
            );
            let expected_path = serde_json::json!(["extend", parent, shim.name]);
            assert_eq!(
                v["path"],
                expected_path,
                "shim '{parent}/{name}': path must include the parent group",
                name = shim.name
            );
            let alias_of = v["data"]["alias_of"]
                .as_array()
                .unwrap_or_else(|| panic!("shim '{parent}/{}' must have alias_of", shim.name));
            assert_eq!(
                alias_of,
                &vec![
                    serde_json::json!(shim.service),
                    serde_json::json!(shim.resource),
                    serde_json::json!(shim.method),
                ],
                "shim '{parent}/{name}': alias_of must match the canonical \
                 service triple",
                name = shim.name
            );
        }
    }

    /// A genuinely unknown three-token extend path returns the standard error.
    #[test]
    fn test_describe_extend_subgroup_unknown_returns_error() {
        let v = describe_extend_subgroup_entry("remote-debug", "no-such-name").unwrap_err();
        assert_eq!(v["kind"], "error");
        assert_eq!(v["data"]["code"], "invalid_extend_path");
    }

    /// Every non-hidden, non-shim subcommand of the extend clap tree
    /// appears in the `describe extend` catalogue as a `node_type: "command"`
    /// child. The expectation is derived from the clap tree, so adding a new
    /// extend command is covered automatically. `app-ui` and `remote-debug`
    /// are asserted by name because a prior regression silently removed them.
    #[test]
    fn test_describe_extend_lists_all_canonical_subcommands() {
        let cmd = builder::build_extend_command();
        let shim_top_level: std::collections::BTreeSet<&str> = SHIMS
            .iter()
            .filter_map(|s| {
                if s.parent.is_none() {
                    Some(s.name)
                } else {
                    None
                }
            })
            .collect();

        // Derive the set of canonical (non-hidden, non-shim) subcommand names.
        let expected_canonical: Vec<String> = cmd
            .get_subcommands()
            .filter(|sub| !sub.is_hide_set())
            .filter(|sub| !shim_top_level.contains(sub.get_name()))
            .map(|sub| sub.get_name().to_string())
            .collect();

        assert!(
            !expected_canonical.is_empty(),
            "the derived canonical set must be non-empty"
        );

        // Specifically confirm that the two parent-group commands are in
        // the derived set — they are the ones the regression removed.
        assert!(
            expected_canonical.contains(&"app-ui".to_string()),
            "app-ui must be a canonical extend subcommand in the clap tree"
        );
        assert!(
            expected_canonical.contains(&"remote-debug".to_string()),
            "remote-debug must be a canonical extend subcommand in the clap tree"
        );

        let v = describe_extend();
        let children = v["data"]["children"].as_array().unwrap();
        let command_children: Vec<&str> = children
            .iter()
            .filter(|c| c["node_type"].as_str() == Some("command"))
            .filter_map(|c| c["name"].as_str())
            .collect();

        // Floor check: at least as many command children as canonical names.
        assert!(
            command_children.len() >= expected_canonical.len(),
            "describe extend must have at least {} command children \
             (one per canonical subcommand); got {}",
            expected_canonical.len(),
            command_children.len()
        );

        // Membership check: every canonical name appears.
        for name in &expected_canonical {
            assert!(
                command_children.contains(&name.as_str()),
                "canonical extend subcommand '{name}' must appear as a \
                 node_type='command' child of describe extend"
            );
        }
    }

    /// No child name appears more than once in the `describe extend`
    /// children list, regardless of `node_type`. This is the invariant the
    /// shim-exclusion filter exists to protect: without it, top-level shim
    /// names would appear twice (once as `command` from clap, once as
    /// `alias` from the shim table).
    #[test]
    fn test_describe_extend_no_duplicate_child_names() {
        let v = describe_extend();
        let children = v["data"]["children"].as_array().unwrap();
        let names: Vec<&str> = children.iter().filter_map(|c| c["name"].as_str()).collect();

        let mut seen = std::collections::BTreeSet::new();
        for name in &names {
            assert!(
                seen.insert(name),
                "child name '{name}' appears more than once in \
                 describe extend children list"
            );
        }
    }

    /// Walking invariant: every `path` array that appears in any
    /// describe-extend output is itself a describable address. This pins the
    /// contract that a catalogue walker can always recurse into any
    /// advertised child without receiving an error.
    #[test]
    fn test_every_advertised_child_path_is_describable() {
        // 1. Walk every child of `describe extend`.
        let root = describe_extend();
        let root_children = root["data"]["children"].as_array().unwrap();

        for child in root_children {
            let path = child["path"].as_array().unwrap();
            let segments: Vec<&str> = path.iter().filter_map(|v| v.as_str()).collect();

            let result = match segments.as_slice() {
                ["extend", sub] => describe_extend_command(sub),
                ["extend", parent, name] => describe_extend_subgroup_entry(parent, name),
                other => panic!("unexpected path shape in describe extend: {other:?}"),
            };

            assert!(
                result.is_ok(),
                "path {segments:?} (advertised by `describe extend`) \
                 must be describable; got error: {:?}",
                result.unwrap_err()
            );
        }

        // 2. Walk every child of each parent group.
        for parent in service_shims::parent_group_names() {
            let group = describe_extend_command(parent).expect("parent group must be describable");
            let group_children = group["data"]["children"]
                .as_array()
                .expect("parent group must have a children array");

            for child in group_children {
                let path = child["path"].as_array().unwrap();
                let segments: Vec<&str> = path.iter().filter_map(|v| v.as_str()).collect();

                let result = match segments.as_slice() {
                    ["extend", p, name] => describe_extend_subgroup_entry(p, name),
                    other => panic!(
                        "unexpected child path shape in parent group \
                         '{parent}': {other:?}"
                    ),
                };

                assert!(
                    result.is_ok(),
                    "path {segments:?} (advertised by `describe extend \
                     {parent}`) must be describable; got error: {:?}",
                    result.unwrap_err()
                );
            }
        }
    }
}
