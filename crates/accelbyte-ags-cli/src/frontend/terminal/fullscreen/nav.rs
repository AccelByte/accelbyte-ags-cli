//! Contextual nav bar — bottom inner line of the fullscreen frame.
//!
//! Draws the keymap for the phase currently in the main area.
//! Key tokens are bracketed `[Key]` and styled Cyan+Bold; action text is White;
//! the `·` separator stays dim for visual grouping.

use ratatui::layout::Rect;
use ratatui::Frame;

use super::phases::Phase;

/// Render the contextual keymap for `phase` into a bordered `Navigation` box.
pub fn render(frame: &mut Frame, area: Rect, phase: &Phase) {
    crate::frontend::terminal::views::nav::render(frame, area, nav_context(phase));
}

/// Render a `Navigation` box showing only `Esc cancel` — used while a
/// dynamic-enum fetch is in flight, when cancelling is the only valid action.
pub fn render_loading(frame: &mut Frame, area: Rect) {
    crate::frontend::terminal::views::nav::render(
        frame,
        area,
        crate::frontend::terminal::views::nav::NavContext::Loading,
    );
}

/// Map the current phase (and its sub-mode) to the nav-bar key context.
fn nav_context(phase: &Phase) -> crate::frontend::terminal::views::nav::NavContext {
    use crate::frontend::terminal::views::nav::NavContext;
    match phase {
        Phase::Briefing(_) => NavContext::Briefing,
        // FieldsSkippable adds the `[s] skip` hint for optional-step review panels.
        Phase::Fields(panel) if panel.optional => NavContext::FieldsSkippable,
        Phase::Fields(_) => NavContext::Fields,
        // The failure gate reuses the confirm phase but its primary action is
        // Retry, so its nav hints must read "retry", not "confirm". Detect it by
        // the Retry action and route to the StepFailure hints (checked before the
        // confirm branches so a skippable failure gate is not mistaken for a
        // skippable confirm).
        Phase::Confirm(panel)
            if panel.card_phase.actions().contains(
                &crate::frontend::terminal::inline::phases::confirm_card::ConfirmAction::Retry,
            ) =>
        {
            if panel.card_phase.offers_skip() {
                NavContext::StepFailureSkippable
            } else {
                NavContext::StepFailure
            }
        }
        // ConfirmCard nav adds the `b` back-to-edit hint; ConfirmSkippable adds
        // `[s] skip` for optional steps; plain Confirm omits both.
        Phase::Confirm(panel) if panel.card_phase.offers_back() => NavContext::ConfirmCard,
        Phase::Confirm(panel) if panel.card_phase.offers_skip() => NavContext::ConfirmSkippable,
        Phase::Confirm(_) => NavContext::Confirm,
        Phase::JsonEdit(panel) => {
            if panel.scalar.is_some() {
                NavContext::JsonEditScalar
            } else if matches!(
                panel.mode,
                crate::frontend::terminal::inline::json_editor::EditorMode::Raw(_)
            ) {
                NavContext::JsonEditRaw
            } else {
                NavContext::JsonEditTree
            }
        }
        Phase::Running(_) => NavContext::Running,
        Phase::Result(_) | Phase::Error(_) => NavContext::Result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::terminal::fullscreen::phases::{running::RunningPanel, Phase};
    use ratatui::{backend::TestBackend, Terminal};

    /// Build a placeholder `Running` phase fixture.
    fn running_phase() -> Phase {
        Phase::Running(RunningPanel {
            step_number: 0,
            step_title: String::new(),
            description: String::new(),
            verb: "Starting".into(),
        })
    }

    /// Render the nav bar for `phase` and return it as text for assertions.
    fn nav_to_text(phase: &Phase) -> String {
        use ratatui::{backend::TestBackend, Terminal};
        let mut term = Terminal::new(TestBackend::new(80, 5)).unwrap();
        term.draw(|f| render(f, f.area(), phase)).unwrap();
        let buf = term.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Render the loading nav bar and return it as text for assertions.
    fn loading_to_text() -> String {
        use ratatui::{backend::TestBackend, Terminal};
        let mut term = Terminal::new(TestBackend::new(80, 5)).unwrap();
        term.draw(|f| render_loading(f, f.area())).unwrap();
        let buf = term.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_render_running_nav_does_not_panic() {
        let mut term = Terminal::new(TestBackend::new(80, 3)).unwrap();
        term.draw(|f| render(f, f.area(), &running_phase()))
            .unwrap();
    }

    #[test]
    fn test_nav_fields_phase_keys_styled_cyan_and_bracketed() {
        use crate::frontend::terminal::fullscreen::phases::fields::FieldsPanel;
        use crate::frontend::terminal::inline::form::Form;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let phase = Phase::Fields(FieldsPanel {
            step_number: 1,
            step_name: "x".into(),
            description: String::new(),
            form: Form::new("x", vec![]),
            optional: false,
        });
        let mut term = Terminal::new(TestBackend::new(80, 5)).unwrap();
        term.draw(|f| render(f, f.area(), &phase)).unwrap();
        let buf = term.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("[Tab] move"), "key bracketed: {content}");
        // Locate the 'T' of "[Tab]" by CELL index — `buf.content()` is
        // cell-indexed, so a byte offset into `content` (which has multi-byte
        // border/separator glyphs) would point at the wrong cell.
        let cells = buf.content();
        let t_cell = cells
            .windows(5)
            .position(|w| {
                w[0].symbol() == "["
                    && w[1].symbol() == "T"
                    && w[2].symbol() == "a"
                    && w[3].symbol() == "b"
                    && w[4].symbol() == "]"
            })
            .map(|i| i + 1)
            .expect("[Tab] in buffer");
        assert_eq!(
            cells[t_cell].fg,
            ratatui::style::Color::Cyan,
            "key 'T' is cyan"
        );
    }

    #[test]
    fn test_snapshot_nav_briefing() {
        use crate::frontend::terminal::fullscreen::phases::briefing::BriefingPanel;
        use ags_protocol::workflow::WorkflowBriefing;
        let phase = Phase::Briefing(BriefingPanel::new(
            &WorkflowBriefing {
                overview: "Test overview.".into(),
                prerequisites: vec![],
                creates: vec![],
            },
            "Test workflow",
        ));
        insta::with_settings!({snapshot_path => "../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(nav_to_text(&phase));
        });
    }

    #[test]
    fn test_snapshot_nav_fields() {
        use crate::frontend::terminal::fullscreen::phases::fields::FieldsPanel;
        use crate::frontend::terminal::inline::form::Form;
        let phase = Phase::Fields(FieldsPanel {
            step_number: 1,
            step_name: "x".into(),
            description: String::new(),
            form: Form::new("x", vec![]),
            optional: false,
        });
        insta::with_settings!({snapshot_path => "../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(nav_to_text(&phase));
        });
    }

    #[test]
    fn test_snapshot_nav_confirm() {
        use crate::frontend::terminal::fullscreen::phases::confirm::ConfirmPanel;
        use crate::frontend::terminal::inline::phases::confirm_card::{
            ConfirmCard, ConfirmCardPhase,
        };
        let phase = Phase::Confirm(ConfirmPanel {
            header: crate::frontend::terminal::fullscreen::header::Header::step(1, "Create", ""),
            card_phase: ConfirmCardPhase::new(ConfirmCard::new(
                "POST /iam/v3/x",
                vec![("namespace".into(), "test-ns".into())],
            )),
        });
        insta::with_settings!({snapshot_path => "../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(nav_to_text(&phase));
        });
    }

    #[test]
    fn test_snapshot_nav_running() {
        insta::with_settings!({snapshot_path => "../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(nav_to_text(&running_phase()));
        });
    }

    #[test]
    fn test_snapshot_nav_result() {
        use crate::frontend::terminal::fullscreen::phases::result::{ResultPanel, ResultStatus};
        let phase = Phase::Result(ResultPanel {
            title: "Complete".into(),
            description: String::new(),
            body: String::new(),
            completion: None,
            status: ResultStatus::Success,
        });
        insta::with_settings!({snapshot_path => "../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(nav_to_text(&phase));
        });
    }

    #[test]
    fn test_snapshot_nav_error() {
        use crate::frontend::terminal::fullscreen::phases::error::ErrorPanel;
        let phase = Phase::Error(ErrorPanel {
            message: "Something failed".into(),
            context: None,
            suggestion: None,
        });
        insta::with_settings!({snapshot_path => "../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(nav_to_text(&phase));
        });
    }

    #[test]
    fn test_snapshot_nav_json_edit_tree() {
        use crate::frontend::terminal::fullscreen::phases::json_edit::JsonEditPanel;
        use crate::frontend::terminal::inline::json_editor::{node::from_schema, EditorMode};
        let schema = serde_json::json!({"type": "object", "properties": {"a": {"type": "string"}}});
        let root = from_schema("body", &schema, &serde_json::Value::Null, true);
        let phase = Phase::JsonEdit(JsonEditPanel {
            title: "body".into(),
            root,
            focus: vec![],
            mode: EditorMode::Structured,
            scalar: None,
            baseline: serde_json::Value::Null,
            scroll_top: std::cell::Cell::new(0),
        });
        insta::with_settings!({snapshot_path => "../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(nav_to_text(&phase));
        });
    }

    #[test]
    fn test_snapshot_nav_json_edit_raw() {
        use crate::frontend::terminal::fullscreen::phases::json_edit::JsonEditPanel;
        use crate::frontend::terminal::inline::json_editor::{
            node::from_schema, raw::RawEditor, EditorMode,
        };
        let schema = serde_json::json!({"type": "object", "properties": {"a": {"type": "string"}}});
        let root = from_schema("body", &schema, &serde_json::Value::Null, true);
        let phase = Phase::JsonEdit(JsonEditPanel {
            title: "body".into(),
            root,
            focus: vec![],
            mode: EditorMode::Raw(RawEditor::from_seed("{}")),
            scalar: None,
            baseline: serde_json::Value::Null,
            scroll_top: std::cell::Cell::new(0),
        });
        insta::with_settings!({snapshot_path => "../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(nav_to_text(&phase));
        });
    }

    #[test]
    fn test_snapshot_nav_json_edit_scalar() {
        use crate::frontend::terminal::fullscreen::phases::json_edit::JsonEditPanel;
        use crate::frontend::terminal::inline::json_editor::{node::from_schema, EditorMode};
        let schema = serde_json::json!({"type": "string"});
        let root = from_schema("value", &schema, &serde_json::Value::Null, true);
        let phase = Phase::JsonEdit(JsonEditPanel {
            title: "value".into(),
            root,
            focus: vec![],
            mode: EditorMode::Structured,
            scalar: Some((vec![], "hello".into())),
            baseline: serde_json::Value::Null,
            scroll_top: std::cell::Cell::new(0),
        });
        insta::with_settings!({snapshot_path => "../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(nav_to_text(&phase));
        });
    }

    #[test]
    fn test_snapshot_nav_loading() {
        insta::with_settings!({snapshot_path => "../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(loading_to_text());
        });
    }

    #[test]
    fn test_snapshot_nav_confirm_skippable() {
        use crate::frontend::terminal::fullscreen::phases::confirm::ConfirmPanel;
        use crate::frontend::terminal::inline::phases::confirm_card::{
            ConfirmAction, ConfirmCard, ConfirmCardPhase,
        };
        // Optional-step confirm: Confirm / Skip / Cancel offered.
        let phase = Phase::Confirm(ConfirmPanel {
            header: crate::frontend::terminal::fullscreen::header::Header::step(
                1,
                "Optional step",
                "",
            ),
            card_phase: ConfirmCardPhase::new(ConfirmCard::new("Optional", vec![])).with_actions(
                &[
                    ConfirmAction::Confirm,
                    ConfirmAction::Skip,
                    ConfirmAction::Cancel,
                ],
            ),
        });
        insta::with_settings!({snapshot_path => "../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(nav_to_text(&phase));
        });
    }

    #[test]
    fn test_failure_gate_nav_context_reads_retry() {
        use crate::frontend::terminal::fullscreen::header::Header;
        use crate::frontend::terminal::fullscreen::phases::confirm::ConfirmPanel;
        use crate::frontend::terminal::inline::phases::confirm_card::{
            ConfirmAction, ConfirmCard, ConfirmCardPhase,
        };
        use crate::frontend::terminal::views::nav::NavContext;

        // Retry / Cancel → StepFailure (not Confirm).
        let gate = Phase::Confirm(ConfirmPanel {
            header: Header::step(1, "s", ""),
            card_phase: ConfirmCardPhase::new(ConfirmCard::new("Step failed", vec![]))
                .with_actions(&[ConfirmAction::Retry, ConfirmAction::Cancel]),
        });
        assert_eq!(nav_context(&gate), NavContext::StepFailure);

        // Retry / Skip / Cancel → StepFailureSkippable (not ConfirmSkippable).
        let gate_skip = Phase::Confirm(ConfirmPanel {
            header: Header::step(1, "s", ""),
            card_phase: ConfirmCardPhase::new(ConfirmCard::new("Step failed", vec![]))
                .with_actions(&[
                    ConfirmAction::Retry,
                    ConfirmAction::Skip,
                    ConfirmAction::Cancel,
                ]),
        });
        assert_eq!(nav_context(&gate_skip), NavContext::StepFailureSkippable);
    }

    #[test]
    fn test_snapshot_nav_fields_skippable() {
        use crate::frontend::terminal::fullscreen::phases::fields::FieldsPanel;
        use crate::frontend::terminal::inline::form::Form;
        // Optional-step review fields: optional flag shows the skip hint.
        let phase = Phase::Fields(FieldsPanel {
            step_number: 1,
            step_name: "optional-step".into(),
            description: String::new(),
            form: Form::new("optional-step", vec![]),
            optional: true,
        });
        insta::with_settings!({snapshot_path => "../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(nav_to_text(&phase));
        });
    }
}
