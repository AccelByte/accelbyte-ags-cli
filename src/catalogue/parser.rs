//! Parse `SwaggerSpec` into protocol `ServiceSchema` using `x-operationId` values.

use std::collections::{HashMap, HashSet};

use super::manifest::{get_resource_description, service_description};
use super::openapi::{SwaggerDefinition, SwaggerProperty, SwaggerSpec};
use crate::protocol::catalogue::{
    ApiVersion, BodyField, BodyFieldType, BodySchema, HttpMethod, MethodSchema, MutationClass,
    OperationId, OperationSchema, ParameterLocation, ParameterSchema, ResourceSchema, ScopeEntry,
    ServiceSchema, ValueType,
};
use crate::support::strings::draft_resource_description;

/// Parse a SwaggerSpec into a protocol `ServiceSchema`.
///
/// All operations are expected to carry `x-operationId` of the form
/// `service/scope/resource/version/method`. Operations without `x-operationId`,
/// marked deprecated, or scoped under the `internal` resource are skipped.
/// Every surviving `(scope, version)` contract for a `(resource, method)`
/// pair is preserved.
///
/// # Panics (debug only)
///
/// Fires `debug_assert!` — and therefore panics in debug builds — when:
/// - Two non-deprecated operations share the same `(resource, method, scope, version)` key.
/// - A method has multiple non-admin scopes with no admin scope present.
///
/// In release builds both cases are handled gracefully (first entry wins for
/// duplicates; `default_scope` is `None` for the ambiguous-scope case).
pub fn parse_spec(service_name: &str, spec: &SwaggerSpec) -> ServiceSchema {
    // Accumulate into (resource, method) -> scope -> version -> OperationSchema.
    // Every non-deprecated contract is preserved. Task 2.2 consumes the richer
    // shape; nothing here filters by scope.
    let mut accumulator: HashMap<
        (String, String),
        HashMap<String, HashMap<ApiVersion, OperationSchema>>,
    > = HashMap::new();

    for (path, methods) in &spec.paths {
        for http_method_name in &["get", "post", "put", "patch", "delete"] {
            let Some(operation) = methods.get(*http_method_name) else {
                continue;
            };
            if operation.is_deprecated {
                continue;
            }
            let Some(x_operation_id) = operation
                .x_operation_id
                .as_deref()
                .filter(|s| !s.is_empty())
            else {
                continue;
            };

            // Expect exactly 5 segments: service/scope/resource/version/method.
            // Both underflow (too few) and overflow (a stray slash that pushes
            // the method into a 6th segment) are spec bugs. Without this
            // check, `splitn` would silently fold the overflow into `parts[4]`
            // and the version-parse below would fail-and-continue, dropping
            // the operation from the CLI surface with no signal.
            let parts: Vec<&str> = x_operation_id.split('/').collect();
            if parts.len() != 5 {
                debug_assert!(
                    false,
                    "x-operationId must have exactly 5 segments \
                     (service/scope/resource/version/method), got {} in '{x_operation_id}'",
                    parts.len(),
                );
                continue;
            }
            let scope = parts[1].to_string();
            let resource = parts[2].to_string();
            if resource == "internal" {
                continue;
            }
            let method_name = parts[4].to_string();
            // Parse the version integer from "v1", "v2", etc. A non-numeric
            // segment here means the spec is malformed — assert in debug so it
            // surfaces during development, skip in release so one bad op does
            // not poison the whole service. Coercing to 0 would create a
            // ghost contract selectable as `--api-version v0`.
            let api_version: ApiVersion = match parts[3].parse() {
                Ok(v) => v,
                Err(_) => {
                    debug_assert!(
                        false,
                        "x-operationId version segment must be numeric (e.g. 'v1' style is allowed \
                         only if ApiVersion parses it), got '{}' in '{x_operation_id}'",
                        parts[3],
                    );
                    continue;
                }
            };

            let http_method_upper = http_method_name.to_uppercase();
            let Some(http_method) = parse_http_method(&http_method_upper) else {
                continue;
            };
            let parameters = get_operation_params(spec, path, &http_method_upper);
            let has_body = operation.parameters.iter().any(|p| p.location == "body");
            let request_body = if has_body {
                resolve_body_schema(spec, path, &http_method_upper)
            } else {
                None
            };
            let permissions = extract_permissions(&operation.x_security);
            let description =
                (!operation.description.is_empty()).then(|| operation.description.clone());
            let mutation_class = classify_mutation_class(http_method, &method_name, &permissions);

            let schema = OperationSchema {
                id: OperationId::new(x_operation_id),
                name: method_name.clone(),
                summary: operation.summary.trim().to_string(),
                description,
                mutation_class,
                http_method,
                path_template: path.clone(),
                parameters,
                request_body,
                response: None,
                permissions,
                scope: scope.clone(),
                api_version,
                deprecated: false,
                response_content_type: operation.produces.first().cloned(),
            };

            let key = (resource.clone(), method_name.clone());
            let scope_bucket = accumulator.entry(key).or_default();
            let version_bucket = scope_bucket.entry(scope.clone()).or_default();
            if let Some(existing) = version_bucket.insert(api_version, schema) {
                // Duplicate x-operationId: keep the first-seen entry. Because
                // spec.paths is a HashMap, "first" is iteration order — the
                // winner is arbitrary. This case only arises from malformed
                // specs; well-formed specs have unique x-operationId values.
                debug_assert!(
                    false,
                    "duplicate x-operationId contract at \
                     (resource={resource}, method={method_name}, scope={scope}, version={api_version}): \
                     existing id '{}' collides with new id",
                    existing.id,
                );
                version_bucket.insert(api_version, existing);
            }
        }
    }

    // Re-group by resource so we can build ResourceSchema/MethodSchema.
    let mut resource_map: HashMap<String, Vec<MethodSchema>> = HashMap::new();

    for ((resource_name, method_name), scope_map) in accumulator {
        // Decide default_scope: admin preferred, else sole scope, else None.
        let mut scope_names: Vec<&str> = scope_map.keys().map(String::as_str).collect();
        scope_names.sort(); // deterministic order for the debug_assert! message below
        let default_scope = if scope_map.contains_key("admin") {
            Some("admin".to_string())
        } else if scope_names.len() == 1 {
            Some(scope_names[0].to_string())
        } else {
            debug_assert!(
                false,
                "method {resource_name}/{method_name} has multiple non-admin scopes {scope_names:?}; \
                 expected only admin/public"
            );
            None
        };

        // Build ScopeEntry list, sorted alphabetically by scope name.
        let mut scopes: Vec<ScopeEntry> = scope_map
            .into_iter()
            .map(|(scope, version_map)| {
                let mut contracts: Vec<OperationSchema> = version_map.into_values().collect();
                contracts.sort_by_key(|contract| contract.api_version);
                let default_version = contracts
                    .iter()
                    .map(|contract| contract.api_version)
                    .max()
                    .unwrap_or(ApiVersion(0));
                ScopeEntry {
                    scope,
                    default_version,
                    contracts,
                }
            })
            .collect();
        scopes.sort_by(|a, b| a.scope.cmp(&b.scope));

        // Summary for the MethodSchema: take the default-scope/default-version
        // contract's summary when available, otherwise the first contract's.
        let summary = scopes
            .iter()
            .find(|scope_entry| Some(&scope_entry.scope) == default_scope.as_ref())
            .and_then(|scope_entry| {
                scope_entry
                    .contracts
                    .iter()
                    .find(|contract| contract.api_version == scope_entry.default_version)
                    .map(|contract| contract.summary.clone())
            })
            .or_else(|| {
                scopes
                    .first()
                    .and_then(|s| s.contracts.first().map(|c| c.summary.clone()))
            })
            .unwrap_or_default();

        resource_map
            .entry(resource_name)
            .or_default()
            .push(MethodSchema {
                name: method_name,
                summary,
                default_scope,
                scopes,
            });
    }

    let service_description = service_description(service_name).to_string();

    let mut resources: Vec<ResourceSchema> = resource_map
        .into_iter()
        .map(|(name, mut methods)| {
            methods.sort_by(|a, b| a.name.cmp(&b.name));
            let description = get_resource_description(service_name, &name)
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    let method_names: Vec<&str> = methods.iter().map(|m| m.name.as_str()).collect();
                    draft_resource_description(&name, &method_names)
                });
            ResourceSchema {
                name,
                description,
                methods,
            }
        })
        .collect();
    resources.sort_by(|a, b| a.name.cmp(&b.name));

    ServiceSchema {
        name: service_name.to_string(),
        description: service_description,
        resources,
    }
}

/// Extract permission strings from the `x-security` field.
/// Handles: [{"userPermissions": ["PERM1", "PERM2"]}], {}, null
fn extract_permissions(x_security: &serde_json::Value) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Item {
        #[serde(default, rename = "userPermissions")]
        permissions: Option<Vec<String>>,
    }
    serde_json::from_value::<Vec<Item>>(x_security.clone())
        .ok()
        .and_then(|items| items.into_iter().find_map(|item| item.permissions))
        .unwrap_or_default()
}

/// Extract parameters for an operation from the spec.
fn get_operation_params(spec: &SwaggerSpec, path: &str, http_method: &str) -> Vec<ParameterSchema> {
    let method_key = http_method.to_lowercase();
    if let Some(path_item) = spec.paths.get(path) {
        if let Some(operation) = path_item.get(&method_key) {
            return operation
                .parameters
                .iter()
                .filter(|parameter| parameter.location != "body")
                .filter_map(|parameter| {
                    let location = parse_parameter_location(&parameter.location)?;
                    let value_type = value_type_from_parameter(
                        &parameter.parameter_type,
                        parameter.enum_values.as_deref(),
                    )?;
                    Some(ParameterSchema {
                        name: parameter.name.clone(),
                        location,
                        required: parameter.is_required,
                        value_type,
                        description: (!parameter.description.is_empty())
                            .then(|| parameter.description.clone()),
                    })
                })
                .collect();
        }
    }
    Vec::new()
}

/// Parse an OpenAPI HTTP method string into the protocol enum.
fn parse_http_method(method_name: &str) -> Option<HttpMethod> {
    match method_name.to_ascii_uppercase().as_str() {
        "GET" => Some(HttpMethod::Get),
        "POST" => Some(HttpMethod::Post),
        "PUT" => Some(HttpMethod::Put),
        "PATCH" => Some(HttpMethod::Patch),
        "DELETE" => Some(HttpMethod::Delete),
        other => {
            debug_assert!(false, "unsupported HTTP method in spec: {other}");
            None
        }
    }
}

/// Parse an OpenAPI parameter location into the protocol enum.
fn parse_parameter_location(location_name: &str) -> Option<ParameterLocation> {
    match location_name {
        "path" => Some(ParameterLocation::Path),
        "query" => Some(ParameterLocation::Query),
        "header" => Some(ParameterLocation::Header),
        "body" => Some(ParameterLocation::Body),
        "formData" => Some(ParameterLocation::FormData),
        other => {
            debug_assert!(false, "unsupported parameter location in spec: {other}");
            None
        }
    }
}

/// Convert an OpenAPI parameter type plus optional enum list into a protocol value type.
fn value_type_from_parameter(type_name: &str, enum_values: Option<&[String]>) -> Option<ValueType> {
    if let Some(values) = enum_values {
        return Some(ValueType::Enum(values.to_vec()));
    }
    match type_name {
        "string" => Some(ValueType::String),
        "integer" => Some(ValueType::Integer),
        "number" => Some(ValueType::Number),
        "boolean" => Some(ValueType::Boolean),
        "array" => Some(ValueType::Array(Box::new(ValueType::String))),
        "file" => Some(ValueType::String),
        other => {
            debug_assert!(false, "unsupported parameter type in spec: {other}");
            None
        }
    }
}

/// Classify an OpenAPI operation into the protocol mutation class.
fn classify_mutation_class(
    method: HttpMethod,
    name: &str,
    permissions: &[String],
) -> MutationClass {
    if method == HttpMethod::Get {
        return MutationClass::ReadOnly;
    }
    if method != HttpMethod::Post {
        return MutationClass::Mutating;
    }
    match classify_by_permissions(permissions) {
        Some(class) => class,
        None if is_read_operation_name(name) => MutationClass::ReadOnly,
        None => MutationClass::Mutating,
    }
}

/// Inspect action brackets on permission strings (for example `[READ]` or
/// `[READ, UPDATE]`) and classify any non-read action as mutating.
fn classify_by_permissions(permissions: &[String]) -> Option<MutationClass> {
    let mut saw_bracket = false;
    let mut saw_write = false;
    for permission in permissions {
        for action in extract_bracketed_actions(permission) {
            saw_bracket = true;
            if action != "READ" {
                saw_write = true;
            }
        }
    }
    if !saw_bracket {
        return None;
    }
    Some(if saw_write {
        MutationClass::Mutating
    } else {
        MutationClass::ReadOnly
    })
}

/// Extract the comma-separated action tokens from the first `[...]` bracket in
/// a permission string. Missing brackets are treated as "no signal".
fn extract_bracketed_actions(permission: &str) -> Vec<String> {
    let Some(open) = permission.rfind('[') else {
        return Vec::new();
    };
    let Some(close_offset) = permission[open..].find(']') else {
        return Vec::new();
    };
    permission[open + 1..open + close_offset]
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Return `true` when a POST operation name follows the known read-only query
/// patterns used by the catalogue specs.
fn is_read_operation_name(name: &str) -> bool {
    const READ_PREFIXES: &[&str] = &[
        "list", "get", "bulk-get", "search", "query", "export", "find", "fetch",
    ];
    READ_PREFIXES
        .iter()
        .any(|&prefix| name == prefix || name.starts_with(&format!("{prefix}-")))
}

/// Extract the short name from a `$ref` path like `#/definitions/FooBar`.
fn ref_short_name(ref_path: &str) -> &str {
    ref_path.rsplit('/').next().unwrap_or(ref_path)
}

/// Compute the typed `BodyFieldType` for a schema property.
fn compute_field_type(prop: &SwaggerProperty) -> BodyFieldType {
    if let Some(ref ref_path) = prop.ref_path {
        return BodyFieldType::Reference(ref_short_name(ref_path).to_string());
    }
    if prop.property_type == "array" {
        let inner = match &prop.items {
            Some(items) if items.ref_path.is_some() => BodyFieldType::Reference(
                ref_short_name(items.ref_path.as_ref().unwrap()).to_string(),
            ),
            Some(items) if !items.property_type.is_empty() => {
                scalar_type_from_swagger(&items.property_type)
            }
            _ => BodyFieldType::Object,
        };
        return BodyFieldType::Array(Box::new(inner));
    }
    if prop.property_type.is_empty() {
        BodyFieldType::Object
    } else {
        scalar_type_from_swagger(&prop.property_type)
    }
}

/// Map an OpenAPI primitive type name to a `BodyFieldType` scalar variant.
fn scalar_type_from_swagger(type_name: &str) -> BodyFieldType {
    match type_name {
        "string" => BodyFieldType::String,
        "integer" => BodyFieldType::Integer,
        "boolean" => BodyFieldType::Boolean,
        "number" => BodyFieldType::Number,
        // Anything unrecognised (e.g. "object" inline) falls through to Object.
        _ => BodyFieldType::Object,
    }
}

/// Resolve the fields of a definition into protocol `BodyField`s.
/// Recursively expands nested `$ref` and array-of-object schemas up to `max_depth`.
fn resolve_definition_fields(spec: &SwaggerSpec, definition: &SwaggerDefinition) -> Vec<BodyField> {
    let mut visited = HashSet::new();
    resolve_definition_fields_recursive(spec, definition, 0, &mut visited)
}

/// Recursively resolve definition fields, expanding nested `$ref` schemas up to depth 3.
fn resolve_definition_fields_recursive(
    spec: &SwaggerSpec,
    definition: &SwaggerDefinition,
    depth: usize,
    visited: &mut HashSet<String>,
) -> Vec<BodyField> {
    let required_set: HashSet<&str> = definition.required.iter().map(|s| s.as_str()).collect();

    let mut fields: Vec<BodyField> = definition
        .properties
        .iter()
        .map(|(name, prop)| {
            let (children, _) = resolve_nested(spec, prop, depth, visited);
            let field_type = if let Some(ref values) = prop.enum_values {
                // Enum value override — wraps the type as Enum regardless of base type.
                // Today only string enums are supported in our specs.
                BodyFieldType::Enum(values.clone())
            } else {
                compute_field_type(prop)
            };
            BodyField {
                name: name.clone(),
                field_type,
                required: required_set.contains(name.as_str()),
                description: (!prop.description.is_empty()).then(|| prop.description.clone()),
                children,
            }
        })
        .collect();

    // Sort: required first, then alphabetical within each group
    fields.sort_by(|a, b| b.required.cmp(&a.required).then(a.name.cmp(&b.name)));
    fields
}

/// Resolve nested children for a property that references another definition.
/// Returns (children, is_array).
fn resolve_nested(
    spec: &SwaggerSpec,
    prop: &SwaggerProperty,
    depth: usize,
    visited: &mut HashSet<String>,
) -> (Vec<BodyField>, bool) {
    if depth >= 3 {
        return (Vec::new(), false);
    }

    // Direct $ref → nested object
    if let Some(ref ref_path) = prop.ref_path {
        let def_name = ref_short_name(ref_path).to_string();
        if visited.contains(&def_name) {
            return (Vec::new(), false);
        }
        if let Some(def) = spec.definitions.get(&def_name) {
            visited.insert(def_name.clone());
            let children = resolve_definition_fields_recursive(spec, def, depth + 1, visited);
            visited.remove(&def_name);
            return (children, false);
        }
    }

    // Array with $ref items → array of nested objects
    if prop.property_type == "array" {
        if let Some(ref items) = prop.items {
            if let Some(ref ref_path) = items.ref_path {
                let def_name = ref_short_name(ref_path).to_string();
                if visited.contains(&def_name) {
                    return (Vec::new(), true);
                }
                if let Some(def) = spec.definitions.get(&def_name) {
                    visited.insert(def_name.clone());
                    let children =
                        resolve_definition_fields_recursive(spec, def, depth + 1, visited);
                    visited.remove(&def_name);
                    return (children, true);
                }
            }
        }
    }

    (Vec::new(), false)
}

/// Resolve the body schema for an operation by finding its body parameter's `$ref`.
fn resolve_body_schema(spec: &SwaggerSpec, path: &str, http_method: &str) -> Option<BodySchema> {
    let method_key = http_method.to_lowercase();
    let path_item = spec.paths.get(path)?;
    let operation = path_item.get(&method_key)?;

    // Find the body parameter with a schema.$ref
    let body_param = operation.parameters.iter().find(|p| p.location == "body")?;

    let schema_ref = body_param.schema.as_ref()?;
    let ref_path = schema_ref.ref_path.as_deref()?;
    let def_name = ref_short_name(ref_path);
    let definition = spec.definitions.get(def_name)?;

    let fields = resolve_definition_fields(spec, definition);
    if fields.is_empty() {
        return None;
    }

    Some(BodySchema {
        definition_name: def_name.to_string(),
        fields,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::openapi::{SwaggerInfo, SwaggerOperation};

    /// Build a minimal SwaggerSpec with the given operations.
    /// Each entry is (path, http_method, x_operation_id, deprecated, summary).
    fn spec_from(ops: &[(&str, &str, &str, bool, &str)]) -> SwaggerSpec {
        let mut paths: HashMap<String, HashMap<String, SwaggerOperation>> = HashMap::new();
        for (path, method, x_op_id, deprecated, summary) in ops {
            let operation = SwaggerOperation {
                is_deprecated: *deprecated,
                summary: summary.to_string(),
                description: String::new(),
                parameters: Vec::new(),
                x_security: serde_json::Value::Null,
                x_operation_id: Some(x_op_id.to_string()),
                produces: Vec::new(),
            };
            paths
                .entry(path.to_string())
                .or_default()
                .insert(method.to_string(), operation);
        }
        SwaggerSpec {
            swagger: "2.0".to_string(),
            info: SwaggerInfo::default(),
            paths,
            definitions: HashMap::new(),
        }
    }

    /// Test shim: collect every operation on a resource into a sorted `Vec`
    /// so assertions can compare against a flat list.
    fn flatten_ops(resource: &ResourceSchema) -> Vec<OperationSchema> {
        let mut ops: Vec<OperationSchema> = resource.operations().cloned().collect();
        ops.sort_by(|a, b| a.name.cmp(&b.name));
        ops
    }

    /// Build a minimal SwaggerSpec from (x_operation_id, path, http_method, deprecated) tuples.
    /// Summary defaults to empty; used by tests that don't care about metadata.
    fn make_spec_with_operations(ops: Vec<(&str, &str, &str, bool)>) -> SwaggerSpec {
        let reshaped: Vec<(&str, &str, &str, bool, &str)> = ops
            .into_iter()
            .map(|(x_op_id, path, method, deprecated)| (path, method, x_op_id, deprecated, ""))
            .collect();
        spec_from(&reshaped)
    }

    /// Admin and public contracts for the same method should be preserved as
    /// separate scope entries rather than collapsing to one.
    #[test]
    fn test_parser_preserves_admin_and_public_scopes_for_same_method() {
        let spec = make_spec_with_operations(vec![
            (
                "iam/admin/users/v4/get",
                "/iam/v4/admin/namespaces/{namespace}/users/{userId}",
                "get",
                false,
            ),
            (
                "iam/public/users/v3/get",
                "/iam/v3/public/namespaces/{namespace}/users/{userId}",
                "get",
                false,
            ),
        ]);
        let service = parse_spec("iam", &spec);

        let users = service
            .resources
            .iter()
            .find(|r| r.name == "users")
            .unwrap();
        let get = users.methods.iter().find(|m| m.name == "get").unwrap();

        let scope_names: Vec<_> = get.scopes.iter().map(|s| s.scope.as_str()).collect();
        assert!(scope_names.contains(&"admin"));
        assert!(scope_names.contains(&"public"));
    }

    /// Resource and method names should be derived from the `x-operationId`
    /// segments, not the raw URL path.
    #[test]
    fn test_derives_resource_and_method_from_x_operation_id() {
        let spec = spec_from(&[
            (
                "/api/v1/bans",
                "get",
                "svc/admin/bans/v1/list",
                false,
                "List bans",
            ),
            (
                "/api/v1/bans",
                "post",
                "svc/admin/bans/v1/create",
                false,
                "Create ban",
            ),
        ]);
        let service = parse_spec("test", &spec);
        assert_eq!(service.resources.len(), 1);
        assert_eq!(service.resources[0].name, "bans");
        let ops = flatten_ops(&service.resources[0]);
        let methods: Vec<&str> = ops
            .iter()
            .map(|operation| operation.name.as_str())
            .collect();
        assert!(methods.contains(&"list"));
        assert!(methods.contains(&"create"));
    }

    /// Operations with different resource segments should group into distinct
    /// resource buckets sorted by resource name.
    #[test]
    fn test_groups_by_resource() {
        let spec = spec_from(&[
            ("/api/v1/bans", "get", "svc/admin/bans/v1/list", false, ""),
            ("/api/v1/users", "get", "svc/admin/users/v1/list", false, ""),
        ]);
        let service = parse_spec("test", &spec);
        let names: Vec<&str> = service
            .resources
            .iter()
            .map(|resource| resource.name.as_str())
            .collect();
        assert_eq!(names, vec!["bans", "users"]); // sorted alphabetically
    }

    /// Deprecated operations should be excluded from the parsed catalogue.
    #[test]
    fn test_skips_deprecated_operations() {
        let spec = spec_from(&[
            ("/api/v1/bans", "get", "svc/admin/bans/v1/list", true, ""),
            ("/api/v1/users", "get", "svc/admin/users/v1/list", false, ""),
        ]);
        let service = parse_spec("test", &spec);
        assert_eq!(service.resources.len(), 1);
        assert_eq!(service.resources[0].name, "users");
    }

    /// `internal` resources should be excluded from the CLI surface.
    #[test]
    fn test_skips_internal_resource_operations() {
        let spec = spec_from(&[
            (
                "/healthz",
                "get",
                "svc/public/internal/v1/check-health",
                false,
                "",
            ),
            ("/api/v1/users", "get", "svc/admin/users/v1/list", false, ""),
        ]);
        let service = parse_spec("test", &spec);
        assert_eq!(service.resources.len(), 1);
        assert_eq!(service.resources[0].name, "users");
    }

    /// Public-scope operations should survive parsing alongside admin ones.
    #[test]
    fn test_public_scope_is_preserved() {
        let spec = spec_from(&[
            (
                "/api/v1/health",
                "get",
                "svc/public/health/v1/check",
                false,
                "",
            ),
            ("/api/v1/bans", "get", "svc/admin/bans/v1/list", false, ""),
        ]);
        let service = parse_spec("test", &spec);
        // Both resources survive; public scope is no longer filtered
        let names: Vec<&str> = service.resources.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"bans"));
        assert!(names.contains(&"health"));
    }

    /// Recognised non-admin scopes should all be preserved in the service
    /// schema rather than filtered out.
    #[test]
    fn test_all_recognised_scopes_are_preserved() {
        let spec = spec_from(&[
            (
                "/api/v1/tokens",
                "get",
                "svc/server/tokens/v1/list",
                false,
                "",
            ),
            ("/api/v1/prefs", "get", "svc/user/prefs/v1/get", false, ""),
            ("/api/v1/bans", "get", "svc/admin/bans/v1/list", false, ""),
        ]);
        let service = parse_spec("test", &spec);
        // All three resources survive; no scope is discarded
        assert_eq!(service.resources.len(), 3);
        let names: Vec<&str> = service.resources.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"bans"));
        assert!(names.contains(&"tokens"));
        assert!(names.contains(&"prefs"));
    }

    /// An `x-operationId` with fewer than 5 segments is malformed and should
    /// trip the debug assertion so spec drift surfaces during development.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "x-operationId must have exactly 5 segments")]
    fn test_parser_underflow_x_operation_id_asserts_in_debug() {
        let spec = spec_from(&[("/api/v1/bans", "get", "too/few/parts", false, "")]);
        let _ = parse_spec("test", &spec);
    }

    /// In release builds, underflow x-operationId values are skipped so one
    /// bad entry does not poison the whole service.
    #[test]
    #[cfg(not(debug_assertions))]
    fn test_parser_underflow_x_operation_id_skips_in_release() {
        let spec = spec_from(&[
            ("/api/v1/bans", "get", "too/few/parts", false, ""),
            ("/api/v1/users", "get", "svc/admin/users/v1/list", false, ""),
        ]);
        let service = parse_spec("test", &spec);
        assert_eq!(service.resources.len(), 1);
        assert_eq!(service.resources[0].name, "users");
    }

    /// An `x-operationId` with more than 5 segments — a real spec bug seen in
    /// `basic/public/profiles/update/v1/my-zip-code` where a stray slash split
    /// the method name — must trip the debug assertion. Previously this was
    /// silently dropped because `splitn(5, '/')` folded the overflow into the
    /// last segment and the version-parse fail path skipped the op.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "x-operationId must have exactly 5 segments")]
    fn test_parser_overflow_x_operation_id_asserts_in_debug() {
        let spec = spec_from(&[(
            "/basic/v1/public/namespaces/{namespace}/users/me/profiles/zipCode",
            "put",
            "basic/public/profiles/update/v1/my-zip-code",
            false,
            "",
        )]);
        let _ = parse_spec("basic", &spec);
    }

    /// In release builds, overflow x-operationId values are skipped (the
    /// operation is omitted from the CLI surface) rather than panicking.
    #[test]
    #[cfg(not(debug_assertions))]
    fn test_parser_overflow_x_operation_id_skips_in_release() {
        let spec = spec_from(&[
            (
                "/basic/v1/public/namespaces/{namespace}/users/me/profiles/zipCode",
                "put",
                "basic/public/profiles/update/v1/my-zip-code",
                false,
                "",
            ),
            (
                "/api/v1/users",
                "get",
                "svc/admin/users/v1/list",
                false,
                "",
            ),
        ]);
        let service = parse_spec("basic", &spec);
        assert_eq!(service.resources.len(), 1);
        assert_eq!(service.resources[0].name, "users");
    }

    /// A non-numeric version segment in an otherwise well-shaped (5-segment)
    /// `x-operationId` should trip the debug assertion as a spec bug.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "x-operationId version segment must be numeric")]
    fn test_parser_non_numeric_version_asserts_in_debug() {
        let spec = spec_from(&[(
            "/api/v1/bans",
            "get",
            "svc/admin/bans/vX/list",
            false,
            "",
        )]);
        let _ = parse_spec("test", &spec);
    }

    /// In release builds, a non-numeric version segment skips the operation
    /// instead of panicking — the rest of the service still loads.
    #[test]
    #[cfg(not(debug_assertions))]
    fn test_parser_non_numeric_version_skips_in_release() {
        let spec = spec_from(&[
            ("/api/v1/bans", "get", "svc/admin/bans/vX/list", false, ""),
            ("/api/v1/users", "get", "svc/admin/users/v1/list", false, ""),
        ]);
        let service = parse_spec("test", &spec);
        assert_eq!(service.resources.len(), 1);
        assert_eq!(service.resources[0].name, "users");
    }

    /// All non-deprecated versions of a contract should be preserved and the
    /// highest version should become the default.
    #[test]
    fn test_parser_preserves_all_versions_and_picks_highest_default() {
        let spec = make_spec_with_operations(vec![
            (
                "iam/admin/users/v1/get",
                "/iam/v1/admin/users",
                "get",
                false,
            ),
            (
                "iam/admin/users/v2/get",
                "/iam/v2/admin/users",
                "get",
                false,
            ),
            (
                "iam/admin/users/v4/get",
                "/iam/v4/admin/users",
                "get",
                false,
            ),
        ]);
        let service = parse_spec("iam", &spec);
        let method = service
            .resources
            .iter()
            .find(|r| r.name == "users")
            .unwrap()
            .methods
            .iter()
            .find(|m| m.name == "get")
            .unwrap();

        let admin = method.scopes.iter().find(|s| s.scope == "admin").unwrap();
        let versions: Vec<_> = admin.contracts.iter().map(|c| c.api_version).collect();
        assert_eq!(versions, vec![ApiVersion(1), ApiVersion(2), ApiVersion(4)]);
        assert_eq!(admin.default_version, ApiVersion(4));
    }

    /// Deprecated versions should not contribute contracts or default-version
    /// selection.
    #[test]
    fn test_parser_still_skips_deprecated_contracts() {
        let spec = make_spec_with_operations(vec![
            ("iam/admin/users/v1/get", "/iam/v1/admin/users", "get", true), // deprecated
            (
                "iam/admin/users/v2/get",
                "/iam/v2/admin/users",
                "get",
                false,
            ),
        ]);
        let service = parse_spec("iam", &spec);
        let method = &service.resources[0].methods[0];
        let admin = method.scopes.iter().find(|s| s.scope == "admin").unwrap();
        let versions: Vec<_> = admin.contracts.iter().map(|c| c.api_version).collect();
        assert_eq!(versions, vec![ApiVersion(2)]);
    }

    /// When admin is absent and exactly one scope remains, that scope should
    /// become the method default.
    #[test]
    fn test_parser_default_scope_is_sole_scope_when_admin_absent() {
        let spec = make_spec_with_operations(vec![
            (
                "iam/public/users/v2/get",
                "/iam/v2/public/users",
                "get",
                false,
            ),
            (
                "iam/public/users/v3/get",
                "/iam/v3/public/users",
                "get",
                false,
            ),
        ]);
        let service = parse_spec("iam", &spec);
        let method = &service.resources[0].methods[0];
        assert_eq!(method.default_scope.as_deref(), Some("public"));
    }

    /// Ambiguous non-admin default-scope selection should assert in debug
    /// builds so spec issues are caught loudly during development.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "multiple non-admin scopes")]
    fn test_parser_multiple_non_admin_scopes_asserts_in_debug() {
        let spec = make_spec_with_operations(vec![
            (
                "iam/public/users/v1/get",
                "/iam/v1/public/users",
                "get",
                false,
            ),
            (
                "iam/server/users/v1/get",
                "/iam/v1/server/users",
                "get",
                false,
            ),
        ]);
        let _ = parse_spec("iam", &spec);
    }

    /// Ambiguous non-admin default-scope selection should degrade to `None` in
    /// release builds instead of panicking.
    #[test]
    #[cfg(not(debug_assertions))]
    fn test_parser_multiple_non_admin_scopes_default_scope_none_in_release() {
        let spec = make_spec_with_operations(vec![
            (
                "iam/public/users/v1/get",
                "/iam/v1/public/users",
                "get",
                false,
            ),
            (
                "iam/server/users/v1/get",
                "/iam/v1/server/users",
                "get",
                false,
            ),
        ]);
        let service = parse_spec("iam", &spec);
        let method = &service.resources[0].methods[0];
        assert_eq!(method.default_scope, None);
    }

    /// Parsed operations should retain the original `x-operationId` as their
    /// stable protocol identifier.
    #[test]
    fn test_stores_x_operation_id_as_id() {
        let spec = spec_from(&[("/api/v1/bans", "get", "svc/admin/bans/v1/list", false, "")]);
        let service = parse_spec("test", &spec);
        let ops = flatten_ops(&service.resources[0]);
        assert_eq!(ops[0].id, OperationId::new("svc/admin/bans/v1/list"));
    }

    /// HTTP verb strings from the spec should map onto the protocol enum.
    #[test]
    fn test_parses_http_method_to_enum() {
        let spec = spec_from(&[(
            "/api/v1/bans",
            "delete",
            "svc/admin/bans/v1/remove",
            false,
            "",
        )]);
        let service = parse_spec("test", &spec);
        let ops = flatten_ops(&service.resources[0]);
        assert_eq!(ops[0].http_method, HttpMethod::Delete);
    }

    /// DELETE operations should always classify as mutating.
    #[test]
    fn test_delete_mutation_class_is_mutating() {
        let spec = spec_from(&[(
            "/api/v1/bans",
            "delete",
            "svc/admin/bans/v1/remove",
            false,
            "",
        )]);
        let service = parse_spec("test", &spec);
        let ops = flatten_ops(&service.resources[0]);
        assert_eq!(ops[0].mutation_class, MutationClass::Mutating);
    }

    /// GET operations should always classify as read-only.
    #[test]
    fn test_classify_mutation_class_get_is_read_only() {
        assert_eq!(
            classify_mutation_class(HttpMethod::Get, "anything", &[]),
            MutationClass::ReadOnly
        );
    }

    /// Permission tier wins over the name heuristic: an [UPDATE] permission on
    /// a `get-*` POST must classify as mutating.
    #[test]
    fn test_classify_mutation_class_permissions_override_read_name() {
        let perms = vec!["ADMIN:NAMESPACE:{namespace}:USER [UPDATE]".to_string()];
        assert_eq!(
            classify_mutation_class(HttpMethod::Post, "get-justice-platform-account", &perms),
            MutationClass::Mutating
        );
    }

    /// Permission tier wins over HTTP verb for POST reads: a [READ] permission
    /// on a write-looking POST must classify as read-only.
    #[test]
    fn test_classify_mutation_class_permissions_override_write_name() {
        let perms = vec!["ADMIN:NAMESPACE:{namespace}:CHAT [READ]".to_string()];
        assert_eq!(
            classify_mutation_class(HttpMethod::Post, "filter-message", &perms),
            MutationClass::ReadOnly
        );
    }

    /// A mix of READ and non-READ actions is treated as mutating.
    #[test]
    fn test_classify_mutation_class_mixed_actions_are_mutating() {
        let perms = vec!["ADMIN:X [READ]".to_string(), "ADMIN:X [UPDATE]".to_string()];
        assert_eq!(
            classify_mutation_class(HttpMethod::Post, "bulk-update", &perms),
            MutationClass::Mutating
        );
    }

    /// Comma-separated actions inside a single bracket are each inspected.
    #[test]
    fn test_classify_mutation_class_comma_separated_actions() {
        let perms = vec!["ADMIN:X [READ, CREATE]".to_string()];
        assert_eq!(
            classify_mutation_class(HttpMethod::Post, "anything", &perms),
            MutationClass::Mutating
        );
    }

    /// Empty or bracketless permissions fall through to the name heuristic.
    #[test]
    fn test_classify_mutation_class_post_read_names_are_read_only() {
        for name in [
            "list",
            "list-by-namespace",
            "get",
            "get-by-user-id",
            "bulk-get",
            "bulk-get-by-ids",
            "search",
            "search-by-channel-id",
            "query",
            "query-by-attributes",
            "export",
            "export-by-csv",
            "find",
            "fetch",
            "fetch-items",
        ] {
            assert_eq!(
                classify_mutation_class(HttpMethod::Post, name, &[]),
                MutationClass::ReadOnly,
                "expected ReadOnly for POST {name}"
            );
        }
    }

    /// `check-*` is intentionally omitted from the read-prefix list: specs use
    /// it for both reads and writes, so it must fall back to mutating when no
    /// permission signal is available.
    #[test]
    fn test_classify_mutation_class_check_falls_back_to_mutating() {
        for name in ["check", "check-balance", "check-purchasable"] {
            assert_eq!(
                classify_mutation_class(HttpMethod::Post, name, &[]),
                MutationClass::Mutating,
                "expected Mutating for POST {name} without permissions"
            );
        }
    }

    /// POST names that look write-like should still classify as mutating when
    /// no permission signal says otherwise.
    #[test]
    fn test_classify_mutation_class_post_write_names_are_mutating() {
        for name in [
            "create", "update", "delete", "import", "submit", "enable", "disable", "link", "sync",
            "send",
        ] {
            assert_eq!(
                classify_mutation_class(HttpMethod::Post, name, &[]),
                MutationClass::Mutating,
                "expected Mutating for POST {name}"
            );
        }
    }

    /// Non-GET, non-POST verbs should always classify as mutating.
    #[test]
    fn test_classify_mutation_class_non_post_non_get_is_always_mutating() {
        for method in [HttpMethod::Put, HttpMethod::Patch, HttpMethod::Delete] {
            for name in ["list", "get", "search"] {
                assert_eq!(
                    classify_mutation_class(method, name, &[]),
                    MutationClass::Mutating,
                    "expected Mutating for {method:?} {name}"
                );
            }
        }
    }

    /// Permission strings without action brackets are treated as absent so the
    /// name heuristic still applies.
    #[test]
    fn test_classify_mutation_class_permission_without_brackets_falls_back() {
        let perms = vec!["legacyScopeName".to_string()];
        assert_eq!(
            classify_mutation_class(HttpMethod::Post, "bulk-get", &perms),
            MutationClass::ReadOnly
        );
    }

    /// Flattened operations should be sorted by method name for deterministic
    /// assertions and output.
    #[test]
    fn test_sorts_operations_by_method_name() {
        let spec = spec_from(&[
            (
                "/api/v1/bans",
                "delete",
                "svc/admin/bans/v1/remove",
                false,
                "",
            ),
            ("/api/v1/bans", "get", "svc/admin/bans/v1/list", false, ""),
            (
                "/api/v1/bans",
                "post",
                "svc/admin/bans/v1/create",
                false,
                "",
            ),
        ]);
        let service = parse_spec("test", &spec);
        let ops = flatten_ops(&service.resources[0]);
        let methods: Vec<&str> = ops
            .iter()
            .map(|operation| operation.name.as_str())
            .collect();
        assert_eq!(methods, vec!["create", "list", "remove"]);
    }

    /// The first OpenAPI `produces` entry should be surfaced as the response
    /// content-type hint.
    #[test]
    fn test_parses_produces_as_response_content_type() {
        let spec_json = r#"{
            "swagger": "2.0",
            "info": { "title": "t", "version": "1" },
            "paths": {
                "/foo": {
                    "get": {
                        "x-operationId": "svc/admin/foo/v1/get-qr",
                        "produces": ["image/png"],
                        "parameters": [],
                        "responses": { "200": { "description": "ok" } }
                    }
                }
            }
        }"#;
        let spec: crate::catalogue::openapi::SwaggerSpec = serde_json::from_str(spec_json).unwrap();
        let schema = parse_spec("svc", &spec);
        let op = &schema.resources[0].methods[0].scopes[0].contracts[0];
        assert_eq!(op.response_content_type.as_deref(), Some("image/png"));
    }

    /// Operations without `produces` metadata should leave the response
    /// content-type hint unset.
    #[test]
    fn test_missing_produces_leaves_response_content_type_none() {
        let spec_json = r#"{
            "swagger": "2.0",
            "info": { "title": "t", "version": "1" },
            "paths": {
                "/foo": {
                    "get": {
                        "x-operationId": "svc/admin/foo/v1/list",
                        "parameters": [],
                        "responses": { "200": { "description": "ok" } }
                    }
                }
            }
        }"#;
        let spec: crate::catalogue::openapi::SwaggerSpec = serde_json::from_str(spec_json).unwrap();
        let schema = parse_spec("svc", &spec);
        let op = &schema.resources[0].methods[0].scopes[0].contracts[0];
        assert!(op.response_content_type.is_none());
    }

    /// Unsupported HTTP verbs should trip the debug assertion path in
    /// development builds.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "unsupported HTTP method in spec")]
    fn test_parser_unknown_http_method_asserts_in_debug() {
        let _ = parse_http_method("HEAD");
    }

    /// Unsupported HTTP verbs should be skipped in release builds so the rest
    /// of the service can still load.
    #[test]
    #[cfg(not(debug_assertions))]
    fn test_parser_unknown_http_method_skips_operation_in_release() {
        let spec = spec_from(&[
            ("/api/v1/bans", "head", "svc/admin/bans/v1/probe", false, ""),
            ("/api/v1/bans", "get", "svc/admin/bans/v1/list", false, ""),
        ]);
        let service = parse_spec("test", &spec);
        let ops = flatten_ops(&service.resources[0]);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].name, "list");
    }

    /// Unsupported parameter locations should trip the debug assertion path in
    /// development builds.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "unsupported parameter location in spec")]
    fn test_parser_unknown_parameter_location_asserts_in_debug() {
        let spec: crate::catalogue::openapi::SwaggerSpec = serde_json::from_str(
            r#"{
            "swagger": "2.0",
            "info": {"title": "t", "version": "1"},
            "paths": {
                "/foo/{ns}": {
                    "get": {
                        "x-operationId": "svc/admin/foo/v1/get",
                        "parameters": [
                            {"name": "bar", "in": "cookie", "required": false, "type": "string"},
                            {"name": "ns", "in": "path", "required": true, "type": "string"}
                        ]
                    }
                }
            }
        }"#,
        )
        .unwrap();
        let _ = parse_spec("svc", &spec);
    }

    /// Unsupported parameter locations should be ignored in release builds.
    #[test]
    #[cfg(not(debug_assertions))]
    fn test_parser_unknown_parameter_location_skips_param_in_release() {
        let spec: crate::catalogue::openapi::SwaggerSpec = serde_json::from_str(
            r#"{
            "swagger": "2.0",
            "info": {"title": "t", "version": "1"},
            "paths": {
                "/foo/{ns}": {
                    "get": {
                        "x-operationId": "svc/admin/foo/v1/get",
                        "parameters": [
                            {"name": "bar", "in": "cookie", "required": false, "type": "string"},
                            {"name": "ns", "in": "path", "required": true, "type": "string"}
                        ]
                    }
                }
            }
        }"#,
        )
        .unwrap();
        let service = parse_spec("svc", &spec);
        let op = &service.resources[0].methods[0].scopes[0].contracts[0];
        assert_eq!(op.parameters.len(), 1);
        assert_eq!(op.parameters[0].name, "ns");
    }

    /// Unsupported parameter types should trip the debug assertion path in
    /// development builds.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "unsupported parameter type in spec")]
    fn test_parser_unknown_parameter_type_asserts_in_debug() {
        let spec: crate::catalogue::openapi::SwaggerSpec = serde_json::from_str(
            r#"{
            "swagger": "2.0",
            "info": {"title": "t", "version": "1"},
            "paths": {
                "/foo": {
                    "get": {
                        "x-operationId": "svc/admin/foo/v1/get",
                        "parameters": [
                            {"name": "foo", "in": "query", "required": false, "type": "object"}
                        ]
                    }
                }
            }
        }"#,
        )
        .unwrap();
        let _ = parse_spec("svc", &spec);
    }

    /// Unsupported parameter types should be skipped in release builds.
    #[test]
    #[cfg(not(debug_assertions))]
    fn test_parser_unknown_parameter_type_skips_param_in_release() {
        let spec: crate::catalogue::openapi::SwaggerSpec = serde_json::from_str(
            r#"{
            "swagger": "2.0",
            "info": {"title": "t", "version": "1"},
            "paths": {
                "/foo": {
                    "get": {
                        "x-operationId": "svc/admin/foo/v1/get",
                        "parameters": [
                            {"name": "foo", "in": "query", "required": false, "type": "object"}
                        ]
                    }
                }
            }
        }"#,
        )
        .unwrap();
        let service = parse_spec("svc", &spec);
        let op = &service.resources[0].methods[0].scopes[0].contracts[0];
        assert!(op.parameters.is_empty());
    }

    /// Supported OpenAPI parameter types should map to the expected protocol variants.
    #[test]
    fn test_value_type_from_parameter_maps_supported_shapes() {
        assert_eq!(
            value_type_from_parameter("string", None),
            Some(ValueType::String)
        );
        assert_eq!(
            value_type_from_parameter("integer", None),
            Some(ValueType::Integer)
        );
        assert_eq!(
            value_type_from_parameter("number", None),
            Some(ValueType::Number)
        );
        assert_eq!(
            value_type_from_parameter("boolean", None),
            Some(ValueType::Boolean)
        );
        assert_eq!(
            value_type_from_parameter("array", None),
            Some(ValueType::Array(Box::new(ValueType::String)))
        );
        assert_eq!(
            value_type_from_parameter("file", None),
            Some(ValueType::String)
        );
    }

    /// Enum-bearing OpenAPI parameter types should collapse into `ValueType::Enum`.
    #[test]
    fn test_value_type_from_parameter_prefers_enum_values() {
        let enum_values = vec!["asc".to_string(), "desc".to_string()];
        assert_eq!(
            value_type_from_parameter("string", Some(&enum_values)),
            Some(ValueType::Enum(enum_values))
        );
    }

    /// When multiple `produces` values exist, the first one should win because
    /// that is the fallback hint stored in the protocol schema.
    #[test]
    fn test_picks_first_when_produces_has_multiple_entries() {
        let spec_json = r#"{
            "swagger": "2.0",
            "info": { "title": "t", "version": "1" },
            "paths": {
                "/foo": {
                    "get": {
                        "x-operationId": "svc/admin/foo/v1/get",
                        "produces": ["application/json", "image/png"],
                        "parameters": [],
                        "responses": { "200": { "description": "ok" } }
                    }
                }
            }
        }"#;
        let spec: crate::catalogue::openapi::SwaggerSpec = serde_json::from_str(spec_json).unwrap();
        let schema = parse_spec("svc", &spec);
        let op = &schema.resources[0].methods[0].scopes[0].contracts[0];
        assert_eq!(
            op.response_content_type.as_deref(),
            Some("application/json")
        );
    }

    /// Duplicate non-deprecated contracts with the same `x-operationId` should
    /// assert in debug builds.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "duplicate x-operationId contract")]
    fn test_parser_duplicate_operation_id_asserts_in_debug() {
        let spec = spec_from(&[
            (
                "/api/v1/bans",
                "get",
                "svc/admin/bans/v1/list",
                false,
                "First",
            ),
            (
                "/api/v1/bans/all",
                "get",
                "svc/admin/bans/v1/list",
                false,
                "Second",
            ),
        ]);
        let _ = parse_spec("test", &spec);
    }

    /// Duplicate `x-operationId` entries should degrade to one surviving
    /// contract in release builds.
    #[test]
    #[cfg(not(debug_assertions))]
    fn test_parser_duplicate_operation_id_one_survives_in_release() {
        // Same x-operationId on two different paths: one survives. Because
        // spec.paths is a HashMap, iteration order is non-deterministic, so
        // the winner is arbitrary — only assert count and name, not summary.
        let spec = spec_from(&[
            (
                "/api/v1/bans",
                "get",
                "svc/admin/bans/v1/list",
                false,
                "First",
            ),
            (
                "/api/v1/bans/all",
                "get",
                "svc/admin/bans/v1/list",
                false,
                "Second",
            ),
        ]);
        let service = parse_spec("test", &spec);
        let ops = flatten_ops(&service.resources[0]);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].name, "list");
        assert!(["First", "Second"].contains(&ops[0].summary.as_str()));
    }

    /// All canonical OpenAPI verbs round-trip through `parse_http_method`.
    #[test]
    fn test_parse_http_method_round_trip() {
        for (input, expected) in [
            ("GET", HttpMethod::Get),
            ("POST", HttpMethod::Post),
            ("PUT", HttpMethod::Put),
            ("PATCH", HttpMethod::Patch),
            ("DELETE", HttpMethod::Delete),
        ] {
            assert_eq!(parse_http_method(input), Some(expected));
        }
    }

    /// Lower- and mixed-case verbs are parsed identically to upper-case.
    #[test]
    fn test_parse_http_method_is_case_insensitive() {
        assert_eq!(parse_http_method("get"), Some(HttpMethod::Get));
        assert_eq!(parse_http_method("PoSt"), Some(HttpMethod::Post));
    }

    /// Unknown verbs trigger the debug assertion to surface spec drift.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "unsupported HTTP method in spec")]
    fn test_parse_http_method_unknown_asserts_in_debug() {
        let _ = parse_http_method("HEAD");
    }

    /// In release builds, unknown verbs degrade to `None` rather than panicking.
    #[test]
    #[cfg(not(debug_assertions))]
    fn test_parse_http_method_unknown_returns_none_in_release() {
        assert_eq!(parse_http_method("HEAD"), None);
    }

    /// Every supported parameter location parses to its protocol enum variant.
    #[test]
    fn test_parse_parameter_location_round_trip() {
        for (input, expected) in [
            ("path", ParameterLocation::Path),
            ("query", ParameterLocation::Query),
            ("header", ParameterLocation::Header),
            ("body", ParameterLocation::Body),
            ("formData", ParameterLocation::FormData),
        ] {
            assert_eq!(parse_parameter_location(input), Some(expected));
        }
    }

    /// Unknown parameter locations trigger the debug assertion to surface spec drift.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "unsupported parameter location in spec")]
    fn test_parse_parameter_location_unknown_asserts_in_debug() {
        let _ = parse_parameter_location("cookie");
    }

    /// In release builds, unknown locations degrade to `None`.
    #[test]
    #[cfg(not(debug_assertions))]
    fn test_parse_parameter_location_unknown_returns_none_in_release() {
        assert_eq!(parse_parameter_location("cookie"), None);
    }

    /// Unknown parameter types trigger the debug assertion to surface spec drift.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "unsupported parameter type in spec")]
    fn test_value_type_from_parameter_unknown_asserts_in_debug() {
        let _ = value_type_from_parameter("object", None);
    }

    /// In release builds, unknown parameter types degrade to `None`.
    #[test]
    #[cfg(not(debug_assertions))]
    fn test_value_type_from_parameter_unknown_returns_none_in_release() {
        assert_eq!(value_type_from_parameter("object", None), None);
    }
}
