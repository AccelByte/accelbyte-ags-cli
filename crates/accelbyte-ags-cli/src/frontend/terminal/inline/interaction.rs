//! `InlineInteraction` — workflow interaction for the inline terminal frontend.

use std::cell::RefCell;
use std::rc::Rc;

use crate::errors::CliError;
use crate::frontend::terminal::inline::session::InlineSession;
use crate::frontend::ExecutionInteraction;
use ags_protocol::workflow::{
    CompiledStep, GatherResult, StepPreview, SuppliedInputView, WorkflowInputNeeded,
};

/// Workflow interaction handler for the inline terminal frontend.
///
/// Holds a shared reference to the active `InlineSession` so it can drive
/// prompt and confirm phases on the same terminal the rendering frontend uses.
/// Construct with [`InlineInteraction::new`] — do not use struct-literal syntax.
pub struct InlineInteraction {
    session: Rc<RefCell<InlineSession>>,
    /// `Some` for a synthesised single command → build the full request surface
    /// (every param + body, required + optional). `None` for forced `--ui=inline`
    /// workflows → gather only what's needed (existing behaviour).
    full_surface_inputs: Option<Vec<ags_protocol::workflow::WorkflowInputSpec>>,
    options_fetch: Option<Box<dyn crate::frontend::dynamic_options::OptionsFetch>>,
}

impl InlineInteraction {
    /// Construct a `InlineInteraction` from a shared session handle.
    ///
    /// This is the sole assembly point. Pass `frontend.session.clone()` to
    /// share the same terminal with the rendering `InlineFrontend`. Pass
    /// `full_surface_inputs` as `Some(..)` for a synthesised single command to
    /// gather the whole request surface, or `None` for a forced `--ui=inline`
    /// workflow to gather only what's needed.
    pub(crate) fn new(
        session: Rc<RefCell<InlineSession>>,
        full_surface_inputs: Option<Vec<ags_protocol::workflow::WorkflowInputSpec>>,
    ) -> Self {
        Self {
            session,
            full_surface_inputs,
            options_fetch: None,
        }
    }

    pub(crate) fn with_options_fetch(
        mut self,
        fetch: Box<dyn crate::frontend::dynamic_options::OptionsFetch>,
    ) -> Self {
        self.options_fetch = Some(fetch);
        self
    }
}

impl ExecutionInteraction for InlineInteraction {
    fn gather_workflow_inputs(
        &mut self,
        needed: &[WorkflowInputNeeded],
        step_context: &CompiledStep,
        supplied: &[SuppliedInputView],
    ) -> Result<GatherResult, CliError> {
        use crate::frontend::terminal::form_runner::{
            crossterm_next_key, run_full_surface_gather, run_gather, FormLayout,
        };

        // Full-surface single command: build the whole request surface and gather
        // through the advanced-toggle form, regardless of `needed`/`supplied`.
        if let Some(full_inputs) = self.full_surface_inputs.clone() {
            let mut session = self.session.borrow_mut();
            let terminal = session.terminal_mut()?;
            let title = step_context
                .description
                .clone()
                .unwrap_or_else(|| "Provide inputs".to_string());
            let fields = crate::frontend::terminal::inline::form_builder::build_full_surface_fields(
                &full_inputs,
                supplied,
            );
            let fields = sort_full_surface_fields(fields, &full_inputs);
            return run_full_surface_gather(terminal, &title, fields, &mut crossterm_next_key);
        }

        // Nothing to gather — return empty without touching the terminal.
        // (The executor only calls this with non-empty `needed`; this guard
        // keeps the no-terminal unit test green and avoids drawing an empty form.)
        if needed.is_empty() {
            return Ok(GatherResult::default());
        }

        let mut session = self.session.borrow_mut();
        let terminal = session.terminal_mut()?;
        run_gather(terminal, needed, supplied, step_context, FormLayout::Inline)
    }

    fn confirm_step(
        &mut self,
        step: &CompiledStep,
        preview: &StepPreview,
    ) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
        use crate::frontend::terminal::form_runner::{run_confirm_phase, ConfirmOutcome};
        use crate::frontend::terminal::inline::phases::confirm_card::{
            ConfirmAction, ConfirmCard, ConfirmCardPhase,
        };
        use crate::frontend::terminal::views::nav::NavContext;
        use ags_protocol::workflow::StepConfirmOutcome;

        // The confirm step renders through the same chrome (main body + nav bar)
        // as every other inline step, as a padded card with the `Confirm`
        // button — not the old bare y/n prompt. The draw/key loop is
        // the shared, unit-tested `run_confirm_phase` driver; here we only pick
        // the offered actions and the nav bar context, then map the outcome to
        // the confirm outcome.
        let card = ConfirmCard::new(
            format!("Step {}: {}", preview.step_index + 1, preview.step_id),
            vec![],
        )
        .with_message(preview.step_label.clone())
        .caution();

        // Optional steps offer Skip so the user can skip the step and continue.
        // Non-optional steps keep Confirm / Cancel only.
        let (phase, nav_ctx) = if step.is_optional {
            (
                ConfirmCardPhase::new(card).with_actions(&[
                    ConfirmAction::Confirm,
                    ConfirmAction::Skip,
                    ConfirmAction::Cancel,
                ]),
                NavContext::ConfirmSkippable,
            )
        } else {
            (
                ConfirmCardPhase::new(card)
                    .with_actions(&[ConfirmAction::Confirm, ConfirmAction::Cancel]),
                NavContext::Confirm,
            )
        };

        let mut session = self.session.borrow_mut();
        let terminal = session.terminal_mut()?;
        match run_confirm_phase(terminal, phase, nav_ctx)? {
            ConfirmOutcome::Confirmed => Ok(StepConfirmOutcome::Proceed),
            ConfirmOutcome::Skipped => Ok(StepConfirmOutcome::Skip),
            // Esc / Ctrl-C / Cancel all end the step as a clean cancel so the
            // run ends cancelled rather than erroring, matching the fullscreen
            // surface. Back is not offered for the per-step confirm, so its
            // outcome maps to cancel too.
            ConfirmOutcome::Cancelled | ConfirmOutcome::BackToEdit => {
                Ok(StepConfirmOutcome::Cancel)
            }
        }
    }

    fn resolve_step_failure(
        &mut self,
        _step: &CompiledStep,
        error: &ags_protocol::error::RuntimeError,
        allow_skip: bool,
    ) -> Result<ags_protocol::workflow::StepFailureAction, CliError> {
        use crate::frontend::terminal::form_runner::{run_confirm_phase, ConfirmOutcome};
        use crate::frontend::terminal::inline::phases::confirm_card::{
            ConfirmAction, ConfirmCard, ConfirmCardPhase,
        };
        use crate::frontend::terminal::views::nav::NavContext;
        use ags_protocol::workflow::StepFailureAction;

        // The gate reuses the confirm card. Its title frames the primary action
        // as a retry, so the `Confirm` button reads as "yes, retry"; `Skip` is
        // offered only when the step is safely skippable; Esc/Cancel gives up.
        let (title, message) =
            crate::frontend::terminal::views::step_failure::step_failure_card_text(error);
        let card = ConfirmCard::new(title, vec![])
            .with_message(message)
            .caution();

        let (phase, nav_ctx) = if allow_skip {
            (
                ConfirmCardPhase::new(card).with_actions(&[
                    ConfirmAction::Retry,
                    ConfirmAction::Skip,
                    ConfirmAction::Cancel,
                ]),
                NavContext::StepFailureSkippable,
            )
        } else {
            (
                ConfirmCardPhase::new(card)
                    .with_actions(&[ConfirmAction::Retry, ConfirmAction::Cancel]),
                NavContext::StepFailure,
            )
        };

        let mut session = self.session.borrow_mut();
        let terminal = session.terminal_mut()?;
        match run_confirm_phase(terminal, phase, nav_ctx)? {
            // Retry is the card's primary action, surfaced as `Confirmed`.
            ConfirmOutcome::Confirmed => Ok(StepFailureAction::Retry),
            ConfirmOutcome::Skipped => Ok(StepFailureAction::Skip),
            ConfirmOutcome::Cancelled | ConfirmOutcome::BackToEdit => Ok(StepFailureAction::Cancel),
        }
    }

    fn present_briefing(
        &mut self,
        briefing: &ags_protocol::workflow::WorkflowBriefing,
        workflow_name: &str,
    ) -> Result<bool, CliError> {
        use crate::frontend::terminal::form_runner::{crossterm_next_key, is_ctrl_c};
        use crate::frontend::terminal::inline::chrome;
        use crate::frontend::terminal::views::nav::NavContext;
        use crossterm::event::{KeyCode, KeyEventKind};

        let mut session = self.session.borrow_mut();
        let terminal = session.terminal_mut()?;
        loop {
            terminal
                .draw(|f| {
                    chrome::render(f, NavContext::Briefing, |frame, main| {
                        render_briefing(frame, main, workflow_name, briefing);
                    });
                })
                .map_err(|e| CliError::Usage {
                    message: format!("TUI draw failed: {e}"),
                    metadata: None,
                })?;
            let key = crossterm_next_key()?;
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if is_ctrl_c(key) {
                return Ok(false);
            }
            match key.code {
                KeyCode::Enter => return Ok(true),
                KeyCode::Esc => return Ok(false),
                _ => {}
            }
        }
    }

    fn collect_workflow_inputs(
        &mut self,
        specs: &[ags_protocol::workflow::WorkflowInputSpec],
        current: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Option<ags_protocol::workflow::CollectOutcome>, CliError> {
        use crate::frontend::terminal::form_runner::collect_inputs_form;
        // Shared orchestration builds the form + projects inputs; the closure is
        // the inline drive. `dynamic_enums: true` — inline renders a picker for
        // options_source inputs (driven by `drive_enum_picker` over the wired
        // `OptionsFetch`). `file_pickers: false` — inline has no directory-browser
        // modal; the file-picker widget is Fullscreen-only, so a file_picker
        // input falls back to a plain editable text field here.
        Ok(
            collect_inputs_form(specs, current, true, false, |form| self.drive_inline(form))?.map(
                |(inputs, run_mode)| ags_protocol::workflow::CollectOutcome { inputs, run_mode },
            ),
        )
    }

    fn review_step(
        &mut self,
        plan: &ags_protocol::workflow::StepFieldPlan,
    ) -> Result<ags_protocol::workflow::StepReviewOutcome, CliError> {
        use crate::frontend::terminal::form_runner::review_step_form;
        if plan.optional {
            self.drive_inline_review_optional(plan)
        } else {
            review_step_form(plan, |form| self.drive_inline(form))
        }
    }
}

impl InlineInteraction {
    /// Drive a prepared form on the inline session terminal, returning the
    /// submitted [`Form`] (`Some`) or `None` on cancel. The single inline drive
    /// primitive behind the shared Phase-1 and per-step-review orchestration.
    fn drive_inline(
        &self,
        form: crate::frontend::terminal::inline::form::Form,
    ) -> Result<Option<crate::frontend::terminal::inline::form::Form>, CliError> {
        use crate::frontend::terminal::form_runner::{crossterm_next_key, drive_inline_form};
        use crate::frontend::terminal::inline::phases::form::FormPhase;

        let mut session = self.session.borrow_mut();
        let terminal = session.terminal_mut()?;
        drive_inline_form(
            terminal,
            FormPhase::new(form),
            self.options_fetch.as_deref(),
            &mut crossterm_next_key,
        )
    }

    /// Drive a review form for an optional step: same as [`drive_inline`] but
    /// renders with `NavContext::FieldsSkippable` and intercepts `s` (when no
    /// field is in edit mode) to return `StepReviewOutcome::Skip` immediately
    /// without the form needing to submit. Mirrors the fullscreen
    /// `review_step_optional_inner` pattern, adapted to the inline chrome.
    fn drive_inline_review_optional(
        &mut self,
        plan: &ags_protocol::workflow::StepFieldPlan,
    ) -> Result<ags_protocol::workflow::StepReviewOutcome, CliError> {
        use crate::frontend::terminal::form_runner::crossterm_next_key;
        let mut session = self.session.borrow_mut();
        let tty = session.terminal_mut()?;
        drive_inline_review_optional_inner(plan, tty, crossterm_next_key)
    }
}

/// Drive an optional step's inline review form, reading keys from `next_key`
/// (real terminal in production, scripted in tests). The key seam mirrors the
/// fullscreen `review_step_optional_inner` and the `form_runner` inline drivers,
/// so the skip-vs-submit and edit-mode-guard branches are unit-testable with a
/// `TestBackend` terminal and a scripted key stream.
///
/// Pressing `s` skips the step, but only when no field editor is open: while a
/// scalar field is in edit mode, `s` must reach the edit buffer so text inputs
/// stay typeable.
fn drive_inline_review_optional_inner<B, F>(
    plan: &ags_protocol::workflow::StepFieldPlan,
    tty: &mut ratatui::Terminal<B>,
    mut next_key: F,
) -> Result<ags_protocol::workflow::StepReviewOutcome, CliError>
where
    B: ratatui::backend::Backend,
    F: FnMut() -> Result<crossterm::event::KeyEvent, CliError>,
{
    use crate::frontend::terminal::form_runner::{
        confirm_skip_outcome, drive_json_editor, is_ctrl_c, is_optional_skip_key,
        STEP_REVIEW_SUBMIT_DESCRIPTION,
    };
    use crate::frontend::terminal::inline::form::{FieldValue, Form};
    use crate::frontend::terminal::inline::phases::form::{FormPhase, PhaseResult};
    use crate::frontend::terminal::inline::phases::{Phase, PhaseStep};
    use crate::frontend::terminal::views::{fields, nav::NavContext};
    use ags_protocol::workflow::StepReviewOutcome;
    use crossterm::event::KeyEventKind;

    let mut form = Form::from_step_plan(plan)
        .with_submit_description(STEP_REVIEW_SUBMIT_DESCRIPTION)
        .with_confirm_skip_buttons(true);
    form.focus_submit_if_available();
    let mut phase = FormPhase::new(form);

    loop {
        tty.draw(|f| {
            crate::frontend::terminal::inline::chrome::render(
                f,
                NavContext::FieldsSkippable,
                |frame, main| {
                    fields::render_inline(frame, main, phase.form(), phase.form().submit_focusable);
                },
            );
        })
        .map_err(|e| CliError::Usage {
            message: format!("TUI draw failed: {e}"),
            metadata: None,
        })?;

        let key = next_key()?;
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if is_ctrl_c(key) {
            return Ok(StepReviewOutcome::Cancel);
        }
        // `s` skips this optional step — the guard rule lives in the shared
        // `is_optional_skip_key` so it cannot drift from the fullscreen twin.
        if is_optional_skip_key(key, phase.form().is_editing()) {
            return Ok(StepReviewOutcome::Skip);
        }
        match phase.on_key(key) {
            PhaseStep::Continue => continue,
            PhaseStep::Cancelled => return Ok(StepReviewOutcome::Cancel),
            PhaseStep::Done(PhaseResult::Submitted(_)) => {
                return Ok(confirm_skip_outcome(&phase.into_form(), plan));
            }
            PhaseStep::Done(PhaseResult::OpenJsonEditor(idx)) => {
                let result = drive_json_editor(tty, phase.form_mut(), idx, &mut next_key, true)?;
                if let Some(value) = result {
                    let pretty =
                        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                    phase.form_mut().fields[idx].value = FieldValue::JsonBody(pretty);
                }
            }
            // OpenEnumPicker does not occur in review forms (StepField has
            // no options_source), so this arm is unreachable in practice.
            // Map to Continue so the loop keeps running rather than
            // panicking if the form ever emits it.
            PhaseStep::Done(PhaseResult::OpenEnumPicker(_)) => continue,
            // OpenFilePicker does not occur in review forms (StepField has no
            // file_picker), so this arm is unreachable in practice. Map to
            // Continue so the loop keeps running rather than panicking if the
            // form ever emits it.
            PhaseStep::Done(PhaseResult::OpenFilePicker(_)) => continue,
        }
    }
}

/// Render the workflow briefing into the inline main area: the workflow name as
/// the box title, the overview prose, then a This-run-creates bullet list.
/// Compressed for the short inline viewport — the Prerequisites section that the
/// fullscreen briefing shows is omitted here. Inline markdown emphasis
/// (`**bold**`, `` `code` ``) is rendered with bold/dim styling, matching the
/// fullscreen briefing, via the shared `views::inline_format` parser.
fn render_briefing(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    workflow_name: &str,
    briefing: &ags_protocol::workflow::WorkflowBriefing,
) {
    use crate::frontend::terminal::views::inline_format::styled_spans;
    use ratatui::layout::{Constraint, Direction, Layout};
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};

    let header = |text: &str| {
        Line::from(Span::styled(
            text.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
    };

    let mut lines: Vec<Line> = Vec::new();
    for para in briefing.overview.split('\n') {
        lines.push(Line::from(styled_spans(para)));
    }
    if !briefing.creates.is_empty() {
        lines.push(Line::raw(""));
        lines.push(header("This run creates"));
        for item in &briefing.creates {
            let mut spans = vec![Span::raw("  \u{2022} ")];
            spans.extend(styled_spans(item));
            lines.push(Line::from(spans));
        }
    }

    // Draw the bordered box, then split its inner area so a `Continue` button
    // sits pinned at the bottom — matching the step forms, which always show a
    // `[ Confirm & Continue → ]` button. The button is the only action here, so
    // it renders in the focused (highlighted) style.
    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::new(2, 2, 1, 1))
        .title(format!(" {workflow_name} "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), chunks[0]);
    // Indent the button 2 cells so its `[` lines up with the text, mirroring the
    // step form's submit button.
    let btn = ratatui::layout::Rect::new(
        chunks[2].x + 2,
        chunks[2].y,
        chunks[2].width.saturating_sub(2),
        chunks[2].height,
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "[ Continue \u{2192} ]",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))),
        btn,
    );
}

/// Sort full-surface fields by request location (Path → Query → Header → Body),
/// then label, matching the ordering `run_gather` applies to needed/supplied.
fn sort_full_surface_fields(
    mut fields: Vec<crate::frontend::terminal::inline::form::FormField>,
    full_inputs: &[ags_protocol::workflow::WorkflowInputSpec],
) -> Vec<crate::frontend::terminal::inline::form::FormField> {
    use ags_protocol::workflow::StepFieldLocation;
    use ags_runtime::support::strings::to_kebab_case;
    use std::collections::HashMap;

    // Field labels are kebab; specs carry the raw name — key the map by kebab.
    let loc_by_label: HashMap<String, StepFieldLocation> = full_inputs
        .iter()
        .map(|s| (to_kebab_case(&s.name), s.location))
        .collect();
    crate::frontend::terminal::form_runner::sort_form_fields_by_location(
        &mut fields,
        &loc_by_label,
    );
    fields
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::InlineInteraction;
    use crate::frontend::terminal::inline::session::InlineSession;
    use crate::frontend::ExecutionInteraction;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Build an inline session over a fake terminal for tests.
    fn make_session() -> Rc<RefCell<InlineSession>> {
        Rc::new(RefCell::new(InlineSession::without_terminal()))
    }

    #[test]
    fn test_inline_interaction_constructs_from_session() {
        let session = make_session();
        let _interaction = InlineInteraction::new(Rc::clone(&session), None);
        // Both the original Rc and the one inside InlineInteraction point to the
        // same allocation: strong_count == 2.
        assert_eq!(Rc::strong_count(&session), 2);
    }

    /// `gather_workflow_inputs` with an empty `needed` slice returns `Ok` with
    /// an empty map and never touches the terminal.
    #[test]
    fn test_inline_interaction_gather_empty_needed_returns_empty_map() {
        let session = make_session();
        let mut interaction = InlineInteraction::new(Rc::clone(&session), None);
        let step = {
            use ags_protocol::catalogue::{OperationId, ServiceId};
            use ags_protocol::workflow::{CompiledStep, OperationReference};
            CompiledStep {
                id: "test-step".to_string(),
                index: 0,
                description: None,
                kind: ags_protocol::workflow::StepKind::default(),
                action: None,
                operation: Some(OperationReference {
                    service: ServiceId::new("iam"),
                    operation: OperationId::new("testOp"),
                }),
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

    #[test]
    fn test_build_full_surface_fields_optional_projects_as_override() {
        use crate::frontend::terminal::inline::form::Form;
        use crate::frontend::terminal::inline::form_builder::build_full_surface_fields;
        use ags_protocol::workflow::{StepFieldLocation, WorkflowInputSpec};

        let full_inputs = vec![WorkflowInputSpec {
            name: "redirectUri".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: false,
            default: None,
            sensitive: false,
            options_source: None,
            location: StepFieldLocation::Body,
            file_picker: None,
        }];
        let mut fields = build_full_surface_fields(&full_inputs, &[]);
        // Simulate the user filling the optional field.
        fields[0].value =
            crate::frontend::terminal::inline::form::FieldValue::Scalar("https://x".into());
        let result = Form::new("t", fields).project_gathered();
        assert_eq!(
            result.input_overrides.get("redirectUri"),
            Some(&serde_json::json!("https://x")),
            "filled optional projects as an input_override keyed by the raw input name"
        );
        assert!(
            result.slot_values.is_empty(),
            "no slot routing on the full surface"
        );
    }

    /// `collect_workflow_inputs` with no declared specs passes the current map
    /// through untouched without acquiring the terminal (mirrors the fullscreen
    /// guard and keeps the no-terminal test green).
    #[test]
    fn test_inline_collect_workflow_inputs_empty_specs_passthrough() {
        let session = make_session();
        let mut interaction = InlineInteraction::new(Rc::clone(&session), None);
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

    #[test]
    fn test_render_briefing_includes_overview_and_creates_but_omits_prerequisites() {
        use ags_protocol::workflow::WorkflowBriefing;
        use ratatui::style::Modifier;
        use ratatui::{backend::TestBackend, Terminal};
        let briefing = WorkflowBriefing {
            overview: "Sets up **matchmaking** for your `game`.".into(),
            prerequisites: vec!["An uploaded image".into()],
            creates: vec!["A dedicated server fleet".into()],
        };
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| {
            super::render_briefing(f, f.area(), "Set up competitive multiplayer", &briefing)
        })
        .unwrap();
        let cells: Vec<(String, Modifier)> = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| (c.symbol().to_string(), c.modifier))
            .collect();
        let s: String = cells.iter().map(|(sym, _)| sym.as_str()).collect();
        assert!(s.contains("Set up competitive multiplayer"), "title: {s}");
        assert!(
            s.contains("Sets up matchmaking for your game"),
            "overview with emphasis rendered: {s}"
        );
        assert!(
            !s.contains("**") && !s.contains('`'),
            "emphasis markers are parsed into styling, not shown literally"
        );
        // The markup is rendered as styling, not stripped: the **matchmaking**
        // run must carry the BOLD modifier (a regression to plain stripping
        // would leave these cells unstyled).
        let target: Vec<String> = "matchmaking".chars().map(|c| c.to_string()).collect();
        let start = (0..cells.len())
            .find(|&i| {
                target
                    .iter()
                    .enumerate()
                    .all(|(k, ch)| cells.get(i + k).map(|(s, _)| s) == Some(ch))
            })
            .expect("matchmaking present in buffer");
        assert!(
            (0..target.len()).all(|k| cells[start + k].1.contains(Modifier::BOLD)),
            "**matchmaking** is rendered bold"
        );
        assert!(s.contains("This run creates") && s.contains("A dedicated server fleet"));
        // Compressed for inline: the Prerequisites section is omitted.
        assert!(
            !s.contains("Prerequisites") && !s.contains("An uploaded image"),
            "prerequisites omitted in the compressed inline briefing: {s}"
        );
        // A Continue button is shown, matching the step forms' submit button.
        assert!(
            s.contains("Continue"),
            "briefing shows a Continue button: {s}"
        );
    }

    /// Build an 80x24 headless terminal for driving the inline review loop.
    fn make_review_tty() -> ratatui::Terminal<ratatui::backend::TestBackend> {
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).expect("TestBackend")
    }

    /// A `Press` key event with no modifiers.
    fn key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    /// An optional plan with a single editable scalar field seeded to `x`.
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

    /// Pressing `s` at the review gate (no field editor open) skips the step.
    /// Inline twin of the fullscreen `test_confirm_step_s_on_optional_returns_skip`.
    #[test]
    fn test_drive_inline_review_optional_s_returns_skip() {
        use ags_protocol::workflow::StepReviewOutcome;
        use crossterm::event::KeyCode;

        let plan = optional_scalar_plan();
        let mut tty = make_review_tty();
        let mut keys = vec![Ok(key(KeyCode::Char('s')))].into_iter();
        let outcome = super::drive_inline_review_optional_inner(&plan, &mut tty, move || {
            keys.next().unwrap()
        })
        .unwrap();
        assert_eq!(outcome, StepReviewOutcome::Skip);
    }

    /// Regression guard: pressing `s` while a scalar field IS in edit mode must
    /// NOT skip — the char must reach the edit buffer. Begins editing the field,
    /// types `s` (appending to the seeded `x`), commits, then submits with
    /// Confirm focused. Outcome is `Proceed` (never `Skip`) with the typed `s`.
    /// Inline twin of the fullscreen `test_review_step_s_while_editing_does_not_skip`.
    #[test]
    fn test_drive_inline_review_optional_s_while_editing_does_not_skip() {
        use ags_protocol::workflow::{StepFieldId, StepReviewOutcome};
        use crossterm::event::KeyCode;

        let plan = optional_scalar_plan();
        let mut tty = make_review_tty();
        // Consumed in order:
        //  Up    → focus the scalar field (off the Submit slot)
        //  Enter → begin edit
        //  's'   → in edit mode → appended to the buffer (NOT a skip)
        //  Enter → commit ("x" → "xs")
        //  Tab   → focus the Submit slot
        //  Enter → submit (Confirm focused) → Proceed
        let mut keys = vec![
            Ok(key(KeyCode::Up)),
            Ok(key(KeyCode::Enter)),
            Ok(key(KeyCode::Char('s'))),
            Ok(key(KeyCode::Enter)),
            Ok(key(KeyCode::Tab)),
            Ok(key(KeyCode::Enter)),
        ]
        .into_iter();
        let outcome = super::drive_inline_review_optional_inner(&plan, &mut tty, move || {
            keys.next().unwrap()
        })
        .unwrap();
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

    /// Submitting with the Skip button focused (Right selects Skip in the
    /// Confirm/Skip group, then Enter submits) returns `Skip` — the submit path,
    /// distinct from the bare-`s` shortcut above.
    #[test]
    fn test_drive_inline_review_optional_submit_with_skip_focused_returns_skip() {
        use ags_protocol::workflow::StepReviewOutcome;
        use crossterm::event::KeyCode;

        let plan = optional_scalar_plan();
        let mut tty = make_review_tty();
        // Submit is focused initially. Right → focus Skip; Enter → submit Skip.
        let mut keys = vec![Ok(key(KeyCode::Right)), Ok(key(KeyCode::Enter))].into_iter();
        let outcome = super::drive_inline_review_optional_inner(&plan, &mut tty, move || {
            keys.next().unwrap()
        })
        .unwrap();
        assert_eq!(outcome, StepReviewOutcome::Skip);
    }
}
