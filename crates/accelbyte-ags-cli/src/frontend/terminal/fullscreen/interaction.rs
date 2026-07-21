//! `FullscreenInteraction` — workflow interaction for the fullscreen terminal
//! frontend.
//!
//! Holds the same `Rc<RefCell<FullscreenSurface>>` the [`FullscreenFrontend`]
//! draws through, so gather and confirm render *inside* the four-region
//! layout (header + step strip + main + Summary + nav) rather than taking the
//! screen over. Each key redraws via the surface's single `render()`. The
//! borrow discipline is unchanged: the executor calls the frontend and the
//! interaction sequentially, never nested.

use std::cell::RefCell;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::errors::CliError;
use crate::frontend::dynamic_options::DynamicOptionResolver;
use crate::frontend::terminal::dynamic_enums::{
    apply_picker_result, compute_dep_key, deps_satisfied, picker_action, store_resolved,
    PickerAction,
};
use crate::frontend::terminal::form_runner::{
    confirm_skip_outcome, is_ctrl_c, is_optional_skip_key,
};
use crate::frontend::terminal::fullscreen::header::Header;
use crate::frontend::terminal::fullscreen::phases::confirm::ConfirmPanel;
use crate::frontend::terminal::fullscreen::phases::enum_picker::EnumPickerModal;
use crate::frontend::terminal::fullscreen::phases::fields::FieldsPanel;
use crate::frontend::terminal::fullscreen::phases::{running::RunningPanel, Phase};
use crate::frontend::terminal::fullscreen::surface::FullscreenSurface;
use crate::frontend::terminal::inline::form::{FieldValue, Form};
use crate::frontend::terminal::inline::phases::confirm_card::{
    ConfirmAction, ConfirmCard, ConfirmCardPhase,
};
use crate::frontend::terminal::inline::phases::form::{FormPhase, PhaseResult};
// The inline `Phase` trait (provides `on_key`) shares its name with the
// fullscreen `Phase` enum; bring it into scope anonymously for method resolution.
use crate::frontend::terminal::inline::phases::Phase as _;
use crate::frontend::terminal::inline::phases::PhaseStep;
use crate::frontend::ExecutionInteraction;
use ags_protocol::workflow::{
    CompiledStep, GatherResult, StepFieldPlan, StepPreview, StepReviewOutcome, SuppliedInputView,
    WorkflowInputNeeded,
};

/// Workflow interaction handler for the fullscreen terminal frontend.
///
/// Construct with [`FullscreenInteraction::new`], passing the surface handle
/// the [`FullscreenFrontend`](super::frontend::FullscreenFrontend) was built
/// from — do not use struct-literal syntax.
pub struct FullscreenInteraction {
    surface: Rc<RefCell<FullscreenSurface>>,
    /// Present only on a real fullscreen run; drives dynamic-enum option
    /// fetches. `None` in tests / when no resolver was constructed.
    resolver: Option<Box<dyn DynamicOptionResolver>>,
}

impl FullscreenInteraction {
    /// Construct from the shared surface handle. This is the sole assembly
    /// point; pass the same `Rc` the frontend holds so both draw through one
    /// surface.
    pub fn new(surface: Rc<RefCell<FullscreenSurface>>) -> Self {
        Self {
            surface,
            resolver: None,
        }
    }

    /// Builder: attach the dynamic-enum resolver (fullscreen production path).
    pub fn with_resolver(mut self, resolver: Box<dyn DynamicOptionResolver>) -> Self {
        self.resolver = Some(resolver);
        self
    }
}

/// Description shown in the gather-inputs (Phase 1) panel body, beneath the
/// `gather-inputs` heading.
pub(crate) const PHASE_1_DESCRIPTION: &str =
    "Provide the workflow inputs. These values are collected once and fixed for the whole run.";

/// Description shown beneath the `gather-inputs` heading for a single
/// synthesised command, mirroring [`PHASE_1_DESCRIPTION`] for the workflow case.
pub(crate) const COMMAND_INPUTS_DESCRIPTION: &str =
    "Provide the inputs for this command. They are sent in a single request.";

/// The `(step_number, heading, description)` for the gather Inputs box.
///
/// A single synthesised command has no per-step concept — the whole run is
/// "gather the inputs, then send" — so it mirrors the workflow Phase-1 gather
/// box (`Step 0` / `gather-inputs` / a one-line description). A real workflow
/// step keeps its own step number and uses its step description as the heading.
fn gather_box_header(
    header_kind: crate::frontend::terminal::fullscreen::step_strip::HeaderKind,
    step: &CompiledStep,
) -> (usize, String, String) {
    use crate::frontend::terminal::fullscreen::step_strip::HeaderKind;
    if header_kind == HeaderKind::Command {
        (
            0,
            "gather-inputs".to_string(),
            COMMAND_INPUTS_DESCRIPTION.to_string(),
        )
    } else {
        let heading = step
            .description
            .as_deref()
            .unwrap_or("Provide inputs")
            .to_string();
        (step.index + 1, heading, String::new())
    }
}

/// A throwaway phase to swap into `current_phase` while a real one is moved
/// out, and the default to restore after an interaction completes.
fn running_placeholder() -> Phase {
    Phase::Running(RunningPanel {
        step_number: 0,
        step_title: String::new(),
        description: String::new(),
        verb: "Working".into(),
    })
}

/// Gather confirm card: the full three-action set (`Confirm / Back / Cancel`).
/// `Back` re-seeds the form and loops back to `Fields`. The `header` fills the
/// shared slot so the card box lines up with the preceding Fields form.
fn gather_confirm_panel(header: Header, card: ConfirmCard) -> ConfirmPanel {
    ConfirmPanel {
        header,
        card_phase: ConfirmCardPhase::new(card),
    }
}

/// Per-step confirm card: `Confirm / Cancel` only. The `confirm_step` contract
/// is `Result<bool>` — there is no `Back` channel and nothing to edit. The
/// `header` fills the shared slot so the card box lines up with the Running
/// verb box that follows.
fn step_confirm_panel(header: Header, card: ConfirmCard) -> ConfirmPanel {
    ConfirmPanel {
        header,
        card_phase: ConfirmCardPhase::new(card)
            .with_actions(&[ConfirmAction::Confirm, ConfirmAction::Cancel]),
    }
}

/// Per-step confirm card for an optional step: `Confirm / Skip / Cancel`.
/// Like [`step_confirm_panel`] but adds the Skip action so `[s] skip` is
/// offered in the nav bar and the `s` key emits `ConfirmAction::Skip`.
fn step_confirm_skippable_panel(header: Header, card: ConfirmCard) -> ConfirmPanel {
    ConfirmPanel {
        header,
        card_phase: ConfirmCardPhase::new(card).with_actions(&[
            ConfirmAction::Confirm,
            ConfirmAction::Skip,
            ConfirmAction::Cancel,
        ]),
    }
}

/// Failure-gate card: `Retry / Cancel`. Like [`step_confirm_panel`] but the
/// primary button reads "Retry" (there is nothing to confirm — the step failed).
fn step_failure_panel(header: Header, card: ConfirmCard) -> ConfirmPanel {
    ConfirmPanel {
        header,
        card_phase: ConfirmCardPhase::new(card)
            .with_actions(&[ConfirmAction::Retry, ConfirmAction::Cancel]),
    }
}

/// Failure-gate card for a safely-skippable step: `Retry / Skip / Cancel`.
fn step_failure_skippable_panel(header: Header, card: ConfirmCard) -> ConfirmPanel {
    ConfirmPanel {
        header,
        card_phase: ConfirmCardPhase::new(card).with_actions(&[
            ConfirmAction::Retry,
            ConfirmAction::Skip,
            ConfirmAction::Cancel,
        ]),
    }
}

/// Run the shared confirm-card event loop against the phase already swapped into
/// `surface.current_phase`: render, read a key, feed it to the card, and return
/// the completing `ConfirmAction`. Returns `None` for Ctrl-C or a
/// `PhaseStep::Cancelled` (both mean "cancelled"). Does NOT restore
/// `current_phase` — the caller swaps the prior phase back. `on_key` only emits
/// `Done` for actions the panel actually offers, so each caller maps its offered
/// actions and folds the rest into cancel.
fn run_confirm_action_loop(
    surface: &mut FullscreenSurface,
    mut next_key: impl FnMut() -> Result<KeyEvent, CliError>,
) -> Result<Option<ConfirmAction>, CliError> {
    loop {
        surface.render()?;
        let key = next_key()?;
        if is_ctrl_c(key) {
            return Ok(None);
        }
        let key_step = {
            let Phase::Confirm(panel) = &mut surface.current_phase else {
                unreachable!("confirm loop expects a Confirm phase")
            };
            panel.phase_mut().on_key(key)
        };
        match key_step {
            PhaseStep::Continue => {}
            PhaseStep::Cancelled => return Ok(None),
            PhaseStep::Done(action) => return Ok(Some(action)),
        }
    }
}

/// Inner confirm-step loop with an injectable key source, matching the
/// `present_briefing_inner` pattern so tests can script keys without a terminal.
///
/// When `step.is_optional` the card offers `[Confirm, Skip, Cancel]` and `s`
/// emits `StepConfirmOutcome::Skip`. For non-optional steps the card offers
/// only `[Confirm, Cancel]` and `s` is inert (the `offers_skip()` guard in
/// `ConfirmCardPhase::on_key` ensures this).
fn confirm_step_inner(
    step: &CompiledStep,
    preview: &StepPreview,
    surface: &mut FullscreenSurface,
    next_key: impl FnMut() -> Result<KeyEvent, CliError>,
) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
    use ags_protocol::workflow::StepConfirmOutcome;

    // Header slot mirrors every other step — the step id as the heading and
    // the step description as the subtitle (StepPreview.step_label is the
    // description). The request method and URL are backend detail.
    let header = Header::step(
        preview.step_index + 1,
        &preview.step_id,
        &preview.step_label,
    );
    let card = ConfirmCard::new("Confirmation", vec![])
        .with_message("This will modify data. Provide confirmation to run this step.")
        .caution();

    // Optional steps offer Skip so the user can skip the step and continue.
    // Non-optional steps keep Confirm / Cancel only.
    let prior = std::mem::replace(
        &mut surface.current_phase,
        if step.is_optional {
            Phase::Confirm(step_confirm_skippable_panel(header, card))
        } else {
            Phase::Confirm(step_confirm_panel(header, card))
        },
    );

    let action = run_confirm_action_loop(surface, next_key)?;
    surface.current_phase = prior;
    Ok(match action {
        Some(ConfirmAction::Confirm) => StepConfirmOutcome::Proceed,
        Some(ConfirmAction::Skip) => StepConfirmOutcome::Skip,
        // Cancel, Ctrl-C, and the never-offered Back / Retry all give up.
        None | Some(ConfirmAction::Cancel | ConfirmAction::Back | ConfirmAction::Retry) => {
            StepConfirmOutcome::Cancel
        }
    })
}

/// Drive the interactive failure gate for a failed step, reusing the confirm
/// card: the `Confirm` button is the primary "retry" action, `Skip` is offered
/// only when `allow_skip`, and Esc / Cancel gives up (`Cancel`).
fn resolve_step_failure_inner(
    step: &CompiledStep,
    error: &ags_protocol::error::RuntimeError,
    allow_skip: bool,
    surface: &mut FullscreenSurface,
    next_key: impl FnMut() -> Result<KeyEvent, CliError>,
) -> Result<ags_protocol::workflow::StepFailureAction, CliError> {
    use ags_protocol::workflow::StepFailureAction;

    let header = Header::step(
        step.index + 1,
        &step.id,
        step.description.as_deref().unwrap_or("Step failed"),
    );
    let (title, message) =
        crate::frontend::terminal::views::step_failure::step_failure_card_text(error);
    let card = ConfirmCard::new(title, vec![])
        .with_message(message)
        .caution();

    let prior = std::mem::replace(
        &mut surface.current_phase,
        if allow_skip {
            Phase::Confirm(step_failure_skippable_panel(header, card))
        } else {
            Phase::Confirm(step_failure_panel(header, card))
        },
    );

    let action = run_confirm_action_loop(surface, next_key)?;
    surface.current_phase = prior;
    Ok(match action {
        Some(ConfirmAction::Retry) => StepFailureAction::Retry,
        Some(ConfirmAction::Skip) => StepFailureAction::Skip,
        // Cancel, Ctrl-C, and the never-offered Confirm / Back all give up.
        None | Some(ConfirmAction::Cancel | ConfirmAction::Confirm | ConfirmAction::Back) => {
            StepFailureAction::Cancel
        }
    })
}

/// Drive the in-layout JSON-edit phase: edit a `JsonBody` field's value *inside*
/// the main region (the surrounding header / step strip / Summary /
/// Navigation chrome stays drawn) rather than taking over the whole screen.
/// Drives the structured tree editor, an in-panel scalar sub-edit, and the
/// raw-JSON mode. Returns `Some(value)` on save (Ctrl-S) and `None` on cancel
/// (Esc / Ctrl-C). On return, `current_phase` is left as `JsonEdit`; the caller
/// restores the `Fields` phase.
fn drive_json_edit_phase(
    surface: &mut FullscreenSurface,
    form: &Form,
    idx: usize,
) -> Result<Option<serde_json::Value>, CliError> {
    use crate::frontend::terminal::form_runner::{
        apply_json_editor_key, crossterm_next_key, JsonEditorOutcome,
    };
    use crate::frontend::terminal::fullscreen::phases::json_edit::JsonEditPanel;
    use crate::frontend::terminal::inline::json_editor::node::from_schema;
    use crate::frontend::terminal::inline::json_editor::EditorMode;

    let schema = form.fields[idx].schema.clone();
    let current = match &form.fields[idx].value {
        FieldValue::JsonBody(s) if !s.is_empty() => {
            serde_json::from_str(s).unwrap_or(serde_json::Value::Null)
        }
        _ => serde_json::Value::Null,
    };
    let field_name = form.fields[idx].label.clone();
    let required = form.fields[idx].required;
    let root = from_schema(&field_name, &schema, &current, required);
    let baseline = root.to_value();

    surface.current_phase = Phase::JsonEdit(JsonEditPanel {
        title: field_name.clone(),
        root,
        focus: Vec::new(),
        mode: EditorMode::Structured,
        scalar: None,
        baseline,
        scroll_top: std::cell::Cell::new(0),
    });

    loop {
        surface.render()?;
        let key = crossterm_next_key()?;
        if is_ctrl_c(key) {
            return Ok(None);
        }
        let Phase::JsonEdit(panel) = &mut surface.current_phase else {
            unreachable!("inline json editor expects a JsonEdit phase")
        };

        match apply_json_editor_key(
            &mut panel.root,
            &mut panel.focus,
            &mut panel.mode,
            &mut panel.scalar,
            &schema,
            &field_name,
            required,
            key,
        ) {
            JsonEditorOutcome::Continue => continue,
            JsonEditorOutcome::Save(value) => return Ok(Some(value)),
            JsonEditorOutcome::Cancel => return Ok(None),
        }
    }
}

/// Run the dynamic-enum picker modal over the current Fields phase. Returns the
/// chosen value (a choice value or the typed custom value) on Enter, or `None`
/// on Esc / Ctrl-C / a mid-loop resize below the minimum size.
fn drive_enum_picker_modal(
    surface: &mut FullscreenSurface,
    modal: EnumPickerModal,
) -> Result<Option<String>, CliError> {
    use crate::frontend::terminal::form_runner::crossterm_next_key;
    drive_enum_picker_modal_inner(surface, modal, crossterm_next_key)
}

/// Inner loop with an injected key reader so it is testable without a terminal.
/// The surface-held modal is the single authoritative copy for the whole loop.
fn drive_enum_picker_modal_inner(
    surface: &mut FullscreenSurface,
    modal: EnumPickerModal,
    mut next_key: impl FnMut() -> Result<KeyEvent, CliError>,
) -> Result<Option<String>, CliError> {
    surface.enum_picker = Some(modal);
    // The loop `break`s with a `Result` rather than using `?`, so the overlay
    // cleanup below runs on *every* exit — including a `render`/`next_key` error.
    let outcome: Result<Option<String>, CliError> = loop {
        // Viewport guard (pre-render): cancel if the terminal is too small.
        if surface
            .enum_picker
            .as_ref()
            .unwrap()
            .layout(surface.terminal_area())
            .is_none()
        {
            break Ok(None);
        }
        if let Err(e) = surface.render() {
            break Err(e);
        }
        let key = match next_key() {
            Ok(k) => k,
            Err(e) => break Err(e),
        };
        // Viewport guard (post-read): the terminal may have shrunk while the
        // read blocked — re-check before acting, so Enter can't submit from an
        // invisible modal.
        let Some(layout) = surface
            .enum_picker
            .as_ref()
            .unwrap()
            .layout(surface.terminal_area())
        else {
            break Ok(None);
        };
        let page = layout.list_height as usize;

        let modal = surface.enum_picker.as_mut().unwrap();
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => break Ok(None),
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => break Ok(None),
            (KeyCode::Enter, _) => {
                break Ok(modal.selected_value().or_else(|| modal.custom_value()))
            }
            (KeyCode::Up, _) => modal.move_up(),
            (KeyCode::Down, _) => modal.move_down(),
            (KeyCode::PageUp, _) => modal.page_up(page),
            (KeyCode::PageDown, _) => modal.page_down(page),
            (KeyCode::Backspace, _) => modal.pop_char(),
            (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => modal.push_char(c),
            _ => {}
        }
    };
    // Single cleanup path — runs whether the loop broke with Ok or Err.
    surface.enum_picker = None;
    outcome
}

/// Phase-1 form driver. Like `drive_panel_form` but also handles
/// `PhaseResult::OpenEnumPicker`: on activation it fetches the field's choices
/// if not cached (the resolver renders its own loading spinner + handles
/// Esc/Ctrl-C cancel through the shared surface), then opens the scrollable
/// type-to-filter modal picker over the Fields panel. The driver borrows the
/// surface per tick so the resolver can borrow it during the fetch.
fn drive_inputs_panel_form(
    surface: &Rc<RefCell<FullscreenSurface>>,
    resolver: Option<&dyn DynamicOptionResolver>,
    description: String,
    form: Form,
) -> Result<Option<Form>, CliError> {
    use crate::frontend::terminal::form_runner::crossterm_next_key;

    let fields_phase = |form: Form| {
        Phase::Fields(FieldsPanel {
            step_number: 0,
            step_name: "gather-inputs".to_string(),
            description: description.clone(),
            form,
            optional: false,
        })
    };

    surface.borrow_mut().current_phase = fields_phase(form);
    loop {
        surface.borrow_mut().render()?;
        let key = crossterm_next_key()?;
        if is_ctrl_c(key) {
            return Ok(None);
        }
        let step = {
            let mut s = surface.borrow_mut();
            let Phase::Fields(panel) = &mut s.current_phase else {
                unreachable!("drive_inputs_panel_form expects a Fields phase")
            };
            let form = std::mem::replace(panel.form_mut(), Form::new("", vec![]));
            let mut fp = FormPhase::new(form);
            let step = fp.on_key(key);
            *panel.form_mut() = fp.into_form();
            step
        };
        match step {
            PhaseStep::Continue => {}
            PhaseStep::Cancelled => return Ok(None),
            PhaseStep::Done(PhaseResult::Submitted(_)) => {
                let mut s = surface.borrow_mut();
                let Phase::Fields(panel) = &mut s.current_phase else {
                    unreachable!()
                };
                return Ok(Some(std::mem::replace(
                    panel.form_mut(),
                    Form::new("", vec![]),
                )));
            }
            PhaseStep::Done(PhaseResult::OpenJsonEditor(idx)) => {
                let mut form = {
                    let mut s = surface.borrow_mut();
                    let Phase::Fields(panel) = &mut s.current_phase else {
                        unreachable!()
                    };
                    std::mem::replace(panel.form_mut(), Form::new("", vec![]))
                };
                let edited = {
                    let mut s = surface.borrow_mut();
                    drive_json_edit_phase(&mut s, &form, idx)?
                };
                if let Some(value) = edited {
                    let pretty =
                        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                    form.fields[idx].value = FieldValue::JsonBody(pretty);
                }
                surface.borrow_mut().current_phase = fields_phase(form);
            }
            PhaseStep::Done(PhaseResult::OpenEnumPicker(idx)) => {
                // Take the form out to fetch — the production resolver borrows
                // the surface for its spinner, so no surface borrow may be held.
                // Leave a clone behind so the panel still renders the real fields
                // (step box + parameters) under the spinner, not an empty box.
                let mut form = {
                    let mut s = surface.borrow_mut();
                    let Phase::Fields(panel) = &mut s.current_phase else {
                        unreachable!()
                    };
                    let real = std::mem::replace(panel.form_mut(), Form::new("", vec![]));
                    *panel.form_mut() = real.clone();
                    real
                };
                // Always resolve on open. `resolve_dynamic_enum_field` reuses the
                // cache when the dependency values are unchanged (matching
                // `dep_key`) and re-fetches when a dependency changed — so editing
                // `search-query` refires the search instead of reusing stale
                // results.
                resolve_dynamic_enum_field(resolver, &mut form, idx);

                match picker_action(&form, idx) {
                    PickerAction::Open { choices, truncated } => {
                        let title = form.fields[idx].label.clone();
                        let current = match &form.fields[idx].value {
                            FieldValue::Enum(Some(s)) => Some(s.clone()),
                            _ => None,
                        };
                        let modal =
                            EnumPickerModal::new(title, choices, truncated, current.as_deref());
                        let area = surface.borrow().terminal_area();
                        if modal.layout(area).is_none() {
                            // Too small for the modal — inline free-text instead.
                            form.focus = idx;
                            form.begin_edit();
                            surface.borrow_mut().current_phase = fields_phase(form);
                        } else {
                            // Put the real form back so it renders behind the
                            // modal, run the modal, then write the result into
                            // the live panel form.
                            surface.borrow_mut().current_phase = fields_phase(form);
                            let chosen = {
                                let mut s = surface.borrow_mut();
                                drive_enum_picker_modal(&mut s, modal)?
                            };
                            let mut s = surface.borrow_mut();
                            if let Phase::Fields(panel) = &mut s.current_phase {
                                apply_picker_result(panel.form_mut(), idx, chosen);
                            }
                        }
                    }
                    PickerAction::Blocked => {
                        surface.borrow_mut().current_phase = fields_phase(form);
                    }
                }
            }
        }
    }
}

/// Resolve (or reuse the cache for) field `idx`'s dynamic-enum choices, mutating
/// the field's `dynamic.resolved`. The field value is NOT changed here, so
/// opening the picker and cancelling leaves the field's prior value intact; the
/// value is committed only when the user selects a choice. Never errors
/// fatally: a deps-missing field sets the form's `validation_note`; an
/// `Err`/cancel leaves the field on free-text and is not cached.
///
/// Note: the note is set on `form` (which the caller restores into the panel
/// after this returns), NOT on the surface's placeholder form.
fn resolve_dynamic_enum_field(
    resolver: Option<&dyn DynamicOptionResolver>,
    form: &mut Form,
    idx: usize,
) {
    let Some(state) = form.fields[idx].dynamic.as_ref() else {
        return;
    };
    let deps = state.deps.clone();
    let optional_deps = state.optional_deps.clone();
    let source = state.source.clone();

    // Only required deps gate opening.
    if !deps_satisfied(form, &deps) {
        let missing = deps
            .iter()
            .find(|d| !deps_satisfied(form, std::slice::from_ref(*d)))
            .cloned()
            .unwrap_or_else(|| deps.first().cloned().unwrap_or_default());
        form.validation_note = Some(format!(
            "Fill `{}` first to load choices",
            ags_runtime::support::strings::to_kebab_case(&missing)
        ));
        return;
    }

    // Cache key spans required + optional deps, so filling an optional dep later
    // invalidates the cache and re-resolves.
    let all_deps: Vec<String> = deps.iter().chain(optional_deps.iter()).cloned().collect();
    let dep_key = compute_dep_key(form, &all_deps);

    if let Some(resolved) = &form.fields[idx].dynamic.as_ref().unwrap().resolved {
        if resolved.dep_key == dep_key {
            return;
        }
    }

    // An empty optional dep means no search context: open in direct-entry mode
    // (empty choices) without fetching.
    if !deps_satisfied(form, &optional_deps) {
        store_resolved(
            &mut form.fields[idx],
            dep_key,
            ags_protocol::workflow::ResolvedOptions {
                choices: vec![],
                truncated: false,
            },
        );
        return;
    }

    let inputs = form.project_inputs();

    let Some(resolver) = resolver else {
        return;
    };

    match resolver.resolve(&source, &inputs) {
        Ok(resolved) => {
            store_resolved(&mut form.fields[idx], dep_key, resolved);
            // Do NOT auto-populate the field value here. The modal preselects
            // the first choice for highlighting, and the value is committed only
            // when the user presses Enter, so cancelling with Esc leaves the
            // field's prior value untouched.
            //
            // No "no matches" note is set on an empty result. The picker still
            // opens, and its own footer ("No matches · type a value or Esc to
            // cancel") is the correct in-context guidance. A form-level note
            // renders behind the open modal and would wrongly tell the user to
            // "reopen" while they are already standing in it.
        }
        Err(e) => {
            // Non-fatal: surface the classified error (per the spec's "fetch
            // failed" state) so the user sees *why* it failed, then fall back to
            // free text. Not cached, so re-activating the field retries.
            let view = e.view();
            let detail = match view.reason {
                Some(reason) => format!("{} ({reason})", view.message),
                None => view.message,
            };
            form.validation_note = Some(format!(
                "Couldn't load choices: {detail}; type a value manually"
            ));
        }
    }
}

/// Drive a Fields-panel form on the shared surface: render, read keys, run the
/// `FormPhase`, handle the in-panel JSON editor, until submit or cancel. Returns
/// `Some(form)` (the final form, for the caller to project) on submit, `None` on
/// cancel (Esc/Ctrl-C). Leaves `current_phase` as the Fields panel; the caller
/// restores the prior phase.
fn drive_panel_form(
    surface: &mut FullscreenSurface,
    step_number: usize,
    step_name: String,
    description: String,
    form: Form,
) -> Result<Option<Form>, CliError> {
    use crate::frontend::terminal::form_runner::crossterm_next_key;

    // Callers set initial focus before invoking drive_panel_form:
    // - review_step: focus the Submit slot (most common action).
    // - collect_workflow_inputs: focus the first editable input.
    surface.current_phase = Phase::Fields(FieldsPanel {
        step_number,
        step_name: step_name.clone(),
        description: description.clone(),
        form,
        optional: false,
    });
    loop {
        surface.render()?;
        let key = crossterm_next_key()?;
        if is_ctrl_c(key) {
            return Ok(None);
        }
        let step = {
            let Phase::Fields(panel) = &mut surface.current_phase else {
                unreachable!("drive_panel_form expects a Fields phase")
            };
            let form = std::mem::replace(panel.form_mut(), Form::new("", vec![]));
            let mut fp = FormPhase::new(form);
            let step = fp.on_key(key);
            *panel.form_mut() = fp.into_form();
            step
        };
        match step {
            PhaseStep::Continue => {}
            PhaseStep::Cancelled => return Ok(None),
            PhaseStep::Done(PhaseResult::Submitted(_)) => {
                let Phase::Fields(panel) = &mut surface.current_phase else {
                    unreachable!()
                };
                return Ok(Some(std::mem::replace(
                    panel.form_mut(),
                    Form::new("", vec![]),
                )));
            }
            PhaseStep::Done(PhaseResult::OpenJsonEditor(idx)) => {
                let mut form = {
                    let Phase::Fields(panel) = &mut surface.current_phase else {
                        unreachable!()
                    };
                    std::mem::replace(panel.form_mut(), Form::new("", vec![]))
                };
                let edited = drive_json_edit_phase(surface, &form, idx)?;
                if let Some(value) = edited {
                    let pretty =
                        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                    form.fields[idx].value = FieldValue::JsonBody(pretty);
                }
                // `drive_json_edit_phase` leaves `current_phase` as `JsonEdit`,
                // so restore the Fields panel unconditionally with the edited form.
                // (A `if let Phase::Fields` guard would never match here — that was
                // a bug that dropped the edit and panicked on the next key.)
                surface.current_phase = Phase::Fields(FieldsPanel {
                    step_number,
                    step_name: step_name.clone(),
                    description: description.clone(),
                    form,
                    optional: false,
                });
            }
            // Unreachable here: the review-step form is built by
            // `Form::from_step_plan`, which never creates `DynamicEnum` fields.
            // Dynamic-enum resolution happens only in the Phase-1
            // `drive_inputs_panel_form`. Ignored defensively.
            PhaseStep::Done(PhaseResult::OpenEnumPicker(_)) => {}
        }
    }
}

/// Run the Briefing-phase dismiss loop. Returns `Ok(true)` on Enter,
/// `Ok(false)` on Esc, propagates errors from `next_key`. Up/Down adjust
/// `briefing_scroll` on the surface and redraw; other keys are ignored.
///
/// `next_key` is the seam tests use to feed scripted events without a
/// real terminal — production callers pass `crossterm_next_key`.
fn run_briefing_dismiss_loop(
    surface: &RefCell<FullscreenSurface>,
    next_key: &mut dyn FnMut() -> Result<KeyEvent, CliError>,
) -> Result<bool, CliError> {
    loop {
        surface.borrow_mut().render()?;
        let key = next_key()?;
        match key.code {
            KeyCode::Enter => return Ok(true),
            KeyCode::Esc => return Ok(false),
            KeyCode::Up => {
                let mut s = surface.borrow_mut();
                s.briefing_scroll = s.briefing_scroll.saturating_sub(1);
            }
            KeyCode::Down => {
                let mut s = surface.borrow_mut();
                s.briefing_scroll = s.briefing_scroll.saturating_add(1);
            }
            _ => {}
        }
    }
}

/// Wrapper-with-seam form of `present_briefing` — installs `Phase::Briefing`,
/// resets `briefing_scroll`, drives the dismiss loop via `next_key`, then
/// restores the prior phase on every return path (Ok(true)/Ok(false)/Err).
///
/// Production `present_briefing` calls this with `crossterm_next_key`; tests
/// pass scripted key iterators (or one that returns Err) to verify the
/// wrapper semantics without a terminal.
fn present_briefing_inner(
    surface: &RefCell<FullscreenSurface>,
    briefing: &ags_protocol::workflow::WorkflowBriefing,
    workflow_name: &str,
    next_key: &mut dyn FnMut() -> Result<KeyEvent, CliError>,
) -> Result<bool, CliError> {
    use crate::frontend::terminal::fullscreen::phases::briefing::BriefingPanel;

    let prior = {
        let mut s = surface.borrow_mut();
        s.briefing_scroll = 0;
        std::mem::replace(
            &mut s.current_phase,
            Phase::Briefing(BriefingPanel::new(briefing, workflow_name)),
        )
    };
    let result = run_briefing_dismiss_loop(surface, next_key);
    surface.borrow_mut().current_phase = prior;
    result
}

impl ExecutionInteraction for FullscreenInteraction {
    fn present_briefing(
        &mut self,
        briefing: &ags_protocol::workflow::WorkflowBriefing,
        workflow_name: &str,
    ) -> Result<bool, CliError> {
        use crate::frontend::terminal::form_runner::crossterm_next_key;
        let mut next = crossterm_next_key;
        present_briefing_inner(&self.surface, briefing, workflow_name, &mut next)
    }

    fn gather_workflow_inputs(
        &mut self,
        needed: &[WorkflowInputNeeded],
        step_context: &CompiledStep,
        supplied: &[SuppliedInputView],
    ) -> Result<GatherResult, CliError> {
        use crate::frontend::terminal::form_runner::{
            build_gather_confirm_card, crossterm_next_key, gather_cancelled_error,
            sort_gather_fields_by_location,
        };
        use crate::frontend::terminal::inline::form_builder::{build_form_fields, reseed_fields};

        // Nothing to gather — return empty without touching the terminal. The
        // guard runs before the surface borrow so the no-terminal unit test
        // stays green and no empty form is drawn.
        if needed.is_empty() {
            return Ok(GatherResult::default());
        }

        let mut surface = self.surface.borrow_mut();

        let (step_number, step_name, description) =
            gather_box_header(surface.header_kind, step_context);

        let prior = std::mem::replace(&mut surface.current_phase, running_placeholder());
        let mut fields = build_form_fields(needed, supplied);
        // Match the inline gather order: path, query, header, then JSON body.
        sort_gather_fields_by_location(&mut fields, needed, supplied);

        let result = 'gather: loop {
            let form = Form::new(step_name.clone(), fields)
                .with_submit_focusable(true)
                .with_submit_description(
                    crate::frontend::terminal::form_runner::GATHER_REVIEW_SUBMIT_DESCRIPTION,
                );
            surface.current_phase = Phase::Fields(FieldsPanel {
                step_number,
                step_name: step_name.clone(),
                description: description.clone(),
                form,
                optional: false,
            });

            // ── Fields key loop ──────────────────────────────────────────────
            let gathered = loop {
                surface.render()?;
                let key = crossterm_next_key()?;
                if is_ctrl_c(key) {
                    return Err(gather_cancelled_error());
                }

                // Reuse FormPhase::on_key by wrapping the panel's owned form in
                // a transient phase, then taking it back.
                let step = {
                    let Phase::Fields(panel) = &mut surface.current_phase else {
                        unreachable!("gather Fields loop expects a Fields phase")
                    };
                    let form = std::mem::replace(panel.form_mut(), Form::new("", vec![]));
                    let mut fp = FormPhase::new(form);
                    let step = fp.on_key(key);
                    *panel.form_mut() = fp.into_form();
                    step
                };

                match step {
                    PhaseStep::Continue => {}
                    PhaseStep::Cancelled => return Err(gather_cancelled_error()),
                    PhaseStep::Done(PhaseResult::Submitted(r)) => break r,
                    PhaseStep::Done(PhaseResult::OpenJsonEditor(idx)) => {
                        // Edit inside the persistent chrome (header / step strip /
                        // Summary / Navigation) via the JsonEdit phase, so the
                        // editor keeps its nav bar — same as the Phase-1 gather.
                        let mut form = {
                            let Phase::Fields(panel) = &mut surface.current_phase else {
                                unreachable!()
                            };
                            std::mem::replace(panel.form_mut(), Form::new("", vec![]))
                        };
                        let edited = drive_json_edit_phase(&mut surface, &form, idx)?;
                        if let Some(value) = edited {
                            let pretty = serde_json::to_string_pretty(&value)
                                .unwrap_or_else(|_| value.to_string());
                            form.fields[idx].value = FieldValue::JsonBody(pretty);
                        }
                        // `drive_json_edit_phase` leaves `current_phase` as
                        // JsonEdit; restore the Fields phase with the edited form.
                        surface.current_phase = Phase::Fields(FieldsPanel {
                            step_number,
                            step_name: step_name.clone(),
                            description: description.clone(),
                            form,
                            optional: false,
                        });
                    }
                    // Unreachable here: `gather_workflow_inputs` builds its form
                    // via `build_form_fields`, which never creates `DynamicEnum`
                    // fields. Dynamic-enum resolution happens only in the Phase-1
                    // `drive_inputs_panel_form`. Ignored defensively.
                    PhaseStep::Done(PhaseResult::OpenEnumPicker(_)) => {}
                }
            };

            // ── Gather confirm card (Confirm / Back / Cancel) ────────────────
            let card = build_gather_confirm_card(needed, supplied, &gathered);
            let header = Header::step(step_number, &step_name, &description);
            surface.current_phase = Phase::Confirm(gather_confirm_panel(header, card));

            loop {
                surface.render()?;
                let key = crossterm_next_key()?;
                if is_ctrl_c(key) {
                    return Err(gather_cancelled_error());
                }
                let step = {
                    let Phase::Confirm(panel) = &mut surface.current_phase else {
                        unreachable!("gather confirm loop expects a Confirm phase")
                    };
                    panel.phase_mut().on_key(key)
                };
                match step {
                    PhaseStep::Continue => {}
                    PhaseStep::Cancelled => return Err(gather_cancelled_error()),
                    PhaseStep::Done(ConfirmAction::Confirm) => break 'gather gathered,
                    PhaseStep::Done(ConfirmAction::Back) => {
                        fields = reseed_fields(needed, supplied, &gathered);
                        sort_gather_fields_by_location(&mut fields, needed, supplied);
                        continue 'gather;
                    }
                    PhaseStep::Done(ConfirmAction::Cancel) => return Err(gather_cancelled_error()),
                    // The gather confirm card never offers Skip or Retry.
                    PhaseStep::Done(ConfirmAction::Skip | ConfirmAction::Retry) => {
                        return Err(gather_cancelled_error())
                    }
                }
            }
        };

        surface.current_phase = prior;
        Ok(result)
    }

    fn confirm_step(
        &mut self,
        step: &CompiledStep,
        preview: &StepPreview,
    ) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
        use crate::frontend::terminal::form_runner::crossterm_next_key;
        let mut surface = self.surface.borrow_mut();
        confirm_step_inner(step, preview, &mut surface, crossterm_next_key)
    }

    fn resolve_step_failure(
        &mut self,
        step: &CompiledStep,
        error: &ags_protocol::error::RuntimeError,
        allow_skip: bool,
    ) -> Result<ags_protocol::workflow::StepFailureAction, CliError> {
        use crate::frontend::terminal::form_runner::crossterm_next_key;
        let mut surface = self.surface.borrow_mut();
        resolve_step_failure_inner(step, error, allow_skip, &mut surface, crossterm_next_key)
    }

    fn review_step(&mut self, plan: &StepFieldPlan) -> Result<StepReviewOutcome, CliError> {
        if plan.optional {
            self.review_step_optional(plan)
        } else {
            use crate::frontend::terminal::form_runner::review_step_form;
            // Shared orchestration builds the form + projects edits; the closure is
            // the fullscreen-specific drive (in-layout panel + step header).
            review_step_form(plan, |form| {
                let mut surface = self.surface.borrow_mut();
                // The real phase is moved into the form drive; park a placeholder in
                // current_phase meanwhile. The prior phase is intentionally dropped —
                // not restored — because after the review the step dispatches, so this
                // step's running panel (installed explicitly below) should take over.
                surface.current_phase = running_placeholder();
                let result = drive_panel_form(
                    &mut surface,
                    plan.step_index + 1,
                    plan.step_label.clone(),
                    plan.step_description.clone().unwrap_or_default(),
                    form,
                );
                surface.current_phase = Phase::Running(RunningPanel {
                    step_number: plan.step_index + 1,
                    step_title: plan.step_label.clone(),
                    description: plan.step_description.clone().unwrap_or_default(),
                    verb: "Sending request".into(),
                });
                result
            })
        }
    }

    fn collect_workflow_inputs(
        &mut self,
        specs: &[ags_protocol::workflow::WorkflowInputSpec],
        current: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Option<ags_protocol::workflow::CollectOutcome>, CliError> {
        use crate::frontend::terminal::form_runner::collect_inputs_form;
        use crate::frontend::terminal::fullscreen::step_strip::StepState;

        // Shared orchestration builds the form + projects inputs (with dynamic-
        // enum support, which this surface has). The closure is the fullscreen-
        // specific drive: it marks the Inputs strip row Current while gathering
        // and Complete/Skipped on the outcome, and resolves dynamic enums.
        let collected = collect_inputs_form(specs, current, true, |form| {
            self.surface
                .borrow_mut()
                .set_inputs_row_state(StepState::Current);
            let prior = std::mem::replace(
                &mut self.surface.borrow_mut().current_phase,
                running_placeholder(),
            );
            let result = drive_inputs_panel_form(
                &self.surface,
                self.resolver.as_deref(),
                PHASE_1_DESCRIPTION.to_string(),
                form,
            );
            self.surface.borrow_mut().current_phase = prior;
            // Submit → Complete; clean cancel → Skipped (so the strip doesn't
            // leave the row stuck Current). On error, leave it as-is to surface.
            match &result {
                Ok(Some(_)) => self
                    .surface
                    .borrow_mut()
                    .set_inputs_row_state(StepState::Complete),
                Ok(None) => self
                    .surface
                    .borrow_mut()
                    .set_inputs_row_state(StepState::Skipped),
                Err(_) => {}
            }
            result
        })?;
        Ok(collected
            .map(|(inputs, run_mode)| ags_protocol::workflow::CollectOutcome { inputs, run_mode }))
    }
}

impl FullscreenInteraction {
    /// Drive the review-step form for an optional step: same as the non-optional
    /// path but the `FieldsPanel` carries `optional: true` (so the nav bar shows
    /// `[s] skip`) and pressing `s` returns `StepReviewOutcome::Skip` immediately
    /// without the form needing to submit. A zero-field plan still renders the
    /// step header and the Submit button — it never early-returns on an empty
    /// field list.
    fn review_step_optional(
        &mut self,
        plan: &StepFieldPlan,
    ) -> Result<StepReviewOutcome, CliError> {
        use crate::frontend::terminal::form_runner::crossterm_next_key;
        let mut surface = self.surface.borrow_mut();
        review_step_optional_inner(plan, &mut surface, crossterm_next_key)
    }
}

/// Drive an optional step's review form, reading keys from `next_key` (real
/// terminal in production, scripted in tests — mirrors `confirm_step_inner`).
///
/// Pressing `s` skips the step, but only when no field editor is open: while a
/// scalar field is in edit mode, `s` must reach the edit buffer so text inputs
/// stay typeable.
fn review_step_optional_inner(
    plan: &StepFieldPlan,
    surface: &mut FullscreenSurface,
    mut next_key: impl FnMut() -> Result<KeyEvent, CliError>,
) -> Result<StepReviewOutcome, CliError> {
    use crate::frontend::terminal::form_runner::STEP_REVIEW_SUBMIT_DESCRIPTION;

    let step_number = plan.step_index + 1;
    let step_name = plan.step_label.clone();
    let description = plan.step_description.clone().unwrap_or_default();

    let mut form = Form::from_step_plan(plan)
        .with_submit_description(STEP_REVIEW_SUBMIT_DESCRIPTION)
        .with_confirm_skip_buttons(true);
    form.focus_submit_if_available();

    surface.current_phase = Phase::Fields(FieldsPanel {
        step_number,
        step_name: step_name.clone(),
        description: description.clone(),
        form,
        optional: true,
    });

    let outcome = loop {
        surface.render()?;
        let key = next_key()?;
        if is_ctrl_c(key) {
            break StepReviewOutcome::Cancel;
        }
        // `s` skips this optional step — the guard rule lives in the shared
        // `is_optional_skip_key` so it cannot drift from the inline twin.
        let in_edit = {
            let Phase::Fields(panel) = &surface.current_phase else {
                unreachable!("review_step_optional expects a Fields phase")
            };
            panel.form.is_editing()
        };
        if is_optional_skip_key(key, in_edit) {
            break StepReviewOutcome::Skip;
        }
        let step_result = {
            let Phase::Fields(panel) = &mut surface.current_phase else {
                unreachable!("review_step_optional expects a Fields phase")
            };
            let current_form = std::mem::replace(panel.form_mut(), Form::new("", vec![]));
            let mut fp = FormPhase::new(current_form);
            let step = fp.on_key(key);
            *panel.form_mut() = fp.into_form();
            step
        };
        match step_result {
            PhaseStep::Continue => {}
            PhaseStep::Cancelled => break StepReviewOutcome::Cancel,
            PhaseStep::Done(PhaseResult::Submitted(_)) => {
                let Phase::Fields(panel) = &mut surface.current_phase else {
                    unreachable!()
                };
                let submitted = std::mem::replace(panel.form_mut(), Form::new("", vec![]));
                break confirm_skip_outcome(&submitted, plan);
            }
            PhaseStep::Done(PhaseResult::OpenJsonEditor(idx)) => {
                let mut current_form = {
                    let Phase::Fields(panel) = &mut surface.current_phase else {
                        unreachable!()
                    };
                    std::mem::replace(panel.form_mut(), Form::new("", vec![]))
                };
                let edited = drive_json_edit_phase(surface, &current_form, idx)?;
                if let Some(value) = edited {
                    let pretty =
                        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                    current_form.fields[idx].value = FieldValue::JsonBody(pretty);
                }
                surface.current_phase = Phase::Fields(FieldsPanel {
                    step_number,
                    step_name: step_name.clone(),
                    description: description.clone(),
                    form: current_form,
                    optional: true,
                });
            }
            // Unreachable: `Form::from_step_plan` never creates DynamicEnum fields.
            PhaseStep::Done(PhaseResult::OpenEnumPicker(_)) => {}
        }
    };

    // On a skip the step never dispatches, so don't flash "Sending request".
    let verb = if matches!(outcome, StepReviewOutcome::Skip) {
        "Skipping"
    } else {
        "Sending request"
    };
    surface.current_phase = Phase::Running(RunningPanel {
        step_number,
        step_title: step_name,
        description,
        verb: verb.into(),
    });

    Ok(outcome)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{
        confirm_step_inner, gather_confirm_panel, resolve_step_failure_inner,
        review_step_optional_inner, step_confirm_panel, FullscreenInteraction,
    };
    use crate::frontend::terminal::fullscreen::header::Header;
    use crate::frontend::terminal::fullscreen::phases::Phase;
    use crate::frontend::terminal::fullscreen::surface::FullscreenSurface;
    use crate::frontend::terminal::inline::phases::confirm_card::{ConfirmAction, ConfirmCard};
    use crate::frontend::terminal::inline::phases::Phase as _;
    use crate::frontend::terminal::inline::phases::PhaseStep;
    use crate::frontend::ExecutionInteraction;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn make_surface() -> Rc<RefCell<FullscreenSurface>> {
        Rc::new(RefCell::new(FullscreenSurface::without_terminal()))
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn picker(current: Option<&str>) -> super::EnumPickerModal {
        use ags_protocol::workflow::OptionChoice;
        super::EnumPickerModal::new(
            "Pick".into(),
            vec![
                OptionChoice {
                    label: "alpha".into(),
                    value: "a".into(),
                },
                OptionChoice {
                    label: "Mike".into(),
                    value: "m".into(),
                },
                OptionChoice {
                    label: "Zeta".into(),
                    value: "z".into(),
                },
            ],
            false,
            current,
        )
    }

    #[test]
    fn test_modal_filter_then_enter_returns_value() {
        let mut surface = FullscreenSurface::without_terminal_sized(80, 24);
        // pop() yields from the end, so script in reverse: 'm', 'i', Enter.
        let mut keys = vec![
            key(KeyCode::Enter),
            key(KeyCode::Char('i')),
            key(KeyCode::Char('m')),
        ];
        let chosen = super::drive_enum_picker_modal_inner(&mut surface, picker(None), || {
            Ok(keys.pop().unwrap())
        })
        .unwrap();
        assert_eq!(chosen.as_deref(), Some("m"));
        assert!(surface.enum_picker.is_none(), "overlay cleared on return");
    }

    #[test]
    fn test_modal_esc_returns_none() {
        let mut surface = FullscreenSurface::without_terminal_sized(80, 24);
        let chosen = super::drive_enum_picker_modal_inner(&mut surface, picker(Some("a")), || {
            Ok(key(KeyCode::Esc))
        })
        .unwrap();
        assert_eq!(chosen, None);
    }

    #[test]
    fn test_modal_zero_match_enter_returns_custom_value() {
        let mut surface = FullscreenSurface::without_terminal_sized(80, 24);
        let mut keys = vec![key(KeyCode::Enter), key(KeyCode::Char('q'))];
        let chosen = super::drive_enum_picker_modal_inner(&mut surface, picker(None), || {
            Ok(keys.pop().unwrap())
        })
        .unwrap();
        assert_eq!(chosen.as_deref(), Some("q"));
    }

    #[test]
    fn test_modal_resize_below_minimum_during_read_cancels() {
        let mut surface = FullscreenSurface::without_terminal_sized(80, 24);
        let handle = surface.area_override_handle();
        // On the key read that would submit, first shrink the terminal, then
        // return Enter. The post-read viewport guard must cancel before Enter.
        let chosen = super::drive_enum_picker_modal_inner(&mut surface, picker(Some("a")), || {
            handle.set(ratatui::layout::Rect::new(0, 0, 4, 4));
            Ok(key(KeyCode::Enter))
        })
        .unwrap();
        assert_eq!(chosen, None, "no submit from a shrunk modal");
        assert!(surface.enum_picker.is_none());
    }

    #[test]
    fn test_modal_clears_overlay_when_key_reader_errors() {
        let mut surface = FullscreenSurface::without_terminal_sized(80, 24);
        let result = super::drive_enum_picker_modal_inner(&mut surface, picker(None), || {
            Err(crate::errors::CliError::Usage {
                message: "boom".into(),
                metadata: None,
            })
        });
        assert!(result.is_err(), "the key-reader error propagates");
        assert!(
            surface.enum_picker.is_none(),
            "overlay is cleared even when the loop exits via Err"
        );
    }

    #[test]
    fn test_picker_action_open_for_resolved_nonempty() {
        use crate::frontend::dynamic_options::CannedResolver;
        use ags_protocol::workflow::OptionChoice;
        let mut form = ns_and_dynamic_form(Some("dev"));
        let resolver = CannedResolver::ok(vec![
            OptionChoice {
                label: "Prod".into(),
                value: "img-1".into(),
            },
            OptionChoice {
                label: "Stg".into(),
                value: "img-2".into(),
            },
        ]);
        super::resolve_dynamic_enum_field(Some(&resolver), &mut form, 1);
        match super::picker_action(&form, 1) {
            super::PickerAction::Open { choices, truncated } => {
                assert_eq!(choices.len(), 2);
                assert!(!truncated);
            }
            _ => panic!("expected Open for a resolved non-empty list"),
        }
    }

    #[test]
    fn test_picker_action_opens_empty_for_resolved_empty() {
        use crate::frontend::dynamic_options::CannedResolver;
        let mut form = ns_and_dynamic_form(Some("dev"));
        let resolver = CannedResolver::ok(vec![]); // resolves to an empty list
        super::resolve_dynamic_enum_field(Some(&resolver), &mut form, 1);
        // A resolved-but-empty result opens the modal with no choices (it shows
        // "<no matches>"), rather than silently falling back to inline free-text.
        match super::picker_action(&form, 1) {
            super::PickerAction::Open { choices, .. } => {
                assert!(choices.is_empty(), "empty result opens an empty picker");
            }
            _ => panic!("expected Open (empty) for a resolved empty list"),
        }
    }

    #[test]
    fn test_picker_action_blocked_for_unresolved() {
        let form = ns_and_dynamic_form(Some("dev")); // never resolved
        assert!(matches!(
            super::picker_action(&form, 1),
            super::PickerAction::Blocked
        ));
    }

    #[test]
    fn test_resolve_refetches_when_dependency_changes() {
        // The picker open handler always calls `resolve_dynamic_enum_field`; this
        // guards the mechanism it relies on: a changed dependency value re-fetches
        // (new `dep_key`) rather than reusing the cached result. Without this,
        // editing `search-query` would not refire the search.
        use crate::frontend::dynamic_options::CannedResolver;
        use crate::frontend::terminal::inline::form::FieldValue;
        use ags_protocol::workflow::OptionChoice;
        let mut form = ns_and_dynamic_form(Some("dev"));
        let resolver = CannedResolver::ok(vec![OptionChoice {
            label: "A".into(),
            value: "a".into(),
        }]);
        super::resolve_dynamic_enum_field(Some(&resolver), &mut form, 1);
        let key1 = form.fields[1]
            .dynamic
            .as_ref()
            .unwrap()
            .resolved
            .as_ref()
            .unwrap()
            .dep_key
            .clone();

        // Change the dependency and re-resolve.
        form.fields[0].value = FieldValue::Scalar("prod".into());
        super::resolve_dynamic_enum_field(Some(&resolver), &mut form, 1);
        let key2 = form.fields[1]
            .dynamic
            .as_ref()
            .unwrap()
            .resolved
            .as_ref()
            .unwrap()
            .dep_key
            .clone();

        assert_ne!(
            key1, key2,
            "a changed dependency must re-fetch and update the dep_key"
        );
    }

    #[test]
    fn test_apply_picker_result_writes_value_and_marks_user_input() {
        use crate::frontend::terminal::inline::form::{FieldSource, FieldValue};
        let mut form = ns_and_dynamic_form(Some("dev"));
        super::apply_picker_result(&mut form, 1, Some("img-9".to_string()));
        assert!(matches!(&form.fields[1].value, FieldValue::Enum(Some(v)) if v == "img-9"));
        assert!(matches!(form.fields[1].source, FieldSource::UserInput));
    }

    #[test]
    fn test_apply_picker_result_none_leaves_field_untouched() {
        use crate::frontend::terminal::inline::form::FieldValue;
        let mut form = ns_and_dynamic_form(Some("dev"));
        form.fields[1].value = FieldValue::Enum(Some("keep".to_string()));
        super::apply_picker_result(&mut form, 1, None);
        assert!(matches!(&form.fields[1].value, FieldValue::Enum(Some(v)) if v == "keep"));
    }

    fn sample_card() -> ConfirmCard {
        ConfirmCard::new("POST /iam/v3/x", vec![("namespace".into(), "ns".into())])
    }

    fn sample_header() -> Header {
        Header::step(1, "Sample step", "")
    }

    // ── Task 4.1: confirm action-set behaviour ──────────────────────────────

    #[test]
    fn test_gather_confirm_panel_offers_confirm_back_cancel() {
        let panel = gather_confirm_panel(sample_header(), sample_card());
        assert_eq!(
            panel.card_phase.actions(),
            &[
                ConfirmAction::Confirm,
                ConfirmAction::Back,
                ConfirmAction::Cancel
            ]
        );
    }

    #[test]
    fn test_step_confirm_panel_offers_confirm_cancel_only() {
        let panel = step_confirm_panel(sample_header(), sample_card());
        assert_eq!(
            panel.card_phase.actions(),
            &[ConfirmAction::Confirm, ConfirmAction::Cancel]
        );
    }

    #[test]
    fn test_gather_confirm_back_signals_back_to_edit() {
        // `b` emits the Back signal the gather loop maps to re-seeding the form
        // and re-entering Fields.
        let mut panel = gather_confirm_panel(sample_header(), sample_card());
        assert!(matches!(
            panel.card_phase.on_key(key(KeyCode::Char('b'))),
            PhaseStep::Done(ConfirmAction::Back)
        ));
    }

    #[test]
    fn test_step_confirm_enter_on_confirm_signals_true() {
        // Default focus = Confirm; Enter → Confirm signal → confirm_step Ok(true).
        let mut panel = step_confirm_panel(sample_header(), sample_card());
        assert!(matches!(
            panel.card_phase.on_key(key(KeyCode::Enter)),
            PhaseStep::Done(ConfirmAction::Confirm)
        ));
    }

    #[test]
    fn test_step_confirm_cancel_signals_false() {
        // Esc cancels the per-step confirm → confirm_step Ok(false).
        let mut panel = step_confirm_panel(sample_header(), sample_card());
        assert!(matches!(
            panel.card_phase.on_key(key(KeyCode::Esc)),
            PhaseStep::Cancelled
        ));
    }

    #[test]
    fn test_surface_confirm_phase_carries_offered_actions() {
        // The interaction sets surface.current_phase to a Confirm panel; assert
        // the offered set is reachable on the surface's current_phase.
        let mut surface = FullscreenSurface::without_terminal();
        surface.current_phase = Phase::Confirm(step_confirm_panel(sample_header(), sample_card()));
        let Phase::Confirm(panel) = &surface.current_phase else {
            panic!("expected Confirm phase");
        };
        assert_eq!(
            panel.card_phase.actions(),
            &[ConfirmAction::Confirm, ConfirmAction::Cancel]
        );
    }

    #[test]
    fn test_fullscreen_interaction_constructs_from_surface() {
        let surface = make_surface();
        let _interaction = FullscreenInteraction::new(Rc::clone(&surface));
        // Both the original Rc and the one inside FullscreenInteraction point
        // to the same allocation: strong_count == 2.
        assert_eq!(Rc::strong_count(&surface), 2);
    }

    /// `gather_workflow_inputs` with an empty `needed` slice returns `Ok` with
    /// an empty map and never touches the terminal.
    #[test]
    fn test_fullscreen_interaction_gather_empty_needed_returns_empty_map() {
        let surface = make_surface();
        let mut interaction = FullscreenInteraction::new(Rc::clone(&surface));
        let step = {
            use ags_protocol::catalogue::{OperationId, ServiceId};
            use ags_protocol::workflow::{CompiledStep, OperationReference};
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
        };
        let result = interaction.gather_workflow_inputs(&[], &step, &[]).unwrap();
        assert!(result.slot_values.is_empty());
        assert!(result.input_overrides.is_empty());
    }

    fn compiled_step(
        index: usize,
        description: Option<&str>,
    ) -> ags_protocol::workflow::CompiledStep {
        use ags_protocol::catalogue::{OperationId, ServiceId};
        use ags_protocol::workflow::{CompiledStep, OperationReference};
        CompiledStep {
            id: "main".to_string(),
            index,
            description: description.map(|s| s.to_string()),
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

    #[test]
    fn test_gather_box_header_command_mirrors_phase1() {
        use crate::frontend::terminal::fullscreen::step_strip::HeaderKind;
        // A single command's gather box reads `Step 0` / `gather-inputs` /
        // the command description — mirroring the workflow Phase-1 box — and
        // ignores the synthesised step's own number/description.
        let step = compiled_step(0, None);
        let (number, heading, description) = super::gather_box_header(HeaderKind::Command, &step);
        assert_eq!(number, 0);
        assert_eq!(heading, "gather-inputs");
        assert_eq!(description, super::COMMAND_INPUTS_DESCRIPTION);
    }

    #[test]
    fn test_gather_box_header_workflow_uses_step_number_and_description() {
        use crate::frontend::terminal::fullscreen::step_strip::HeaderKind;
        // A real workflow step keeps its 1-based number and uses its own
        // description as the heading, with no separate description line.
        let step = compiled_step(2, Some("Create the fleet"));
        let (number, heading, description) = super::gather_box_header(HeaderKind::Workflow, &step);
        assert_eq!(number, 3, "1-based step number");
        assert_eq!(heading, "Create the fleet");
        assert!(description.is_empty());
    }

    #[test]
    fn test_gather_box_header_workflow_falls_back_when_no_description() {
        use crate::frontend::terminal::fullscreen::step_strip::HeaderKind;
        let step = compiled_step(0, None);
        let (_, heading, _) = super::gather_box_header(HeaderKind::Workflow, &step);
        assert_eq!(heading, "Provide inputs");
    }

    #[test]
    fn test_build_inputs_form_seam() {
        use crate::frontend::terminal::inline::form_builder::build_inputs_form;
        use ags_protocol::workflow::WorkflowInputSpec;
        use std::collections::BTreeMap;
        let specs = vec![WorkflowInputSpec {
            name: "namespace".into(),
            description: None,
            schema: Some(serde_json::json!({"type":"string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }];
        let form = build_inputs_form(&specs, &BTreeMap::new(), true);
        assert_eq!(form.len(), 1);
        assert!(form[0].required);
    }

    #[test]
    fn test_phase1_description_constant_is_phrased_for_users() {
        assert!(super::PHASE_1_DESCRIPTION.starts_with("Provide the workflow inputs"));
        assert!(super::PHASE_1_DESCRIPTION.contains("once"));
    }

    #[test]
    fn test_review_step_default_proceeds_without_terminal_guarded() {
        // Driving the review loop needs a terminal, so assert the panel-
        // construction seam instead: build the form from the plan.
        use crate::frontend::terminal::inline::form::Form;
        use ags_protocol::workflow::{
            StepField, StepFieldId, StepFieldLocation, StepFieldPlan, StepFieldSource,
        };
        let plan = StepFieldPlan {
            step_index: 0,
            step_label: "s".into(),
            step_description: None,
            optional: false,
            fields: vec![StepField {
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
            }],
        };
        let form = Form::from_step_plan(&plan);
        assert_eq!(form.fields.len(), 1);
        assert!(form.submit_focusable);
    }

    // ── Task 12 b-d: run_briefing_dismiss_loop key behaviour ────────────────

    #[test]
    fn test_run_briefing_dismiss_loop_enter_returns_true() {
        use crate::frontend::terminal::fullscreen::phases::briefing::BriefingPanel;
        use ags_protocol::workflow::WorkflowBriefing;

        let surface = RefCell::new(FullscreenSurface::without_terminal());
        surface.borrow_mut().current_phase = Phase::Briefing(BriefingPanel::new(
            &WorkflowBriefing {
                overview: "x".into(),
                prerequisites: vec![],
                creates: vec![],
            },
            "WF",
        ));
        let mut script = vec![Ok(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))].into_iter();
        let mut next = move || script.next().expect("script exhausted");
        let result = super::run_briefing_dismiss_loop(&surface, &mut next).expect("ok");
        assert!(result);
    }

    #[test]
    fn test_run_briefing_dismiss_loop_esc_returns_false() {
        use crate::frontend::terminal::fullscreen::phases::briefing::BriefingPanel;
        use ags_protocol::workflow::WorkflowBriefing;

        let surface = RefCell::new(FullscreenSurface::without_terminal());
        surface.borrow_mut().current_phase = Phase::Briefing(BriefingPanel::new(
            &WorkflowBriefing {
                overview: "x".into(),
                prerequisites: vec![],
                creates: vec![],
            },
            "WF",
        ));
        let mut script = vec![Ok(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))].into_iter();
        let mut next = move || script.next().expect("script exhausted");
        let result = super::run_briefing_dismiss_loop(&surface, &mut next).expect("ok");
        assert!(!result);
    }

    #[test]
    fn test_run_briefing_dismiss_loop_down_increments_scroll_then_enter() {
        use crate::frontend::terminal::fullscreen::phases::briefing::BriefingPanel;
        use ags_protocol::workflow::WorkflowBriefing;

        // Tall briefing so max_offset >> Down-key count; clamp never fires
        // and increments stick.
        let overview: String = (0..40)
            .map(|i| format!("paragraph {i}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let surface = RefCell::new(FullscreenSurface::without_terminal());
        surface.borrow_mut().current_phase = Phase::Briefing(BriefingPanel::new(
            &WorkflowBriefing {
                overview,
                prerequisites: vec![],
                creates: vec![],
            },
            "WF",
        ));
        assert_eq!(surface.borrow().briefing_scroll, 0);

        let mut script = vec![
            Ok(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            Ok(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            Ok(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            Ok(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        ]
        .into_iter();
        let mut next = move || script.next().expect("script exhausted");
        let result = super::run_briefing_dismiss_loop(&surface, &mut next).expect("ok");
        assert!(result);
        assert_eq!(surface.borrow().briefing_scroll, 3);
    }

    #[test]
    fn test_run_briefing_dismiss_loop_up_saturates_at_zero() {
        use crate::frontend::terminal::fullscreen::phases::briefing::BriefingPanel;
        use ags_protocol::workflow::WorkflowBriefing;

        let surface = RefCell::new(FullscreenSurface::without_terminal());
        surface.borrow_mut().current_phase = Phase::Briefing(BriefingPanel::new(
            &WorkflowBriefing {
                overview: "short".into(),
                prerequisites: vec![],
                creates: vec![],
            },
            "WF",
        ));
        assert_eq!(surface.borrow().briefing_scroll, 0);

        let mut script = vec![
            Ok(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            Ok(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            Ok(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        ]
        .into_iter();
        let mut next = move || script.next().expect("script exhausted");
        super::run_briefing_dismiss_loop(&surface, &mut next).expect("ok");
        assert_eq!(surface.borrow().briefing_scroll, 0);
    }

    // ── Task 14: dep-key helpers + resolver ────────────────────────────────

    #[test]
    fn test_dep_key_built_with_schema_coercion() {
        use crate::frontend::terminal::inline::form::{
            FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        let ns = FormField {
            label: "namespace".into(),
            field_type: FieldType::Scalar,
            required: true,
            value: FieldValue::Scalar("dev".into()),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("namespace".into()),
            schema: serde_json::json!({"type": "string"}),
            read_only: false,
            dynamic: None,
        };
        let form = Form::new("t", vec![ns]);
        let key = super::compute_dep_key(&form, &["namespace".to_string()]);
        assert_eq!(key.get("namespace"), Some(&serde_json::json!("dev")));
    }

    #[test]
    fn test_cache_hit_when_dep_key_unchanged() {
        use std::collections::BTreeMap;
        let mut a = BTreeMap::new();
        a.insert("namespace".to_string(), serde_json::json!("dev"));
        let b = a.clone();
        assert_eq!(a, b);
        let mut c = a.clone();
        c.insert("namespace".to_string(), serde_json::json!("prod"));
        assert_ne!(a, c);
    }

    #[test]
    fn test_deps_satisfied_requires_all_filled() {
        use crate::frontend::terminal::inline::form::{
            FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        let ns = FormField {
            label: "namespace".into(),
            field_type: FieldType::Scalar,
            required: true,
            value: FieldValue::Empty,
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("namespace".into()),
            schema: serde_json::json!({"type": "string"}),
            read_only: false,
            dynamic: None,
        };
        let mut form = Form::new("t", vec![ns]);
        assert!(!super::deps_satisfied(&form, &["namespace".to_string()]));
        form.fields[0].value = FieldValue::Scalar("dev".into());
        assert!(super::deps_satisfied(&form, &["namespace".to_string()]));
    }

    struct RecordingResolver {
        calls: std::cell::Cell<usize>,
        last_inputs: std::cell::RefCell<std::collections::BTreeMap<String, serde_json::Value>>,
    }
    impl RecordingResolver {
        fn new() -> Self {
            Self {
                calls: std::cell::Cell::new(0),
                last_inputs: std::cell::RefCell::new(Default::default()),
            }
        }
    }
    impl crate::frontend::dynamic_options::DynamicOptionResolver for RecordingResolver {
        fn resolve(
            &self,
            _source: &ags_protocol::workflow::OptionsSource,
            inputs: &std::collections::BTreeMap<String, serde_json::Value>,
        ) -> Result<ags_protocol::workflow::ResolvedOptions, crate::errors::CliError> {
            self.calls.set(self.calls.get() + 1);
            *self.last_inputs.borrow_mut() = inputs.clone();
            Ok(ags_protocol::workflow::ResolvedOptions {
                choices: vec![],
                truncated: false,
            })
        }
    }

    /// Build a [namespace, searchQuery, userId] form. `userId` is a DynamicEnum
    /// with a required `namespace` dep and an optional `searchQuery` dep.
    fn ns_query_user_form(
        namespace: Option<&str>,
        query: Option<&str>,
    ) -> crate::frontend::terminal::inline::form::Form {
        use crate::frontend::terminal::inline::form::{
            DynamicEnumState, FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        use ags_protocol::workflow::{OperationReference, OptionParameterBinding, OptionsSource};
        let scalar = |v: Option<&str>| match v {
            Some(s) => FieldValue::Scalar(s.into()),
            None => FieldValue::Empty,
        };
        let field = |name: &str, ft: FieldType, val: FieldValue, dynamic| FormField {
            label: name.into(),
            field_type: ft,
            required: false,
            value: val,
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input(name.into()),
            schema: serde_json::json!({"type": "string"}),
            read_only: false,
            dynamic,
        };
        let ns = field("namespace", FieldType::Scalar, scalar(namespace), None);
        let query_field = field("searchQuery", FieldType::Scalar, scalar(query), None);
        let user = field(
            "userId",
            FieldType::DynamicEnum,
            FieldValue::Enum(None),
            Some(DynamicEnumState {
                source: OptionsSource {
                    operation: OperationReference {
                        service: ags_protocol::catalogue::ServiceId::new("iam"),
                        operation: ags_protocol::catalogue::OperationId::new(
                            "iam/admin/users/v3/search",
                        ),
                    },
                    parameters: std::collections::BTreeMap::from([
                        (
                            "namespace".to_string(),
                            OptionParameterBinding::FromInput("namespace".into()),
                        ),
                        (
                            "query".to_string(),
                            OptionParameterBinding::FromInputOptional("searchQuery".into()),
                        ),
                    ]),
                    items_path: "$.data".into(),
                    value: "$.userId".into(),
                    label: None,
                    label_detail: None,
                    fallback_description: None,
                    filter: None,
                },
                deps: vec!["namespace".into()],
                optional_deps: vec!["searchQuery".into()],
                resolved: None,
            }),
        );
        Form::new("t", vec![ns, query_field, user])
    }

    #[test]
    fn test_picker_gating_optional_empty_direct_entry() {
        let resolver = RecordingResolver::new();
        let mut form = ns_query_user_form(Some("prod"), None); // required filled, optional empty
        super::resolve_dynamic_enum_field(Some(&resolver), &mut form, 2);
        assert_eq!(resolver.calls.get(), 0, "no fetch when optional dep empty");
        let resolved = form.fields[2].dynamic.as_ref().unwrap().resolved.as_ref();
        assert!(
            resolved.is_some(),
            "resolved set to empty choices for direct entry"
        );
        assert!(resolved.unwrap().choices.is_empty());
        assert!(matches!(
            super::picker_action(&form, 2),
            super::PickerAction::Open { .. }
        ));
    }

    #[test]
    fn test_picker_gating_optional_filled_fetches() {
        let resolver = RecordingResolver::new();
        let mut form = ns_query_user_form(Some("prod"), Some("ada")); // required + optional filled
        super::resolve_dynamic_enum_field(Some(&resolver), &mut form, 2);
        assert_eq!(
            resolver.calls.get(),
            1,
            "fetch runs when required + optional deps filled"
        );
        assert_eq!(
            resolver.last_inputs.borrow().get("searchQuery"),
            Some(&serde_json::json!("ada")),
            "the optional search param is included in the fetch inputs"
        );
    }

    #[test]
    fn test_picker_gating_required_missing_blocks() {
        let resolver = RecordingResolver::new();
        let mut form = ns_query_user_form(None, Some("ada")); // required namespace empty
        super::resolve_dynamic_enum_field(Some(&resolver), &mut form, 2);
        assert_eq!(
            resolver.calls.get(),
            0,
            "required dep missing must not fetch"
        );
        assert!(
            form.fields[2].dynamic.as_ref().unwrap().resolved.is_none(),
            "stays unresolved"
        );
        assert!(matches!(
            super::picker_action(&form, 2),
            super::PickerAction::Blocked
        ));
        assert!(
            form.validation_note.is_some(),
            "a fill-namespace note is set"
        );
    }

    /// Build a [namespace (Scalar), fleetImageId (DynamicEnum)] form.
    fn ns_and_dynamic_form(
        namespace: Option<&str>,
    ) -> crate::frontend::terminal::inline::form::Form {
        use crate::frontend::terminal::inline::form::{
            DynamicEnumState, FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        use ags_protocol::workflow::{OperationReference, OptionParameterBinding, OptionsSource};
        let ns = FormField {
            label: "namespace".into(),
            field_type: FieldType::Scalar,
            required: true,
            value: match namespace {
                Some(v) => FieldValue::Scalar(v.into()),
                None => FieldValue::Empty,
            },
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("namespace".into()),
            schema: serde_json::json!({"type": "string"}),
            read_only: false,
            dynamic: None,
        };
        let img = FormField {
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
                    parameters: std::collections::BTreeMap::from([(
                        "namespace".to_string(),
                        OptionParameterBinding::FromInput("namespace".to_string()),
                    )]),
                    items_path: "$.images".into(),
                    value: "$.id".into(),
                    label: Some("$.name".into()),
                    label_detail: None,
                    fallback_description: None,
                    filter: None,
                },
                deps: vec!["namespace".into()],
                optional_deps: vec![],
                resolved: None,
            }),
        };
        Form::new("gather-inputs", vec![ns, img])
    }

    #[test]
    fn test_resolve_dynamic_enum_missing_dep_sets_note_on_form() {
        let mut form = ns_and_dynamic_form(None);
        super::resolve_dynamic_enum_field(None, &mut form, 1);
        let note = form.validation_note.as_deref().expect("note set on form");
        assert!(
            note.contains("namespace"),
            "note names the dependency: {note}"
        );
        assert!(
            form.fields[1].dynamic.as_ref().unwrap().resolved.is_none(),
            "no fetch attempted"
        );
    }

    #[test]
    fn test_resolve_dynamic_enum_caches_without_selecting_a_value() {
        use crate::frontend::dynamic_options::CannedResolver;
        use crate::frontend::terminal::inline::form::FieldValue;
        use ags_protocol::workflow::OptionChoice;
        let mut form = ns_and_dynamic_form(Some("dev"));
        let resolver = CannedResolver::ok(vec![
            OptionChoice {
                label: "Prod".into(),
                value: "img-1".into(),
            },
            OptionChoice {
                label: "Stg".into(),
                value: "img-2".into(),
            },
        ]);
        super::resolve_dynamic_enum_field(Some(&resolver), &mut form, 1);
        // Resolving caches the choices but must not commit a value, so cancelling
        // the picker leaves the field unset (here: still Enum(None)).
        assert!(matches!(form.fields[1].value, FieldValue::Enum(None)));
        let resolved = form.fields[1]
            .dynamic
            .as_ref()
            .unwrap()
            .resolved
            .as_ref()
            .expect("cached");
        assert_eq!(resolved.choices.len(), 2);
        assert_eq!(
            resolved.dep_key.get("namespace"),
            Some(&serde_json::json!("dev"))
        );
        super::resolve_dynamic_enum_field(Some(&resolver), &mut form, 1);
        assert_eq!(*resolver.calls.borrow(), 1, "cache hit → no second resolve");
    }

    #[test]
    fn test_resolve_dynamic_enum_err_surfaces_message_and_does_not_cache() {
        use crate::frontend::dynamic_options::CannedResolver;
        let mut form = ns_and_dynamic_form(Some("dev"));
        // A resolver whose `result` is an Err → its `resolve` returns the
        // sentinel `CliError::Usage { message: "canned error" }`.
        let resolver = CannedResolver {
            result: std::cell::RefCell::new(Err(crate::errors::CliError::Usage {
                message: "ignored".into(),
                metadata: None,
            })),
            calls: std::cell::RefCell::new(0),
        };
        super::resolve_dynamic_enum_field(Some(&resolver), &mut form, 1);
        let note = form.validation_note.as_deref().expect("note set on Err");
        // The classified message is surfaced in the note (not swallowed).
        assert!(
            note.contains("canned error"),
            "surfaces the classified error: {note}"
        );
        assert!(
            form.fields[1].dynamic.as_ref().unwrap().resolved.is_none(),
            "an Err result must not be cached"
        );
    }

    // ── Task 12 e-g: present_briefing_inner wrapper semantics ───────────────

    /// Build a Running-phase sentinel mirroring the running_phase() helper
    /// pattern already used in nav.rs tests.
    fn running_panel_for_test(
    ) -> crate::frontend::terminal::fullscreen::phases::running::RunningPanel {
        crate::frontend::terminal::fullscreen::phases::running::RunningPanel {
            step_number: 0,
            step_title: String::new(),
            description: String::new(),
            verb: "Starting".into(),
        }
    }

    #[test]
    fn test_present_briefing_inner_resets_briefing_scroll_then_restores_prior_phase_on_ok_true() {
        use ags_protocol::workflow::WorkflowBriefing;

        let mut initial_surface = FullscreenSurface::without_terminal();
        initial_surface.briefing_scroll = 99;
        initial_surface.current_phase = Phase::Running(running_panel_for_test());
        let surface = RefCell::new(initial_surface);

        let mut script = vec![Ok(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))].into_iter();
        let mut next = move || script.next().expect("script exhausted");

        let briefing = WorkflowBriefing {
            overview: "x".into(),
            prerequisites: vec![],
            creates: vec![],
        };
        let result =
            super::present_briefing_inner(&surface, &briefing, "WF", &mut next).expect("ok");
        assert!(result);

        let s = surface.borrow();
        assert!(
            matches!(s.current_phase, Phase::Running(_)),
            "prior phase must be restored after wrapper returns"
        );
        assert_eq!(s.briefing_scroll, 0);
    }

    #[test]
    fn test_present_briefing_inner_restores_prior_phase_on_ok_false() {
        use ags_protocol::workflow::WorkflowBriefing;

        let mut initial_surface = FullscreenSurface::without_terminal();
        initial_surface.current_phase = Phase::Running(running_panel_for_test());
        let surface = RefCell::new(initial_surface);

        let mut script = vec![Ok(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))].into_iter();
        let mut next = move || script.next().expect("script exhausted");

        let briefing = WorkflowBriefing {
            overview: "x".into(),
            prerequisites: vec![],
            creates: vec![],
        };
        let result =
            super::present_briefing_inner(&surface, &briefing, "WF", &mut next).expect("ok");
        assert!(!result);
        assert!(matches!(surface.borrow().current_phase, Phase::Running(_)));
    }

    // ── Task 4: confirm_step_inner skip behaviour ──────────────────────────────

    fn optional_compiled_step(index: usize) -> ags_protocol::workflow::CompiledStep {
        use ags_protocol::catalogue::{OperationId, ServiceId};
        use ags_protocol::workflow::{CompiledStep, OperationReference};
        CompiledStep {
            id: "opt-step".to_string(),
            index,
            description: Some("An optional step".to_string()),
            operation: OperationReference {
                service: ServiceId::new("iam"),
                operation: OperationId::new("testOp"),
            },
            dependencies: vec![],
            confirm: true,
            is_optional: true,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![],
            auto_derived: vec![],
        }
    }

    fn sample_preview(index: usize, id: &str, label: &str) -> ags_protocol::workflow::StepPreview {
        use ags_protocol::catalogue::{HttpMethod, MutationClass, OperationId, ServiceId};
        use ags_protocol::result::CommandPreview;
        ags_protocol::workflow::StepPreview {
            workflow_name: "test-workflow".into(),
            step_id: id.into(),
            step_label: label.into(),
            step_index: index,
            step_total: 2,
            command: CommandPreview {
                service: ServiceId::new("iam"),
                operation_id: OperationId::new("testOp"),
                summary: "Test".into(),
                http_method: HttpMethod::Post,
                url: "http://localhost/test".into(),
                mutation_class: MutationClass::Mutating,
                confirmation_required: true,
                warnings: vec![],
            },
        }
    }

    /// Pressing `s` on an optional step at the confirm gate returns `Skip`.
    #[test]
    fn test_confirm_step_s_on_optional_returns_skip() {
        use ags_protocol::workflow::StepConfirmOutcome;

        let step = optional_compiled_step(0);
        let preview = sample_preview(0, "opt-step", "Optional Step");
        let mut surface = FullscreenSurface::without_terminal_sized(80, 24);
        // Script (pop order): 's' → Skip
        let mut keys = vec![Ok(key(KeyCode::Char('s')))];
        let outcome = confirm_step_inner(&step, &preview, &mut surface, move || {
            keys.pop().expect("script exhausted")
        })
        .unwrap();
        assert_eq!(outcome, StepConfirmOutcome::Skip);
    }

    /// Pressing `s` on a non-optional step is inert; the confirm card does not
    /// offer Skip, so `s` falls through to `PhaseStep::Continue`. The subsequent
    /// Esc key cancels normally.
    #[test]
    fn test_confirm_step_s_on_non_optional_is_inert() {
        use ags_protocol::workflow::StepConfirmOutcome;

        let step = compiled_step(0, None);
        let preview = sample_preview(0, "main", "Main Step");
        let mut surface = FullscreenSurface::without_terminal_sized(80, 24);
        // Script (pop order — last = first consumed): 's' then Esc.
        // 's' on a non-optional step has no binding → PhaseStep::Continue.
        // Esc → PhaseStep::Cancelled → StepConfirmOutcome::Cancel.
        let mut keys = vec![Ok(key(KeyCode::Esc)), Ok(key(KeyCode::Char('s')))];
        let outcome = confirm_step_inner(&step, &preview, &mut surface, move || {
            keys.pop().expect("script exhausted")
        })
        .unwrap();
        assert_eq!(outcome, StepConfirmOutcome::Cancel);
    }

    #[test]
    fn test_resolve_step_failure_enter_retries() {
        use ags_protocol::workflow::StepFailureAction;
        let step = compiled_step(0, None);
        let error = ags_protocol::error::RuntimeError::internal("boom");
        let mut surface = FullscreenSurface::without_terminal_sized(80, 24);
        let mut keys = vec![Ok(key(KeyCode::Enter))];
        let outcome = resolve_step_failure_inner(&step, &error, true, &mut surface, move || {
            keys.pop().expect("script exhausted")
        })
        .unwrap();
        assert_eq!(outcome, StepFailureAction::Retry);
    }

    #[test]
    fn test_resolve_step_failure_s_skips_when_allowed() {
        use ags_protocol::workflow::StepFailureAction;
        let step = compiled_step(0, None);
        let error = ags_protocol::error::RuntimeError::internal("boom");
        let mut surface = FullscreenSurface::without_terminal_sized(80, 24);
        let mut keys = vec![Ok(key(KeyCode::Char('s')))];
        let outcome = resolve_step_failure_inner(&step, &error, true, &mut surface, move || {
            keys.pop().expect("script exhausted")
        })
        .unwrap();
        assert_eq!(outcome, StepFailureAction::Skip);
    }

    #[test]
    fn test_resolve_step_failure_s_inert_when_skip_not_allowed() {
        use ags_protocol::workflow::StepFailureAction;
        let step = compiled_step(0, None);
        let error = ags_protocol::error::RuntimeError::internal("boom");
        let mut surface = FullscreenSurface::without_terminal_sized(80, 24);
        // 's' is inert when Skip is not offered → Continue; Esc → Cancel.
        let mut keys = vec![Ok(key(KeyCode::Esc)), Ok(key(KeyCode::Char('s')))];
        let outcome = resolve_step_failure_inner(&step, &error, false, &mut surface, move || {
            keys.pop().expect("script exhausted")
        })
        .unwrap();
        assert_eq!(outcome, StepFailureAction::Cancel);
    }

    /// Build an optional plan with a single editable scalar field seeded to `x`.
    fn optional_scalar_plan() -> ags_protocol::workflow::StepFieldPlan {
        use ags_protocol::workflow::{
            StepField, StepFieldId, StepFieldLocation, StepFieldPlan, StepFieldSource,
        };
        StepFieldPlan {
            step_index: 0,
            step_label: "optional-step".into(),
            step_description: None,
            optional: true,
            fields: vec![StepField {
                id: StepFieldId(0),
                field: "note".into(),
                label: "note".into(),
                description: None,
                location: StepFieldLocation::Body,
                schema: serde_json::json!({"type": "string"}),
                value: serde_json::json!("x"),
                // Unset → editable (FieldSource::UserInput, not read-only).
                source: StepFieldSource::Unset,
                required: false,
                workflow_input: None,
                body_overflow: false,
                show_in_review: true,
            }],
        }
    }

    /// Regression guard: pressing `s` while a scalar field IS in edit mode must
    /// NOT skip the step — the char must reach the edit buffer. Here the run
    /// begins editing the field, types `s` (appending to the seeded `x`), then
    /// commits and submits. The outcome is `Proceed` (never `Skip`) and the
    /// projected edit reflects the typed `s`.
    #[test]
    fn test_review_step_s_while_editing_does_not_skip() {
        use ags_protocol::workflow::{StepFieldId, StepReviewOutcome};

        let plan = optional_scalar_plan();
        let mut surface = FullscreenSurface::without_terminal_sized(80, 24);
        // Consumed in order via `.next()`:
        //  Up    → focus the scalar field (off the Submit slot)
        //  Enter → begin edit (scalar + submit_focusable)
        //  's'   → in edit mode → appended to buffer (NOT a skip)
        //  Enter → commit the edit ("x" → "xs")
        //  Tab   → focus the Submit slot
        //  Enter → submit → Proceed
        let mut keys = vec![
            Ok(key(KeyCode::Up)),
            Ok(key(KeyCode::Enter)),
            Ok(key(KeyCode::Char('s'))),
            Ok(key(KeyCode::Enter)),
            Ok(key(KeyCode::Tab)),
            Ok(key(KeyCode::Enter)),
        ]
        .into_iter();
        let outcome =
            review_step_optional_inner(&plan, &mut surface, move || keys.next().unwrap()).unwrap();
        match outcome {
            StepReviewOutcome::Proceed(edits) => {
                assert_eq!(
                    edits.values.get(&StepFieldId(0)),
                    Some(&serde_json::json!("xs")),
                    "the typed 's' landed in the field, not a skip"
                );
            }
            other => panic!("expected Proceed, got {other:?}"),
        }
    }

    #[test]
    fn test_present_briefing_inner_restores_prior_phase_on_err() {
        use crate::errors::CliError;
        use ags_protocol::workflow::WorkflowBriefing;

        let mut initial_surface = FullscreenSurface::without_terminal();
        initial_surface.current_phase = Phase::Running(running_panel_for_test());
        let surface = RefCell::new(initial_surface);

        // Key source returns Err on first call — wrapper must still restore the
        // prior phase before propagating the Err.
        let mut next = || {
            Err(CliError::Usage {
                message: "key-read failed".into(),
                metadata: None,
            })
        };

        let briefing = WorkflowBriefing {
            overview: "x".into(),
            prerequisites: vec![],
            creates: vec![],
        };
        let err = super::present_briefing_inner(&surface, &briefing, "WF", &mut next)
            .expect_err("propagated err");
        // The exact Display format is incidental; assert the message text round-trips.
        let displayed = err.to_string();
        assert!(
            displayed.contains("key-read failed"),
            "expected message round-trip, got: {displayed}"
        );
        assert!(
            matches!(surface.borrow().current_phase, Phase::Running(_)),
            "prior phase must be restored even on Err return path"
        );
    }
}
