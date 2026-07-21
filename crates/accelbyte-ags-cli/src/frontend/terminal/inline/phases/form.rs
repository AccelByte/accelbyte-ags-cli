//! Multi-field form phase.
//!
//! Drives the [`Form`] widget via crossterm key events. `FormPhase` handles
//! multi-field service-command input (and, once Phase 4 wires it, the
//! fullscreen workflow Fields panel too). It superseded the old single-slot
//! prompt phase, which the form/`form_runner` path replaced.

use ags_protocol::workflow::GatherResult;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;

use super::{Phase, PhaseStep};
use crate::frontend::terminal::inline::form::{
    derived_field_note, ConfirmSkipChoice, FieldSource, FieldType, FieldValue, Form,
};

/// Result returned by a [`FormPhase`] to the caller.
pub enum PhaseResult {
    /// User completed the phase and submitted values.
    Submitted(GatherResult),
    /// User pressed Enter on a `JsonBody` field — open the JSON editor for
    /// the given field index.
    OpenJsonEditor(usize),
    /// User activated a `DynamicEnum` field — the driver should open the modal
    /// picker for the given field index (fetching choices first if needed).
    OpenEnumPicker(usize),
}

/// Phase that hosts an inline [`Form`] and translates key events into
/// focus/edit/submit actions. The button at the bottom of
/// the form maps to Enter when no field is being edited and all required
/// fields have values; Ctrl-S works regardless of focus.
pub struct FormPhase {
    form: Form,
}

impl FormPhase {
    /// Wrap a `Form` in its phase.
    pub fn new(form: Form) -> Self {
        Self { form }
    }

    /// Borrow the wrapped form.
    pub fn form(&self) -> &Form {
        &self.form
    }

    /// Mutably borrow the wrapped form.
    pub fn form_mut(&mut self) -> &mut Form {
        &mut self.form
    }

    /// Consume the phase, returning the owned [`Form`]. The fullscreen
    /// interaction wraps a panel's form in a transient `FormPhase` to reuse
    /// [`on_key`](FormPhase::on_key), then takes it back via this.
    pub fn into_form(self) -> Form {
        self.form
    }
}

impl Phase for FormPhase {
    type Output = PhaseResult;

    fn render(&self, frame: &mut Frame<'_>) {
        self.form.render(frame, frame.area());
    }

    fn on_key(&mut self, key: KeyEvent) -> PhaseStep<Self::Output> {
        // A DateTime segment edit intercepts every key before the global
        // focus/button arms, so the arrow/digit segment controls are not
        // swallowed by focus navigation. Enter commits, esc cancels.
        if self.form.editing.as_ref().is_some_and(|s| s.is_date()) {
            use crate::frontend::terminal::date_field::{apply_date_key, DateStep};
            if let Some(date) = self.form.editing.as_mut().and_then(|s| s.date_mut()) {
                return match apply_date_key(date, key) {
                    DateStep::Continue => PhaseStep::Continue,
                    DateStep::Commit => {
                        self.form.commit_edit();
                        PhaseStep::Continue
                    }
                    DateStep::Cancel => {
                        self.form.cancel_edit();
                        PhaseStep::Continue
                    }
                };
            }
        }

        match (key.code, key.modifiers) {
            (KeyCode::Tab, _) | (KeyCode::Down, _) => {
                self.form.focus_next();
                PhaseStep::Continue
            }
            (KeyCode::BackTab, _) | (KeyCode::Up, _) => {
                self.form.focus_prev();
                PhaseStep::Continue
            }
            // Left/Right navigate across button groups (run-mode and
            // confirm/skip) while the submit slot is focused. Elsewhere they
            // are inert (fields have no horizontal navigation).
            (KeyCode::Left, _)
                if self.form.editing.is_none()
                    && self.form.run_mode_buttons
                    && self.form.is_submit_focused() =>
            {
                self.form.run_mode_focus_prev();
                PhaseStep::Continue
            }
            (KeyCode::Right, _)
                if self.form.editing.is_none()
                    && self.form.run_mode_buttons
                    && self.form.is_submit_focused() =>
            {
                self.form.run_mode_focus_next();
                PhaseStep::Continue
            }
            (KeyCode::Left, _)
                if self.form.editing.is_none()
                    && self.form.confirm_skip_buttons
                    && self.form.is_submit_focused() =>
            {
                self.form.confirm_skip_focus_prev();
                PhaseStep::Continue
            }
            (KeyCode::Right, _)
                if self.form.editing.is_none()
                    && self.form.confirm_skip_buttons
                    && self.form.is_submit_focused() =>
            {
                self.form.confirm_skip_focus_next();
                PhaseStep::Continue
            }
            (KeyCode::Enter, _) => {
                // Not in line-edit mode + focused field is
                // JsonBody → open the tree editor. This is checked BEFORE
                // submit, so Enter on a (possibly pre-filled) JsonBody field
                // always re-opens the editor; submit is via Ctrl-S or Enter on
                // a non-JsonBody field once required inputs are filled.
                if self.form.editing.is_some() {
                    self.form.commit_edit();
                    PhaseStep::Continue
                } else if self.form.is_submit_focused() {
                    // When the confirm/skip group is active and Skip is focused,
                    // submit immediately — the caller reads selected_confirm_skip()
                    // to route to Skip rather than Proceed, so required-field
                    // validation is bypassed (the step is being skipped entirely).
                    if self.form.confirm_skip_buttons
                        && self.form.selected_confirm_skip() == ConfirmSkipChoice::Skip
                    {
                        return PhaseStep::Done(PhaseResult::Submitted(
                            self.form.project_gathered(),
                        ));
                    }
                    // Focused the Confirm button.
                    if self.form.all_required_filled() {
                        PhaseStep::Done(PhaseResult::Submitted(self.form.project_gathered()))
                    } else if let Some(i) = self.form.first_unfilled_required() {
                        // Block and flag: jump to the offending field + set a note.
                        let field = &self.form.fields[i];
                        let label = field.label.clone();
                        let note = if matches!(field.field_type, FieldType::JsonBody) {
                            if let FieldValue::JsonBody(s) = &field.value {
                                let missing =
                                    crate::frontend::terminal::inline::form::missing_required_keys(
                                        s,
                                        &field.schema,
                                    );
                                if !missing.is_empty() {
                                    format!("Required: {label} (missing: {})", missing.join(", "))
                                } else {
                                    format!("Required: {label}")
                                }
                            } else {
                                format!("Required: {label}")
                            }
                        } else {
                            format!("Required: {label}")
                        };
                        self.form.focus = i;
                        self.form.validation_note = Some(note);
                        PhaseStep::Continue
                    } else {
                        PhaseStep::Continue
                    }
                } else if let Some(field) = self.form.focused() {
                    // Read-only fields cannot be edited. For Derived fields,
                    // surface an explanatory message naming the source inputs
                    // so the user knows how to change the value. Other
                    // read-only sources (PriorOutput, WorkflowInput, Literal)
                    // are already self-explanatory from their rendered suffix,
                    // so they silently no-op.
                    if field.read_only {
                        if let FieldSource::Derived { sources } = &field.source {
                            self.form.validation_note =
                                Some(derived_field_note(&field.label, sources));
                        }
                        return PhaseStep::Continue;
                    }
                    match field.field_type {
                        FieldType::Enum { .. } => {
                            self.form.cycle_focused_enum(true);
                            PhaseStep::Continue
                        }
                        FieldType::Bool => {
                            self.form.toggle_focused_bool();
                            PhaseStep::Continue
                        }
                        FieldType::JsonBody => {
                            PhaseStep::Done(PhaseResult::OpenJsonEditor(self.form.focus))
                        }
                        FieldType::Scalar => {
                            if self.form.all_required_filled() && !self.form.submit_focusable {
                                PhaseStep::Done(PhaseResult::Submitted(
                                    self.form.project_gathered(),
                                ))
                            } else {
                                self.form.begin_edit();
                                PhaseStep::Continue
                            }
                        }
                        FieldType::DynamicEnum => {
                            // Always open the modal picker (it fetches lazily if
                            // the choices aren't cached yet).
                            PhaseStep::Done(PhaseResult::OpenEnumPicker(self.form.focus))
                        }
                        FieldType::DateTime => {
                            // Always open the segmented editor; never take the
                            // Scalar submit shortcut. Segment keys are handled by
                            // the early date guard at the top of on_key.
                            self.form.begin_edit();
                            PhaseStep::Continue
                        }
                    }
                } else {
                    PhaseStep::Continue
                }
            }
            (KeyCode::Esc, _) => {
                if self.form.editing.is_some() {
                    self.form.cancel_edit();
                    PhaseStep::Continue
                } else {
                    PhaseStep::Cancelled
                }
            }
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                if self.form.all_required_filled() {
                    PhaseStep::Done(PhaseResult::Submitted(self.form.project_gathered()))
                } else {
                    PhaseStep::Continue
                }
            }
            (KeyCode::Char('o'), m)
                if self.form.editing.is_none()
                    && !m.contains(KeyModifiers::CONTROL)
                    && self.form.optional_filter =>
            {
                self.form.show_optional = !self.form.show_optional;
                // Collapsing can hide the focused optional row — snap focus to a
                // visible row so Enter/edit never acts on an unrendered field.
                self.form.normalize_focus_to_visible();
                PhaseStep::Continue
            }
            (KeyCode::Char(' '), m) if self.form.editing.is_none() => {
                if let Some(field) = self.form.focused() {
                    match field.field_type {
                        FieldType::Enum { .. } => {
                            self.form
                                .cycle_focused_enum(!m.contains(KeyModifiers::SHIFT));
                            return PhaseStep::Continue;
                        }
                        FieldType::Bool => {
                            self.form.toggle_focused_bool();
                            return PhaseStep::Continue;
                        }
                        FieldType::DynamicEnum => {
                            return PhaseStep::Done(PhaseResult::OpenEnumPicker(self.form.focus));
                        }
                        _ => {}
                    }
                }
                // Fall through for scalar fields in non-edit mode — no-op.
                PhaseStep::Continue
            }
            (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
                if let Some(buf) = self.form.editing.as_mut().and_then(|s| s.text_buffer_mut()) {
                    buf.push(c);
                }
                PhaseStep::Continue
            }
            (KeyCode::Backspace, _) => {
                if let Some(buf) = self.form.editing.as_mut().and_then(|s| s.text_buffer_mut()) {
                    buf.pop();
                }
                PhaseStep::Continue
            }
            _ => PhaseStep::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::terminal::inline::form::{
        FieldKey, FieldSource, FieldType, FieldValue, FormField,
    };
    use ags_protocol::workflow::GatherSlotId;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn two_field_form() -> Form {
        Form::new(
            "test",
            vec![
                FormField {
                    label: "first".into(),
                    field_type: FieldType::Scalar,
                    required: true,
                    value: FieldValue::Empty,
                    description: String::new(),
                    source: FieldSource::Default,
                    key: FieldKey::Slot(GatherSlotId(0)),
                    schema: serde_json::json!({"type": "string"}),
                    read_only: false,
                    dynamic: None,
                },
                FormField {
                    label: "second".into(),
                    field_type: FieldType::Scalar,
                    required: false,
                    value: FieldValue::Empty,
                    description: String::new(),
                    source: FieldSource::Optional,
                    key: FieldKey::Slot(GatherSlotId(1)),
                    schema: serde_json::json!({"type": "string"}),
                    read_only: false,
                    dynamic: None,
                },
            ],
        )
    }

    #[test]
    fn test_submit_focused_enter_submits_when_required_filled() {
        let mut form = two_field_form(); // field0 required
        form.submit_focusable = true;
        form.fields[0].value = FieldValue::Scalar("x".into());
        form.focus = form.fields.len(); // submit slot
        let mut phase = FormPhase::new(form);
        assert!(matches!(
            phase.on_key(key(KeyCode::Enter)),
            PhaseStep::Done(PhaseResult::Submitted(_))
        ));
    }

    #[test]
    fn test_submit_focused_enter_blocks_and_flags_offending_field() {
        // field0 required+empty, field1 optional. Submit blocks, jumps focus to
        // the first unfilled required field, and sets a validation note.
        let mut form = two_field_form();
        form.submit_focusable = true;
        form.focus = form.fields.len(); // submit slot
        let mut phase = FormPhase::new(form);
        assert!(matches!(
            phase.on_key(key(KeyCode::Enter)),
            PhaseStep::Continue
        ));
        assert_eq!(phase.form().focus, 0); // jumped to offending field
        assert!(phase.form().validation_note.is_some());
    }

    #[test]
    fn test_tab_focus_advances_to_next_field() {
        let mut phase = FormPhase::new(two_field_form());
        assert!(matches!(
            phase.on_key(key(KeyCode::Tab)),
            PhaseStep::Continue
        ));
        assert_eq!(phase.form().focus, 1);
    }

    #[test]
    fn test_enter_on_unfilled_required_field_starts_edit_mode() {
        let mut phase = FormPhase::new(two_field_form());
        phase.on_key(key(KeyCode::Enter));
        assert!(phase.form().editing.is_some());
    }

    fn datetime_phase(iso: &str) -> FormPhase {
        use crate::frontend::terminal::inline::form::{
            FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        let field = FormField {
            label: "start".into(),
            field_type: FieldType::DateTime,
            required: false,
            value: FieldValue::Scalar(iso.into()),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("start".into()),
            schema: serde_json::json!({"type": "string", "format": "date-time"}),
            read_only: false,
            dynamic: None,
        };
        let mut form = Form::new("t", vec![field]);
        form.focus = 0;
        FormPhase::new(form)
    }

    #[test]
    fn test_enter_on_datetime_begins_edit_not_submit() {
        // Single prefilled DateTime field, submit not focusable, all required
        // filled — a Scalar here would submit; a DateTime must open for editing.
        let mut phase = datetime_phase("2026-08-01T00:00:00Z");
        let step = phase.on_key(key(KeyCode::Enter));
        assert!(matches!(step, PhaseStep::Continue));
        assert!(
            phase.form().editing.as_ref().unwrap().is_date(),
            "enter opens the segmented date editor"
        );
    }

    #[test]
    fn test_up_down_adjust_segment_not_focus_while_date_editing() {
        let mut phase = datetime_phase("2026-08-01T00:00:00Z");
        phase.on_key(key(KeyCode::Enter)); // begin date edit, segment 0 (year)
        phase.on_key(key(KeyCode::Right)); // -> month
        phase.on_key(key(KeyCode::Up)); // month 08 -> 09
        let d = phase.form().editing.as_ref().unwrap().date().unwrap();
        assert_eq!(d.segment, 1);
        assert_eq!(
            d.parts.month, 9,
            "Up adjusted the segment, did not move focus"
        );
    }

    #[test]
    fn test_typing_in_edit_mode_appends_to_buffer() {
        let mut phase = FormPhase::new(two_field_form());
        phase.on_key(key(KeyCode::Enter)); // begin_edit
        phase.on_key(key(KeyCode::Char('a')));
        phase.on_key(key(KeyCode::Char('b')));
        assert_eq!(
            phase.form().editing.as_ref().unwrap().text_buffer(),
            Some("ab")
        );
    }

    #[test]
    fn test_enter_after_edit_commits_then_subsequent_enter_submits_when_required_filled() {
        let mut phase = FormPhase::new(two_field_form());
        phase.on_key(key(KeyCode::Enter)); // begin_edit on focus=0
        phase.on_key(key(KeyCode::Char('x')));
        phase.on_key(key(KeyCode::Enter)); // commit
        assert!(phase.form().editing.is_none());
        match phase.on_key(key(KeyCode::Enter)) {
            PhaseStep::Done(PhaseResult::Submitted(result)) => {
                // field[0] is Slot(0) → slot_values
                assert_eq!(
                    result.slot_values.get(&GatherSlotId(0)),
                    Some(&serde_json::Value::String("x".into()))
                );
                // field[1] is optional and empty → absent from both maps
                assert!(!result.slot_values.contains_key(&GatherSlotId(1)));
                assert!(result.input_overrides.is_empty());
            }
            other => panic!("expected Submitted, got non-Done: {:?}", other.kind()),
        }
    }

    #[test]
    fn test_enter_on_json_body_field_not_editing_emits_open_json_editor() {
        let form = Form::new(
            "test",
            vec![FormField {
                label: "body".into(),
                field_type: FieldType::JsonBody,
                required: true,
                value: FieldValue::Empty,
                description: String::new(),
                source: FieldSource::Default,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({"type": "object"}),
                read_only: false,
                dynamic: None,
            }],
        );
        let mut phase = FormPhase::new(form);
        match phase.on_key(key(KeyCode::Enter)) {
            PhaseStep::Done(PhaseResult::OpenJsonEditor(idx)) => {
                assert_eq!(idx, 0);
            }
            other => panic!("expected OpenJsonEditor, got: {:?}", other.kind()),
        }
    }

    #[test]
    fn test_enter_on_filled_json_body_opens_editor_not_submit() {
        // Regression guard: the JsonBody check must precede the submit check.
        // A pre-filled JsonBody (so all_required_filled() is true) must still
        // open the editor on Enter, not submit.
        let form = Form::new(
            "test",
            vec![FormField {
                label: "body".into(),
                field_type: FieldType::JsonBody,
                required: true,
                value: FieldValue::JsonBody("{\"a\":1}".into()),
                description: String::new(),
                source: FieldSource::FromFlag,
                key: FieldKey::Input("body".into()),
                schema: serde_json::json!({"type": "object"}),
                read_only: false,
                dynamic: None,
            }],
        );
        let mut phase = FormPhase::new(form);
        match phase.on_key(key(KeyCode::Enter)) {
            PhaseStep::Done(PhaseResult::OpenJsonEditor(idx)) => assert_eq!(idx, 0),
            other => panic!("expected OpenJsonEditor, got: {:?}", other.kind()),
        }
    }

    #[test]
    fn test_esc_in_field_mode_cancels_phase() {
        let mut phase = FormPhase::new(two_field_form());
        assert!(matches!(
            phase.on_key(key(KeyCode::Esc)),
            PhaseStep::Cancelled
        ));
    }

    #[test]
    fn test_esc_in_edit_mode_only_cancels_the_edit() {
        let mut phase = FormPhase::new(two_field_form());
        phase.on_key(key(KeyCode::Enter)); // begin_edit
        phase.on_key(key(KeyCode::Char('a')));
        assert!(matches!(
            phase.on_key(key(KeyCode::Esc)),
            PhaseStep::Continue
        ));
        assert!(phase.form().editing.is_none());
    }

    #[test]
    fn test_ctrl_s_submits_when_required_filled() {
        let mut phase = FormPhase::new(two_field_form());
        phase.form.fields[0].value = FieldValue::Scalar("ready".into());
        let step = phase.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(matches!(step, PhaseStep::Done(PhaseResult::Submitted(_))));
    }

    #[test]
    fn test_ctrl_s_is_noop_when_required_not_filled() {
        let mut phase = FormPhase::new(two_field_form());
        let step = phase.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(matches!(step, PhaseStep::Continue));
    }

    #[test]
    fn test_submit_focusable_enter_on_field_edits_not_submits_when_filled() {
        use crate::frontend::terminal::inline::form::{
            FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        use ags_protocol::workflow::GatherSlotId;
        let field = FormField {
            label: "a".into(),
            field_type: FieldType::Scalar,
            required: true,
            value: FieldValue::Scalar("filled".into()),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type":"string"}),
            read_only: false,
            dynamic: None,
        };
        let mut phase = FormPhase::new(Form::new("t", vec![field]).with_submit_focusable(true));
        // focus 0 (the field), all required filled → Enter must begin edit, NOT submit.
        let step = phase.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(step, PhaseStep::Continue));
        assert!(
            phase.form().editing.is_some(),
            "Enter began editing, did not submit"
        );
    }

    #[test]
    fn test_space_cycles_focused_enum() {
        use crate::frontend::terminal::inline::form::{
            FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        use ags_protocol::workflow::GatherSlotId;
        let field = FormField {
            label: "mode".into(),
            field_type: FieldType::Enum {
                variants: vec!["a".into(), "b".into()],
            },
            required: false,
            value: FieldValue::Enum(Some("a".into())),
            description: String::new(),
            source: FieldSource::Default,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({}),
            read_only: false,
            dynamic: None,
        };
        let mut phase = FormPhase::new(Form::new("t", vec![field]));
        let step = phase.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(matches!(step, PhaseStep::Continue));
        assert!(matches!(phase.form().fields[0].value, FieldValue::Enum(Some(ref s)) if s == "b"));
    }

    #[test]
    fn test_shift_space_cycles_focused_enum_backward() {
        use crate::frontend::terminal::inline::form::{
            FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        use ags_protocol::workflow::GatherSlotId;
        let field = FormField {
            label: "mode".into(),
            field_type: FieldType::Enum {
                variants: vec!["a".into(), "b".into(), "c".into()],
            },
            required: false,
            value: FieldValue::Enum(Some("a".into())),
            description: String::new(),
            source: FieldSource::Default,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({}),
            read_only: false,
            dynamic: None,
        };
        let mut phase = FormPhase::new(Form::new("t", vec![field]));
        let step = phase.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT));
        assert!(matches!(step, PhaseStep::Continue));
        assert!(matches!(phase.form().fields[0].value, FieldValue::Enum(Some(ref s)) if s == "c"));
    }

    #[test]
    fn test_space_toggles_focused_bool() {
        use crate::frontend::terminal::inline::form::{
            FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        use ags_protocol::workflow::GatherSlotId;
        let field = FormField {
            label: "active".into(),
            field_type: FieldType::Bool,
            required: false,
            value: FieldValue::Bool(Some(false)),
            description: String::new(),
            source: FieldSource::Default,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({}),
            read_only: false,
            dynamic: None,
        };
        let mut phase = FormPhase::new(Form::new("t", vec![field]));
        let step = phase.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(matches!(step, PhaseStep::Continue));
        assert!(matches!(
            phase.form().fields[0].value,
            FieldValue::Bool(Some(true))
        ));
    }

    #[test]
    fn test_enter_on_enum_cycles_not_text_edits() {
        use crate::frontend::terminal::inline::form::{
            FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        use ags_protocol::workflow::GatherSlotId;
        let field = FormField {
            label: "mode".into(),
            field_type: FieldType::Enum {
                variants: vec!["a".into(), "b".into()],
            },
            required: false,
            value: FieldValue::Enum(Some("a".into())),
            description: String::new(),
            source: FieldSource::Default,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({}),
            read_only: false,
            dynamic: None,
        };
        let mut phase = FormPhase::new(Form::new("t", vec![field]));
        let step = phase.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(step, PhaseStep::Continue));
        assert!(matches!(phase.form().fields[0].value, FieldValue::Enum(Some(ref s)) if s == "b"));
        assert!(
            phase.form().editing.is_none(),
            "Enter does not enter edit mode for enum"
        );
    }

    #[test]
    fn test_submit_block_lists_missing_jsonbody_subkeys() {
        use crate::frontend::terminal::inline::form::{
            FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        let schema = serde_json::json!({
            "type": "object",
            "required": ["imageId", "commandLine"],
            "properties": {
                "imageId": {"type": "string"},
                "commandLine": {"type": "string"},
            },
        });
        let field = FormField {
            label: "img-deploy".into(),
            field_type: FieldType::JsonBody,
            required: true,
            value: FieldValue::JsonBody(
                serde_json::to_string(&serde_json::json!({"imageId": "x"})).unwrap(),
            ),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("img-deploy".into()),
            schema,
            read_only: false,
            dynamic: None,
        };
        let mut form = Form::new("t", vec![field]).with_submit_focusable(true);
        form.focus = form.fields.len(); // Submit slot
        let mut phase = FormPhase::new(form);
        let _ = phase.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let note = phase
            .form()
            .validation_note
            .as_deref()
            .expect("validation note set");
        assert!(
            note.contains("img-deploy"),
            "note names the parent field: {note}"
        );
        assert!(
            note.contains("missing: commandLine"),
            "note lists missing sub-key: {note}"
        );
    }

    /// Pressing Enter on a focused Derived field must:
    /// - return `Continue` (not open an editor or submit)
    /// - set a `validation_note` that names the source inputs and mentions "derived"
    #[test]
    fn test_enter_on_derived_field_sets_rejection_note_with_source_names() {
        use crate::frontend::terminal::inline::form::{
            FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        use ags_protocol::workflow::GatherSlotId;
        let field = FormField {
            label: "resource-prefix".into(),
            field_type: FieldType::Scalar,
            required: false,
            value: FieldValue::Scalar("ranked-fleet".into()),
            description: String::new(),
            source: FieldSource::Derived {
                sources: vec!["resourcePrefix".into(), "teamCount".into()],
            },
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type": "string"}),
            read_only: true,
            dynamic: None,
        };
        let mut phase = FormPhase::new(Form::new("t", vec![field]));
        let step = phase.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            matches!(step, PhaseStep::Continue),
            "Enter on Derived must return Continue"
        );
        assert!(
            phase.form().editing.is_none(),
            "edit must not open on a Derived field"
        );
        let note = phase
            .form()
            .validation_note
            .as_deref()
            .expect("validation note set");
        assert!(
            note.to_lowercase().contains("derived"),
            "note must mention 'derived': {note}"
        );
        assert!(
            note.contains("resourcePrefix"),
            "note must name first source input: {note}"
        );
        assert!(
            note.contains("teamCount"),
            "note must name second source input: {note}"
        );
    }

    /// Pressing Enter on a focused Derived JsonBody field must also be rejected
    /// (not emit `OpenJsonEditor`) and surface the explanatory note.
    #[test]
    fn test_enter_on_derived_jsonbody_field_is_rejected_not_editor_opened() {
        use crate::frontend::terminal::inline::form::{
            FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        use ags_protocol::workflow::GatherSlotId;
        let field = FormField {
            label: "config".into(),
            field_type: FieldType::JsonBody,
            required: false,
            value: FieldValue::JsonBody("{\"k\":\"v\"}".into()),
            description: String::new(),
            source: FieldSource::Derived {
                sources: vec!["baseConfig".into()],
            },
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type": "object"}),
            read_only: true,
            dynamic: None,
        };
        let mut phase = FormPhase::new(Form::new("t", vec![field]));
        let step = phase.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        // Must not open the JSON editor.
        assert!(
            matches!(step, PhaseStep::Continue),
            "Enter on Derived JsonBody must not emit OpenJsonEditor"
        );
        let note = phase
            .form()
            .validation_note
            .as_deref()
            .expect("validation note set");
        assert!(
            note.to_lowercase().contains("derived"),
            "note must mention 'derived': {note}"
        );
        assert!(
            note.contains("baseConfig"),
            "note must name the source input: {note}"
        );
    }

    fn dynamic_field(resolved: bool) -> crate::frontend::terminal::inline::form::FormField {
        use crate::frontend::terminal::inline::form::{
            DynamicEnumState, FieldKey, FieldSource, FieldType, FieldValue, FormField,
            ResolvedChoices,
        };
        use ags_protocol::workflow::{OperationReference, OptionChoice, OptionsSource};
        let resolved = if resolved {
            Some(ResolvedChoices {
                dep_key: std::collections::BTreeMap::new(),
                choices: vec![
                    OptionChoice {
                        label: "Prod".into(),
                        value: "img-1".into(),
                    },
                    OptionChoice {
                        label: "Stg".into(),
                        value: "img-2".into(),
                    },
                ],
                truncated: false,
            })
        } else {
            None
        };
        FormField {
            label: "fleet-image-id".into(),
            field_type: FieldType::DynamicEnum,
            required: true,
            value: FieldValue::Enum(None),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("fleetImageId".into()),
            schema: serde_json::json!({"type": "string"}),
            read_only: false,
            dynamic: Some(DynamicEnumState {
                source: OptionsSource {
                    operation: OperationReference {
                        service: ags_protocol::catalogue::ServiceId::new("ams"),
                        operation: ags_protocol::catalogue::OperationId::new(
                            "ams/admin/images/v1/list",
                        ),
                    },
                    parameters: std::collections::BTreeMap::new(),
                    items_path: "$.images".into(),
                    value: "$.id".into(),
                    label: Some("$.name".into()),
                    label_detail: None,
                    fallback_description: None,
                    filter: None,
                },
                deps: vec!["namespace".into()],
                optional_deps: vec![],
                resolved,
            }),
        }
    }

    #[test]
    fn test_enter_on_dynamic_enum_opens_picker() {
        use crate::frontend::terminal::inline::form::Form;
        let mut phase =
            FormPhase::new(Form::new("t", vec![dynamic_field(false)]).with_submit_focusable(true));
        phase.form_mut().focus = 0;
        match phase.on_key(key(KeyCode::Enter)) {
            PhaseStep::Done(PhaseResult::OpenEnumPicker(idx)) => assert_eq!(idx, 0),
            other => panic!("expected OpenEnumPicker, got {:?}", other.kind()),
        }
    }

    #[test]
    fn test_enter_on_resolved_dynamic_enum_opens_picker() {
        use crate::frontend::terminal::inline::form::Form;
        let mut phase =
            FormPhase::new(Form::new("t", vec![dynamic_field(true)]).with_submit_focusable(true));
        phase.form_mut().focus = 0;
        match phase.on_key(key(KeyCode::Enter)) {
            PhaseStep::Done(PhaseResult::OpenEnumPicker(idx)) => assert_eq!(idx, 0),
            other => panic!("expected OpenEnumPicker, got {:?}", other.kind()),
        }
    }

    #[test]
    fn test_space_on_dynamic_enum_opens_picker() {
        use crate::frontend::terminal::inline::form::Form;
        let mut phase =
            FormPhase::new(Form::new("t", vec![dynamic_field(true)]).with_submit_focusable(true));
        phase.form_mut().focus = 0;
        match phase.on_key(key(KeyCode::Char(' '))) {
            PhaseStep::Done(PhaseResult::OpenEnumPicker(idx)) => assert_eq!(idx, 0),
            other => panic!("expected OpenEnumPicker, got {:?}", other.kind()),
        }
    }

    #[test]
    fn test_o_toggles_show_optional_when_filter_active() {
        let form = two_field_form().with_optional_filter(true);
        assert!(!form.show_optional);
        let mut phase = FormPhase::new(form);
        let step = phase.on_key(key(KeyCode::Char('o')));
        assert!(matches!(step, PhaseStep::Continue));
        assert!(phase.form().show_optional, "o reveals optional rows");
        phase.on_key(key(KeyCode::Char('o')));
        assert!(!phase.form().show_optional, "o hides them again");
    }

    #[test]
    fn test_o_is_typed_into_buffer_during_edit() {
        let mut phase = FormPhase::new(two_field_form().with_optional_filter(true));
        phase.on_key(key(KeyCode::Enter)); // begin_edit on focus 0
        phase.on_key(key(KeyCode::Char('o')));
        assert_eq!(
            phase.form().editing.as_ref().unwrap().text_buffer(),
            Some("o")
        );
        assert!(
            !phase.form().show_optional,
            "o does not toggle while editing"
        );
    }

    #[test]
    fn test_collapsing_renormalizes_focus_off_hidden_row() {
        // field0 required+empty (visible), field1 optional+empty (hidden when
        // collapsed). Expand, focus the optional row, then collapse with `o`.
        let mut form = two_field_form().with_optional_filter(true);
        form.show_optional = true;
        form.focus = 1; // the optional row
        let mut phase = FormPhase::new(form);
        phase.on_key(key(KeyCode::Char('o'))); // collapse
        assert!(!phase.form().show_optional);
        assert_eq!(phase.form().focus, 0, "focus snaps off the now-hidden row");
    }

    // PhaseStep is a generic enum without Debug — give the panic message
    // path a stable label without forcing a Debug bound on Self::Output.
    impl<O> PhaseStep<O> {
        fn kind(&self) -> &'static str {
            match self {
                PhaseStep::Continue => "Continue",
                PhaseStep::Done(_) => "Done",
                PhaseStep::Cancelled => "Cancelled",
            }
        }
    }
}
