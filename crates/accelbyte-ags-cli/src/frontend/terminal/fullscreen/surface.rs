//! `FullscreenSurface` — owns the alt-screen terminal **and** the render
//! model, exposing a single [`render`](FullscreenSurface::render) that draws
//! the whole frame from its own state.
//!
//! It supersedes `FullscreenSession` (terminal-only): the frontend and the
//! interaction will both hold one `Rc<RefCell<FullscreenSurface>>`, so every
//! phase — progress, gather, confirm, result — draws through the identical
//! layout path. Borrow discipline is unchanged from the session: rendering
//! and interaction borrow the surface at strictly disjoint times.

use std::cell::Cell;
use std::rc::Rc;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::errors::CliError;
use crate::frontend::terminal::fullscreen::lifecycle::{acquire, release, Tty};
use crate::frontend::RenderOptions;

use super::layout;
use super::nav;
use super::phases::enum_picker::EnumPickerModal;
use super::phases::{self, Phase};
use super::step_strip::{self, HeaderKind, Step};
use super::summary::{self, SummaryEntry};

/// Captured first-failure context the dismiss teardown plays back to stderr as
/// a terse one-line pointer. The reason and detail are not stored here — the
/// full error block is rendered separately after the alt screen closes.
#[derive(Debug, Clone)]
pub struct FailureSummary {
    pub step_index: usize,
    pub step_title: String,
}

/// Owns the live alt-screen terminal and the full render model for the
/// fullscreen workflow surface.
pub struct FullscreenSurface {
    terminal: Option<Tty>,
    pub options: RenderOptions,
    pub stdout_is_tty: bool,
    pub title: String,
    /// Whether the frame title reads `ags workflow:` or `ags command:`. Set
    /// after construction by `select_fullscreen_workflow_surfaces`.
    pub header_kind: HeaderKind,
    pub status: String,
    pub steps: Vec<Step>,
    pub summary_entries: Vec<SummaryEntry>,
    pub cleanup_required: bool,
    pub current_phase: Phase,
    pub summary_scroll: usize,
    pub result_scroll: usize,
    pub briefing_scroll: usize,
    pub last_failure: Option<FailureSummary>,
    pub workflow_description: Option<String>,
    /// When `Some`, the Fields phase renders this spinner/status message in
    /// the nav area while a dynamic-enum fetch is in flight (Task 15).
    pub(crate) options_loading: Option<String>,
    /// When `Some`, an open dynamic-enum picker drawn as a centered overlay.
    pub(crate) enum_picker: Option<EnumPickerModal>,
    /// Terminal size used by `terminal_area()` when no live terminal is attached
    /// (headless tests) or `Terminal::size()` errors. `Rc<Cell<_>>` so tests can
    /// mutate the reported size mid-loop via `area_override_handle()`.
    area_override: Rc<Cell<Rect>>,
}

impl FullscreenSurface {
    /// Acquire the terminal (raw mode + alternate screen) and seed default
    /// render state. Errors if the terminal cannot be acquired.
    pub fn new(
        options: RenderOptions,
        title: impl Into<String>,
        steps: Vec<Step>,
        stdout_is_tty: bool,
    ) -> Result<Self, CliError> {
        let terminal = acquire()?;
        Ok(Self::with_state(
            Some(terminal),
            options,
            title.into(),
            steps,
            stdout_is_tty,
        ))
    }

    /// Construct a surface from an explicit terminal and initial render state.
    /// Shared by the production constructor and tests (which pass a fake `Tty`).
    fn with_state(
        terminal: Option<Tty>,
        options: RenderOptions,
        title: String,
        steps: Vec<Step>,
        stdout_is_tty: bool,
    ) -> Self {
        Self {
            terminal,
            options,
            stdout_is_tty,
            title,
            header_kind: HeaderKind::Workflow,
            status: String::new(),
            steps,
            summary_entries: Vec::new(),
            cleanup_required: false,
            current_phase: Phase::Running(phases::running::RunningPanel {
                step_number: 0,
                step_title: String::new(),
                description: String::new(),
                verb: "Starting".into(),
            }),
            summary_scroll: 0,
            result_scroll: 0,
            briefing_scroll: 0,
            last_failure: None,
            workflow_description: None,
            options_loading: None,
            enum_picker: None,
            area_override: Rc::new(Cell::new(Rect::new(0, 0, 80, 24))),
        }
    }

    /// Tear down the terminal exactly once. Idempotent.
    pub fn teardown(&mut self) {
        if let Some(terminal) = self.terminal.take() {
            release(terminal);
        }
    }

    /// Whether the alt-screen terminal is still attached. Goes `false` once
    /// [`teardown`](Self::teardown) has run — e.g. after `render_error`
    /// restored the terminal to print a plain error — so callers can skip
    /// surface-only work (like the dismiss loop) that would otherwise block on
    /// an already-released terminal.
    pub fn is_active(&self) -> bool {
        self.terminal.is_some()
    }

    /// Draw the whole surface once from current state — the frame as
    /// discrete bordered panels: header (brand + step strip) + main (current
    /// phase) + Summary + Navigation bar.
    pub fn render(&mut self) -> Result<(), CliError> {
        // No terminal attached → headless path (test-only, via
        // `without_terminal()`). Skip drawing rather than fail with a
        // "terminal already released" error: callers like
        // `run_briefing_dismiss_loop` legitimately invoke render on every
        // iteration and need to work in headless tests too.
        let Some(mut terminal) = self.terminal.take() else {
            return Ok(());
        };
        let result = terminal.draw(|frame| self.render_frame(frame)).map(|_| ());
        self.terminal = Some(terminal);
        result.map_err(|e| CliError::Usage {
            message: format!("TUI draw failed: {e}"),
            metadata: None,
        })
    }

    /// Draw one full frame: the four §13 regions (header, main phase panel,
    /// summary, nav), each rendering its own border.
    fn render_frame(&mut self, frame: &mut Frame) {
        let area = frame.area();
        // Discrete bordered panels — no single outer frame. Each renderer draws
        // its own border (header, Summary, Navigation, and the phase panels).
        let show_strip = !self.steps.is_empty();
        let regions =
            layout::split_with_strip(area, show_strip, self.workflow_description.is_some());
        step_strip::render(
            frame,
            regions.header,
            &self.title,
            self.header_kind,
            &self.status,
            &self.steps,
            self.workflow_description.as_deref(),
        );
        // While a dynamic-enum fetch is in flight (only possible in the Fields
        // phase) the nav shows only `Esc cancel` and a spinner overlays the
        // field area. The panel itself renders normally — it holds a clone of
        // the real form, so the fields and hint stay on screen under the spinner.
        let loading = self.options_loading.is_some();
        let (phase_scroll, scroll_target) = match &self.current_phase {
            Phase::Briefing(_) => (self.briefing_scroll as u16, ScrollTarget::Briefing),
            _ => (self.result_scroll as u16, ScrollTarget::Result),
        };
        let max = self
            .current_phase
            .render_scrolled(frame, regions.main, phase_scroll);
        // Clamp the stored scroll to what the rendered content actually
        // permits — otherwise a held-down ↓ keeps incrementing past the end
        // and pressing ↑ later has to drain that phantom excess before the
        // viewport visibly moves.
        match scroll_target {
            ScrollTarget::Briefing => {
                if self.briefing_scroll > max as usize {
                    self.briefing_scroll = max as usize;
                }
            }
            ScrollTarget::Result => {
                if self.result_scroll > max as usize {
                    self.result_scroll = max as usize;
                }
            }
        }
        let summary_max = summary::render(
            frame,
            regions.summary,
            &self.summary_entries,
            self.cleanup_required,
            self.summary_scroll as u16,
        );
        if self.summary_scroll > summary_max as usize {
            self.summary_scroll = summary_max as usize;
        }
        if loading {
            nav::render_loading(frame, regions.nav);
        } else {
            nav::render(frame, regions.nav, &self.current_phase);
        }
        // When a dynamic-enum fetch is in flight (Fields phase), show the spinner
        // as a small centered box *in the field area*. The nav already shows only
        // `Esc cancel`, so the user knows the fetch is cancellable.
        if matches!(self.current_phase, Phase::Fields(_)) {
            if let Some(msg) = &self.options_loading {
                let width = (msg.chars().count() as u16 + 4).min(regions.main.width);
                let height = 3.min(regions.main.height);
                let rect = Rect {
                    x: regions.main.x + regions.main.width.saturating_sub(width) / 2,
                    y: regions.main.y + regions.main.height.saturating_sub(height) / 2,
                    width,
                    height,
                };
                let block = Block::default().borders(Borders::ALL);
                let body = block.inner(rect);
                frame.render_widget(Clear, rect);
                frame.render_widget(block, rect);
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        msg.clone(),
                        Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC),
                    )))
                    .alignment(ratatui::layout::Alignment::Center),
                    body,
                );
            }
        }
        // A dynamic-enum picker, when open, draws on top of everything as a
        // centered modal. `&mut` so its scroll offset persists across frames.
        if let Some(modal) = &mut self.enum_picker {
            modal.render(frame, area);
        }
    }

    /// Set the state of the leading Inputs row, if present. Used by Phase 1.
    pub fn set_inputs_row_state(&mut self, state: super::step_strip::StepState) {
        if let Some(row) = self
            .steps
            .iter_mut()
            .find(|s| s.kind == super::step_strip::StepRowKind::Inputs)
        {
            row.state = state;
        }
    }

    /// Set (or clear) the dynamic-enum loading spinner message displayed in the
    /// nav area while a fetch is in flight. `None` clears the overlay.
    pub(crate) fn set_options_loading(&mut self, message: Option<String>) {
        self.options_loading = message;
    }

    /// Current terminal `Rect`. Infallible: live `Terminal::size()` when a
    /// terminal is attached (falling back to `area_override` on an io error),
    /// else the stored `area_override`. Never panics — a size query error must
    /// not crash the picker.
    pub(crate) fn terminal_area(&self) -> Rect {
        if let Some(terminal) = &self.terminal {
            if let Ok(size) = terminal.size() {
                return Rect::new(0, 0, size.width, size.height);
            }
        }
        self.area_override.get()
    }

    /// A clone of the `area_override` handle so tests can change the reported
    /// size mid-loop without borrowing the surface.
    #[cfg(test)]
    pub(crate) fn area_override_handle(&self) -> Rc<Cell<Rect>> {
        Rc::clone(&self.area_override)
    }

    /// Test-only constructor: builds a surface with `terminal: None` and
    /// default render state, so wiring code can be unit-tested in CI where
    /// there is no TTY.
    #[cfg(test)]
    pub fn without_terminal() -> Self {
        Self::with_state(
            None,
            RenderOptions::default(),
            String::new(),
            Vec::new(),
            false,
        )
    }

    /// Test-only: like `without_terminal`, but with a chosen reported size for
    /// `terminal_area()` (drives modal paging and the too-small fallback).
    #[cfg(test)]
    pub fn without_terminal_sized(width: u16, height: u16) -> Self {
        let surface = Self::without_terminal();
        surface.area_override.set(Rect::new(0, 0, width, height));
        surface
    }
}

impl Drop for FullscreenSurface {
    fn drop(&mut self) {
        self.teardown();
    }
}

enum ScrollTarget {
    Briefing,
    Result,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::terminal::fullscreen::phases::confirm::ConfirmPanel;
    use crate::frontend::terminal::fullscreen::phases::fields::FieldsPanel;
    use crate::frontend::terminal::fullscreen::phases::result::{ResultPanel, ResultStatus};
    use crate::frontend::terminal::fullscreen::step_strip::{Step, StepRowKind, StepState};
    use crate::frontend::terminal::inline::form::{
        FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
    };
    use crate::frontend::terminal::inline::phases::confirm_card::{ConfirmCard, ConfirmCardPhase};
    use ags_protocol::workflow::GatherSlotId;
    use ratatui::{backend::TestBackend, Terminal};

    fn surface_with(phase: Phase) -> FullscreenSurface {
        let mut s = FullscreenSurface::without_terminal();
        s.title = "competitive-multiplayer".into();
        s.status = "running".into();
        s.steps = vec![Step {
            kind: StepRowKind::Workflow { runtime_index: 0 },
            title: "Define".into(),
            state: StepState::Current,
        }];
        s.current_phase = phase;
        s
    }

    fn render_to_string(surface: &mut FullscreenSurface, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| surface.render_frame(f)).unwrap();
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn one_field_form() -> Form {
        Form::new(
            "Provide inputs",
            vec![FormField {
                label: "user-id".into(),
                field_type: FieldType::Scalar,
                required: true,
                value: FieldValue::Empty,
                description: "The player".into(),
                source: FieldSource::UserInput,
                key: FieldKey::Slot(GatherSlotId(0)),
                schema: serde_json::json!({"type": "string"}),
                read_only: false,
                dynamic: None,
            }],
        )
        .with_submit_focusable(true)
    }

    #[test]
    fn test_without_terminal_seeds_default_running_phase() {
        let surface = FullscreenSurface::without_terminal();
        assert!(matches!(surface.current_phase, Phase::Running(_)));
        assert!(surface.summary_entries.is_empty());
        assert!(!surface.cleanup_required);
    }

    #[test]
    fn test_terminal_area_uses_override_when_headless() {
        let surface = FullscreenSurface::without_terminal_sized(50, 20);
        assert_eq!(
            surface.terminal_area(),
            ratatui::layout::Rect::new(0, 0, 50, 20)
        );
    }

    #[test]
    fn test_enum_picker_overlay_renders_over_fields() {
        use crate::frontend::terminal::fullscreen::phases::enum_picker::EnumPickerModal;
        use ags_protocol::workflow::OptionChoice;
        let mut surface = surface_with(Phase::Fields(FieldsPanel {
            step_number: 1,
            step_name: "Define".into(),
            description: String::new(),
            form: one_field_form(),
            optional: false,
        }));
        surface.enum_picker = Some(EnumPickerModal::new(
            "Pick image".into(),
            vec![OptionChoice {
                label: "prod".into(),
                value: "p".into(),
            }],
            false,
            None,
        ));
        let buf = render_to_string(&mut surface, 80, 24);
        assert!(
            buf.contains("Pick image"),
            "modal title drawn over the frame: {buf}"
        );
        assert!(buf.contains("Filter:"), "modal filter line drawn: {buf}");
    }

    #[test]
    fn test_fields_phase_keeps_step_strip_summary_and_nav() {
        let mut surface = surface_with(Phase::Fields(FieldsPanel {
            step_number: 1,
            step_name: "Define".into(),
            description: String::new(),
            form: one_field_form(),
            optional: false,
        }));
        let buf = render_to_string(&mut surface, 80, 24);
        assert!(buf.contains("Define"), "step strip title persists");
        assert!(buf.contains("Summary"), "Summary panel persists");
        assert!(buf.contains("Navigation"), "Navigation box persists");
        assert!(buf.contains("Parameters"), "Parameters box title present");
    }

    #[test]
    fn test_confirm_phase_keeps_step_strip_summary_and_nav() {
        let mut surface = surface_with(Phase::Confirm(ConfirmPanel {
            header: crate::frontend::terminal::fullscreen::header::Header::step(1, "Define", ""),
            card_phase: ConfirmCardPhase::new(ConfirmCard::new("POST /x", vec![])),
        }));
        let buf = render_to_string(&mut surface, 80, 24);
        assert!(buf.contains("Define"));
        assert!(buf.contains("Summary"));
        assert!(buf.contains("confirm"), "Confirm nav token present");
    }

    #[test]
    fn test_result_phase_keeps_step_strip_summary_and_nav() {
        let mut surface = surface_with(Phase::Result(ResultPanel {
            title: "complete".into(),
            description: String::new(),
            body: "{}".into(),
            completion: None,
            status: ResultStatus::Success,
        }));
        let buf = render_to_string(&mut surface, 80, 24);
        assert!(buf.contains("Define"));
        assert!(buf.contains("Summary"));
        assert!(buf.contains("exit"), "Result nav token present");
    }

    #[test]
    fn test_set_options_loading_sets_message_and_clears() {
        let mut s = FullscreenSurface::without_terminal();
        s.set_options_loading(Some("Loading choices… ⠙".to_string()));
        assert_eq!(s.options_loading.as_deref(), Some("Loading choices… ⠙"));
        s.set_options_loading(None);
        assert!(s.options_loading.is_none());
    }

    #[test]
    fn test_options_loading_shows_spinner_and_esc_only_nav() {
        let mut surface = surface_with(Phase::Fields(FieldsPanel {
            step_number: 1,
            step_name: "Define".into(),
            description: String::new(),
            form: one_field_form(),
            optional: false,
        }));
        // Without loading, the full keymap renders in the nav.
        let normal = render_to_string(&mut surface, 80, 24);
        assert!(normal.contains("[Tab]"), "full keymap shows normally");

        surface.set_options_loading(Some("Loading choices… ⠙".to_string()));
        let buf = render_to_string(&mut surface, 80, 24);
        assert!(
            buf.contains("Loading choices"),
            "loading message shown in the field area: {buf}"
        );
        // Only Esc is valid while loading: the nav shows Esc and nothing else.
        assert!(buf.contains("[Esc]"), "nav shows [Esc] cancel: {buf}");
        assert!(
            !buf.contains("[Tab]"),
            "no other keymap entries while loading: {buf}"
        );
    }

    /// Render a phase into a fresh 80x24 backend and return the row of the
    /// content box's top border — the lowest `┌` in column 0. Every per-step
    /// phase draws a fixed-height header slot (top box) above its content box,
    /// so the content box border is the second, lower `┌`.
    fn content_box_top_row(draw: impl FnOnce(&mut ratatui::Frame, ratatui::layout::Rect)) -> u16 {
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| {
            let area = f.area();
            draw(f, area);
        })
        .unwrap();
        let buf = term.backend().buffer();
        (0..buf.area.height)
            .rfind(|&y| buf[(0, y)].symbol() == "\u{250C}")
            .expect("at least one boxed border in column 0")
    }

    #[test]
    fn test_content_box_top_is_stable_across_confirm_running_result() {
        use crate::frontend::terminal::fullscreen::header::Header;
        use crate::frontend::terminal::fullscreen::phases::running::RunningPanel;
        use crate::frontend::terminal::inline::phases::confirm_card::ConfirmCard;

        // The same step, advancing Confirm → Running → Result, must place the
        // content box on the same row every time — the frame must not reflow.
        let confirm_top = {
            let panel = ConfirmPanel {
                header: Header::step(2, "Publish the store", "Publishes the catalog live"),
                card_phase: ConfirmCardPhase::new(
                    ConfirmCard::new("Step 2: publish", vec![])
                        .with_message("Publishes the catalog live"),
                ),
            };
            content_box_top_row(|f, a| panel.render(f, a))
        };
        let running_top = {
            let panel = RunningPanel {
                step_number: 2,
                step_title: "Publish the store".into(),
                description: "Publishes the catalog live".into(),
                verb: "Sending request".into(),
            };
            content_box_top_row(|f, a| panel.render(f, a))
        };
        let result_top = {
            let panel = ResultPanel {
                title: "store complete".into(),
                description: String::new(),
                body: "{}".into(),
                completion: None,
                status: ResultStatus::Success,
            };
            content_box_top_row(|f, a| {
                panel.render(f, a, 0);
            })
        };

        assert!(
            confirm_top > 0,
            "confirm renders a header slot above its content box (top={confirm_top})"
        );
        assert_eq!(
            confirm_top, running_top,
            "confirm and running content box align (confirm={confirm_top}, running={running_top})"
        );
        assert_eq!(
            running_top, result_top,
            "running and result content box align (running={running_top}, result={result_top})"
        );
    }

    #[test]
    fn test_briefing_scroll_is_clamped_to_panel_max() {
        use crate::frontend::terminal::fullscreen::phases::briefing::BriefingPanel;
        use ags_protocol::workflow::WorkflowBriefing;
        let mut s = surface_with(Phase::Briefing(BriefingPanel::new(
            &WorkflowBriefing {
                overview: "short".into(),
                prerequisites: vec![],
                creates: vec![],
            },
            "WF",
        )));
        s.briefing_scroll = 9_999;
        let _ = render_to_string(&mut s, 60, 10);
        assert!(
            s.briefing_scroll < 9_999,
            "briefing_scroll should be clamped after render"
        );
    }
}
