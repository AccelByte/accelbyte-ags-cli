//! Inline form widget.
//!
//! Renders a list of fields with focus management, hint pane below,
//! and Confirm button. Used by both the inline base surface
//! and the fullscreen workflow Fields panel.

use ags_protocol::workflow::{GatherResult, GatherSlotId, StepFieldId};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

/// Routes a field's value to the correct `GatherResult` channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKey {
    /// A missing input being gathered → GatherResult.slot_values.
    Slot(GatherSlotId),
    /// A pre-filled workflow input (flag/default), editable →
    /// GatherResult.input_overrides by name.
    Input(String),
    /// Per-step review field, identified by its plan id → StepFieldEdits by id.
    Review(StepFieldId),
}

#[derive(Debug, Clone)]
pub struct FormField {
    pub label: String,
    pub field_type: FieldType,
    pub required: bool,
    pub value: FieldValue,
    pub description: String,
    pub source: FieldSource,
    pub key: FieldKey,
    pub schema: serde_json::Value,
    /// When true, the field is shown for context but cannot be focused-for-edit
    /// (a Phase-2 input/prior-output value, fixed earlier in the run).
    pub read_only: bool,
    /// Dynamic-enum state when `field_type == FieldType::DynamicEnum`; `None`
    /// for every other field type.
    pub dynamic: Option<DynamicEnumState>,
    /// File-picker spec when `field_type == FieldType::FilePicker`; `None`
    /// for every other field type.
    pub file_picker: Option<ags_protocol::workflow::FilePickerSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldType {
    Scalar,
    Enum {
        variants: Vec<String>,
    },
    Bool,
    JsonBody,
    /// String input whose choices are fetched at runtime (fullscreen Phase-1
    /// only). The selected/typed value is held in `FieldValue::Enum(Some(..))`;
    /// the source + dependency names + resolved-choices cache live in
    /// `FormField.dynamic`.
    DynamicEnum,
    /// A declared workflow input with schema `format: "date-time"`. Edited via
    /// the segmented `date_field` widget; the carrier is `FieldValue::Scalar`
    /// holding the ISO string. Activated only in `build_inputs_form`.
    DateTime,
    /// A declared workflow input with a `file_picker` spec (fullscreen Phase-1
    /// only). The selected absolute path is held in `FieldValue::Enum(Some(..))`
    /// — the same carrier shape as `DynamicEnum` — with the extension filter +
    /// starting directory living in `FormField.file_picker`.
    FilePicker,
}

#[derive(Debug, Clone)]
pub enum FieldValue {
    Scalar(String),
    Enum(Option<String>),
    Bool(Option<bool>),
    JsonBody(String),
    Empty,
}

/// Out-of-band per-field state for a `DynamicEnum`. Kept off `FieldType` so the
/// latter stays `Eq`, and off `FieldValue` so projection/coercion are unchanged.
#[derive(Debug, Clone)]
pub struct DynamicEnumState {
    /// How to fetch the choices.
    pub source: ags_protocol::workflow::OptionsSource,
    /// Workflow input names this field depends on (derived from the source's
    /// `FromInput` parameters). Drives the "fill `<dep>` first" hint + cache key.
    pub deps: Vec<String>,
    /// Non-gating dependencies (from `FromInputOptional`). They join the cache
    /// key and the fetch, but never block opening; when any is empty the picker
    /// opens in direct-entry mode with no fetch.
    pub optional_deps: Vec<String>,
    /// `None` = never resolved. `Some` = an `Ok` result cached for the
    /// dependency values it carries (choices possibly empty). Failures/cancels
    /// never populate this.
    pub resolved: Option<ResolvedChoices>,
}

/// A cached successful resolution, keyed by the dependency values it was fetched
/// against (compared by `serde_json::Value` equality).
#[derive(Debug, Clone)]
pub struct ResolvedChoices {
    pub dep_key: std::collections::BTreeMap<String, serde_json::Value>,
    pub choices: Vec<ags_protocol::workflow::OptionChoice>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldSource {
    FromFlag,
    UserInput,
    Default,
    // Constructed only in tests today; the production render path matches against it.
    #[allow(dead_code)]
    Optional,
    /// Review: value from an explicitly-provided workflow input.
    WorkflowInput,
    /// Review: hard-coded literal in the workflow definition.
    Literal,
    /// Review: value captured from a previous step.
    PriorOutput,
    /// Review: value computed from one or more workflow inputs (format template
    /// or arithmetic expression). Read-only; `sources` names the inputs.
    Derived {
        sources: Vec<String>,
    },
}

/// The choice mapped from the focused button in the confirm/skip group
/// (`confirm_skip_buttons`). Read after `PhaseResult::Submitted` to decide
/// whether to proceed or skip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmSkipChoice {
    Confirm,
    Skip,
}

#[derive(Clone)]
pub struct Form {
    pub title: String,
    pub fields: Vec<FormField>,
    pub focus: usize,
    pub editing: Option<EditState>,
    /// When true, the focus ring gains a trailing Submit slot (the
    /// Confirm button). Default false → inline behaviour
    /// unchanged. Fullscreen opts in.
    pub submit_focusable: bool,
    /// Set when a submit is blocked by an unfilled required field; rendered
    /// (red) in the hint area. Cleared on the next edit/focus change. Only
    /// ever set on the fullscreen (submit_focusable) path.
    pub validation_note: Option<String>,
    /// Description shown in the hint slot when the Submit row is focused.
    /// Empty by default; builders (gather-inputs, review_step) set it.
    pub submit_description: String,
    /// When true, the submit slot renders the three run-mode buttons
    /// (`[ Run ] [ Run & Review ] [ Run & Accept Defaults ]`) instead of the
    /// single `[ Confirm ]`. Only the run-start gather sets this;
    /// per-step gather/review keep the single button. Default false.
    pub run_mode_buttons: bool,
    /// Which run-mode button is focused (0..3) when `run_mode_buttons`. Default
    /// 0 → `RunMode::ReviewInputSteps`.
    pub run_mode_focus: usize,
    /// When true, the submit slot renders `[ Confirm ] [ Skip ]` instead of the
    /// single `[ Confirm ]`. Set only for optional-step review forms. Default false.
    pub confirm_skip_buttons: bool,
    /// Which confirm/skip button is focused (0=Confirm, 1=Skip) when
    /// `confirm_skip_buttons`. Default 0 → Confirm.
    pub confirm_skip_focus: usize,
    /// When true, this form distinguishes optional-empty rows for the advanced
    /// toggle (inline single-command form only). Default false → all rows shown.
    pub optional_filter: bool,
    /// Advanced-toggle state. Only meaningful when `optional_filter`. When false
    /// (and filtering active) optional-empty rows are hidden.
    pub show_optional: bool,
    /// When true, required fields render a trailing `*` marker (inline single-
    /// command form only; the workflow-review path leaves this false).
    // Read by the render path; set by `with_mark_required`.
    pub mark_required: bool,
    /// Title for the inline `Parameters` box. `None` → the default `Parameters`;
    /// the per-step review form sets it to the step name so the box reflects the
    /// current step. Ignored by the fullscreen surface (it renders its own step
    /// header box).
    pub box_title: Option<String>,
    /// Scroll offset (index into the visible-row list) of the top rendered field.
    /// Updated during render to keep focus visible with minimal scrolling, so the
    /// window only moves when focus leaves it. `Cell` for interior mutability
    /// through the `&self` render path.
    pub(crate) scroll_top: std::cell::Cell<usize>,
}

/// What kind of in-place edit is open. `Text` is the classic character buffer;
/// `Date` is the segmented date-time editor (see `date_field`). Must stay
/// `Clone` — `Form` derives `Clone` and holds `Option<EditState>`.
#[derive(Debug, Clone)]
pub(crate) enum EditKind {
    Text(String),
    Date(crate::frontend::terminal::date_field::DateEditState),
}

#[derive(Debug, Clone)]
pub struct EditState {
    pub(crate) kind: EditKind,
    pub field_index: usize,
}

impl EditState {
    pub fn text_buffer(&self) -> Option<&str> {
        match &self.kind {
            EditKind::Text(s) => Some(s.as_str()),
            EditKind::Date(_) => None,
        }
    }
    pub fn text_buffer_mut(&mut self) -> Option<&mut String> {
        match &mut self.kind {
            EditKind::Text(s) => Some(s),
            EditKind::Date(_) => None,
        }
    }
    pub(crate) fn date(&self) -> Option<&crate::frontend::terminal::date_field::DateEditState> {
        match &self.kind {
            EditKind::Date(d) => Some(d),
            EditKind::Text(_) => None,
        }
    }
    pub(crate) fn date_mut(
        &mut self,
    ) -> Option<&mut crate::frontend::terminal::date_field::DateEditState> {
        match &mut self.kind {
            EditKind::Date(d) => Some(d),
            EditKind::Text(_) => None,
        }
    }
    pub fn is_date(&self) -> bool {
        matches!(self.kind, EditKind::Date(_))
    }
}

impl Form {
    /// Build a form with the given title over `fields`, focused on the first row.
    pub fn new(title: impl Into<String>, fields: Vec<FormField>) -> Self {
        Self {
            title: title.into(),
            fields,
            focus: 0,
            editing: None,
            submit_focusable: false,
            validation_note: None,
            submit_description: String::new(),
            run_mode_buttons: false,
            run_mode_focus: 0,
            confirm_skip_buttons: false,
            confirm_skip_focus: 0,
            optional_filter: false,
            show_optional: false,
            mark_required: false,
            box_title: None,
            scroll_top: std::cell::Cell::new(0),
        }
    }

    /// Builder: include the Submit target in the focus ring. The fullscreen
    /// gather form opts in; inline leaves it off (default).
    pub fn with_submit_focusable(mut self, yes: bool) -> Self {
        self.submit_focusable = yes;
        self
    }

    /// Builder: set the inline `Parameters` box title (e.g. the step name on the
    /// per-step review form). `None`/unset keeps the default `Parameters`.
    pub fn with_box_title(mut self, title: impl Into<String>) -> Self {
        self.box_title = Some(title.into());
        self
    }

    /// Builder: enable the optional-row filter (inline single-command form).
    pub fn with_optional_filter(mut self, yes: bool) -> Self {
        self.optional_filter = yes;
        self
    }

    /// Builder: render a `*` marker on required fields (inline single-command form).
    pub fn with_mark_required(mut self, yes: bool) -> Self {
        self.mark_required = yes;
        self
    }

    /// Builder: set the description that the hint slot shows when Submit is
    /// focused (the gather-inputs / per-step review panels set this).
    pub fn with_submit_description(mut self, text: impl Into<String>) -> Self {
        self.submit_description = text.into();
        self
    }

    /// Builder: render the three run-mode buttons on the submit row (run-start
    /// gather only). Per-step gather/review leave this off.
    pub fn with_run_mode_buttons(mut self, yes: bool) -> Self {
        self.run_mode_buttons = yes;
        self
    }

    /// Builder: render `[ Confirm ] [ Skip ]` on the submit row (optional-step
    /// review only). Non-optional review and all other forms leave this off.
    pub fn with_confirm_skip_buttons(mut self, yes: bool) -> Self {
        self.confirm_skip_buttons = yes;
        self
    }

    /// True when focus is on the trailing Submit slot.
    pub fn is_submit_focused(&self) -> bool {
        self.submit_focusable && self.focus == self.fields.len()
    }

    /// Move the run-mode button focus one to the right (saturating at the last
    /// button). No-op unless `run_mode_buttons`.
    pub fn run_mode_focus_next(&mut self) {
        if self.run_mode_buttons && self.run_mode_focus < 2 {
            self.run_mode_focus += 1;
        }
    }

    /// Move the run-mode button focus one to the left (saturating at the first
    /// button). No-op unless `run_mode_buttons`.
    pub fn run_mode_focus_prev(&mut self) {
        if self.run_mode_buttons && self.run_mode_focus > 0 {
            self.run_mode_focus -= 1;
        }
    }

    /// The run stop-mode the focused button maps to. Defaults to
    /// `ReviewInputSteps` (button 0).
    pub(crate) fn selected_run_mode(&self) -> ags_protocol::workflow::RunMode {
        use ags_protocol::workflow::RunMode;
        match self.run_mode_focus {
            0 => RunMode::ReviewInputSteps,
            1 => RunMode::ReviewEveryStep,
            _ => RunMode::RunWithoutStopping,
        }
    }

    /// One-line description of the focused run-mode button, shown in the hint
    /// pane while the button group is focused. Leads with the `←/→` affordance so
    /// the arrow-key navigation across the three buttons is discoverable (it is
    /// otherwise absent from the Fields nav bar).
    fn run_mode_description(&self) -> &'static str {
        match self.run_mode_focus {
            0 => "\u{2190}/\u{2192} to choose  \u{00B7}  Stops only on steps that need your input. Fully automatic steps run without a pause.",
            1 => "\u{2190}/\u{2192} to choose  \u{00B7}  Stops before every step so you can review it before it runs.",
            _ => "\u{2190}/\u{2192} to choose  \u{00B7}  Runs straight through with default values. Publish steps still ask you to confirm.",
        }
    }

    /// Move the confirm/skip button focus one to the right (saturating at Skip).
    /// No-op unless `confirm_skip_buttons`.
    pub fn confirm_skip_focus_next(&mut self) {
        if self.confirm_skip_buttons && self.confirm_skip_focus < 1 {
            self.confirm_skip_focus += 1;
        }
    }

    /// Move the confirm/skip button focus one to the left (saturating at Confirm).
    /// No-op unless `confirm_skip_buttons`.
    pub fn confirm_skip_focus_prev(&mut self) {
        if self.confirm_skip_buttons && self.confirm_skip_focus > 0 {
            self.confirm_skip_focus -= 1;
        }
    }

    /// The choice the focused confirm/skip button maps to. Defaults to `Confirm`
    /// (focus 0).
    pub fn selected_confirm_skip(&self) -> ConfirmSkipChoice {
        if self.confirm_skip_focus == 0 {
            ConfirmSkipChoice::Confirm
        } else {
            ConfirmSkipChoice::Skip
        }
    }

    /// One-line description of the focused confirm/skip button, shown in the hint
    /// pane while the button group is focused.
    fn confirm_skip_description(&self) -> &'static str {
        if self.confirm_skip_focus == 0 {
            "\u{2190}/\u{2192} to choose  \u{00B7}  Confirm this step and continue"
        } else {
            "\u{2190}/\u{2192} to choose  \u{00B7}  Skip this optional step and continue to the next"
        }
    }

    /// Open an inline editor on the currently focused field, seeding the
    /// buffer from the field's current value (empty for unset / non-text
    /// variants). No-op if no field is focused.
    pub fn begin_edit(&mut self) {
        self.validation_note = None;
        if let Some(field) = self.fields.get(self.focus) {
            if field.read_only {
                // Surface an explanatory message for Derived fields, naming the
                // workflow inputs that control this value, so the user knows
                // which inputs to change rather than seeing a silent no-op.
                if let FieldSource::Derived { sources } = &field.source {
                    self.validation_note = Some(derived_field_note(&field.label, sources));
                }
                return;
            }
            if matches!(field.field_type, FieldType::Enum { .. } | FieldType::Bool) {
                return;
            }
            let kind = if matches!(field.field_type, FieldType::DateTime) {
                match &field.value {
                    FieldValue::Scalar(s) => {
                        match crate::frontend::terminal::date_field::parse_iso(s) {
                            Some(parts) => EditKind::Date(
                                crate::frontend::terminal::date_field::DateEditState::new(parts),
                            ),
                            // Unparseable → plain-text fallback on the raw string.
                            None => EditKind::Text(s.clone()),
                        }
                    }
                    _ => EditKind::Text(String::new()),
                }
            } else {
                let buffer = match &field.value {
                    FieldValue::Scalar(s) | FieldValue::JsonBody(s) => s.clone(),
                    FieldValue::Enum(Some(s)) => s.clone(),
                    FieldValue::Bool(Some(b)) => b.to_string(),
                    _ => String::new(),
                };
                EditKind::Text(buffer)
            };
            self.editing = Some(EditState {
                kind,
                field_index: self.focus,
            });
        }
    }

    /// Discard the in-progress edit; the field keeps its previous value.
    pub fn cancel_edit(&mut self) {
        self.editing = None;
    }

    /// Whether an inline field editor is currently open. Callers that steal
    /// letter keys (e.g. the optional-step `s` skip intercept) must gate on
    /// this so typed characters reach the edit buffer instead.
    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    /// Whether every required field has a non-empty value matching its type.
    /// Drives whether the form's submit action is enabled.
    pub fn all_required_filled(&self) -> bool {
        self.fields
            .iter()
            .all(|f| !f.required || is_field_filled(f))
    }

    /// Index of the first required-but-unfilled field, if any.
    pub fn first_unfilled_required(&self) -> Option<usize> {
        self.fields
            .iter()
            .position(|f| f.required && !is_field_filled(f))
    }

    /// The authoritative gather projection.
    /// Project every field into a `GatherResult`, coerced per field schema.
    /// Slot-keyed fields go to `slot_values`; input-keyed fields go to
    /// `input_overrides`. Empty fields are skipped.
    pub fn project_gathered(&self) -> GatherResult {
        let mut result = GatherResult::default();
        for field in &self.fields {
            let Some(raw) = field_value_string(&field.value) else {
                continue;
            };
            let value = crate::frontend::coerce_to_schema(&raw, &field.schema);
            match &field.key {
                FieldKey::Slot(id) => {
                    result.slot_values.insert(*id, value);
                }
                FieldKey::Input(name) => {
                    result.input_overrides.insert(name.clone(), value);
                }
                // Review fields are projected via `project_step_edits`, not here.
                FieldKey::Review(_) => {}
            }
        }
        result
    }

    /// Build a review form from a step plan. Each field carries
    /// `FieldKey::Review(id)` so edits project back by id; the submit slot is
    /// focusable (the `[ Confirm ]` button advances the step). The
    /// field's `description` feeds the hint area; provenance feeds `source`
    /// (rendered as the row suffix), so the two never get crossed.
    ///
    /// Filtering rules applied here:
    /// - `body_overflow` synthetic fields are excluded entirely.
    /// - `Literal`-source fields are included only when `show_in_review = true`.
    /// - All other provenance-traced sources (WorkflowInput, Default, Derived,
    ///   PriorOutput, Unset) are always included.
    pub fn from_step_plan(plan: &ags_protocol::workflow::StepFieldPlan) -> Self {
        use crate::frontend::terminal::inline::form_builder::schema_to_field_type;
        use ags_protocol::workflow::StepFieldSource;

        let mut fields: Vec<FormField> = plan
            .fields
            .iter()
            .filter(|f| {
                // Drop the body-overflow synthetic row.
                if f.body_overflow {
                    return false;
                }
                // Literal-source fields are opt-in; all other sources always shown.
                if matches!(f.source, StepFieldSource::Literal) {
                    return f.show_in_review;
                }
                true
            })
            .map(|f| {
                // Build the FieldValue against the schema-derived field type
                // so the carrier variant matches what edit/toggle/cycle expect
                // — e.g. a bool field must hold `FieldValue::Bool`, not a
                // `Scalar("true")` string. Mismatched carriers make the first
                // space-press on a bool look like a no-op (toggle falls
                // through to `Bool(Some(true))`, identical to the rendered
                // "true" string), and force the same recovery hop for enums.
                let field_type = schema_to_field_type(&f.schema);
                let value = if f.value.is_null() {
                    FieldValue::Empty
                } else {
                    match (&field_type, &f.value) {
                        (FieldType::Bool, serde_json::Value::Bool(b)) => FieldValue::Bool(Some(*b)),
                        (FieldType::Enum { .. }, serde_json::Value::String(s)) => {
                            FieldValue::Enum(Some(s.clone()))
                        }
                        (FieldType::JsonBody, _)
                        | (_, serde_json::Value::Object(_))
                        | (_, serde_json::Value::Array(_)) => FieldValue::JsonBody(
                            serde_json::to_string_pretty(&f.value).unwrap_or_default(),
                        ),
                        (_, serde_json::Value::String(s)) => FieldValue::Scalar(s.clone()),
                        (_, other) => FieldValue::Scalar(other.to_string()),
                    }
                };
                let source = match &f.source {
                    StepFieldSource::WorkflowInput { .. } => FieldSource::WorkflowInput,
                    StepFieldSource::Default { .. } => FieldSource::Default,
                    StepFieldSource::Literal => FieldSource::Literal,
                    StepFieldSource::PriorOutput => FieldSource::PriorOutput,
                    StepFieldSource::Unset => FieldSource::UserInput,
                    StepFieldSource::Derived { sources } => FieldSource::Derived {
                        sources: sources.clone(),
                    },
                };
                let read_only = f.workflow_input.is_some()
                    || matches!(
                        f.source,
                        StepFieldSource::PriorOutput | StepFieldSource::Derived { .. }
                    );
                FormField {
                    label: f.label.clone(),
                    field_type,
                    required: f.required,
                    value,
                    description: f.description.clone().unwrap_or_default(),
                    source,
                    key: FieldKey::Review(f.id),
                    schema: f.schema.clone(),
                    read_only,
                    dynamic: None,
                    file_picker: None,
                }
            })
            .collect();
        // Group read-only inputs (workflow values / derived / prior-output)
        // ahead of the editable step fields, so the inline review — which renders
        // fields in order — doesn't interleave them. The fullscreen surface
        // re-partitions by source for its sectioned layout, so the order it sees
        // here is irrelevant; only the inline ordering changes. A stable sort
        // preserves each group's original order. Edits project by id
        // (`project_step_edits`), so reordering is safe.
        fields.sort_by_key(|f: &FormField| !f.read_only);
        // 1-based step number: Phase-1 input collection is "Step 0", so the first
        // workflow step (index 0) reads "Step 1".
        Form::new(plan.step_label.clone(), fields)
            .with_submit_focusable(true)
            .with_box_title(format!("Step {}: {}", plan.step_index + 1, plan.step_label))
    }

    /// Project changed fields into `StepFieldEdits`, keyed by `StepFieldId`.
    /// Only fields whose coerced value differs from the plan's resolved value
    /// are emitted.
    pub fn project_step_edits(
        &self,
        plan: &ags_protocol::workflow::StepFieldPlan,
    ) -> ags_protocol::workflow::StepFieldEdits {
        // Match each form field to its plan field BY ID, not by position: the
        // form is a filtered (and, on the inline surface, reordered) subset of
        // the plan, so a positional zip would compare against the wrong
        // baseline. Each editable field carries `FieldKey::Review(id)`.
        let plan_by_id: std::collections::BTreeMap<_, _> =
            plan.fields.iter().map(|f| (f.id, f)).collect();
        let mut edits = ags_protocol::workflow::StepFieldEdits::default();
        for form_field in &self.fields {
            if form_field.read_only {
                continue;
            }
            let FieldKey::Review(id) = &form_field.key else {
                continue;
            };
            let Some(plan_field) = plan_by_id.get(id) else {
                continue;
            };
            let Some(raw) = field_value_string(&form_field.value) else {
                continue;
            };
            let new_value = crate::frontend::coerce_to_schema(&raw, &form_field.schema);
            if new_value != plan_field.value {
                edits.values.insert(*id, new_value);
            }
        }
        edits
    }

    /// Write the in-progress buffer back to the field, coerced to the
    /// field type. Marks the field as `UserInput`. No-op if no edit
    /// is in progress.
    pub fn commit_edit(&mut self) {
        self.validation_note = None;
        if let Some(state) = self.editing.take() {
            if let Some(field) = self.fields.get_mut(state.field_index) {
                field.value = match state.kind {
                    EditKind::Date(d) => FieldValue::Scalar(d.to_iso()),
                    EditKind::Text(buffer) => match &field.field_type {
                        // A DateTime whose value did not parse committed as Text:
                        // write the raw buffer straight through, never via to_iso.
                        FieldType::Scalar | FieldType::DateTime => FieldValue::Scalar(buffer),
                        FieldType::Enum { .. } => FieldValue::Enum(Some(buffer)),
                        FieldType::DynamicEnum | FieldType::FilePicker => {
                            FieldValue::Enum(Some(buffer))
                        }
                        FieldType::Bool => FieldValue::Bool(buffer.parse().ok()),
                        FieldType::JsonBody => FieldValue::JsonBody(buffer),
                    },
                };
                field.source = FieldSource::UserInput;
            }
        }
    }

    /// Cycle the focused field's enum value to the next (or previous) variant,
    /// wrapping at the ends. No-op if the focused field isn't an enum, has no
    /// variants, or is read-only.
    pub fn cycle_focused_enum(&mut self, forward: bool) {
        self.validation_note = None;
        let Some(field) = self.fields.get_mut(self.focus) else {
            return;
        };
        if field.read_only {
            return;
        }
        let FieldType::Enum { variants } = &field.field_type else {
            return;
        };
        if variants.is_empty() {
            return;
        }
        let current_idx = match &field.value {
            FieldValue::Enum(Some(s)) => variants.iter().position(|v| v == s),
            _ => None,
        };
        let next_idx = match (current_idx, forward) {
            (Some(i), true) => (i + 1) % variants.len(),
            (Some(i), false) => (i + variants.len() - 1) % variants.len(),
            (None, true) => 0,
            (None, false) => variants.len() - 1,
        };
        field.value = FieldValue::Enum(Some(variants[next_idx].clone()));
        field.source = FieldSource::UserInput;
    }

    /// Toggle the focused field's bool value. Unset becomes `true`; `true`
    /// becomes `false`; `false` becomes `true`. No-op if the focused field
    /// isn't a bool or is read-only.
    pub fn toggle_focused_bool(&mut self) {
        self.validation_note = None;
        let Some(field) = self.fields.get_mut(self.focus) else {
            return;
        };
        if field.read_only {
            return;
        }
        if !matches!(field.field_type, FieldType::Bool) {
            return;
        }
        field.value = match field.value {
            FieldValue::Bool(Some(b)) => FieldValue::Bool(Some(!b)),
            _ => FieldValue::Bool(Some(true)),
        };
        field.source = FieldSource::UserInput;
    }

    /// Field indices in the same order the renderer displays them: workflow
    /// values, then step values. The Submit slot (when `submit_focusable`) sits
    /// conceptually at the end (represented as `self.fields.len()` by callers).
    fn visual_order(&self) -> Vec<usize> {
        let groups = partition_fields_by_source(&self.fields);
        let mut order = Vec::with_capacity(groups.workflow_values.len() + groups.step_values.len());
        order.extend(groups.workflow_values);
        order.extend(groups.step_values);
        order.retain(|&i| self.is_row_visible(i));
        order
    }

    /// Whether row `idx` is currently visible given the optional filter + toggle.
    /// A row is visible if filtering is off, the toggle is on, or the field is
    /// required or already filled.
    pub(crate) fn is_row_visible(&self, idx: usize) -> bool {
        let Some(f) = self.fields.get(idx) else {
            return false;
        };
        if !self.optional_filter || self.show_optional {
            return true;
        }
        f.required || is_field_filled(f)
    }

    /// Count of optional rows that are currently hidden when collapsed (drives
    /// the `[o] show optional (+N)` affordance rendered by `views::fields::render_inline`).
    /// Zero when filtering is off.
    pub(crate) fn optional_empty_count(&self) -> usize {
        if !self.optional_filter {
            return 0;
        }
        self.fields
            .iter()
            .filter(|f| !f.required && !is_field_filled(f))
            .count()
    }

    /// Ensure `focus` points at a currently-visible row (or the submit slot).
    /// No-op when filtering is off or expanded (all rows visible). When the
    /// focused row is hidden, snaps to the nearest following visible row, else
    /// the last visible row; when no row is visible, the submit slot (if
    /// `submit_focusable`), else index 0. Must be called whenever the visible
    /// set changes (form build, filter enable, expanded→collapsed).
    pub(crate) fn normalize_focus_to_visible(&mut self) {
        if !self.optional_filter || self.show_optional || self.is_submit_focused() {
            return;
        }
        if self.fields.get(self.focus).is_some() && self.is_row_visible(self.focus) {
            return;
        }
        let visible = self.visual_order(); // already filtered to visible rows
        if visible.is_empty() {
            self.focus = if self.submit_focusable {
                self.fields.len()
            } else {
                0
            };
            return;
        }
        self.focus = visible
            .iter()
            .copied()
            .find(|&i| i >= self.focus)
            .unwrap_or_else(|| *visible.last().unwrap());
    }

    /// Advance focus to the next field in visual order, wrapping through the
    /// Submit slot. Read-only fields stay in the ring so they can be inspected.
    pub fn focus_next(&mut self) {
        self.validation_note = None;
        let visual = self.visual_order();
        let total = visual.len() + usize::from(self.submit_focusable);
        if total == 0 {
            return;
        }
        // Current position in the visual chain.
        let current_pos = if self.is_submit_focused() {
            visual.len()
        } else {
            visual.iter().position(|&i| i == self.focus).unwrap_or(0)
        };
        // Advance one step — read-only fields are included in the ring so the
        // user can Tab to them and see their description in the hint area.
        let next_pos = (current_pos + 1) % total;
        if next_pos == visual.len() {
            // Submit slot.
            self.focus = self.fields.len();
        } else {
            self.focus = visual[next_pos];
        }
    }

    /// Step focus back to the previous field in visual order, wrapping through
    /// the Submit slot.
    pub fn focus_prev(&mut self) {
        self.validation_note = None;
        let visual = self.visual_order();
        let total = visual.len() + usize::from(self.submit_focusable);
        if total == 0 {
            return;
        }
        let current_pos = if self.is_submit_focused() {
            visual.len()
        } else {
            visual.iter().position(|&i| i == self.focus).unwrap_or(0)
        };
        // Step back one — read-only fields are included in the ring so the
        // user can Tab to them and see their description in the hint area.
        let prev_pos = (current_pos + total - 1) % total;
        if prev_pos == visual.len() {
            self.focus = self.fields.len();
        } else {
            self.focus = visual[prev_pos];
        }
    }

    /// Normalise the initial focus onto the first editable field, or onto the
    /// Move focus to the trailing Submit slot when the form is
    /// `submit_focusable`. The step-walk uses this so the most common
    /// action (submit and continue) is selected on phase entry; falls back
    /// to `focus_first_editable()` semantics when submit isn't focusable.
    pub fn focus_submit_if_available(&mut self) {
        if self.submit_focusable {
            self.focus = self.fields.len();
        } else {
            self.focus_first_editable();
        }
    }

    /// Submit slot if every field is read-only (so the form never starts on a
    /// read-only row, where Enter could otherwise submit immediately). Call
    /// after building a form whose first fields may be read-only.
    pub fn focus_first_editable(&mut self) {
        let visual = self.visual_order();
        if let Some(&i) = visual.iter().find(|&&i| !self.fields[i].read_only) {
            self.focus = i;
        } else if self.submit_focusable {
            self.focus = self.fields.len();
        } else {
            self.focus = 0;
        }
    }

    /// The currently focused field, or `None` when focus is on the Submit slot.
    pub fn focused(&self) -> Option<&FormField> {
        self.fields.get(self.focus)
    }

    /// Draw the form — field rows, hint pane, and Submit button — into `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // fields
                Constraint::Length(4), // hint pane
                Constraint::Length(3), // confirm button
            ])
            .split(area);

        self.render_fields(frame, chunks[0]);
        self.render_hint(frame, chunks[1]);
        self.render_submit(frame, chunks[2], self.is_submit_focused());
    }

    /// Draw the bordered field-rows box into `area`, skipping rows hidden by the
    /// optional-row filter.
    pub(crate) fn render_fields(&self, frame: &mut Frame, area: Rect) {
        // Skip rows hidden by the optional-row filter so this monolithic render
        // path stays consistent with `views::fields::render_inline` (no-op for
        // forms without `optional_filter`, where every row is visible).
        let lines: Vec<Line> = self
            .fields
            .iter()
            .enumerate()
            .filter(|(i, _)| self.is_row_visible(*i))
            .map(|(i, f)| {
                let editing_buffer = self
                    .editing
                    .as_ref()
                    .filter(|s| s.field_index == i)
                    .and_then(|s| s.text_buffer());
                field_line(f, i == self.focus, editing_buffer)
            })
            .collect();
        let para = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(self.title.as_str()),
        );
        frame.render_widget(para, area);
    }

    /// Draw the hint pane into `area`: the validation note in red, or the
    /// focused field's description otherwise. When a button group is focused,
    /// show the focused button's description instead.
    fn render_hint(&self, frame: &mut Frame, area: Rect) {
        let (text, color) = match &self.validation_note {
            Some(note) => (note.clone(), Color::Red),
            None if self.run_mode_buttons && self.is_submit_focused() => {
                (self.run_mode_description().to_string(), Color::Indexed(244))
            }
            None if self.confirm_skip_buttons && self.is_submit_focused() => (
                self.confirm_skip_description().to_string(),
                Color::Indexed(244),
            ),
            None => (
                self.focused()
                    .map(|f| f.description.clone())
                    .unwrap_or_default(),
                Color::Indexed(244),
            ),
        };
        let base = Style::default().fg(color).add_modifier(Modifier::ITALIC);
        let para = Paragraph::new(text)
            .style(base)
            .block(Block::default().borders(Borders::ALL).title("hint"));
        frame.render_widget(para, area);
    }

    /// Draw the submit control into `area`: the three-button run-mode group when
    /// `run_mode_buttons` is set, the two-button confirm/skip group when
    /// `confirm_skip_buttons` is set, or the single Confirm button otherwise.
    pub(crate) fn render_submit(&self, frame: &mut Frame, area: Rect, focused: bool) {
        if self.run_mode_buttons {
            self.render_run_mode_buttons(frame, area, focused);
        } else if self.confirm_skip_buttons {
            self.render_confirm_skip_buttons(frame, area, focused);
        } else {
            self.render_button(frame, area, focused);
        }
    }

    /// Draw the Submit button into `area`, highlighted when `focused`.
    pub(crate) fn render_button(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let style = if focused {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        };
        let label = Span::styled("[ Confirm ]", style);
        frame.render_widget(Paragraph::new(Line::from(label)), area);
    }

    /// Draw the three run-mode buttons on one row, the focused one highlighted.
    /// The focused button's description drives the hint pane (see
    /// [`render_hint`](Self::render_hint)). Only rendered on the run-start gather
    /// (`run_mode_buttons`); `group_focused` is true when focus is on the submit
    /// slot, so the buttons only highlight while the group holds focus.
    fn render_run_mode_buttons(&self, frame: &mut Frame, area: Rect, group_focused: bool) {
        let labels = ["Run", "Run & Review", "Run & Accept Defaults"];
        let spans = button_group_spans(&labels, self.run_mode_focus, group_focused);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// Draw `[ Confirm ] [ Skip ]` on the submit row, the focused button
    /// highlighted. Only rendered on optional-step review forms
    /// (`confirm_skip_buttons`); `group_focused` is true when focus is on the
    /// submit slot.
    fn render_confirm_skip_buttons(&self, frame: &mut Frame, area: Rect, group_focused: bool) {
        let labels = ["Confirm", "Skip"];
        let spans = button_group_spans(&labels, self.confirm_skip_focus, group_focused);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// Render a single field row at `index` into `area` — one line, highlighted
    /// when focused. The fullscreen Fields panel lays fields out row-by-row so
    /// it can interleave the hint directly under the focused field.
    ///
    /// `label_width` is the longest label across the panel's fields; labels are
    /// rendered as `Name:` left-padded to that width so the value columns line
    /// up. (Inline's [`field_line`] keeps its own fixed-width layout.)
    pub(crate) fn render_field_row(
        &self,
        frame: &mut Frame,
        area: Rect,
        index: usize,
        label_width: usize,
    ) {
        let Some(field) = self.fields.get(index) else {
            return;
        };
        let focused = index == self.focus;
        let date_editing = self
            .editing
            .as_ref()
            .filter(|s| s.field_index == index)
            .and_then(|s| s.date())
            .map(|d| d.render_editing());
        let editing_buffer = self
            .editing
            .as_ref()
            .filter(|s| s.field_index == index)
            .and_then(|s| s.text_buffer())
            .map(|s| s.to_string());

        // Read-only rows are secondary information (auto-bound refs, captured
        // values), so they render dimmed to let the editable inputs stand out.
        let dim = Style::default().fg(Color::Indexed(244));
        let label_style = if focused {
            Style::default()
                .bg(Color::Indexed(240))
                .add_modifier(Modifier::BOLD)
        } else if field.read_only {
            dim
        } else {
            Style::default()
        };
        // Label cell: focused rows lead with a Cyan caret. A dim `*` marks
        // required fields ONLY when `mark_required` is set (the inline single-
        // command form); the workflow-review path leaves it off, since by the
        // time fields reach review gather has already enforced required-ness.
        let mut spans = Vec::new();
        if focused {
            spans.push(Span::styled(
                "\u{25B8} ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(field.label.clone(), label_style));
        spans.push(Span::styled(":", label_style));
        // Required marker (inline single-command form, advanced mode only): an
        // orange `*` immediately AFTER the colon, in place of the usual trailing
        // space. Optional rows keep the normal `: ` separator, so no extra indent
        // is introduced for them and the value column stays aligned for both.
        let show_marker = self.mark_required && self.show_optional && field.required;
        if show_marker {
            spans.push(Span::styled("*", Style::default().fg(Color::Indexed(208))));
        } else {
            spans.push(Span::styled(" ", label_style));
        }
        // Pad so every value starts at the same column with at least
        // MIN_VALUE_GAP cells of breathing room.
        const MIN_VALUE_GAP: usize = 4;
        // caret-or-pad (2) + name + colon + (marker-or-space) (2).
        let used = 2 + field.label.chars().count() + 2;
        // Target column for the value: longest label's used cells + MIN_VALUE_GAP.
        // All rows pad to this column so values align.
        let target = 2 + label_width + 2 + MIN_VALUE_GAP;
        let pad = target.saturating_sub(used);
        spans.push(Span::raw(" ".repeat(pad)));
        let bracket_style = Style::default().fg(Color::Indexed(244));
        if let Some(date_line) = date_editing {
            spans.push(Span::raw(date_line));
        } else if let Some(buffer) = editing_buffer {
            spans.push(Span::styled("[", bracket_style));
            spans.push(Span::raw(format!("{buffer}\u{2588}")));
            spans.push(Span::styled("]", bracket_style));
        } else {
            let value_text = if field.field_type == FieldType::DynamicEnum {
                dynamic_enum_display(field)
            } else if field.field_type == FieldType::FilePicker {
                file_picker_display(field)
            } else if field.field_type == FieldType::DateTime {
                match &field.value {
                    FieldValue::Scalar(s) => {
                        crate::frontend::terminal::date_field::friendly_display(s)
                    }
                    _ => String::new(),
                }
            } else {
                render_value(&field.value)
            };
            // Trailing suffixes (dynamic-enum affordance, derived-from hint),
            // computed up front so their width is reserved before the value is
            // truncated to fit the row.
            let dyn_suffix = if field.field_type == FieldType::DynamicEnum {
                dynamic_enum_affordance(field)
            } else if field.field_type == FieldType::FilePicker {
                file_picker_affordance(field)
            } else {
                None
            };
            let derived_suffix = if let FieldSource::Derived { sources } = &field.source {
                Some(derived_from_suffix(sources))
            } else {
                None
            };
            let suffix_w = dyn_suffix.as_deref().map_or(0, str_cells)
                + derived_suffix.as_deref().map_or(0, str_cells);

            // Keep the whole line within `area.width` so a long value trails off
            // with an ellipsis instead of running under the scrollbar. The caller
            // narrows `area` to reserve the scrollbar column + a one-space gap.
            let budget = area.width as usize;
            let prefix_w = used + pad;

            // Bracket scalar-like editable values (`[value]`). JsonBody values
            // already start with `{`/`[`, so they render unbracketed and unpadded
            // — the JSON's own opening brace/bracket lines up under the `[` of
            // bracketed rows. Read-only values get a single leading space so
            // their first visible character still lines up with `[`.
            let editable_scalar_like = !field.read_only
                && matches!(
                    field.field_type,
                    FieldType::Scalar
                        | FieldType::Enum { .. }
                        | FieldType::Bool
                        | FieldType::DynamicEnum
                        | FieldType::DateTime
                        | FieldType::FilePicker
                );
            if editable_scalar_like {
                // Frame is `[` + value + `]` (2 cells).
                let avail = budget.saturating_sub(prefix_w + 2 + suffix_w);
                spans.push(Span::styled("[", bracket_style));
                spans.push(Span::raw(truncate_cells(&value_text, avail)));
                spans.push(Span::styled("]", bracket_style));
            } else if !field.read_only && field.field_type == FieldType::JsonBody {
                // Editable JSON body field (inline single-command form OR the
                // fullscreen gather form): frame the JSON value in a persistent
                // dim delimiter (like the scalar's dim `[ ]`), with the value
                // itself rendered normally. The frame matches the field's JSON
                // type — `{ }` for an object, `[ ]` for an array — and folds the
                // value's own outer delimiter into the frame so it isn't doubled:
                // an object `{…}` shows as `{…}`, an array `[…]` as `[…]`, never
                // `{[…]}`. The schema type drives the frame so an EMPTY array
                // reads `[]` and an empty object `{}` (not both `{}`); the value
                // text is only a fallback when the schema carries no `type`.
                let is_array = match field.schema.get("type").and_then(|t| t.as_str()) {
                    Some("array") => true,
                    Some("object") => false,
                    _ => value_text.trim_start().starts_with('['),
                };
                let (open, close) = if is_array { ('[', ']') } else { ('{', '}') };
                // An empty body summarises to `{}` regardless of type, so a blank
                // or empty-container value has no inner content — the frame alone
                // reads `[]` or `{}`. Otherwise fold the value's own outer
                // delimiter into the frame so it isn't doubled.
                let is_empty = matches!(&field.value, FieldValue::JsonBody(s) if {
                    let t = s.trim();
                    t.is_empty() || t == "{}" || t == "[]"
                });
                let mut inner: &str = if is_empty { "" } else { &value_text };
                if let Some(rest) = inner.strip_prefix(open) {
                    inner = rest;
                }
                if let Some(rest) = inner.strip_suffix(close) {
                    inner = rest;
                }
                // Frame is `<open>` + value + `<close>` (2 cells); the closing
                // delimiter is kept after the ellipsis, e.g. `{"sched":"value", …}`.
                let avail = budget.saturating_sub(prefix_w + 2 + suffix_w);
                spans.push(Span::styled(open.to_string(), bracket_style));
                spans.push(Span::raw(truncate_cells(inner, avail)));
                spans.push(Span::styled(close.to_string(), bracket_style));
            } else {
                // Review path: JsonBody and read-only values get a single leading
                // space so their first visible character sits one column right of
                // the editable `[`, matching read-only scalar content (unchanged).
                // Read-only values render dimmed (secondary info); focused rows
                // keep normal contrast so the caret target stays legible.
                let value_style = if field.read_only && !focused {
                    dim
                } else {
                    Style::default()
                };
                let avail = budget.saturating_sub(prefix_w + 1 + suffix_w);
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    truncate_cells(&value_text, avail),
                    value_style,
                ));
            }
            if let Some(suffix) = dyn_suffix {
                spans.push(Span::styled(
                    suffix,
                    Style::default().fg(Color::Indexed(244)),
                ));
            }
            if let Some(hint_suffix) = derived_suffix {
                spans.push(Span::styled(
                    hint_suffix,
                    Style::default().fg(Color::Indexed(244)),
                ));
            }
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// Project the Phase-1 inputs form into an authoritative declared-input map:
    /// every filled `Input` field contributes `name -> coerced value`; a cleared
    /// (empty) field is omitted, representing a genuine "unset". Required fields
    /// can't be empty (submit is blocked by `first_unfilled_required`).
    pub fn project_inputs(&self) -> std::collections::BTreeMap<String, serde_json::Value> {
        let mut map = std::collections::BTreeMap::new();
        for field in &self.fields {
            let FieldKey::Input(name) = &field.key else {
                continue;
            };
            let Some(raw) = field_value_string(&field.value) else {
                continue;
            };
            map.insert(
                name.clone(),
                crate::frontend::coerce_to_schema(&raw, &field.schema),
            );
        }
        map
    }

    /// The longest field label (character count) — drives value-column
    /// alignment in [`render_field_row`].
    pub(crate) fn label_width(&self) -> usize {
        self.fields
            .iter()
            .map(|f| f.label.chars().count())
            .max()
            .unwrap_or(0)
    }

    /// The hint to show for the current focus: the red validation note if a
    /// submit was blocked, else the focused field's description. `None` when
    /// there is nothing to show (e.g. the Submit slot is focused, or the field
    /// has no description). The bool is `true` for an error (red) note.
    pub(crate) fn current_hint(&self) -> Option<(String, bool)> {
        if let Some(note) = &self.validation_note {
            return Some((note.clone(), true));
        }
        if self.is_submit_focused() {
            if self.run_mode_buttons {
                return Some((self.run_mode_description().to_string(), false));
            }
            if self.confirm_skip_buttons {
                return Some((self.confirm_skip_description().to_string(), false));
            }
            if self.submit_description.is_empty() {
                return None;
            }
            return Some((self.submit_description.clone(), false));
        }
        let desc = self.focused().map(|f| f.description.clone())?;
        if desc.is_empty() {
            None
        } else {
            // The hint is a field label, not a sentence — strip a
            // single trailing full stop so descriptions sourced from OpenAPI
            // parameter docs read consistently (project convention: no full
            // stops on labels). Mirrors the plain surface's `emit_description_hint`.
            let label = desc.strip_suffix('.').unwrap_or(&desc).to_string();
            Some((label, false))
        }
    }
}

/// Build the styled spans for a button group (e.g. `[ Confirm ] [ Skip ]`).
///
/// `labels` are the button display names (without brackets). The button at
/// `focus_idx` renders filled (black on cyan, bold) when `group_focused`; all
/// others render in dim cyan. The separator between buttons is three spaces,
/// matching the run-mode button group. Shared by the confirm-card and form
/// submit renderers so the styling stays identical in both.
pub(crate) fn button_group_spans(
    labels: &[&str],
    focus_idx: usize,
    group_focused: bool,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        let focused = group_focused && i == focus_idx;
        let style = if focused {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };
        if i != 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(format!("[ {label} ]"), style));
    }
    spans
}

/// Two ordered groups of field indices for the section-headered renderer.
/// Empty sections are skipped by the renderer.
#[derive(Debug, Default, Clone)]
pub(crate) struct FieldGroups {
    pub workflow_values: Vec<usize>,
    pub step_values: Vec<usize>,
}

/// Partition a form's fields by source for grouped rendering.
///
/// - **Workflow values**: `WorkflowInput` + `PriorOutput` + `Derived` (all read-only).
///   Also includes gather slot / input-keyed fields (the Phase-1 inputs form) — those
///   are workflow-input values being collected.
/// - **Step values**: `Literal` (step-author constants with `show_in_review = true`)
///   and `UserInput` on a `Review` key (step-local edits, currently all required).
///
/// `body_overflow` fields and `Literal` fields with `show_in_review = false` are
/// excluded by `Form::from_step_plan` before they reach this function; this
/// function does not apply those filters itself (it sees the already-filtered list).
pub(crate) fn partition_fields_by_source(fields: &[FormField]) -> FieldGroups {
    let mut groups = FieldGroups::default();
    for (i, f) in fields.iter().enumerate() {
        match &f.source {
            FieldSource::WorkflowInput | FieldSource::PriorOutput | FieldSource::Derived { .. } => {
                groups.workflow_values.push(i);
            }
            FieldSource::Literal => groups.step_values.push(i),
            FieldSource::UserInput => match &f.key {
                FieldKey::Review(_) => groups.step_values.push(i),
                FieldKey::Slot(_) | FieldKey::Input(_) => groups.workflow_values.push(i),
            },
            // FromFlag / Default / Optional come from the inline gather path; they
            // are workflow-input values being filled.
            FieldSource::FromFlag | FieldSource::Default | FieldSource::Optional => {
                groups.workflow_values.push(i);
            }
        }
    }
    groups
}

/// Build the display line for one field row — label plus value (or the live
/// `editing_buffer`), styled for the focused row.
fn field_line(field: &FormField, focused: bool, editing_buffer: Option<&str>) -> Line<'static> {
    let mut spans = Vec::new();
    let label = field.label.clone();
    // Read-only rows are secondary information (auto-bound refs, captured
    // values), so they render dimmed to let the editable inputs stand out.
    let dim = Style::default().fg(Color::Indexed(244));
    let style = if focused {
        Style::default()
            .bg(Color::Indexed(238))
            .add_modifier(Modifier::BOLD)
    } else if field.read_only {
        dim
    } else {
        Style::default()
    };
    spans.push(Span::styled(format!("  {:<14}", label), style));
    spans.push(Span::raw("  "));
    if let Some(buffer) = editing_buffer {
        // Edit mode: render the live buffer with a trailing block cursor.
        spans.push(Span::raw(format!("{buffer}\u{258C}")));
    } else {
        let value_style = if field.read_only && !focused {
            dim
        } else {
            Style::default()
        };
        spans.push(Span::styled(render_value(&field.value), value_style));
        if matches!(field.source, FieldSource::FromFlag) {
            spans.push(Span::styled(
                " (from flag)",
                Style::default().fg(Color::Indexed(244)),
            ));
        } else if matches!(field.source, FieldSource::Optional) {
            spans.push(Span::styled(
                " (optional)",
                Style::default().fg(Color::Indexed(244)),
            ));
        } else if let FieldSource::Derived { sources } = &field.source {
            spans.push(Span::styled(
                derived_from_suffix(sources),
                Style::default().fg(Color::Indexed(244)),
            ));
        }
    }
    Line::from(spans)
}

/// The editable text backing a field value, if any. Empty `Scalar` and
/// `JsonBody` strings are treated as absent (returns `None`), matching
/// `is_filled`; an `Enum(Some(""))` selection is kept.
fn field_value_string(v: &FieldValue) -> Option<String> {
    match v {
        FieldValue::Scalar(s) | FieldValue::JsonBody(s) if !s.is_empty() => Some(s.clone()),
        FieldValue::Enum(Some(s)) => Some(s.clone()),
        FieldValue::Bool(Some(b)) => Some(b.to_string()),
        _ => None,
    }
}

/// True when the field value carries content (a non-empty entry).
fn is_filled(v: &FieldValue) -> bool {
    match v {
        FieldValue::Scalar(s) | FieldValue::JsonBody(s) => !s.is_empty(),
        FieldValue::Enum(opt) => opt.is_some(),
        FieldValue::Bool(opt) => opt.is_some(),
        FieldValue::Empty => false,
    }
}

/// Trailing `(derived from a, b)` row suffix naming the kebab-cased workflow
/// inputs a derived field's value is computed from.
fn derived_from_suffix(sources: &[String]) -> String {
    let kebabed: Vec<String> = sources
        .iter()
        .map(|s| ags_runtime::support::strings::to_kebab_case(s))
        .collect();
    format!(" (derived from {})", kebabed.join(", "))
}

/// Validation-note text explaining that a derived field's value flows from the
/// named workflow input(s) and can only be changed by editing those.
pub(crate) fn derived_field_note(label: &str, sources: &[String]) -> String {
    let sources_list = sources.join(", ");
    format!(
        "'{label}' is derived from workflow input(s): {sources_list}. \
         Edit the source input(s) to change this value."
    )
}

/// Whether a field is "filled" in the form sense. For `JsonBody` fields,
/// extends the scalar non-empty check by requiring that the JSON value
/// satisfies the schema's `required` keys (recursively for nested objects).
pub(crate) fn is_field_filled(field: &FormField) -> bool {
    if !is_filled(&field.value) {
        return false;
    }
    if matches!(field.field_type, FieldType::JsonBody) {
        if let FieldValue::JsonBody(s) = &field.value {
            return json_satisfies_schema(s, &field.schema);
        }
    }
    true
}

/// Parse `text` as JSON and check that the resulting value satisfies the
/// schema's `required` keys recursively. Returns `false` on parse error.
fn json_satisfies_schema(text: &str, schema: &serde_json::Value) -> bool {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return false,
    };
    json_meets_schema_required(&value, schema)
}

/// True when `value` satisfies the `required`-property constraints declared in
/// JSON `schema`, recursing into nested objects.
fn json_meets_schema_required(value: &serde_json::Value, schema: &serde_json::Value) -> bool {
    match value {
        // Validate every element against the `items` schema, so required
        // properties inside array items are enforced just like top-level ones.
        serde_json::Value::Array(items) => {
            let Some(item_schema) = schema.get("items") else {
                return true;
            };
            items
                .iter()
                .all(|item| json_meets_schema_required(item, item_schema))
        }
        serde_json::Value::Object(obj) => {
            let Some(required) = schema.get("required").and_then(|v| v.as_array()) else {
                return true;
            };
            let properties = schema.get("properties");
            for key in required {
                let Some(name) = key.as_str() else {
                    continue;
                };
                // A required key must be present and non-null. An empty string
                // counts as present (our APIs are often unclear on empties, so we
                // defer that judgement to the server).
                match obj.get(name) {
                    None | Some(serde_json::Value::Null) => return false,
                    Some(child_value) => {
                        if let Some(child_schema) = properties.and_then(|p| p.get(name)) {
                            if !json_meets_schema_required(child_value, child_schema) {
                                return false;
                            }
                        }
                    }
                }
            }
            true
        }
        _ => true,
    }
}

/// Required keys missing or null in `text` against `schema`, as dotted/bracketed
/// paths (e.g. `regions[0].region`, `timeout.seconds`). Recurses into nested
/// objects and array items so the validation note can point at the exact field,
/// matching the recursion in [`json_meets_schema_required`].
pub(crate) fn missing_required_keys(text: &str, schema: &serde_json::Value) -> Vec<String> {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut path = String::new();
    collect_missing_required(&value, schema, &mut path, &mut out);
    out
}

/// Append `.name` (or `name` at the root) to a path prefix, mirroring how the
/// note reads back to the user.
fn push_path_key(path: &mut String, name: &str) {
    if !path.is_empty() {
        path.push('.');
    }
    path.push_str(name);
}

/// Walk `value`/`schema` collecting missing-or-null required keys as paths into
/// `out`. `path` is the prefix to the current node (mutated and restored as the
/// walk descends, so it never allocates per branch).
fn collect_missing_required(
    value: &serde_json::Value,
    schema: &serde_json::Value,
    path: &mut String,
    out: &mut Vec<String>,
) {
    use std::fmt::Write;
    match value {
        serde_json::Value::Array(items) => {
            let Some(item_schema) = schema.get("items") else {
                return;
            };
            for (i, item) in items.iter().enumerate() {
                let base = path.len();
                let _ = write!(path, "[{i}]");
                collect_missing_required(item, item_schema, path, out);
                path.truncate(base);
            }
        }
        serde_json::Value::Object(obj) => {
            let Some(required) = schema.get("required").and_then(|v| v.as_array()) else {
                return;
            };
            let properties = schema.get("properties");
            for key in required {
                let Some(name) = key.as_str() else {
                    continue;
                };
                match obj.get(name) {
                    None | Some(serde_json::Value::Null) => {
                        let base = path.len();
                        push_path_key(path, name);
                        out.push(path.clone());
                        path.truncate(base);
                    }
                    Some(child_value) => {
                        if let Some(child_schema) = properties.and_then(|p| p.get(name)) {
                            let base = path.len();
                            push_path_key(path, name);
                            collect_missing_required(child_value, child_schema, path, out);
                            path.truncate(base);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Character (≈ display cell) count of a string. JSON previews are ASCII in
/// practice, so a char count is an adequate proxy for terminal width.
fn str_cells(s: &str) -> usize {
    s.chars().count()
}

/// Truncate `text` to at most `max` display cells, replacing the tail with an
/// ellipsis when it would overflow. Returns `text` unchanged when it fits.
fn truncate_cells(text: &str, max: usize) -> String {
    if str_cells(text) <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let head: String = text.chars().take(max - 1).collect();
    format!("{head}\u{2026}")
}

/// Compact `{key: value, …}` summary for a `JsonBody` value. Nested objects /
/// arrays collapse to `{…}` / `[N]`; truncated to `JSONBODY_PREVIEW_MAX` chars.
pub(crate) fn summarise_jsonbody(value: &FieldValue) -> String {
    const JSONBODY_PREVIEW_MAX: usize = 100;
    let FieldValue::JsonBody(s) = value else {
        return String::new();
    };
    if s.is_empty() {
        return String::from("{}");
    }
    let truncate = |text: String| -> String {
        if text.chars().count() > JSONBODY_PREVIEW_MAX {
            let head: String = text.chars().take(JSONBODY_PREVIEW_MAX).collect();
            format!("{head}\u{2026}")
        } else {
            text
        }
    };
    match serde_json::from_str::<serde_json::Value>(s) {
        Ok(serde_json::Value::Object(map)) if map.is_empty() => String::from("{}"),
        Ok(serde_json::Value::Object(map)) => {
            let parts: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{k}: {}", short_value(v)))
                .collect();
            truncate(format!("{{{}}}", parts.join(", ")))
        }
        Ok(other) => truncate(other.to_string()),
        Err(_) => truncate(s.clone()),
    }
}

/// One-line representation of a JSON value for the body summary: strings keep
/// their quotes, nested objects collapse to `{…}`, arrays collapse to `[N]`
/// where N is the element count.
fn short_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{s}\""),
        serde_json::Value::Array(arr) => format!("[{}]", arr.len()),
        serde_json::Value::Object(_) => String::from("{\u{2026}}"),
        other => other.to_string(),
    }
}

/// Render a field value as its single-line display string.
fn render_value(v: &FieldValue) -> String {
    match v {
        FieldValue::Scalar(s) => s.clone(),
        FieldValue::Enum(Some(s)) => s.clone(),
        FieldValue::Enum(None) => String::new(),
        FieldValue::Bool(Some(b)) => b.to_string(),
        FieldValue::Bool(None) => String::new(),
        FieldValue::JsonBody(_) => summarise_jsonbody(v),
        FieldValue::Empty => String::new(),
    }
}

/// Display text for a `DynamicEnum` field: the label of the choice whose value
/// equals the carrier string; if no choice matches (raw entry or not-yet-
/// resolved) the raw carrier string verbatim (the escape-hatch fallback).
pub(crate) fn dynamic_enum_display(field: &FormField) -> String {
    let value = match &field.value {
        FieldValue::Enum(Some(s)) => s.clone(),
        _ => return String::new(),
    };
    if let Some(state) = &field.dynamic {
        if let Some(resolved) = &state.resolved {
            if let Some(choice) = resolved.choices.iter().find(|c| c.value == value) {
                return choice.label.clone();
            }
        }
    }
    value
}

/// Dim suffix on a `DynamicEnum` row hinting that Enter opens the picker:
/// ` ‹Enter to choose›` (unresolved), ` ‹Enter to choose — N options›`
/// (resolved), or ` ‹no matches — type a value›` (resolved but empty).
pub(crate) fn dynamic_enum_affordance(field: &FormField) -> Option<String> {
    let state = field.dynamic.as_ref()?;
    Some(match &state.resolved {
        None => " \u{2039}Enter to choose\u{203a}".to_string(),
        Some(r) if r.choices.is_empty() => {
            " \u{2039}no matches \u{2014} type a value\u{203a}".to_string()
        }
        Some(r) => format!(
            " \u{2039}Enter to choose \u{2014} {} options\u{203a}",
            r.choices.len()
        ),
    })
}

/// Display text for a `FilePicker` field: the chosen absolute path, or an
/// empty string when nothing has been picked yet (the affordance hints how
/// to open the browser).
pub(crate) fn file_picker_display(field: &FormField) -> String {
    match &field.value {
        FieldValue::Enum(Some(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Dim suffix on a `FilePicker` row hinting that Enter opens the directory
/// browser. Unlike `dynamic_enum_affordance`, there is no unresolved/resolved
/// state to distinguish — a local directory read never needs a network fetch
/// — so this is always the same hint.
pub(crate) fn file_picker_affordance(field: &FormField) -> Option<String> {
    let _ = field; // kept for signature parity with dynamic_enum_affordance
    Some(" \u{2039}Enter to browse\u{203a}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a sample scalar `FormField` fixture.
    fn sample_field(label: &str, required: bool, source: FieldSource) -> FormField {
        FormField {
            label: label.into(),
            field_type: FieldType::Scalar,
            required,
            value: FieldValue::Empty,
            description: format!("description for {label}"),
            source,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type": "string"}),
            read_only: false,
            dynamic: None,
            file_picker: None,
        }
    }

    /// Build a sample `FilePicker` `FormField` fixture.
    fn sample_file_picker_field(value: FieldValue) -> FormField {
        FormField {
            label: "icon-file".into(),
            field_type: FieldType::FilePicker,
            required: true,
            value,
            description: "pick a file".into(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("iconFile".into()),
            schema: serde_json::json!({"type": "string"}),
            read_only: false,
            dynamic: None,
            file_picker: Some(ags_protocol::workflow::FilePickerSpec {
                extensions: Some(vec!["png".to_string()]),
                start_dir: None,
            }),
        }
    }

    #[test]
    fn test_file_picker_display_shows_chosen_path() {
        let field = sample_file_picker_field(FieldValue::Enum(Some("/tmp/icon.png".to_string())));
        assert_eq!(file_picker_display(&field), "/tmp/icon.png");
    }

    #[test]
    fn test_file_picker_display_empty_when_unset() {
        let field = sample_file_picker_field(FieldValue::Enum(None));
        assert_eq!(file_picker_display(&field), "");
    }

    #[test]
    fn test_file_picker_affordance_hints_enter_to_browse() {
        let field = sample_file_picker_field(FieldValue::Enum(None));
        assert_eq!(
            file_picker_affordance(&field).as_deref(),
            Some(" \u{2039}Enter to browse\u{203a}")
        );
    }

    #[test]
    fn test_commit_edit_file_picker_writes_enum_value() {
        let mut form = Form::new("f", vec![sample_file_picker_field(FieldValue::Empty)]);
        form.begin_edit();
        if let Some(buf) = form.editing.as_mut().and_then(|s| s.text_buffer_mut()) {
            buf.push_str("/tmp/manual.png");
        }
        form.commit_edit();
        assert!(matches!(
            &form.fields[0].value,
            FieldValue::Enum(Some(s)) if s == "/tmp/manual.png"
        ));
    }

    #[test]
    fn test_optional_filter_hides_optional_empty_rows_until_shown() {
        let mut required = sample_field("ns", true, FieldSource::UserInput);
        required.value = FieldValue::Scalar("x".into());
        let optional = sample_field("opt", false, FieldSource::UserInput); // empty
        let mut form = Form::new("t", vec![required, optional])
            .with_optional_filter(true)
            .with_mark_required(true);
        // Collapsed (default): required visible, optional-empty hidden.
        assert!(form.is_row_visible(0));
        assert!(!form.is_row_visible(1));
        assert_eq!(form.optional_empty_count(), 1);
        // Expanded: both visible.
        form.show_optional = true;
        assert!(form.is_row_visible(1));
    }

    #[test]
    fn test_optional_filter_keeps_filled_optional_visible_when_collapsed() {
        let mut optional = sample_field("opt", false, FieldSource::FromFlag);
        optional.value = FieldValue::Scalar("set".into());
        let form = Form::new("t", vec![optional]).with_optional_filter(true);
        assert!(form.is_row_visible(0), "filled optional stays visible");
        assert_eq!(form.optional_empty_count(), 0);
    }

    #[test]
    fn test_normalize_focus_snaps_off_hidden_optional_row() {
        // Row 0 optional-empty (hidden when collapsed), row 1 required (visible).
        let optional = sample_field("opt", false, FieldSource::UserInput); // empty
        let mut required = sample_field("req", true, FieldSource::UserInput);
        required.value = FieldValue::Scalar("x".into());
        let mut form = Form::new("t", vec![optional, required]).with_optional_filter(true);
        form.focus = 0; // hidden row
        form.normalize_focus_to_visible();
        assert_eq!(form.focus, 1, "focus snaps to the visible required row");
    }

    #[test]
    fn test_render_field_row_shows_required_marker_in_advanced_mode() {
        use ratatui::{backend::TestBackend, Terminal};
        let field = sample_field("ns", true, FieldSource::UserInput);
        let mut form = Form::new("t", vec![field]).with_mark_required(true);
        form.show_optional = true; // advanced mode → marker shown
        let mut term = Terminal::new(TestBackend::new(40, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, form.label_width()))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains("ns:*"),
            "required marker after colon in advanced mode: {s}"
        );
    }

    #[test]
    fn test_render_field_row_omits_required_marker_when_collapsed() {
        use ratatui::{backend::TestBackend, Terminal};
        let field = sample_field("ns", true, FieldSource::UserInput);
        // mark_required but collapsed (show_optional defaults false) → no marker.
        let form = Form::new("t", vec![field]).with_mark_required(true);
        let mut term = Terminal::new(TestBackend::new(40, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, form.label_width()))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(!s.contains('*'), "no marker in collapsed mode: {s}");
    }

    #[test]
    fn test_render_field_row_frames_jsonbody_array_value_in_brackets() {
        use ratatui::{backend::TestBackend, Terminal};
        let field = FormField {
            label: "audiences".into(),
            field_type: FieldType::JsonBody,
            required: false,
            value: FieldValue::JsonBody("[\"test\"]".into()),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("audiences".into()),
            schema: serde_json::json!({"type":"array","items":{"type":"string"}}),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        let form = Form::new("t", vec![field]).with_mark_required(true);
        let mut term = Terminal::new(TestBackend::new(60, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, form.label_width()))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains("[\"test\"]"),
            "array value framed in its own dim brackets, not object braces: {s}"
        );
        assert!(
            !s.contains("{[\"test\"]}"),
            "array must NOT be wrapped in object braces: {s}"
        );
    }

    #[test]
    fn test_render_field_row_frames_jsonbody_object_value_in_braces() {
        // An object value keeps a single dim `{ }` frame — its own outer braces
        // are folded in, never doubled.
        use ratatui::{backend::TestBackend, Terminal};
        let field = FormField {
            label: "body".into(),
            field_type: FieldType::JsonBody,
            required: false,
            value: FieldValue::JsonBody(r#"{"region":"eu"}"#.into()),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("body".into()),
            schema: serde_json::json!({"type":"object"}),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        let form = Form::new("t", vec![field]).with_mark_required(true);
        let mut term = Terminal::new(TestBackend::new(60, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, form.label_width()))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        // The value renders via the compact object summary (`{key: value}`),
        // then the dim-brace branch folds the outer braces into the frame — so
        // the object is single-framed, never doubled (`{{…}}`).
        assert!(
            s.contains(r#"{region: "eu"}"#),
            "object value single-framed in dim braces: {s}"
        );
        assert!(!s.contains("{{"), "object braces must not be doubled: {s}");
    }

    #[test]
    fn test_render_field_row_truncates_long_jsonbody_with_ellipsis() {
        use ratatui::{backend::TestBackend, Terminal};
        let field = FormField {
            label: "body".into(),
            field_type: FieldType::JsonBody,
            required: false,
            value: FieldValue::JsonBody(
                r#"{"scheduledServerName":"a-very-long-server-name-value","region":"eu-west-1"}"#
                    .into(),
            ),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("body".into()),
            schema: serde_json::json!({"type":"object"}),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        let form = Form::new("t", vec![field]).with_mark_required(true);
        // Narrow area: the value cannot fit, so it must trail off.
        let mut term = Terminal::new(TestBackend::new(36, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, form.label_width()))
            .unwrap();
        let row: String = {
            let buf = term.backend().buffer();
            (0..buf.area.width).map(|x| buf[(x, 0)].symbol()).collect()
        };
        // Trails off with an ellipsis and keeps the closing brace (`…}`); only
        // blank padding follows, so nothing runs past the right edge.
        assert!(
            row.trim_end().ends_with("\u{2026}}"),
            "value trails off with `…}}` then blanks: {row:?}"
        );
    }

    #[test]
    fn test_render_field_row_empty_jsonbody_shows_braces_frame() {
        use ratatui::{backend::TestBackend, Terminal};
        let field = FormField {
            label: "body".into(),
            field_type: FieldType::JsonBody,
            required: false,
            value: FieldValue::JsonBody(String::new()),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("body".into()),
            schema: serde_json::json!({"type":"object"}),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        let form = Form::new("t", vec![field]).with_mark_required(true);
        let mut term = Terminal::new(TestBackend::new(60, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, form.label_width()))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(s.contains("{}"), "empty body shows the braces frame: {s}");
    }

    #[test]
    fn test_render_field_row_empty_jsonbody_array_shows_bracket_frame() {
        // An empty array-typed body reads `[]`, not `{}` — the schema type drives
        // the frame so it stays consistent before and after a value is entered.
        use ratatui::{backend::TestBackend, Terminal};
        let field = FormField {
            label: "regions".into(),
            field_type: FieldType::JsonBody,
            required: false,
            value: FieldValue::JsonBody(String::new()),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("regions".into()),
            schema: serde_json::json!({"type":"array","items":{"type":"object"}}),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        let form = Form::new("t", vec![field]).with_mark_required(true);
        let mut term = Terminal::new(TestBackend::new(60, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, form.label_width()))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains("[]"),
            "empty array body shows a bracket frame: {s}"
        );
        assert!(
            !s.contains("{}"),
            "empty array must not show object braces: {s}"
        );
    }

    #[test]
    fn test_render_field_row_frames_editable_jsonbody_without_mark_required() {
        // Regression for the fullscreen gather form: the dim `{ }` frame is keyed
        // on the field being an editable JSON body, NOT on `mark_required` (which
        // the fullscreen forms don't set). An editable JsonBody must show braces
        // even without `mark_required`.
        use ratatui::{backend::TestBackend, Terminal};
        let field = FormField {
            label: "body".into(),
            field_type: FieldType::JsonBody,
            required: false,
            value: FieldValue::JsonBody(String::new()),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("body".into()),
            schema: serde_json::json!({"type":"object"}),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        // No `.with_mark_required(true)` — mirrors the fullscreen form.
        let form = Form::new("t", vec![field]);
        let mut term = Terminal::new(TestBackend::new(60, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, form.label_width()))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains("{}"),
            "editable body shows the braces frame without mark_required: {s}"
        );
    }

    #[test]
    fn test_render_field_row_readonly_jsonbody_is_unframed() {
        // A read-only JSON body (review path) renders unframed — no editable
        // `{ }` affordance — even when `mark_required` is set. An array value is
        // shown bare (`["x"]`), not wrapped in the editable frame (`{["x"]}`).
        use ratatui::{backend::TestBackend, Terminal};
        let field = FormField {
            label: "body".into(),
            field_type: FieldType::JsonBody,
            required: false,
            value: FieldValue::JsonBody(r#"["x"]"#.into()),
            description: String::new(),
            source: FieldSource::Literal,
            key: FieldKey::Input("body".into()),
            schema: serde_json::json!({"type":"array"}),
            read_only: true,
            dynamic: None,
            file_picker: None,
        };
        let form = Form::new("t", vec![field]).with_mark_required(true);
        let mut term = Terminal::new(TestBackend::new(60, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, form.label_width()))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains(r#"["x"]"#),
            "read-only array value shown bare: {s}"
        );
        assert!(
            !s.contains(r#"{["x"]}"#),
            "read-only value must NOT get the editable brace frame: {s}"
        );
    }

    #[test]
    fn test_render_field_row_datetime_shows_friendly_rest_display() {
        use ratatui::{backend::TestBackend, Terminal};
        // `datetime_field` helper is defined earlier in this tests module.
        let form = Form::new("t", vec![datetime_field("2026-08-01T00:00:00Z")]);
        let mut term = Terminal::new(TestBackend::new(60, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, form.label_width()))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains("2026-08-01  00:00  UTC"),
            "friendly rest display: {s}"
        );
        // The raw ISO form (with the `T` separator and trailing `Z`) must not
        // be shown. Check the whole ISO string, not a bare `T` — the friendly
        // `UTC` label legitimately contains a `T`.
        assert!(
            !s.contains("2026-08-01T00:00:00Z"),
            "raw ISO string must not be rendered: {s}"
        );
    }

    #[test]
    fn test_render_field_row_datetime_editing_brackets_active_segment() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut form = Form::new("t", vec![datetime_field("2026-08-01T00:00:00Z")]);
        form.focus = 0;
        form.begin_edit(); // Date edit, segment 0 (year)
        let mut term = Terminal::new(TestBackend::new(60, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, form.label_width()))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains("[2026]-08-01"),
            "active year segment bracketed: {s}"
        );
    }

    #[test]
    fn test_render_fields_hides_optional_empty_rows_when_collapsed() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut required = sample_field("req", true, FieldSource::UserInput);
        required.value = FieldValue::Scalar("x".into());
        let optional = sample_field("zebra", false, FieldSource::UserInput); // empty
        let form = Form::new("t", vec![required, optional]).with_optional_filter(true);
        let mut term = Terminal::new(TestBackend::new(40, 8)).unwrap();
        term.draw(|f| form.render_fields(f, f.area())).unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(s.contains("req"), "required row visible: {s}");
        assert!(
            !s.contains("zebra"),
            "optional-empty row hidden in the monolithic render path: {s}"
        );
    }

    #[test]
    fn test_render_field_row_omits_required_marker_without_mark_required() {
        use ratatui::{backend::TestBackend, Terminal};
        let field = sample_field("ns", true, FieldSource::UserInput);
        let form = Form::new("t", vec![field]); // mark_required defaults false
        let mut term = Terminal::new(TestBackend::new(40, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, form.label_width()))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            !s.contains("ns *"),
            "no required marker when mark_required is off: {s}"
        );
    }

    #[test]
    fn test_normalize_focus_snaps_to_last_visible_when_no_following_row() {
        // Row 0 required (visible), row 1 optional-empty (hidden). Focus on the
        // hidden last row → no visible idx >= 1 → fall back to the last visible.
        let mut required = sample_field("req", true, FieldSource::UserInput);
        required.value = FieldValue::Scalar("x".into());
        let optional = sample_field("opt", false, FieldSource::UserInput); // empty
        let mut form = Form::new("t", vec![required, optional]).with_optional_filter(true);
        form.focus = 1; // hidden row, no visible row has idx >= 1
        form.normalize_focus_to_visible();
        assert_eq!(
            form.focus, 0,
            "snaps to last visible row when no following row exists"
        );
    }

    #[test]
    fn test_normalize_focus_noop_when_expanded() {
        let optional = sample_field("opt", false, FieldSource::UserInput);
        let required = sample_field("req", true, FieldSource::UserInput);
        let mut form = Form::new("t", vec![optional, required]).with_optional_filter(true);
        form.show_optional = true; // expanded → everything visible
        form.focus = 0;
        form.normalize_focus_to_visible();
        assert_eq!(form.focus, 0, "expanded view leaves focus untouched");
    }

    #[test]
    fn test_current_hint_strips_trailing_full_stop_from_description() {
        // OpenAPI parameter descriptions are inconsistent — some
        // end in '.', some are bare noun phrases. The hint slot is a field
        // label, not a sentence, so a single trailing full stop is stripped
        // (project convention: no full stops on labels). Both the inline and
        // fullscreen surfaces render hints through this method.
        let mut field = sample_field("stat-code", true, FieldSource::UserInput);
        field.description = "The stat code of the stat to create a stat item for.".into();
        let form = Form::new("t", vec![field]);
        let (hint, is_error) = form.current_hint().expect("focused field yields a hint");
        assert!(!is_error);
        assert_eq!(hint, "The stat code of the stat to create a stat item for");
    }

    #[test]
    fn test_current_hint_leaves_description_without_full_stop_unchanged() {
        // Guard against over-stripping: a bare noun-phrase description (no
        // trailing '.') is rendered verbatim.
        let mut field = sample_field("user-id", true, FieldSource::UserInput);
        field.description = "AccelByte user ID".into();
        let form = Form::new("t", vec![field]);
        let (hint, _) = form.current_hint().expect("focused field yields a hint");
        assert_eq!(hint, "AccelByte user ID");
    }

    #[test]
    fn test_focus_includes_read_only_fields() {
        let mut a = sample_field("a", false, FieldSource::WorkflowInput);
        a.read_only = true;
        let b = sample_field("b", false, FieldSource::Literal); // editable
        let mut form = Form::new("t", vec![a, b]);
        // Read-only fields are included in the Tab ring so users can reach them
        // and view their description in the hint area.  begin_edit is still a no-op.
        form.focus = 0;
        form.begin_edit();
        assert!(
            form.editing.is_none(),
            "read-only field cannot begin editing"
        );
        // focus_next from 0 (read-only) → 1 (editable), not skipping 0 on the way in.
        // Tab from 1 → wraps to 0 (read-only is now reachable).
        form.focus = 1;
        form.focus_next();
        assert_eq!(form.focus, 0, "focus wraps to read-only field");
        form.focus_next();
        assert_eq!(form.focus, 1, "focus advances to editable field");
    }

    #[test]
    fn test_focus_first_editable_normalises_initial_focus() {
        let mut ro = sample_field("a", false, FieldSource::WorkflowInput);
        ro.read_only = true;
        let editable = sample_field("b", false, FieldSource::Literal);
        let mut form = Form::new("t", vec![ro, editable]).with_submit_focusable(true);
        form.focus = 0; // would start on the read-only row
        form.focus_first_editable();
        assert_eq!(form.focus, 1, "starts on the first editable field");

        // All read-only → focus the Submit slot.
        let mut a = sample_field("a", false, FieldSource::WorkflowInput);
        a.read_only = true;
        let mut all_ro = Form::new("t", vec![a]).with_submit_focusable(true);
        all_ro.focus_first_editable();
        assert!(all_ro.is_submit_focused(), "all read-only → focus Submit");
    }

    #[test]
    fn test_run_mode_button_group_focus_and_select() {
        use ags_protocol::workflow::RunMode;
        // `with_submit_focusable(true)` gives the form a focusable submit group;
        // `focus_submit_if_available()` moves focus onto it (the real API names).
        let mut form = Form::new("Inputs", vec![])
            .with_submit_focusable(true)
            .with_run_mode_buttons(true);
        form.focus_submit_if_available();
        assert_eq!(form.selected_run_mode(), RunMode::ReviewInputSteps); // default = first
        form.run_mode_focus_next();
        assert_eq!(form.selected_run_mode(), RunMode::ReviewEveryStep);
        form.run_mode_focus_next();
        assert_eq!(form.selected_run_mode(), RunMode::RunWithoutStopping);
        // Saturates at the last button.
        form.run_mode_focus_next();
        assert_eq!(form.selected_run_mode(), RunMode::RunWithoutStopping);
        // Prev walks back to the first.
        form.run_mode_focus_prev();
        assert_eq!(form.selected_run_mode(), RunMode::ReviewEveryStep);
        form.run_mode_focus_prev();
        assert_eq!(form.selected_run_mode(), RunMode::ReviewInputSteps);
        form.run_mode_focus_prev();
        assert_eq!(form.selected_run_mode(), RunMode::ReviewInputSteps);
    }

    #[test]
    fn test_confirm_skip_button_group_focus_and_select() {
        // `with_confirm_skip_buttons(true)` enables the two-button group;
        // default focus is 0 → Confirm; Right moves to Skip (1); Left returns
        // to Confirm (0); both ends saturate.
        let mut form = Form::new("Review", vec![])
            .with_submit_focusable(true)
            .with_confirm_skip_buttons(true);
        form.focus_submit_if_available();
        assert_eq!(form.selected_confirm_skip(), ConfirmSkipChoice::Confirm); // default
        form.confirm_skip_focus_next();
        assert_eq!(form.selected_confirm_skip(), ConfirmSkipChoice::Skip);
        // Saturates at the last button (Skip).
        form.confirm_skip_focus_next();
        assert_eq!(form.selected_confirm_skip(), ConfirmSkipChoice::Skip);
        // Prev walks back to Confirm.
        form.confirm_skip_focus_prev();
        assert_eq!(form.selected_confirm_skip(), ConfirmSkipChoice::Confirm);
        // Saturates at the first button (Confirm).
        form.confirm_skip_focus_prev();
        assert_eq!(form.selected_confirm_skip(), ConfirmSkipChoice::Confirm);
    }

    #[test]
    fn test_submit_target_absent_by_default_focus_cycles_fields_only() {
        let mut form = Form::new(
            "t",
            vec![
                sample_field("a", true, FieldSource::UserInput),
                sample_field("b", false, FieldSource::Optional),
            ],
        );
        // default: submit not focusable → ring is just the 2 fields
        assert!(!form.is_submit_focused());
        form.focus = 1;
        form.focus_next();
        assert_eq!(form.focus, 0); // wraps over fields only
        assert!(!form.is_submit_focused());
    }

    #[test]
    fn test_submit_focusable_appends_submit_target_to_ring() {
        let mut form = Form::new(
            "t",
            vec![
                sample_field("a", true, FieldSource::UserInput),
                sample_field("b", false, FieldSource::Optional),
            ],
        )
        .with_submit_focusable(true);
        form.focus = 1;
        form.focus_next(); // → submit slot (index == fields.len())
        assert!(form.is_submit_focused());
        assert!(form.focused().is_none()); // no field focused on the submit slot
        form.focus_next(); // wraps back to field 0
        assert_eq!(form.focus, 0);
        assert!(!form.is_submit_focused());
    }

    #[test]
    fn test_render_into_test_backend_does_not_panic() {
        use ratatui::{backend::TestBackend, Terminal};
        let form = Form::new("t", vec![sample_field("a", true, FieldSource::UserInput)]);
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| form.render(f, f.area())).unwrap();
    }

    #[test]
    fn test_editing_clears_validation_note() {
        let mut form = Form::new(
            "t",
            vec![sample_field("first", true, FieldSource::UserInput)],
        );
        form.validation_note = Some("Required: first".into());
        form.begin_edit();
        assert!(form.validation_note.is_none());
    }

    #[test]
    fn test_focus_next_wraps_to_zero() {
        let mut form = Form::new(
            "test",
            vec![
                sample_field("a", true, FieldSource::UserInput),
                sample_field("b", false, FieldSource::Optional),
            ],
        );
        form.focus = 1;
        form.focus_next();
        assert_eq!(form.focus, 0);
    }

    #[test]
    fn test_focus_prev_wraps_to_last() {
        let mut form = Form::new(
            "test",
            vec![
                sample_field("a", true, FieldSource::UserInput),
                sample_field("b", false, FieldSource::Optional),
            ],
        );
        form.focus = 0;
        form.focus_prev();
        assert_eq!(form.focus, 1);
    }

    #[test]
    fn test_focused_returns_field_at_focus_index() {
        let form = Form::new(
            "test",
            vec![
                sample_field("first", true, FieldSource::UserInput),
                sample_field("second", false, FieldSource::Optional),
            ],
        );
        assert_eq!(form.focused().unwrap().label, "first");
    }

    #[test]
    fn test_begin_edit_loads_buffer_from_current_value() {
        let mut form = Form::new(
            "t",
            vec![FormField {
                label: "a".into(),
                field_type: FieldType::Scalar,
                required: true,
                value: FieldValue::Scalar("hello".into()),
                description: String::new(),
                source: FieldSource::Default,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({"type": "string"}),
                read_only: false,
                dynamic: None,
                file_picker: None,
            }],
        );
        form.begin_edit();
        let state = form.editing.as_ref().unwrap();
        assert_eq!(state.text_buffer(), Some("hello"));
        assert_eq!(state.field_index, 0);
    }

    #[test]
    fn test_commit_edit_writes_buffer_back_and_marks_user_input() {
        let mut form = Form::new(
            "t",
            vec![FormField {
                label: "a".into(),
                field_type: FieldType::Scalar,
                required: true,
                value: FieldValue::Empty,
                description: String::new(),
                source: FieldSource::Default,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({"type": "string"}),
                read_only: false,
                dynamic: None,
                file_picker: None,
            }],
        );
        form.begin_edit();
        form.editing
            .as_mut()
            .unwrap()
            .text_buffer_mut()
            .unwrap()
            .push_str("world");
        form.commit_edit();
        assert!(matches!(form.fields[0].value, FieldValue::Scalar(ref s) if s == "world"));
        assert_eq!(form.fields[0].source, FieldSource::UserInput);
        assert!(form.editing.is_none());
    }

    #[test]
    fn test_cancel_edit_discards_buffer() {
        let mut form = Form::new(
            "t",
            vec![FormField {
                label: "a".into(),
                field_type: FieldType::Scalar,
                required: true,
                value: FieldValue::Scalar("original".into()),
                description: String::new(),
                source: FieldSource::UserInput,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({"type": "string"}),
                read_only: false,
                dynamic: None,
                file_picker: None,
            }],
        );
        form.begin_edit();
        *form.editing.as_mut().unwrap().text_buffer_mut().unwrap() = "changed".into();
        form.cancel_edit();
        assert!(matches!(form.fields[0].value, FieldValue::Scalar(ref s) if s == "original"));
        assert!(form.editing.is_none());
    }

    fn datetime_field(iso: &str) -> FormField {
        FormField {
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
            file_picker: None,
        }
    }

    #[test]
    fn test_datetime_field_edits_via_date_kind_and_commits_iso() {
        let mut form = Form::new("t", vec![datetime_field("2026-08-01T00:00:00Z")]);
        form.focus = 0;
        form.begin_edit();
        assert!(
            form.editing.as_ref().unwrap().is_date(),
            "a parseable DateTime opens a Date edit"
        );
        form.commit_edit();
        match &form.fields[0].value {
            FieldValue::Scalar(s) => assert_eq!(s, "2026-08-01T00:00:00Z"),
            other => panic!("expected Scalar ISO, got {other:?}"),
        }
    }

    #[test]
    fn test_datetime_field_with_unparseable_value_falls_back_to_text() {
        let mut form = Form::new("t", vec![datetime_field("garbage")]);
        form.focus = 0;
        form.begin_edit();
        let st = form.editing.as_ref().unwrap();
        assert!(!st.is_date(), "unparseable DateTime opens a Text edit");
        assert_eq!(st.text_buffer(), Some("garbage"));
        form.commit_edit();
        match &form.fields[0].value {
            FieldValue::Scalar(s) => {
                assert_eq!(s, "garbage", "text fallback commits raw, not via to_iso")
            }
            other => panic!("expected raw Scalar, got {other:?}"),
        }
    }

    #[test]
    fn test_all_required_filled_true_when_all_required_have_values() {
        let form = Form::new(
            "t",
            vec![
                FormField {
                    label: "name".into(),
                    field_type: FieldType::Scalar,
                    required: true,
                    value: FieldValue::Scalar("alice".into()),
                    description: String::new(),
                    source: FieldSource::UserInput,
                    key: FieldKey::Slot(GatherSlotId(0)),
                    schema: serde_json::json!({"type": "string"}),
                    read_only: false,
                    dynamic: None,
                    file_picker: None,
                },
                FormField {
                    label: "nickname".into(),
                    field_type: FieldType::Scalar,
                    required: false,
                    value: FieldValue::Empty,
                    description: String::new(),
                    source: FieldSource::Optional,
                    key: FieldKey::Slot(GatherSlotId(1)),
                    schema: serde_json::json!({"type": "string"}),
                    read_only: false,
                    dynamic: None,
                    file_picker: None,
                },
            ],
        );
        assert!(form.all_required_filled());
    }

    #[test]
    fn test_all_required_filled_false_when_required_is_empty() {
        let form = Form::new(
            "t",
            vec![FormField {
                label: "name".into(),
                field_type: FieldType::Scalar,
                required: true,
                value: FieldValue::Empty,
                description: String::new(),
                source: FieldSource::Default,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({"type": "string"}),
                read_only: false,
                dynamic: None,
                file_picker: None,
            }],
        );
        assert!(!form.all_required_filled());
    }

    #[test]
    fn test_cycle_focused_enum_wraps_forward() {
        let mut form = Form::new(
            "t",
            vec![FormField {
                label: "mode".into(),
                field_type: FieldType::Enum {
                    variants: vec!["a".into(), "b".into(), "c".into()],
                },
                required: true,
                value: FieldValue::Enum(Some("a".into())),
                description: String::new(),
                source: FieldSource::Default,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({"type":"string"}),
                read_only: false,
                dynamic: None,
                file_picker: None,
            }],
        );
        form.focus = 0;
        form.cycle_focused_enum(true);
        assert!(matches!(form.fields[0].value, FieldValue::Enum(Some(ref s)) if s == "b"));
        form.cycle_focused_enum(true);
        assert!(matches!(form.fields[0].value, FieldValue::Enum(Some(ref s)) if s == "c"));
        form.cycle_focused_enum(true);
        assert!(matches!(form.fields[0].value, FieldValue::Enum(Some(ref s)) if s == "a"));
        assert_eq!(form.fields[0].source, FieldSource::UserInput);
    }

    #[test]
    fn test_cycle_focused_enum_backward() {
        let mut form = Form::new(
            "t",
            vec![FormField {
                label: "mode".into(),
                field_type: FieldType::Enum {
                    variants: vec!["a".into(), "b".into(), "c".into()],
                },
                required: true,
                value: FieldValue::Enum(Some("a".into())),
                description: String::new(),
                source: FieldSource::Default,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({"type":"string"}),
                read_only: false,
                dynamic: None,
                file_picker: None,
            }],
        );
        form.focus = 0;
        form.cycle_focused_enum(false);
        assert!(matches!(form.fields[0].value, FieldValue::Enum(Some(ref s)) if s == "c"));
    }

    #[test]
    fn test_cycle_focused_enum_unset_picks_first_variant() {
        let mut form = Form::new(
            "t",
            vec![FormField {
                label: "mode".into(),
                field_type: FieldType::Enum {
                    variants: vec!["a".into(), "b".into()],
                },
                required: false,
                value: FieldValue::Enum(None),
                description: String::new(),
                source: FieldSource::Default,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({"type":"string"}),
                read_only: false,
                dynamic: None,
                file_picker: None,
            }],
        );
        form.focus = 0;
        form.cycle_focused_enum(true);
        assert!(matches!(form.fields[0].value, FieldValue::Enum(Some(ref s)) if s == "a"));
    }

    #[test]
    fn test_toggle_focused_bool_initialises_then_flips() {
        let mut form = Form::new(
            "t",
            vec![FormField {
                label: "active".into(),
                field_type: FieldType::Bool,
                required: false,
                value: FieldValue::Bool(None),
                description: String::new(),
                source: FieldSource::Default,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({"type":"boolean"}),
                read_only: false,
                dynamic: None,
                file_picker: None,
            }],
        );
        form.focus = 0;
        form.toggle_focused_bool();
        assert!(matches!(form.fields[0].value, FieldValue::Bool(Some(true))));
        form.toggle_focused_bool();
        assert!(matches!(
            form.fields[0].value,
            FieldValue::Bool(Some(false))
        ));
        assert_eq!(form.fields[0].source, FieldSource::UserInput);
    }

    #[test]
    fn test_begin_edit_no_ops_on_enum_and_bool() {
        let mut form = Form::new(
            "t",
            vec![
                FormField {
                    label: "mode".into(),
                    field_type: FieldType::Enum {
                        variants: vec!["a".into()],
                    },
                    required: false,
                    value: FieldValue::Enum(Some("a".into())),
                    description: String::new(),
                    source: FieldSource::Default,
                    key: FieldKey::Slot(GatherSlotId(0)),
                    schema: serde_json::json!({}),
                    read_only: false,
                    dynamic: None,
                    file_picker: None,
                },
                FormField {
                    label: "active".into(),
                    field_type: FieldType::Bool,
                    required: false,
                    value: FieldValue::Bool(Some(true)),
                    description: String::new(),
                    source: FieldSource::Default,
                    key: FieldKey::Slot(GatherSlotId(1)),
                    schema: serde_json::json!({}),
                    read_only: false,
                    dynamic: None,
                    file_picker: None,
                },
            ],
        );
        form.focus = 0;
        form.begin_edit();
        assert!(form.editing.is_none(), "Enum does not enter edit mode");
        form.focus = 1;
        form.begin_edit();
        assert!(form.editing.is_none(), "Bool does not enter edit mode");
    }

    #[test]
    fn test_form_from_plan_round_trips_changed_fields_only() {
        use ags_protocol::workflow::{
            StepField, StepFieldId, StepFieldLocation, StepFieldPlan, StepFieldSource,
        };
        let plan = StepFieldPlan {
            step_index: 0,
            step_label: "Create the AMS fleet".into(),
            step_description: None,
            optional: false,
            fields: vec![
                StepField {
                    id: StepFieldId(0),
                    field: "namespace".into(),
                    label: "namespace".into(),
                    description: Some("Game namespace".into()),
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
        let mut form = Form::from_step_plan(&plan);
        // Field 0 (namespace, WorkflowInput) is read-only; edit its value to
        // confirm that read-only fields are NOT projected.
        form.fields[0].value = FieldValue::Scalar("prod".into());
        // Edit field 1 (statCode, Literal) — the editable field.
        form.fields[1].value = FieldValue::Scalar("elo".into());
        let edits = form.project_step_edits(&plan);
        // Only the editable field's change projects; the read-only edit is skipped.
        assert_eq!(edits.values.len(), 1);
        assert_eq!(
            edits.values.get(&StepFieldId(1)),
            Some(&serde_json::json!("elo"))
        );
        assert!(
            !edits.values.contains_key(&StepFieldId(0)),
            "read-only field must not project"
        );
        // Provenance maps to a dedicated FieldSource bucket, NOT FromFlag.
        assert!(matches!(form.fields[1].source, FieldSource::Literal));
        assert!(matches!(form.fields[0].source, FieldSource::WorkflowInput));
        // Description carries the field hint, not the provenance label.
        assert_eq!(form.fields[0].description, "Game namespace");
    }

    #[test]
    fn test_from_step_plan_marks_input_and_prior_read_only() {
        use ags_protocol::workflow::{
            StepField, StepFieldId, StepFieldLocation, StepFieldPlan, StepFieldSource,
        };
        let plan = StepFieldPlan {
            step_index: 0,
            step_label: "s".into(),
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
                    field: "ruleSet".into(),
                    label: "rule-set".into(),
                    description: None,
                    location: StepFieldLocation::Body,
                    schema: serde_json::json!({"type":"string"}),
                    value: serde_json::json!("x"),
                    source: StepFieldSource::PriorOutput,
                    required: true,
                    workflow_input: None,
                    body_overflow: false,
                    show_in_review: false,
                },
                StepField {
                    id: StepFieldId(2),
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
        let form = Form::from_step_plan(&plan);
        assert!(form.fields[0].read_only, "workflow input is read-only");
        assert!(form.fields[1].read_only, "prior output is read-only");
        assert!(
            !form.fields[2].read_only,
            "step value (literal) is editable"
        );
    }

    #[test]
    fn test_from_step_plan_groups_read_only_before_editable() {
        use ags_protocol::workflow::{
            StepField, StepFieldId, StepFieldLocation, StepFieldPlan, StepFieldSource,
        };
        // Plan order interleaves: editable, read-only, editable, read-only. The
        // inline review must group all read-only inputs ahead of the editable
        // step fields, preserving each group's relative order, and title the box
        // with the step name.
        let field = |id, label: &str, src, ro_input: Option<&str>, show| StepField {
            id: StepFieldId(id),
            field: label.into(),
            label: label.into(),
            description: None,
            location: StepFieldLocation::Body,
            schema: serde_json::json!({"type":"string"}),
            value: serde_json::json!("v"),
            source: src,
            required: false,
            workflow_input: ro_input.map(|s| s.to_string()),
            body_overflow: false,
            show_in_review: show,
        };
        let plan = StepFieldPlan {
            step_index: 0,
            step_label: "create-fleet".into(),
            step_description: None,
            optional: false,
            fields: vec![
                field(0, "edit-a", StepFieldSource::Literal, None, true),
                field(
                    1,
                    "ns",
                    StepFieldSource::WorkflowInput { name: "ns".into() },
                    Some("ns"),
                    false,
                ),
                field(2, "edit-b", StepFieldSource::Literal, None, true),
                field(3, "prior", StepFieldSource::PriorOutput, None, false),
            ],
        };
        let form = Form::from_step_plan(&plan);
        let order: Vec<(&str, bool)> = form
            .fields
            .iter()
            .map(|f| (f.label.as_str(), f.read_only))
            .collect();
        assert_eq!(
            order,
            vec![
                ("ns", true),
                ("prior", true),
                ("edit-a", false),
                ("edit-b", false),
            ],
            "read-only inputs grouped first, each group's order preserved"
        );
        assert_eq!(
            form.box_title.as_deref(),
            Some("Step 1: create-fleet"),
            "box title prepends the 1-based step number to the step name"
        );
    }

    #[test]
    fn test_project_step_edits_matches_by_id_after_reorder() {
        use ags_protocol::workflow::{
            StepField, StepFieldId, StepFieldLocation, StepFieldPlan, StepFieldSource,
        };
        // Two editable literals plus a read-only input. The form is reordered vs
        // the plan (read-only first), so edits must match plan fields by id — a
        // positional zip would compare against the wrong baseline.
        let plan = StepFieldPlan {
            step_index: 0,
            step_label: "s".into(),
            step_description: None,
            optional: false,
            fields: vec![
                StepField {
                    id: StepFieldId(0),
                    field: "a".into(),
                    label: "a".into(),
                    description: None,
                    location: StepFieldLocation::Body,
                    schema: serde_json::json!({"type":"string"}),
                    value: serde_json::json!("a0"),
                    source: StepFieldSource::Literal,
                    required: false,
                    workflow_input: None,
                    body_overflow: false,
                    show_in_review: true,
                },
                StepField {
                    id: StepFieldId(1),
                    field: "ns".into(),
                    label: "ns".into(),
                    description: None,
                    location: StepFieldLocation::Path,
                    schema: serde_json::json!({"type":"string"}),
                    value: serde_json::json!("dev"),
                    source: StepFieldSource::WorkflowInput { name: "ns".into() },
                    required: true,
                    workflow_input: Some("ns".into()),
                    body_overflow: false,
                    show_in_review: false,
                },
                StepField {
                    id: StepFieldId(2),
                    field: "b".into(),
                    label: "b".into(),
                    description: None,
                    location: StepFieldLocation::Body,
                    schema: serde_json::json!({"type":"string"}),
                    value: serde_json::json!("b0"),
                    source: StepFieldSource::Literal,
                    required: false,
                    workflow_input: None,
                    body_overflow: false,
                    show_in_review: true,
                },
            ],
        };
        let mut form = Form::from_step_plan(&plan);
        // Edit field `b` (id 2) only, wherever it landed after the reorder.
        let idx = form
            .fields
            .iter()
            .position(|f| f.label == "b")
            .expect("b present");
        form.fields[idx].value = FieldValue::Scalar("b1".into());
        let edits = form.project_step_edits(&plan);
        assert_eq!(
            edits.values.get(&StepFieldId(2)),
            Some(&serde_json::json!("b1")),
            "edit on id 2 projects against its own plan baseline"
        );
        assert!(
            !edits.values.contains_key(&StepFieldId(0)),
            "unchanged field a (id 0) is not falsely reported as edited"
        );
    }

    #[test]
    fn test_project_inputs_omits_cleared_optional() {
        let mut form = Form::new(
            "Inputs",
            vec![
                FormField {
                    label: "namespace".into(),
                    field_type: FieldType::Scalar,
                    required: true,
                    value: FieldValue::Scalar("dev".into()),
                    description: String::new(),
                    source: FieldSource::UserInput,
                    key: FieldKey::Input("namespace".into()),
                    schema: serde_json::json!({"type":"string"}),
                    read_only: false,
                    dynamic: None,
                    file_picker: None,
                },
                FormField {
                    label: "fleetName".into(),
                    field_type: FieldType::Scalar,
                    required: false,
                    value: FieldValue::Empty,
                    description: String::new(),
                    source: FieldSource::UserInput,
                    key: FieldKey::Input("fleetName".into()),
                    schema: serde_json::json!({"type":"string"}),
                    read_only: false,
                    dynamic: None,
                    file_picker: None,
                },
            ],
        );
        form.editing = None;
        let map = form.project_inputs();
        assert_eq!(map.get("namespace"), Some(&serde_json::json!("dev")));
        assert!(
            !map.contains_key("fleetName"),
            "cleared optional is omitted (unset)"
        );
    }

    #[test]
    fn test_partition_fields_by_source_groups_in_order() {
        // A mixed-source form: assert each group's labels in order.
        use ags_protocol::workflow::StepFieldId;
        let mut form = Form::new(
            "t",
            vec![
                // Step value — literal (editable)
                sample_field("step-lit", true, FieldSource::Literal),
                // Workflow value — input (read-only)
                {
                    let mut f = sample_field("wf-in", true, FieldSource::WorkflowInput);
                    f.read_only = true;
                    f
                },
                // Workflow value — prior output (read-only)
                {
                    let mut f = sample_field("prev", true, FieldSource::PriorOutput);
                    f.read_only = true;
                    f
                },
                // Step value — user input on a review field (editable)
                FormField {
                    label: "user-step".into(),
                    field_type: FieldType::Scalar,
                    required: true,
                    value: FieldValue::Empty,
                    description: String::new(),
                    source: FieldSource::UserInput,
                    key: FieldKey::Review(StepFieldId(0)),
                    schema: serde_json::json!({"type":"string"}),
                    read_only: false,
                    dynamic: None,
                    file_picker: None,
                },
            ],
        );
        form.editing = None;
        let groups = partition_fields_by_source(&form.fields);
        assert_eq!(
            groups
                .workflow_values
                .iter()
                .map(|i| form.fields[*i].label.as_str())
                .collect::<Vec<_>>(),
            vec!["wf-in", "prev"]
        );
        assert_eq!(
            groups
                .step_values
                .iter()
                .map(|i| form.fields[*i].label.as_str())
                .collect::<Vec<_>>(),
            vec!["step-lit", "user-step"]
        );
    }

    #[test]
    fn test_partition_fields_by_source_skips_empty_sections() {
        let form = Form::new(
            "t",
            vec![sample_field("only-step", true, FieldSource::Literal)],
        );
        let groups = partition_fields_by_source(&form.fields);
        assert!(groups.workflow_values.is_empty());
        assert_eq!(groups.step_values.len(), 1);
    }

    /// `from_step_plan` filtering: provenance-traced sources always appear;
    /// Literal fields are gated on `show_in_review`.
    #[test]
    fn test_from_step_plan_filters_literal_by_show_in_review() {
        use ags_protocol::workflow::{
            StepField, StepFieldId, StepFieldLocation, StepFieldPlan, StepFieldSource,
        };
        let plan = StepFieldPlan {
            step_index: 0,
            step_label: "s".into(),
            step_description: None,
            optional: false,
            fields: vec![
                // WorkflowInput → always shown
                StepField {
                    id: StepFieldId(0),
                    field: "ns".into(),
                    label: "ns".into(),
                    description: None,
                    location: StepFieldLocation::Path,
                    schema: serde_json::json!({"type":"string"}),
                    value: serde_json::json!("dev"),
                    source: StepFieldSource::WorkflowInput { name: "ns".into() },
                    required: true,
                    workflow_input: Some("ns".into()),
                    body_overflow: false,
                    show_in_review: false,
                },
                // Derived → always shown
                StepField {
                    id: StepFieldId(1),
                    field: "prefix".into(),
                    label: "prefix".into(),
                    description: None,
                    location: StepFieldLocation::Body,
                    schema: serde_json::json!({"type":"string"}),
                    value: serde_json::json!("ranked"),
                    source: StepFieldSource::Derived {
                        sources: vec!["resourcePrefix".into()],
                    },
                    required: false,
                    workflow_input: None,
                    body_overflow: false,
                    show_in_review: false,
                },
                // PriorOutput → always shown
                StepField {
                    id: StepFieldId(2),
                    field: "ruleSet".into(),
                    label: "rule-set".into(),
                    description: None,
                    location: StepFieldLocation::Body,
                    schema: serde_json::json!({"type":"string"}),
                    value: serde_json::json!("x"),
                    source: StepFieldSource::PriorOutput,
                    required: false,
                    workflow_input: None,
                    body_overflow: false,
                    show_in_review: false,
                },
                // Literal show_in_review=true → shown
                StepField {
                    id: StepFieldId(3),
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
                // Literal show_in_review=false → NOT shown
                StepField {
                    id: StepFieldId(4),
                    field: "hidden".into(),
                    label: "hidden".into(),
                    description: None,
                    location: StepFieldLocation::Body,
                    schema: serde_json::json!({"type":"string"}),
                    value: serde_json::json!("secret"),
                    source: StepFieldSource::Literal,
                    required: false,
                    workflow_input: None,
                    body_overflow: false,
                    show_in_review: false,
                },
                // body_overflow → NOT shown (regardless of show_in_review)
                StepField {
                    id: StepFieldId(5),
                    field: "extras".into(),
                    label: "extras".into(),
                    description: None,
                    location: StepFieldLocation::Body,
                    schema: serde_json::json!({"type":"object"}),
                    value: serde_json::json!({}),
                    source: StepFieldSource::Literal,
                    required: false,
                    workflow_input: None,
                    body_overflow: true,
                    show_in_review: true,
                },
            ],
        };
        let form = Form::from_step_plan(&plan);
        let labels: Vec<&str> = form.fields.iter().map(|f| f.label.as_str()).collect();
        // WorkflowInput, Derived, PriorOutput always present.
        assert!(labels.contains(&"ns"), "WorkflowInput always shown");
        assert!(labels.contains(&"prefix"), "Derived always shown");
        assert!(labels.contains(&"rule-set"), "PriorOutput always shown");
        // Literal with show_in_review=true is present.
        assert!(
            labels.contains(&"stat-code"),
            "Literal show_in_review=true shown"
        );
        // Literal with show_in_review=false is absent.
        assert!(
            !labels.contains(&"hidden"),
            "Literal show_in_review=false hidden"
        );
        // body_overflow is absent regardless of show_in_review.
        assert!(!labels.contains(&"extras"), "body_overflow field excluded");
        assert_eq!(form.fields.len(), 4, "four visible fields");
    }

    #[test]
    fn test_project_gathered_routes_slot_and_input_keys() {
        let mut form = Form::new(
            "t",
            vec![
                FormField {
                    label: "user-id".into(),
                    field_type: FieldType::Scalar,
                    required: true,
                    value: FieldValue::Scalar("u-1".into()),
                    description: String::new(),
                    source: FieldSource::UserInput,
                    key: FieldKey::Slot(GatherSlotId(0)),
                    schema: serde_json::json!({"type": "string"}),
                    read_only: false,
                    dynamic: None,
                    file_picker: None,
                },
                FormField {
                    label: "count".into(),
                    field_type: FieldType::Scalar,
                    required: false,
                    value: FieldValue::Scalar("42".into()),
                    description: String::new(),
                    source: FieldSource::FromFlag,
                    key: FieldKey::Input("count".into()),
                    schema: serde_json::json!({"type": "integer"}),
                    read_only: false,
                    dynamic: None,
                    file_picker: None,
                },
            ],
        );
        form.editing = None;
        let r = form.project_gathered();
        assert_eq!(
            r.slot_values.get(&GatherSlotId(0)),
            Some(&serde_json::json!("u-1"))
        );
        assert_eq!(r.input_overrides.get("count"), Some(&serde_json::json!(42)));
    }

    #[test]
    fn test_render_field_row_omits_required_marker() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let field = FormField {
            label: "ns".into(),
            field_type: FieldType::Scalar,
            required: true,
            value: FieldValue::Scalar("dev".into()),
            description: String::new(),
            source: FieldSource::WorkflowInput,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type":"string"}),
            read_only: true,
            dynamic: None,
            file_picker: None,
        };
        let form = Form::new("t", vec![field]);
        let mut term = Terminal::new(TestBackend::new(40, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, 2))
            .unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(buf.contains("ns:"), "label and colon present: {buf}");
        assert!(
            !buf.contains("ns:*"),
            "required marker dropped (gather phase enforces required-ness): {buf}"
        );
    }

    #[test]
    fn test_render_field_row_brackets_editable_scalar() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let field = FormField {
            label: "a".into(),
            field_type: FieldType::Scalar,
            required: false,
            value: FieldValue::Scalar("hi".into()),
            description: String::new(),
            source: FieldSource::Literal,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type":"string"}),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        let form = Form::new("t", vec![field]);
        let mut term = Terminal::new(TestBackend::new(40, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, 1))
            .unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            buf.contains("[hi]"),
            "editable scalar wrapped in tight brackets: {buf}"
        );
    }

    #[test]
    fn test_render_field_row_brackets_dim_and_tight() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let field = FormField {
            label: "a".into(),
            field_type: FieldType::Scalar,
            required: false,
            value: FieldValue::Scalar("hi".into()),
            description: String::new(),
            source: FieldSource::Literal,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type":"string"}),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        let form = Form::new("t", vec![field]);
        let mut term = Terminal::new(TestBackend::new(40, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, 1))
            .unwrap();
        let buf = term.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("[hi]"), "tight brackets: {content}");
        assert!(!content.contains("[ hi ]"), "no inner spaces: {content}");
        // Locate brackets by CELL index — `content.find` returns a byte offset,
        // which drifts past the true cell when a multi-byte glyph (the focus
        // caret `▸`) precedes the bracket.
        let lb = buf
            .content()
            .iter()
            .position(|c| c.symbol() == "[")
            .expect("`[` in buffer");
        let rb = buf
            .content()
            .iter()
            .position(|c| c.symbol() == "]")
            .expect("`]` in buffer");
        assert_eq!(buf.content()[lb].fg, ratatui::style::Color::Indexed(244));
        assert_eq!(buf.content()[rb].fg, ratatui::style::Color::Indexed(244));
    }

    #[test]
    fn test_render_field_row_readonly_value_unbracketed() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let field = FormField {
            label: "a".into(),
            field_type: FieldType::Scalar,
            required: false,
            value: FieldValue::Scalar("hi".into()),
            description: String::new(),
            source: FieldSource::WorkflowInput,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type":"string"}),
            read_only: true,
            dynamic: None,
            file_picker: None,
        };
        let form = Form::new("t", vec![field]);
        let mut term = Terminal::new(TestBackend::new(40, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, 1))
            .unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            !buf.contains('['),
            "read-only value is not bracketed: {buf}"
        );
        assert!(buf.contains(" hi"), "read-only value still shown: {buf}");
    }

    #[test]
    fn test_jsonbody_summary_lists_top_level_keys() {
        let body = FieldValue::JsonBody(
            serde_json::to_string_pretty(&serde_json::json!({
                "imageId": "img",
                "commandLine": "./srv",
                "portConfigurations": []
            }))
            .unwrap(),
        );
        let summary = summarise_jsonbody(&body);
        assert!(summary.contains("imageId"));
        assert!(summary.contains("commandLine"));
        assert!(summary.contains("portConfigurations"));
        assert!(
            !summary.contains("chars"),
            "no `(N chars)` fallback: {summary}"
        );
    }

    #[test]
    fn test_jsonbody_summary_empty_object() {
        let body = FieldValue::JsonBody("{}".into());
        assert_eq!(summarise_jsonbody(&body), "{}");
    }

    #[test]
    fn test_jsonbody_summary_includes_values() {
        let body = FieldValue::JsonBody(
            serde_json::to_string(&serde_json::json!({"imageId": "img-1"})).unwrap(),
        );
        assert_eq!(summarise_jsonbody(&body), "{imageId: \"img-1\"}");
    }

    #[test]
    fn test_jsonbody_summary_two_keys_with_values() {
        let body = FieldValue::JsonBody(
            serde_json::to_string(&serde_json::json!({
                "imageId": "img-1",
                "commandLine": "./srv"
            }))
            .unwrap(),
        );
        let s = summarise_jsonbody(&body);
        assert!(s.starts_with("{"));
        assert!(s.contains("imageId: \"img-1\""));
        assert!(s.contains("commandLine: \"./srv\""));
    }

    #[test]
    fn test_jsonbody_summary_collapses_nested_object_and_array() {
        let body = FieldValue::JsonBody(
            serde_json::to_string(&serde_json::json!({
                "x": {"a":1,"b":2},
                "y": [10, 20, 30]
            }))
            .unwrap(),
        );
        let s = summarise_jsonbody(&body);
        assert!(s.contains("x: {\u{2026}}"), "nested object collapsed: {s}");
        assert!(s.contains("y: [3]"), "array shows length: {s}");
    }

    #[test]
    fn test_summarise_jsonbody_empty_returns_braces() {
        assert_eq!(summarise_jsonbody(&FieldValue::JsonBody("{}".into())), "{}");
        assert_eq!(
            summarise_jsonbody(&FieldValue::JsonBody(String::new())),
            "{}"
        );
    }

    #[test]
    fn test_summarise_jsonbody_preview_limit_100() {
        let mut map = serde_json::Map::new();
        for i in 0..30 {
            map.insert(format!("key-{i:02}"), serde_json::json!("v"));
        }
        let body =
            FieldValue::JsonBody(serde_json::to_string(&serde_json::Value::Object(map)).unwrap());
        let s = summarise_jsonbody(&body);
        assert!(s.ends_with("\u{2026}"), "truncated: {s}");
        assert!(
            s.chars().count() <= 101,
            "≤ 100 chars + ellipsis: len={}",
            s.chars().count()
        );
    }

    #[test]
    fn test_is_field_filled_jsonbody_missing_required_key_is_not_filled() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["imageId", "commandLine"],
            "properties": {
                "imageId": {"type": "string"},
                "commandLine": {"type": "string"},
            },
        });
        let field = FormField {
            label: "x".into(),
            field_type: FieldType::JsonBody,
            required: true,
            value: FieldValue::JsonBody(
                serde_json::to_string(&serde_json::json!({"imageId": "img-1"})).unwrap(),
            ),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("x".into()),
            schema,
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        assert!(
            !is_field_filled(&field),
            "missing required `commandLine` → not filled"
        );
    }

    #[test]
    fn test_is_field_filled_jsonbody_all_required_present_is_filled() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["imageId"],
            "properties": {"imageId": {"type": "string"}},
        });
        let field = FormField {
            label: "x".into(),
            field_type: FieldType::JsonBody,
            required: true,
            value: FieldValue::JsonBody(
                serde_json::to_string(&serde_json::json!({"imageId": "img-1"})).unwrap(),
            ),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("x".into()),
            schema,
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        assert!(is_field_filled(&field));
    }

    #[test]
    fn test_is_field_filled_jsonbody_nested_required_recurses() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["timeout"],
            "properties": {
                "timeout": {
                    "type": "object",
                    "required": ["seconds"],
                    "properties": {"seconds": {"type": "integer"}},
                },
            },
        });
        let mk = |v: FieldValue| FormField {
            label: "x".into(),
            field_type: FieldType::JsonBody,
            required: true,
            value: v,
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("x".into()),
            schema: schema.clone(),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        let missing = FieldValue::JsonBody(
            serde_json::to_string(&serde_json::json!({"timeout": {}})).unwrap(),
        );
        let ok = FieldValue::JsonBody(
            serde_json::to_string(&serde_json::json!({"timeout": {"seconds": 30}})).unwrap(),
        );
        assert!(
            !is_field_filled(&mk(missing)),
            "nested required missing → not filled"
        );
        assert!(is_field_filled(&mk(ok)), "nested required present → filled");
    }

    #[test]
    fn test_is_field_filled_jsonbody_required_inside_array_items_enforced() {
        // A required property inside array items must be enforced — mirrors the
        // `regions[].region` case. Empty strings stay acceptable (present + not
        // null); only a missing/null key blocks.
        let schema = serde_json::json!({
            "type": "object",
            "required": ["regions"],
            "properties": {
                "regions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["region"],
                        "properties": {"region": {"type": "string"}},
                    },
                },
            },
        });
        let mk = |v: serde_json::Value| FormField {
            label: "x".into(),
            field_type: FieldType::JsonBody,
            required: true,
            value: FieldValue::JsonBody(serde_json::to_string(&v).unwrap()),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("x".into()),
            schema: schema.clone(),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        assert!(
            !is_field_filled(&mk(serde_json::json!({"regions": [{}]}))),
            "required `region` missing in an array item → not filled"
        );
        assert!(
            is_field_filled(&mk(serde_json::json!({"regions": [{"region": ""}]}))),
            "empty-string `region` is present → filled (server adjudicates empties)"
        );
        assert!(
            is_field_filled(&mk(serde_json::json!({"regions": [{"region": "us-east"}]}))),
            "populated `region` → filled"
        );
    }

    #[test]
    fn test_missing_required_keys_lists_top_level_missing() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["imageId", "commandLine"],
            "properties": {
                "imageId": {"type": "string"},
                "commandLine": {"type": "string"},
            },
        });
        let text = serde_json::to_string(&serde_json::json!({"imageId": "img-1"})).unwrap();
        let missing = missing_required_keys(&text, &schema);
        assert_eq!(missing, vec!["commandLine"]);
    }

    #[test]
    fn test_missing_required_keys_reports_deep_array_and_object_paths() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["regions", "timeout"],
            "properties": {
                "regions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["region", "minServerCount"],
                        "properties": {
                            "region": {"type": "string"},
                            "minServerCount": {"type": "integer"},
                        },
                    },
                },
                "timeout": {
                    "type": "object",
                    "required": ["seconds"],
                    "properties": {"seconds": {"type": "integer"}},
                },
            },
        });
        // regions[0] is missing `region`; regions[1] has it but lacks
        // `minServerCount`; timeout.seconds is missing. Top-level keys are present
        // so the note must name the deep paths instead.
        let text = serde_json::to_string(&serde_json::json!({
            "regions": [
                {"minServerCount": 1},
                {"region": "us-east"},
            ],
            "timeout": {},
        }))
        .unwrap();
        let missing = missing_required_keys(&text, &schema);
        assert_eq!(
            missing,
            vec![
                "regions[0].region",
                "regions[1].minServerCount",
                "timeout.seconds"
            ]
        );
    }

    #[test]
    fn test_all_required_filled_blocks_until_jsonbody_required_present() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["imageId"],
            "properties": {"imageId": {"type": "string"}},
        });
        let mut form = Form::new(
            "t",
            vec![FormField {
                label: "img-deploy".into(),
                field_type: FieldType::JsonBody,
                required: true,
                value: FieldValue::JsonBody(serde_json::to_string(&serde_json::json!({})).unwrap()),
                description: String::new(),
                source: FieldSource::UserInput,
                key: FieldKey::Input("img-deploy".into()),
                schema: schema.clone(),
                read_only: false,
                dynamic: None,
                file_picker: None,
            }],
        );
        assert!(!form.all_required_filled(), "missing required key blocks");
        form.fields[0].value = FieldValue::JsonBody(
            serde_json::to_string(&serde_json::json!({"imageId": "img-1"})).unwrap(),
        );
        assert!(form.all_required_filled(), "all required present");
    }

    #[test]
    fn test_render_field_row_unbracketed_value_has_leading_space() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        // Read-only scalar — unbracketed branch. Leading space aligns the
        // value with bracketed `[value]` rows.
        let field = FormField {
            label: "a".into(),
            field_type: FieldType::Scalar,
            required: false,
            value: FieldValue::Scalar("hi".into()),
            description: String::new(),
            source: FieldSource::WorkflowInput,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type":"string"}),
            read_only: true,
            dynamic: None,
            file_picker: None,
        };
        let form = Form::new("t", vec![field]);
        let mut term = Terminal::new(TestBackend::new(40, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, 1))
            .unwrap();
        let content: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            content.contains(" hi"),
            "leading space before unbracketed value: {content}"
        );
    }

    #[test]
    fn test_focused_row_renders_caret_in_marker_column() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let field = FormField {
            label: "a".into(),
            field_type: FieldType::Scalar,
            required: false,
            value: FieldValue::Scalar("v".into()),
            description: String::new(),
            source: FieldSource::Literal,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type":"string"}),
            read_only: false,
            dynamic: None,
            file_picker: None,
        };
        let mut form = Form::new("t", vec![field]);
        form.focus = 0;
        let mut term = Terminal::new(TestBackend::new(40, 3)).unwrap();
        term.draw(|f| form.render_field_row(f, f.area(), 0, 1))
            .unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            buf.contains("\u{25B8}"),
            "▸ caret rendered for focused row: {buf}"
        );
    }

    #[test]
    fn test_current_hint_returns_submit_description_when_focused() {
        let mut form = Form::new("t", vec![sample_field("a", false, FieldSource::Literal)])
            .with_submit_focusable(true)
            .with_submit_description("Submit and continue.");
        form.focus = form.fields.len(); // Submit slot
        let hint = form.current_hint().expect("Submit-focused hint present");
        assert_eq!(hint.0, "Submit and continue.");
        assert!(!hint.1, "not an error");
    }

    #[test]
    fn test_current_hint_empty_submit_description_returns_none() {
        let mut form = Form::new("t", vec![sample_field("a", false, FieldSource::Literal)])
            .with_submit_focusable(true);
        form.focus = form.fields.len();
        assert!(
            form.current_hint().is_none(),
            "no hint when description is empty"
        );
    }

    #[test]
    fn test_derived_field_renders_with_source_label_in_field_line() {
        use ratatui::{backend::TestBackend, Terminal};

        let field = FormField {
            label: "resource-prefix".into(),
            field_type: FieldType::Scalar,
            required: false,
            value: FieldValue::Scalar("ranked-fleet".into()),
            description: String::new(),
            source: FieldSource::Derived {
                sources: vec!["resourcePrefix".into()],
            },
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type": "string"}),
            read_only: true,
            dynamic: None,
            file_picker: None,
        };
        let form = Form::new("t", vec![field]);
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| form.render_fields(f, f.area())).unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();

        assert!(buf.contains("ranked-fleet"), "resolved value shown: {buf}");
        assert!(
            buf.contains("derived"),
            "word 'derived' in hint label: {buf}"
        );
        // Source names render kebab-cased to match the CLI's flag convention.
        assert!(
            buf.contains("(derived from resource-prefix)"),
            "source name shown: {buf}"
        );
    }

    #[test]
    fn test_field_line_dims_read_only_but_not_editable() {
        use ratatui::style::Color;
        let dim = Some(Color::Indexed(244));

        let mut read_only = sample_field("captured-id", false, FieldSource::UserInput);
        read_only.value = FieldValue::Scalar("abc123".into());
        read_only.read_only = true;
        let line = field_line(&read_only, false, None);
        assert_eq!(line.spans[0].style.fg, dim, "read-only label is dimmed");
        let value_span = line
            .spans
            .iter()
            .find(|s| s.content.contains("abc123"))
            .expect("value span");
        assert_eq!(value_span.style.fg, dim, "read-only value is dimmed");

        let mut editable = sample_field("code", false, FieldSource::UserInput);
        editable.value = FieldValue::Scalar("xyz".into());
        let line2 = field_line(&editable, false, None);
        assert_eq!(
            line2.spans[0].style.fg, None,
            "editable label keeps normal contrast"
        );
    }

    #[test]
    fn test_derived_field_routes_to_workflow_values_section() {
        let derived = FormField {
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
            file_picker: None,
        };
        let editable = sample_field("stat-code", false, FieldSource::Literal);
        let fields = vec![derived, editable];
        let groups = partition_fields_by_source(&fields);
        assert!(
            groups.workflow_values.contains(&0),
            "Derived field in workflow section"
        );
        assert!(
            groups.step_values.contains(&1),
            "Literal field in step section"
        );
    }

    #[test]
    fn test_derived_field_from_step_plan_is_read_only_with_derived_source() {
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
        let form = Form::from_step_plan(&plan);
        let field = &form.fields[0];
        assert!(field.read_only, "Derived field is read-only");
        assert!(
            matches!(&field.source, FieldSource::Derived { sources } if sources == &["resourcePrefix"]),
            "Derived source preserved with correct input names"
        );
    }

    /// `begin_edit` on a Derived read-only field must not open an edit session
    /// and must set a validation note that names the source inputs and contains
    /// the word "derived".
    #[test]
    fn test_begin_edit_on_derived_field_sets_explanatory_note() {
        let mut form = Form::new(
            "t",
            vec![FormField {
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
                file_picker: None,
            }],
        );
        form.focus = 0;
        form.begin_edit();
        assert!(
            form.editing.is_none(),
            "edit must not open on a Derived field"
        );
        let note = form
            .validation_note
            .as_deref()
            .expect("validation note set");
        assert!(
            note.to_lowercase().contains("derived"),
            "note must mention 'derived': {note}"
        );
        assert!(
            note.contains("resourcePrefix"),
            "note must name the first source input: {note}"
        );
        assert!(
            note.contains("teamCount"),
            "note must name the second source input: {note}"
        );
    }

    /// `begin_edit` on a non-Derived read-only field remains a silent no-op
    /// (no edit session, no validation note — the rendered suffix is already
    /// self-explanatory for WorkflowInput / PriorOutput).
    #[test]
    fn test_begin_edit_on_non_derived_read_only_is_silent() {
        let mut form = Form::new(
            "t",
            vec![FormField {
                label: "namespace".into(),
                field_type: FieldType::Scalar,
                required: true,
                value: FieldValue::Scalar("dev".into()),
                description: String::new(),
                source: FieldSource::WorkflowInput,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({"type": "string"}),
                read_only: true,
                dynamic: None,
                file_picker: None,
            }],
        );
        form.focus = 0;
        form.begin_edit();
        assert!(form.editing.is_none(), "no edit on read-only WorkflowInput");
        assert!(
            form.validation_note.is_none(),
            "no note for non-Derived read-only"
        );
    }

    // ── DynamicEnum model tests (Task 10) ────────────────────────────────────

    /// Build a sample `OptionsSource` fixture for dynamic-enum tests.
    fn sample_options_source() -> ags_protocol::workflow::OptionsSource {
        ags_protocol::workflow::OptionsSource {
            operation: ags_protocol::workflow::OperationReference {
                service: ags_protocol::catalogue::ServiceId::new("ams"),
                operation: ags_protocol::catalogue::OperationId::new("ams/admin/images/v1/list"),
            },
            parameters: std::collections::BTreeMap::new(),
            items_path: "$.images".into(),
            value: "$.id".into(),
            label: Some("$.name".into()),
            label_detail: None,
            fallback_description: None,
            filter: None,
        }
    }

    #[test]
    fn test_dynamic_enum_field_carries_state_and_defaults_to_none() {
        use super::*;
        let field = FormField {
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
                source: sample_options_source(),
                deps: vec!["namespace".into()],
                optional_deps: vec![],
                resolved: None,
            }),
            file_picker: None,
        };
        assert!(field.dynamic.is_some());
        assert_eq!(
            field.dynamic.as_ref().unwrap().deps,
            vec!["namespace".to_string()]
        );
    }

    // ── DynamicEnum behaviour tests (Task 11) ────────────────────────────────

    /// Build a dynamic-enum `FormField` fixture with resolved choices.
    fn dynamic_field_resolved(values: &[(&str, &str)]) -> FormField {
        use ags_protocol::workflow::OptionChoice;
        let choices = values
            .iter()
            .map(|(l, v)| OptionChoice {
                label: (*l).into(),
                value: (*v).into(),
            })
            .collect();
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
                source: sample_options_source(),
                deps: vec!["namespace".into()],
                optional_deps: vec![],
                resolved: Some(ResolvedChoices {
                    dep_key: std::collections::BTreeMap::new(),
                    choices,
                    truncated: false,
                }),
            }),
            file_picker: None,
        }
    }

    #[test]
    fn test_dynamic_enum_begin_edit_permits_raw_entry() {
        let mut form = Form::new("t", vec![dynamic_field_resolved(&[("Prod", "img-1")])]);
        form.focus = 0;
        form.begin_edit();
        assert!(form.editing.is_some(), "DynamicEnum permits raw text entry");
        *form.editing.as_mut().unwrap().text_buffer_mut().unwrap() = "typed-uuid".into();
        form.commit_edit();
        assert!(matches!(form.fields[0].value, FieldValue::Enum(Some(ref s)) if s == "typed-uuid"));
    }

    #[test]
    fn test_dynamic_enum_display_resolves_value_to_label_else_raw() {
        let field = {
            let mut f = dynamic_field_resolved(&[("Prod", "img-1")]);
            f.value = FieldValue::Enum(Some("img-1".into()));
            f
        };
        assert_eq!(dynamic_enum_display(&field), "Prod");
        let raw = {
            let mut f = dynamic_field_resolved(&[("Prod", "img-1")]);
            f.value = FieldValue::Enum(Some("typed-uuid".into()));
            f
        };
        assert_eq!(dynamic_enum_display(&raw), "typed-uuid");
    }

    #[test]
    fn test_dynamic_enum_affordance_hints_enter_and_empty() {
        // Resolved, non-empty → "Enter to choose — N options".
        let mut f = dynamic_field_resolved(&[("Prod", "img-1"), ("Stg", "img-2")]);
        f.value = FieldValue::Enum(Some("img-2".into()));
        assert_eq!(
            dynamic_enum_affordance(&f).as_deref(),
            Some(" \u{2039}Enter to choose \u{2014} 2 options\u{203a}")
        );
        // Resolved but empty → free-text hint.
        let empty = {
            let mut f = dynamic_field_resolved(&[]);
            f.value = FieldValue::Enum(None);
            f
        };
        assert_eq!(
            dynamic_enum_affordance(&empty).as_deref(),
            Some(" \u{2039}no matches \u{2014} type a value\u{203a}")
        );
    }

    #[test]
    fn test_dynamic_enum_affordance_unresolved_hints_enter() {
        // Unresolved (never fetched) → still discoverable via Enter.
        let mut f = dynamic_field_resolved(&[("Prod", "img-1")]);
        f.dynamic.as_mut().unwrap().resolved = None;
        assert_eq!(
            dynamic_enum_affordance(&f).as_deref(),
            Some(" \u{2039}Enter to choose\u{203a}")
        );
    }
}
