//! Three-action confirmation card.
//!
//! Sits below the [`Form`](super::super::form::Form) once required fields
//! are filled. Shows the assembled request summary and three focusable
//! buttons: Confirm, Back to edit, Cancel. The y/n
//! [`ConfirmPhase`](super::confirm::ConfirmPhase) stays in place for the
//! workflow gather flow until that path is migrated; this phase is the
//! service-command equivalent.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::{Phase, PhaseStep};

/// One of the confirmation buttons. Which subset is offered is configured
/// per moment via [`ConfirmCardPhase::with_actions`]: the gather confirm-card
/// offers `Confirm / Back / Cancel`; the per-step workflow confirm offers only
/// `Confirm / Cancel` (its `Result<bool>` contract has no `Back` channel);
/// optional-step confirms offer `Confirm / Skip / Cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    Confirm,
    Back,
    Cancel,
    Skip,
    /// Re-run the step. Used by the interactive failure gate as its primary
    /// action, in place of `Confirm` (there is nothing to "confirm" after a
    /// failure — the step already ran).
    Retry,
}

impl ConfirmAction {
    /// The button label text for this action (without outer brackets — the
    /// group renderer wraps each label in `[ … ]`). Used via `button_label`
    /// for the confirm-card button group.
    fn label_text(self) -> &'static str {
        match self {
            ConfirmAction::Confirm => "Confirm",
            ConfirmAction::Back => "Back to edit",
            ConfirmAction::Cancel => "Cancel",
            ConfirmAction::Skip => "Skip",
            ConfirmAction::Retry => "Retry",
        }
    }
}

/// Confirmation card model: a frame `title`, and EITHER a key-value `summary`
/// of the assembled request (the service-command review) OR a one-line
/// `message` (the workflow per-step confirm — the step's user-facing action).
/// The title and message never carry the request method or URL: those are
/// backend detail. The focused button and offered action set live on
/// [`ConfirmCardPhase`].
pub struct ConfirmCard {
    pub title: String,
    pub summary: Vec<(String, String)>,
    pub message: Option<String>,
    /// When true the title renders in the warning colour with a leading `!`,
    /// flagging a destructive confirm. Matches the plain surface's yellow `!`
    /// step header. Set for the workflow per-step confirm; left false for the
    /// request-review and gather cards.
    pub caution: bool,
}

impl ConfirmCard {
    /// Build a card from a frame title and a key-value request summary.
    pub fn new(title: impl Into<String>, summary: Vec<(String, String)>) -> Self {
        Self {
            title: title.into(),
            summary,
            message: None,
            caution: false,
        }
    }

    /// Set a one-line message shown in the main frame (the step's user-facing
    /// action and risk), with the Confirm button just below it. Used by the
    /// workflow per-step confirm in place of the request summary rows.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Flag this as a destructive confirm: the title renders in the warning
    /// colour with a leading `!`, so the risk reads the same across surfaces.
    pub fn caution(mut self) -> Self {
        self.caution = true;
        self
    }
}

/// Phase that hosts a [`ConfirmCard`]. Output is the [`ConfirmAction`] the
/// user selected — re-opening the form on `Back` is the outer phase loop's
/// job (it owns the form state to seed back). `focus` indexes the PRIMARY
/// actions (`Confirm` and `Skip`) for arrow-key navigation; `Back` and
/// `Cancel` stay keyboard-shortcut-only.
pub struct ConfirmCardPhase {
    card: ConfirmCard,
    actions: Vec<ConfirmAction>,
    /// Index into `primary_actions()` for arrow-key navigation. Default 0 →
    /// Confirm. Clamped to `primary_actions().len() - 1` on Right.
    focus: usize,
}

impl ConfirmCardPhase {
    /// Wrap a card in its confirmation phase.
    pub fn new(card: ConfirmCard) -> Self {
        Self {
            card,
            // Default set is all three: Confirm (Enter), Back (b), Cancel (Esc).
            actions: vec![
                ConfirmAction::Confirm,
                ConfirmAction::Back,
                ConfirmAction::Cancel,
            ],
            focus: 0,
        }
    }

    /// Builder: restrict the offered action set. The per-step confirm uses
    /// `&[Confirm, Cancel]` (no "back to edit"); the gather card keeps the
    /// default three.
    pub fn with_actions(mut self, actions: &[ConfirmAction]) -> Self {
        self.actions = actions.to_vec();
        self
    }

    /// The offered action set.
    // Consumed by fullscreen nav + tests.
    #[allow(dead_code)]
    pub fn actions(&self) -> &[ConfirmAction] {
        &self.actions
    }

    /// Whether "back to edit" is offered (only the gather flow can go back).
    /// Drives both the `b` key handler and the nav-bar hint.
    pub(crate) fn offers_back(&self) -> bool {
        self.actions.contains(&ConfirmAction::Back)
    }

    /// Whether "skip" is offered (only optional-step confirms offer it).
    /// Drives the `s` key handler and the nav-bar hint; when false, `s` is
    /// inert so a non-optional confirm card cannot accidentally emit Skip.
    pub(crate) fn offers_skip(&self) -> bool {
        self.actions.contains(&ConfirmAction::Skip)
    }

    /// The primary actions shown as arrow-navigable buttons: `Confirm`/`Retry`
    /// and, when offered, `Skip`. `Back` and `Cancel` remain keyboard-shortcut-
    /// only and are excluded from this group.
    fn primary_actions(&self) -> Vec<ConfirmAction> {
        self.actions
            .iter()
            .filter(|&&a| {
                matches!(
                    a,
                    ConfirmAction::Confirm | ConfirmAction::Skip | ConfirmAction::Retry
                )
            })
            .copied()
            .collect()
    }
}

impl ConfirmCardPhase {
    /// Area-aware render: draws the card within `area`. The [`Phase::render`]
    /// impl delegates to this with `frame.area()` (inline full-screen);
    /// the fullscreen `ConfirmPanel` calls it with the layout's main rect.
    pub fn render_in(&self, frame: &mut Frame<'_>, area: Rect) {
        use super::super::form::{
            button_group_spans, FieldKey, FieldSource, FieldType, FieldValue, Form, FormField,
        };
        use ratatui::widgets::Padding;

        // One box wraps the whole review — read-only fields AND the Confirm
        // button — exactly like the Parameters box wraps the fields and the
        // submit button.
        // A caution card (destructive per-step confirm) renders its title in the
        // warning colour with a leading `!`, matching the plain surface. Other
        // cards keep the default title.
        let title = if self.card.caution {
            Line::from(Span::styled(
                format!(
                    " {} {} ",
                    crate::frontend::style::text::SYMBOL_WARNING,
                    self.card.title
                ),
                Style::default().fg(Color::Yellow),
            ))
        } else {
            Line::from(format!(" {} ", self.card.title))
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .padding(Padding::new(2, 2, 1, 1))
            .title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height == 0 || inner.width == 0 {
            return;
        }

        // Message variant (workflow per-step confirm): the step's user-facing
        // action and risk, with the button group on the line just below it —
        // not pinned to the bottom. No request method or URL.
        if let Some(message) = &self.card.message {
            let wrapped = wrap_message(message, inner.width as usize);
            let msg_height = (wrapped.len() as u16).min(inner.height);
            frame.render_widget(
                Paragraph::new(wrapped.join("\n")),
                Rect::new(inner.x, inner.y, inner.width, msg_height),
            );
            let btn_y = inner.y + msg_height + 1;
            if btn_y < inner.y + inner.height {
                let primary = self.primary_actions();
                let labels: Vec<&str> = primary.iter().map(|a| a.label_text()).collect();
                let spans = button_group_spans(&labels, self.focus, true);
                frame.render_widget(
                    Paragraph::new(Line::from(spans)),
                    Rect::new(inner.x, btn_y, inner.width, 1),
                );
            }
            return;
        }

        // Fields region (flex), a blank gap, then the single Confirm button —
        // mirroring `views::fields::render_inline`'s inner split.
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // read-only fields
                Constraint::Length(1), // gap
                Constraint::Length(1), // Confirm button
            ])
            .split(inner);

        // Reuse the parameters-form row renderer so the review stays identical to
        // the form. Read-only scalar fields render unframed (`  label: value`),
        // column-aligned, and long values truncate with an ellipsis.
        let fields: Vec<FormField> = self
            .card
            .summary
            .iter()
            .map(|(k, v)| FormField {
                label: k.clone(),
                field_type: FieldType::Scalar,
                required: false,
                value: FieldValue::Scalar(v.clone()),
                description: String::new(),
                source: FieldSource::Default,
                key: FieldKey::Input(k.clone()),
                schema: serde_json::json!({ "type": "string" }),
                read_only: true,
                dynamic: None,
                file_picker: None,
            })
            .collect();
        let mut form = Form::new("", fields);
        form.focus = usize::MAX; // read-only review: never highlight a row
        let label_width = form.label_width();
        let fields_area = chunks[0];
        for idx in 0..form.fields.len().min(fields_area.height as usize) {
            let row_rect = Rect::new(
                fields_area.x,
                fields_area.y + idx as u16,
                fields_area.width,
                1,
            );
            form.render_field_row(frame, row_rect, idx, label_width);
        }

        // Button group, indented 2 to align with field labels (which lead with
        // a 2-cell caret pad), as the submit button does.
        let btn = Rect::new(
            chunks[2].x + 2,
            chunks[2].y,
            chunks[2].width.saturating_sub(2),
            chunks[2].height,
        );
        let primary = self.primary_actions();
        let labels: Vec<&str> = primary.iter().map(|a| a.label_text()).collect();
        let spans = button_group_spans(&labels, self.focus, true);
        frame.render_widget(Paragraph::new(Line::from(spans)), btn);
    }
}

impl Phase for ConfirmCardPhase {
    type Output = ConfirmAction;

    fn render(&self, frame: &mut Frame<'_>) {
        self.render_in(frame, frame.area());
    }

    fn on_key(&mut self, key: KeyEvent) -> PhaseStep<Self::Output> {
        // Button group navigation: Left/Right move focus across the primary
        // actions (Confirm and, when offered, Skip). Enter activates the
        // focused button. Keyboard shortcuts `b` (Back) and `s` (Skip) remain
        // for discoverability; `s` is guarded by `offers_skip()` so non-optional
        // cards cannot accidentally emit Skip.
        let primary = self.primary_actions();
        match key.code {
            KeyCode::Left => {
                self.focus = self.focus.saturating_sub(1);
                PhaseStep::Continue
            }
            KeyCode::Right => {
                self.focus = (self.focus + 1).min(primary.len().saturating_sub(1));
                PhaseStep::Continue
            }
            KeyCode::Enter => {
                let action = primary
                    .get(self.focus)
                    .copied()
                    .unwrap_or(ConfirmAction::Confirm);
                PhaseStep::Done(action)
            }
            KeyCode::Char('b') if self.offers_back() => PhaseStep::Done(ConfirmAction::Back),
            KeyCode::Char('s') if self.offers_skip() => PhaseStep::Done(ConfirmAction::Skip),
            KeyCode::Esc => PhaseStep::Cancelled,
            _ => PhaseStep::Continue,
        }
    }
}

/// Greedy word-wrap `text` to `width` columns for the message body. A word
/// longer than `width` overflows its own line rather than being split. Always
/// returns at least one line.
fn wrap_message(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn card() -> ConfirmCard {
        ConfirmCard::new(
            "DELETE /iam/v3/users/u-1",
            vec![("namespace".into(), "test-ns".into())],
        )
    }

    #[test]
    fn test_actions_default_is_confirm_back_cancel() {
        let p = ConfirmCardPhase::new(ConfirmCard::new("POST /x", vec![]));
        assert_eq!(
            p.actions(),
            &[
                ConfirmAction::Confirm,
                ConfirmAction::Back,
                ConfirmAction::Cancel
            ]
        );
    }

    #[test]
    fn test_actions_confirm_cancel_only_omits_back() {
        let p = ConfirmCardPhase::new(ConfirmCard::new("POST /x", vec![]))
            .with_actions(&[ConfirmAction::Confirm, ConfirmAction::Cancel]);
        assert_eq!(
            p.actions(),
            &[ConfirmAction::Confirm, ConfirmAction::Cancel]
        );
    }

    /// The review reuses the parameters-form row format: `label:` with a colon,
    /// the value unframed, the method/path as the box title, and the buttons
    /// below.
    #[test]
    fn test_render_in_matches_parameters_form_row_format() {
        use ratatui::{backend::TestBackend, Terminal};
        let card = ConfirmCard::new("Review request", vec![("namespace".into(), "acme".into())]);
        let phase = ConfirmCardPhase::new(card);
        let mut term = Terminal::new(TestBackend::new(60, 12)).unwrap();
        term.draw(|f| phase.render_in(f, f.area())).unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(s.contains("namespace:"), "label rendered with a colon: {s}");
        assert!(s.contains("acme"), "value rendered: {s}");
        assert!(
            s.contains("Review request"),
            "method/path is the box title: {s}"
        );
        assert!(s.contains("Confirm"), "confirm button present: {s}");
    }

    /// The message variant shows the frame title, the message, and the Confirm
    /// button — and never the request method or URL.
    #[test]
    fn test_render_in_message_variant_hides_endpoint() {
        use ratatui::{backend::TestBackend, Terminal};
        let card = ConfirmCard::new("Step 5: publish", vec![])
            .with_message("Publishes the store's catalog live for the namespace.");
        let phase = ConfirmCardPhase::new(card)
            .with_actions(&[ConfirmAction::Confirm, ConfirmAction::Cancel]);
        let mut term = Terminal::new(TestBackend::new(60, 12)).unwrap();
        term.draw(|f| phase.render_in(f, f.area())).unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains("Step 5: publish"),
            "step title as frame title: {s}"
        );
        assert!(s.contains("Publishes the store"), "message rendered: {s}");
        assert!(s.contains("Confirm"), "confirm button present: {s}");
        assert!(
            !s.contains("PUT") && !s.to_lowercase().contains("http"),
            "no request method or URL: {s}"
        );
    }

    /// A caution card (destructive per-step confirm) renders its title in
    /// yellow with a leading `!`, matching the plain surface's warning header.
    #[test]
    fn test_caution_card_title_is_yellow_with_warning_glyph() {
        use ratatui::{backend::TestBackend, Terminal};
        let card = ConfirmCard::new("Step 1: create-user", vec![])
            .with_message("Creates the user")
            .caution();
        let phase = ConfirmCardPhase::new(card)
            .with_actions(&[ConfirmAction::Confirm, ConfirmAction::Cancel]);
        let mut term = Terminal::new(TestBackend::new(60, 12)).unwrap();
        term.draw(|f| phase.render_in(f, f.area())).unwrap();
        let buf = term.backend().buffer();
        // The `!` glyph appears in the title and is yellow.
        let bang = buf
            .content()
            .iter()
            .find(|c| c.symbol() == "!")
            .expect("caution title must render the ! glyph");
        assert_eq!(bang.fg, Color::Yellow, "the ! glyph is warning-coloured");
        // The title text is yellow too (sample a letter from the title).
        let title_cell = buf
            .content()
            .iter()
            .find(|c| c.symbol() == "S" && c.fg == Color::Yellow);
        assert!(
            title_cell.is_some(),
            "the caution title text renders in yellow"
        );
    }

    /// A plain (non-caution) card keeps its default title colour and shows no
    /// `!` glyph — the request-review and gather cards must be unaffected.
    #[test]
    fn test_non_caution_card_title_has_no_warning() {
        use ratatui::{backend::TestBackend, Terminal};
        let card = ConfirmCard::new("Review request", vec![("namespace".into(), "acme".into())]);
        let phase = ConfirmCardPhase::new(card);
        let mut term = Terminal::new(TestBackend::new(60, 12)).unwrap();
        term.draw(|f| phase.render_in(f, f.area())).unwrap();
        let buf = term.backend().buffer();
        assert!(
            buf.content().iter().all(|c| c.symbol() != "!"),
            "non-caution card shows no ! glyph"
        );
        assert!(
            buf.content().iter().all(|c| c.fg != Color::Yellow),
            "non-caution card has no yellow cells"
        );
    }

    #[test]
    fn test_enter_returns_confirm_action() {
        let mut phase = ConfirmCardPhase::new(card());
        match phase.on_key(key(KeyCode::Enter)) {
            PhaseStep::Done(ConfirmAction::Confirm) => (),
            _ => panic!("expected Done(Confirm)"),
        }
    }

    #[test]
    fn test_b_returns_back_when_offered() {
        let mut phase = ConfirmCardPhase::new(card()); // default set offers Back
        match phase.on_key(key(KeyCode::Char('b'))) {
            PhaseStep::Done(ConfirmAction::Back) => (),
            _ => panic!("expected Done(Back)"),
        }
    }

    #[test]
    fn test_b_is_noop_when_back_not_offered() {
        let mut phase = ConfirmCardPhase::new(card())
            .with_actions(&[ConfirmAction::Confirm, ConfirmAction::Cancel]);
        assert!(matches!(
            phase.on_key(key(KeyCode::Char('b'))),
            PhaseStep::Continue
        ));
    }

    #[test]
    fn test_esc_cancels_phase() {
        let mut phase = ConfirmCardPhase::new(card());
        assert!(matches!(
            phase.on_key(key(KeyCode::Esc)),
            PhaseStep::Cancelled
        ));
    }

    /// On an optional card (Confirm + Skip offered), Right moves focus to Skip
    /// and Enter returns Done(Skip); Left returns to Confirm and Enter returns
    /// Done(Confirm).
    #[test]
    fn test_optional_card_right_focuses_skip_enter_returns_skip() {
        let mut phase = ConfirmCardPhase::new(card()).with_actions(&[
            ConfirmAction::Confirm,
            ConfirmAction::Skip,
            ConfirmAction::Cancel,
        ]);
        // Initially focused on Confirm.
        assert!(
            matches!(
                phase.on_key(key(KeyCode::Enter)),
                PhaseStep::Done(ConfirmAction::Confirm)
            ),
            "expected Done(Confirm) at focus=0"
        );
        // Right → Skip.
        phase.on_key(key(KeyCode::Right));
        assert!(
            matches!(
                phase.on_key(key(KeyCode::Enter)),
                PhaseStep::Done(ConfirmAction::Skip)
            ),
            "expected Done(Skip) after Right"
        );
        // Left → back to Confirm.
        phase.on_key(key(KeyCode::Left));
        assert!(
            matches!(
                phase.on_key(key(KeyCode::Enter)),
                PhaseStep::Done(ConfirmAction::Confirm)
            ),
            "expected Done(Confirm) after Left"
        );
    }

    /// `s` is a keyboard shortcut for Skip on optional cards — even when focus
    /// is on Confirm.
    #[test]
    fn test_s_shortcut_returns_skip_on_optional_card() {
        let mut phase = ConfirmCardPhase::new(card()).with_actions(&[
            ConfirmAction::Confirm,
            ConfirmAction::Skip,
            ConfirmAction::Cancel,
        ]);
        assert!(
            matches!(
                phase.on_key(key(KeyCode::Char('s'))),
                PhaseStep::Done(ConfirmAction::Skip)
            ),
            "expected Done(Skip) from s shortcut"
        );
    }

    /// On a non-optional card (no Skip in actions), Left/Right are no-ops and
    /// Enter always returns Done(Confirm).
    #[test]
    fn test_non_optional_card_arrows_are_noop_enter_returns_confirm() {
        let mut phase = ConfirmCardPhase::new(card())
            .with_actions(&[ConfirmAction::Confirm, ConfirmAction::Cancel]);
        // Right saturates at the single primary action.
        assert!(matches!(
            phase.on_key(key(KeyCode::Right)),
            PhaseStep::Continue
        ));
        // Left saturates at 0.
        assert!(matches!(
            phase.on_key(key(KeyCode::Left)),
            PhaseStep::Continue
        ));
        // Enter still returns Confirm.
        assert!(
            matches!(
                phase.on_key(key(KeyCode::Enter)),
                PhaseStep::Done(ConfirmAction::Confirm)
            ),
            "expected Done(Confirm) on non-optional card"
        );
    }
}
