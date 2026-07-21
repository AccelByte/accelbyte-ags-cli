//! Input-contract tests for registered workflows.
//!
//! Sibling of `contract_input/services.rs`, for runtime-owned workflows. The
//! contract under test is the input surface a user types: per input, the
//! kebab-cased flag name, type, required, enum_values, and default. Source of
//! truth is the *compiled* workflow (all schemas resolved), not the raw
//! definition — the run path builds flags and coerces values from the compiled
//! inputs (`invocation/workflows.rs`).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;

use ags_protocol::workflow::WorkflowId;
use ags_runtime::catalogue::Catalogue;
use ags_runtime::runtime::workflows::compile::compile_workflow;
use ags_runtime::runtime::workflows::registry;
use ags_runtime::support::strings::to_kebab_case;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::fixture_helpers::fixture_path;

/// One input's contract surface. Field order matches the committed baseline
/// JSON. `description`, `sensitive`, and `dynamic` are intentionally absent —
/// they are not part of what the user types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ContractInput {
    /// Kebab-cased CLI flag (`--<name>`).
    name: String,
    /// Recognised JSON-schema type, or `null` for an unrecognised/absent type.
    #[serde(rename = "type")]
    input_type: Option<String>,
    required: bool,
    enum_values: Option<Vec<Value>>,
    default: Option<Value>,
}

/// The baseline file shape: `{ "inputs": [ ... ] }`.
#[derive(Debug, Serialize, Deserialize)]
struct BaselineFile {
    inputs: Vec<ContractInput>,
}

/// JSON-schema types the CLI recognises; anything else (or a missing/non-string
/// `type`) projects to `None`. Matches describe's recognised-type gate.
fn is_recognised_schema_type(t: &str) -> bool {
    matches!(
        t,
        "string" | "integer" | "number" | "boolean" | "object" | "array"
    )
}

/// Compile a registered workflow and project its inputs to the contract
/// surface. Panics (failing the test) if the id is unregistered or the workflow
/// does not compile — every shipped builtin must compile.
fn contract_inputs_for(id: &str) -> Vec<ContractInput> {
    let workflow = registry()
        .resolve(&WorkflowId::new(id))
        .unwrap_or_else(|| panic!("workflow '{id}' not registered"));
    let definition = workflow.definition();
    let mut catalogue = Catalogue::new();
    let compiled = compile_workflow(definition, &mut catalogue)
        .unwrap_or_else(|e| panic!("workflow '{id}' failed to compile: {e:?}"));

    compiled
        .inputs
        .iter()
        .map(|spec| {
            let recognised_type = spec
                .schema
                .as_ref()
                .and_then(|s| s.get("type"))
                .and_then(|t| t.as_str())
                .filter(|t| is_recognised_schema_type(t));
            let (input_type, enum_values) = match recognised_type {
                Some(ty) => (
                    Some(ty.to_string()),
                    spec.schema
                        .as_ref()
                        .and_then(|s| s.get("enum"))
                        .and_then(|e| e.as_array())
                        .cloned(),
                ),
                None => (None, None),
            };
            ContractInput {
                name: to_kebab_case(&spec.name),
                input_type,
                required: spec.required,
                enum_values,
                default: spec.default.clone(),
            }
        })
        .collect()
}

#[test]
fn test_contract_inputs_competitive_multiplayer_projection() {
    let inputs = contract_inputs_for("competitive-multiplayer");

    let namespace = inputs
        .iter()
        .find(|i| i.name == "namespace")
        .expect("namespace input present");
    assert_eq!(namespace.input_type.as_deref(), Some("string"));
    assert!(namespace.required, "namespace is required");
    assert_eq!(namespace.default, None);

    let ppt = inputs
        .iter()
        .find(|i| i.name == "players-per-team")
        .expect("players-per-team input present");
    assert_eq!(ppt.input_type.as_deref(), Some("integer"));
    assert!(
        !ppt.required,
        "players-per-team has a default → not required"
    );
    assert_eq!(ppt.default, Some(serde_json::json!(4)));
}

/// Ids of every registered workflow.
fn registry_ids() -> BTreeSet<String> {
    registry().ids().map(|id| id.as_str().to_string()).collect()
}

/// Regenerate every workflow baseline from the live registry. Run deliberately
/// after an *intentional* contract change:
///
/// ```text
/// cargo test -p accelbyte-ags-cli --test contract_input \
///     generate_workflow_baselines -- --ignored
/// ```
///
/// Inputs are sorted by flag name so the file is canonical and regeneration is
/// diff-free. The assertion tests compare order-independently, so sorting here
/// does not affect them.
#[test]
#[ignore = "regenerates committed baselines; run manually after a contract change"]
fn generate_workflow_baselines() {
    let dir = fixture_path("baselines/workflows");
    std::fs::create_dir_all(&dir).expect("create baselines/workflows dir");
    for id in registry_ids() {
        let mut inputs = contract_inputs_for(&id);
        inputs.sort_by(|a, b| a.name.cmp(&b.name));
        let file = BaselineFile { inputs };
        let json = serde_json::to_string_pretty(&file).expect("serialize baseline");
        let path = dir.join(format!("{id}_input_contract.json"));
        // Write atomically, mirroring `support::file_system::write_file_restricted`:
        // a temp file in the same dir, flushed to stable storage, then renamed into
        // place. `rename` is atomic on one filesystem, so a baseline reader running
        // concurrently — e.g. under `cargo test --include-ignored` — sees either the
        // previous complete file or the new one, never a truncated, empty file (the
        // race that `std::fs::write`'s in-place truncate exposed: a 0-byte read and
        // an EOF parse error). `sync_all` flushes before the rename so a crash cannot
        // leave a renamed-but-empty file. The `NamedTempFile` auto-deletes on drop,
        // so a panic mid-loop leaves no orphan. The `.ags-tmp-` prefix matches the
        // project convention, though this fixtures dir is outside the runtime
        // temp-sweep — the drop-cleanup is what keeps it tidy.
        let mut tmp = tempfile::Builder::new()
            .prefix(".ags-tmp-")
            .tempfile_in(&dir)
            .expect("create baseline temp file");
        tmp.write_all(format!("{json}\n").as_bytes())
            .expect("write baseline temp file");
        tmp.as_file().sync_all().expect("sync baseline temp file");
        tmp.persist(&path).expect("persist baseline file");
    }
}

/// Workflow ids parsed from the baseline directory (filename minus the
/// `_input_contract.json` suffix).
fn baseline_ids() -> BTreeSet<String> {
    let dir = fixture_path("baselines/workflows");
    let mut ids = BTreeSet::new();
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("read dir entry");
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if let Some(id) = name.strip_suffix("_input_contract.json") {
            ids.insert(id.to_string());
        }
    }
    ids
}

/// Load and parse one workflow's committed baseline.
fn load_baseline(id: &str) -> Vec<ContractInput> {
    let path = fixture_path(&format!("baselines/workflows/{id}_input_contract.json"));
    assert!(path.exists(), "baseline missing: {}", path.display());
    let raw = std::fs::read_to_string(&path).expect("read baseline file");
    let file: BaselineFile = serde_json::from_str(&raw).expect("parse baseline JSON");
    file.inputs
}

/// Baseline files and registered workflows must be the same set. The forward
/// loop in the other tests catches an *added* workflow with no baseline; this
/// catches a *removed* workflow whose orphan baseline lingers. A workflow id is
/// public invocation surface (`ags workflow run <id>`), so removing one must
/// touch the baseline dir.
#[test]
fn test_workflow_baseline_files_match_registry() {
    let files = baseline_ids();
    let registered = registry_ids();
    let orphan: Vec<&String> = files.difference(&registered).collect();
    let missing: Vec<&String> = registered.difference(&files).collect();
    assert!(
        orphan.is_empty() && missing.is_empty(),
        "baseline files without a registered workflow: {orphan:?}\n\
         registered workflows without a baseline file: {missing:?}"
    );
}

/// The set of input flags matches the baseline, and no two inputs collapse to
/// the same flag. The run path builds the long flag from
/// `to_kebab_case(spec.name)`, so two distinct raw names can collide on one
/// `--flag`; a name-keyed set would silently hide that, so uniqueness is
/// asserted first.
#[test]
fn test_workflow_input_set_matches_baseline() {
    let mut failures: Vec<String> = Vec::new();

    for id in registry_ids() {
        let observed = contract_inputs_for(&id);

        let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
        for input in &observed {
            *counts.entry(input.name.as_str()).or_default() += 1;
        }
        let dups: Vec<&str> = counts
            .iter()
            .filter(|(_, &n)| n > 1)
            .map(|(&name, _)| name)
            .collect();
        assert!(
            dups.is_empty(),
            "[{id}] inputs normalize to duplicate flags: {dups:?}"
        );

        let observed_names: BTreeSet<&str> = observed.iter().map(|i| i.name.as_str()).collect();
        let baseline = load_baseline(&id);
        let baseline_names: BTreeSet<&str> = baseline.iter().map(|i| i.name.as_str()).collect();

        let missing: Vec<&str> = baseline_names
            .difference(&observed_names)
            .copied()
            .collect();
        let extra: Vec<&str> = observed_names
            .difference(&baseline_names)
            .copied()
            .collect();
        if !missing.is_empty() || !extra.is_empty() {
            failures.push(format!(
                "[{id}] input set mismatch — missing: {missing:?}, extra: {extra:?}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "workflow input set drift:\n{}",
        failures.join("\n")
    );
}

/// For each input present in both observed and baseline, every contract field
/// matches (type, required, enum_values, default).
#[test]
fn test_workflow_input_fields_match_baseline() {
    let mut failures: Vec<String> = Vec::new();

    for id in registry_ids() {
        let observed = contract_inputs_for(&id);
        let baseline = load_baseline(&id);
        let baseline_by_name: BTreeMap<&str, &ContractInput> =
            baseline.iter().map(|i| (i.name.as_str(), i)).collect();

        // Compare fields only for inputs present in both sides. Names that exist
        // on only one side (added/removed inputs) are caught by
        // `test_workflow_input_set_matches_baseline`, so they are not re-flagged here.
        for input in &observed {
            if let Some(expected) = baseline_by_name.get(input.name.as_str()) {
                if input != *expected {
                    failures.push(format!(
                        "[{id}] {}: rust={:?}, baseline={:?}",
                        input.name, input, expected
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "workflow input contract drift:\n{}",
        failures.join("\n")
    );
}
