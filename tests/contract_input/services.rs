use std::collections::{HashMap, HashSet};

use ags::catalogue::Catalogue;
use ags::protocol::catalogue::{ParameterLocation, ParameterSchema, ServiceSchema};

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

fn operation_id(value: &serde_json::Value) -> &str {
    value["x_operation_id"]
        .as_str()
        .expect("baseline operation should include x_operation_id")
}

#[test]
fn test_naming_matches_baseline() {
    let mut failures: Vec<String> = Vec::new();

    for service in Catalogue::service_ids() {
        let schema = load_service(service);
        let baseline = load_baseline(service);

        let rust_resources: HashSet<&str> =
            schema.resources.iter().map(|r| r.name.as_str()).collect();
        let baseline_resources: HashSet<&str> = baseline.keys().map(|k| k.as_str()).collect();

        let missing_resources: Vec<&&str> =
            baseline_resources.difference(&rust_resources).collect();
        let extra_resources: Vec<&&str> = rust_resources.difference(&baseline_resources).collect();
        if !missing_resources.is_empty() || !extra_resources.is_empty() {
            failures.push(format!(
                "[{service}] resource mismatch — missing: {missing_resources:?}, extra: {extra_resources:?}"
            ));
            continue;
        }

        for resource in &schema.resources {
            let rust_methods: HashSet<&str> =
                resource.operations().map(|o| o.id.as_str()).collect();
            let baseline_methods: HashSet<&str> =
                baseline[&resource.name].iter().map(operation_id).collect();

            let missing: Vec<&&str> = baseline_methods.difference(&rust_methods).collect();
            let extra: Vec<&&str> = rust_methods.difference(&baseline_methods).collect();
            if !missing.is_empty() || !extra.is_empty() {
                failures.push(format!(
                    "[{service}] {}: missing {missing:?}, extra {extra:?}",
                    resource.name
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "naming mismatches:\n{}",
        failures.join("\n")
    );
}

#[test]
fn test_summaries_match_baseline() {
    let mut failures: Vec<String> = Vec::new();

    for service in Catalogue::service_ids() {
        let schema = load_service(service);
        let baseline = load_baseline(service);

        for resource in &schema.resources {
            let Some(baseline_methods) = baseline.get(&resource.name) else {
                continue;
            };
            let baseline_summaries: HashMap<&str, &str> = baseline_methods
                .iter()
                .map(|m| (operation_id(m), m["summary"].as_str().unwrap_or("")))
                .collect();

            for op in resource.operations() {
                if let Some(&expected) = baseline_summaries.get(op.id.as_str()) {
                    if op.summary != expected {
                        failures.push(format!(
                            "[{service}] {} [{}]: rust={:?}, baseline={:?}",
                            resource.name, op.id, op.summary, expected
                        ));
                    }
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "summary mismatches:\n{}",
        failures.join("\n")
    );
}

/// Tuple form of a parameter for set-equality comparison: (name, location, required).
type ParameterKey = (String, &'static str, bool);

/// Render a `ParameterLocation` as the same lowercase string the baseline JSON uses.
fn location_as_string(location: ParameterLocation) -> &'static str {
    match location {
        ParameterLocation::Path => "path",
        ParameterLocation::Query => "query",
        ParameterLocation::Header => "header",
        ParameterLocation::Body => "body",
        ParameterLocation::FormData => "form_data",
        _ => unreachable!("ParameterLocation gained a variant; update this match"),
    }
}

/// Project a parsed operation's parameters into the comparison key set.
fn rust_parameter_keys(parameters: &[ParameterSchema]) -> HashSet<ParameterKey> {
    parameters
        .iter()
        .map(|parameter| {
            (
                parameter.name.clone(),
                location_as_string(parameter.location),
                parameter.required,
            )
        })
        .collect()
}

/// Project a baseline operation's `parameters` array into the comparison key set.
fn baseline_parameter_keys(value: &serde_json::Value) -> HashSet<ParameterKey> {
    let parameters = value["parameters"]
        .as_array()
        .expect("baseline operation should include parameters array");
    parameters
        .iter()
        .map(|parameter| {
            let name = parameter["name"].as_str().unwrap_or("").to_string();
            let location = match parameter["location"].as_str().unwrap_or("") {
                "path" => "path",
                "query" => "query",
                "header" => "header",
                "body" => "body",
                "form_data" => "form_data",
                other => panic!("unknown baseline location: {other}"),
            };
            let required = parameter["required"].as_bool().unwrap_or(false);
            (name, location, required)
        })
        .collect()
}

#[test]
fn test_parameters_match_baseline() {
    let mut failures: Vec<String> = Vec::new();

    for service in Catalogue::service_ids() {
        let schema = load_service(service);
        let baseline = load_baseline(service);

        for resource in &schema.resources {
            let Some(baseline_methods) = baseline.get(&resource.name) else {
                continue;
            };
            let baseline_by_id: HashMap<&str, &serde_json::Value> = baseline_methods
                .iter()
                .map(|method| (operation_id(method), method))
                .collect();

            for operation in resource.operations() {
                let Some(baseline_operation) = baseline_by_id.get(operation.id.as_str()) else {
                    continue;
                };
                let observed = rust_parameter_keys(&operation.parameters);
                let expected = baseline_parameter_keys(baseline_operation);

                let missing: Vec<&ParameterKey> = expected.difference(&observed).collect();
                let extra: Vec<&ParameterKey> = observed.difference(&expected).collect();
                if !missing.is_empty() || !extra.is_empty() {
                    failures.push(format!(
                        "[{service}] {} [{}]: missing {missing:?}, extra {extra:?}",
                        resource.name, operation.id
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "parameter contract drift:\n{}",
        failures.join("\n")
    );
}
