//! The `in-game-store` built-in workflow.
//!
//! Creates a structured in-game store in one run: a draft store, a virtual
//! soft currency, a three-category tree (`/InGameStore` with `/Durable`
//! and `/Consumable` leaves), a durable item and a consumable item priced
//! in that currency, then publishes the store.  It is the first built-in
//! to capture a created resource id (`storeId`) and pipe it into later
//! steps — into the category and item query params and the publish path
//! param.
//!
//! Each item's `name` mirrors its reviewable `en-US` display title, so a
//! title typed at the step review reflects in the Admin Portal's item name
//! too. The final names are captured back from the create-item responses
//! and surfaced as `Items` output aliases, keeping the run output, the
//! summary, and the portal in agreement.
//!
//! Language and region are fixed to the `en-US` locale and the `US` region
//! so the store and items always agree (the platform API validates the
//! item's localization language and region against the store's supported
//! set). The locale is `en-US`, not bare `en`, because publishing requires
//! the draft to match the namespace's existing published store, whose
//! default locale is `en-US`. The final publish step is `confirm: true`
//! because it changes the namespace's live published catalog.

use ags_protocol::catalogue::{OperationId, ServiceId};
use ags_protocol::workflow::{
    BindingSource, CaptureSource, CompletionResource, CompletionStep, LiteralBinding,
    MirrorBinding, OperationReference, ReferenceBinding, ReferenceTarget, StepDefinition,
    StepInputBinding, StepOutputCapture, WorkflowBriefing, WorkflowCompletion, WorkflowDefinition,
    WorkflowId, WorkflowInputSpec, WorkflowOutputAlias,
};
use serde_json::{json, Value};

use crate::runtime::workflows::Workflow;

/// The `in-game-store` built-in workflow.
pub struct InGameStore {
    definition: WorkflowDefinition,
}

impl InGameStore {
    /// Build the workflow with its hand-written definition.
    pub fn new() -> Self {
        Self {
            definition: build_definition(),
        }
    }
}

impl Default for InGameStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Workflow for InGameStore {
    fn definition(&self) -> &WorkflowDefinition {
        &self.definition
    }
}

// ── Binding helpers ─────────────────────────────────────────────────────────

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

/// A `from: step/<id>, output: <name>` reference binding — pipes a captured
/// step output into a later step's field.
fn step_ref(step_id: &str, output: &str) -> BindingSource {
    BindingSource::Reference(ReferenceBinding {
        from: ReferenceTarget::Step {
            id: step_id.to_string(),
        },
        output: Some(output.to_string()),
        transform: None,
    })
}

/// A `{mirror_of: <field>}` binding — copies the effective value of another
/// same-step field, including any step-review edit to it.
fn mirror_of(field: &str) -> BindingSource {
    BindingSource::Mirror(MirrorBinding {
        mirror_of: field.to_string(),
    })
}

/// Pair an operation field name with its value source (hidden from review).
fn bind(field: &str, source: BindingSource) -> StepInputBinding {
    StepInputBinding {
        field: field.to_string(),
        source,
        show_in_review: false,
        description: None,
    }
}

/// Like `bind`, but the resolved field is shown in the step-walk review form
/// even though its source is a literal.
fn bind_visible(field: &str, source: BindingSource) -> StepInputBinding {
    StepInputBinding {
        field: field.to_string(),
        source,
        show_in_review: true,
        description: None,
    }
}

// ── Step / input helpers ─────────────────────────────────────────────────────

/// Per-step behaviour flags. Both default false, so a call site names only the
/// flag it sets (`StepFlags { skip_if_exists: true, ..Default::default() }`).
/// A struct rather than two positional `bool`s, which are silently
/// transposable at a call site with no compiler error.
#[derive(Default)]
struct StepFlags {
    /// Require interactive confirmation before this step dispatches.
    confirm: bool,
    /// Auto-skip an already-exists 409 so re-running the workflow is idempotent.
    skip_if_exists: bool,
}

/// Build one step. `flags` default false; `is_optional` and
/// `continue_on_failure` default false; `is_reviewed` inherits the workflow
/// default. Pass `outputs` for capture steps.
fn step(
    id: &str,
    description: &str,
    operation: &str,
    dependencies: Vec<String>,
    flags: StepFlags,
    inputs: Vec<StepInputBinding>,
    outputs: Vec<StepOutputCapture>,
) -> StepDefinition {
    StepDefinition {
        id: id.to_string(),
        description: Some(description.to_string()),
        operation: OperationReference {
            service: ServiceId::new("platform"),
            operation: OperationId::new(operation),
        },
        dependencies,
        confirm: flags.confirm,
        is_optional: false,
        continue_on_failure: false,
        skip_if_exists: flags.skip_if_exists,
        is_reviewed: None,
        inputs,
        outputs,
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

// ── Definition ───────────────────────────────────────────────────────────────

fn build_definition() -> WorkflowDefinition {
    WorkflowDefinition {
        id: WorkflowId::new("in-game-store"),
        name: "Create an in-game store".to_string(),
        intent: Some(
            "store catalog economy virtual currency soft currency item category durable consumable publish platform in-game"
                .to_string(),
        ),
        description: Some(
            "Create a structured in-game store with a durable and a consumable item priced in a virtual soft currency, then publish it."
                .to_string(),
        ),
        briefing: Some(WorkflowBriefing {
            overview: concat!(
                "This workflow creates a **structured in-game store** priced in a ",
                "**virtual soft currency**, then publishes it so the store goes live. ",
                "It makes the ordered platform calls and wires the resources together, ",
                "so you get a working, organised store you can build on.\n\n",

                "It builds a small category tree under `/InGameStore` with a **Durable** ",
                "and a **Consumable** section, then adds one item of each kind priced in the ",
                "currency. This is a deliberately small, opinionated starting point. It uses ",
                "the **English (`en-US`)** locale and the **US** region only. It is priced ",
                "in a virtual soft currency with no real money or payment setup. To add ",
                "more items, categories, languages, or regions, use the `ags platform` ",
                "commands after the run.\n\n",

                "**The final step publishes the store.** It applies the changes in the draft ",
                "store to the namespace's published store, making the new categories and ",
                "items live for players."
            )
            .into(),
            prerequisites: vec![
                concat!(
                    "**A namespace** already created. Everything this workflow ",
                    "creates lives inside that namespace."
                )
                .into(),
                concat!(
                    "**Admin credentials** with permission to manage the ",
                    "`platform` service (stores, currencies, categories, and ",
                    "items). The workflow uses your current `ags` login session."
                )
                .into(),
                concat!(
                    "**A namespace using the standard `en-US` store locale.** This ",
                    "workflow creates its store in `en-US` to match the published ",
                    "store. A namespace set up with a different default locale ",
                    "would need manual adjustment before publishing succeeds."
                )
                .into(),
            ],
            creates: vec![
                "**A draft store.** It holds the categories and items, and stays a draft until the final publish step.".into(),
                "**A virtual soft currency.** Players spend this on items. It defaults to `GOLD` with no decimal places.".into(),
                "**A category tree.** A `/InGameStore` root with a `Durable` and a `Consumable` section.".into(),
                "**A durable item.** A permanent in-game item filed under Durable, priced in the soft currency.".into(),
                "**A consumable item.** A single-use in-game item filed under Consumable, priced in the soft currency.".into(),
                "**The publish.** Makes the store's categories and items live by publishing them to the namespace's published store.".into(),
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
                "currencyCode",
                "The code of the virtual soft currency to create. Uppercase letters, for example `GOLD`. Items are priced in this currency.",
                false,
                Some(json!("GOLD")),
                json!({"type": "string"}),
            ),
        ],
        steps: vec![
            // 1 — draft store, captures storeId. Locale/region fixed en-US/US.
            step(
                "create-store",
                "Creates the draft store that holds the categories and items. The store id is captured and used by the later steps.",
                "platform/admin/stores/v1/create",
                vec![],
                StepFlags::default(),
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("title", literal(json!("In-Game Store"))),
                    bind("defaultLanguage", literal(json!("en-US"))),
                    bind("supportedLanguages[0]", literal(json!("en-US"))),
                    bind("defaultRegion", literal(json!("US"))),
                    bind("supportedRegions[0]", literal(json!("US"))),
                ],
                vec![StepOutputCapture {
                    name: "storeId".to_string(),
                    source: CaptureSource::ResponseBody {
                        path: "$.storeId".to_string(),
                    },
                    default: None,
                    sensitive: false,
                }],
            ),
            // 2 — virtual soft currency.
            step(
                "create-currency",
                "Creates the virtual soft currency that items are priced in.",
                "platform/admin/currencies/v1/create",
                vec![],
                StepFlags {
                    skip_if_exists: true,
                    ..Default::default()
                },
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("currencyCode", workflow_ref("currencyCode")),
                    bind("currencyType", literal(json!("VIRTUAL"))),
                    bind("currencySymbol", workflow_ref("currencyCode")),
                    bind_visible("decimals", literal(json!(0))),
                    bind(
                        "localizationDescriptions",
                        literal(json!({"en-US": "Soft currency"})),
                    ),
                ],
                vec![],
            ),
            // 3 — root category /InGameStore.
            step(
                "create-category-root",
                "Creates the root store category /InGameStore, which holds the durable and consumable sections.",
                "platform/admin/categories/v1/create",
                vec!["create-store".to_string()],
                StepFlags {
                    skip_if_exists: true,
                    ..Default::default()
                },
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("storeId", step_ref("create-store", "storeId")),
                    bind("categoryPath", literal(json!("/InGameStore"))),
                    bind("localizationDisplayNames", literal(json!({}))),
                    bind(
                        "localizationDisplayNames.en-US",
                        literal(json!("In-Game Store")),
                    ),
                ],
                vec![],
            ),
            // 4 — durable leaf category.
            step(
                "create-category-durable",
                "Creates the durable-items category /InGameStore/Durable under the store root.",
                "platform/admin/categories/v1/create",
                vec!["create-store".to_string(), "create-category-root".to_string()],
                StepFlags {
                    skip_if_exists: true,
                    ..Default::default()
                },
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("storeId", step_ref("create-store", "storeId")),
                    bind("categoryPath", literal(json!("/InGameStore/Durable"))),
                    bind("localizationDisplayNames", literal(json!({}))),
                    bind("localizationDisplayNames.en-US", literal(json!("Durable"))),
                ],
                vec![],
            ),
            // 5 — consumable leaf category.
            step(
                "create-category-consumable",
                "Creates the consumable-items category /InGameStore/Consumable under the store root.",
                "platform/admin/categories/v1/create",
                vec!["create-store".to_string(), "create-category-root".to_string()],
                StepFlags {
                    skip_if_exists: true,
                    ..Default::default()
                },
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("storeId", step_ref("create-store", "storeId")),
                    bind("categoryPath", literal(json!("/InGameStore/Consumable"))),
                    bind("localizationDisplayNames", literal(json!({}))),
                    bind("localizationDisplayNames.en-US", literal(json!("Consumable"))),
                ],
                vec![],
            ),
            // 6 — durable item, priced in the currency, in the durable category.
            step(
                "create-item-durable",
                "Creates a durable in-game item in the durable category, priced in the virtual currency.",
                "platform/admin/items/v1/create",
                vec![
                    "create-store".to_string(),
                    "create-currency".to_string(),
                    "create-category-durable".to_string(),
                ],
                StepFlags {
                    skip_if_exists: true,
                    ..Default::default()
                },
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("storeId", step_ref("create-store", "storeId")),
                    // The item's `name` mirrors the reviewable title below, so an
                    // edited title reflects in the Admin Portal's item name too
                    // (the portal keeps `name` and the default-locale title in sync).
                    bind("name", mirror_of("localizations.en-US.title")),
                    bind("itemType", literal(json!("INGAMEITEM"))),
                    bind_visible("entitlementType", literal(json!("DURABLE"))),
                    bind("status", literal(json!("ACTIVE"))),
                    bind("categoryPath", literal(json!("/InGameStore/Durable"))),
                    // Skeleton + reviewable localized display title (nested Literal).
                    bind("localizations", literal(json!({}))),
                    bind_visible(
                        "localizations.en-US.title",
                        literal(json!("Starter Skin")),
                    ),
                    // Region-data skeleton (static currencyType) + dynamic leaves; price
                    // is a reviewable literal.
                    bind(
                        "regionData",
                        literal(json!({"US": [{"currencyType": "VIRTUAL"}]})),
                    ),
                    bind("regionData.US[0].currencyCode", workflow_ref("currencyCode")),
                    bind("regionData.US[0].currencyNamespace", workflow_ref("namespace")),
                    bind_visible("regionData.US[0].price", literal(json!(500))),
                ],
                vec![StepOutputCapture {
                    name: "itemName".to_string(),
                    // Null (not the title) so an auto-skipped re-run (item
                    // already exists) does not report a possibly-stale name:
                    // the human summary shows "Unavailable" (Skipped
                    // provenance) and the JSON output emits null, not a guess.
                    default: Some(serde_json::Value::Null),
                    source: CaptureSource::ResponseBody {
                        path: "$.name".to_string(),
                    },
                    sensitive: false,
                }],
            ),
            // 7 — consumable item.
            step(
                "create-item-consumable",
                "Creates a consumable in-game item in the consumable category, priced in the virtual currency.",
                "platform/admin/items/v1/create",
                vec![
                    "create-store".to_string(),
                    "create-currency".to_string(),
                    "create-category-consumable".to_string(),
                ],
                StepFlags {
                    skip_if_exists: true,
                    ..Default::default()
                },
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("storeId", step_ref("create-store", "storeId")),
                    // Mirrors the reviewable title, same as the durable item.
                    bind("name", mirror_of("localizations.en-US.title")),
                    bind("itemType", literal(json!("INGAMEITEM"))),
                    bind_visible("entitlementType", literal(json!("CONSUMABLE"))),
                    // A CONSUMABLE item must carry a useCount >= 1 (single use
                    // here). The platform rejects a consumable without it with a
                    // validation error (20002); DURABLE items do not use it.
                    bind("useCount", literal(json!(1))),
                    bind("status", literal(json!("ACTIVE"))),
                    bind("categoryPath", literal(json!("/InGameStore/Consumable"))),
                    bind("localizations", literal(json!({}))),
                    bind_visible(
                        "localizations.en-US.title",
                        literal(json!("Health Boost")),
                    ),
                    bind(
                        "regionData",
                        literal(json!({"US": [{"currencyType": "VIRTUAL"}]})),
                    ),
                    bind("regionData.US[0].currencyCode", workflow_ref("currencyCode")),
                    bind("regionData.US[0].currencyNamespace", workflow_ref("namespace")),
                    bind_visible("regionData.US[0].price", literal(json!(100))),
                ],
                vec![StepOutputCapture {
                    name: "itemName".to_string(),
                    // Null (not the title) so an auto-skipped re-run (item
                    // already exists) does not report a possibly-stale name:
                    // the human summary shows "Unavailable" (Skipped
                    // provenance) and the JSON output emits null, not a guess.
                    default: Some(serde_json::Value::Null),
                    source: CaptureSource::ResponseBody {
                        path: "$.name".to_string(),
                    },
                    sensitive: false,
                }],
            ),
            // 8 — publish (mutating: changes the live catalog). confirm.
            {
                // publish is user-skippable (like season-pass) but NOT
                // skip_if_exists: a publish conflict must surface at the gate,
                // not be silently swallowed.
                let mut publish = step(
                    "publish",
                    "Publishes the draft store, applying its categories and items to the namespace's published store.",
                    "platform/admin/catalog-changes/v1/publish-all",
                    vec![
                        "create-item-durable".to_string(),
                        "create-item-consumable".to_string(),
                    ],
                    StepFlags {
                        confirm: true,
                        ..Default::default()
                    },
                    vec![
                        bind("namespace", workflow_ref("namespace")),
                        bind("storeId", step_ref("create-store", "storeId")),
                    ],
                    vec![],
                );
                publish.is_optional = true;
                publish
            },
        ],
        outputs: vec![
            WorkflowOutputAlias {
                name: "storeId".to_string(),
                from_step_id: "create-store".to_string(),
                output: "storeId".to_string(),
                sensitive: false,
                section: Some("Store".to_string()),
                label: Some("Store id".to_string()),
                item_fields: None,
            },
            // The item names come from the create-item responses, so the
            // output always shows what the platform actually stored — the
            // same name the Admin Portal lists — including a review edit.
            WorkflowOutputAlias {
                name: "durableItemName".to_string(),
                from_step_id: "create-item-durable".to_string(),
                output: "itemName".to_string(),
                sensitive: false,
                section: Some("Items".to_string()),
                label: Some("Durable item".to_string()),
                item_fields: None,
            },
            WorkflowOutputAlias {
                name: "consumableItemName".to_string(),
                from_step_id: "create-item-consumable".to_string(),
                output: "itemName".to_string(),
                sensitive: false,
                section: Some("Items".to_string()),
                label: Some("Consumable item".to_string()),
                item_fields: None,
            },
        ],
        completion: Some(WorkflowCompletion {
            created: vec![
                CompletionResource {
                    label: "Store".into(),
                    value: "In-Game Store".into(),
                },
                CompletionResource {
                    label: "Currency".into(),
                    value: "{currencyCode}".into(),
                },
                CompletionResource {
                    label: "Category".into(),
                    value: "/InGameStore".into(),
                },
                CompletionResource {
                    label: "Durable category".into(),
                    value: "/InGameStore/Durable".into(),
                },
                CompletionResource {
                    label: "Consumable category".into(),
                    value: "/InGameStore/Consumable".into(),
                },
                // The two items are NOT listed here: their names can be edited
                // at the step review, and completion templates interpolate only
                // workflow inputs. They surface through the `Items` output
                // aliases instead, captured from the create-item responses.
            ],
            next_steps: vec![
                CompletionStep {
                    description: "View the published store".into(),
                    command: "ags platform stores get-published --namespace {namespace}".into(),
                },
                CompletionStep {
                    description: "List the currencies".into(),
                    command: "ags platform currencies list --namespace {namespace}".into(),
                },
                CompletionStep {
                    description: "List the store's categories".into(),
                    command: "ags platform categories list --namespace {namespace} --store-id <store-id>".into(),
                },
                CompletionStep {
                    description: "List the items you created".into(),
                    command: "ags platform items list --namespace {namespace} --store-id <store-id>".into(),
                },
                CompletionStep {
                    description: "Add another item to the store".into(),
                    command: "ags platform items create --namespace {namespace} --store-id <store-id> --json '<item-json>'".into(),
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
    fn test_in_game_store_compiles_against_bundled_catalogue() {
        let workflow = InGameStore::new();
        let mut catalogue = Catalogue::new();
        let compiled = compile_workflow(workflow.definition(), &mut catalogue)
            .expect("must compile against the bundled catalogue");
        assert_eq!(compiled.steps.len(), 8);
    }

    #[test]
    fn test_in_game_store_inputs() {
        let def = InGameStore::new();
        let names: Vec<&str> = def
            .definition()
            .inputs
            .iter()
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(names, ["namespace", "currencyCode"]);
        let required: Vec<&str> = def
            .definition()
            .inputs
            .iter()
            .filter(|i| i.required)
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(required, ["namespace"]);
        let by_name: std::collections::BTreeMap<_, _> = def
            .definition()
            .inputs
            .iter()
            .map(|i| (i.name.as_str(), i))
            .collect();
        assert_eq!(by_name["currencyCode"].default, Some(json!("GOLD")));
    }

    /// name/title guard: each item's `name` mirrors the reviewable
    /// `localizations.en-US.title`, so a title typed at the step review
    /// reflects in the item's `name` — the field the Admin Portal lists.
    /// Prevents regressing to a fixed literal `name` that desyncs from an
    /// edited title (review overrides key on exact field paths).
    #[test]
    fn test_item_name_mirrors_reviewable_title() {
        let def = InGameStore::new();
        for step_id in ["create-item-durable", "create-item-consumable"] {
            let step = def
                .definition()
                .steps
                .iter()
                .find(|s| s.id == step_id)
                .unwrap_or_else(|| panic!("step {step_id}"));
            let name = step
                .inputs
                .iter()
                .find(|b| b.field == "name")
                .expect("name binding");
            match &name.source {
                BindingSource::Mirror(mirror) => assert_eq!(
                    mirror.mirror_of, "localizations.en-US.title",
                    "{step_id} name must mirror the localized title"
                ),
                other => panic!("{step_id} name must be a mirror, got {other:?}"),
            }
            assert!(
                !name.show_in_review,
                "{step_id} name must NOT be reviewable (the title is the edit surface)"
            );
            let title = step
                .inputs
                .iter()
                .find(|b| b.field == "localizations.en-US.title")
                .expect("title binding");
            assert!(
                matches!(&title.source, BindingSource::Literal(_)),
                "{step_id} title must be a literal (required for a nested review row)"
            );
            assert!(title.show_in_review, "{step_id} title must be reviewable");
        }
    }

    /// Each item step captures the created item's `name` from the response,
    /// and the workflow exposes both through `Items` output aliases — the
    /// final output shows the name the platform stored, edits included.
    #[test]
    fn test_item_steps_capture_response_name_into_aliases() {
        let def = InGameStore::new();
        for step_id in ["create-item-durable", "create-item-consumable"] {
            let step = def
                .definition()
                .steps
                .iter()
                .find(|s| s.id == step_id)
                .unwrap_or_else(|| panic!("step {step_id}"));
            let capture = step
                .outputs
                .iter()
                .find(|c| c.name == "itemName")
                .unwrap_or_else(|| panic!("{step_id} itemName capture"));
            let CaptureSource::ResponseBody { path } = &capture.source;
            assert_eq!(path, "$.name", "{step_id} captures the response name");
            // Null default (required by the skip_if_exists guard) so an
            // auto-skipped re-run reports no stale name — the summary shows
            // "Unavailable" and the JSON output emits null, not a guess.
            assert_eq!(
                capture.default,
                Some(serde_json::Value::Null),
                "{step_id} itemName default must be null, not a guessed name"
            );
        }
        for (alias_name, from_step) in [
            ("durableItemName", "create-item-durable"),
            ("consumableItemName", "create-item-consumable"),
        ] {
            let alias = def
                .definition()
                .outputs
                .iter()
                .find(|a| a.name == alias_name)
                .unwrap_or_else(|| panic!("{alias_name} output alias"));
            assert_eq!(alias.from_step_id, from_step);
            assert_eq!(alias.output, "itemName");
            assert_eq!(alias.section.as_deref(), Some("Items"));
        }
    }

    /// The completion `created` list must not hardcode item names: they are
    /// editable at the step review, and completion templates interpolate only
    /// workflow inputs — a hardcoded name desyncs from an edited one.
    #[test]
    fn test_completion_does_not_hardcode_item_names() {
        let def = InGameStore::new();
        let completion = def.definition().completion.as_ref().expect("completion");
        for resource in &completion.created {
            assert!(
                !resource.value.contains("Starter Skin")
                    && !resource.value.contains("Health Boost"),
                "completion must not hardcode an editable item name: {} = {}",
                resource.label,
                resource.value
            );
        }
    }

    #[test]
    fn test_leaf_categories_depend_on_root() {
        let def = InGameStore::new();
        for step_id in ["create-category-durable", "create-category-consumable"] {
            let step = def
                .definition()
                .steps
                .iter()
                .find(|s| s.id == step_id)
                .unwrap_or_else(|| panic!("step {step_id}"));
            assert!(
                step.dependencies
                    .iter()
                    .any(|d| d == "create-category-root"),
                "{step_id} must depend on create-category-root (parent-first ordering)"
            );
        }
    }

    /// True if `value` (or any nested object/array) contains an object key
    /// exactly equal to `key`.
    fn json_has_key(value: &Value, key: &str) -> bool {
        match value {
            Value::Object(map) => {
                map.contains_key(key) || map.values().any(|v| json_has_key(v, key))
            }
            Value::Array(arr) => arr.iter().any(|v| json_has_key(v, key)),
            _ => false,
        }
    }

    /// Guards the en-US locale fix: every locale-bearing binding must use the
    /// `en-US` tag, never bare `en` — as a literal value, a field-path segment,
    /// or a JSON object key. A regression here reproduces the silent "Item not
    /// found" (30122) publish failure this workflow exists to avoid.
    #[test]
    fn test_locale_bindings_use_en_us_not_bare_en() {
        let workflow = InGameStore::new();
        let def = workflow.definition();

        // create-store: the store's own locale/region literals.
        let store = def
            .steps
            .iter()
            .find(|s| s.id == "create-store")
            .expect("create-store step");
        let source = |field: &str| {
            store
                .inputs
                .iter()
                .find(|b| b.field == field)
                .unwrap_or_else(|| panic!("create-store missing binding {field}"))
                .source
                .clone()
        };
        assert_eq!(source("defaultLanguage"), literal(json!("en-US")));
        assert_eq!(source("supportedLanguages[0]"), literal(json!("en-US")));
        assert_eq!(source("defaultRegion"), literal(json!("US")));
        assert_eq!(source("supportedRegions[0]"), literal(json!("US")));

        // No binding may key a localization on the bare `en` locale — neither in
        // a field path (localizations.en.title) nor a literal JSON object key.
        for step in &def.steps {
            for binding in &step.inputs {
                assert!(
                    !binding.field.split(['.', '[']).any(|seg| seg == "en"),
                    "step '{}' binds a bare-en locale path: {}",
                    step.id,
                    binding.field
                );
                if let BindingSource::Literal(lit) = &binding.source {
                    assert!(
                        !json_has_key(&lit.value, "en"),
                        "step '{}' has a literal with a bare-en locale key: {}",
                        step.id,
                        lit.value
                    );
                }
            }
        }
    }

    #[test]
    fn test_store_step_captures_store_id_with_no_default() {
        let def = InGameStore::new();
        let store = def
            .definition()
            .steps
            .iter()
            .find(|s| s.id == "create-store")
            .expect("create-store step");
        let cap = store
            .outputs
            .iter()
            .find(|c| c.name == "storeId")
            .expect("storeId capture");
        let CaptureSource::ResponseBody { path } = &cap.source;
        assert_eq!(path, "$.storeId");
        assert!(
            cap.default.is_none(),
            "storeId capture must have no default"
        );
    }

    #[test]
    fn test_downstream_steps_reference_captured_store_id() {
        let def = InGameStore::new();
        for step_id in [
            "create-category-root",
            "create-category-durable",
            "create-category-consumable",
            "create-item-durable",
            "create-item-consumable",
            "publish",
        ] {
            let step = def
                .definition()
                .steps
                .iter()
                .find(|s| s.id == step_id)
                .unwrap_or_else(|| panic!("step {step_id}"));
            let binding = step
                .inputs
                .iter()
                .find(|b| b.field == "storeId")
                .unwrap_or_else(|| panic!("{step_id} storeId binding"));
            match &binding.source {
                BindingSource::Reference(r) => {
                    assert_eq!(
                        r.from,
                        ReferenceTarget::Step {
                            id: "create-store".to_string()
                        },
                        "{step_id} must reference create-store"
                    );
                    assert_eq!(r.output.as_deref(), Some("storeId"));
                }
                other => panic!("{step_id} storeId must be a step reference, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_only_publish_confirms() {
        let def = InGameStore::new();
        let confirming: Vec<&str> = def
            .definition()
            .steps
            .iter()
            .filter(|s| s.confirm)
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(confirming, ["publish"]);
        assert!(
            def.definition()
                .steps
                .iter()
                .filter(|s| s.id != "publish")
                .all(|s| !s.is_optional),
            "only publish is optional"
        );
    }

    #[test]
    fn test_in_game_store_conflict_recovery_flags() {
        let def = InGameStore::new();
        let by_id = |id: &str| {
            def.definition()
                .steps
                .iter()
                .find(|s| s.id == id)
                .unwrap_or_else(|| panic!("no step {id}"))
        };
        for id in [
            "create-currency",
            "create-category-root",
            "create-category-durable",
            "create-category-consumable",
            "create-item-durable",
            "create-item-consumable",
        ] {
            assert!(by_id(id).skip_if_exists, "{id} must be skip_if_exists");
        }
        assert!(
            !by_id("create-store").skip_if_exists,
            "create-store must NOT be skip_if_exists"
        );
        assert!(
            !by_id("publish").skip_if_exists,
            "publish must NOT be skip_if_exists"
        );
        assert!(
            by_id("publish").is_optional,
            "publish must be user-skippable"
        );
    }

    #[test]
    fn test_show_in_review_counts_match_curation() {
        let def = InGameStore::new();
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
                ("create-store".into(), 0),
                ("create-currency".into(), 1),
                ("create-category-root".into(), 0),
                ("create-category-durable".into(), 0),
                ("create-category-consumable".into(), 0),
                ("create-item-durable".into(), 3),
                ("create-item-consumable".into(), 3),
                ("publish".into(), 0),
            ]
        );
    }

    /// A CONSUMABLE item must bind `useCount` >= 1 — the platform rejects a
    /// consumable without it (error 20002). A DURABLE item does not consume, so
    /// the durable step deliberately omits `useCount`.
    #[test]
    fn test_consumable_item_binds_use_count() {
        let def = InGameStore::new();
        let steps = &def.definition().steps;

        let consumable = steps
            .iter()
            .find(|s| s.id == "create-item-consumable")
            .expect("create-item-consumable step");
        let use_count = consumable
            .inputs
            .iter()
            .find(|b| b.field == "useCount")
            .expect("consumable must bind useCount");
        let count = match &use_count.source {
            BindingSource::Literal(l) => l
                .value
                .as_i64()
                .expect("useCount literal must be an integer"),
            _ => panic!("consumable useCount must be a literal"),
        };
        assert!(count >= 1, "consumable useCount must be >= 1, got {count}");

        let durable = steps
            .iter()
            .find(|s| s.id == "create-item-durable")
            .expect("create-item-durable step");
        assert!(
            durable.inputs.iter().all(|b| b.field != "useCount"),
            "durable item must not bind useCount (DURABLE does not consume)"
        );
    }

    #[test]
    fn test_no_required_field_left_unbound() {
        let workflow = InGameStore::new();
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

    #[test]
    fn test_completion_and_output_alias_present() {
        let def = InGameStore::new();
        assert!(def.definition().completion.is_some(), "completion authored");
        let alias = def
            .definition()
            .outputs
            .iter()
            .find(|a| a.name == "storeId")
            .expect("storeId output alias");
        assert_eq!(alias.from_step_id, "create-store");
        assert_eq!(alias.output, "storeId");
        // compile_workflow runs validate_completion; a bad template would fail.
        let mut catalogue = Catalogue::new();
        compile_workflow(def.definition(), &mut catalogue).expect("completion must validate");
    }
}
