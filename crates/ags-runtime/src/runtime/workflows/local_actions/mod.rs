//! Closed registry of local actions available to `kind: local` workflow
//! steps. Each action is a named handler that runs a local subprocess or
//! performs a local-only operation — no API dispatch. The registry is
//! static and exhaustive: unknown action names are rejected at compile
//! time (`compile_workflow`).
//!
//! Actions are registered via [`lookup`] and enumerated via
//! [`known_names`]. Adding a new action means adding an entry to both.

use ags_protocol::error::RuntimeError;
use ags_protocol::event::ProgressSink;
use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::runtime::Runtime;

/// One input a local action accepts.
///
/// Deliberately smaller than `WorkflowInputSpec`: a local action declares
/// what it needs, and `compile_workflow` checks the step's bindings
/// against this rather than against an OpenAPI schema.
pub struct LocalActionInput {
    pub name: &'static str,
    pub required: bool,
    pub description: &'static str,
}

/// Trait implemented by every local action handler.
///
/// `?Send` because the progress sink is a plain `&mut dyn ProgressSink`
/// and the executor awaits steps directly rather than spawning them.
#[async_trait(?Send)]
pub trait LocalAction: Send + Sync {
    /// Inputs this action accepts, used by `compile_workflow` to validate
    /// the step's bindings and to reject unknown fields.
    fn inputs(&self) -> Vec<LocalActionInput>;

    /// Run the action and return the JSON value its outputs are bound from.
    ///
    /// `dry_run` must produce a representative value without performing
    /// any side effect, so `--dry-run` previews a workflow end to end.
    async fn run(
        &self,
        runtime: &Runtime,
        inputs: &Map<String, Value>,
        sink: &mut dyn ProgressSink,
        dry_run: bool,
    ) -> Result<Value, RuntimeError>;
}

/// Look up a local action by name. Returns `None` for unknown names.
pub fn lookup(name: &str) -> Option<&'static dyn LocalAction> {
    match name {
        "docker-login" => Some(&docker_login::DockerLoginAction),
        "ams/upload-image" => Some(&ams_upload::AmsUploadStep),
        #[cfg(test)]
        "test-echo" => Some(&test_support::TestEchoAction),
        #[cfg(test)]
        "test-fail" => Some(&test_support::TestFailAction),
        _ => None,
    }
}

/// Return the sorted list of known action names. Used in compile-time
/// error messages to enumerate valid choices.
pub fn known_names() -> Vec<&'static str> {
    let mut names = vec!["ams/upload-image", "docker-login"];
    #[cfg(test)]
    names.push("test-echo");
    #[cfg(test)]
    names.push("test-fail");
    names.sort();
    names
}

pub mod ams_upload;
pub mod docker_login;

/// Resolve a required string input, or fail with a message naming the step.
pub(crate) fn required_string(
    inputs: &Map<String, Value>,
    name: &str,
    step: &str,
) -> Result<String, RuntimeError> {
    inputs
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| RuntimeError {
            kind: ags_protocol::error::RuntimeErrorKind::Validation,
            message: format!(
                "local action '{step}' is missing required input '{name}'"
            ),
            details: None,
            hint: Some(format!(
                "Check that the workflow definition binds a value to '{name}' for local action '{step}'."
            )),
            trace: None,
        })
}

#[cfg(test)]
mod test_support {
    //! Test-only action used by compile and executor unit tests. Never
    //! shipped in release builds.

    use super::*;

    pub struct TestEchoAction;

    #[async_trait(?Send)]
    impl LocalAction for TestEchoAction {
        fn inputs(&self) -> Vec<LocalActionInput> {
            vec![]
        }

        async fn run(
            &self,
            _runtime: &Runtime,
            inputs: &Map<String, Value>,
            _sink: &mut dyn ProgressSink,
            _dry_run: bool,
        ) -> Result<Value, RuntimeError> {
            // Echo the inputs back as the action's output body so tests
            // can verify output-binding through the capture path.
            Ok(Value::Object(
                inputs.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            ))
        }
    }

    /// Test-only action that always fails. Used by executor tests to verify
    /// that a dry-run action failure is routed through step bookkeeping
    /// rather than escaping via bare `?`.
    pub struct TestFailAction;

    #[async_trait(?Send)]
    impl LocalAction for TestFailAction {
        fn inputs(&self) -> Vec<LocalActionInput> {
            vec![]
        }

        async fn run(
            &self,
            _runtime: &Runtime,
            _inputs: &Map<String, Value>,
            _sink: &mut dyn ProgressSink,
            _dry_run: bool,
        ) -> Result<Value, RuntimeError> {
            Err(RuntimeError {
                kind: ags_protocol::error::RuntimeErrorKind::Validation,
                message: "test-fail: intentional failure".into(),
                details: None,
                hint: None,
                trace: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_known_action_returns_some() {
        assert!(lookup("docker-login").is_some());
        assert!(lookup("ams/upload-image").is_some());
        assert!(lookup("test-echo").is_some());
    }

    #[test]
    fn lookup_unknown_action_returns_none() {
        assert!(lookup("nonexistent").is_none());
    }

    #[test]
    fn known_names_includes_registered_actions() {
        let names = known_names();
        assert!(names.contains(&"docker-login"));
        assert!(names.contains(&"ams/upload-image"));
        assert!(names.contains(&"test-echo"));
    }

    /// Every registered action id resolves to a handler. Prevents a
    /// name in known_names() that doesn't resolve in lookup().
    #[test]
    fn every_known_name_resolves() {
        for name in known_names() {
            assert!(
                lookup(name).is_some(),
                "'{name}' is in known_names() but does not resolve in lookup()"
            );
        }
    }

    #[test]
    fn test_required_string_rejects_missing_and_empty() {
        let mut inputs = Map::new();
        assert!(required_string(&inputs, "path", "s").is_err());
        inputs.insert("path".into(), Value::String(String::new()));
        assert!(required_string(&inputs, "path", "s").is_err());
        inputs.insert("path".into(), Value::String("./build".into()));
        assert_eq!(required_string(&inputs, "path", "s").unwrap(), "./build");
    }

    /// A non-string JSON value (number, bool, array, object) is not
    /// coercible to a string input — `required_string` must reject it
    /// the same way it rejects a missing key.
    #[test]
    fn test_required_string_rejects_non_string_value() {
        let mut inputs = Map::new();
        inputs.insert("count".into(), Value::Number(42.into()));
        let err = required_string(&inputs, "count", "test-action").unwrap_err();
        assert_eq!(
            err.kind,
            ags_protocol::error::RuntimeErrorKind::Validation,
            "a non-string value is a Validation error"
        );
        assert!(
            err.message.contains("count"),
            "message must name the input: {}",
            err.message
        );
        assert!(
            err.message.contains("test-action"),
            "message must name the action: {}",
            err.message
        );
    }

    /// A missing or empty required input is a workflow-authoring mistake,
    /// not an AGS bug — the error kind must be `Validation` (not
    /// `Internal`) and the message must name both the input and the
    /// action so the author can find the binding site.
    #[test]
    fn test_required_string_error_is_validation_and_names_input() {
        let inputs = Map::new();
        let err = required_string(&inputs, "path", "ams/upload-image").unwrap_err();
        assert_eq!(
            err.kind,
            ags_protocol::error::RuntimeErrorKind::Validation,
            "a missing input is a Validation error, not Internal"
        );
        assert!(
            err.message.contains("path"),
            "message must name the missing input: {}",
            err.message
        );
        assert!(
            err.message.contains("ams/upload-image"),
            "message must name the local action: {}",
            err.message
        );
        assert!(
            err.hint.is_some(),
            "a Validation error should carry a hint for the workflow author"
        );
    }
}
