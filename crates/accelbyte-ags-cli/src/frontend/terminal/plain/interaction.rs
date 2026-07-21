//! `PlainInteraction` — workflow interaction for the human-readable frontend.

use crate::errors::CliError;
use crate::frontend::ExecutionInteraction;
use ags_protocol::workflow::{
    CompiledStep, GatherResult, StepPreview, SuppliedInputView, WorkflowInputNeeded,
    WorkflowInputSpec,
};
use std::collections::BTreeMap;

/// Zero-sized interaction handler for the human frontend.
///
/// Interaction is driven entirely through stdin/stderr prompts — no state
/// is needed beyond what the methods receive as arguments.
pub struct PlainInteraction;

impl ExecutionInteraction for PlainInteraction {
    fn gather_workflow_inputs(
        &mut self,
        needed: &[WorkflowInputNeeded],
        _step_context: &CompiledStep,
        _supplied: &[SuppliedInputView],
    ) -> Result<GatherResult, CliError> {
        // Dim header so the user knows up front how many values they'll be asked
        // for, rather than the prompts feeling open-ended.
        if let Some(header) = gather_header(needed.len()) {
            let color_enabled = crate::frontend::style::is_stderr_enabled();
            crate::frontend::write_stderr_line(&crate::frontend::style::apply_tone(
                &header,
                crate::frontend::style::Tone::Dim,
                color_enabled,
            ));
        }
        let mut slot_values = BTreeMap::new();
        let mut read = |label: &str, sensitive: bool| stdin_reader(label, sensitive);
        for entry in needed {
            let value = gather_one_slot(entry, &mut read)?;
            slot_values.insert(entry.id, value);
        }
        Ok(GatherResult {
            slot_values,
            input_overrides: BTreeMap::new(),
        })
    }

    fn confirm_step(
        &mut self,
        step: &CompiledStep,
        preview: &StepPreview,
    ) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
        crate::frontend::terminal::plain::prompt::confirm_step(preview, step.is_optional)
    }

    fn resolve_step_failure(
        &mut self,
        _step: &CompiledStep,
        error: &ags_protocol::error::RuntimeError,
        allow_skip: bool,
    ) -> Result<ags_protocol::workflow::StepFailureAction, CliError> {
        crate::frontend::terminal::plain::prompt::resolve_step_failure(error, allow_skip)
    }

    /// Phase 1: collect the declared workflow inputs up front (line-oriented),
    /// rather than gathering each step's inputs as it runs. When any input is
    /// missing, every declared input is prompted in execution-flow order —
    /// flag- and default-supplied values appear as the prompt default (accept
    /// with Enter or override), so plain presents the same inputs as the
    /// inline/fullscreen forms. A fully flag-/default-supplied run prompts for
    /// nothing (never blocks on stdin). Plain stays minimal — no briefing, no
    /// per-step review — so this is the single interactive point.
    fn collect_workflow_inputs(
        &mut self,
        specs: &[WorkflowInputSpec],
        current: &BTreeMap<String, serde_json::Value>,
    ) -> Result<Option<ags_protocol::workflow::CollectOutcome>, CliError> {
        let mut read = |label: &str, sensitive: bool| stdin_reader(label, sensitive);
        Ok(
            collect_inputs_impl(specs, current, &mut read)?.map(|inputs| {
                ags_protocol::workflow::CollectOutcome {
                    inputs,
                    run_mode: ags_protocol::workflow::RunMode::ReviewInputSteps,
                }
            }),
        )
    }
}

/// Phase-1 collection core, with the line reader injected so the prompting path
/// is testable without real stdin (the trait method passes [`stdin_reader`]).
fn collect_inputs_impl(
    specs: &[WorkflowInputSpec],
    current: &BTreeMap<String, serde_json::Value>,
    read: &mut dyn FnMut(&str, bool) -> Result<String, CliError>,
) -> Result<Option<BTreeMap<String, serde_json::Value>>, CliError> {
    use ags_runtime::support::strings::to_kebab_case;

    if specs.is_empty() {
        return Ok(Some(current.clone()));
    }
    // A fully flag-/default-supplied run has nothing to gather — return the
    // supplied map unchanged rather than re-prompting, so a fully flagged
    // plain run never blocks on stdin.
    if specs.iter().all(|s| current.contains_key(&s.name)) {
        return Ok(Some(current.clone()));
    }
    // Something is missing, so prompt for every declared input (not just the
    // missing ones) so plain shows the same set, in the same order, as the
    // inline/fullscreen forms. Already-supplied values are pre-filled below.
    if let Some(header) = gather_header(specs.len()) {
        let color_enabled = crate::frontend::style::is_stderr_enabled();
        crate::frontend::write_stderr_line(&crate::frontend::style::apply_tone(
            &header,
            crate::frontend::style::Tone::Dim,
            color_enabled,
        ));
    }
    // Start from the supplied values so they survive the executor's
    // replace-all-declared-inputs step.
    let mut out = current.clone();
    for spec in specs {
        let schema = spec
            .schema
            .clone()
            .unwrap_or_else(|| serde_json::json!({"type": "string"}));
        // Show the already-resolved value (flag or declared default) as the
        // prompt default so the user accepts with Enter or overrides — the
        // line-prompt equivalent of a pre-filled form field.
        let prefill = out.get(&spec.name).or(spec.default.as_ref()).cloned();
        let value = prompt_for_input(
            &humanize_label(&to_kebab_case(&spec.name)),
            &spec.name,
            spec.sensitive,
            prefill.as_ref(),
            spec.required,
            &schema,
            read,
        )?;
        // An empty optional input stays unset rather than sending null.
        if !value.is_null() {
            out.insert(spec.name.clone(), value);
        }
    }
    Ok(Some(out))
}

/// Prompt for a single workflow input slot, retrying up to 3 times on empty
/// required inputs.
///
/// Returns `Value::Null` for optional slots where the user submits an empty
/// value. A non-empty raw string is coerced via
/// [`crate::frontend::coerce_to_schema`] (the shared soft helper): on a type
/// mismatch (e.g. "abc" for an integer field) the raw string is preserved as
/// `Value::String` and passed through to the server for validation — no
/// client-side re-prompt is issued for type errors.
///
/// No description hint is shown (mirrors `ags auth login`); OpenAPI
/// descriptions live in `--help`.
fn gather_one_slot(
    entry: &WorkflowInputNeeded,
    read: &mut dyn FnMut(&str, bool) -> Result<String, CliError>,
) -> Result<serde_json::Value, CliError> {
    prompt_for_input(
        &humanize_label(&entry.label),
        &entry.label,
        entry.sensitive,
        entry.default.as_ref(),
        entry.required,
        &entry.schema,
        read,
    )
}

/// The production line reader: read a line from stdin, masking it when the input
/// is sensitive. The seam that `gather_one_slot` / `collect_inputs_impl` take so
/// tests can inject scripted answers instead of blocking on real stdin.
fn stdin_reader(label: &str, sensitive: bool) -> Result<String, CliError> {
    if sensitive {
        crate::frontend::terminal::plain::prompt::gather_secret(label)
    } else {
        crate::frontend::terminal::plain::prompt::gather_text(label)
    }
}

/// Prompt for a single value, retrying up to 3 times on an empty required input.
/// Shared by the per-step slot gather and the Phase-1 declared-input collection.
///
/// `display` is the title-cased label shown after `Enter `; `raw_name` is the
/// underlying input name used in error messages. Returns `Value::Null` for an
/// optional input left empty. A non-empty raw string is coerced via
/// [`crate::frontend::coerce_to_schema`] — a type mismatch is preserved as a
/// string and left for the server to validate (no client-side re-prompt).
fn prompt_for_input(
    display: &str,
    raw_name: &str,
    sensitive: bool,
    default: Option<&serde_json::Value>,
    required: bool,
    schema: &serde_json::Value,
    read: &mut dyn FnMut(&str, bool) -> Result<String, CliError>,
) -> Result<serde_json::Value, CliError> {
    let default_hint = default
        .map(|v| format!(" [default: {v}]"))
        .unwrap_or_default();
    // `Enter <Label>: ` mirrors `ags auth login` (inline input after the colon).
    let label = format!("Enter {display}{default_hint}: ");

    for attempt in 0..3u8 {
        let raw = read(&label, sensitive)?;

        if raw.is_empty() {
            if let Some(default) = default {
                return Ok(default.clone());
            }
            if !required {
                return Ok(serde_json::Value::Null);
            }
            if attempt < 2 {
                crate::frontend::write_stderr_line(
                    "  This field is required. Please enter a value.",
                );
                continue;
            } else {
                return Err(CliError::Usage {
                    message: format!(
                        "Required workflow input '{raw_name}' was not provided after 3 attempts.",
                    ),
                    metadata: None,
                });
            }
        }

        return Ok(crate::frontend::coerce_to_schema(&raw, schema));
    }

    // Unreachable: the loop either returns or errors within 3 iterations.
    Err(CliError::Usage {
        message: format!("Required workflow input '{raw_name}' was not provided."),
        metadata: None,
    })
}

/// Dim header announcing how many values will be gathered (`None` when there
/// are none). Pluralised: `Gathering 1 input` vs `Gathering 3 inputs`.
fn gather_header(count: usize) -> Option<String> {
    if count == 0 {
        return None;
    }
    let noun = if count == 1 { "input" } else { "inputs" };
    Some(format!("Gathering {count} {noun}"))
}

/// Turn a kebab/snake/dotted input label into a Title-Case display label for
/// the prompt (`client-name` -> `Client Name`), matching the conversational
/// `Enter Client ID:` style of `ags auth login`.
fn humanize_label(label: &str) -> String {
    label
        .split(['-', '_', '.'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{collect_inputs_impl, gather_header, humanize_label, PlainInteraction};
    use crate::frontend::ExecutionInteraction;
    use ags_protocol::catalogue::{OperationId, ServiceId};
    use ags_protocol::workflow::{CompiledStep, OperationReference};

    #[test]
    fn test_humanize_label_title_cases_kebab_and_snake() {
        assert_eq!(humanize_label("client-name"), "Client Name");
        assert_eq!(humanize_label("namespace"), "Namespace");
        assert_eq!(humanize_label("max_player_count"), "Max Player Count");
    }

    #[test]
    fn test_gather_header_singular_and_plural() {
        assert_eq!(gather_header(0), None);
        assert_eq!(gather_header(1).as_deref(), Some("Gathering 1 input"));
        assert_eq!(gather_header(2).as_deref(), Some("Gathering 2 inputs"));
    }

    // (Description-hint formatting moved to `prompt::emit_description_hint`,
    // which writes the dim hint line directly to stderr — the legacy
    // `format_description_part` helper that returned a parenthetical
    // fragment is gone with it.)

    // --- PlainInteraction construction ---

    /// Build a minimal `CompiledStep` for the interaction tests.
    fn make_compiled_step() -> CompiledStep {
        CompiledStep {
            id: "test-step".to_string(),
            index: 0,
            description: None,
            operation: OperationReference {
                service: ServiceId::new("iam"),
                operation: OperationId::new("testOp"),
            },
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![],
            auto_derived: vec![],
        }
    }

    /// `gather_workflow_inputs` with an empty `needed` slice returns an empty result.
    #[test]
    fn test_gather_workflow_inputs_empty_needed_returns_empty_map() {
        let mut interaction = PlainInteraction;
        let step = make_compiled_step();
        let result = interaction.gather_workflow_inputs(&[], &step, &[]).unwrap();
        assert!(result.slot_values.is_empty());
        assert!(result.input_overrides.is_empty());
    }

    fn spec(name: &str, required: bool) -> ags_protocol::workflow::WorkflowInputSpec {
        ags_protocol::workflow::WorkflowInputSpec {
            name: name.into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }
    }

    /// Phase-1 collection with no declared specs passes the current map through
    /// untouched without reading stdin.
    #[test]
    fn test_collect_workflow_inputs_empty_specs_passthrough() {
        let mut interaction = PlainInteraction;
        let current =
            std::collections::BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let result = interaction.collect_workflow_inputs(&[], &current).unwrap();
        assert_eq!(
            result,
            Some(ags_protocol::workflow::CollectOutcome {
                inputs: current,
                run_mode: ags_protocol::workflow::RunMode::ReviewInputSteps,
            })
        );
    }

    /// When every declared input is already supplied (flags/defaults), Phase-1
    /// prompts for nothing and returns the supplied map unchanged — so a fully
    /// flagged plain run never blocks on stdin.
    #[test]
    fn test_collect_workflow_inputs_all_supplied_does_not_prompt() {
        let mut interaction = PlainInteraction;
        let specs = vec![spec("namespace", true), spec("statCode", true)];
        let current = std::collections::BTreeMap::from([
            ("namespace".to_string(), serde_json::json!("dev")),
            ("statCode".to_string(), serde_json::json!("mmr")),
        ]);
        let result = interaction
            .collect_workflow_inputs(&specs, &current)
            .unwrap();
        assert_eq!(
            result,
            Some(ags_protocol::workflow::CollectOutcome {
                inputs: current,
                run_mode: ags_protocol::workflow::RunMode::ReviewInputSteps,
            })
        );
    }

    /// When any input is missing, Phase-1 prompts for EVERY declared input in
    /// order (not just the missing ones), pre-filling flag-/default-supplied
    /// values so an empty answer accepts them — matching the inline/fullscreen
    /// forms. Driven through the injected line reader so it never touches stdin.
    #[test]
    fn test_collect_inputs_prompts_all_in_order_with_prefilled_defaults() {
        use serde_json::json;
        // namespace: required, no default → typed.
        // stat-code:  optional with a default → empty answer accepts it.
        // fleet-region: required, no default → typed. It's absent from `current`,
        //   so the all-supplied shortcut does NOT fire and every input is prompted.
        let mut stat = spec("statCode", false);
        stat.default = Some(json!("mmr"));
        let specs = vec![spec("namespace", true), stat, spec("fleetRegion", true)];
        // The executor seeds declared defaults into `current` before this runs.
        let current = std::collections::BTreeMap::from([("statCode".to_string(), json!("mmr"))]);

        let mut prompted: Vec<String> = Vec::new();
        let result;
        {
            // Scripted answers in prompt order: type namespace, accept the
            // stat-code default (empty), type fleet-region.
            let mut answers =
                vec!["ns".to_string(), String::new(), "us-east-1".to_string()].into_iter();
            let mut read =
                |label: &str, _sensitive: bool| -> Result<String, crate::errors::CliError> {
                    prompted.push(label.to_string());
                    Ok(answers.next().expect("scripted answer available"))
                };
            result = collect_inputs_impl(&specs, &current, &mut read)
                .unwrap()
                .unwrap();
        }

        // Every input was prompted, in declared (first-use) order.
        assert_eq!(prompted.len(), 3, "prompted all inputs: {prompted:?}");
        assert!(prompted[0].contains("Namespace"), "first: {}", prompted[0]);
        assert!(prompted[1].contains("Stat Code"), "second: {}", prompted[1]);
        assert!(
            prompted[2].contains("Fleet Region"),
            "third: {}",
            prompted[2]
        );
        // The stat-code default is shown in its prompt (pre-filled).
        assert!(
            prompted[1].contains("mmr"),
            "default shown in prompt: {}",
            prompted[1]
        );

        // Typed values land; the accepted-empty stat-code keeps its default.
        assert_eq!(result.get("namespace"), Some(&json!("ns")));
        assert_eq!(result.get("statCode"), Some(&json!("mmr")));
        assert_eq!(result.get("fleetRegion"), Some(&json!("us-east-1")));
    }
}
