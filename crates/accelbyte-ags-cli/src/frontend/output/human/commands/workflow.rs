//! Human-readable rendering for completed workflow runs.

use crate::errors::CliError;
use crate::frontend::RenderedOutput;
use ags_protocol::output_views::{
    WorkflowOutputItem, WorkflowOutputProvenance, WorkflowOutputView,
};
use ags_protocol::workflow::{WorkflowId, WorkflowListEntry};
use std::collections::BTreeMap;

/// Render the final output of a completed workflow run.
///
/// Emits a short header plus any completion guidance (Created / Next steps) to
/// stderr, and the output data to stdout. `is_stdout_first` is `false`, so the
/// stderr guidance appears before the stdout data, separated by a blank line.
/// Per-step summaries are not repeated here — they stream live during the run.
///
/// When `output_view` is `Some`, stdout is built from the structured view:
/// items are grouped in declaration order under their `section` headings,
/// each line `<label>: <value>`.  When `None`, the flat `outputs` map is
/// serialised as JSON (legacy/plain-workflow behaviour).
pub(crate) fn render_workflow(
    _workflow_id: &WorkflowId,
    outputs: &BTreeMap<String, serde_json::Value>,
    _step_summaries: &[String],
    completion: &Option<ags_protocol::output_views::WorkflowCompletionView>,
    output_view: Option<&WorkflowOutputView>,
    _options: &crate::frontend::RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let color_enabled = crate::frontend::style::is_stderr_enabled();
    let mut stderr_lines = Vec::new();

    // The workflow id is omitted — the user knows which workflow they ran.
    stderr_lines.push(crate::frontend::style::success(
        "Workflow completed",
        color_enabled,
    ));

    // Post-run completion is guidance (chrome), so it goes on stderr —
    // stdout stays reserved for the output data below.
    if let Some(view) = completion {
        if !view.created.is_empty() {
            stderr_lines.push(String::new());
            stderr_lines.push("Created".to_string());
            for r in &view.created {
                // Colon, not an em dash (house style).
                stderr_lines.push(format!("  {}: {}", r.label, r.value));
            }
        }
        if !view.next_steps.is_empty() {
            stderr_lines.push(String::new());
            for s in &view.next_steps {
                // Shared suggestion convention: `→ Next: <description>` with the
                // command indented underneath. No "Next steps" header — each line
                // is self-labelling, matching error/warning suggestions.
                stderr_lines.push(format!(
                    "{} Next: {}",
                    crate::frontend::style::text::SYMBOL_FIX,
                    s.description
                ));
                stderr_lines.push(format!("    {}", s.command));
            }
        }
    }

    // Stdout: structured sectioned view when available, flat JSON otherwise.
    let stdout = if let Some(view) = output_view {
        let rendered = render_sectioned_output(view);
        if rendered.is_empty() {
            None
        } else {
            Some(rendered)
        }
    } else if outputs.is_empty() {
        None
    } else {
        let value =
            serde_json::to_value(outputs).map_err(|e| CliError::Internal(anyhow::anyhow!(e)))?;
        Some(
            crate::frontend::output::json::format_json(&value)
                .map_err(|e| CliError::Internal(anyhow::anyhow!(e)))?,
        )
    };

    // When there is a stdout data block, end the stderr chrome with a blank
    // line so a separator sits between the guidance and the data.
    if stdout.is_some() {
        stderr_lines.push(String::new());
    }
    let stderr = Some(stderr_lines.join("\n")).filter(|s| !s.is_empty());

    Ok(RenderedOutput {
        stdout,
        stderr,
        is_stdout_first: false,
    })
}

/// Build the sectioned stdout string from a `WorkflowOutputView`.
///
/// Items are emitted in their declared order, grouped under section headings.
/// A blank line separates each section.  Items without a section are emitted
/// without a heading.
fn render_sectioned_output(view: &WorkflowOutputView) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut current_section: Option<&str> = None;

    for item in &view.items {
        let section = item.section.as_deref();
        if section != current_section {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            if let Some(s) = section {
                lines.push(s.to_string());
            }
            current_section = section;
        }
        let label = item.label.as_deref().unwrap_or(&item.name);

        // Array value with declared item fields → render a sub-list (capped),
        // so the overview shows the actual records rather than just a count.
        if item.provenance != WorkflowOutputProvenance::Skipped {
            if let (Some(fields), serde_json::Value::Array(arr)) = (&item.item_fields, &item.value)
            {
                if !arr.is_empty() {
                    const CAP: usize = 10;
                    lines.push(format!("  {label} ({})", arr.len()));
                    for obj in arr.iter().take(CAP) {
                        lines.push(format!("    {}", render_item_line(obj, fields)));
                    }
                    if arr.len() > CAP {
                        lines.push(format!("    +{} more", arr.len() - CAP));
                    }
                    continue;
                }
            }
        }

        let value = render_output_value(item);
        lines.push(format!("  {label}: {value}"));
    }

    lines.join("\n")
}

/// Render one array item as a sub-list line from its declared fields: the first
/// resolved field is the label, the rest are detail joined after a colon
/// (e.g. `steam: 7656…` or `GOLD: 1500`). Absent/null fields are skipped.
fn render_item_line(obj: &serde_json::Value, fields: &[String]) -> String {
    let values: Vec<String> = fields
        .iter()
        .filter_map(|f| obj.get(f))
        .filter(|v| !v.is_null())
        .map(value_to_plain_string)
        .collect();
    match values.split_first() {
        Some((head, [])) => head.clone(),
        Some((head, rest)) => format!("{head}: {}", rest.join("  ")),
        None => "(no detail)".to_string(),
    }
}

/// Render a single `WorkflowOutputItem` value to its human-readable string.
///
/// Rules (in priority order):
/// - `Skipped` provenance → `"Unavailable"` (regardless of value)
/// - `null` → `"none"`
/// - empty array → `"none"`
/// - empty string → `"none"`
/// - non-empty array → `"{n} item(s)"`
/// - any other scalar (incl. `0` / `false`) → its plain string form
fn render_output_value(item: &WorkflowOutputItem) -> String {
    if item.provenance == WorkflowOutputProvenance::Skipped {
        return "Unavailable".to_string();
    }
    match &item.value {
        serde_json::Value::Null => "none".into(),
        serde_json::Value::Array(a) if a.is_empty() => "none".into(),
        serde_json::Value::String(s) if s.is_empty() => "none".into(),
        serde_json::Value::Array(a) => format!("{} item(s)", a.len()),
        other => value_to_plain_string(other),
    }
}

/// Convert a non-array, non-null JSON scalar (or object) to its plain string
/// representation without surrounding quotes.
fn value_to_plain_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        // Objects fall back to compact JSON.
        other => other.to_string(),
    }
}

/// Render the registered-workflow catalogue for `ags workflow list`.
/// One `id  name` line per workflow on stdout.
pub(crate) fn render_workflow_catalogue(
    entries: &[WorkflowListEntry],
    _options: &crate::frontend::RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let stdout = if entries.is_empty() {
        "No workflows registered".to_string()
    } else {
        // Align the id column so the names line up, matching the tabulated
        // look of `ags auth status`.
        let width = entries.iter().map(|e| e.id.len()).max().unwrap_or(0);
        entries
            .iter()
            .map(|entry| format!("{:<width$}  {}", entry.id, entry.name, width = width))
            .collect::<Vec<_>>()
            .join("\n")
    };
    Ok(RenderedOutput {
        stdout: Some(stdout),
        stderr: None,
        is_stdout_first: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::RenderOptions;
    use ags_protocol::output_views::{
        WorkflowOutputItem, WorkflowOutputProvenance, WorkflowOutputView,
    };

    /// Build a default `RenderOptions` shared by the workflow render tests.
    fn make_options() -> RenderOptions {
        RenderOptions::default()
    }

    /// Convenience constructor for test `WorkflowOutputItem` values.
    fn item(
        section: &str,
        label: &str,
        value: serde_json::Value,
        provenance: WorkflowOutputProvenance,
    ) -> WorkflowOutputItem {
        WorkflowOutputItem {
            name: label.to_lowercase().replace(' ', "_"),
            section: Some(section.to_string()),
            label: Some(label.to_string()),
            value,
            provenance,
            item_fields: None,
        }
    }

    #[test]
    fn test_render_workflow_catalogue_lists_entries() {
        let entries = vec![WorkflowListEntry {
            id: "competitive-multiplayer".into(),
            name: "Set up competitive multiplayer".into(),
        }];
        let rendered = render_workflow_catalogue(&entries, &make_options()).unwrap();
        let stdout = rendered.stdout.as_deref().unwrap_or("");
        assert!(stdout.contains("competitive-multiplayer"));
        assert!(stdout.contains("Set up competitive multiplayer"));
    }

    #[test]
    fn test_render_workflow_catalogue_aligns_id_column() {
        let entries = vec![
            WorkflowListEntry {
                id: "competitive-multiplayer".into(),
                name: "Alpha".into(),
            },
            WorkflowListEntry {
                id: "short".into(),
                name: "Beta".into(),
            },
        ];
        let rendered = render_workflow_catalogue(&entries, &make_options()).unwrap();
        let stdout = rendered.stdout.unwrap();
        let lines: Vec<&str> = stdout.lines().collect();
        // Names start at the same column — the shorter id is padded to the
        // longest id's width.
        assert_eq!(
            lines[0].find("Alpha"),
            lines[1].find("Beta"),
            "name column should align across rows"
        );
        assert!(
            lines[1].starts_with("short "),
            "shorter id should be space-padded: {:?}",
            lines[1]
        );
    }

    #[test]
    fn test_render_workflow_catalogue_empty() {
        let rendered = render_workflow_catalogue(&[], &make_options()).unwrap();
        assert_eq!(rendered.stdout.as_deref(), Some("No workflows registered"));
    }

    #[test]
    fn test_render_workflow_with_aliases() {
        let id = WorkflowId::new("competitive-multiplayer");
        let mut outputs = BTreeMap::new();
        outputs.insert("poolName".to_string(), serde_json::json!("ranked-1v1"));
        outputs.insert("statCode".to_string(), serde_json::json!("mmr"));
        let summaries = vec![
            "\u{2714} create-stat — OK".to_string(),
            "\u{2714} create-pool — OK".to_string(),
        ];
        let rendered =
            render_workflow(&id, &outputs, &summaries, &None, None, &make_options()).unwrap();

        // Stderr carries the header — no per-run id (the user knows the
        // workflow) and no per-step summaries (those stream live during the run).
        let stderr = rendered.stderr.as_deref().unwrap_or("");
        assert!(
            stderr.contains("Workflow completed"),
            "header present: {stderr}"
        );
        assert!(
            !stderr.contains("competitive-multiplayer"),
            "header omits the workflow id: {stderr}"
        );
        assert!(
            !stderr.contains("create-stat"),
            "per-step summaries are not repeated in the completion: {stderr}"
        );

        // Stdout carries the alias map.
        let stdout = rendered.stdout.as_deref().unwrap_or("");
        assert!(stdout.contains("poolName"), "missing alias in stdout");
        assert!(
            stdout.contains("ranked-1v1"),
            "missing alias value in stdout"
        );

        // A blank separator line ends the stderr block before the stdout data.
        assert!(
            stderr.ends_with('\n'),
            "stderr ends with a blank separator line: {stderr:?}"
        );

        // stderr-first ordering (unchanged).
        assert!(!rendered.is_stdout_first);
    }

    #[test]
    fn test_render_workflow_empty_outputs_omits_stdout() {
        let id = WorkflowId::new("no-outputs");
        let outputs = BTreeMap::new();
        let summaries = vec!["\u{2714} step-one — OK".to_string()];
        let rendered =
            render_workflow(&id, &outputs, &summaries, &None, None, &make_options()).unwrap();

        assert!(
            rendered.stdout.is_none(),
            "stdout should be absent when outputs map is empty"
        );
        assert!(
            rendered.stderr.is_some(),
            "stderr header must still be present"
        );
    }

    #[test]
    fn test_render_workflow_emits_completion_to_stderr() {
        let id = WorkflowId::new("competitive-multiplayer");
        let outputs = BTreeMap::new();
        let summaries = vec!["\u{2714} create-pool — OK".to_string()];
        let completion = Some(ags_protocol::output_views::WorkflowCompletionView {
            created: vec![ags_protocol::workflow::CompletionResource {
                label: "Match pool".into(),
                value: "ranked-pool".into(),
            }],
            next_steps: vec![ags_protocol::workflow::CompletionStep {
                description: "Inspect the match pool".into(),
                command: "ags matchmaking match-pools get --namespace dev --pool ranked-pool"
                    .into(),
            }],
        });
        let rendered = render_workflow(
            &id,
            &outputs,
            &summaries,
            &completion,
            None,
            &make_options(),
        )
        .unwrap();
        let stderr = rendered.stderr.as_deref().unwrap_or("");
        assert!(
            stderr.contains("Match pool"),
            "created label on stderr: {stderr}"
        );
        assert!(stderr.contains("ranked-pool"), "created value on stderr");
        assert!(
            stderr.contains("ags matchmaking match-pools get --namespace dev --pool ranked-pool"),
            "next-step command on stderr"
        );
        assert!(
            rendered.stdout.is_none(),
            "stdout must stay clean of guidance prose"
        );
    }

    #[test]
    fn test_render_sectioned_overview_groups_and_marks_unavailable() {
        use serde_json::json;
        use WorkflowOutputProvenance::{Captured, Skipped};

        let view = WorkflowOutputView {
            items: vec![
                item("Account", "Display name", json!("Ada"), Captured),
                item("Moderation", "Active bans", json!(0), Captured), // real 0
                item("Economy", "Entitlements", json!([]), Captured),  // empty → "none"
                item("Data", "Inventory", json!(null), Skipped),       // → Unavailable
            ],
        };
        let out = render_workflow(
            &WorkflowId::new("player-overview"),
            &BTreeMap::new(),
            &[],
            &None,
            Some(&view),
            &RenderOptions::default(),
        )
        .unwrap();
        let s = out.stdout.unwrap();
        assert!(s.contains("Account"));
        assert!(
            s.contains("Active bans: 0"),
            "0 renders as 0, not none/Unavailable: {s}"
        );
        assert!(
            s.contains("Entitlements: none"),
            "empty array renders as none: {s}"
        );
        assert!(
            s.contains("Inventory: Unavailable"),
            "Skipped renders as Unavailable: {s}"
        );
    }

    #[test]
    fn test_completion_uses_next_convention() {
        let id = WorkflowId::new("season-pass");
        let completion = Some(ags_protocol::output_views::WorkflowCompletionView {
            created: vec![ags_protocol::workflow::CompletionResource {
                label: "Season".into(),
                value: "Season 1".into(),
            }],
            next_steps: vec![ags_protocol::workflow::CompletionStep {
                description: "Publish the season to make it live".into(),
                command: "ags season-pass seasons publish --namespace ns --season-id abc".into(),
            }],
        });
        let rendered = render_workflow(
            &id,
            &BTreeMap::new(),
            &[],
            &completion,
            None,
            &make_options(),
        )
        .unwrap();
        let stderr = rendered.stderr.as_deref().unwrap_or("");
        // Created: colon, not em dash.
        assert!(
            stderr.contains("Season: Season 1"),
            "created uses colon: {stderr}"
        );
        assert!(
            !stderr.contains('\u{2014}'),
            "no em dash anywhere: {stderr}"
        );
        // Next steps in the → Next: convention, no "Next steps" header.
        assert!(
            !stderr.contains("Next steps"),
            "old header dropped: {stderr}"
        );
        assert!(
            stderr.contains("Next: Publish the season to make it live"),
            "next-step uses convention label: {stderr}"
        );
    }

    #[test]
    fn test_render_sectioned_overview_renders_item_sublists() {
        use serde_json::json;
        let mut platforms = item(
            "Account",
            "Linked platforms",
            json!([
                {"platformId": "steam", "platformUserId": "7656"},
                {"platformId": "psn", "platformUserId": "abc"},
            ]),
            WorkflowOutputProvenance::Captured,
        );
        platforms.item_fields = Some(vec!["platformId".into(), "platformUserId".into()]);
        let view = WorkflowOutputView {
            items: vec![platforms],
        };
        let out = render_workflow(
            &WorkflowId::new("player-overview"),
            &BTreeMap::new(),
            &[],
            &None,
            Some(&view),
            &RenderOptions::default(),
        )
        .unwrap();
        let s = out.stdout.unwrap();
        assert!(
            s.contains("Linked platforms (2)"),
            "header shows count: {s}"
        );
        assert!(
            s.contains("steam: 7656"),
            "item renders first field as label, rest as detail: {s}"
        );
        assert!(s.contains("psn: abc"), "second item rendered: {s}");
    }
}
