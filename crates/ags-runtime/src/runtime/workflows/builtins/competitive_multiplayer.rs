//! The `competitive-multiplayer` built-in workflow.
//!
//! Chains the six AccelByte operations that stand up competitive
//! matchmaking with dedicated servers: a skill stat, a match ruleset, a
//! session template, a match pool, an AMS fleet, and a final update wiring
//! the session template to the fleet's claim key.
//!
//! v3 uses a simplified 8-input contract. Resource names are derived from a
//! single `resourcePrefix` via Format bindings. Player counts are derived
//! from `playersPerTeam * teamCount` via Arithmetic bindings. The fleet
//! image is specified by `fleetImageId` alone (no whole-object input).
//! See `docs/private/workflow-protocol.md`.

use ags_protocol::catalogue::{OperationId, ServiceId};
use ags_protocol::workflow::{
    ArithmeticOp, ArithmeticOperand, ArithmeticTransform, BindingSource, CompletionResource,
    CompletionStep, FormatBinding, LiteralBinding, OperationReference, OptionParameterBinding,
    OptionsSource, ReferenceBinding, ReferenceTarget, StepDefinition, StepInputBinding,
    TransformKind, WorkflowBriefing, WorkflowCompletion, WorkflowDefinition, WorkflowId,
    WorkflowInputSpec,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::runtime::workflows::Workflow;

/// The `competitive-multiplayer` built-in workflow.
pub struct CompetitiveMultiplayer {
    definition: WorkflowDefinition,
}

impl CompetitiveMultiplayer {
    /// Build the workflow with its hand-written definition.
    pub fn new() -> Self {
        Self {
            definition: build_definition(),
        }
    }
}

impl Default for CompetitiveMultiplayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Workflow for CompetitiveMultiplayer {
    fn definition(&self) -> &WorkflowDefinition {
        &self.definition
    }
}

/// A literal-valued binding (`{const: <value>}`).
fn literal(value: Value) -> BindingSource {
    BindingSource::Literal(LiteralBinding {
        value,
        sensitive: false,
    })
}

/// A `from: workflow/<input>` reference binding.
fn workflow_ref(input: &str) -> BindingSource {
    BindingSource::Reference(ReferenceBinding {
        from: ReferenceTarget::Workflow {
            input: input.to_string(),
        },
        output: None,
        transform: None,
    })
}

/// A Format binding that interpolates workflow inputs into a template string.
fn format_binding(template: &str) -> BindingSource {
    BindingSource::Format(FormatBinding {
        template: template.into(),
    })
}

/// An Arithmetic binding: `workflow/<input> <op> workflow/<operand_input>`.
fn arithmetic(input: &str, op: ArithmeticOp, operand_input: &str) -> BindingSource {
    BindingSource::Reference(ReferenceBinding {
        from: ReferenceTarget::Workflow {
            input: input.into(),
        },
        output: None,
        transform: Some(TransformKind::Arithmetic(ArithmeticTransform {
            op,
            operand: ArithmeticOperand::WorkflowInput(operand_input.into()),
        })),
    })
}

/// Pair an operation field name with its value source.
fn bind(field: &str, source: BindingSource) -> StepInputBinding {
    StepInputBinding {
        field: field.to_string(),
        source,
        show_in_review: false,
        description: None,
    }
}

/// Pair an operation field name with its value source, marked as visible
/// in the step-walk review form. Use for opinionated step-local defaults
/// (e.g. `joinability`, `auto_backfill`) that users may want to override
/// but that don't deserve to be promoted to workflow inputs.
fn bind_visible(field: &str, source: BindingSource) -> StepInputBinding {
    StepInputBinding {
        field: field.to_string(),
        source,
        show_in_review: true,
        description: None,
    }
}

/// Like `bind_visible`, but also carries a description shown in the hint area
/// when the field is focused. Use for per-leaf nested bindings whose schema
/// has no description to inherit.
fn bind_visible_with_description(
    field: &str,
    source: BindingSource,
    description: &str,
) -> StepInputBinding {
    StepInputBinding {
        field: field.to_string(),
        source,
        show_in_review: true,
        description: Some(description.to_string()),
    }
}

/// Build one step. `confirm` is always false — these operations are all
/// non-risky creates and updates.
fn step(
    id: &str,
    description: &str,
    service: &str,
    operation: &str,
    dependencies: Vec<String>,
    inputs: Vec<StepInputBinding>,
) -> StepDefinition {
    StepDefinition {
        id: id.to_string(),
        description: Some(description.to_string()),
        operation: OperationReference {
            service: ServiceId::new(service),
            operation: OperationId::new(operation),
        },
        dependencies,
        confirm: false,
        is_optional: false,
        continue_on_failure: false,
        skip_if_exists: false,
        is_reviewed: None,
        inputs,
        outputs: vec![],
    }
}

/// Declare a workflow input.
fn input(
    name: &str,
    description: &str,
    required: bool,
    default: Option<Value>,
    schema: Value,
) -> WorkflowInputSpec {
    WorkflowInputSpec {
        name: name.to_string(),
        description: Some(description.to_string()),
        schema: Some(schema),
        required,
        default,
        sensitive: false,
        options_source: None,
        location: ags_protocol::workflow::StepFieldLocation::Body,
    }
}

/// Declare a workflow input with a runtime-fetched options source (dynamic enum).
fn input_with_options(
    name: &str,
    description: &str,
    required: bool,
    default: Option<Value>,
    schema: Value,
    options_source: OptionsSource,
) -> WorkflowInputSpec {
    WorkflowInputSpec {
        name: name.to_string(),
        description: Some(description.to_string()),
        schema: Some(schema),
        required,
        default,
        sensitive: false,
        options_source: Some(options_source),
        location: ags_protocol::workflow::StepFieldLocation::Body,
    }
}

/// Build the hand-written `competitive-multiplayer` definition.
fn build_definition() -> WorkflowDefinition {
    WorkflowDefinition {
        id: WorkflowId::new("competitive-multiplayer"),
        name: "Set up competitive multiplayer".to_string(),
        intent: Some("matchmaking ranked competitive dedicated servers AMS match pool".to_string()),
        description: Some(
            "Stand up competitive matchmaking with dedicated servers: a skill stat, \
             a match ruleset, a session template, a match pool, an AMS fleet, and \
             the session template wired to the fleet. Matches start only when teams \
             are exactly full (no under-filled sessions) — appropriate for ranked play."
                .to_string(),
        ),
        briefing: Some(WorkflowBriefing {
            overview: concat!(
                "This workflow sets up **skill-based matchmaking** using ",
                "**dedicated servers** hosted by ",
                "**AccelByte Multiplayer Servers (AMS)**. ",
                "Skill-based matchmaking pairs players using a **rating stat** ",
                "(often called **Matchmaking Rating** or **MMR**) so the matchmaker ",
                "prefers opponents of similar skill instead of pairing players ",
                "at random. Dedicated servers mean each game session runs on a ",
                "separate server build managed by AMS rather than being hosted ",
                "by one of the players.\n\n",

                "Configuring this by hand involves quite a few steps across ",
                "multiple AccelByte services. This workflow simplifies the ",
                "process by creating the required resources and connecting ",
                "them for you."
            ).into(),
            prerequisites: vec![
                concat!(
                    "**An AMS server image** already available in the namespace you ",
                    "want to use. This is the dedicated server build AMS will launch ",
                    "when matches need hosting. If you already have images uploaded, ",
                    "you can list them with `ags ams images list --namespace <namespace>`. ",
                    "If you do not have one yet, upload your server image first using ",
                    "the AMS CLI, note the image id, then come back and run this workflow."
                ).into(),
                concat!(
                    "**A namespace** already created. Everything this workflow creates ",
                    "lives inside that namespace."
                ).into(),
                concat!(
                    "**Admin credentials** with permission to create or update resources ",
                    "in `iam`, `matchmaking`, `session` and `ams`. The workflow uses your current ",
                    "`ags` login session."
                ).into(),
            ],
            creates: vec![
                "**A player-rating stat.** This represents each player's matchmaking skill.".into(),
                "**A matchmaking ruleset.** This uses that stat together with your team and player-count settings.".into(),
                "**A session template.** This defines how match sessions are created.".into(),
                "**A match pool.** This is the queue players join when searching for a match.".into(),
                "**A dedicated server fleet.** This is created from your AMS image in the region you choose.".into(),
                "**The final session-to-fleet wiring.** This makes sure matched players are hosted on that fleet.".into(),
            ],
        }),
        inputs: vec![
            input(
                "namespace",
                "Your game's AccelByte namespace. This is the same value used by the global --namespace flag.",
                true,
                None,
                json!({"type": "string"}),
            ),
            input(
                "playersPerTeam",
                "How many players make up one team. Teams are symmetric, so with the default teamCount of 2, setting this to 4 produces 4v4 matches. The matchmaker only starts a match when every team is exactly full.",
                false,
                Some(json!(4)),
                json!({"type": "integer", "minimum": 1}),
            ),
            input(
                "teamCount",
                "How many teams compete in each match. Defaults to 2 for the classic X v X format. Increase this value for free-for-all formats. For example, 4 teams of 3 makes a 12-player free-for-all.",
                false,
                Some(json!(2)),
                json!({"type": "integer", "minimum": 2}),
            ),
            input_with_options(
                "fleetImageId",
                "The UUID of an AMS image. An image is the server build for your game which you must upload using the AMS CLI. If you've already uploaded an image you can find it using `ags ams images list`. If you haven't uploaded an image yet you should exit this workflow and upload it first.",
                true,
                None,
                json!({"type": "string"}),
                OptionsSource {
                    operation: OperationReference {
                        service: ServiceId::new("ams"),
                        operation: OperationId::new("ams/admin/images/v1/list"),
                    },
                    parameters: BTreeMap::from([(
                        "namespace".to_string(),
                        OptionParameterBinding::FromInput("namespace".to_string()),
                    )]),
                    items_path: "$.images".into(),
                    value: "$.id".into(),
                    label: Some("$.name".into()),
                    label_detail: None,
                    fallback_description: None,
                    filter: None,
                },
            ),
            input_with_options(
                "fleetRegion",
                "The region the AMS fleet will run in. Run `ags ams info list-regions` to see the regions configured for your account.",
                true,
                None,
                json!({"type": "string"}),
                OptionsSource {
                    operation: OperationReference {
                        service: ServiceId::new("ams"),
                        operation: OperationId::new("ams/admin/info/v1/list-regions"),
                    },
                    parameters: BTreeMap::from([(
                        "namespace".to_string(),
                        OptionParameterBinding::FromInput("namespace".to_string()),
                    )]),
                    items_path: "$.regions".into(),
                    value: "$".into(),
                    label: None,
                    label_detail: None,
                    fallback_description: None,
                    filter: None,
                },
            ),
            input_with_options(
                "fleetInstanceId",
                "The UUID of an AMS instance type. The instance type determines the per-server VM size. Run `ags ams info list-supported-instances` to see the instance types configured for your account.",
                true,
                None,
                json!({"type": "string"}),
                OptionsSource {
                    operation: OperationReference {
                        service: ServiceId::new("ams"),
                        operation: OperationId::new("ams/admin/info/v1/list-supported-instances"),
                    },
                    parameters: BTreeMap::from([(
                        "namespace".to_string(),
                        OptionParameterBinding::FromInput("namespace".to_string()),
                    )]),
                    items_path: "$.availableInstanceTypes".into(),
                    value: "$.id".into(),
                    label: Some("$.name".into()),
                    label_detail: None,
                    fallback_description: None,
                    filter: None,
                },
            ),
            input(
                "statCode",
                "The identifier of the skill stat the matchmaker groups players by. AGS matchmaking defaults to using distance-based Matchmaking Rating (MMR).",
                false,
                Some(json!("mmr")),
                json!({"type": "string"}),
            ),
            input(
                "resourcePrefix",
                "Prefixes the names of every resource this workflow creates (ruleset, session template, match pool, fleet, and claim key). With the default value `ranked`, you get names like `ranked-ruleset`, `ranked-fleet`, and so on.",
                false,
                Some(json!("ranked")),
                json!({"type": "string"}),
            ),
        ],
        is_reviewed_by_default: true,
        steps: vec![
            // Step 1 — create the skill stat.
            step(
                "create-stat",
                "Creates the player skill statistic the matchmaker reads when grouping players.",
                "social",
                "social/admin/stat-definitions/v1/create",
                vec![],
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("statCode", workflow_ref("statCode")),
                    // Display name is fixed as "MMR"; statCode drives the matchmaker's code.
                    bind_visible("name", literal(json!("MMR"))),
                    bind_visible("defaultValue", literal(json!(1000))),
                    bind("setBy", literal(json!("SERVER"))),
                ],
            ),
            // Step 2 — create the match ruleset.
            step(
                "create-ruleset",
                "Defines the matchmaking rules: how the stat is compared and the tolerances the matchmaker may relax over time.",
                "match2",
                "matchmaking/admin/rule-sets/v1/create",
                vec![],
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("name", format_binding("{resourcePrefix}-ruleset")),
                    bind("enable_custom_match_function", literal(json!(false))),
                    bind(
                        "data",
                        literal(json!({
                            "matching_rule": [
                                {"criteria": "distance"}
                            ]
                        })),
                    ),
                    bind_visible_with_description(
                        "data.matching_rule[0].reference",
                        literal(json!(200)),
                        "MMR distance tolerance — how far apart two players' MMR can be and still match",
                    ),
                    bind_visible_with_description(
                        "data.auto_backfill",
                        literal(json!(true)),
                        "Whether the matchmaker should backfill empty slots in an active match",
                    ),
                    bind("data.matching_rule[0].attribute", workflow_ref("statCode")),
                    bind("data.alliance.min_number", workflow_ref("teamCount")),
                    bind("data.alliance.max_number", workflow_ref("teamCount")),
                    bind("data.alliance.player_min_number", workflow_ref("playersPerTeam")),
                    bind("data.alliance.player_max_number", workflow_ref("playersPerTeam")),
                ],
            ),
            // Step 3 — create the session template.
            step(
                "create-session-template",
                "Creates the session template the matchmaker hands successful matches to. It carries the player slot configuration, joinability, and reconnect policy.",
                "session",
                "session/admin/templates/v1/create",
                vec![],
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("name", format_binding("{resourcePrefix}-session")),
                    bind("type", literal(json!("DS"))),
                    // A DS-type session template must name its DS provider or it
                    // never claims a server. This workflow builds an AMS fleet, so
                    // the source is AMS; without it the requestedRegions and
                    // preferredClaimKeys below are inert.
                    bind("dsSource", literal(json!("AMS"))),
                    bind_visible("joinability", literal(json!("OPEN"))),
                    bind("clientVersion", literal(json!("1.0.0"))),
                    // The session API marks `deployment` required, but it only
                    // applies when DS type is `custom`. For DS type `DS` (AMS),
                    // an empty string is accepted and the field is unused. Binding
                    // it as a hidden literal keeps the form quiet without surfacing
                    // it to the user as an unset required field.
                    bind("deployment", literal(json!(""))),
                    bind(
                        "minPlayers",
                        arithmetic("playersPerTeam", ArithmeticOp::Mul, "teamCount"),
                    ),
                    bind(
                        "maxPlayers",
                        arithmetic("playersPerTeam", ArithmeticOp::Mul, "teamCount"),
                    ),
                    bind("inviteTimeout", literal(json!(60))),
                    bind_visible("inactiveTimeout", literal(json!(60))),
                    bind("persistent", literal(json!(false))),
                    bind("textChat", literal(json!(true))),
                    bind("requestedRegions[0]", workflow_ref("fleetRegion")),
                ],
            ),
            // Step 4 — create the match pool referencing the ruleset and
            // session template by their derived names.
            step(
                "create-match-pool",
                "Creates the match pool that ties the ruleset and session template together. Players queue into this pool and the matchmaker forms parties from it.",
                "match2",
                "matchmaking/admin/match-pools/v1/create",
                vec![
                    "create-ruleset".to_string(),
                    "create-session-template".to_string(),
                ],
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("name", format_binding("{resourcePrefix}-pool")),
                    bind("rule_set", format_binding("{resourcePrefix}-ruleset")),
                    bind("session_template", format_binding("{resourcePrefix}-session")),
                    bind("match_function", literal(json!("default"))),
                    bind("match_function_override", literal(json!({}))),
                    bind_visible("ticket_expiration_seconds", literal(json!(300))),
                    bind_visible("backfill_ticket_expiration_seconds", literal(json!(300))),
                    bind_visible("backfill_proposal_expiration_seconds", literal(json!(60))),
                    bind_visible("auto_accept_backfill_proposal", literal(json!(true))),
                ],
            ),
            // Step 5 — create the AMS fleet. The image and instance type are
            // provided by individual inputs; nested-field bindings patch into
            // the literal object skeletons.
            step(
                "create-ams-fleet",
                "Spins up the dedicated server fleet that hosts matches, attaching the image and instance type, and registering a claim key for the session template.",
                "ams",
                "ams/admin/fleets/v1/create",
                vec![],
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("name", format_binding("{resourcePrefix}-fleet")),
                    // New fleets start inactive so the workflow doesn't auto-provision
                    // servers (and incur cost) before the user has reviewed them.
                    bind_visible("active", literal(json!(false))),
                    // On-demand fleets must NOT carry an image (the AMS API rejects
                    // `onDemand: true` together with an imageDeploymentProfile.imageId);
                    // this workflow supplies an image, so it creates a reserved fleet.
                    // Required field, kept off the review since there is nothing to choose.
                    bind("onDemand", literal(json!(false))),
                    bind(
                        "regions",
                        literal(json!([{
                            "minServerCount": 0,
                            "maxServerCount": 2,
                            "bufferSize": 1,
                            "dynamicBuffer": true,
                        }])),
                    ),
                    bind("regions[0].region", workflow_ref("fleetRegion")),
                    bind("dsHostConfiguration", literal(json!({"serversPerVm": 1}))),
                    bind("dsHostConfiguration.instanceId", workflow_ref("fleetInstanceId")),
                    // commandLine and portConfigurations are required; sensible defaults
                    // (vs the previous empty values) make the created fleet actually
                    // launchable and reachable. The launch command is game-specific, so
                    // it is shown in the review for the user to tune; `${dsid}` and
                    // `${default_port}` are AMS launch-time substitution variables.
                    bind(
                        "imageDeploymentProfile",
                        literal(json!({
                            "portConfigurations": [{"name": "default", "protocol": "UDP"}],
                            "timeout": {
                                "creation": 120,
                                "session": 3600,
                                "drain": 60,
                                "unresponsive": 60,
                            },
                        })),
                    ),
                    bind_visible(
                        "imageDeploymentProfile.commandLine",
                        literal(json!("-dsid=${dsid} -port=${default_port}")),
                    ),
                    bind("imageDeploymentProfile.imageId", workflow_ref("fleetImageId")),
                    bind("claimKeys[0]", format_binding("{resourcePrefix}-claim-key")),
                ],
            ),
            // Step 6 — wire the session template to the fleet's claim key.
            step(
                "update-session-template",
                "Wires the session template to the fleet's claim key so successful matches reserve a server from the fleet you just created.",
                "session",
                "session/admin/templates/v1/update",
                vec![
                    "create-session-template".to_string(),
                    "create-ams-fleet".to_string(),
                ],
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("name", format_binding("{resourcePrefix}-session")),
                    bind("type", literal(json!("DS"))),
                    // A DS-type session template must name its DS provider or it
                    // never claims a server. This workflow builds an AMS fleet, so
                    // the source is AMS; without it the requestedRegions and
                    // preferredClaimKeys below are inert.
                    bind("dsSource", literal(json!("AMS"))),
                    bind("joinability", literal(json!("OPEN"))),
                    bind("clientVersion", literal(json!("1.0.0"))),
                    // See step 3 — `deployment` is spec-required but only
                    // meaningful for DS type `custom`. Empty string keeps it
                    // out of the review form on the AMS path.
                    bind("deployment", literal(json!(""))),
                    bind(
                        "minPlayers",
                        arithmetic("playersPerTeam", ArithmeticOp::Mul, "teamCount"),
                    ),
                    bind(
                        "maxPlayers",
                        arithmetic("playersPerTeam", ArithmeticOp::Mul, "teamCount"),
                    ),
                    bind("inviteTimeout", literal(json!(60))),
                    bind("inactiveTimeout", literal(json!(60))),
                    bind("persistent", literal(json!(false))),
                    bind("textChat", literal(json!(true))),
                    bind("requestedRegions[0]", workflow_ref("fleetRegion")),
                    bind("preferredClaimKeys[0]", format_binding("{resourcePrefix}-claim-key")),
                ],
            ),
        ],
        outputs: vec![],
        completion: Some(WorkflowCompletion {
            created: vec![
                CompletionResource {
                    label: "Skill stat".into(),
                    value: "{statCode}".into(),
                },
                CompletionResource {
                    label: "Match ruleset".into(),
                    value: "{resourcePrefix}-ruleset".into(),
                },
                CompletionResource {
                    label: "Session template".into(),
                    value: "{resourcePrefix}-session".into(),
                },
                CompletionResource {
                    label: "Match pool".into(),
                    value: "{resourcePrefix}-pool".into(),
                },
                CompletionResource {
                    label: "AMS fleet".into(),
                    value: "{resourcePrefix}-fleet".into(),
                },
            ],
            next_steps: vec![
                CompletionStep {
                    description: "Inspect the match pool you'll matchmake against".into(),
                    command: "ags matchmaking match-pools get --namespace {namespace} --pool {resourcePrefix}-pool".into(),
                },
                CompletionStep {
                    description: "Check your fleet".into(),
                    command: "ags ams fleets list --namespace {namespace} --name {resourcePrefix}-fleet".into(),
                },
                CompletionStep {
                    description: "Review the session template".into(),
                    command: "ags session templates get --namespace {namespace} --name {resourcePrefix}-session".into(),
                },
                CompletionStep {
                    description: "List your matchmaking rulesets".into(),
                    command: "ags matchmaking rule-sets list --namespace {namespace}".into(),
                },
            ],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;
    use crate::runtime::workflows::compile::compile_workflow;

    #[test]
    fn test_session_template_steps_set_ds_source_ams() {
        // A DS-type session template only claims a server fleet when dsSource is
        // set. This workflow builds an AMS fleet with claim keys, so both the
        // create and the update of the session template must set dsSource=AMS,
        // or the region/claim-key wiring is inert and the session never claims.
        let wf = CompetitiveMultiplayer::new();
        for step_id in ["create-session-template", "update-session-template"] {
            let step = wf
                .definition()
                .steps
                .iter()
                .find(|s| s.id == step_id)
                .unwrap_or_else(|| panic!("step {step_id} must exist"));
            let binding = step
                .inputs
                .iter()
                .find(|b| b.field == "dsSource")
                .unwrap_or_else(|| panic!("{step_id} must bind dsSource"));
            match &binding.source {
                BindingSource::Literal(l) => {
                    assert_eq!(l.value, json!("AMS"), "{step_id} dsSource must be AMS")
                }
                other => panic!("{step_id} dsSource must be a literal, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_workflow_declares_eight_inputs_four_required() {
        let wf = CompetitiveMultiplayer::new();
        let inputs = &wf.definition().inputs;
        let names: Vec<&str> = inputs.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "namespace",
                "playersPerTeam",
                "teamCount",
                "fleetImageId",
                "fleetRegion",
                "fleetInstanceId",
                "statCode",
                "resourcePrefix",
            ]
        );
        let required: Vec<&str> = inputs
            .iter()
            .filter(|i| i.required)
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(
            required,
            [
                "namespace",
                "fleetImageId",
                "fleetRegion",
                "fleetInstanceId"
            ]
        );
    }

    #[test]
    fn test_workflow_compiles_against_bundled_catalogue() {
        let workflow = CompetitiveMultiplayer::new();
        let mut catalogue = Catalogue::new();
        let compiled = compile_workflow(workflow.definition(), &mut catalogue)
            .expect("must compile against the bundled catalogue");
        assert_eq!(compiled.steps.len(), 6);
    }

    #[test]
    fn test_competitive_multiplayer_declares_completion_that_compiles() {
        let workflow = CompetitiveMultiplayer::new();
        assert!(
            workflow.definition().completion.is_some(),
            "completion authored"
        );
        let mut catalogue = Catalogue::new();
        // compile_workflow runs validate_completion; a bad template/placeholder
        // (or a single-step/empty completion) would fail here.
        compile_workflow(workflow.definition(), &mut catalogue).expect("completion must validate");
    }

    #[test]
    fn test_show_in_review_bindings_match_curation_list() {
        let wf = CompetitiveMultiplayer::new();
        let counts_per_step: Vec<(String, usize)> = wf
            .definition()
            .steps
            .iter()
            .map(|s| {
                (
                    s.id.clone(),
                    s.inputs.iter().filter(|b| b.show_in_review).count(),
                )
            })
            .collect();
        assert_eq!(
            counts_per_step,
            vec![
                ("create-stat".into(), 2),
                ("create-ruleset".into(), 2),
                ("create-session-template".into(), 2),
                ("create-match-pool".into(), 4),
                ("create-ams-fleet".into(), 2),
                ("update-session-template".into(), 0),
            ]
        );
    }

    #[test]
    fn test_competitive_multiplayer_inputs_declare_options_sources() {
        let def = build_definition();
        let by_name: std::collections::BTreeMap<_, _> =
            def.inputs.iter().map(|i| (i.name.as_str(), i)).collect();
        let img = by_name["fleetImageId"]
            .options_source
            .as_ref()
            .expect("fleetImageId options_source");
        assert_eq!(img.operation.operation.as_str(), "ams/admin/images/v1/list");
        assert_eq!(img.items_path, "$.images");
        assert_eq!(img.value, "$.id");
        assert_eq!(img.label.as_deref(), Some("$.name"));
        let inst = by_name["fleetInstanceId"]
            .options_source
            .as_ref()
            .expect("fleetInstanceId options_source");
        assert_eq!(
            inst.operation.operation.as_str(),
            "ams/admin/info/v1/list-supported-instances"
        );
        assert_eq!(inst.items_path, "$.availableInstanceTypes");
        assert_eq!(inst.value, "$.id");
        assert_eq!(inst.label.as_deref(), Some("$.name"));
        // `regions` is an array of plain strings, so the value path selects the
        // element itself (`$`) and the label falls back to that value.
        let region = by_name["fleetRegion"]
            .options_source
            .as_ref()
            .expect("fleetRegion options_source");
        assert_eq!(
            region.operation.operation.as_str(),
            "ams/admin/info/v1/list-regions"
        );
        assert_eq!(region.items_path, "$.regions");
        assert_eq!(region.value, "$");
        assert_eq!(region.label, None);
        // Required with no default, matching the fleetImageId / fleetInstanceId pickers.
        assert_eq!(by_name["fleetRegion"].default, None);
        assert!(by_name["fleetRegion"].required);
    }

    #[test]
    fn test_no_required_field_left_unbound() {
        let workflow = CompetitiveMultiplayer::new();
        let mut catalogue = Catalogue::new();
        let compiled = compile_workflow(workflow.definition(), &mut catalogue).unwrap();
        for step in &compiled.steps {
            let unbound: Vec<&str> = step
                .auto_derived
                .iter()
                .filter(|f| f.required)
                .map(|f| f.field.as_str())
                .collect();
            assert!(
                unbound.is_empty(),
                "step '{}' has unbound required fields: {unbound:?}",
                step.id
            );
        }
    }
}
