//! OpenAPI 2.0 (Swagger) wire types used by the catalogue parser.
//!
//! CLI-facing schemas live in `crate::protocol::catalogue`; this module only
//! holds the intermediate shapes produced by `serde_json::from_str` on a raw
//! spec file.

use std::collections::HashMap;

use serde::Deserialize;

// ── OpenAPI 2.0 Serde types ──

/// Top-level OpenAPI 2.0 spec with paths, definitions, and metadata
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SwaggerSpec {
    pub swagger: String,
    #[serde(default)]
    pub info: SwaggerInfo,
    #[serde(default)]
    pub paths: HashMap<String, HashMap<String, SwaggerOperation>>,
    #[serde(default)]
    pub definitions: HashMap<String, SwaggerDefinition>,
}

/// Spec metadata: title and version string
#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct SwaggerInfo {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub version: String,
}

/// Single API operation within a path (GET /users, POST /users, etc.)
#[derive(Debug, Deserialize)]
pub struct SwaggerOperation {
    #[serde(default, rename = "deprecated")]
    pub is_deprecated: bool,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub parameters: Vec<SwaggerParameter>,
    #[serde(default, rename = "x-security")]
    pub x_security: serde_json::Value,
    #[serde(default, rename = "x-operationId")]
    pub x_operation_id: Option<String>,
    /// OpenAPI 2.0 `produces` list — content types this operation returns.
    #[serde(default)]
    pub produces: Vec<String>,
}

/// Parameter for an API operation (query, path, header, or body)
#[derive(Debug, Deserialize)]
pub struct SwaggerParameter {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "in")]
    pub location: String,
    #[serde(default, rename = "required")]
    pub is_required: bool,
    #[serde(default, rename = "type")]
    pub parameter_type: String,
    #[serde(default)]
    pub description: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub format: String,
    #[serde(rename = "enum", default)]
    pub enum_values: Option<Vec<String>>,
    #[serde(default)]
    pub schema: Option<SwaggerSchemaRef>,
}

/// Named model definition with typed properties and required-field list
#[derive(Debug, Deserialize)]
pub struct SwaggerDefinition {
    #[serde(default)]
    pub properties: HashMap<String, SwaggerProperty>,
    #[serde(default)]
    pub required: Vec<String>,
}

/// Single property within a definition (type, ref, array items, or enum)
#[derive(Debug, Deserialize)]
pub struct SwaggerProperty {
    #[serde(rename = "type", default)]
    pub property_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "$ref", default)]
    pub ref_path: Option<String>,
    #[serde(default)]
    pub items: Option<Box<SwaggerProperty>>,
    #[serde(rename = "enum", default)]
    pub enum_values: Option<Vec<String>>,
}

/// Schema reference (`$ref`) or inline type used by body parameters
#[derive(Debug, Deserialize)]
pub struct SwaggerSchemaRef {
    #[serde(rename = "$ref", default)]
    pub ref_path: Option<String>,
    #[allow(dead_code)]
    #[serde(rename = "type", default)]
    pub schema_type: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    pub items: Option<Box<SwaggerProperty>>,
}
