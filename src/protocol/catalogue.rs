//! Catalogue types — describe what commands exist and what they accept.

use serde::{Deserialize, Serialize};

/// Stable `x-operationId` from the OpenAPI spec — a kebab-or-Camel string
/// uniquely identifying one CLI-callable operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OperationId(String);

impl OperationId {
    /// Construct from any string-like value.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Return the underlying `&str`.
    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for OperationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for OperationId {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(OperationId(s.to_string()))
    }
}

impl From<String> for OperationId {
    fn from(s: String) -> Self {
        OperationId(s)
    }
}

impl From<&str> for OperationId {
    fn from(s: &str) -> Self {
        OperationId(s.to_string())
    }
}

/// Full service description: metadata plus every resource it exposes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceSchema {
    pub name: String,
    pub description: String,
    pub resources: Vec<ResourceSchema>,
}

/// One resource within a service (e.g. `ags iam users`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceSchema {
    pub name: String,
    pub description: String,
    pub methods: Vec<MethodSchema>,
}

impl ResourceSchema {
    /// Iterate every operation across all methods, scopes, and versions.
    pub fn operations(&self) -> impl Iterator<Item = &OperationSchema> {
        self.methods
            .iter()
            .flat_map(|method| method.scopes.iter())
            .flat_map(|scope_entry| scope_entry.contracts.iter())
    }
}

/// A CLI method (e.g. `iam users get`), grouping every non-deprecated contract
/// across scopes and versions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MethodSchema {
    pub name: String,
    pub summary: String,
    /// Default scope per spec §3.2: admin if present, else sole scope, else None.
    pub default_scope: Option<String>,
    pub scopes: Vec<ScopeEntry>,
}

impl MethodSchema {
    /// Return the default-contract operation (default scope's default version).
    pub fn default_operation(&self) -> Option<&OperationSchema> {
        let default_scope_name = self.default_scope.as_deref()?;
        let scope_entry = self.scopes.iter().find(|s| s.scope == default_scope_name)?;
        scope_entry
            .contracts
            .iter()
            .find(|c| c.api_version == scope_entry.default_version)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopeEntry {
    pub scope: String,
    /// Highest non-deprecated vN available in `contracts`.
    pub default_version: ApiVersion,
    pub contracts: Vec<OperationSchema>,
}

/// One CLI-callable operation, keyed on stable `x-operationId`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationSchema {
    /// Stable `x-operationId` from the OpenAPI spec.
    pub id: OperationId,
    /// CLI display name (kebab-case).
    pub name: String,
    pub summary: String,
    pub description: Option<String>,
    pub mutation_class: MutationClass,
    pub http_method: HttpMethod,
    pub path_template: String,
    pub parameters: Vec<ParameterSchema>,
    pub request_body: Option<BodySchema>,
    pub response: Option<ResponseSchema>,
    /// Permissions required for this operation, as declared in `x-security`.
    pub permissions: Vec<String>,
    /// Scope of this operation (`admin`, `public`, etc.).
    pub scope: String,
    /// API version extracted from the path template (e.g. `3` for `v3`).
    pub api_version: ApiVersion,
    /// Whether this operation is marked deprecated in the OpenAPI spec.
    pub deprecated: bool,
    /// First entry of the OpenAPI `produces` list, when present. Used as a
    /// fallback hint for response-body classification when the server omits
    /// or under-specifies `Content-Type`. `None` means the spec declares no
    /// response media type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_content_type: Option<String>,
}

/// API version number for an operation. Wraps a `u32`; `Display` emits
/// `"v3"` (with the `v` prefix), `FromStr` accepts `"v3"` or `"3"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApiVersion(pub u32);

impl ApiVersion {
    /// Construct from a raw integer.
    #[allow(dead_code)]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the underlying integer.
    #[allow(dead_code)]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}", self.0)
    }
}

impl std::str::FromStr for ApiVersion {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let stripped = s.strip_prefix('v').unwrap_or(s);
        stripped.parse::<u32>().map(ApiVersion)
    }
}

/// How safe an operation is to run unprompted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MutationClass {
    ReadOnly,
    Mutating,
    Diagnostic,
}

/// HTTP verb for an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
#[non_exhaustive]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

/// Single parameter of an operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterSchema {
    pub name: String,
    pub location: ParameterLocation,
    pub required: bool,
    pub value_type: ValueType,
    pub description: Option<String>,
}

/// Where a parameter lives in the HTTP request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ParameterLocation {
    Path,
    Query,
    Header,
    Body,
    /// OpenAPI 2.0 `formData` — multipart or URL-encoded form fields.
    FormData,
}

/// The shape of a parameter value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "of")]
#[non_exhaustive]
pub enum ValueType {
    String,
    Integer,
    Boolean,
    Number,
    Enum(Vec<String>),
    Array(Box<ValueType>),
}

/// The shape of a body-field value. Distinct from `ValueType` (used for
/// query/path/header parameters) because body fields can contain nested
/// object structures with their own children, plus references to named
/// schema definitions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "of")]
#[non_exhaustive]
pub enum BodyFieldType {
    String,
    Integer,
    Boolean,
    Number,
    /// String enum with allowed values.
    Enum(Vec<String>),
    /// Array of an inner type.
    Array(Box<BodyFieldType>),
    /// Object — the field's `children` carry its sub-fields.
    Object,
    /// Reference to a named definition not inlined here (e.g. `$ref` resolved
    /// to a short name). Used when depth limits prevent inlining.
    Reference(String),
}

/// Request body definition, with a name and typed fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BodySchema {
    pub definition_name: String,
    pub fields: Vec<BodyField>,
}

/// One field within a body schema, with optional nested children for nested objects/arrays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BodyField {
    pub name: String,
    pub field_type: BodyFieldType,
    pub required: bool,
    pub description: Option<String>,
    pub children: Vec<BodyField>,
}

/// Response shape for an operation. Currently only captures the declared
/// response kind and an optional description; fields are not modelled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseSchema {
    pub kind: Option<String>,
    pub description: Option<String>,
}

impl HttpMethod {
    /// Uppercase string form matching OpenAPI and HTTP conventions.
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Delete => "DELETE",
        }
    }
}

impl ParameterLocation {
    /// OpenAPI request-location string form for this parameter location.
    pub fn as_str(&self) -> &'static str {
        match self {
            ParameterLocation::Path => "path",
            ParameterLocation::Query => "query",
            ParameterLocation::Header => "header",
            ParameterLocation::Body => "body",
            ParameterLocation::FormData => "formData",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize a value to JSON and deserialize it again, asserting lossless
    /// protocol round-tripping.
    fn round_trip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let parsed: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, &parsed);
    }

    /// `ServiceSchema` should serialize and deserialize without losing data.
    #[test]
    fn test_service_schema_round_trip() {
        round_trip(&ServiceSchema {
            name: "iam".to_string(),
            description: "Identity and access management".to_string(),
            resources: vec![ResourceSchema {
                name: "users".to_string(),
                description: "Manage user accounts".to_string(),
                methods: vec![],
            }],
        });
    }

    /// `ResourceSchema` should serialize and deserialize without losing data.
    #[test]
    fn test_resource_schema_round_trip() {
        round_trip(&ResourceSchema {
            name: "users".to_string(),
            description: "Manage user accounts".to_string(),
            methods: vec![],
        });
    }

    /// A minimal `OperationSchema` should round-trip through JSON unchanged.
    #[test]
    fn test_operation_schema_round_trip_minimal() {
        round_trip(&OperationSchema {
            id: OperationId::new("AdminGetUserByUserIDV3"),
            name: "get-by-user-id".to_string(),
            summary: "Retrieve a user by user ID".to_string(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/iam/v3/admin/namespaces/{namespace}/users/{userId}".to_string(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: "admin".to_string(),
            api_version: ApiVersion(3),
            deprecated: false,
            response_content_type: None,
        });
    }

    /// A fully populated `OperationSchema` should round-trip through JSON
    /// unchanged.
    #[test]
    fn test_operation_schema_round_trip_full() {
        round_trip(&OperationSchema {
            id: OperationId::new("AdminCreateUserV4"),
            name: "create".to_string(),
            summary: "Creates a new user account".to_string(),
            description: Some("Full description with detail.".to_string()),
            mutation_class: MutationClass::Mutating,
            http_method: HttpMethod::Post,
            path_template: "/iam/v4/admin/namespaces/{namespace}/users".to_string(),
            parameters: vec![ParameterSchema {
                name: "namespace".to_string(),
                location: ParameterLocation::Path,
                required: true,
                value_type: ValueType::String,
                description: Some("Target namespace".to_string()),
            }],
            request_body: Some(BodySchema {
                definition_name: "ModelUserCreateRequestV4".to_string(),
                fields: vec![BodyField {
                    name: "emailAddress".to_string(),
                    field_type: BodyFieldType::String,
                    required: true,
                    description: Some("User email".to_string()),
                    children: vec![],
                }],
            }),
            response: Some(ResponseSchema {
                kind: Some("ModelUserResponseV4".to_string()),
                description: None,
            }),
            permissions: vec!["ADMIN:NAMESPACE:{namespace}:USER [CREATE]".to_string()],
            scope: "admin".to_string(),
            api_version: ApiVersion(4),
            deprecated: false,
            response_content_type: None,
        });
    }

    /// Every `MutationClass` variant should round-trip through JSON unchanged.
    #[test]
    fn test_mutation_class_all_variants_round_trip() {
        for value in [
            MutationClass::ReadOnly,
            MutationClass::Mutating,
            MutationClass::Diagnostic,
        ] {
            round_trip(&value);
        }
    }

    /// Every `HttpMethod` variant should round-trip through JSON unchanged.
    #[test]
    fn test_http_method_all_variants_round_trip() {
        for value in [
            HttpMethod::Get,
            HttpMethod::Post,
            HttpMethod::Put,
            HttpMethod::Patch,
            HttpMethod::Delete,
        ] {
            round_trip(&value);
        }
    }

    /// `ParameterSchema` should round-trip through JSON unchanged.
    #[test]
    fn test_parameter_schema_round_trip() {
        round_trip(&ParameterSchema {
            name: "limit".to_string(),
            location: ParameterLocation::Query,
            required: false,
            value_type: ValueType::Integer,
            description: Some("Page size".to_string()),
        });
    }

    /// Every `ParameterLocation` variant should round-trip through JSON
    /// unchanged.
    #[test]
    fn test_parameter_location_all_variants_round_trip() {
        for value in [
            ParameterLocation::Path,
            ParameterLocation::Query,
            ParameterLocation::Header,
            ParameterLocation::Body,
            ParameterLocation::FormData,
        ] {
            round_trip(&value);
        }
    }

    /// Scalar `ValueType` variants should round-trip through JSON unchanged.
    #[test]
    fn test_value_type_scalar_variants_round_trip() {
        for value in [
            ValueType::String,
            ValueType::Integer,
            ValueType::Boolean,
            ValueType::Number,
        ] {
            round_trip(&value);
        }
    }

    /// Enum `ValueType` values should round-trip through JSON unchanged.
    #[test]
    fn test_value_type_enum_round_trip() {
        round_trip(&ValueType::Enum(vec![
            "asc".to_string(),
            "desc".to_string(),
        ]));
    }

    /// Nested array `ValueType` values should round-trip through JSON
    /// unchanged.
    #[test]
    fn test_value_type_array_recursive_round_trip() {
        round_trip(&ValueType::Array(Box::new(ValueType::Array(Box::new(
            ValueType::String,
        )))));
    }

    /// `BodySchema` should round-trip through JSON unchanged.
    #[test]
    fn test_body_schema_round_trip() {
        round_trip(&BodySchema {
            definition_name: "ModelFoo".to_string(),
            fields: vec![],
        });
    }

    /// Nested `BodyField` trees should round-trip through JSON unchanged.
    #[test]
    fn test_body_field_with_children_round_trip() {
        round_trip(&BodyField {
            name: "address".to_string(),
            field_type: BodyFieldType::Object,
            required: false,
            description: None,
            children: vec![BodyField {
                name: "street".to_string(),
                field_type: BodyFieldType::String,
                required: true,
                description: None,
                children: vec![],
            }],
        });
    }

    /// Empty `ResponseSchema` values should round-trip through JSON unchanged.
    #[test]
    fn test_response_schema_empty_round_trip() {
        round_trip(&ResponseSchema {
            kind: None,
            description: None,
        });
    }

    /// `HttpMethod::as_str` should emit the canonical uppercase HTTP verb.
    #[test]
    fn test_http_method_as_str_matches_openapi_casing() {
        assert_eq!(HttpMethod::Get.as_str(), "GET");
        assert_eq!(HttpMethod::Post.as_str(), "POST");
        assert_eq!(HttpMethod::Put.as_str(), "PUT");
        assert_eq!(HttpMethod::Patch.as_str(), "PATCH");
        assert_eq!(HttpMethod::Delete.as_str(), "DELETE");
    }

    /// `ParameterLocation::as_str` should emit the canonical OpenAPI keys.
    #[test]
    fn test_parameter_location_as_str_matches_openapi_keys() {
        assert_eq!(ParameterLocation::Path.as_str(), "path");
        assert_eq!(ParameterLocation::Query.as_str(), "query");
        assert_eq!(ParameterLocation::Header.as_str(), "header");
        assert_eq!(ParameterLocation::Body.as_str(), "body");
        assert_eq!(ParameterLocation::FormData.as_str(), "formData");
    }

    /// `MethodSchema` should round-trip through JSON unchanged.
    #[test]
    fn test_method_schema_round_trip() {
        round_trip(&MethodSchema {
            name: "get".to_string(),
            summary: "Get a user".to_string(),
            default_scope: Some("admin".to_string()),
            scopes: vec![ScopeEntry {
                scope: "admin".to_string(),
                default_version: ApiVersion(4),
                contracts: vec![OperationSchema {
                    id: OperationId::new("iam/admin/users/v4/get"),
                    name: "get".to_string(),
                    summary: "Get a user".to_string(),
                    description: None,
                    mutation_class: MutationClass::ReadOnly,
                    http_method: HttpMethod::Get,
                    path_template: "/iam/v4/admin/namespaces/{namespace}/users/{userId}"
                        .to_string(),
                    parameters: vec![],
                    request_body: None,
                    response: None,
                    permissions: vec![],
                    scope: "admin".to_string(),
                    api_version: ApiVersion(4),
                    deprecated: false,
                    response_content_type: None,
                }],
            }],
        });
    }

    /// Operation scope and API version should be serialized as first-class
    /// protocol fields.
    #[test]
    fn test_operation_schema_carries_scope_and_version() {
        let op = OperationSchema {
            id: OperationId::new("iam/admin/users/v4/get"),
            name: "get".to_string(),
            summary: "Get a user".to_string(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/iam/v4/admin/namespaces/{namespace}/users/{userId}".to_string(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: "admin".to_string(),
            api_version: ApiVersion(4),
            deprecated: false,
            response_content_type: None,
        };
        round_trip(&op);
    }
}
