//! Fields phase: the gather form embedded in the main region.
//!
//! Two stacked bordered panels: a `Step N` box (the step name + description)
//! above a `Parameters` box. The Parameters box lays fields out row-by-row so
//! the focused field's hint renders in a bordered box *below the Submit row*, and the
//! `[ Confirm ]` button lives inline as the last row of the scroll
//! plan (immediately after the last field). The shared [`Form`] supplies the
//! per-row and button renders; the inline surface keeps its own monolithic layout,
//! unchanged.

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::frontend::terminal::fullscreen::header;
use crate::frontend::terminal::inline::form::Form;

pub struct FieldsPanel {
    /// 1-based step number for the `Step N` box title.
    pub step_number: usize,
    /// Step name shown bold at the top of the Step box.
    pub step_name: String,
    /// Optional longer description under the step name.
    pub description: String,
    pub form: Form,
    /// When `true` the nav bar shows `[s] skip` — set by `review_step` when
    /// `plan.optional` is true so the user can skip the step without editing.
    pub optional: bool,
}

impl FieldsPanel {
    /// Draw the step box and field form into `area`, including the Submit row
    /// and the focused field's hint box. The step header fills the shared
    /// fixed-height slot so the fields box below stays put across phases.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let content = header::render_slot(frame, area, &self.header_model());
        self.render_fields_box(frame, content);
    }

    /// Build the step-box header model (title, step name, optional description).
    fn header_model(&self) -> header::Header {
        header::Header::step(self.step_number, &self.step_name, &self.description)
    }

    /// Draw the bordered field-rows box into `area`, including the Submit row
    /// and the focused field's hint box.
    fn render_fields_box(&self, frame: &mut Frame, area: Rect) {
        // step_number == 0 → gather phase, where Inputs are being collected
        // (editable). Otherwise → step review, where Inputs flowed from gather
        // and are read-only at the step. The header text reflects this; both
        // headers stay Cyan (same brand colour as keys and the step name).
        let inputs_title = if self.step_number == 0 {
            "Inputs"
        } else {
            "Inputs (read-only)"
        };
        crate::frontend::terminal::views::fields::render(
            frame,
            area,
            inputs_title,
            &self.form,
            true,
        );
    }

    /// Mutable access to the form for the interaction's key loop.
    pub fn form_mut(&mut self) -> &mut Form {
        &mut self.form
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::terminal::inline::form::{
        FieldKey, FieldSource, FieldType, FieldValue, FormField,
    };
    use ags_protocol::workflow::GatherSlotId;
    use ratatui::style::Color;
    use ratatui::{backend::TestBackend, Terminal};

    /// Visual row (0-based) of the first cell-row whose rendered text contains
    /// `label`. Scans cells so the result is a true terminal row, immune to the
    /// multi-byte border/caret glyphs that make byte offsets misleading.
    fn label_row(buf: &ratatui::buffer::Buffer, label: &str) -> u16 {
        for y in 0..buf.area.height {
            let mut row = String::new();
            for x in 0..buf.area.width {
                row.push_str(buf[(x, y)].symbol());
            }
            if row.contains(label) {
                return y;
            }
        }
        panic!("{label} not rendered");
    }

    /// Build a sample `FieldsPanel` fixture.
    fn panel() -> FieldsPanel {
        let field = FormField {
            label: "user-id".into(),
            field_type: FieldType::Scalar,
            required: true,
            value: FieldValue::Empty,
            description: "The player to target".into(),
            source: FieldSource::UserInput,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type": "string"}),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        FieldsPanel {
            step_number: 1,
            step_name: "Define".into(),
            description: "Provide inputs".into(),
            form: Form::new("Provide inputs", vec![field]).with_submit_focusable(true),
            optional: false,
        }
    }

    #[test]
    fn test_render_into_test_backend_does_not_panic() {
        let p = panel();
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| p.render(f, f.area())).unwrap();
    }

    #[test]
    fn test_render_shows_submit_and_hint() {
        let p = panel();
        // Tall enough that the Submit row and hint box are both visible.
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| p.render(f, f.area())).unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(buf.contains("Confirm"), "submit row shown: {buf}");
        assert!(
            buf.contains("The player to target"),
            "focused field hint shown: {buf}"
        );
        assert!(buf.contains("user-id"), "fields shown: {buf}");
    }

    #[test]
    fn test_render_submit_row_visible_after_last_field() {
        use ags_protocol::workflow::StepFieldId;
        // More fields than fit: the button must still be visible (scroll-to-focus
        // includes it) and the focused field must scroll into view.
        let fields: Vec<FormField> = (0..20u32)
            .map(|i| FormField {
                label: format!("field-{i}"),
                field_type: FieldType::Scalar,
                required: false,
                value: FieldValue::Scalar(format!("v{i}")),
                description: String::new(),
                source: FieldSource::Literal,
                key: FieldKey::Review(StepFieldId(i)),
                schema: serde_json::json!({"type": "string"}),
                read_only: false,
                dynamic: None,
                file_picker: None,
            })
            .collect();
        let mut form = Form::new("Fields", fields).with_submit_focusable(true);
        form.focus = 18;
        let p = FieldsPanel {
            step_number: 1,
            step_name: "Many".into(),
            description: String::new(),
            form,
            optional: false,
        };
        let mut term = Terminal::new(TestBackend::new(80, 18)).unwrap();
        term.draw(|f| p.render(f, f.area())).unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            buf.contains("field-18"),
            "focused field is scrolled into view: {buf}"
        );
    }

    #[test]
    fn test_render_hint_always_visible_when_fields_overflow() {
        use ags_protocol::workflow::StepFieldId;
        // 20 fields — more than fit in a short terminal — with the focused
        // field (index 18) carrying a description. The hint box must still
        // render even though the fields overflow the panel: this used to be
        // dropped entirely (not shrunk) once Header/Field/Blank/Submit rows
        // consumed all of the panel's vertical budget.
        let fields: Vec<FormField> = (0..20u32)
            .map(|i| FormField {
                label: format!("field-{i}"),
                field_type: FieldType::Scalar,
                required: false,
                value: FieldValue::Scalar(format!("v{i}")),
                description: if i == 18 {
                    "AccelByte user ID".into()
                } else {
                    String::new()
                },
                source: FieldSource::Literal,
                key: FieldKey::Review(StepFieldId(i)),
                schema: serde_json::json!({"type": "string"}),
                read_only: false,
                dynamic: None,
                file_picker: None,
            })
            .collect();
        let mut form = Form::new("Fields", fields).with_submit_focusable(true);
        form.focus = 18;
        let p = FieldsPanel {
            step_number: 1,
            step_name: "Many".into(),
            description: String::new(),
            form,
            optional: false,
        };
        let mut term = Terminal::new(TestBackend::new(60, 14)).unwrap();
        term.draw(|f| p.render(f, f.area())).unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            buf.contains("AccelByte user ID"),
            "focused field's hint remains visible despite overflow: {buf}"
        );
    }

    #[test]
    fn test_render_keeps_field_row_when_inner_height_is_four() {
        use ags_protocol::workflow::StepFieldId;
        // Same 20-field/focus-on-18-with-description fixture as
        // `test_render_hint_always_visible_when_fields_overflow`, but one row
        // taller (60x15 → inner.height == 4, vs. that test's 60x14 →
        // inner.height == 3). At this exact height the hint-slot ladder must
        // reserve only 3 rows (not the usual 4) so a field row still fits
        // alongside a legible one-line hint, instead of spending the whole
        // 4-row budget on the hint (0 field rows + blank hint padding).
        let fields: Vec<FormField> = (0..20u32)
            .map(|i| FormField {
                label: format!("field-{i}"),
                field_type: FieldType::Scalar,
                required: false,
                value: FieldValue::Scalar(format!("v{i}")),
                description: if i == 18 {
                    "AccelByte user ID".into()
                } else {
                    String::new()
                },
                source: FieldSource::Literal,
                key: FieldKey::Review(StepFieldId(i)),
                schema: serde_json::json!({"type": "string"}),
                read_only: false,
                dynamic: None,
                file_picker: None,
            })
            .collect();
        let mut form = Form::new("Fields", fields).with_submit_focusable(true);
        form.focus = 18;
        let p = FieldsPanel {
            step_number: 1,
            step_name: "Many".into(),
            description: String::new(),
            form,
            optional: false,
        };
        let mut term = Terminal::new(TestBackend::new(60, 15)).unwrap();
        term.draw(|f| p.render(f, f.area())).unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            buf.contains("field-18"),
            "focused field row stays visible at inner.height == 4: {buf}"
        );
        assert!(
            buf.contains("AccelByte user ID"),
            "hint text stays visible at inner.height == 4: {buf}"
        );
    }

    #[test]
    fn test_render_groups_fields_under_section_headers() {
        use ags_protocol::workflow::{
            StepField, StepFieldId, StepFieldLocation, StepFieldPlan, StepFieldSource,
        };
        let plan = StepFieldPlan {
            step_index: 0,
            step_label: "create-stat".into(),
            step_description: Some("Creates the stat.".into()),
            optional: false,
            fields: vec![
                // workflow input (read-only) → Workflow values
                StepField {
                    id: StepFieldId(0),
                    field: "namespace".into(),
                    label: "namespace".into(),
                    description: None,
                    location: StepFieldLocation::Path,
                    schema: serde_json::json!({"type":"string"}),
                    value: serde_json::json!("dev"),
                    source: StepFieldSource::WorkflowInput {
                        name: "namespace".into(),
                    },
                    required: true,
                    workflow_input: Some("namespace".into()),
                    body_overflow: false,
                    show_in_review: false,
                },
                // literal (editable, opted in) → Step values
                StepField {
                    id: StepFieldId(1),
                    field: "statCode".into(),
                    label: "stat-code".into(),
                    description: None,
                    location: StepFieldLocation::Body,
                    schema: serde_json::json!({"type":"string"}),
                    value: serde_json::json!("mmr"),
                    source: StepFieldSource::Literal,
                    required: true,
                    workflow_input: None,
                    body_overflow: false,
                    show_in_review: true,
                },
            ],
        };
        let panel = FieldsPanel {
            step_number: 1,
            step_name: "create-stat".into(),
            description: "Creates the stat.".into(),
            form: Form::from_step_plan(&plan),
            optional: false,
        };
        let mut term = Terminal::new(TestBackend::new(100, 24)).unwrap();
        term.draw(|f| panel.render(f, f.area())).unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            buf.contains("Inputs (read-only)"),
            "Inputs header present: {buf}"
        );
        assert!(buf.contains("Options"), "Options header present: {buf}");
        // Per-row source suffixes are gone.
        assert!(
            !buf.contains("(workflow value)"),
            "row suffix removed: {buf}"
        );
        assert!(!buf.contains("(step value)"), "row suffix removed: {buf}");
        // Description renders in the step box.
        assert!(
            buf.contains("Creates the stat."),
            "description rendered: {buf}"
        );
    }

    #[test]
    fn test_hint_slot_fixed_below_submit_does_not_shift_fields() {
        use ags_protocol::workflow::StepFieldId;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let fields = vec![
            FormField {
                label: "alpha".into(),
                field_type: FieldType::Scalar,
                required: false,
                value: FieldValue::Scalar("va".into()),
                description: "describe alpha".into(),
                source: FieldSource::Literal,
                key: FieldKey::Review(StepFieldId(0)),
                schema: serde_json::json!({"type":"string"}),
                read_only: false,
                dynamic: None,
                file_picker: None,
            },
            FormField {
                label: "beta".into(),
                field_type: FieldType::Scalar,
                required: false,
                value: FieldValue::Scalar("vb".into()),
                description: "describe beta".into(),
                source: FieldSource::Literal,
                key: FieldKey::Review(StepFieldId(1)),
                schema: serde_json::json!({"type":"string"}),
                read_only: false,
                dynamic: None,
                file_picker: None,
            },
        ];
        let mut form_a = Form::new("Parameters", fields.clone()).with_submit_focusable(true);
        form_a.focus = 0;
        let p_a = FieldsPanel {
            step_number: 1,
            step_name: "x".into(),
            description: String::new(),
            form: form_a,
            optional: false,
        };
        let mut term_a = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term_a.draw(|f| p_a.render(f, f.area())).unwrap();
        let buf_a = term_a.backend().buffer().clone();
        let alpha_off_a = label_row(&buf_a, "alpha");
        let beta_off_a = label_row(&buf_a, "beta");

        let mut form_b = Form::new("Parameters", fields).with_submit_focusable(true);
        form_b.focus = 1;
        let p_b = FieldsPanel {
            step_number: 1,
            step_name: "x".into(),
            description: String::new(),
            form: form_b,
            optional: false,
        };
        let mut term_b = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term_b.draw(|f| p_b.render(f, f.area())).unwrap();
        let buf_b = term_b.backend().buffer().clone();
        let alpha_off_b = label_row(&buf_b, "alpha");
        let beta_off_b = label_row(&buf_b, "beta");

        assert_eq!(
            alpha_off_a, alpha_off_b,
            "alpha row position is stable across focus"
        );
        assert_eq!(
            beta_off_a, beta_off_b,
            "beta row position is stable across focus"
        );
    }

    #[test]
    fn test_hint_box_flush_with_panel_inner_edge() {
        use ags_protocol::workflow::StepFieldId;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let field = FormField {
            label: "alpha".into(),
            field_type: FieldType::Scalar,
            required: false,
            value: FieldValue::Scalar("va".into()),
            description: "describe alpha".into(),
            source: FieldSource::Literal,
            key: FieldKey::Review(StepFieldId(0)),
            schema: serde_json::json!({"type":"string"}),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        let form = Form::new("Parameters", vec![field]).with_submit_focusable(true);
        let p = FieldsPanel {
            step_number: 1,
            step_name: "x".into(),
            description: String::new(),
            form,
            optional: false,
        };
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| p.render(f, f.area())).unwrap();
        let buf = term.backend().buffer().clone();
        // The hint box is the only bordered box drawn with a dim Indexed(244)
        // border, so its top-left `┌` is uniquely identifiable. Locate it by
        // CELL index (not a byte offset into the joined symbols, which the
        // multi-byte border glyphs would skew) so `% 80` is the true column.
        let corner = buf
            .content()
            .iter()
            .position(|c| c.symbol() == "\u{250C}" && c.fg == Color::Indexed(244))
            .expect("dim hint box top-left corner present");
        let col = corner % 80;
        // The hint box sits flush with the Parameters box inner edge (outer
        // border 1 + left padding 2 = col 3) so its left border aligns with the
        // Confirm button rather than indenting under the labels (`render_hint_box`).
        assert_eq!(col, 3, "hint box flush with panel inner edge: col={col}");
    }

    #[test]
    fn test_derived_field_renders_with_value_and_source_label() {
        use ags_protocol::workflow::{
            StepField, StepFieldId, StepFieldLocation, StepFieldPlan, StepFieldSource,
        };
        let plan = StepFieldPlan {
            step_index: 0,
            step_label: "create-fleet".into(),
            step_description: None,
            optional: false,
            fields: vec![StepField {
                id: StepFieldId(0),
                field: "resourcePrefix".into(),
                label: "resource-prefix".into(),
                description: None,
                location: StepFieldLocation::Body,
                schema: serde_json::json!({"type": "string"}),
                value: serde_json::json!("ranked-fleet"),
                source: StepFieldSource::Derived {
                    sources: vec!["resourcePrefix".into()],
                },
                required: false,
                workflow_input: None,
                body_overflow: false,
                show_in_review: false,
            }],
        };
        let panel = FieldsPanel {
            step_number: 1,
            step_name: "create-fleet".into(),
            description: String::new(),
            form: Form::from_step_plan(&plan),
            optional: false,
        };
        let mut term = Terminal::new(TestBackend::new(100, 24)).unwrap();
        term.draw(|f| panel.render(f, f.area())).unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(buf.contains("ranked-fleet"), "resolved value shown: {buf}");
        assert!(buf.contains("resource-prefix"), "field label shown: {buf}");
        assert!(buf.contains("derived"), "derived source hint shown: {buf}");
    }

    /// Render a panel to an 80x24 TestBackend and return the buffer as lines of text.
    fn render_to_text(panel: &FieldsPanel) -> String {
        use ratatui::{backend::TestBackend, Terminal};
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| panel.render(f, f.area())).unwrap();
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
    fn test_snapshot_fields_panel_gather_variant() {
        // step_number 0 -> gather variant ("Inputs").
        let field = FormField {
            label: "user-id".into(),
            field_type: FieldType::Scalar,
            required: true,
            value: FieldValue::Empty,
            description: "The player to target".into(),
            source: FieldSource::UserInput,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type": "string"}),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        let panel = FieldsPanel {
            step_number: 0,
            step_name: "Provide inputs".into(),
            description: String::new(),
            form: Form::new("Provide inputs", vec![field]).with_submit_focusable(true),
            optional: false,
        };
        insta::with_settings!({snapshot_path => "../../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(render_to_text(&panel));
        });
    }

    #[test]
    fn test_snapshot_fields_panel_review_variant() {
        // step_number 1 -> review variant ("Inputs (read-only)") with a description.
        let p = panel(); // existing helper
        insta::with_settings!({snapshot_path => "../../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(render_to_text(&p));
        });
    }

    #[test]
    fn test_review_form_renders_fixed_and_input_fields() {
        use ags_protocol::workflow::{
            StepField, StepFieldId, StepFieldLocation, StepFieldPlan, StepFieldSource,
        };
        let plan = StepFieldPlan {
            step_index: 0,
            step_label: "Create the MMR skill stat".into(),
            step_description: None,
            optional: false,
            fields: vec![
                StepField {
                    id: StepFieldId(0),
                    field: "namespace".into(),
                    label: "namespace".into(),
                    description: None,
                    location: StepFieldLocation::Path,
                    schema: serde_json::json!({"type":"string"}),
                    value: serde_json::json!("dev"),
                    source: StepFieldSource::WorkflowInput {
                        name: "namespace".into(),
                    },
                    required: true,
                    workflow_input: Some("namespace".into()),
                    body_overflow: false,
                    show_in_review: false,
                },
                StepField {
                    id: StepFieldId(1),
                    field: "statCode".into(),
                    label: "stat-code".into(),
                    description: None,
                    location: StepFieldLocation::Body,
                    schema: serde_json::json!({"type":"string"}),
                    value: serde_json::json!("mmr"),
                    source: StepFieldSource::Literal,
                    required: true,
                    workflow_input: None,
                    body_overflow: false,
                    show_in_review: true,
                },
            ],
        };
        let panel = FieldsPanel {
            step_number: 1,
            step_name: "Create the MMR skill stat".into(),
            description: String::new(),
            form: Form::from_step_plan(&plan),
            optional: false,
        };
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| panel.render(f, f.area())).unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(buf.contains("stat-code"), "fixed field shown: {buf}");
        assert!(buf.contains("mmr"), "fixed value shown");
        assert!(buf.contains("namespace"), "workflow-input field shown");
    }
}
