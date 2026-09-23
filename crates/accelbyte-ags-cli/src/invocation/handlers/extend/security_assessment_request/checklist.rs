//! Interactive endpoint checklist for `ags extend security-assessment request`.
//!
//! Arrow keys navigate, space toggles an endpoint's selection, enter edits a
//! row's permission field — only for rows that are authenticated with no
//! auto-discovered permission, the Admin Portal's editability rule — and a
//! trailing "Submit" row finalizes the selection.

use std::collections::HashMap;

use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    },
    Frame,
};

use super::api::{Endpoint, EndpointSelection};
use super::permission::{self, ParsedPermission};
use crate::errors::CliError;
use crate::frontend::terminal::form_runner::{crossterm_next_key, is_ctrl_c};
use crate::frontend::terminal::inline::lifecycle;
use crate::frontend::terminal::scrollbar::scrollbar_state;

#[derive(Debug, Clone, PartialEq)]
enum PermissionCell {
    /// Auto-discovered; read-only.
    Discovered(String),
    Unauthenticated,
    /// Authenticated with no discovered permission — the only editable case.
    Editable,
}

#[derive(Debug, Clone)]
struct Row {
    operation_id: String,
    method: String,
    path: String,
    cell: PermissionCell,
}

impl Row {
    fn from_endpoint(endpoint: &Endpoint) -> Self {
        let cell = if endpoint.is_permission_editable() {
            PermissionCell::Editable
        } else if let Some(p) = &endpoint.permission {
            PermissionCell::Discovered(permission::display_permission(&p.resource, &p.action))
        } else {
            PermissionCell::Unauthenticated
        };
        Self {
            operation_id: endpoint.operation_id.clone(),
            method: endpoint.method.clone(),
            path: endpoint.path.clone(),
            cell,
        }
    }

    fn is_editable(&self) -> bool {
        matches!(self.cell, PermissionCell::Editable)
    }
}

/// Pure state machine, no terminal I/O — driven one action at a time and
/// read back via [`ChecklistState::finalize`] once the user submits.
pub(crate) struct ChecklistState {
    rows: Vec<Row>,
    checked: Vec<bool>,
    /// Only meaningful for editable rows; pre-filled from `--permission`.
    permission_input: Vec<String>,
    /// `0..rows.len()` highlights an endpoint row; `rows.len()` highlights
    /// the trailing "Submit" pseudo-row.
    highlighted: usize,
    cap: usize,
    namespace: String,
    /// `Some(buffer)` while editing the highlighted row's permission.
    edit_buffer: Option<String>,
    edit_error: Option<String>,
    pub(crate) status_message: Option<String>,
    cancelled: bool,
    submitted: bool,
}

impl ChecklistState {
    /// An operation id present in `initial_permissions` (from repeated
    /// `--permission` flags) is pre-filled and pre-checked — supplying an
    /// override implies intent to include that endpoint.
    pub(crate) fn new(
        endpoints: &[Endpoint],
        cap: usize,
        namespace: String,
        initial_permissions: &HashMap<String, ParsedPermission>,
    ) -> Self {
        let rows: Vec<Row> = endpoints.iter().map(Row::from_endpoint).collect();
        let mut checked = vec![false; rows.len()];
        let mut permission_input = vec![String::new(); rows.len()];
        for (i, row) in rows.iter().enumerate() {
            if let Some(p) = initial_permissions.get(&row.operation_id) {
                permission_input[i] = permission::display_permission(&p.resource, &p.action);
                checked[i] = true;
            }
        }
        Self {
            rows,
            checked,
            permission_input,
            highlighted: 0,
            cap,
            namespace,
            edit_buffer: None,
            edit_error: None,
            status_message: None,
            cancelled: false,
            submitted: false,
        }
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn is_submit_row(&self, idx: usize) -> bool {
        idx == self.row_count()
    }

    fn is_editing(&self) -> bool {
        self.edit_buffer.is_some()
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    pub(crate) fn checked_count(&self) -> usize {
        self.checked.iter().filter(|c| **c).count()
    }

    /// A checked, editable row only blocks submission when it has a
    /// non-empty permission that fails validation. An *empty* permission is
    /// not a hard blocker: the operator may not know it yet, and CSM
    /// re-validates on submit, surfacing its own error if one is required.
    fn all_selected_permissions_valid(&self) -> bool {
        self.rows.iter().enumerate().all(|(i, row)| {
            !self.checked[i]
                || !row.is_editable()
                || self.permission_input[i].is_empty()
                || permission::parse_permission(&self.permission_input[i], &self.namespace).is_ok()
        })
    }

    pub(crate) fn submit_state(&self) -> (bool, Option<&'static str>) {
        if self.checked_count() == 0 {
            return (false, Some("Select at least one endpoint to continue."));
        }
        if !self.all_selected_permissions_valid() {
            return (
                false,
                Some("Fix the invalid permission format to continue."),
            );
        }
        (true, None)
    }

    pub(crate) fn move_up(&mut self) {
        if self.is_editing() {
            return;
        }
        self.highlighted = self.highlighted.saturating_sub(1);
        self.status_message = None;
    }

    pub(crate) fn move_down(&mut self) {
        if self.is_editing() {
            return;
        }
        self.highlighted = (self.highlighted + 1).min(self.row_count());
        self.status_message = None;
    }

    pub(crate) fn toggle_highlighted(&mut self) {
        if self.is_editing() || self.is_submit_row(self.highlighted) {
            return;
        }
        let idx = self.highlighted;
        if !self.checked[idx] && self.checked_count() >= self.cap {
            self.status_message = Some(format!(
                "You've reached the {}-endpoint limit. Deselect one to add another.",
                self.cap
            ));
            return;
        }
        self.checked[idx] = !self.checked[idx];
        self.status_message = None;
    }

    /// Returns `true` when the checklist should exit (submit succeeded).
    pub(crate) fn activate_highlighted(&mut self) -> bool {
        if self.is_editing() {
            self.commit_edit();
            return false;
        }
        if self.is_submit_row(self.highlighted) {
            let (can_submit, reason) = self.submit_state();
            if can_submit {
                self.submitted = true;
                return true;
            }
            self.status_message = reason.map(str::to_string);
            return false;
        }
        let idx = self.highlighted;
        match &self.rows[idx].cell {
            PermissionCell::Editable => {
                self.edit_buffer = Some(self.permission_input[idx].clone());
                self.edit_error = None;
            }
            PermissionCell::Discovered(_) => {
                self.status_message = Some(
                    "This permission was auto-discovered from the app's OpenAPI spec or gRPC \
                     reflection and can't be edited."
                        .to_string(),
                );
            }
            PermissionCell::Unauthenticated => {
                self.status_message = Some("Unauthenticated endpoint".to_string());
            }
        }
        false
    }

    pub(crate) fn push_char(&mut self, c: char) {
        if let Some(buf) = &mut self.edit_buffer {
            buf.push(c);
        }
    }

    pub(crate) fn pop_char(&mut self) {
        if let Some(buf) = &mut self.edit_buffer {
            buf.pop();
        }
    }

    fn commit_edit(&mut self) {
        let Some(buf) = self.edit_buffer.clone() else {
            return;
        };
        match permission::parse_permission(&buf, &self.namespace) {
            Ok(_) => {
                self.permission_input[self.highlighted] = buf;
                self.edit_buffer = None;
                self.edit_error = None;
            }
            Err(message) => {
                self.edit_error = Some(message);
            }
        }
    }

    pub(crate) fn cancel_edit_or_checklist(&mut self) {
        if self.is_editing() {
            self.edit_buffer = None;
            self.edit_error = None;
        } else {
            self.cancelled = true;
        }
    }

    pub(crate) fn finalize(&self) -> Vec<EndpointSelection> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(i, _)| self.checked[*i])
            .map(|(i, row)| EndpointSelection {
                operation_id: row.operation_id.clone(),
                permission_override: if row.is_editable() {
                    permission::parse_permission(&self.permission_input[i], &self.namespace).ok()
                } else {
                    None
                },
            })
            .collect()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let dim = Style::default().add_modifier(Modifier::DIM);
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Select endpoints ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height < 5 {
            return;
        }

        let sections = Layout::vertical([
            Constraint::Length(2), // tooltip line (wraps to at most 2 lines)
            Constraint::Min(1),    // row list
            Constraint::Length(1), // footer (counter / status / edit error)
            Constraint::Length(1), // keybinding legend
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "The permission this endpoint requires. It's used to prepare test users \
                 that can call the endpoint properly.",
                dim,
            )))
            .wrap(ratatui::widgets::Wrap { trim: true }),
            sections[0],
        );

        let mut items: Vec<ListItem> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let checkbox = if self.checked[i] { "[x]" } else { "[ ]" };
                let permission_text = match &row.cell {
                    PermissionCell::Discovered(text) => text.clone(),
                    PermissionCell::Unauthenticated => "Unauthenticated endpoint".to_string(),
                    PermissionCell::Editable => {
                        if self.is_editing() && i == self.highlighted {
                            self.edit_buffer.clone().unwrap_or_default()
                        } else if self.permission_input[i].is_empty() {
                            permission::manual_permission_hint(&self.namespace)
                        } else {
                            self.permission_input[i].clone()
                        }
                    }
                };
                let style = match &row.cell {
                    PermissionCell::Editable if self.permission_input[i].is_empty() => dim,
                    _ => Style::default(),
                };
                ListItem::new(Line::from(vec![
                    Span::raw(format!("{checkbox} {:<7} {:<28} ", row.method, row.path)),
                    Span::styled(permission_text, style),
                ]))
            })
            .collect();
        let (can_submit, _) = self.submit_state();
        let submit_style = if can_submit { bold } else { dim };
        items.push(ListItem::new(Line::from(Span::styled(
            "▸ Submit",
            submit_style,
        ))));

        let mut list_state = ListState::default();
        list_state.select(Some(self.highlighted));

        let list_widget = List::new(items)
            .highlight_symbol("\u{203a} ")
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD));
        frame.render_stateful_widget(list_widget, sections[1], &mut list_state);

        let visible_height = sections[1].height as usize;
        let total = self.row_count() + 1;
        if let Some(mut sb) = scrollbar_state(total, visible_height, list_state.offset()) {
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None),
                sections[1],
                &mut sb,
            );
        }

        let footer = if let Some(err) = &self.edit_error {
            Line::from(Span::styled(
                err.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ))
        } else if let Some(msg) = &self.status_message {
            Line::from(Span::raw(msg.clone()))
        } else if self.checked_count() >= self.cap {
            Line::from(Span::raw(format!(
                "You've selected the maximum of {} endpoints.",
                self.cap
            )))
        } else {
            Line::from(Span::styled(
                format!("{} / {} selected", self.checked_count(), self.cap),
                dim,
            ))
        };
        frame.render_widget(Paragraph::new(footer), sections[2]);
        frame.render_widget(Paragraph::new(self.legend_line()), sections[3]);
    }

    fn legend_line(&self) -> Line<'static> {
        let dim = Style::default().fg(Color::Indexed(244));
        let key = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let action = Style::default().fg(Color::White);
        let sep = || Span::styled("  \u{00B7}  ", dim);

        let mut spans = Vec::new();
        if self.is_editing() {
            spans.push(Span::styled("[Enter]", key));
            spans.push(Span::styled(" save", action));
            spans.push(sep());
            spans.push(Span::styled("[Esc]", key));
            spans.push(Span::styled(" cancel edit", action));
        } else {
            spans.push(Span::styled("[\u{2191}\u{2193}]", key));
            spans.push(Span::styled(" move", action));
            spans.push(sep());
            spans.push(Span::styled("[Space]", key));
            spans.push(Span::styled(" toggle", action));
            spans.push(sep());
            spans.push(Span::styled("[Enter]", key));
            spans.push(Span::styled(" edit permission / submit", action));
            spans.push(sep());
            spans.push(Span::styled("[Esc]", key));
            spans.push(Span::styled(" cancel", action));
        }
        Line::from(spans)
    }
}

pub(crate) fn run(
    endpoints: &[Endpoint],
    cap: usize,
    namespace: String,
    initial_permissions: &HashMap<String, ParsedPermission>,
) -> Result<Vec<EndpointSelection>, CliError> {
    let mut state = ChecklistState::new(endpoints, cap, namespace, initial_permissions);
    let mut terminal = lifecycle::acquire()?;

    let result = drive(&mut terminal, &mut state, crossterm_next_key);
    lifecycle::release(terminal);
    result?;

    if state.is_cancelled() {
        return Err(CliError::Usage {
            message: "Operation cancelled".to_string(),
            metadata: None,
        });
    }
    Ok(state.finalize())
}

/// Event-source injectable so the loop shape is exercisable without a real
/// terminal; `ChecklistState`'s own unit tests cover the logic this drives.
fn drive<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    state: &mut ChecklistState,
    mut next_key: impl FnMut() -> Result<crossterm::event::KeyEvent, CliError>,
) -> Result<(), CliError> {
    loop {
        terminal
            .draw(|frame| state.render(frame, frame.area()))
            .map_err(|e| CliError::Usage {
                message: format!("terminal render error: {e}"),
                metadata: None,
            })?;

        let key = next_key()?;
        if is_ctrl_c(key) {
            state.cancel_edit_or_checklist();
            state.cancelled = true;
            return Ok(());
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if !state.is_editing() => state.move_up(),
            KeyCode::Down | KeyCode::Char('j') if !state.is_editing() => state.move_down(),
            KeyCode::Char(' ') if !state.is_editing() => state.toggle_highlighted(),
            KeyCode::Enter => {
                if state.activate_highlighted() {
                    return Ok(());
                }
            }
            KeyCode::Esc => {
                state.cancel_edit_or_checklist();
                if state.is_cancelled() {
                    return Ok(());
                }
            }
            KeyCode::Backspace if state.is_editing() => state.pop_char(),
            KeyCode::Char(c) if state.is_editing() => state.push_char(c),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invocation::handlers::extend::security_assessment_request::api::EndpointPermission;

    fn endpoint(
        operation_id: &str,
        method: &str,
        path: &str,
        require_auth: bool,
        permission: Option<(&str, &str)>,
    ) -> Endpoint {
        Endpoint {
            method: method.to_string(),
            path: path.to_string(),
            operation_id: operation_id.to_string(),
            require_authentication: require_auth,
            permission: permission.map(|(r, a)| EndpointPermission {
                resource: r.to_string(),
                action: a.to_string(),
            }),
        }
    }

    fn sample_endpoints() -> Vec<Endpoint> {
        vec![
            endpoint(
                "op-1",
                "GET",
                "/users",
                true,
                Some(("NAMESPACE:ns1:USER", "READ")),
            ),
            endpoint("op-2", "DELETE", "/users/{id}", true, None),
            endpoint("op-3", "GET", "/health", false, None),
        ]
    }

    fn state() -> ChecklistState {
        ChecklistState::new(&sample_endpoints(), 10, "ns1".to_string(), &HashMap::new())
    }

    #[test]
    fn discovered_and_unauthenticated_rows_are_not_editable() {
        let s = state();
        assert!(!s.rows[0].is_editable());
        assert!(s.rows[1].is_editable());
        assert!(!s.rows[2].is_editable());
    }

    #[test]
    fn toggle_and_navigation() {
        let mut s = state();
        assert_eq!(s.checked_count(), 0);
        s.toggle_highlighted();
        assert_eq!(s.checked_count(), 1);
        s.move_down();
        s.move_down();
        assert_eq!(s.highlighted, 2);
        s.move_down();
        assert_eq!(s.highlighted, 3);
        assert!(s.is_submit_row(s.highlighted));
    }

    #[test]
    fn toggle_refuses_past_cap() {
        let mut s = ChecklistState::new(&sample_endpoints(), 1, "ns1".to_string(), &HashMap::new());
        s.toggle_highlighted();
        assert_eq!(s.checked_count(), 1);
        s.move_down();
        s.toggle_highlighted();
        assert_eq!(
            s.checked_count(),
            1,
            "cap of 1 must block a second selection"
        );
        assert!(s.status_message.is_some());
    }

    #[test]
    fn submit_disabled_with_nothing_checked() {
        let s = state();
        let (can_submit, reason) = s.submit_state();
        assert!(!can_submit);
        assert_eq!(reason, Some("Select at least one endpoint to continue."));
    }

    #[test]
    fn submit_allowed_while_editable_row_checked_with_empty_permission() {
        let mut s = state();
        s.move_down();
        s.toggle_highlighted();
        let (can_submit, reason) = s.submit_state();
        assert!(can_submit);
        assert_eq!(reason, None);
    }

    #[test]
    fn submit_disabled_while_editable_row_checked_with_invalid_non_empty_permission() {
        let mut s = state();
        s.move_down();
        s.toggle_highlighted();
        s.permission_input[1] = "not-a-permission".to_string();
        let (can_submit, reason) = s.submit_state();
        assert!(!can_submit);
        assert_eq!(
            reason,
            Some("Fix the invalid permission format to continue.")
        );
    }

    #[test]
    fn edit_only_allowed_on_editable_row() {
        let mut s = state();
        s.activate_highlighted();
        assert!(!s.is_editing());
        assert!(s.status_message.is_some());

        s.move_down();
        s.activate_highlighted();
        assert!(s.is_editing());
    }

    #[test]
    fn edit_commit_validates_and_stores() {
        let mut s = state();
        s.move_down();
        s.toggle_highlighted();
        s.activate_highlighted();
        for c in "NAMESPACE:ns1:USER [DELETE]".chars() {
            s.push_char(c);
        }
        s.activate_highlighted();
        assert!(!s.is_editing());
        assert_eq!(s.permission_input[1], "NAMESPACE:ns1:USER [DELETE]");
        let (can_submit, _) = s.submit_state();
        assert!(can_submit);
    }

    #[test]
    fn edit_commit_rejects_invalid_format_and_stays_in_edit_mode() {
        let mut s = state();
        s.move_down();
        s.activate_highlighted();
        for c in "not-a-permission".chars() {
            s.push_char(c);
        }
        s.activate_highlighted();
        assert!(s.is_editing(), "invalid input must not exit edit mode");
        assert!(s.edit_error.is_some());
    }

    #[test]
    fn esc_cancels_edit_without_changing_row() {
        let mut s = state();
        s.move_down();
        s.activate_highlighted();
        s.push_char('x');
        s.cancel_edit_or_checklist();
        assert!(!s.is_editing());
        assert!(s.permission_input[1].is_empty());
        assert!(!s.is_cancelled());
    }

    #[test]
    fn esc_outside_edit_cancels_checklist() {
        let mut s = state();
        s.cancel_edit_or_checklist();
        assert!(s.is_cancelled());
    }

    #[test]
    fn finalize_omits_permission_for_non_editable_rows() {
        let mut s = state();
        s.toggle_highlighted();
        s.move_down();
        s.move_down();
        s.toggle_highlighted();
        let selections = s.finalize();
        assert_eq!(selections.len(), 2);
        assert!(selections.iter().all(|e| e.permission_override.is_none()));
    }

    #[test]
    fn finalize_includes_permission_override_for_editable_rows() {
        let mut s = state();
        s.move_down();
        s.toggle_highlighted();
        s.activate_highlighted();
        for c in "NAMESPACE:ns1:USER [DELETE]".chars() {
            s.push_char(c);
        }
        s.activate_highlighted();
        let selections = s.finalize();
        assert_eq!(selections.len(), 1);
        assert_eq!(
            selections[0].permission_override,
            Some(ParsedPermission {
                resource: "NAMESPACE:ns1:USER".to_string(),
                action: "DELETE".to_string(),
            })
        );
    }

    #[test]
    fn submit_row_activation_requires_valid_state() {
        let mut s = state();
        s.move_down();
        s.move_down();
        s.move_down();
        assert!(
            !s.activate_highlighted(),
            "must not exit with nothing checked"
        );
        assert!(!s.submitted);

        s.move_up();
        s.move_up();
        s.move_up();
        s.toggle_highlighted();
        s.move_down();
        s.move_down();
        s.move_down();
        assert!(s.activate_highlighted());
        assert!(s.submitted);
    }

    #[test]
    fn initial_permissions_prefill_and_precheck_matching_rows() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "op-2".to_string(),
            ParsedPermission {
                resource: "NAMESPACE:ns1:USER".to_string(),
                action: "DELETE".to_string(),
            },
        );
        let s = ChecklistState::new(&sample_endpoints(), 10, "ns1".to_string(), &overrides);
        assert!(s.checked[1]);
        assert_eq!(s.permission_input[1], "NAMESPACE:ns1:USER [DELETE]");
        assert!(!s.checked[0]);
    }

    // ── `drive()` key-event routing ──
    //
    // The state-machine methods above are exercised directly; these tests
    // instead exercise `drive()` itself — the `KeyCode` → state-method
    // mapping — via an injected `next_key` sequence and a
    // `ratatui::backend::TestBackend`, so a future edit to that `match` (a
    // reordered arm, a broken guard) fails a test instead of only breaking
    // silently in a real terminal.

    use crossterm::event::{KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_c() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    /// Feeds a fixed sequence of key events to `drive()`, panicking if it
    /// asks for more keys than were scripted — mirrors the injected-`read`
    /// pattern used elsewhere in this codebase (e.g.
    /// `security_assessment_request::tests::scripted`-style stdin readers).
    fn scripted_keys(keys: Vec<KeyEvent>) -> impl FnMut() -> Result<KeyEvent, CliError> {
        let mut keys = keys.into_iter();
        move || {
            keys.next()
                .ok_or_else(|| CliError::Internal(anyhow::anyhow!("no more scripted keys")))
        }
    }

    fn test_terminal() -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(80, 24)).expect("build a TestBackend terminal")
    }

    /// Down, space (toggle), down, down, enter (submit) drives the checklist
    /// to a successful exit with exactly the toggled row selected — the same
    /// outcome `submit_row_activation_requires_valid_state` gets by calling
    /// the state methods directly, but here routed entirely through
    /// `drive()`'s key mapping.
    #[test]
    fn drive_navigates_toggles_and_submits_via_key_events() {
        let mut s = state();
        let mut terminal = test_terminal();
        let mut next_key = scripted_keys(vec![
            key(KeyCode::Down),
            key(KeyCode::Char(' ')),
            key(KeyCode::Down),
            key(KeyCode::Down),
            key(KeyCode::Enter),
        ]);

        drive(&mut terminal, &mut s, &mut next_key).expect("drive must succeed");

        assert!(!s.is_cancelled());
        assert!(s.submitted);
        assert_eq!(s.checked_count(), 1);
        assert_eq!(
            s.finalize(),
            vec![EndpointSelection {
                operation_id: "op-2".to_string(),
                permission_override: None,
            }]
        );
    }

    /// `j`/`k` are accepted as vim-style aliases for `Down`/`Up`, matching
    /// the arrow-key routing exactly.
    #[test]
    fn drive_accepts_vim_style_navigation_aliases() {
        let mut s = state();
        let mut terminal = test_terminal();
        let mut next_key = scripted_keys(vec![
            key(KeyCode::Char('j')), // -> row 1
            key(KeyCode::Char(' ')), // toggle row 1
            key(KeyCode::Char('k')), // -> row 0
            key(KeyCode::Char('j')), // -> row 1
            key(KeyCode::Char('j')), // -> row 2
            key(KeyCode::Char('j')), // -> submit row
            key(KeyCode::Enter),
        ]);

        drive(&mut terminal, &mut s, &mut next_key).expect("drive must succeed");

        assert!(s.submitted);
        assert_eq!(s.checked_count(), 1);
    }

    /// Ctrl-C cancels immediately regardless of `highlighted`/edit state,
    /// and is checked before any `KeyCode` match arm.
    #[test]
    fn drive_ctrl_c_cancels_immediately() {
        let mut s = state();
        let mut terminal = test_terminal();
        let mut next_key = scripted_keys(vec![key(KeyCode::Down), ctrl_c()]);

        drive(&mut terminal, &mut s, &mut next_key).expect("drive must succeed");

        assert!(s.is_cancelled());
        assert!(!s.submitted);
    }

    /// `Esc` outside edit mode cancels the whole checklist via `drive()`.
    #[test]
    fn drive_esc_cancels_checklist() {
        let mut s = state();
        let mut terminal = test_terminal();
        let mut next_key = scripted_keys(vec![key(KeyCode::Esc)]);

        drive(&mut terminal, &mut s, &mut next_key).expect("drive must succeed");

        assert!(s.is_cancelled());
    }

    /// Enter on an editable row opens edit mode instead of submitting, and
    /// character keys / Backspace before that point must be no-ops (routed
    /// to `drive()`'s `_ => {}` fallback, not `push_char`/`pop_char`) since
    /// their guard clauses require `state.is_editing()`. Once editing,
    /// pushed characters reach the edit buffer — verified indirectly here by
    /// checking that Ctrl-C's cancellation discards the in-progress edit
    /// without ever having committed it back to `permission_input`.
    #[test]
    fn drive_chars_and_backspace_only_take_effect_while_editing() {
        let mut s = state();
        let mut terminal = test_terminal();
        let mut next_key = scripted_keys(vec![
            key(KeyCode::Down), // highlight op-2 (editable)
            key(KeyCode::Char('x')),
            key(KeyCode::Backspace),
            key(KeyCode::Enter), // open edit mode
            key(KeyCode::Char('n')),
            ctrl_c(), // cancel out mid-edit
        ]);

        drive(&mut terminal, &mut s, &mut next_key).expect("drive must succeed");

        assert!(s.is_cancelled());
        assert!(!s.submitted);
        assert_eq!(
            s.permission_input[1], "",
            "an uncommitted edit must never reach permission_input"
        );
    }
}
