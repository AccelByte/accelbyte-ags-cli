//! Phase-1 scope + version resolution for a command invocation.

use crate::protocol::catalogue::{ApiVersion, MethodSchema, OperationSchema};

/// A command contract selected by resolving explicit or default
/// `--api-scope`/`--api-version` against a method's supported matrix.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedContract {
    pub scope: String,
    pub api_version: ApiVersion,
    pub operation: OperationSchema,
}

/// Errors raised while resolving an `--api-scope`/`--api-version` to a concrete contract.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ResolutionError {
    /// User omitted `--api-scope` on a method with no default.
    #[error(
        "no default --api-scope for '{command}'\nSupported scopes: {}",
        supported.join(", ")
    )]
    MissingScope {
        command: String,
        supported: Vec<String>,
    },
    #[error(
        "api scope '{requested}' is not supported for '{command}'\nSupported scopes: {}",
        supported.join(", ")
    )]
    UnsupportedScope {
        command: String,
        requested: String,
        supported: Vec<String>,
    },
    #[error(
        "api version '{requested}' is not supported for '{command}' with --api-scope {scope}\nSupported {scope} versions: {}",
        supported.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ")
    )]
    UnsupportedVersion {
        command: String,
        scope: String,
        requested: String,
        supported: Vec<ApiVersion>,
    },
}

/// Resolve `--api-scope` / `--api-version` flags against a method's contracts to a single concrete operation.
pub fn resolve(
    command: &str,
    method: &MethodSchema,
    explicit_scope: Option<&str>,
    explicit_version: Option<&str>,
) -> Result<ResolvedContract, ResolutionError> {
    let supported_scopes: Vec<String> = method.scopes.iter().map(|s| s.scope.clone()).collect();

    let scope_name = match explicit_scope {
        Some(s) => s.to_string(),
        None => match &method.default_scope {
            Some(d) => d.clone(),
            None => {
                return Err(ResolutionError::MissingScope {
                    command: command.to_string(),
                    supported: supported_scopes,
                });
            }
        },
    };

    let scope_entry = method
        .scopes
        .iter()
        .find(|s| s.scope == scope_name)
        .ok_or_else(|| ResolutionError::UnsupportedScope {
            command: command.to_string(),
            requested: scope_name.clone(),
            supported: supported_scopes.clone(),
        })?;

    let unsupported_version = || ResolutionError::UnsupportedVersion {
        command: command.to_string(),
        scope: scope_name.clone(),
        requested: explicit_version
            .map(str::to_string)
            .unwrap_or_else(|| scope_entry.default_version.to_string()),
        supported: scope_entry
            .contracts
            .iter()
            .map(|c| c.api_version)
            .collect(),
    };

    let version_num = match explicit_version {
        Some(raw) => parse_version(raw).ok_or_else(unsupported_version)?,
        None => scope_entry.default_version,
    };

    let operation = scope_entry
        .contracts
        .iter()
        .find(|c| c.api_version == version_num)
        .cloned()
        .ok_or_else(unsupported_version)?;

    Ok(ResolvedContract {
        scope: scope_name,
        api_version: version_num,
        operation,
    })
}

/// Parse `v3` / `V3` / `3` into an `ApiVersion`, returning `None` for anything else.
fn parse_version(raw: &str) -> Option<ApiVersion> {
    let trimmed = raw
        .strip_prefix('v')
        .or_else(|| raw.strip_prefix('V'))
        .unwrap_or(raw);
    trimmed.parse::<u32>().ok().map(ApiVersion)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::catalogue::{
        HttpMethod, MethodSchema, MutationClass, OperationId, OperationSchema, ScopeEntry,
    };

    /// Build a minimal `OperationSchema` parameterised by scope, version, and deprecated flag for the resolve tests.
    fn op(scope: &str, version: u32, deprecated: bool) -> OperationSchema {
        OperationSchema {
            id: OperationId::new(format!("svc/{scope}/res/v{version}/get")),
            name: "get".to_string(),
            summary: "".to_string(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: format!("/svc/v{version}/{scope}/res"),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: scope.to_string(),
            api_version: ApiVersion(version),
            deprecated,
            response_content_type: None,
        }
    }

    /// Build a `MethodSchema` from a list of `(scope, versions)` pairs and an optional default scope.
    fn method_with(scopes: Vec<(&str, Vec<u32>)>, default_scope: Option<&str>) -> MethodSchema {
        let entries = scopes
            .into_iter()
            .map(|(scope, versions)| ScopeEntry {
                scope: scope.to_string(),
                default_version: ApiVersion(*versions.iter().max().unwrap()),
                contracts: versions.iter().map(|v| op(scope, *v, false)).collect(),
            })
            .collect();
        MethodSchema {
            name: "get".to_string(),
            summary: "".to_string(),
            default_scope: default_scope.map(str::to_string),
            scopes: entries,
        }
    }

    #[test]
    fn test_resolves_defaults_when_both_omitted() {
        let m = method_with(vec![("admin", vec![1, 2, 4])], Some("admin"));
        let r = resolve("svc res get", &m, None, None).unwrap();
        assert_eq!(r.scope, "admin");
        assert_eq!(r.api_version, ApiVersion(4));
    }

    #[test]
    fn test_resolves_explicit_scope_with_default_version() {
        let m = method_with(
            vec![("admin", vec![1, 2, 4]), ("public", vec![2, 3])],
            Some("admin"),
        );
        let r = resolve("svc res get", &m, Some("public"), None).unwrap();
        assert_eq!(r.scope, "public");
        assert_eq!(r.api_version, ApiVersion(3));
    }

    #[test]
    fn test_resolves_explicit_scope_and_version() {
        let m = method_with(
            vec![("admin", vec![1, 2, 4]), ("public", vec![2, 3])],
            Some("admin"),
        );
        let r = resolve("svc res get", &m, Some("public"), Some("v2")).unwrap();
        assert_eq!(r.scope, "public");
        assert_eq!(r.api_version, ApiVersion(2));
    }

    #[test]
    fn test_rejects_unsupported_scope() {
        let m = method_with(vec![("admin", vec![1])], Some("admin"));
        let err = resolve("svc res create", &m, Some("public"), None).unwrap_err();
        assert_eq!(
            err,
            ResolutionError::UnsupportedScope {
                command: "svc res create".to_string(),
                requested: "public".to_string(),
                supported: vec!["admin".to_string()],
            }
        );
    }

    #[test]
    fn test_rejects_unsupported_version() {
        let m = method_with(
            vec![("admin", vec![1]), ("public", vec![2, 3])],
            Some("admin"),
        );
        let err = resolve("svc res get", &m, Some("public"), Some("v1")).unwrap_err();
        assert_eq!(
            err,
            ResolutionError::UnsupportedVersion {
                command: "svc res get".to_string(),
                scope: "public".to_string(),
                requested: "v1".to_string(),
                supported: vec![ApiVersion(2), ApiVersion(3)],
            }
        );
    }

    #[test]
    fn test_missing_scope_errors_when_no_default() {
        let m = method_with(vec![("public", vec![1]), ("server", vec![1])], None);
        let err = resolve("svc res get", &m, None, None).unwrap_err();
        assert_eq!(
            err,
            ResolutionError::MissingScope {
                command: "svc res get".to_string(),
                supported: vec!["public".to_string(), "server".to_string()],
            }
        );
    }

    #[test]
    fn test_accepts_raw_numeric_version_string() {
        let m = method_with(vec![("admin", vec![2, 3])], Some("admin"));
        let r = resolve("svc res get", &m, None, Some("3")).unwrap();
        assert_eq!(r.api_version, ApiVersion(3));
    }
}
