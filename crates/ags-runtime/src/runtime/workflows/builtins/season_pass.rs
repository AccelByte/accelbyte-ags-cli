//! The `season-pass` built-in workflow.
//!
//! Builds a complete, publishable season pass in a namespace's existing draft
//! store: a `/Season` category, a free and a premium SEASON pass item, a SEASON
//! tier item, then publishes the store, then the season with a free + premium
//! pass, six item rewards (free and premium tracks), and three tiers, and
//! publishes the season. It captures the created item ids and the season id
//! and pipes them forward.
//!
//! Locale is fixed to `en-US` (never bare `en`, which reproduces a silent
//! publish failure). SEASON items are priced in a picked virtual currency;
//! rewards are `type: ITEM`. Only the two publish steps are `confirm: true`.

use std::collections::BTreeMap;

use ags_protocol::catalogue::{OperationId, ServiceId};
use ags_protocol::workflow::{
    BindingSource, CaptureSource, CompletionResource, CompletionStep, LiteralBinding,
    OperationReference, OptionFilter, OptionParameterBinding, OptionsSource, ReferenceBinding,
    ReferenceTarget, StepDefinition, StepFieldLocation, StepInputBinding, StepOutputCapture,
    WorkflowBriefing, WorkflowCompletion, WorkflowDefinition, WorkflowId, WorkflowInputSpec,
    WorkflowOutputAlias,
};
use serde_json::{json, Value};

use crate::runtime::workflows::Workflow;

/// The `season-pass` built-in workflow.
pub struct SeasonPass {
    definition: WorkflowDefinition,
}

impl SeasonPass {
    /// Build the workflow with its hand-written definition.
    pub fn new() -> Self {
        Self {
            definition: build_definition(),
        }
    }
}

impl Default for SeasonPass {
    fn default() -> Self {
        Self::new()
    }
}

impl Workflow for SeasonPass {
    fn definition(&self) -> &WorkflowDefinition {
        &self.definition
    }
}

// ── Binding helpers ─────────────────────────────────────────────────────────

fn literal(value: Value) -> BindingSource {
    BindingSource::Literal(LiteralBinding {
        value,
        sensitive: false,
    })
}

fn workflow_ref(input: &str) -> BindingSource {
    BindingSource::Reference(ReferenceBinding {
        from: ReferenceTarget::Workflow {
            input: input.to_string(),
        },
        output: None,
        transform: None,
    })
}

fn step_ref(step_id: &str, output: &str) -> BindingSource {
    BindingSource::Reference(ReferenceBinding {
        from: ReferenceTarget::Step {
            id: step_id.to_string(),
        },
        output: Some(output.to_string()),
        transform: None,
    })
}

fn bind(field: &str, source: BindingSource) -> StepInputBinding {
    StepInputBinding {
        field: field.to_string(),
        source,
        show_in_review: false,
        description: None,
    }
}

fn bind_visible(field: &str, source: BindingSource) -> StepInputBinding {
    StepInputBinding {
        field: field.to_string(),
        source,
        show_in_review: true,
        description: None,
    }
}

// ── Step / input helpers ─────────────────────────────────────────────────────

/// Build one step. `service` is the manifest internal id (`platform` or
/// `seasonpass`). `confirm`/`is_optional`/`continue_on_failure` default false.
#[allow(clippy::too_many_arguments)]
fn step(
    service: &str,
    id: &str,
    description: &str,
    operation: &str,
    dependencies: Vec<String>,
    confirm: bool,
    inputs: Vec<StepInputBinding>,
    outputs: Vec<StepOutputCapture>,
) -> StepDefinition {
    StepDefinition {
        id: id.to_string(),
        description: Some(description.to_string()),
        operation: OperationReference {
            service: ServiceId::new(service),
            operation: OperationId::new(operation),
        },
        dependencies,
        confirm,
        is_optional: false,
        continue_on_failure: false,
        skip_if_exists: false,
        is_reviewed: None,
        inputs,
        outputs,
    }
}

fn capture(name: &str, path: &str) -> StepOutputCapture {
    StepOutputCapture {
        name: name.to_string(),
        source: CaptureSource::ResponseBody {
            path: path.to_string(),
        },
        default: None,
        sensitive: false,
    }
}

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
        location: StepFieldLocation::Body,
    }
}

fn input_with_options(
    name: &str,
    description: &str,
    required: bool,
    schema: Value,
    options_source: OptionsSource,
) -> WorkflowInputSpec {
    WorkflowInputSpec {
        name: name.to_string(),
        description: Some(description.to_string()),
        schema: Some(schema),
        required,
        default: None,
        sensitive: false,
        options_source: Some(options_source),
        location: StepFieldLocation::Body,
    }
}

/// Root-array picker over `platform/admin/stores/v1/list`.
fn store_options() -> OptionsSource {
    OptionsSource {
        operation: OperationReference {
            service: ServiceId::new("platform"),
            operation: OperationId::new("platform/admin/stores/v1/list"),
        },
        parameters: BTreeMap::from([(
            "namespace".to_string(),
            OptionParameterBinding::FromInput("namespace".to_string()),
        )]),
        items_path: "$".to_string(),
        value: "$.storeId".to_string(),
        label: Some("$.title".to_string()),
        label_detail: None,
        fallback_description: None,
        filter: Some(OptionFilter {
            path: "$.published".to_string(),
            equals: json!(false),
        }),
    }
}

/// Root-array picker over `platform/admin/currencies/v1/list`, VIRTUAL only.
fn currency_options() -> OptionsSource {
    OptionsSource {
        operation: OperationReference {
            service: ServiceId::new("platform"),
            operation: OperationId::new("platform/admin/currencies/v1/list"),
        },
        parameters: BTreeMap::from([
            (
                "namespace".to_string(),
                OptionParameterBinding::FromInput("namespace".to_string()),
            ),
            (
                "currencyType".to_string(),
                OptionParameterBinding::Literal(json!("VIRTUAL")),
            ),
        ]),
        items_path: "$".to_string(),
        value: "$.currencyCode".to_string(),
        label: Some("$.currencyCode".to_string()),
        label_detail: None,
        fallback_description: None,
        filter: None,
    }
}

/// Dependent picker over the chosen store's INGAMEITEM items.
fn reward_options() -> OptionsSource {
    OptionsSource {
        operation: OperationReference {
            service: ServiceId::new("platform"),
            operation: OperationId::new("platform/admin/items/v1/list"),
        },
        parameters: BTreeMap::from([
            (
                "namespace".to_string(),
                OptionParameterBinding::FromInput("namespace".to_string()),
            ),
            (
                "storeId".to_string(),
                OptionParameterBinding::FromInput("storeId".to_string()),
            ),
            (
                "itemType".to_string(),
                OptionParameterBinding::Literal(json!("INGAMEITEM")),
            ),
        ]),
        items_path: "$.data".to_string(),
        value: "$.itemId".to_string(),
        label: Some("$.name".to_string()),
        label_detail: None,
        fallback_description: None,
        filter: None,
    }
}

/// One SEASON item create step, capturing its `$.itemId` into `capture_name`.
/// Priced in the picked virtual currency. `title`/`price` are reviewable.
fn season_item_step(
    id: &str,
    capture_name: &str,
    name: &str,
    title: &str,
    season_type: &str,
    price: i64,
) -> StepDefinition {
    step(
        "platform",
        id,
        "Creates a SEASON store item and captures its id for the season pass.",
        "platform/admin/items/v1/create",
        vec!["create-category".to_string()],
        false,
        vec![
            bind("namespace", workflow_ref("namespace")),
            bind("storeId", workflow_ref("storeId")),
            bind("name", literal(json!(name))),
            bind("itemType", literal(json!("SEASON"))),
            bind("seasonType", literal(json!(season_type))),
            bind("entitlementType", literal(json!("DURABLE"))),
            bind("status", literal(json!("ACTIVE"))),
            bind("categoryPath", literal(json!("/Season"))),
            bind("localizations", literal(json!({}))),
            bind_visible("localizations.en-US.title", literal(json!(title))),
            bind(
                "regionData",
                literal(json!({"US": [{"currencyType": "VIRTUAL"}]})),
            ),
            bind(
                "regionData.US[0].currencyCode",
                workflow_ref("currencyCode"),
            ),
            bind(
                "regionData.US[0].currencyNamespace",
                workflow_ref("namespace"),
            ),
            bind_visible("regionData.US[0].price", literal(json!(price))),
        ],
        vec![capture(capture_name, "$.itemId")],
    )
}

/// One item reward granting `item_input`'s item in `quantity` (reviewable).
fn reward_step(id: &str, code: &str, item_input: &str, quantity: i64) -> StepDefinition {
    step(
        "seasonpass",
        id,
        "Creates an item reward granted on a pass's tiers.",
        "season-pass/admin/rewards/v1/create",
        vec!["create-season".to_string()],
        false,
        vec![
            bind("namespace", workflow_ref("namespace")),
            bind("seasonId", step_ref("create-season", "seasonId")),
            bind("type", literal(json!("ITEM"))),
            bind("itemId", workflow_ref(item_input)),
            bind("code", literal(json!(code))),
            bind_visible("quantity", literal(json!(quantity))),
        ],
        vec![],
    )
}

/// Tier `n` (1-based): unlocks at `required_exp` (reviewable) and grants the
/// free reward on the free pass and the premium reward on the premium pass.
/// `tier.rewards` is a whole literal (no capture leaves).
fn tier_step(n: i64, required_exp: i64) -> StepDefinition {
    let free_code = format!("reward-free-{n}");
    let premium_code = format!("reward-premium-{n}");
    step(
        "seasonpass",
        &format!("create-tier-{n}"),
        "Creates a tier that unlocks at an experience threshold and grants its rewards on the free and premium passes.",
        "season-pass/admin/tiers/v1/create",
        vec![
            "create-season".to_string(),
            "create-pass-free".to_string(),
            "create-pass-premium".to_string(),
            format!("create-reward-free-{n}"),
            format!("create-reward-premium-{n}"),
        ],
        false,
        vec![
            bind("namespace", workflow_ref("namespace")),
            bind("seasonId", step_ref("create-season", "seasonId")),
            bind("index", literal(json!(n - 1))),
            bind("quantity", literal(json!(1))),
            bind("tier", literal(json!({}))),
            bind_visible("tier.requiredExp", literal(json!(required_exp))),
            bind(
                "tier.rewards",
                literal(json!({ "free-pass": [free_code], "premium-pass": [premium_code] })),
            ),
        ],
        vec![],
    )
}

// ── Definition ───────────────────────────────────────────────────────────────

fn build_definition() -> WorkflowDefinition {
    WorkflowDefinition {
        id: WorkflowId::new("season-pass"),
        name: "Create a season pass".to_string(),
        intent: Some(
            "season pass battle pass tier reward premium free progression seasonpass store publish live"
                .to_string(),
        ),
        description: Some(
            "Create a complete season pass in your draft store with a free and premium pass, item rewards on both tracks, and three tiers, then publish it."
                .to_string(),
        ),
        briefing: Some(WorkflowBriefing {
            overview: concat!(
                "This workflow builds a complete **season pass** in your namespace's ",
                "existing draft store and publishes it so it goes live. It creates the ",
                "SEASON store items, publishes the store, then creates the season with a ",
                "**free** and a **premium** pass, **item rewards** on both tracks, and three **tiers**.\n\n",

                "You pick the draft store, the virtual currency the SEASON items are priced ",
                "in, and the in-game item to grant as the reward. Language is fixed to ",
                "**English (`en-US`)** and prices are set for the **US region** only. This is ",
                "a deliberately opinionated starting point. To add more passes, tiers, or ",
                "rewards, use the `ags season-pass` commands after the run. To price other ",
                "regions, use `ags platform items update` afterwards.\n\n",

                "**The last step publishes the season.** It makes the passes, rewards, and ",
                "tiers live for players."
            )
            .into(),
            prerequisites: vec![
                concat!(
                    "**A draft store** in the namespace, with a virtual currency and at ",
                    "least one in-game item. Run the `in-game-store` workflow first if you ",
                    "have not. You pick these during the run."
                )
                .into(),
                concat!(
                    "**Admin credentials** for the `platform` and `season-pass` services. ",
                    "The workflow uses your current `ags` login session."
                )
                .into(),
            ],
            creates: vec![
                "**A `/Season` category** to hold the SEASON items.".into(),
                "**A free and a premium SEASON pass item**, priced in the chosen currency (the free one at 0).".into(),
                "**A SEASON tier item** for progression.".into(),
                "**A season** with a free pass and a premium pass.".into(),
                "**Six item rewards** on the free and premium tracks, in escalating quantity (premium gets more).".into(),
                "**Three tiers**, each unlocking at an escalating experience threshold.".into(),
                "**The publishes.** Makes the store items and then the season live.".into(),
            ],
        }),
        is_reviewed_by_default: true,
        inputs: vec![
            input(
                "namespace",
                "Your game's AccelByte namespace. This is the same value used by the global --namespace flag.",
                true,
                None,
                json!({"type": "string"}),
            ),
            input(
                "seasonName",
                "The season's name, shown to players. Defaults to Season 1.",
                false,
                Some(json!("Season 1")),
                json!({"type": "string"}),
            ),
            input(
                "start",
                "When the season starts, as an ISO 8601 date-time, for example 2026-08-01T00:00:00Z.",
                false,
                Some(json!("2020-01-01T00:00:00Z")),
                json!({"type": "string", "format": "date-time"}),
            ),
            input(
                "end",
                "When the season ends, as an ISO 8601 date-time, for example 2026-12-31T23:59:59Z.",
                false,
                Some(json!("2099-12-31T23:59:59Z")),
                json!({"type": "string", "format": "date-time"}),
            ),
            input_with_options(
                "storeId",
                "Pick the draft store to build the season pass in. This is a picker.",
                true,
                json!({"type": "string"}),
                store_options(),
            ),
            input_with_options(
                "currencyCode",
                "Pick the virtual currency to price the SEASON items in. This is a picker.",
                true,
                json!({"type": "string"}),
                currency_options(),
            ),
            input_with_options(
                "freeRewardItemId",
                "Pick the in-game item to grant on the free pass tiers. This is a picker.",
                true,
                json!({"type": "string"}),
                reward_options(),
            ),
            input_with_options(
                "premiumRewardItemId",
                "Pick the in-game item to grant on the premium pass tiers. This is a picker.",
                true,
                json!({"type": "string"}),
                reward_options(),
            ),
        ],
        steps: vec![
            // 1 — category for the SEASON items.
            step(
                "platform",
                "create-category",
                "Creates the /Season category that holds the SEASON items.",
                "platform/admin/categories/v1/create",
                vec![],
                false,
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("storeId", workflow_ref("storeId")),
                    bind("categoryPath", literal(json!("/Season"))),
                    bind("localizationDisplayNames", literal(json!({}))),
                    bind("localizationDisplayNames.en-US", literal(json!("Season"))),
                ],
                vec![],
            ),
            // 2 — free pass item (price 0).
            season_item_step(
                "create-pass-item-free",
                "freePassItemId",
                "Season Free Pass",
                "Free Pass",
                "PASS",
                0,
            ),
            // 3 — premium pass item (priced).
            season_item_step(
                "create-pass-item-premium",
                "premiumPassItemId",
                "Season Premium Pass",
                "Premium Pass",
                "PASS",
                500,
            ),
            // 4 — tier item.
            season_item_step(
                "create-tier-item",
                "tierItemId",
                "Season Tier",
                "Tier Upgrade",
                "TIER",
                100,
            ),
            // 5 — publish the store (mutating). confirm + optional (user may
            //     skip to leave the store in draft).
            StepDefinition {
                is_optional: true,
                ..step(
                    "platform",
                    "publish-store",
                    "Publishes the draft store so the SEASON items are live for the season.",
                    "platform/admin/catalog-changes/v1/publish-all",
                    vec![
                        "create-pass-item-free".to_string(),
                        "create-pass-item-premium".to_string(),
                        "create-tier-item".to_string(),
                    ],
                    true,
                    vec![
                        bind("namespace", workflow_ref("namespace")),
                        bind("storeId", workflow_ref("storeId")),
                    ],
                    vec![],
                )
            },
            // 6 — the season, referencing the captured tier item.
            step(
                "seasonpass",
                "create-season",
                "Creates the draft season in the store. The season id is captured for later steps.",
                "season-pass/admin/seasons/v1/create",
                vec!["create-tier-item".to_string(), "publish-store".to_string()],
                false,
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("draftStoreId", workflow_ref("storeId")),
                    bind("tierItemId", step_ref("create-tier-item", "tierItemId")),
                    bind("name", workflow_ref("seasonName")),
                    bind("defaultLanguage", literal(json!("en-US"))),
                    bind("defaultRequiredExp", literal(json!(100))),
                    bind("localizations", literal(json!({}))),
                    bind("localizations.en-US.title", workflow_ref("seasonName")),
                    bind("start", workflow_ref("start")),
                    bind("end", workflow_ref("end")),
                ],
                vec![capture("seasonId", "$.id")],
            ),
            // 7 — free pass (auto-enroll), referencing its captured pass item.
            step(
                "seasonpass",
                "create-pass-free",
                "Creates the free pass players auto-enroll into.",
                "season-pass/admin/passes/v1/create",
                vec![
                    "create-season".to_string(),
                    "create-pass-item-free".to_string(),
                ],
                false,
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("seasonId", step_ref("create-season", "seasonId")),
                    bind("code", literal(json!("free-pass"))),
                    bind("autoEnroll", literal(json!(true))),
                    bind("displayOrder", literal(json!(1))),
                    bind(
                        "passItemId",
                        step_ref("create-pass-item-free", "freePassItemId"),
                    ),
                    bind("localizations", literal(json!({}))),
                    bind("localizations.en-US.title", literal(json!("Free Pass"))),
                ],
                vec![],
            ),
            // 8 — premium pass, referencing its captured pass item.
            step(
                "seasonpass",
                "create-pass-premium",
                "Creates the premium pass, referencing the premium pass item.",
                "season-pass/admin/passes/v1/create",
                vec![
                    "create-season".to_string(),
                    "create-pass-item-premium".to_string(),
                ],
                false,
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("seasonId", step_ref("create-season", "seasonId")),
                    bind("code", literal(json!("premium-pass"))),
                    bind("autoEnroll", literal(json!(false))),
                    bind("displayOrder", literal(json!(2))),
                    bind(
                        "passItemId",
                        step_ref("create-pass-item-premium", "premiumPassItemId"),
                    ),
                    bind("localizations", literal(json!({}))),
                    bind("localizations.en-US.title", literal(json!("Premium Pass"))),
                ],
                vec![],
            ),
            // 9-14 — item rewards: free track (smaller) + premium track (larger).
            reward_step("create-reward-free-1", "reward-free-1", "freeRewardItemId", 5),
            reward_step("create-reward-free-2", "reward-free-2", "freeRewardItemId", 10),
            reward_step("create-reward-free-3", "reward-free-3", "freeRewardItemId", 25),
            reward_step("create-reward-premium-1", "reward-premium-1", "premiumRewardItemId", 10),
            reward_step("create-reward-premium-2", "reward-premium-2", "premiumRewardItemId", 25),
            reward_step("create-reward-premium-3", "reward-premium-3", "premiumRewardItemId", 50),
            // 15-17 — tiers, granting the free reward on the free pass and the
            // premium reward on the premium pass.
            tier_step(1, 100),
            tier_step(2, 250),
            tier_step(3, 500),
            // 18 — publish the season (mutating). confirm + optional (user may
            //     skip to leave the season in draft).
            StepDefinition {
                is_optional: true,
                ..step(
                    "seasonpass",
                    "publish-season",
                    "Publishes the season, making its passes, rewards, and tiers live.",
                    "season-pass/admin/seasons/v1/publish",
                    vec![
                        "create-tier-1".to_string(),
                        "create-tier-2".to_string(),
                        "create-tier-3".to_string(),
                    ],
                    true,
                    vec![
                        bind("namespace", workflow_ref("namespace")),
                        bind("seasonId", step_ref("create-season", "seasonId")),
                    ],
                    vec![],
                )
            },
        ],
        outputs: vec![WorkflowOutputAlias {
            name: "seasonId".to_string(),
            from_step_id: "create-season".to_string(),
            output: "seasonId".to_string(),
            sensitive: false,
            section: Some("Season".to_string()),
            label: Some("Season id".to_string()),
            item_fields: None,
        }],
        completion: Some(WorkflowCompletion {
            created: vec![
                CompletionResource {
                    label: "Category".into(),
                    value: "/Season".into(),
                },
                CompletionResource {
                    label: "Season".into(),
                    value: "{seasonName}".into(),
                },
                CompletionResource {
                    label: "Passes".into(),
                    value: "free-pass, premium-pass".into(),
                },
                CompletionResource {
                    label: "Rewards".into(),
                    value: "6 (free and premium, tiers 1 to 3)".into(),
                },
                CompletionResource {
                    label: "Tiers".into(),
                    value: "3 (exp 100, 250, 500)".into(),
                },
            ],
            next_steps: vec![
                // The workflow already publishes the season (final publish-season
                // step), so suggest viewing it, not publishing again (a re-publish
                // 409s). Mirrors in-game-store's "View the published store".
                CompletionStep {
                    description: "View the published season".into(),
                    command: "ags season-pass seasons get --namespace {namespace} --season-id <season-id>".into(),
                },
                CompletionStep {
                    description: "Unpublish to edit, then publish again".into(),
                    command: "ags season-pass seasons unpublish --namespace {namespace} --season-id <season-id>".into(),
                },
                CompletionStep {
                    description: "Add another reward tier".into(),
                    command: "ags season-pass tiers create --namespace {namespace} --season-id <season-id> --json '<tier-json>'".into(),
                },
                CompletionStep {
                    description: "Change a tier's rewards".into(),
                    command: "ags season-pass tiers update --namespace {namespace} --season-id <season-id> --id <tier-id> --json '<tier-json>'".into(),
                },
                CompletionStep {
                    description: "View this season's tiers".into(),
                    command: "ags season-pass tiers list --namespace {namespace} --season-id <season-id>".into(),
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

    fn json_has_key(value: &Value, key: &str) -> bool {
        match value {
            Value::Object(map) => {
                map.contains_key(key) || map.values().any(|v| json_has_key(v, key))
            }
            Value::Array(items) => items.iter().any(|v| json_has_key(v, key)),
            _ => false,
        }
    }

    #[test]
    fn test_season_pass_compiles_against_bundled_catalogue() {
        let workflow = SeasonPass::new();
        let mut catalogue = Catalogue::new();
        let compiled = compile_workflow(workflow.definition(), &mut catalogue)
            .expect("must compile against the bundled catalogue");
        assert_eq!(compiled.steps.len(), 18);
    }

    #[test]
    fn test_season_pass_inputs_and_picker_params() {
        let def = SeasonPass::new();
        let inputs = &def.definition().inputs;
        let names: Vec<&str> = inputs.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "namespace",
                "seasonName",
                "start",
                "end",
                "storeId",
                "currencyCode",
                "freeRewardItemId",
                "premiumRewardItemId"
            ]
        );
        for name in [
            "storeId",
            "currencyCode",
            "freeRewardItemId",
            "premiumRewardItemId",
        ] {
            let spec = inputs.iter().find(|i| i.name == name).unwrap();
            assert!(spec.required, "{name} must be required");
            assert!(spec.options_source.is_some(), "{name} must be a picker");
        }
        let by_name: std::collections::BTreeMap<_, _> =
            inputs.iter().map(|i| (i.name.as_str(), i)).collect();
        // Store + currency are root-array pickers.
        assert_eq!(
            by_name["storeId"]
                .options_source
                .as_ref()
                .unwrap()
                .items_path,
            "$"
        );
        let cur = by_name["currencyCode"].options_source.as_ref().unwrap();
        assert_eq!(cur.items_path, "$");
        assert!(matches!(
            cur.parameters.get("currencyType"),
            Some(OptionParameterBinding::Literal(v)) if v == &json!("VIRTUAL")
        ));
        // Both reward pickers are store-scoped INGAMEITEM.
        for name in ["freeRewardItemId", "premiumRewardItemId"] {
            let rew = by_name[name].options_source.as_ref().unwrap();
            assert_eq!(rew.items_path, "$.data", "{name} items_path");
            assert!(
                matches!(
                    rew.parameters.get("storeId"),
                    Some(OptionParameterBinding::FromInput(i)) if i == "storeId"
                ),
                "{name} storeId param"
            );
            assert!(
                matches!(
                    rew.parameters.get("itemType"),
                    Some(OptionParameterBinding::Literal(v)) if v == &json!("INGAMEITEM")
                ),
                "{name} itemType param"
            );
        }
    }

    #[test]
    fn test_captures_and_downstream_references() {
        let def = SeasonPass::new();
        let steps = &def.definition().steps;
        let cap = |id: &str, name: &str, path: &str| {
            let s = steps.iter().find(|s| s.id == id).unwrap();
            let c = s.outputs.iter().find(|c| c.name == name).unwrap();
            assert!(matches!(&c.source, CaptureSource::ResponseBody { path: p } if p == path));
            assert!(c.default.is_none(), "{name} capture has no default");
        };
        cap("create-pass-item-free", "freePassItemId", "$.itemId");
        cap("create-pass-item-premium", "premiumPassItemId", "$.itemId");
        cap("create-tier-item", "tierItemId", "$.itemId");
        cap("create-season", "seasonId", "$.id");
        // Each pass binds its own captured pass item.
        let step_binds = |id: &str, field: &str, src_step: &str, out: &str| {
            let s = steps.iter().find(|s| s.id == id).unwrap();
            let b = s.inputs.iter().find(|b| b.field == field).unwrap();
            assert!(
                matches!(
                    &b.source,
                    BindingSource::Reference(r)
                        if matches!(&r.from, ReferenceTarget::Step { id } if id == src_step)
                            && r.output.as_deref() == Some(out)
                ),
                "{id}.{field} must ref {src_step}.{out}"
            );
        };
        step_binds(
            "create-pass-free",
            "passItemId",
            "create-pass-item-free",
            "freePassItemId",
        );
        step_binds(
            "create-pass-premium",
            "passItemId",
            "create-pass-item-premium",
            "premiumPassItemId",
        );
        step_binds(
            "create-season",
            "tierItemId",
            "create-tier-item",
            "tierItemId",
        );
    }

    #[test]
    fn test_only_publish_steps_confirm() {
        let def = SeasonPass::new();
        let confirming: Vec<&str> = def
            .definition()
            .steps
            .iter()
            .filter(|s| s.confirm)
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(confirming, ["publish-store", "publish-season"]);
        let optional_ids: Vec<&str> = def
            .definition()
            .steps
            .iter()
            .filter(|s| s.is_optional)
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(
            optional_ids,
            ["publish-store", "publish-season"],
            "exactly the two publish steps must be optional"
        );
        assert!(
            def.definition()
                .steps
                .iter()
                .filter(|s| s.id != "publish-store" && s.id != "publish-season")
                .all(|s| !s.is_optional),
            "all non-publish steps must not be optional"
        );
    }

    #[test]
    fn test_season_items_carry_season_type() {
        let def = SeasonPass::new();
        let steps = &def.definition().steps;
        for (id, ty) in [
            ("create-pass-item-free", "PASS"),
            ("create-pass-item-premium", "PASS"),
            ("create-tier-item", "TIER"),
        ] {
            let s = steps.iter().find(|s| s.id == id).unwrap();
            let st = s.inputs.iter().find(|b| b.field == "seasonType").unwrap();
            assert!(matches!(&st.source, BindingSource::Literal(l) if l.value == json!(ty)));
            let et = s
                .inputs
                .iter()
                .find(|b| b.field == "entitlementType")
                .unwrap();
            assert!(matches!(&et.source, BindingSource::Literal(l) if l.value == json!("DURABLE")));
        }
    }

    #[test]
    fn test_locale_is_en_us_not_bare_en() {
        let def = SeasonPass::new();
        for s in &def.definition().steps {
            for b in &s.inputs {
                // The `en-US` locale lives mostly in binding FIELD PATHS
                // (`localizations.en-US.title`), so scan the field segments for a
                // bare `en` — a bare `en` reproduces the silent 30122 publish
                // failure. `en-US` splits to the segment `en-US`, never `en`.
                assert!(
                    !b.field.split(['.', '[']).any(|seg| seg == "en"),
                    "step '{}' field '{}' uses a bare `en` locale segment",
                    s.id,
                    b.field
                );
                if let BindingSource::Literal(l) = &b.source {
                    assert!(
                        !json_has_key(&l.value, "en"),
                        "step '{}' field '{}' uses a bare `en` locale key",
                        s.id,
                        b.field
                    );
                }
            }
        }
        let season = def
            .definition()
            .steps
            .iter()
            .find(|s| s.id == "create-season")
            .unwrap();
        let lang = season
            .inputs
            .iter()
            .find(|b| b.field == "defaultLanguage")
            .unwrap();
        assert!(matches!(&lang.source, BindingSource::Literal(l) if l.value == json!("en-US")));
    }

    #[test]
    fn test_tier_rewards_wire_free_and_premium_tracks() {
        let def = SeasonPass::new();
        let steps = &def.definition().steps;
        for (id, free_code, premium_code) in [
            ("create-tier-1", "reward-free-1", "reward-premium-1"),
            ("create-tier-2", "reward-free-2", "reward-premium-2"),
            ("create-tier-3", "reward-free-3", "reward-premium-3"),
        ] {
            let s = steps.iter().find(|s| s.id == id).unwrap();
            let rewards = s.inputs.iter().find(|b| b.field == "tier.rewards").unwrap();
            assert!(
                matches!(
                    &rewards.source,
                    BindingSource::Literal(l)
                        if l.value == json!({ "free-pass": [free_code], "premium-pass": [premium_code] })
                ),
                "{id} must grant the free reward to free-pass and premium reward to premium-pass"
            );
            let tier = s.inputs.iter().find(|b| b.field == "tier").unwrap();
            assert!(matches!(&tier.source, BindingSource::Literal(l) if l.value == json!({})));
        }
    }

    #[test]
    fn test_reward_steps_wire_free_and_premium_item_tracks() {
        // The two reward tracks bind different item inputs and escalating
        // quantities. A transposition of the free/premium item input, or a wrong
        // quantity, compiles and passes every other test (show_in_review counts
        // and tier.rewards keys are identical either way), so assert the wiring
        // here. Free track 5/10/25, premium 10/25/50.
        let def = SeasonPass::new();
        let steps = &def.definition().steps;
        for (id, item_input, quantity) in [
            ("create-reward-free-1", "freeRewardItemId", 5),
            ("create-reward-free-2", "freeRewardItemId", 10),
            ("create-reward-free-3", "freeRewardItemId", 25),
            ("create-reward-premium-1", "premiumRewardItemId", 10),
            ("create-reward-premium-2", "premiumRewardItemId", 25),
            ("create-reward-premium-3", "premiumRewardItemId", 50),
        ] {
            let s = steps.iter().find(|s| s.id == id).unwrap();
            let item = s.inputs.iter().find(|b| b.field == "itemId").unwrap();
            assert!(
                matches!(
                    &item.source,
                    BindingSource::Reference(r)
                        if matches!(&r.from, ReferenceTarget::Workflow { input } if input == item_input)
                ),
                "{id} must bind itemId to the {item_input} input"
            );
            let qty = s.inputs.iter().find(|b| b.field == "quantity").unwrap();
            assert!(
                matches!(&qty.source, BindingSource::Literal(l) if l.value == json!(quantity)),
                "{id} must grant quantity {quantity}"
            );
        }
    }

    #[test]
    fn test_show_in_review_counts_match_curation() {
        let def = SeasonPass::new();
        let counts: Vec<(String, usize)> = def
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
            counts,
            vec![
                ("create-category".into(), 0),
                ("create-pass-item-free".into(), 2),
                ("create-pass-item-premium".into(), 2),
                ("create-tier-item".into(), 2),
                ("publish-store".into(), 0),
                ("create-season".into(), 0),
                ("create-pass-free".into(), 0),
                ("create-pass-premium".into(), 0),
                ("create-reward-free-1".into(), 1),
                ("create-reward-free-2".into(), 1),
                ("create-reward-free-3".into(), 1),
                ("create-reward-premium-1".into(), 1),
                ("create-reward-premium-2".into(), 1),
                ("create-reward-premium-3".into(), 1),
                ("create-tier-1".into(), 1),
                ("create-tier-2".into(), 1),
                ("create-tier-3".into(), 1),
                ("publish-season".into(), 0),
            ]
        );
    }

    #[test]
    fn test_no_required_field_left_unbound() {
        let workflow = SeasonPass::new();
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
                "step '{}' unbound: {unbound:?}",
                step.id
            );
        }
    }

    #[test]
    fn test_completion_and_output_alias_present() {
        let def = SeasonPass::new();
        assert!(def.definition().completion.is_some());
        let alias = def
            .definition()
            .outputs
            .iter()
            .find(|a| a.name == "seasonId")
            .expect("seasonId alias");
        assert_eq!(alias.from_step_id, "create-season");
    }

    #[test]
    fn test_store_options_filters_to_draft_stores() {
        let source = store_options();
        let filter = source
            .filter
            .expect("store picker must filter to draft stores");
        assert_eq!(filter.path, "$.published");
        assert_eq!(filter.equals, json!(false));
    }
}
