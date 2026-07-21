//! Surface-agnostic form ↔ JSON-editor ↔ confirm-card driver. Generic
//! over the ratatui backend so inline (stderr viewport) and fullscreen
//! (alt-screen) interactions share one implementation.
//!
//! The public entry points are:
//! - [`run_gather`] — needs-only gather loop (form → confirm-card → back-to-edit)
//!   used by `InlineInteraction` for workflows. (The fullscreen surface drives
//!   gather/confirm in-layout via `FullscreenSurface`, reusing this module's
//!   `pub(crate)` `drive_json_editor` / `crossterm_next_key` /
//!   `build_gather_confirm_card`.)
//! - [`run_full_surface_gather`] — inline single-command gather over the FULL
//!   request surface (all params + body, with the optional-row `a` toggle).
//! - [`run_form_with_editor`] — drives a [`FormPhase`], opening the JSON
//!   tree editor when the user activates a `JsonBody` field.
//! - [`run_confirm_card`] — drives a [`ConfirmCardPhase`].
//!
//! Both functions wrap an injectable-event inner driver (`drive_form` /
//! `drive_confirm`) that accept a `FnMut() -> Result<KeyEvent, CliError>`
//! closure instead of polling crossterm directly. This seam makes the
//! whole unit testable with a `TestBackend` + scripted key stream without
//! a real terminal.
use crossterm::event::KeyEvent;
use ratatui::{backend::Backend, Terminal};

use ags_protocol::workflow::{CompiledStep, GatherResult, SuppliedInputView, WorkflowInputNeeded};

use crate::errors::CliError;
use crate::frontend::terminal::inline::form::{FieldKey, FieldValue, FormField};
use crate::frontend::terminal::inline::json_editor::navigation::{get_mut, NodePath};
use crate::frontend::terminal::inline::json_editor::node::{
    from_schema, Node, NodeKind, ScalarValue,
};
use crate::frontend::terminal::inline::json_editor::render::{
    focused_description, render_tree_view,
};
use crate::frontend::terminal::inline::json_editor::{
    self, commit_raw_to_tree, EditorMode, EditorStep,
};
use crate::frontend::terminal::inline::phases::confirm_card::{
    ConfirmAction, ConfirmCard, ConfirmCardPhase,
};
use crate::frontend::terminal::inline::phases::form::{FormPhase, PhaseResult};
use crate::frontend::terminal::inline::phases::{Phase, PhaseStep};
use crate::frontend::terminal::views::fields::render_hint_box;

// ─────────────────────────────────────────────────────────────────────────────
// Public surface
// ─────────────────────────────────────────────────────────────────────────────

/// Hint to the driver about which surface is hosting the form.
///
/// Only the inline surface uses this driver now — the fullscreen surface
/// renders gather/confirm in-layout via `FullscreenSurface`, not
/// through a full-screen takeover. The enum is retained so the inline call
/// site can declare intent.
#[derive(Debug, Clone, Copy)]
pub enum FormLayout {
    Inline,
}

/// Outcome of [`run_form_with_editor`].
pub enum FormRunResult {
    /// User submitted the form; contains the projected field values.
    Submitted(GatherResult),
    /// User pressed Esc / Ctrl-C at the form level without submitting.
    Cancelled,
}

/// Outcome of [`run_confirm_card`].
pub enum ConfirmOutcome {
    /// User confirmed the operation.
    Confirmed,
    /// User chose "Back to edit" — the caller should re-open the form.
    BackToEdit,
    /// User cancelled.
    Cancelled,
    /// User chose to skip this optional step.
    Skipped,
}

/// Sort gather fields into CLI request order: path, then query, then header,
/// then body — each group alphabetical by label. Shared by the inline gather
/// path ([`run_gather`]) and the fullscreen gather form so both surfaces order
/// fields identically (scalars first, JSON body last).
pub(crate) fn sort_gather_fields_by_location(
    fields: &mut [crate::frontend::terminal::inline::form::FormField],
    needed: &[WorkflowInputNeeded],
    supplied: &[SuppliedInputView],
) {
    use ags_protocol::workflow::StepFieldLocation;
    use std::collections::HashMap;

    // Build a label→location map from both needed and supplied inputs so the
    // combined FormField list can be sorted by request location.
    let loc_by_label: HashMap<String, StepFieldLocation> = needed
        .iter()
        .map(|n| (n.label.clone(), n.location))
        .chain(supplied.iter().map(|s| (s.label.clone(), s.location)))
        .collect();

    sort_form_fields_by_location(fields, &loc_by_label);
}

/// Sort form fields by request location (Path → Query → Header → Body), then
/// case-insensitive label, given a prebuilt `label → location` map. Labels
/// absent from the map sort as the default location. Callers build the map
/// from whichever input shape they hold (workflow needs/supplied, or the full
/// request surface); the ordering itself lives here so both stay in step.
pub(crate) fn sort_form_fields_by_location(
    fields: &mut [crate::frontend::terminal::inline::form::FormField],
    loc_by_label: &std::collections::HashMap<String, ags_protocol::workflow::StepFieldLocation>,
) {
    use ags_protocol::workflow::StepFieldLocation;

    /// Stable ordering rank for a field by location: path < query < header < body.
    fn location_rank(loc: StepFieldLocation) -> u8 {
        match loc {
            StepFieldLocation::Path => 0,
            StepFieldLocation::Query => 1,
            StepFieldLocation::Header => 2,
            StepFieldLocation::Body => 3,
        }
    }

    fields.sort_by(|a, b| {
        let la = loc_by_label.get(&a.label).copied().unwrap_or_default();
        let lb = loc_by_label.get(&b.label).copied().unwrap_or_default();
        location_rank(la)
            .cmp(&location_rank(lb))
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });
}

/// Drive the full gather loop for one workflow step: form → confirm-card →
/// optionally back-to-edit, repeating until the user submits or cancels.
///
/// The caller is responsible for the `needed.is_empty()` early-return: callers
/// acquire the terminal via `session.terminal_mut()`, which errors when there is
/// no live terminal (the case in the no-terminal unit tests), so the guard must
/// run *before* that borrow — outside this function, which already holds `tty`.
pub(crate) fn run_gather<B: Backend>(
    tty: &mut Terminal<B>,
    needed: &[WorkflowInputNeeded],
    supplied: &[SuppliedInputView],
    step_context: &CompiledStep,
    layout: FormLayout,
) -> Result<GatherResult, CliError> {
    use crate::frontend::terminal::inline::form_builder::{build_form_fields, reseed_fields};

    let title = step_context
        .description
        .as_deref()
        .unwrap_or("Provide inputs")
        .to_owned();

    let mut fields = build_form_fields(needed, supplied);
    sort_gather_fields_by_location(&mut fields, needed, supplied);

    loop {
        let result = match run_form_with_editor(tty, &title, fields, layout)? {
            FormRunResult::Submitted(r) => r,
            FormRunResult::Cancelled => return Err(gather_cancelled_error()),
        };

        let card = build_gather_confirm_card(needed, supplied, &result);

        match run_confirm_card(tty, card)? {
            ConfirmOutcome::Confirmed => return Ok(result),
            ConfirmOutcome::Cancelled => return Err(gather_cancelled_error()),
            // The gather confirm card never offers Skip; map defensively to cancel.
            ConfirmOutcome::Skipped => return Err(gather_cancelled_error()),
            ConfirmOutcome::BackToEdit => {
                let mut reseeded = reseed_fields(needed, supplied, &result);
                sort_gather_fields_by_location(&mut reseeded, needed, supplied);
                fields = reseeded;
            }
        }
    }
}

/// Drive the inline single-command FULL-surface gather: a form built from the
/// operation's entire input set (required + optional), with the advanced toggle.
/// Optional values project as `input_overrides`; the executor folds those into
/// the request. Loops form → confirm card → back-to-edit until submit or cancel.
pub(crate) fn run_full_surface_gather<B, F>(
    tty: &mut Terminal<B>,
    title: &str,
    mut fields: Vec<FormField>,
    next_event: &mut F,
) -> Result<GatherResult, CliError>
where
    B: Backend,
    F: FnMut() -> Result<KeyEvent, CliError>,
{
    // The pristine field set (original seeds). On "Back to edit" we rebuild from
    // this and overlay the user's edits, so a field that was filled then cleared
    // reverts to its seed rather than retaining a stale value (mirrors
    // `reseed_fields` semantics on the needs-only `run_gather` path).
    let original = fields.clone();
    loop {
        let mut form = crate::frontend::terminal::inline::form::Form::new(title, fields.clone())
            .with_submit_focusable(true)
            .with_optional_filter(true)
            .with_mark_required(true)
            .with_submit_description(GATHER_REVIEW_SUBMIT_DESCRIPTION);
        // Default focus is row 0, which may be a hidden optional row when an
        // optional path/query field sorts ahead of a required body field. Snap
        // focus onto a visible row before the first draw.
        form.normalize_focus_to_visible();
        let phase = FormPhase::new(form);
        let result = match drive_form(tty, phase, &FormLayout::Inline, &mut *next_event)? {
            FormRunResult::Submitted(r) => r,
            FormRunResult::Cancelled => return Err(gather_cancelled_error()),
        };

        let card = build_full_surface_confirm_card(&fields, &result);
        match drive_confirm(
            tty,
            ConfirmCardPhase::new(card),
            crate::frontend::terminal::views::nav::NavContext::ConfirmCard,
            &mut *next_event,
        )? {
            ConfirmOutcome::Confirmed => return Ok(result),
            ConfirmOutcome::Cancelled => return Err(gather_cancelled_error()),
            // The full-surface gather confirm card never offers Skip; map defensively.
            ConfirmOutcome::Skipped => return Err(gather_cancelled_error()),
            ConfirmOutcome::BackToEdit => {
                // Rebuild from the pristine seeds, then overlay the projected
                // overrides so edits survive but cleared fields don't go stale.
                fields = original.clone();
                for f in &mut fields {
                    if let FieldKey::Input(name) = &f.key {
                        if let Some(v) = result.input_overrides.get(name) {
                            f.value =
                                crate::frontend::terminal::inline::form_builder::value_to_field_value(v);
                        }
                    }
                }
            }
        }
    }
}

/// Confirm card summarising the full-surface request: every projected override
/// (name → value), kebab-labelled, in the form's field order.
///
/// Rows follow `fields` — already sorted by request location (path → query →
/// header → body) — so the review matches the form and the underlying step. A
/// plain alphabetical sort would, e.g., put a `permissions` body field ahead of a
/// `role-id` path param. Any override without a matching field (defensive; should
/// not happen for a full-surface gather) sorts last, by label.
pub(crate) fn build_full_surface_confirm_card(
    fields: &[FormField],
    result: &GatherResult,
) -> ConfirmCard {
    use ags_runtime::support::strings::to_kebab_case;
    // The full-surface form is built entirely from `FieldKey::Input` fields, so
    // every value lands in `input_overrides`; `slot_values` must stay empty.
    debug_assert!(
        result.slot_values.is_empty(),
        "full-surface gather projects only input_overrides, never slot_values"
    );
    let order: std::collections::HashMap<&str, usize> = fields
        .iter()
        .enumerate()
        .filter_map(|(i, f)| match &f.key {
            FieldKey::Input(name) => Some((name.as_str(), i)),
            _ => None,
        })
        .collect();
    let mut summary: Vec<(usize, String, String)> = result
        .input_overrides
        .iter()
        .map(|(k, v)| {
            let rank = order.get(k.as_str()).copied().unwrap_or(usize::MAX);
            (rank, to_kebab_case(k), gather_json_value_to_display(v))
        })
        .collect();
    summary.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    ConfirmCard::new(
        "Review request",
        summary
            .into_iter()
            .map(|(_, label, value)| (label, value))
            .collect(),
    )
}

/// Build the error returned when the user cancels interactive input gathering.
pub(crate) fn gather_cancelled_error() -> CliError {
    CliError::Usage {
        message: "Input gathering cancelled".into(),
        metadata: None,
    }
}

/// Build a [`ConfirmCard`] summarising the request the user is about to
/// confirm. Supplied inputs come first (with any user-edited override from
/// `result`), followed by the gathered slot values.
pub(crate) fn build_gather_confirm_card(
    needed: &[WorkflowInputNeeded],
    supplied: &[SuppliedInputView],
    result: &GatherResult,
) -> ConfirmCard {
    let mut summary: Vec<(String, String)> = Vec::new();

    // Supplied inputs — show the user-edited override if present, else the
    // original supplied value.
    for sup in supplied {
        let value = if let Some(v) = result.input_overrides.get(&sup.label) {
            gather_json_value_to_display(v)
        } else {
            gather_json_value_to_display(&sup.value)
        };
        summary.push((sup.label.clone(), value));
    }

    // Gathered slot values. Required slots are always present after the form
    // validates required fields; an absent slot is skipped defensively.
    for entry in needed {
        if let Some(v) = result.slot_values.get(&entry.id) {
            summary.push((entry.label.clone(), gather_json_value_to_display(v)));
        }
    }

    ConfirmCard::new("Review request", summary)
}

/// Render a JSON value for display in the gather confirm-card summary.
/// Strings are shown as raw strings; everything else is JSON-serialised.
fn gather_json_value_to_display(v: &serde_json::Value) -> String {
    match v {
        // Supplied values can carry prior-step API captures; strip terminal
        // control sequences before they reach the confirm card (CONTRIBUTING
        // § Security). The serialised arm is already safe — serde_json escapes
        // control chars.
        serde_json::Value::String(s) => {
            ags_runtime::support::strings::strip_terminal_control_sequences(s)
        }
        other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
    }
}

/// Drive a form (with inline JSON-editor support) to completion.
///
/// `layout` controls future surface-specific drawing adjustments; both
/// variants currently render into the full frame area.
pub(crate) fn run_form_with_editor<B: Backend>(
    tty: &mut Terminal<B>,
    title: &str,
    fields: Vec<FormField>,
    layout: FormLayout,
) -> Result<FormRunResult, CliError> {
    // The inline form shows a focusable `[ Confirm ]` button at the
    // bottom (pinned by `render_inline`); without this the Submit row is absent.
    let phase = FormPhase::new(
        crate::frontend::terminal::inline::form::Form::new(title, fields)
            .with_submit_focusable(true),
    );
    drive_form(tty, phase, &layout, crossterm_next_key)
}

/// Drive a confirm card to completion.
pub(crate) fn run_confirm_card<B: Backend>(
    tty: &mut Terminal<B>,
    card: ConfirmCard,
) -> Result<ConfirmOutcome, CliError> {
    drive_confirm(
        tty,
        ConfirmCardPhase::new(card),
        crate::frontend::terminal::views::nav::NavContext::ConfirmCard,
        crossterm_next_key,
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Private drivers (event-source injectable for tests)
// ─────────────────────────────────────────────────────────────────────────────

/// Production event source: block on the next crossterm key event, ignoring
/// non-key events (resize/mouse/focus). Shared by every `run_*` entry point;
/// the `drive_*` cores take it as an injectable `FnMut` so tests can script keys.
pub(crate) fn crossterm_next_key() -> Result<KeyEvent, CliError> {
    use crossterm::event::{self, Event};
    loop {
        match event::read() {
            Ok(Event::Key(k)) => {
                if k.kind == crossterm::event::KeyEventKind::Press {
                    return Ok(k);
                }
            }
            Ok(_) => continue,
            Err(e) => {
                return Err(CliError::Usage {
                    message: format!("terminal read error: {e}"),
                    metadata: None,
                })
            }
        }
    }
}

/// Inner form driver. Accepts any `FnMut() -> Result<KeyEvent, CliError>` as
/// the event source — production callers pass a crossterm reader, tests pass
/// a closure that drains a `Vec<KeyEvent>`.
fn drive_form<B, F>(
    tty: &mut Terminal<B>,
    phase: FormPhase,
    _layout: &FormLayout,
    mut next_event: F,
) -> Result<FormRunResult, CliError>
where
    B: Backend,
    F: FnMut() -> Result<KeyEvent, CliError>,
{
    // `drive_form` projects a `GatherResult` (slot + input overrides); other
    // callers (Phase-1 inputs, per-step review) need the `Form` back to project
    // declared inputs / step edits. The interactive loop is identical, so it
    // lives once in `drive_inline_form` and each caller projects what it needs.
    // The review / step-edit gather path has no dynamic-enum picker fetch.
    match drive_inline_form(tty, phase, None, &mut next_event)? {
        Some(form) => Ok(FormRunResult::Submitted(form.project_gathered())),
        None => Ok(FormRunResult::Cancelled),
    }
}

/// Drive an inline form to completion and return the final [`Form`] on submit
/// (`Some`) or `None` on cancel (Esc / Ctrl-C). Handles the in-place JSON editor
/// sub-loop and — when `options_fetch` is supplied — the dynamic-enum picker
/// sub-loop. `options_fetch` is `None` on paths that never carry `DynamicEnum`
/// fields (review / step edits); the inline Phase-1 path (Task 5) passes the
/// real fetch.
///
/// Returning the `Form` (rather than a projected `GatherResult`) lets the
/// caller project whatever it needs: `project_gathered`, `project_inputs`
/// (Phase-1), or `project_step_edits` (per-step review).
pub(crate) fn drive_inline_form<B, F>(
    tty: &mut Terminal<B>,
    mut phase: FormPhase,
    options_fetch: Option<&dyn crate::frontend::dynamic_options::OptionsFetch>,
    next_event: &mut F,
) -> Result<Option<crate::frontend::terminal::inline::form::Form>, CliError>
where
    B: Backend,
    F: FnMut() -> Result<KeyEvent, CliError>,
{
    loop {
        // Inline single-command form: show the `[o] show optional (+N)` / `[o]
        // hide optional` toggle in the nav bar. Empty for any other form.
        let nav_suffix = if phase.form().optional_filter {
            crate::frontend::terminal::views::nav::optional_toggle_suffix(
                phase.form().optional_empty_count(),
                phase.form().show_optional,
            )
        } else {
            Vec::new()
        };
        tty.draw(|f| {
            crate::frontend::terminal::inline::chrome::render_with_nav_suffix(
                f,
                crate::frontend::terminal::views::nav::NavContext::Fields,
                nav_suffix.clone(),
                |frame, main| {
                    crate::frontend::terminal::views::fields::render_inline(
                        frame,
                        main,
                        phase.form(),
                        phase.form().submit_focusable,
                    );
                },
            );
        })
        .map_err(|e| CliError::Usage {
            message: format!("TUI draw failed: {e}"),
            metadata: None,
        })?;

        let key = next_event()?;

        if key.kind != crossterm::event::KeyEventKind::Press {
            continue;
        }

        // Intercept Ctrl-C globally (raw mode swallows SIGINT).
        if is_ctrl_c(key) {
            return Ok(None);
        }

        match phase.on_key(key) {
            PhaseStep::Continue => continue,
            PhaseStep::Cancelled => return Ok(None),
            PhaseStep::Done(PhaseResult::Submitted(_)) => return Ok(Some(phase.into_form())),
            PhaseStep::Done(PhaseResult::OpenJsonEditor(idx)) => {
                // Run the JSON-editor sub-loop against the field's schema
                // and current value, then write the result back.
                let result = drive_json_editor(tty, phase.form_mut(), idx, next_event, true)?;
                if let Some(value) = result {
                    let pretty =
                        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                    phase.form_mut().fields[idx].value = FieldValue::JsonBody(pretty);
                }
                // Continue the form loop regardless of editor save/cancel.
                continue;
            }
            // Take over the viewport to resolve + pick a dynamic-enum field,
            // writing the chosen value back into the field, then resume the form.
            PhaseStep::Done(PhaseResult::OpenEnumPicker(idx)) => {
                drive_enum_picker(tty, phase.form_mut(), idx, options_fetch, next_event)?;
                continue;
            }
        }
    }
}

/// Take over the viewport to resolve and pick a dynamic-enum field's value.
///
/// Draws only through `tty` (the `&mut Terminal` the caller already holds) —
/// never the session — mirroring [`drive_json_editor`]. The flow is:
/// dependency gate → cache check → fetch-with-spinner (only on a cache miss) →
/// filterable list → write the chosen value back into `form.fields[idx]`.
///
/// Returning `Ok(())` always resumes the form; a blocked dependency or a failed
/// fetch leaves a `validation_note` and the field on its free-text fallback,
/// never aborting the run.
///
/// Blocking-vs-poll: the fetch spinner phase polls crossterm directly (80 ms
/// budget) so the spinner animates and the fetch result is seen — it does NOT
/// use the injected `next_event`, which blocks. The list phase reads through the
/// injected `next_event` (blocking is correct there; scripted keys drive it in
/// tests). With a ready `CannedFetch`, the first `try_recv` resolves before the
/// spinner loop body runs, so tests never touch the crossterm poll.
fn drive_enum_picker<B, F>(
    tty: &mut Terminal<B>,
    form: &mut crate::frontend::terminal::inline::form::Form,
    idx: usize,
    options_fetch: Option<&dyn crate::frontend::dynamic_options::OptionsFetch>,
    next_event: &mut F,
) -> Result<(), CliError>
where
    B: Backend,
    F: FnMut() -> Result<KeyEvent, CliError>,
{
    use crate::frontend::terminal::dynamic_enums::{
        apply_picker_result, compute_dep_key, deps_satisfied, picker_action, store_resolved,
        PickerAction, SPINNER_FRAMES,
    };
    use crate::frontend::terminal::picker_list::PickerList;
    use crossterm::event::KeyCode;

    let Some(state) = form.fields[idx].dynamic.as_ref() else {
        return Ok(());
    };
    let deps = state.deps.clone();
    let optional_deps = state.optional_deps.clone();
    let source = state.source.clone();

    // 1. Dependency gate: only required deps block opening.
    if !deps_satisfied(form, &deps) {
        // Name the first *unsatisfied* dependency (not just the first), matching
        // fullscreen's `resolve_dynamic_enum_field`.
        let missing = deps
            .iter()
            .find(|d| !deps_satisfied(form, std::slice::from_ref(*d)))
            .or_else(|| deps.first())
            .map(String::as_str)
            .unwrap_or("");
        form.validation_note = Some(format!(
            "Fill `{}` first to load choices",
            ags_runtime::support::strings::to_kebab_case(missing)
        ));
        return Ok(());
    }

    // 2. Cache: reuse choices while the dependency values are unchanged. The key
    // spans required + optional deps.
    let all_deps: Vec<String> = deps.iter().chain(optional_deps.iter()).cloned().collect();
    let dep_key = compute_dep_key(form, &all_deps);
    let cached = form.fields[idx]
        .dynamic
        .as_ref()
        .and_then(|d| d.resolved.as_ref())
        .filter(|r| r.dep_key == dep_key)
        .is_some();

    // 3. Resolve (cache miss only). An empty optional dep opens direct-entry
    // (empty choices) with no fetch; otherwise fetch with the spinner sub-loop.
    if !cached && !deps_satisfied(form, &optional_deps) {
        store_resolved(
            &mut form.fields[idx],
            dep_key.clone(),
            ags_protocol::workflow::ResolvedOptions {
                choices: vec![],
                truncated: false,
            },
        );
    } else if !cached {
        let Some(fetch) = options_fetch else {
            return Ok(());
        };
        let inputs = form.project_inputs();
        let mut task = fetch.start(&source, &inputs);
        let mut frame = 0usize;
        loop {
            match task.rx.try_recv() {
                Ok(Ok(resolved)) => {
                    store_resolved(&mut form.fields[idx], dep_key.clone(), resolved);
                    break;
                }
                Ok(Err(err)) => {
                    form.validation_note = Some(format!(
                        "Couldn't load choices: {}; type a value manually",
                        err.message
                    ));
                    return Ok(());
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    form.validation_note = Some("Options fetch ended unexpectedly".into());
                    return Ok(());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }

            // Draw the spinner line through the held terminal.
            let spinner = SPINNER_FRAMES[frame % SPINNER_FRAMES.len()];
            tty.draw(|f| {
                crate::frontend::terminal::inline::chrome::render(
                    f,
                    crate::frontend::terminal::views::nav::NavContext::Loading,
                    |frame_, area| {
                        use ratatui::widgets::{Block, Borders, Paragraph};
                        frame_.render_widget(
                            Paragraph::new(format!("Loading choices\u{2026} {spinner}"))
                                .block(Block::default().borders(Borders::ALL).title("Parameters")),
                            area,
                        );
                    },
                );
            })
            .map_err(|e| CliError::Usage {
                message: format!("TUI draw failed: {e}"),
                metadata: None,
            })?;
            frame += 1;

            // Poll crossterm directly (the injected reader blocks): animate the
            // spinner and honour Esc / Ctrl-C to abort the in-flight fetch.
            if crossterm::event::poll(std::time::Duration::from_millis(80)).unwrap_or(false) {
                if let Ok(crossterm::event::Event::Key(k)) = crossterm::event::read() {
                    if k.kind == crossterm::event::KeyEventKind::Press
                        && (k.code == KeyCode::Esc || is_ctrl_c(k))
                    {
                        task.abort();
                        return Ok(());
                    }
                }
            }
        }
    }

    // 4. Open the list. A `Blocked` action means the field is unresolved (e.g. a
    // fetch error already noted); leave the note and resume the form.
    let PickerAction::Open { choices, truncated } = picker_action(form, idx) else {
        return Ok(());
    };
    let current = match &form.fields[idx].value {
        FieldValue::Enum(Some(s)) => Some(s.clone()),
        _ => None,
    };
    let mut list = PickerList::new(choices, current.as_deref());
    loop {
        tty.draw(|f| {
            crate::frontend::terminal::inline::chrome::render(
                f,
                crate::frontend::terminal::views::nav::NavContext::Picker,
                |frame_, area| list.render_in(frame_, area, truncated),
            );
        })
        .map_err(|e| CliError::Usage {
            message: format!("TUI draw failed: {e}"),
            metadata: None,
        })?;

        let key = next_event()?;
        if key.kind != crossterm::event::KeyEventKind::Press {
            continue;
        }
        if is_ctrl_c(key) || key.code == KeyCode::Esc {
            // Cancel: field unchanged.
            return Ok(());
        }
        match key.code {
            KeyCode::Up => list.move_up(),
            KeyCode::Down => list.move_down(),
            KeyCode::PageUp => list.page_up(5),
            KeyCode::PageDown => list.page_down(5),
            KeyCode::Backspace => list.pop_char(),
            KeyCode::Char(c) => list.push_char(c),
            KeyCode::Enter => {
                let chosen = list.selected_value().or_else(|| list.custom_value());
                apply_picker_result(form, idx, chosen);
                return Ok(());
            }
            _ => {}
        }
    }
}

/// Submit-button hint shown on the Phase-1 declared-inputs form (both surfaces).
pub(crate) const GATHER_INPUTS_SUBMIT_DESCRIPTION: &str =
    "Submit these inputs and continue to the workflow steps.";

/// Submit-button hint shown on the per-step review form (both surfaces).
pub(crate) const STEP_REVIEW_SUBMIT_DESCRIPTION: &str = "Submit any changes and run this step.";

/// Submit-button hint shown on the full-request gather form — the inline
/// single-command surface and the fullscreen per-step/command gather. Submitting
/// opens the confirm card to review the request before it is sent.
pub(crate) const GATHER_REVIEW_SUBMIT_DESCRIPTION: &str = "Review the request before it is sent";

/// Shared Phase-1 orchestration: build the declared-inputs form, drive it via
/// the surface-specific `drive`, then project the submitted form to the
/// declared-input map (`Ok(None)` on cancel). Both rich surfaces (fullscreen,
/// inline) call this, so the form construction — field builder, submit hint,
/// focus, box title — lives in one place and cannot drift; only `drive` (the
/// render loop) is surface-specific. `dynamic_enums` is the surface's picker
/// capability (fullscreen `true`; inline `false` → plain-text inputs).
///
/// FIELD ORDER: this renders `specs` in the order given — it deliberately does
/// NOT sort. Ordering is the caller's job: the workflow executor hands inputs
/// in step-of-first-use order (`order_inputs_by_first_use`) so the form follows
/// execution flow. (Single commands take a different path —
/// `sort_full_surface_fields` — that sorts by request location then label.)
/// Do not add a sort here, or it silently overrides the executor's intent;
/// `test_build_inputs_form_preserves_given_spec_order` guards this.
#[allow(clippy::type_complexity)]
pub(crate) fn collect_inputs_form<D>(
    specs: &[ags_protocol::workflow::WorkflowInputSpec],
    current: &std::collections::BTreeMap<String, serde_json::Value>,
    dynamic_enums: bool,
    drive: D,
) -> Result<
    Option<(
        std::collections::BTreeMap<String, serde_json::Value>,
        ags_protocol::workflow::RunMode,
    )>,
    CliError,
>
where
    D: FnOnce(
        crate::frontend::terminal::inline::form::Form,
    ) -> Result<Option<crate::frontend::terminal::inline::form::Form>, CliError>,
{
    use crate::frontend::terminal::inline::form::Form;
    use crate::frontend::terminal::inline::form_builder::build_inputs_form;
    use ags_protocol::workflow::RunMode;

    // Nothing declared → no screen; caller uses today's default mode.
    if specs.is_empty() {
        return Ok(Some((current.clone(), RunMode::ReviewInputSteps)));
    }
    let mut form = Form::new(
        "gather-inputs",
        build_inputs_form(specs, current, dynamic_enums),
    )
    .with_submit_focusable(true)
    .with_run_mode_buttons(true) // run-start gather shows the three buttons
    .with_box_title("Step 0: gather-inputs")
    .with_submit_description(GATHER_INPUTS_SUBMIT_DESCRIPTION);
    form.focus_first_editable();
    Ok(drive(form)?.map(|f| (f.project_inputs(), f.selected_run_mode())))
}

/// Shared per-step review orchestration: build the review form from the plan,
/// drive it via the surface-specific `drive`, then project step edits (`Cancel`
/// on cancel). As with [`collect_inputs_form`], form construction is
/// centralised; only `drive` differs per surface.
pub(crate) fn review_step_form<D>(
    plan: &ags_protocol::workflow::StepFieldPlan,
    drive: D,
) -> Result<ags_protocol::workflow::StepReviewOutcome, CliError>
where
    D: FnOnce(
        crate::frontend::terminal::inline::form::Form,
    ) -> Result<Option<crate::frontend::terminal::inline::form::Form>, CliError>,
{
    use crate::frontend::terminal::inline::form::Form;

    let mut form =
        Form::from_step_plan(plan).with_submit_description(STEP_REVIEW_SUBMIT_DESCRIPTION);
    form.focus_submit_if_available();
    Ok(match drive(form)? {
        Some(f) => ags_protocol::workflow::StepReviewOutcome::Proceed(f.project_step_edits(plan)),
        None => ags_protocol::workflow::StepReviewOutcome::Cancel,
    })
}

/// Drive a caller-built confirm phase under a caller-chosen nav context, using
/// the production crossterm key source. Lets a surface offer a custom action set
/// (e.g. the inline per-step confirm's Confirm/Cancel under the terse `Confirm`
/// nav bar) while reusing the one tested confirm loop below. The caller maps the
/// returned [`ConfirmOutcome`] to its own result type.
pub(crate) fn run_confirm_phase<B: Backend>(
    tty: &mut Terminal<B>,
    phase: ConfirmCardPhase,
    nav: crate::frontend::terminal::views::nav::NavContext,
) -> Result<ConfirmOutcome, CliError> {
    drive_confirm(tty, phase, nav, crossterm_next_key)
}

/// Inner confirm driver — same injectable-event pattern as `drive_form`. `nav`
/// selects the nav-bar hints so callers can keep their own bar (the per-step
/// confirm stays on `Confirm`, the request-review card on `ConfirmCard`).
fn drive_confirm<B, F>(
    tty: &mut Terminal<B>,
    mut phase: ConfirmCardPhase,
    nav: crate::frontend::terminal::views::nav::NavContext,
    mut next_event: F,
) -> Result<ConfirmOutcome, CliError>
where
    B: Backend,
    F: FnMut() -> Result<KeyEvent, CliError>,
{
    loop {
        tty.draw(|f| {
            crate::frontend::terminal::inline::chrome::render(f, nav, |frame, main| {
                phase.render_in(frame, main)
            });
        })
        .map_err(|e| CliError::Usage {
            message: format!("TUI draw failed: {e}"),
            metadata: None,
        })?;

        let key = next_event()?;

        if key.kind != crossterm::event::KeyEventKind::Press {
            continue;
        }

        if is_ctrl_c(key) {
            return Ok(ConfirmOutcome::Cancelled);
        }

        match phase.on_key(key) {
            PhaseStep::Continue => continue,
            PhaseStep::Cancelled => return Ok(ConfirmOutcome::Cancelled),
            // Both the affirmative confirm and the failure-gate retry are the
            // card's primary action; callers interpret `Confirmed` per context
            // (Proceed for a confirm, Retry for the failure gate).
            PhaseStep::Done(ConfirmAction::Confirm | ConfirmAction::Retry) => {
                return Ok(ConfirmOutcome::Confirmed)
            }
            PhaseStep::Done(ConfirmAction::Back) => return Ok(ConfirmOutcome::BackToEdit),
            PhaseStep::Done(ConfirmAction::Cancel) => return Ok(ConfirmOutcome::Cancelled),
            PhaseStep::Done(ConfirmAction::Skip) => return Ok(ConfirmOutcome::Skipped),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// JSON-editor sub-loop
// ─────────────────────────────────────────────────────────────────────────────

/// What the JSON-editor driver should do after applying one key.
pub(crate) enum JsonEditorOutcome {
    /// Re-render and keep editing.
    Continue,
    /// Commit this value back to the field.
    Save(serde_json::Value),
    /// Discard the edit.
    Cancel,
}

/// Apply one key to the JSON editor's mutable state (`root`/`focus`/`mode`/
/// `scalar`), returning what the surrounding loop should do. Shared by the
/// inline (`drive_json_editor`) and fullscreen (`drive_json_edit_phase`)
/// drivers, which differ only in where this state lives and how it renders —
/// the key→action mapping (scalar in-place edit, raw-vs-tree dispatch, save/
/// cancel) is identical and lives here so the two surfaces cannot drift. The
/// caller handles Ctrl-C before calling this.
// The four mutable state pieces plus the field's schema/name/required and the
// key are all genuinely needed; grouping them would only add indirection.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_json_editor_key(
    root: &mut Node,
    focus: &mut NodePath,
    mode: &mut EditorMode,
    scalar: &mut Option<(NodePath, String)>,
    schema: &serde_json::Value,
    field_name: &str,
    required: bool,
    key: KeyEvent,
) -> JsonEditorOutcome {
    use crossterm::event::{KeyCode, KeyModifiers};

    // In-place scalar text edit takes priority over tree/raw key handling.
    if let Some((path, buffer)) = scalar.as_mut() {
        match (key.code, key.modifiers) {
            (KeyCode::Enter, _) => {
                // Clone out of the `scalar` borrow before clearing it, so the
                // write-back to `root` and `*scalar = None` don't overlap.
                let path = path.clone();
                let text = buffer.clone();
                set_scalar_value(root, &path, &text);
                *scalar = None;
            }
            (KeyCode::Esc, _) => *scalar = None,
            (KeyCode::Backspace, _) => {
                buffer.pop();
            }
            (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => buffer.push(c),
            _ => {}
        }
        return JsonEditorOutcome::Continue;
    }

    // Mode changes that reborrow `mode`/`root` are deferred past the match so a
    // live `editor` borrow (raw mode) never overlaps the reassignment.
    let mut switch_to_raw = false;
    let mut to_tree_root: Option<Node> = None;
    let outcome = match mode {
        EditorMode::Structured => match json_editor::dispatch_key(root, focus, key) {
            EditorStep::Continue => JsonEditorOutcome::Continue,
            EditorStep::Cancel => JsonEditorOutcome::Cancel,
            EditorStep::Save(value) => JsonEditorOutcome::Save(value),
            EditorStep::OpenRaw => {
                switch_to_raw = true;
                JsonEditorOutcome::Continue
            }
            EditorStep::OpenScalar(path) => {
                let seed = json_editor::navigation::get(root, &path)
                    .and_then(|n| match &n.kind {
                        NodeKind::Scalar { value: Some(sv) } => Some(scalar_value_to_string(sv)),
                        _ => None,
                    })
                    .unwrap_or_default();
                *scalar = Some((path, seed));
                JsonEditorOutcome::Continue
            }
        },
        // Raw mode is a co-equal view of the whole edit: Ctrl-S saves and exits
        // (parsing first; invalid JSON shows the error and stays), Esc cancels.
        // Ctrl-R re-parses into the tree and flips to the structured view.
        // Neither path checks required-ness — that is enforced at Confirm.
        EditorMode::Raw(editor) => match json_editor::raw::dispatch(editor, key) {
            json_editor::raw::RawStep::Continue => JsonEditorOutcome::Continue,
            json_editor::raw::RawStep::Cancel => JsonEditorOutcome::Cancel,
            json_editor::raw::RawStep::Commit => {
                match commit_raw_to_tree(&editor.to_text(), schema, field_name, required) {
                    Ok(new_root) => JsonEditorOutcome::Save(new_root.to_value()),
                    Err(e) => {
                        editor.set_error(e);
                        JsonEditorOutcome::Continue
                    }
                }
            }
            json_editor::raw::RawStep::ToTree => {
                match commit_raw_to_tree(&editor.to_text(), schema, field_name, required) {
                    Ok(new_root) => to_tree_root = Some(new_root),
                    Err(e) => editor.set_error(e),
                }
                JsonEditorOutcome::Continue
            }
        },
    };
    if switch_to_raw {
        *mode = EditorMode::into_raw(root);
    } else if let Some(new_root) = to_tree_root {
        *root = new_root;
        *focus = vec![];
        *mode = EditorMode::Structured;
    }
    outcome
}

/// Drive the structured JSON editor for `fields[idx]`. Returns `Some(Value)`
/// on save, `None` on cancel. The event source is shared with the calling
/// form loop so a scripted test can script form→editor→form in one key stream.
///
/// `inline` controls whether the editor is wrapped in the shared nav chrome.
/// Pass `true` for the inline surface and `false` for fullscreen (renders to
/// the full frame area, unchanged).
pub(crate) fn drive_json_editor<B, F>(
    tty: &mut Terminal<B>,
    form: &mut crate::frontend::terminal::inline::form::Form,
    idx: usize,
    next_event: &mut F,
    inline: bool,
) -> Result<Option<serde_json::Value>, CliError>
where
    B: Backend,
    F: FnMut() -> Result<KeyEvent, CliError>,
{
    let schema = form.fields[idx].schema.clone();
    let current_value = match &form.fields[idx].value {
        FieldValue::JsonBody(s) if !s.is_empty() => {
            serde_json::from_str(s).unwrap_or(serde_json::Value::Null)
        }
        _ => serde_json::Value::Null,
    };

    let field_name = form.fields[idx].label.clone();
    let required = form.fields[idx].required;
    let mut root = from_schema(&field_name, &schema, &current_value, required);
    let mut focus: NodePath = vec![];
    let mut mode = EditorMode::Structured;
    // `Some((path, buffer))` while a text scalar leaf is being edited in place.
    let mut scalar: Option<(NodePath, String)> = None;
    // Baseline for the unsaved-changes dot: the normalised value (and its raw
    // pretty form) when the editor opened. Using `root.to_value()` rather than
    // the field's raw value avoids a false dot from schema normalisation.
    let baseline = root.to_value();
    let baseline_text =
        serde_json::to_string_pretty(&baseline).unwrap_or_else(|_| baseline.to_string());
    // Persisted scroll window for the tree view, so long bodies stay navigable
    // and the window moves minimally as focus changes.
    let tree_scroll = std::cell::Cell::new(0usize);

    loop {
        let nav_ctx = if scalar.is_some() {
            crate::frontend::terminal::views::nav::NavContext::JsonEditScalar
        } else {
            match &mode {
                EditorMode::Raw { .. } => {
                    crate::frontend::terminal::views::nav::NavContext::JsonEditRaw
                }
                EditorMode::Structured => {
                    crate::frontend::terminal::views::nav::NavContext::JsonEditTree
                }
            }
        };

        let editing = scalar.as_ref().map(|(p, b)| (p, b.as_str()));
        let render_body = |frame: &mut ratatui::Frame, area: ratatui::layout::Rect| {
            use ratatui::widgets::{Block, Borders, Padding};
            // Box around the editor body, matching fullscreen's JsonEditPanel. The
            // title names the mode so the nav bar doesn't have to: raw mode reads
            // `Edit raw JSON: <field>`, the tree reads `Edit: <field>`. A leading
            // `●` flags unsaved changes versus the value the editor opened with.
            let modified = match &mode {
                EditorMode::Structured => root.to_value() != baseline,
                EditorMode::Raw(editor) => editor.to_text() != baseline_text,
            };
            let label = match &mode {
                EditorMode::Raw(_) => format!("Edit raw JSON: {field_name}"),
                EditorMode::Structured => format!("Edit: {field_name}"),
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .padding(Padding::new(2, 2, 1, 1))
                .title(json_editor::editor_title_line(modified, &label));
            let inner = block.inner(area);
            frame.render_widget(block, area);
            if inner.height == 0 || inner.width == 0 {
                return;
            }
            match &mode {
                EditorMode::Structured => {
                    use ratatui::layout::{Constraint, Direction, Layout};
                    // Match the main form: scrollable tree on top, a 1-row gap,
                    // then a bordered hint box (focused node's description) pinned
                    // at the bottom. The gap keeps the hint box off the last tree row.
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Min(1),
                            Constraint::Length(1),
                            Constraint::Length(4),
                        ])
                        .split(inner);
                    render_tree_view(frame, chunks[0], &root, &focus, editing, &tree_scroll);
                    let hint = focused_description(&root, &focus).map(|d| (d, false));
                    render_hint_box(frame, chunks[2], hint.as_ref());
                }
                EditorMode::Raw(editor) => match editor.error() {
                    // Raw mode has no descriptions, so the hint box only appears
                    // to carry a commit error (red) — matching the form's error
                    // box. With no error the editor fills the whole area.
                    Some(err) => {
                        use ratatui::layout::{Constraint, Direction, Layout};
                        let chunks = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Min(1),
                                Constraint::Length(1),
                                Constraint::Length(4),
                            ])
                            .split(inner);
                        json_editor::raw::render_raw(frame, chunks[0], editor);
                        render_hint_box(frame, chunks[2], Some(&(err.to_owned(), true)));
                    }
                    None => json_editor::raw::render_raw(frame, inner, editor),
                },
            }
        };

        tty.draw(|f| {
            if inline {
                crate::frontend::terminal::inline::chrome::render(f, nav_ctx, render_body);
            } else {
                let area = f.area();
                render_body(f, area);
            }
        })
        .map_err(|e| CliError::Usage {
            message: format!("TUI draw failed: {e}"),
            metadata: None,
        })?;

        let key = next_event()?;
        if key.kind != crossterm::event::KeyEventKind::Press {
            continue;
        }
        if is_ctrl_c(key) {
            return Ok(None);
        }

        match apply_json_editor_key(
            &mut root,
            &mut focus,
            &mut mode,
            &mut scalar,
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

/// Write a committed scalar-editor string back into the tree node at `path`.
/// The string is coerced to the node's declared schema type. No-op if the
/// path does not resolve to a `Scalar` node.
pub(crate) fn set_scalar_value(root: &mut Node, path: &NodePath, text: &str) {
    use crate::frontend::coerce_to_schema;

    let Some(node) = get_mut(root, path) else {
        return;
    };
    let coerced = coerce_to_schema(text, &node.schema);
    let sv = match &coerced {
        serde_json::Value::String(s) => {
            // Honour enum variant if present, else plain string.
            if node.schema.get("enum").is_some() {
                Some(ScalarValue::Enum(s.clone()))
            } else {
                Some(ScalarValue::String(s.clone()))
            }
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(ScalarValue::Integer(i))
            } else {
                n.as_f64().map(ScalarValue::Number)
            }
        }
        serde_json::Value::Bool(b) => Some(ScalarValue::Boolean(*b)),
        _ => None,
    };
    if let NodeKind::Scalar { value } = &mut node.kind {
        *value = sv;
    }
}

/// Render a scalar JSON-editor value as its display string.
pub(crate) fn scalar_value_to_string(sv: &ScalarValue) -> String {
    match sv {
        ScalarValue::String(s) | ScalarValue::Enum(s) => s.clone(),
        ScalarValue::Integer(i) => i.to_string(),
        ScalarValue::Number(f) => f.to_string(),
        ScalarValue::Boolean(b) => b.to_string(),
    }
}

/// Raw mode swallows SIGINT, so Ctrl-C arrives as an ordinary key. Shared with
/// the fullscreen interaction loops so both treat it identically.
pub(crate) fn is_ctrl_c(key: KeyEvent) -> bool {
    use crossterm::event::{KeyCode, KeyModifiers};
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// The optional-step review gate skips on `s`, but only when no field editor is
/// open — while a scalar field is in edit mode the char must reach the edit
/// buffer, or text inputs become un-typeable for 's'. Shared by both surfaces'
/// optional-review loops (`fullscreen::review_step_optional_inner`,
/// `inline::drive_inline_review_optional_inner`) so the guard rule stays in one
/// place and cannot drift between them.
pub(crate) fn is_optional_skip_key(key: KeyEvent, is_editing: bool) -> bool {
    use crossterm::event::KeyCode;
    key.code == KeyCode::Char('s') && !is_editing
}

/// Route a submitted optional-review form to its `StepReviewOutcome`: Skip when
/// the Skip button was selected, otherwise Proceed with the projected edits.
/// Shared by both surfaces' optional-review loops so the submit-routing rule
/// stays in one place. Call with the form already moved out of its
/// surface-specific holder.
pub(crate) fn confirm_skip_outcome(
    form: &crate::frontend::terminal::inline::form::Form,
    plan: &ags_protocol::workflow::StepFieldPlan,
) -> ags_protocol::workflow::StepReviewOutcome {
    use crate::frontend::terminal::inline::form::ConfirmSkipChoice;
    use ags_protocol::workflow::StepReviewOutcome;
    match form.selected_confirm_skip() {
        ConfirmSkipChoice::Skip => StepReviewOutcome::Skip,
        ConfirmSkipChoice::Confirm => StepReviewOutcome::Proceed(form.project_step_edits(plan)),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::terminal::inline::form::{
        FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
    };
    use ags_protocol::workflow::GatherSlotId;
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    use ratatui::{backend::TestBackend, Terminal};

    /// Build a plain (no-modifier) key-press event for `code`.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Build a key event for `code` with an explicit press/release `kind`.
    fn key_with_kind(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind)
    }

    /// Build a Ctrl-modified key event for `code`.
    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn test_build_gather_confirm_card_strips_control_sequences_from_supplied() {
        // Supplied values can carry prior-step API captures; terminal control
        // sequences must be stripped before they render into the confirm card.
        let supplied = vec![SuppliedInputView {
            label: "region".into(),
            value: serde_json::Value::String("\x1b[32meu-west\x1b[0m".into()),
            schema: serde_json::Value::Null,
            description: None,
            source: ags_protocol::workflow::SuppliedSource::Default,
            location: ags_protocol::workflow::StepFieldLocation::default(),
        }];
        let card = build_gather_confirm_card(&[], &supplied, &GatherResult::default());
        let (label, value) = &card.summary[0];
        assert_eq!(label, "region");
        assert!(
            !value.contains('\x1b'),
            "control sequences must be stripped: {value:?}"
        );
        assert!(
            value.contains("eu-west"),
            "visible content must be preserved: {value:?}"
        );
    }

    /// Build a scripted event source from a `Vec<KeyEvent>`.
    fn scripted(keys: Vec<KeyEvent>) -> impl FnMut() -> Result<KeyEvent, CliError> {
        let mut iter = keys.into_iter();
        move || {
            iter.next().ok_or_else(|| CliError::Usage {
                message: "test key stream exhausted".into(),
                metadata: None,
            })
        }
    }

    /// Build an 80x24 test terminal backed by ratatui's `TestBackend`.
    fn make_tty() -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(80, 24)).expect("TestBackend")
    }

    #[test]
    fn test_sort_gather_fields_by_location_orders_params_before_body() {
        use crate::frontend::terminal::inline::form::{FieldType, FieldValue};
        use ags_protocol::workflow::{AutoDeriveScope, StepFieldLocation, WorkflowInputNeeded};

        let needed_field = |label: &str, ty: FieldType| FormField {
            label: label.into(),
            field_type: ty,
            required: true,
            value: FieldValue::Empty,
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input(label.into()),
            schema: serde_json::json!({}),
            read_only: false,
            dynamic: None,
        };
        // Built in a deliberately scrambled order: body, header, query, path.
        let mut fields = vec![
            needed_field("body", FieldType::JsonBody),
            needed_field("trace-id", FieldType::Scalar),
            needed_field("limit", FieldType::Scalar),
            needed_field("namespace", FieldType::Scalar),
        ];
        let slot = |label: &str, loc: StepFieldLocation| WorkflowInputNeeded {
            id: GatherSlotId(0),
            label: label.into(),
            description: None,
            schema: serde_json::json!({}),
            default: None,
            required: true,
            sensitive: false,
            scope: AutoDeriveScope::StepLocal {
                field_name: label.into(),
            },
            location: loc,
        };
        let needed = vec![
            slot("body", StepFieldLocation::Body),
            slot("trace-id", StepFieldLocation::Header),
            slot("limit", StepFieldLocation::Query),
            slot("namespace", StepFieldLocation::Path),
        ];
        sort_gather_fields_by_location(&mut fields, &needed, &[]);
        let order: Vec<&str> = fields.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(
            order,
            vec!["namespace", "limit", "trace-id", "body"],
            "path → query → header → body, JSON body last"
        );
    }

    #[test]
    fn test_full_surface_confirm_card_orders_rows_by_field_position() {
        use crate::frontend::terminal::inline::form::{FieldType, FieldValue};
        use ags_protocol::workflow::GatherResult;

        let input_field = |name: &str, ty: FieldType| FormField {
            label: name.into(),
            field_type: ty,
            required: true,
            value: FieldValue::Empty,
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input(name.into()),
            schema: serde_json::json!({}),
            read_only: false,
            dynamic: None,
        };
        // Form order: role-id (path scalar) first, permissions (body) last.
        let fields = vec![
            input_field("role-id", FieldType::Scalar),
            input_field("permissions", FieldType::JsonBody),
        ];
        // Overrides inserted body-first to prove the card does not just echo map
        // order; the card must follow `fields`, not alphabetical or insertion order.
        let mut result = GatherResult::default();
        result
            .input_overrides
            .insert("permissions".into(), serde_json::json!([{ "action": 2 }]));
        result
            .input_overrides
            .insert("role-id".into(), serde_json::json!("r-1"));

        let card = build_full_surface_confirm_card(&fields, &result);
        let labels: Vec<&str> = card.summary.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(
            labels,
            vec!["role-id", "permissions"],
            "review follows the form's request-location order (path before body), \
             not alphabetical (which would put permissions first)"
        );
    }

    /// Build a scalar `FormField` fixture bound to gather `slot`.
    fn scalar_field(label: &str, required: bool, slot: u32) -> FormField {
        FormField {
            label: label.into(),
            field_type: FieldType::Scalar,
            required,
            value: FieldValue::Empty,
            description: String::new(),
            source: FieldSource::Default,
            key: FieldKey::Slot(GatherSlotId(slot)),
            schema: serde_json::json!({"type": "string"}),
            read_only: false,
            dynamic: None,
        }
    }

    // ── form driver ──────────────────────────────────────────────────────────

    #[test]
    fn test_run_form_submits_after_required_filled() {
        // Script: Enter (begin edit on slot 0), 'x', Enter (commit),
        //         Enter (submit — all_required_filled now true).
        let keys = vec![
            key(KeyCode::Enter),     // begin_edit
            key(KeyCode::Char('x')), // type 'x'
            key(KeyCode::Enter),     // commit_edit
            key(KeyCode::Enter),     // submit
        ];
        let form = Form::new("test", vec![scalar_field("name", true, 0)]);
        let phase = FormPhase::new(form);
        let mut tty = make_tty();
        let result = drive_form(&mut tty, phase, &FormLayout::Inline, scripted(keys))
            .expect("drive_form should not error");
        match result {
            FormRunResult::Submitted(gr) => {
                assert_eq!(
                    gr.slot_values.get(&GatherSlotId(0)),
                    Some(&serde_json::Value::String("x".into())),
                    "slot 0 must have value 'x'"
                );
            }
            FormRunResult::Cancelled => panic!("expected Submitted, got Cancelled"),
        }
    }

    #[test]
    fn test_run_form_ignores_key_release_events() {
        let keys = vec![
            key(KeyCode::Enter),
            key(KeyCode::Char('x')),
            key_with_kind(KeyCode::Char('x'), KeyEventKind::Release),
            key(KeyCode::Enter),
            key(KeyCode::Enter),
        ];
        let form = Form::new("test", vec![scalar_field("name", true, 0)]);
        let phase = FormPhase::new(form);
        let mut tty = make_tty();
        let result = drive_form(&mut tty, phase, &FormLayout::Inline, scripted(keys))
            .expect("drive_form should not error");
        match result {
            FormRunResult::Submitted(gr) => {
                assert_eq!(
                    gr.slot_values.get(&GatherSlotId(0)),
                    Some(&serde_json::Value::String("x".into())),
                    "release events must not duplicate typed input"
                );
            }
            FormRunResult::Cancelled => panic!("expected Submitted, got Cancelled"),
        }
    }

    #[test]
    fn test_run_form_esc_cancels() {
        let keys = vec![key(KeyCode::Esc)];
        let form = Form::new("test", vec![scalar_field("name", true, 0)]);
        let phase = FormPhase::new(form);
        let mut tty = make_tty();
        let result = drive_form(&mut tty, phase, &FormLayout::Inline, scripted(keys))
            .expect("drive_form should not error");
        assert!(matches!(result, FormRunResult::Cancelled));
    }

    #[test]
    fn test_run_form_ctrl_s_submits_when_required_filled() {
        // Pre-fill the field so ctrl-s is immediately valid.
        let mut field = scalar_field("name", true, 0);
        field.value = FieldValue::Scalar("preset".into());
        let form = Form::new("test", vec![field]);
        let phase = FormPhase::new(form);
        let mut tty = make_tty();
        let keys = vec![ctrl(KeyCode::Char('s'))];
        let result = drive_form(&mut tty, phase, &FormLayout::Inline, scripted(keys))
            .expect("drive_form should not error");
        assert!(matches!(result, FormRunResult::Submitted(_)));
    }

    // ── render test ─────────────────────────────────────────────────────────

    /// `drive_form` must render the Parameters box and the Navigation bar with
    /// `[Tab] move`.
    #[test]
    fn test_drive_form_renders_chrome_layout() {
        let mut field = scalar_field("name", true, 0);
        field.value = FieldValue::Scalar("preset".into());
        let form = Form::new("test", vec![field]);
        let phase = FormPhase::new(form);
        // Larger terminal so both regions (main/nav) are visible.
        let mut tty = Terminal::new(TestBackend::new(80, 30)).expect("TestBackend");
        let keys = vec![ctrl(KeyCode::Char('s'))]; // submit immediately
        drive_form(&mut tty, phase, &FormLayout::Inline, scripted(keys))
            .expect("drive_form should not error");
        let buf: String = tty
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(buf.contains("Parameters"), "Parameters box present: {buf}");
        assert!(buf.contains("Navigation"), "Navigation box present: {buf}");
        assert!(buf.contains("[Tab] move"), "fields keymap present: {buf}");
    }

    /// With an `optional_filter` form that has a hidden optional row, the nav bar
    /// shows the `[o] show optional (+N)` toggle affordance.
    #[test]
    fn test_drive_form_shows_optional_toggle_in_nav() {
        let mut required = scalar_field("name", true, 0);
        required.value = FieldValue::Scalar("preset".into());
        let optional = scalar_field("extra", false, 1); // empty optional → hidden
        let form = Form::new("test", vec![required, optional])
            .with_submit_focusable(true)
            .with_optional_filter(true);
        let phase = FormPhase::new(form);
        // Wide enough (chrome clamps to MAX_WIDTH) that the full Fields keymap +
        // the appended toggle fit on the nav line without clipping.
        let mut tty = Terminal::new(TestBackend::new(120, 30)).expect("TestBackend");
        let keys = vec![ctrl(KeyCode::Char('s'))]; // submit immediately (required filled)
        drive_form(&mut tty, phase, &FormLayout::Inline, scripted(keys))
            .expect("drive_form should not error");
        let buf: String = tty
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            buf.contains("show optional (+1)"),
            "nav shows the optional toggle: {buf}"
        );
    }

    // ── confirm driver ───────────────────────────────────────────────────────

    #[test]
    fn test_drive_confirm_enter_on_confirm_returns_confirmed() {
        let card = ConfirmCard::new("POST /foo", vec![]);
        let phase = ConfirmCardPhase::new(card);
        let mut tty = make_tty();
        let keys = vec![key(KeyCode::Enter)]; // default focus = Confirm
        let result = drive_confirm(
            &mut tty,
            phase,
            crate::frontend::terminal::views::nav::NavContext::ConfirmCard,
            scripted(keys),
        )
        .expect("drive_confirm should not error");
        assert!(matches!(result, ConfirmOutcome::Confirmed));
    }

    #[test]
    fn test_drive_confirm_esc_cancels() {
        let card = ConfirmCard::new("DELETE /bar", vec![]);
        let phase = ConfirmCardPhase::new(card);
        let mut tty = make_tty();
        let keys = vec![key(KeyCode::Esc)];
        let result = drive_confirm(
            &mut tty,
            phase,
            crate::frontend::terminal::views::nav::NavContext::ConfirmCard,
            scripted(keys),
        )
        .expect("drive_confirm should not error");
        assert!(matches!(result, ConfirmOutcome::Cancelled));
    }

    #[test]
    fn test_drive_confirm_b_returns_back_to_edit() {
        let card = ConfirmCard::new("PATCH /baz", vec![]);
        let phase = ConfirmCardPhase::new(card);
        let mut tty = make_tty();
        // `b` is "back to edit" (offered on the gather confirm card).
        let keys = vec![key(KeyCode::Char('b'))];
        let result = drive_confirm(
            &mut tty,
            phase,
            crate::frontend::terminal::views::nav::NavContext::ConfirmCard,
            scripted(keys),
        )
        .expect("drive_confirm should not error");
        assert!(matches!(result, ConfirmOutcome::BackToEdit));
    }

    /// Pressing `s` on a card that offers `ConfirmAction::Skip` returns
    /// `ConfirmOutcome::Skipped`. This covers the inline per-step confirm gate
    /// for optional steps (Task 5).
    #[test]
    fn test_drive_confirm_s_with_skip_action_returns_skipped() {
        let card = ConfirmCard::new("Step 2: create-item", vec![]);
        let phase = ConfirmCardPhase::new(card).with_actions(&[
            ConfirmAction::Confirm,
            ConfirmAction::Skip,
            ConfirmAction::Cancel,
        ]);
        let mut tty = make_tty();
        let keys = vec![key(KeyCode::Char('s'))];
        let result = drive_confirm(
            &mut tty,
            phase,
            crate::frontend::terminal::views::nav::NavContext::ConfirmSkippable,
            scripted(keys),
        )
        .expect("drive_confirm should not error");
        assert!(matches!(result, ConfirmOutcome::Skipped));
    }

    /// `drive_confirm` must render the card's method/path and the ConfirmCard
    /// nav bar (`Navigation`, `move`, `select`).
    #[test]
    fn test_drive_confirm_renders_chrome_layout() {
        let card = ConfirmCard::new(
            "POST /sessions/v1/public/namespaces/ns/servers",
            vec![("namespace".into(), "test-ns".into())],
        );
        let phase = ConfirmCardPhase::new(card);
        // Larger terminal so both regions (main/nav) are visible.
        let mut tty = Terminal::new(TestBackend::new(80, 30)).expect("TestBackend");
        // Enter confirms immediately (default focus = Confirm).
        let keys = vec![key(KeyCode::Enter)];
        drive_confirm(
            &mut tty,
            phase,
            crate::frontend::terminal::views::nav::NavContext::ConfirmCard,
            scripted(keys),
        )
        .expect("drive_confirm should not error");
        let buf: String = tty
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            buf.contains("POST /sessions/v1/public/namespaces/ns/servers"),
            "card method/path present: {buf}"
        );
        assert!(buf.contains("Navigation"), "Navigation box present: {buf}");
        assert!(
            buf.contains("confirm"),
            "ConfirmCard 'confirm' hint present: {buf}"
        );
        assert!(
            buf.contains("cancel"),
            "ConfirmCard 'cancel' hint present: {buf}"
        );
    }

    // ── json editor render ───────────────────────────────────────────────────

    /// Build a JSON-body `FormField` fixture bound to gather `slot`.
    fn json_body_field(label: &str, required: bool, slot: u32) -> FormField {
        FormField {
            label: label.into(),
            field_type: crate::frontend::terminal::inline::form::FieldType::JsonBody,
            required,
            value: FieldValue::JsonBody(String::new()),
            description: String::new(),
            source: FieldSource::Default,
            key: FieldKey::Slot(GatherSlotId(slot)),
            schema: serde_json::json!({"type": "object", "properties": {}}),
            read_only: false,
            dynamic: None,
        }
    }

    /// `drive_json_editor` with `inline=true` must render the `Navigation` box
    /// and a `JsonEditTree` keymap token (`raw JSON` or `expand`).
    #[test]
    fn test_drive_json_editor_renders_chrome_layout() {
        let mut form = crate::frontend::terminal::inline::form::Form::new(
            "test",
            vec![json_body_field("body", false, 0)],
        );
        // Larger terminal so both regions (main/nav) are visible.
        let mut tty = Terminal::new(TestBackend::new(80, 30)).expect("TestBackend");
        // Ctrl-S saves immediately (no required fields to fill).
        let keys = vec![ctrl(KeyCode::Char('s'))];
        drive_json_editor(&mut tty, &mut form, 0, &mut scripted(keys), true)
            .expect("drive_json_editor should not error");
        let buf: String = tty
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(buf.contains("Navigation"), "Navigation box present: {buf}");
        // JsonEditTree nav includes the "expand" token.
        assert!(
            buf.contains("expand"),
            "JsonEditTree nav token present: {buf}"
        );
    }

    #[test]
    fn test_inline_json_editor_edits_scalar_leaf_in_place() {
        // Schema with a single string leaf. Script: Down (focus the leaf),
        // Enter (open scalar in place), 'h','i' (type), Enter (commit),
        // Ctrl-S (save the tree).
        let mut form = crate::frontend::terminal::inline::form::Form::new(
            "test",
            vec![FormField {
                label: "body".into(),
                field_type: crate::frontend::terminal::inline::form::FieldType::JsonBody,
                required: false,
                value: FieldValue::JsonBody(String::new()),
                description: String::new(),
                source: FieldSource::Default,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": { "name": { "type": "string" } }
                }),
                read_only: false,
                dynamic: None,
            }],
        );
        let mut tty = Terminal::new(TestBackend::new(80, 30)).expect("TestBackend");
        let keys = vec![
            key(KeyCode::Down), // focus the "name" leaf (root path vec![], child path vec![0])
            key(KeyCode::Enter), // open in-place scalar edit
            key(KeyCode::Char('h')),
            key(KeyCode::Char('i')),
            key(KeyCode::Enter),      // commit leaf
            ctrl(KeyCode::Char('s')), // save tree
        ];
        let result = drive_json_editor(&mut tty, &mut form, 0, &mut scripted(keys), true)
            .expect("editor should not error")
            .expect("Ctrl-S saves a value");
        assert_eq!(result, serde_json::json!({ "name": "hi" }));
    }

    /// Raw mode is genuinely editable AND saves the whole edit in one step: open
    /// it, clear the seed `{}`, type a new object, then Ctrl-S saves and exits
    /// (no separate apply-to-tree step). Guards both the append-only fix and the
    /// unified raw save/cancel semantics.
    #[test]
    fn test_inline_json_editor_raw_mode_edits_and_commits() {
        let mut form = crate::frontend::terminal::inline::form::Form::new(
            "test",
            vec![FormField {
                label: "body".into(),
                field_type: crate::frontend::terminal::inline::form::FieldType::JsonBody,
                required: false,
                value: FieldValue::JsonBody(String::new()),
                description: String::new(),
                source: FieldSource::Default,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": { "name": { "type": "string" } }
                }),
                read_only: false,
                dynamic: None,
            }],
        );
        let mut tty = Terminal::new(TestBackend::new(80, 30)).expect("TestBackend");
        // Seed buffer is `{}` with the cursor at the start. Move to the end,
        // delete both braces, type a fresh object, apply, then save the tree.
        let mut keys = vec![
            ctrl(KeyCode::Char('r')), // open raw mode (Ctrl-R)
            key(KeyCode::End),        // cursor after `}`
            key(KeyCode::Backspace),  // delete `}`
            key(KeyCode::Backspace),  // delete `{`
        ];
        for c in r#"{"name":"hi"}"#.chars() {
            keys.push(key(KeyCode::Char(c)));
        }
        keys.push(ctrl(KeyCode::Char('s'))); // save and exit from raw mode
        let result = drive_json_editor(&mut tty, &mut form, 0, &mut scripted(keys), true)
            .expect("editor should not error")
            .expect("Ctrl-S saves a value");
        assert_eq!(result, serde_json::json!({ "name": "hi" }));
    }

    /// Ctrl-R re-parses the raw buffer into the tree and switches to structured
    /// view — raw edits carry over rather than being lost. After switching, a
    /// tree-mode Ctrl-S saves the value that was typed in raw.
    #[test]
    fn test_inline_json_editor_raw_ctrl_r_syncs_into_tree() {
        let mut form = crate::frontend::terminal::inline::form::Form::new(
            "test",
            vec![FormField {
                label: "body".into(),
                field_type: crate::frontend::terminal::inline::form::FieldType::JsonBody,
                required: false,
                value: FieldValue::JsonBody(String::new()),
                description: String::new(),
                source: FieldSource::Default,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": { "name": { "type": "string" } }
                }),
                read_only: false,
                dynamic: None,
            }],
        );
        let mut tty = Terminal::new(TestBackend::new(80, 30)).expect("TestBackend");
        let mut keys = vec![
            ctrl(KeyCode::Char('r')), // open raw mode (Ctrl-R)
            key(KeyCode::End),
            key(KeyCode::Backspace), // delete `}`
            key(KeyCode::Backspace), // delete `{`
        ];
        for c in r#"{"name":"hi"}"#.chars() {
            keys.push(key(KeyCode::Char(c)));
        }
        keys.push(ctrl(KeyCode::Char('r'))); // back to tree (edits sync over)
        keys.push(ctrl(KeyCode::Char('s'))); // save tree
        let result = drive_json_editor(&mut tty, &mut form, 0, &mut scripted(keys), true)
            .expect("editor should not error")
            .expect("Ctrl-S saves a value");
        assert_eq!(result, serde_json::json!({ "name": "hi" }));
    }

    #[test]
    fn test_inline_json_editor_renders_edit_box_title() {
        let mut form = crate::frontend::terminal::inline::form::Form::new(
            "test",
            vec![json_body_field("payload", false, 0)],
        );
        let mut tty = Terminal::new(TestBackend::new(80, 30)).expect("TestBackend");
        let keys = vec![ctrl(KeyCode::Char('s'))];
        drive_json_editor(&mut tty, &mut form, 0, &mut scripted(keys), true)
            .expect("editor should not error");
        let buf: String = tty
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(buf.contains("Edit: payload"), "editor body is boxed: {buf}");
    }

    /// `drive_json_editor` with `inline=false` renders to the full area without
    /// nav chrome — the `Navigation` box must NOT appear.
    #[test]
    fn test_drive_json_editor_no_chrome_renders_full_area() {
        let mut form = crate::frontend::terminal::inline::form::Form::new(
            "test",
            vec![json_body_field("body", false, 0)],
        );
        let mut tty = Terminal::new(TestBackend::new(80, 24)).expect("TestBackend");
        // Ctrl-S saves (no content yet → saves Null, but the loop exits).
        let keys = vec![ctrl(KeyCode::Char('s'))];
        drive_json_editor(&mut tty, &mut form, 0, &mut scripted(keys), false)
            .expect("drive_json_editor should not error");
        let buf: String = tty
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            !buf.contains("Navigation"),
            "no chrome: nav must not appear: {buf}"
        );
    }

    // ── set_scalar_value ─────────────────────────────────────────────────────

    #[test]
    fn test_set_scalar_value_writes_string_into_node() {
        let schema = serde_json::json!({"type": "string"});
        let mut root = from_schema("name", &schema, &serde_json::Value::Null, false);
        set_scalar_value(&mut root, &vec![], "hello");
        match &root.kind {
            NodeKind::Scalar {
                value: Some(ScalarValue::String(s)),
            } => {
                assert_eq!(s, "hello");
            }
            other => panic!("unexpected kind: {other:?}"),
        }
    }

    #[test]
    fn test_set_scalar_value_coerces_integer() {
        let schema = serde_json::json!({"type": "integer"});
        let mut root = from_schema("count", &schema, &serde_json::Value::Null, false);
        set_scalar_value(&mut root, &vec![], "42");
        match &root.kind {
            NodeKind::Scalar {
                value: Some(ScalarValue::Integer(n)),
            } => {
                assert_eq!(*n, 42);
            }
            other => panic!("unexpected kind: {other:?}"),
        }
    }

    // ── build_gather_confirm_card ─────────────────────────────────────────────

    /// `build_gather_confirm_card` includes a supplied input with its override
    /// value when the gather result contains an override.
    #[test]
    fn test_build_gather_confirm_card_shows_override_for_supplied_input() {
        use ags_protocol::workflow::{GatherResult, SuppliedInputView, SuppliedSource};
        use std::collections::BTreeMap;

        let supplied = vec![SuppliedInputView {
            label: "namespace".into(),
            value: serde_json::json!("original-ns"),
            schema: serde_json::json!({"type": "string"}),
            description: None,
            source: SuppliedSource::FromFlag,
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }];
        let mut input_overrides = BTreeMap::new();
        input_overrides.insert("namespace".to_string(), serde_json::json!("edited-ns"));
        let result = GatherResult {
            slot_values: BTreeMap::new(),
            input_overrides,
        };

        let card = build_gather_confirm_card(&[], &supplied, &result);
        assert_eq!(card.title, "Review request");
        assert_eq!(card.summary.len(), 1);
        assert_eq!(card.summary[0].0, "namespace");
        assert_eq!(card.summary[0].1, "edited-ns");
    }

    // ── run_full_surface_gather ───────────────────────────────────────────────

    #[test]
    fn test_full_surface_gather_projects_filled_optionals_as_overrides() {
        use crate::frontend::terminal::inline::form::{FieldType, FormField};
        // One required (pre-filled) + one optional (we will fill it).
        let fields = vec![
            FormField {
                label: "namespace".into(),
                field_type: FieldType::Scalar,
                required: true,
                value: FieldValue::Scalar("acme".into()),
                description: String::new(),
                source: FieldSource::FromFlag,
                key: FieldKey::Input("namespace".into()),
                schema: serde_json::json!({"type":"string"}),
                read_only: false,
                dynamic: None,
            },
            FormField {
                label: "client-name".into(),
                field_type: FieldType::Scalar,
                required: false,
                value: FieldValue::Empty,
                description: String::new(),
                source: FieldSource::UserInput,
                key: FieldKey::Input("clientName".into()),
                schema: serde_json::json!({"type":"string"}),
                read_only: false,
                dynamic: None,
            },
        ];
        let mut tty = Terminal::new(TestBackend::new(80, 30)).expect("TestBackend");
        // Script: 'o' (reveal optional), Tab (focus client-name), Enter (edit),
        // 'b','o','t', Enter (commit), Ctrl-S (submit), Enter (confirm card).
        let keys = vec![
            key(KeyCode::Char('o')),
            key(KeyCode::Tab),
            key(KeyCode::Enter),
            key(KeyCode::Char('b')),
            key(KeyCode::Char('o')),
            key(KeyCode::Char('t')),
            key(KeyCode::Enter),
            ctrl(KeyCode::Char('s')),
            key(KeyCode::Enter), // confirm card default focus = Confirm
        ];
        let result =
            run_full_surface_gather(&mut tty, "iam clients create", fields, &mut scripted(keys))
                .expect("gather should not error");
        assert_eq!(
            result.input_overrides.get("clientName"),
            Some(&serde_json::json!("bot"))
        );
        assert_eq!(
            result.input_overrides.get("namespace"),
            Some(&serde_json::json!("acme"))
        );
    }

    // ── dynamic-enum picker sub-loop ──────────────────────────────────────────

    /// A dynamic-enum `OptionsSource` with `fallback_description: None`, mirroring
    /// the fixtures used in `dynamic_options`'s tests.
    fn picker_options_source() -> ags_protocol::workflow::OptionsSource {
        ags_protocol::workflow::OptionsSource {
            operation: ags_protocol::workflow::OperationReference {
                service: ags_protocol::catalogue::ServiceId::new("iam"),
                operation: ags_protocol::catalogue::OperationId::new("iam/admin/users/v3/search"),
            },
            parameters: std::collections::BTreeMap::new(),
            items_path: "$.data".into(),
            value: "$.userId".into(),
            label: None,
            label_detail: None,
            fallback_description: None,
            filter: None,
        }
    }

    /// A form with a `namespace` scalar (filled or empty) and a `userId`
    /// `DynamicEnum` field that depends on it. A filled `namespace` satisfies the
    /// dependency, so the picker fetches; an empty one blocks it.
    fn picker_form_with_namespace(namespace_value: FieldValue) -> Form {
        use crate::frontend::terminal::inline::form::DynamicEnumState;
        let namespace = FormField {
            label: "namespace".into(),
            field_type: FieldType::Scalar,
            required: true,
            value: namespace_value,
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("namespace".into()),
            schema: serde_json::json!({"type": "string"}),
            read_only: false,
            dynamic: None,
        };
        let user_id = FormField {
            label: "userId".into(),
            field_type: FieldType::DynamicEnum,
            required: true,
            value: FieldValue::Enum(None),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("userId".into()),
            schema: serde_json::json!({"type": "string"}),
            read_only: false,
            dynamic: Some(DynamicEnumState {
                source: picker_options_source(),
                deps: vec!["namespace".into()],
                optional_deps: vec![],
                resolved: None,
            }),
        };
        Form::new("pick", vec![namespace, user_id])
    }

    fn picker_form_fixture() -> Form {
        picker_form_with_namespace(FieldValue::Scalar("dev".into()))
    }

    fn picker_form_fixture_unfilled_dep() -> Form {
        picker_form_with_namespace(FieldValue::Empty)
    }

    #[test]
    fn test_inline_picker_open_filter_select_writes_value() {
        use crate::frontend::dynamic_options::CannedFetch;
        use ags_protocol::workflow::OptionChoice;

        // A form with one DynamicEnum field whose single dependency is already filled.
        let mut form = picker_form_fixture();
        let fetch = CannedFetch::ok(vec![
            OptionChoice {
                label: "ada".into(),
                value: "u-1".into(),
            },
            OptionChoice {
                label: "adam".into(),
                value: "u-2".into(),
            },
        ]);
        let mut tty = make_tty();
        // type 'a' 'd' 'a' 'm' to filter to adam, Enter to select.
        let mut keys = scripted(vec![
            key(KeyCode::Char('a')),
            key(KeyCode::Char('d')),
            key(KeyCode::Char('a')),
            key(KeyCode::Char('m')),
            key(KeyCode::Enter),
        ]);
        let idx = form
            .fields
            .iter()
            .position(|f| f.label == "userId")
            .unwrap();
        drive_enum_picker(&mut tty, &mut form, idx, Some(&fetch), &mut keys)
            .expect("picker completes");
        assert!(
            matches!(&form.fields[idx].value, FieldValue::Enum(Some(v)) if v == "u-2"),
            "selected value written back: {:?}",
            form.fields[idx].value
        );
    }

    #[test]
    fn test_inline_picker_blocks_when_dependency_unfilled() {
        use crate::frontend::dynamic_options::CannedFetch;
        let mut form = picker_form_fixture_unfilled_dep();
        let fetch = CannedFetch::ok(vec![]);
        let mut tty = make_tty();
        let mut keys = scripted(vec![]); // no keys read: it returns before the list
        let idx = form
            .fields
            .iter()
            .position(|f| f.label == "userId")
            .unwrap();
        drive_enum_picker(&mut tty, &mut form, idx, Some(&fetch), &mut keys).unwrap();
        assert!(
            form.validation_note
                .as_deref()
                .unwrap_or("")
                .contains("first"),
            "unfilled dep sets a validation note: {:?}",
            form.validation_note
        );
        // Field left unset.
        assert!(matches!(
            form.fields[idx].value,
            crate::frontend::terminal::inline::form::FieldValue::Enum(None)
                | crate::frontend::terminal::inline::form::FieldValue::Scalar(_)
        ));
    }

    /// With no declared specs, `collect_inputs_form` returns the current map and
    /// the default `ReviewInputSteps` mode WITHOUT driving the terminal (the
    /// `drive` closure must not run).
    #[test]
    fn test_collect_inputs_form_empty_specs_yields_default_mode_without_driving() {
        use ags_protocol::workflow::RunMode;
        let current =
            std::collections::BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let result = collect_inputs_form(&[], &current, true, |_form| {
            panic!("drive must not run when there are no specs to collect");
        })
        .unwrap();
        assert_eq!(result, Some((current, RunMode::ReviewInputSteps)));
    }
}
