//! The `player-overview` built-in workflow.
//!
//! A read-only cross-service overview for a single player. All 11 steps are
//! optional so a missing service or permission error on any individual call
//! leaves the rest of the run intact. `is_reviewed_by_default: false` means
//! the run proceeds straight through without per-step review gates.
//!
//! The `userId` input is backed by a dynamic-enum picker that calls the IAM
//! admin user-search endpoint to resolve a display name or email address to
//! a user id before the overview steps begin.

use ags_protocol::catalogue::{OperationId, ServiceId};
use ags_protocol::workflow::{
    BindingSource, CaptureSource, LabelDetail, OperationReference, OptionParameterBinding,
    OptionsSource, ReferenceBinding, ReferenceTarget, StepDefinition, StepInputBinding,
    StepOutputCapture, WorkflowBriefing, WorkflowDefinition, WorkflowId, WorkflowInputSpec,
    WorkflowOutputAlias,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::runtime::workflows::Workflow;

/// The `player-overview` built-in workflow.
pub struct PlayerOverview {
    definition: WorkflowDefinition,
}

impl PlayerOverview {
    /// Build the workflow with its hand-written definition.
    pub fn new() -> Self {
        Self {
            definition: build_definition(),
        }
    }
}

impl Default for PlayerOverview {
    fn default() -> Self {
        Self::new()
    }
}

impl Workflow for PlayerOverview {
    fn definition(&self) -> &WorkflowDefinition {
        &self.definition
    }
}

// ── Binding helpers ─────────────────────────────────────────────────────────

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

/// Pair an operation field name with its value source.
fn bind(field: &str, source: BindingSource) -> StepInputBinding {
    StepInputBinding {
        field: field.to_string(),
        source,
        show_in_review: false,
        description: None,
    }
}

// ── Step helpers ─────────────────────────────────────────────────────────────

/// Build a read-only failure-tolerant step: `continue_on_failure: true`,
/// `confirm: false`, `is_reviewed: None`, `dependencies: []`.
fn opt_read(
    id: &str,
    description: &str,
    service: &str,
    operation: &str,
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
        dependencies: vec![],
        confirm: false,
        is_optional: false,
        continue_on_failure: true,
        skip_if_exists: false,
        is_reviewed: None,
        inputs,
        outputs,
    }
}

// ── Capture helpers ──────────────────────────────────────────────────────────

/// Single-value capture from the response body; defaults to `null` when the
/// JSONPath misses.
fn cap(name: &str, path: &str) -> StepOutputCapture {
    StepOutputCapture {
        name: name.to_string(),
        source: CaptureSource::ResponseBody {
            path: path.to_string(),
        },
        default: Some(json!(null)),
        sensitive: false,
    }
}

/// Array capture from the response body; defaults to `[]` when the JSONPath
/// misses.
fn cap_arr(name: &str, path: &str) -> StepOutputCapture {
    StepOutputCapture {
        name: name.to_string(),
        source: CaptureSource::ResponseBody {
            path: path.to_string(),
        },
        default: Some(json!([])),
        sensitive: false,
    }
}

// ── Input/output helpers ─────────────────────────────────────────────────────

/// The standard `namespace` + `userId` binding pair used by most steps.
fn ns_user() -> Vec<StepInputBinding> {
    vec![
        bind("namespace", workflow_ref("namespace")),
        bind("userId", workflow_ref("userId")),
    ]
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

/// Build a scalar `WorkflowOutputAlias` with section and label metadata.
fn alias(name: &str, step: &str, output: &str, section: &str, label: &str) -> WorkflowOutputAlias {
    WorkflowOutputAlias {
        name: name.to_string(),
        from_step_id: step.to_string(),
        output: output.to_string(),
        sensitive: false,
        section: Some(section.to_string()),
        label: Some(label.to_string()),
        item_fields: None,
    }
}

/// Build an array `WorkflowOutputAlias` rendered as a per-item sub-list. The
/// first field is the line label, the rest are detail.
fn alias_list(
    name: &str,
    step: &str,
    output: &str,
    section: &str,
    label: &str,
    fields: &[&str],
) -> WorkflowOutputAlias {
    WorkflowOutputAlias {
        name: name.to_string(),
        from_step_id: step.to_string(),
        output: output.to_string(),
        sensitive: false,
        section: Some(section.to_string()),
        label: Some(label.to_string()),
        item_fields: Some(fields.iter().map(|f| f.to_string()).collect()),
    }
}

// ── Definition ───────────────────────────────────────────────────────────────

fn build_definition() -> WorkflowDefinition {
    WorkflowDefinition {
        id: WorkflowId::new("player-overview"),
        name: "Investigate a player".to_string(),
        intent: Some("player investigation moderation overview lookup user".to_string()),
        description: Some("Read-only cross-service overview for one player".to_string()),
        briefing: Some(WorkflowBriefing {
            overview: concat!(
                "This workflow builds a **read-only overview** of a single player. ",
                "It reads their data across AccelByte services. That covers account ",
                "and linked platforms, bans and reports, entitlements, wallet and ",
                "orders, stats and achievements, cloud-save records, and inventory. ",
                "**Nothing is modified.**\n\n",
                "You can enter the player's user id directly if you have it. If ",
                "you do not, search IAM by email or display name and pick them ",
                "from the matches. The workflow then reads every service for that ",
                "user. A ",
                "service that is unavailable or returns no data is shown as such, ",
                "and the rest of the run carries on."
            )
            .into(),
            prerequisites: vec![
                concat!(
                    "**Admin credentials** with read access to the services above ",
                    "(iam, platform, social, achievement, cloudsave, inventory, ",
                    "reporting). The workflow uses your current `ags` login."
                )
                .into(),
                concat!(
                    "**The player's user id, or their email address or display ",
                    "name.** With an id you go straight to the overview. Without ",
                    "one, search by email or display name and pick from the matches."
                )
                .into(),
            ],
            creates: vec![],
        }),
        is_reviewed_by_default: false,
        inputs: vec![
            input(
                "namespace",
                "Your game's AccelByte namespace",
                true,
                None,
                json!({"type": "string"}),
            ),
            input(
                "searchBy",
                "Which field to search. Choose email address, display name, or unique display name.",
                false,
                Some(json!("emailAddress")),
                json!({"type": "string", "enum": ["emailAddress", "displayName", "uniqueDisplayName"]}),
            ),
            input(
                "searchQuery",
                "What to search for by email or display name. It doesn't have to be an exact match. Leave it empty if you are entering a user id directly.",
                false,
                None,
                json!({"type": "string"}),
            ),
            input_with_options(
                "userId",
                "Choose the player to investigate. If you have their user id, open this and type it. Otherwise fill search-query first, then open to search and pick from the matches.",
                true,
                None,
                json!({"type": "string"}),
                OptionsSource {
                    operation: OperationReference {
                        service: ServiceId::new("iam"),
                        operation: OperationId::new("iam/admin/users/v3/search"),
                    },
                    parameters: BTreeMap::from([
                        (
                            "namespace".into(),
                            OptionParameterBinding::FromInput("namespace".into()),
                        ),
                        (
                            "query".into(),
                            OptionParameterBinding::FromInputOptional("searchQuery".into()),
                        ),
                        (
                            "by".into(),
                            OptionParameterBinding::FromInputOptional("searchBy".into()),
                        ),
                    ]),
                    items_path: "$.data".into(),
                    value: "$.userId".into(),
                    // Label by username (always present, unlike displayName).
                    label: Some("$.userName".into()),
                    // Show the field the user searched by in brackets, so the
                    // match is easy to confirm. Searching by username adds no
                    // bracket since the label already shows it.
                    label_detail: Some(LabelDetail::ByInput {
                        input: "searchBy".into(),
                        paths: BTreeMap::from([
                            ("emailAddress".into(), "$.emailAddress".into()),
                            ("displayName".into(), "$.displayName".into()),
                            ("uniqueDisplayName".into(), "$.uniqueDisplayName".into()),
                        ]),
                    }),
                    // On surfaces without the picker (plain, inline), this input
                    // is a plain text field and the search inputs are hidden, so
                    // the picker guidance above does not apply — type the id.
                    fallback_description: Some(
                        "Enter the player's user id (the search fields are not used here)".into(),
                    ),
                    filter: None,
                },
            ),
        ],
        steps: vec![
            opt_read(
                "account",
                "Account details",
                "iam",
                "iam/admin/users/v3/get",
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("userId", workflow_ref("userId")),
                ],
                vec![
                    cap("account_displayName", "$.displayName"),
                    cap("account_email", "$.emailAddress"),
                    cap("account_country", "$.country"),
                    cap("account_enabled", "$.enabled"),
                    cap("account_created", "$.createdAt"),
                ],
            ),
            opt_read(
                "linked-platforms",
                "Linked platform accounts",
                "iam",
                "iam/admin/users/v3/list-platform-accounts",
                ns_user(),
                vec![cap_arr("linked_platforms", "$.data")],
            ),
            opt_read(
                "bans",
                "Bans",
                "iam",
                "iam/admin/users/v3/list-bans",
                ns_user(),
                vec![cap_arr("bans", "$.data")],
            ),
            opt_read(
                "entitlements",
                "Entitlements",
                "platform",
                "platform/admin/entitlements/v1/list",
                ns_user(),
                vec![cap_arr("entitlements", "$.data")],
            ),
            opt_read(
                "wallet",
                "Wallet balances",
                "platform",
                "platform/admin/wallets/v1/get-currency-summary",
                ns_user(),
                vec![cap_arr("wallet", "$")],
            ),
            opt_read(
                "orders",
                "Orders",
                "platform",
                "platform/admin/orders/v1/list-user",
                ns_user(),
                vec![cap_arr("orders", "$.data")],
            ),
            opt_read(
                "stats",
                "Stat items",
                "social",
                "social/admin/user-stat-values/v1/list",
                ns_user(),
                vec![cap_arr("stats", "$.data")],
            ),
            opt_read(
                "achievements",
                "Achievements",
                "achievement",
                "achievement/admin/user-progress/v1/list",
                ns_user(),
                vec![cap_arr("achievements", "$.data")],
            ),
            opt_read(
                "cloudsave",
                "Cloud save record keys",
                "cloudsave",
                "cloud-save/admin/user-records/v1/list-by-user-id",
                ns_user(),
                vec![cap_arr("cloudsave", "$.data")],
            ),
            opt_read(
                "inventory",
                "Inventory",
                "inventory",
                "inventory/admin/inventories/v1/list",
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("userId", workflow_ref("userId")),
                ],
                vec![cap_arr("inventory", "$.data")],
            ),
            opt_read(
                "reports",
                "Reports against the player",
                "reporting",
                "reporting/admin/reports/v1/list",
                vec![
                    bind("namespace", workflow_ref("namespace")),
                    bind("reportedUserId", workflow_ref("userId")),
                ],
                vec![cap_arr("reports", "$.data")],
            ),
        ],
        outputs: vec![
            alias("account_displayName", "account", "account_displayName", "Account", "Display name"),
            alias("account_email", "account", "account_email", "Account", "Email"),
            alias("account_country", "account", "account_country", "Account", "Country"),
            alias("account_enabled", "account", "account_enabled", "Account", "Enabled"),
            alias("account_created", "account", "account_created", "Account", "Created"),
            alias_list("linked_platforms", "linked-platforms", "linked_platforms", "Account", "Linked platforms", &["platformId", "platformUserId"]),
            alias_list("bans", "bans", "bans", "Moderation", "Bans", &["ban", "reason", "endDate"]),
            alias_list("reports", "reports", "reports", "Moderation", "Reports", &["category", "reason", "createdAt"]),
            alias_list("entitlements", "entitlements", "entitlements", "Economy", "Entitlements", &["name", "status", "useCount"]),
            alias_list("wallet", "wallet", "wallet", "Economy", "Wallet", &["currencyCode", "balance"]),
            alias_list("orders", "orders", "orders", "Economy", "Orders", &["orderNo", "status"]),
            alias_list("stats", "stats", "stats", "Progression", "Stat items", &["statCode", "value"]),
            alias_list("achievements", "achievements", "achievements", "Progression", "Achievements", &["name", "status"]),
            alias_list("cloudsave", "cloudsave", "cloudsave", "Data", "Cloud save keys", &["key"]),
            alias_list("inventory", "inventory", "inventory", "Data", "Inventory", &["inventoryConfigurationCode", "usedCountSlots"]),
        ],
        completion: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_player_overview_compiles_against_bundled_catalogue() {
        let wf = PlayerOverview::new();
        let mut catalogue = crate::catalogue::Catalogue::new();
        let compiled =
            crate::runtime::workflows::compile::compile_workflow(wf.definition(), &mut catalogue)
                .expect("must compile against the bundled catalogue");
        assert_eq!(compiled.steps.len(), 11);
        assert!(
            compiled.steps.iter().all(|s| s.continue_on_failure),
            "all reads continue_on_failure"
        );
        assert!(!compiled.is_reviewed_by_default, "runs straight through");
    }

    #[test]
    fn test_player_overview_inputs() {
        let def = PlayerOverview::new();
        let names: Vec<&str> = def
            .definition()
            .inputs
            .iter()
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(names, ["namespace", "searchBy", "searchQuery", "userId"]);
        let user = def
            .definition()
            .inputs
            .iter()
            .find(|i| i.name == "userId")
            .unwrap();
        let os = user.options_source.as_ref().expect("userId picker");
        assert_eq!(os.operation.operation.as_str(), "iam/admin/users/v3/search");
        assert_eq!(os.items_path, "$.data");
        assert_eq!(os.value, "$.userId");
    }

    #[test]
    fn test_player_overview_supports_direct_user_id() {
        use ags_protocol::workflow::OptionParameterBinding;
        let def = PlayerOverview::new();
        let inputs = &def.definition().inputs;

        let search_query = inputs.iter().find(|i| i.name == "searchQuery").unwrap();
        assert!(
            !search_query.required,
            "searchQuery must be optional so --user-id works alone"
        );

        let user = inputs.iter().find(|i| i.name == "userId").unwrap();
        let os = user.options_source.as_ref().expect("userId picker");
        assert!(
            matches!(os.parameters.get("query"), Some(OptionParameterBinding::FromInputOptional(n)) if n == "searchQuery"),
            "query param must be FromInputOptional(searchQuery)"
        );
        assert!(
            matches!(os.parameters.get("by"), Some(OptionParameterBinding::FromInputOptional(n)) if n == "searchBy"),
            "by param must be FromInputOptional(searchBy)"
        );
        assert!(
            matches!(os.parameters.get("namespace"), Some(OptionParameterBinding::FromInput(n)) if n == "namespace"),
            "namespace stays the sole gating dep"
        );
    }
}
