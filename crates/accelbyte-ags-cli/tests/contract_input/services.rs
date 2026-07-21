use std::collections::{BTreeMap, BTreeSet, HashMap};

use ags_protocol::catalogue::{ParameterLocation, ServiceSchema};
use ags_runtime::catalogue::Catalogue;

use crate::common::fixture_helpers::fixture_path;

fn load_service(service: &str) -> ServiceSchema {
    Catalogue::load_bundled(service)
        .unwrap_or_else(|e| panic!("Failed to load bundled '{service}' spec: {e:?}"))
}

fn load_baseline(service: &str) -> HashMap<String, Vec<serde_json::Value>> {
    let path = fixture_path(&format!("baselines/{service}_input_contract.json"));
    assert!(path.exists(), "baseline missing: {}", path.display());
    let raw = std::fs::read_to_string(&path).unwrap();
    serde_json::from_str(&raw).unwrap()
}

/// Normalised view of one operation for contract comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ContractOp {
    http_method: String,
    path: String,
    /// (name, location, required)
    params: BTreeSet<(String, String, bool)>,
    has_request_body: bool,
    summary: String,
}

/// resource name → (x_operation_id → ContractOp)
type Contract = BTreeMap<String, BTreeMap<String, ContractOp>>;

fn contract_from_schema(schema: &ServiceSchema) -> Contract {
    let mut out: Contract = BTreeMap::new();
    for resource in &schema.resources {
        let entry = out.entry(resource.name.clone()).or_default();
        for op in resource.operations() {
            let params = op
                .parameters
                .iter()
                .map(|p| {
                    (
                        p.name.clone(),
                        location_as_string(p.location).to_string(),
                        p.required,
                    )
                })
                .collect();
            entry.insert(
                op.id.as_str().to_string(),
                ContractOp {
                    http_method: op.http_method.as_str().to_string(),
                    path: op.path_template.clone(),
                    params,
                    has_request_body: op.request_body.is_some(),
                    summary: op.summary.clone(),
                },
            );
        }
    }
    out
}

fn contract_from_baseline(
    service: &str,
    baseline: &HashMap<String, Vec<serde_json::Value>>,
) -> Contract {
    let mut out: Contract = BTreeMap::new();
    for (resource, ops) in baseline {
        let entry = out.entry(resource.clone()).or_default();
        for op in ops {
            // The baseline is a controlled, generated artifact. A missing
            // required field is a generator bug, not a tolerable default —
            // fail loudly rather than silently projecting a wrong ContractOp
            // (e.g. a missing `has_request_body` masquerading as `false`).
            let id = op["x_operation_id"]
                .as_str()
                .unwrap_or_else(|| panic!("[{service}] baseline op missing x_operation_id"));
            let params = op["parameters"]
                .as_array()
                .unwrap_or_else(|| panic!("[{service}] baseline op {id} missing parameters array"))
                .iter()
                .map(|p| {
                    (
                        p["name"]
                            .as_str()
                            .unwrap_or_else(|| {
                                panic!("[{service}] baseline op {id} param missing name")
                            })
                            .to_string(),
                        p["location"]
                            .as_str()
                            .unwrap_or_else(|| {
                                panic!("[{service}] baseline op {id} param missing location")
                            })
                            .to_string(),
                        p["required"].as_bool().unwrap_or_else(|| {
                            panic!("[{service}] baseline op {id} param missing required")
                        }),
                    )
                })
                .collect();
            entry.insert(
                id.to_string(),
                ContractOp {
                    http_method: op["http_method"]
                        .as_str()
                        .unwrap_or_else(|| {
                            panic!("[{service}] baseline op {id} missing http_method")
                        })
                        .to_string(),
                    path: op["path"]
                        .as_str()
                        .unwrap_or_else(|| panic!("[{service}] baseline op {id} missing path"))
                        .to_string(),
                    params,
                    has_request_body: op["has_request_body"].as_bool().unwrap_or_else(|| {
                        panic!("[{service}] baseline op {id} missing has_request_body")
                    }),
                    summary: op["summary"]
                        .as_str()
                        .unwrap_or_else(|| panic!("[{service}] baseline op {id} missing summary"))
                        .to_string(),
                },
            );
        }
    }
    out
}

/// Breaking gate: every baseline resource/op/param/body must survive unchanged.
/// Additions (extra resources, ops, params) are tolerated.
fn breaking_changes(service: &str, baseline: &Contract, current: &Contract) -> Vec<String> {
    let mut findings = Vec::new();
    for (resource, base_ops) in baseline {
        let Some(cur_ops) = current.get(resource) else {
            findings.push(format!("[{service}] resource deleted: {resource}"));
            continue;
        };
        for (id, base) in base_ops {
            let Some(cur) = cur_ops.get(id) else {
                findings.push(format!("[{service}] operation deleted: {id}"));
                continue;
            };
            if base.http_method != cur.http_method {
                findings.push(format!(
                    "[{service}] {id}: http_method {} -> {}",
                    base.http_method, cur.http_method
                ));
            }
            if base.path != cur.path {
                findings.push(format!(
                    "[{service}] {id}: path {} -> {}",
                    base.path, cur.path
                ));
            }
            if base.has_request_body != cur.has_request_body {
                findings.push(format!(
                    "[{service}] {id}: has_request_body {} -> {}",
                    base.has_request_body, cur.has_request_body
                ));
            }
            for param in base.params.difference(&cur.params) {
                findings.push(format!("[{service}] {id}: param removed/changed {param:?}"));
            }
        }
    }
    findings
}

/// Freshness: full exact match. Any difference (incl. additions and summary
/// changes) is the signal to regenerate the baseline.
fn freshness_diffs(service: &str, baseline: &Contract, current: &Contract) -> Vec<String> {
    let mut findings = Vec::new();
    let resources: BTreeSet<&String> = baseline.keys().chain(current.keys()).collect();
    for resource in resources {
        match (baseline.get(resource), current.get(resource)) {
            (None, Some(_)) => findings.push(format!(
                "[{service}] resource added in spec (not in baseline): {resource}"
            )),
            (Some(_), None) => findings.push(format!(
                "[{service}] resource removed from spec (still in baseline): {resource}"
            )),
            (Some(base_ops), Some(cur_ops)) => {
                let ids: BTreeSet<&String> = base_ops.keys().chain(cur_ops.keys()).collect();
                for id in ids {
                    match (base_ops.get(id), cur_ops.get(id)) {
                        (None, Some(_)) => findings.push(format!(
                            "[{service}] op added in spec (not in baseline): {id}"
                        )),
                        (Some(_), None) => findings.push(format!(
                            "[{service}] op removed from spec (still in baseline): {id}"
                        )),
                        (Some(b), Some(c)) if b != c => {
                            findings.extend(op_field_diffs(service, id, b, c))
                        }
                        _ => {}
                    }
                }
            }
            (None, None) => {}
        }
    }
    findings
}

/// Field-level diff of two `ContractOp`s that compared unequal. Used by
/// `freshness_diffs` so its output is surgical (like `breaking_changes`) rather
/// than a `Debug` dump of two full structs.
fn op_field_diffs(service: &str, id: &str, base: &ContractOp, cur: &ContractOp) -> Vec<String> {
    let mut out = Vec::new();
    if base.http_method != cur.http_method {
        out.push(format!(
            "[{service}] {id}: http_method {} -> {}",
            base.http_method, cur.http_method
        ));
    }
    if base.path != cur.path {
        out.push(format!(
            "[{service}] {id}: path {} -> {}",
            base.path, cur.path
        ));
    }
    if base.has_request_body != cur.has_request_body {
        out.push(format!(
            "[{service}] {id}: has_request_body {} -> {}",
            base.has_request_body, cur.has_request_body
        ));
    }
    if base.summary != cur.summary {
        out.push(format!(
            "[{service}] {id}: summary {:?} -> {:?}",
            base.summary, cur.summary
        ));
    }
    for param in base.params.difference(&cur.params) {
        out.push(format!("[{service}] {id}: param removed {param:?}"));
    }
    for param in cur.params.difference(&base.params) {
        out.push(format!("[{service}] {id}: param added {param:?}"));
    }
    out
}

/// Render a `ParameterLocation` as the same lowercase string the baseline JSON uses.
fn location_as_string(location: ParameterLocation) -> &'static str {
    match location {
        ParameterLocation::Path => "path",
        ParameterLocation::Query => "query",
        ParameterLocation::Header => "header",
        ParameterLocation::Body => "body",
        ParameterLocation::FormData => "form_data",
    }
}

#[test]
fn test_no_breaking_changes() {
    let mut failures: Vec<String> = Vec::new();
    for service in Catalogue::service_ids() {
        let current = contract_from_schema(&load_service(service));
        let baseline = contract_from_baseline(service, &load_baseline(service));
        failures.extend(breaking_changes(service, &baseline, &current));
    }
    assert!(
        failures.is_empty(),
        "breaking contract changes (do NOT regenerate baselines until resolved):\n{}",
        failures.join("\n")
    );
}

#[test]
fn test_baseline_is_current() {
    let mut failures: Vec<String> = Vec::new();
    for service in Catalogue::service_ids() {
        let current = contract_from_schema(&load_service(service));
        let baseline = contract_from_baseline(service, &load_baseline(service));
        failures.extend(freshness_diffs(service, &baseline, &current));
    }
    assert!(
        failures.is_empty(),
        "baseline out of date — confirm `test_no_breaking_changes` passes, then regenerate:\n  \
         python3 scripts/generate_cli_command_catalogue.py --emit-baselines \
         crates/accelbyte-ags-cli/tests/fixtures/baselines\n{}",
        failures.join("\n")
    );
}

/// Find the (resource, op_id) of the first op matching `pred`. Panics if none —
/// a panic here means the projection failed to populate the field under test.
fn find_op(contract: &Contract, pred: impl Fn(&ContractOp) -> bool) -> (String, String) {
    for (resource, ops) in contract {
        for (id, op) in ops {
            if pred(op) {
                return (resource.clone(), id.clone());
            }
        }
    }
    panic!("no operation matched the predicate — projection likely dropped a field");
}

#[test]
fn test_gate_catches_simulated_breaking_changes() {
    // Use the live schema as both sides so this gate-logic test stays robust to
    // baseline staleness — a csm spec refresh without a baseline regen must not
    // disable it. (Baseline-vs-schema currency is covered by `test_baseline_is_current`.)
    let current = contract_from_schema(&load_service("csm"));
    let baseline = current.clone();

    // Sanity: a self-consistent pair yields no findings, and the projection
    // actually captured request bodies and params (guards the "field never set"
    // bug that would otherwise make the mutations below silently no-op).
    assert!(freshness_diffs("csm", &baseline, &current).is_empty());
    assert!(breaking_changes("csm", &baseline, &current).is_empty());
    assert!(
        current
            .values()
            .flat_map(|ops| ops.values())
            .any(|op| op.has_request_body),
        "projection must capture at least one request body"
    );
    assert!(
        current
            .values()
            .flat_map(|ops| ops.values())
            .any(|op| !op.params.is_empty()),
        "projection must capture parameters"
    );

    // Op deleted.
    {
        let mut cur = current.clone();
        let (res, id) = find_op(&current, |_| true);
        cur.get_mut(&res).unwrap().remove(&id);
        assert!(
            !breaking_changes("csm", &baseline, &cur).is_empty(),
            "deleted op must break"
        );
    }
    // Param removed.
    {
        let mut cur = current.clone();
        let (res, id) = find_op(&current, |op| !op.params.is_empty());
        cur.get_mut(&res)
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .params
            .clear();
        assert!(
            !breaking_changes("csm", &baseline, &cur).is_empty(),
            "removed param must break"
        );
    }
    // Request body removed (true -> false).
    {
        let mut cur = current.clone();
        let (res, id) = find_op(&current, |op| op.has_request_body);
        cur.get_mut(&res)
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .has_request_body = false;
        assert!(
            !breaking_changes("csm", &baseline, &cur).is_empty(),
            "body removal must break"
        );
    }
    // HTTP method changed (pick a non-PATCH op so the new verb is guaranteed different).
    {
        let mut cur = current.clone();
        let (res, id) = find_op(&current, |op| op.http_method != "PATCH");
        cur.get_mut(&res).unwrap().get_mut(&id).unwrap().http_method = "PATCH".to_string();
        assert!(
            !breaking_changes("csm", &baseline, &cur).is_empty(),
            "http_method change must break"
        );
    }
    // Path changed.
    {
        let mut cur = current.clone();
        let (res, id) = find_op(&current, |_| true);
        cur.get_mut(&res).unwrap().get_mut(&id).unwrap().path = "/changed-path".to_string();
        assert!(
            !breaking_changes("csm", &baseline, &cur).is_empty(),
            "path change must break"
        );
    }
}

#[test]
fn test_gate_ignores_additive_changes_that_freshness_catches() {
    // Live schema as both sides (see `test_gate_catches_simulated_breaking_changes`):
    // keeps this gate-logic test independent of baseline currency.
    let current = contract_from_schema(&load_service("csm"));
    let baseline = current.clone();
    assert!(freshness_diffs("csm", &baseline, &current).is_empty());

    // New op added.
    {
        let mut cur = current.clone();
        // The synthetic op-id is unique, so which resource bucket it lands in does not matter.
        let (res, _) = find_op(&current, |_| true);
        cur.get_mut(&res).unwrap().insert(
            "csm/admin/synthetic/v1/created".to_string(),
            ContractOp {
                http_method: "POST".to_string(),
                path: "/synthetic".to_string(),
                params: std::collections::BTreeSet::new(),
                has_request_body: false,
                summary: "synthetic".to_string(),
            },
        );
        assert!(
            breaking_changes("csm", &baseline, &cur).is_empty(),
            "added op must not break"
        );
        assert!(
            !freshness_diffs("csm", &baseline, &cur).is_empty(),
            "added op must fail freshness"
        );
    }
    // New param added.
    {
        let mut cur = current.clone();
        let (res, id) = find_op(&current, |_| true);
        cur.get_mut(&res)
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .params
            .insert(("synthetic".to_string(), "query".to_string(), false));
        assert!(
            breaking_changes("csm", &baseline, &cur).is_empty(),
            "added param must not break"
        );
        assert!(
            !freshness_diffs("csm", &baseline, &cur).is_empty(),
            "added param must fail freshness"
        );
    }
    // Summary changed.
    {
        let mut cur = current.clone();
        let (res, id) = find_op(&current, |_| true);
        cur.get_mut(&res).unwrap().get_mut(&id).unwrap().summary = "CHANGED".to_string();
        assert!(
            breaking_changes("csm", &baseline, &cur).is_empty(),
            "summary change must not break"
        );
        assert!(
            !freshness_diffs("csm", &baseline, &cur).is_empty(),
            "summary change must fail freshness"
        );
    }
}

#[cfg(test)]
mod split_logic {
    use super::*;

    fn op(has_body: bool, summary: &str, params: &[(&str, &str, bool)]) -> ContractOp {
        ContractOp {
            http_method: "POST".to_string(),
            path: "/x".to_string(),
            params: params
                .iter()
                .map(|(n, l, r)| (n.to_string(), l.to_string(), *r))
                .collect(),
            has_request_body: has_body,
            summary: summary.to_string(),
        }
    }

    fn contract(ops: &[(&str, ContractOp)]) -> Contract {
        let mut inner = std::collections::BTreeMap::new();
        for (id, o) in ops {
            inner.insert((*id).to_string(), o.clone());
        }
        let mut outer = std::collections::BTreeMap::new();
        outer.insert("res".to_string(), inner);
        outer
    }

    #[test]
    fn test_breaking_flags_deleted_op() {
        let base = contract(&[("svc/admin/res/v1/get", op(false, "s", &[]))]);
        let cur = contract(&[]);
        assert!(!breaking_changes("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_breaking_ignores_added_op() {
        let base: Contract = std::collections::BTreeMap::new();
        let cur = contract(&[("svc/admin/res/v1/get", op(false, "s", &[]))]);
        assert!(breaking_changes("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_breaking_flags_removed_param() {
        let base = contract(&[(
            "svc/admin/res/v1/get",
            op(false, "s", &[("id", "query", false)]),
        )]);
        let cur = contract(&[("svc/admin/res/v1/get", op(false, "s", &[]))]);
        assert!(!breaking_changes("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_breaking_ignores_added_param() {
        let base = contract(&[("svc/admin/res/v1/get", op(false, "s", &[]))]);
        let cur = contract(&[(
            "svc/admin/res/v1/get",
            op(false, "s", &[("id", "query", false)]),
        )]);
        assert!(breaking_changes("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_breaking_flags_has_request_body_flip() {
        let base = contract(&[("svc/admin/res/v1/get", op(true, "s", &[]))]);
        let cur = contract(&[("svc/admin/res/v1/get", op(false, "s", &[]))]);
        assert!(!breaking_changes("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_breaking_ignores_summary_change() {
        let base = contract(&[("svc/admin/res/v1/get", op(false, "old", &[]))]);
        let cur = contract(&[("svc/admin/res/v1/get", op(false, "new", &[]))]);
        assert!(breaking_changes("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_freshness_flags_added_op() {
        let base: Contract = std::collections::BTreeMap::new();
        let cur = contract(&[("svc/admin/res/v1/get", op(false, "s", &[]))]);
        assert!(!freshness_diffs("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_freshness_flags_summary_change() {
        let base = contract(&[("svc/admin/res/v1/get", op(false, "old", &[]))]);
        let cur = contract(&[("svc/admin/res/v1/get", op(false, "new", &[]))]);
        assert!(!freshness_diffs("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_freshness_passes_on_identical() {
        let base = contract(&[(
            "svc/admin/res/v1/get",
            op(true, "s", &[("id", "query", true)]),
        )]);
        let cur = base.clone();
        assert!(freshness_diffs("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_breaking_flags_path_change() {
        let base = contract(&[("svc/admin/res/v1/get", op(false, "s", &[]))]);
        let mut cur = base.clone();
        cur.get_mut("res")
            .unwrap()
            .get_mut("svc/admin/res/v1/get")
            .unwrap()
            .path = "/changed".to_string();
        assert!(!breaking_changes("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_breaking_flags_deleted_resource() {
        let base = contract(&[("svc/admin/res/v1/get", op(false, "s", &[]))]);
        let cur: Contract = std::collections::BTreeMap::new();
        assert!(!breaking_changes("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_freshness_flags_new_resource() {
        let base: Contract = std::collections::BTreeMap::new();
        let cur = contract(&[("svc/admin/res/v1/get", op(false, "s", &[]))]);
        assert!(!freshness_diffs("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_breaking_flags_param_required_flip() {
        let base = contract(&[(
            "svc/admin/res/v1/get",
            op(false, "s", &[("id", "query", true)]),
        )]);
        let cur = contract(&[(
            "svc/admin/res/v1/get",
            op(false, "s", &[("id", "query", false)]),
        )]);
        assert!(!breaking_changes("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_freshness_flags_http_method_change() {
        let base = contract(&[("svc/admin/res/v1/get", op(false, "s", &[]))]);
        let mut cur = base.clone();
        cur.get_mut("res")
            .unwrap()
            .get_mut("svc/admin/res/v1/get")
            .unwrap()
            .http_method = "PUT".to_string();
        assert!(!freshness_diffs("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_freshness_flags_path_change() {
        let base = contract(&[("svc/admin/res/v1/get", op(false, "s", &[]))]);
        let mut cur = base.clone();
        cur.get_mut("res")
            .unwrap()
            .get_mut("svc/admin/res/v1/get")
            .unwrap()
            .path = "/changed".to_string();
        assert!(!freshness_diffs("svc", &base, &cur).is_empty());
    }

    #[test]
    fn test_freshness_flags_has_request_body_flip() {
        let base = contract(&[("svc/admin/res/v1/get", op(false, "s", &[]))]);
        let mut cur = base.clone();
        cur.get_mut("res")
            .unwrap()
            .get_mut("svc/admin/res/v1/get")
            .unwrap()
            .has_request_body = true;
        assert!(!freshness_diffs("svc", &base, &cur).is_empty());
    }
}
