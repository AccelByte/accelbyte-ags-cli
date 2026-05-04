//! JSON envelope construction for the `ags describe` command.
//!
//! This module builds strongly-typed envelope structures from catalogue data
//! and serialises them into `serde_json::Value`s for the frontend to render.
//! It does not write to stdout directly — the handler hands the built value
//! to `Frontend::render` via `CommandOutput::Describe`.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::protocol::catalogue::{
    BodyField, BodyFieldType, BodySchema, MethodSchema, OperationSchema, ParameterSchema,
    ResponseSchema, ScopeEntry, ValueType,
};
use crate::support::strings::to_kebab_case;

// ── Envelope ──

#[derive(Serialize)]
pub struct DescribeEnvelope<T: Serialize> {
    pub schema_version: &'static str,
    pub kind: DescribeKind,
    pub path: Vec<String>,
    pub generated_by: GeneratorInfo,
    pub data: T,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DescribeKind {
    Catalogue,
    Command,
    Error,
}

#[derive(Serialize)]
pub struct GeneratorInfo {
    pub cli: &'static str,
    pub version: &'static str,
}

// ── Catalogue ──

#[derive(Serialize)]
pub struct CatalogueData {
    pub node_type: &'static str,
    pub name: String,
    pub summary: String,
    pub children: Vec<CatalogueChild>,
}

#[derive(Serialize)]
pub struct CatalogueChild {
    pub node_type: &'static str,
    pub name: String,
    pub path: Vec<String>,
    pub summary: String,
}

// ── Command (method matrix) ──

/// Data block for method-level describe: exposes the full scope/version
/// contract matrix. Deprecated contracts are excluded upstream in the
/// catalogue parser, so every entry here is callable.
#[derive(Serialize)]
pub struct MethodMatrixData {
    pub command: String,
    pub default_scope: Option<String>,
    pub scopes: BTreeMap<String, ScopeMatrix>,
}

#[derive(Serialize)]
pub struct ScopeMatrix {
    pub default_version: String,
    pub supported_versions: Vec<String>,
    pub contracts: BTreeMap<String, ContractDetail>,
}

#[derive(Serialize)]
pub struct ContractDetail {
    pub path_template: String,
    pub http_method: String,
    pub parameters: Vec<InputParameter>,
    pub request_body: Option<DescribeBodySchema>,
    pub response: Option<ContractResponse>,
    pub permissions: Vec<String>,
    pub x_operation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_content_type: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub response_is_binary: bool,
}

#[derive(Serialize)]
pub struct ContractResponse {
    pub kind: Option<String>,
    pub description: Option<String>,
}

#[derive(Serialize)]
pub struct DescribeBodySchema {
    pub definition_name: String,
    pub fields: Vec<DescribeBodyField>,
}

#[derive(Serialize)]
pub struct DescribeBodyField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub required: bool,
    pub description: String,
    pub enum_values: Option<Vec<String>>,
    pub children: Vec<DescribeBodyField>,
    pub is_array: bool,
}

#[derive(Serialize)]
pub struct InputParameter {
    pub name: String,
    pub location: String,
    pub required: bool,
    #[serde(rename = "type")]
    pub parameter_type: String,
    pub description: String,
    pub enum_values: Option<Vec<String>>,
}

// ── Error ──

#[derive(Serialize)]
pub struct DescribeErrorData {
    pub code: String,
    pub message: String,
    pub suggestions: Vec<String>,
}

// ── Constructors ──

/// Identify the CLI and version embedded in every describe envelope so consumers can detect drift.
pub(crate) fn generator_info() -> GeneratorInfo {
    GeneratorInfo {
        cli: "ags",
        version: env!("CARGO_PKG_VERSION"),
    }
}

/// Build the matrix view of a method: every non-deprecated contract across
/// every scope and version, keyed so consumers can pick a specific contract
/// without re-parsing the OpenAPI spec.
pub(crate) fn build_method_matrix(
    service: &str,
    resource: &str,
    method: &MethodSchema,
) -> MethodMatrixData {
    let mut scopes: BTreeMap<String, ScopeMatrix> = BTreeMap::new();
    for entry in &method.scopes {
        scopes.insert(entry.scope.clone(), scope_matrix_from_entry(entry));
    }
    MethodMatrixData {
        command: format!("{service} {resource} {}", method.name),
        default_scope: method.default_scope.clone(),
        scopes,
    }
}

/// Build the scope matrix payload — versions, default, and per-version contract details — for one scope.
fn scope_matrix_from_entry(entry: &ScopeEntry) -> ScopeMatrix {
    let mut contracts: BTreeMap<String, ContractDetail> = BTreeMap::new();
    // Contracts arrive sorted by `api_version` and deduplicated by the
    // catalogue parser, so no additional sort/dedup is needed here.
    let versions: Vec<crate::protocol::catalogue::ApiVersion> =
        entry.contracts.iter().map(|c| c.api_version).collect();

    for operation in &entry.contracts {
        let key = operation.api_version.to_string();
        contracts.insert(key, contract_detail_from_operation(operation));
    }

    ScopeMatrix {
        default_version: entry.default_version.to_string(),
        supported_versions: versions.into_iter().map(|v| v.to_string()).collect(),
        contracts,
    }
}

/// Build a single contract detail (path, parameters, body, response, permissions) from an operation schema.
fn contract_detail_from_operation(operation: &OperationSchema) -> ContractDetail {
    ContractDetail {
        path_template: operation.path_template.clone(),
        http_method: operation.http_method.as_str().to_string(),
        parameters: operation
            .parameters
            .iter()
            .map(InputParameter::from_schema)
            .collect(),
        request_body: operation
            .request_body
            .as_ref()
            .map(DescribeBodySchema::from_schema),
        response: operation
            .response
            .as_ref()
            .map(ContractResponse::from_schema),
        permissions: operation.permissions.clone(),
        x_operation_id: operation.id.to_string(),
        response_content_type: operation.response_content_type.clone(),
        response_is_binary: operation
            .response_content_type
            .as_deref()
            .map(|content_type| !is_text_content_type(content_type))
            .unwrap_or(false),
    }
}

impl ContractResponse {
    /// Project a runtime `ResponseSchema` into the describe envelope's response shape.
    fn from_schema(schema: &ResponseSchema) -> Self {
        Self {
            kind: schema.kind.clone(),
            description: schema.description.clone(),
        }
    }
}

impl InputParameter {
    /// Project a runtime `ParameterSchema` into the describe envelope's input-parameter shape, kebab-casing the flag name.
    fn from_schema(parameter: &ParameterSchema) -> Self {
        let (parameter_type, enum_values) = match &parameter.value_type {
            ValueType::String => ("string", None),
            ValueType::Integer => ("integer", None),
            ValueType::Number => ("number", None),
            ValueType::Boolean => ("boolean", None),
            ValueType::Enum(values) => ("string", Some(values.clone())),
            ValueType::Array(_) => ("array", None),
        };
        // Match the transform applied when building Clap flags in
        // invocation::builder so consumers can feed the emitted name
        // straight back as `--<name>`.
        Self {
            name: to_kebab_case(&parameter.name),
            location: parameter.location.as_str().to_string(),
            required: parameter.required,
            parameter_type: parameter_type.to_string(),
            description: parameter.description.clone().unwrap_or_default(),
            enum_values,
        }
    }
}

impl DescribeBodySchema {
    /// Project a runtime `BodySchema` into the describe envelope's body shape, recursing into fields.
    fn from_schema(schema: &BodySchema) -> Self {
        Self {
            definition_name: schema.definition_name.clone(),
            fields: schema
                .fields
                .iter()
                .map(DescribeBodyField::from_field)
                .collect(),
        }
    }
}

impl DescribeBodyField {
    /// Project a runtime `BodyField` into the describe envelope's nested-field shape.
    fn from_field(field: &BodyField) -> Self {
        let (field_type_str, enum_values, is_array) = describe_field_type(&field.field_type);
        Self {
            name: field.name.clone(),
            field_type: field_type_str,
            required: field.required,
            description: field.description.clone().unwrap_or_default(),
            enum_values,
            children: field
                .children
                .iter()
                .map(DescribeBodyField::from_field)
                .collect(),
            is_array,
        }
    }
}

/// Derive the legacy describe-envelope field shape from a `BodyFieldType`.
///
/// Returns `(type_string, enum_values, is_array)` matching the wire format
/// that `ags describe` consumers expect.
fn describe_field_type(ft: &BodyFieldType) -> (String, Option<Vec<String>>, bool) {
    match ft {
        BodyFieldType::String => ("string".to_string(), None, false),
        BodyFieldType::Integer => ("integer".to_string(), None, false),
        BodyFieldType::Boolean => ("boolean".to_string(), None, false),
        BodyFieldType::Number => ("number".to_string(), None, false),
        BodyFieldType::Object => ("object".to_string(), None, false),
        BodyFieldType::Reference(name) => (format!("object ({})", name), None, false),
        BodyFieldType::Enum(values) => ("string".to_string(), Some(values.clone()), false),
        BodyFieldType::Array(inner) => {
            let inner_str = match inner.as_ref() {
                BodyFieldType::Reference(name) => format!("array[{}]", name),
                BodyFieldType::String => "array[string]".to_string(),
                BodyFieldType::Integer => "array[integer]".to_string(),
                BodyFieldType::Boolean => "array[boolean]".to_string(),
                BodyFieldType::Number => "array[number]".to_string(),
                _ => "array[object]".to_string(),
            };
            (inner_str, None, true)
        }
    }
}

// ── Helpers ──

/// Find names that contain the query as a substring, for typo suggestions
pub(crate) fn find_suggestions(query: &str, candidates: &[&str]) -> Vec<String> {
    let query_lower = query.to_lowercase();
    candidates
        .iter()
        .filter(|c| {
            let c_lower = c.to_lowercase();
            c_lower.contains(&query_lower) || query_lower.contains(&c_lower)
        })
        .map(|c| c.to_string())
        .collect()
}

/// Serialise a describe envelope into a `serde_json::Value` for the frontend.
pub(crate) fn to_value<T: Serialize>(envelope: &DescribeEnvelope<T>) -> serde_json::Value {
    // DescribeEnvelope and its payload types are all `Serialize` with no
    // fallible variants, so this cannot fail in practice. Panic loudly if it ever does
    // rather than emitting a silent `null` envelope.
    serde_json::to_value(envelope).expect("describe envelope serialization is infallible")
}

/// Return true for content types the CLI can render as text without treating
/// the response as a binary download.
fn is_text_content_type(content_type: &str) -> bool {
    let content_type = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();

    content_type.starts_with("text/")
        || matches!(
            content_type.as_str(),
            "application/json"
                | "application/problem+json"
                | "application/xml"
                | "application/yaml"
                | "application/x-yaml"
                | "application/javascript"
                | "application/x-www-form-urlencoded"
        )
        || content_type.ends_with("+json")
        || content_type.ends_with("+xml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::catalogue::{
        ApiVersion, HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema,
        ScopeEntry,
    };

    /// Build a minimal `OperationSchema` parameterised by scope and version for the matrix tests.
    fn op(scope: &str, version: u32) -> OperationSchema {
        OperationSchema {
            id: OperationId::new(format!("svc/{scope}/res/v{version}/get")),
            name: "get".to_string(),
            summary: "Get".to_string(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: format!("/svc/v{version}/{scope}/thing"),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: scope.to_string(),
            api_version: ApiVersion(version),
            deprecated: false,
            response_content_type: None,
        }
    }

    #[test]
    fn test_method_matrix_groups_scopes_and_versions() {
        let method = MethodSchema {
            name: "get".to_string(),
            summary: "Get".to_string(),
            default_scope: Some("admin".to_string()),
            scopes: vec![
                ScopeEntry {
                    scope: "admin".to_string(),
                    default_version: ApiVersion(4),
                    contracts: vec![op("admin", 1), op("admin", 4)],
                },
                ScopeEntry {
                    scope: "public".to_string(),
                    default_version: ApiVersion(3),
                    contracts: vec![op("public", 2), op("public", 3)],
                },
            ],
        };
        let data = build_method_matrix("svc", "res", &method);
        assert_eq!(data.command, "svc res get");
        assert_eq!(data.default_scope.as_deref(), Some("admin"));
        assert_eq!(data.scopes.len(), 2);
        let admin = data.scopes.get("admin").unwrap();
        assert_eq!(admin.default_version, "v4");
        assert_eq!(admin.supported_versions, vec!["v1", "v4"]);
        assert!(admin.contracts.contains_key("v1"));
        assert!(admin.contracts.contains_key("v4"));
        let public = data.scopes.get("public").unwrap();
        assert_eq!(public.default_version, "v3");
        assert_eq!(public.supported_versions, vec!["v2", "v3"]);
    }

    #[test]
    fn test_contract_detail_carries_x_operation_id() {
        let entry = ScopeEntry {
            scope: "admin".to_string(),
            default_version: ApiVersion(4),
            contracts: vec![op("admin", 4)],
        };
        let matrix = scope_matrix_from_entry(&entry);
        let c = matrix.contracts.get("v4").unwrap();
        assert_eq!(c.x_operation_id, "svc/admin/res/v4/get");
        assert_eq!(c.http_method, "GET");
    }
}
