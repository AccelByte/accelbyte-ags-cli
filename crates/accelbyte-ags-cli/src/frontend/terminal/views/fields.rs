//! Surface-neutral `Parameters` box: section headers, field rows,
//! scroll-to-focus, hint box below the submit row.
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{
    Block, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::Frame;

use crate::frontend::terminal::inline::form::Form;

/// Render the inline `Parameters` box.
///
/// Fields scroll in the top region; the Submit button (when `show_submit`) and
/// hint box are pinned at the bottom so they are always visible even in a short
/// inline viewport. No section headers are shown (kept compact for inline).
pub(crate) fn render_inline(frame: &mut Frame, area: Rect, form: &Form, show_submit: bool) {
    let title = match &form.box_title {
        Some(t) => format!(" {t} "),
        None => " Parameters ".to_string(),
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

    // Split the inner area: fields on top (scrollable), then a 1-row gap, the
    // submit button, a 1-row gap, and the hint box pinned at the bottom. The gap
    // before the submit button keeps a blank line between the last field and it.
    let constraints: Vec<Constraint> = if show_submit {
        vec![
            Constraint::Min(1),    // fields region
            Constraint::Length(1), // gap (blank line above submit)
            Constraint::Length(1), // submit button
            Constraint::Length(1), // gap
            Constraint::Length(4), // hint box
        ]
    } else {
        vec![
            Constraint::Min(1),    // fields region
            Constraint::Length(4), // hint box
        ]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let (fields_area, submit_area, hint_area) = if show_submit {
        (chunks[0], Some(chunks[2]), chunks[4])
    } else {
        (chunks[0], None, chunks[1])
    };

    // ── Fields region (scroll-to-focus, one row per VISIBLE field) ──
    let visible_indices: Vec<usize> = (0..form.fields.len())
        .filter(|&i| form.is_row_visible(i))
        .collect();
    let len = visible_indices.len();
    let label_width = form.label_width();
    let visible = (fields_area.height as usize).max(1);

    // Stateful scroll: keep the focused field visible while moving the window as
    // little as possible — it only scrolls when focus leaves the current window
    // (so moving up from the bottom item does not scroll until you pass the top
    // of the window). When the submit row is focused, pin to the bottom.
    let focus_pos = visible_indices
        .iter()
        .position(|&i| i == form.focus)
        .unwrap_or(0);
    let max_start = len.saturating_sub(visible);
    let mut start = form.scroll_top.get().min(max_start);
    if form.is_submit_focused() {
        start = max_start;
    } else if focus_pos < start {
        start = focus_pos;
    } else if focus_pos >= start + visible {
        start = focus_pos + 1 - visible;
    }
    start = start.min(max_start);
    form.scroll_top.set(start);

    // Reserve the scrollbar column plus a one-space gap so a long value trails
    // off with an ellipsis before the scrollbar rather than running under it.
    // Without a scrollbar the box's own right padding already provides margin.
    let right_reserve = if len > visible { 2 } else { 0 };
    let end = (start + visible).min(len);
    for (row_offset, &field_idx) in visible_indices[start..end].iter().enumerate() {
        let row_rect = Rect::new(
            fields_area.x,
            fields_area.y + row_offset as u16,
            fields_area.width.saturating_sub(right_reserve),
            1,
        );
        form.render_field_row(frame, row_rect, field_idx, label_width);
    }

    if len > visible {
        // ratatui's scrollbar maps `position` over `[0, content_length-1]` and
        // extends the thumb by `viewport_content_length`. For the thumb to reach
        // the track bottom when scrolled fully down, `content_length` must be the
        // number of scroll POSITIONS (`max_start + 1`) — NOT the total item count.
        // With total-item-count the thumb stops around the middle. `position` is
        // the scroll offset `start` (0..=max_start) and the thumb size stays
        // proportional to `visible / len`.
        let mut sb = ScrollbarState::new(max_start + 1)
            .position(start)
            .viewport_content_length(visible);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            fields_area,
            &mut sb,
        );
    }

    // ── Submit (pinned) ──
    if let Some(area) = submit_area {
        // Indent the button 2 cells so its `[` aligns with the field labels,
        // which lead with a 2-cell focus caret / pad.
        let btn = Rect::new(
            area.x + 2,
            area.y,
            area.width.saturating_sub(2),
            area.height,
        );
        form.render_submit(frame, btn, form.is_submit_focused());
    }
    // The `[o] show optional (+N)` toggle affordance is shown in the nav bar
    // (driven by `form_runner::drive_form` via `views::nav::optional_toggle_suffix`).

    // ── Hint (pinned) ──
    render_hint_box(frame, hint_area, form.current_hint().as_ref());
}

/// Render the `Parameters` box.
///
/// `inputs_title` is the workflow-values section header ("Inputs" or
/// "Inputs (read-only)") — the caller decides so the view stays surface-neutral.
///
/// Used by the fullscreen surface only. Output is snapshot-guarded; do not
/// change layout or styling here.
pub(crate) fn render(
    frame: &mut Frame,
    area: Rect,
    inputs_title: &str,
    form: &Form,
    show_submit: bool,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::new(2, 2, 1, 1))
        .title(" Parameters ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Row plan: each field followed by a blank line (so fields breathe), with
    // the hint box right under the focused field. Fields are grouped under
    // section headers. The Submit row lives inline after the last field;
    // scroll-to-focus brings it into view when the form is taller than the panel.
    let hint = form.current_hint();
    let label_width = form.label_width();

    use crate::frontend::terminal::inline::form::partition_fields_by_source;
    let groups = partition_fields_by_source(&form.fields);

    #[derive(Clone)]
    enum Row {
        Header(String),
        Field(usize),
        Hint,
        Blank,
        Submit,
    }
    let mut rows: Vec<(Row, u16)> = Vec::new();
    let mut focus_row: usize = 0;
    let sections: [(&str, &Vec<usize>); 2] = [
        (inputs_title, &groups.workflow_values),
        ("Options", &groups.step_values),
    ];
    let mut first_section = true;
    for (title, indices) in sections {
        if indices.is_empty() {
            continue;
        }
        if !first_section {
            rows.push((Row::Blank, 1));
        }
        first_section = false;
        rows.push((Row::Header(title.to_owned()), 1));
        rows.push((Row::Blank, 1));
        for &i in indices {
            if i == form.focus {
                focus_row = rows.len();
            }
            rows.push((Row::Field(i), 1));
            rows.push((Row::Blank, 1));
        }
    }

    // Submit lives in the row stream after the last field. The previous
    // pin-at-bottom render is removed; scroll-to-focus brings it into view
    // when the form is taller than the panel. Hidden while loading: there's
    // nothing to confirm until the fetch resolves.
    if show_submit {
        if !rows.is_empty() {
            rows.push((Row::Blank, 1));
        }
        rows.push((Row::Submit, 1));
        if form.is_submit_focused() {
            focus_row = rows.len() - 1;
        }
        rows.push((Row::Blank, 1));
    }
    // Fixed hint slot BELOW the Submit row: shows the focused field's
    // description, or the form's `submit_description` when Submit is focused.
    // 4 rows: top + bottom border + 2 content lines for descriptions that
    // wrap (workflow-input descriptions are often 2 sentences).
    rows.push((Row::Hint, 4));

    let avail = inner.height;

    // Cumulative top offsets, so we can scroll just enough to keep the
    // focused field in view.
    let mut tops = vec![0u16; rows.len() + 1];
    for k in 0..rows.len() {
        tops[k + 1] = tops[k] + rows[k].1;
    }
    let total = tops[rows.len()];
    let start_row = if total <= avail || rows.is_empty() {
        0
    } else {
        let focus_block_h = rows[focus_row].1;
        let focus_bottom = tops[focus_row] + focus_block_h;
        let start_top = focus_bottom.saturating_sub(avail);
        (0..rows.len()).find(|&k| tops[k] >= start_top).unwrap_or(0)
    };

    // Render rows sequentially from the scroll start; stop before the button.
    let limit = inner.top() + avail;
    let mut y = inner.top();
    for (row, h) in &rows[start_row..] {
        if y + h > limit {
            break;
        }
        let rect = Rect::new(inner.x, y, inner.width, *h);
        match row {
            Row::Header(title) => {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        title.as_str(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )),
                    rect,
                );
            }
            Row::Field(i) => form.render_field_row(frame, rect, *i, label_width),
            Row::Hint => render_hint_box(frame, rect, hint.as_ref()),
            Row::Blank => {}
            Row::Submit => form.render_submit(frame, rect, form.is_submit_focused()),
        }
        y += h;
    }
}

/// Fixed hint slot rendered below the Submit row.
///
/// The bordered box is ALWAYS drawn so it never flickers on and off as focus
/// moves between fields that do and don't have a description. When the focused
/// field has a hint, the box carries its text; otherwise it renders empty (just
/// the dim border), keeping the layout stable. Also reused by the JSON editor
/// so its hint box matches the form's.
pub(crate) fn render_hint_box(frame: &mut Frame, area: Rect, hint: Option<&(String, bool)>) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    // An error note is red; a plain field hint (or the empty no-hint box) is dim.
    let (text, color) = match hint {
        Some((text, true)) => (text.clone(), Color::Red),
        Some((text, false)) => (text.clone(), Color::Indexed(244)),
        None => (String::new(), Color::Indexed(244)),
    };
    // The hint slot sits below the Confirm button — keep the bordered box at
    // the panel's inner-x so its left edge aligns with the Confirm button
    // rather than indenting under the parameter labels.
    let para = Paragraph::new(Span::styled(
        text,
        Style::default().fg(color).add_modifier(Modifier::ITALIC),
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(color))
            .padding(Padding::horizontal(1)),
    )
    .wrap(Wrap { trim: false });
    frame.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::terminal::inline::form::{
        FieldKey, FieldSource, FieldType, FieldValue, FormField,
    };
    use ags_protocol::workflow::GatherSlotId;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_fields_view_renders_parameters_box_and_inputs_title() {
        let field = FormField {
            label: "user-id".into(),
            field_type: FieldType::Scalar,
            required: true,
            value: FieldValue::Empty,
            description: "who".into(),
            source: FieldSource::UserInput,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type":"string"}),
            read_only: false,
            dynamic: None,
        };
        let form = Form::new("x", vec![field]).with_submit_focusable(true);
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| render(f, f.area(), "Inputs", &form, true))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(s.contains("Parameters"));
        assert!(s.contains("Inputs"));
        assert!(s.contains("user-id"));
    }

    // ── render_inline tests ──────────────────────────────────────────────────

    /// Build field fixtures carrying descriptions.
    fn make_fields_with_desc(count: u32, desc: &str) -> Vec<FormField> {
        (0..count)
            .map(|i| FormField {
                label: format!("field-{i}"),
                field_type: FieldType::Scalar,
                required: false,
                value: FieldValue::Scalar(format!("v{i}")),
                description: if i == 0 {
                    desc.to_owned()
                } else {
                    String::new()
                },
                source: FieldSource::UserInput,
                key: FieldKey::Slot(GatherSlotId(i)),
                schema: serde_json::json!({"type":"string"}),
                read_only: false,
                dynamic: None,
            })
            .collect()
    }

    #[test]
    fn test_render_inline_hint_always_visible() {
        // More fields than fit; the first field (focus=0) has a description.
        // The hint box must appear in the buffer regardless of scroll.
        let desc = "AccelByte user ID";
        let fields = make_fields_with_desc(20, desc);
        let form = Form::new("x", fields).with_submit_focusable(true);
        // 12 rows total: border (top 1) + padding (1) + ~4 fields + submit (1)
        // + hint (4) + padding (1) + border (bottom 1) — hint is always pinned.
        let mut term = Terminal::new(TestBackend::new(60, 14)).unwrap();
        term.draw(|f| render_inline(f, f.area(), &form, true))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains(desc),
            "focused field hint visible despite many fields: {s}"
        );
    }

    #[test]
    fn test_render_inline_submit_always_visible() {
        // Many fields + show_submit=true: "[ Confirm ]" must appear
        // (pinned, not scrolled away).
        let fields = make_fields_with_desc(20, "");
        let form = Form::new("x", fields).with_submit_focusable(true);
        let mut term = Terminal::new(TestBackend::new(60, 14)).unwrap();
        term.draw(|f| render_inline(f, f.area(), &form, true))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains("[ Confirm ]"),
            "Submit button pinned and visible: {s}"
        );
    }

    #[test]
    fn test_hint_box_drawn_even_with_no_hint() {
        // Regression: a focused field with no description must still show the
        // bordered hint box (empty), not blank space, so it doesn't flicker on
        // and off as focus moves between fields.
        let mut term = Terminal::new(TestBackend::new(40, 4)).unwrap();
        term.draw(|f| render_hint_box(f, f.area(), None)).unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains('┌') && s.contains('│') && s.contains('└'),
            "empty hint box must still draw a border: {s}"
        );
    }

    #[test]
    fn test_render_inline_scrollbar_thumb_reaches_bottom_when_scrolled_down() {
        // With more fields than fit, the scrollbar thumb must (a) sit lower when
        // scrolled to the bottom than at the top, and (b) actually reach the
        // bottom of the track when fully scrolled down. The track bottom is found
        // from the rendered buffer (the bottom-most non-space cell in the
        // scrollbar column), so the assertion does not hard-code layout offsets.
        let fields = make_fields_with_desc(20, "");
        // (lowest thumb row, whether the thumb is flush with the scrollbar track
        // bottom — i.e. no track glyph directly below it). The cell below the
        // scrollbar track is the layout's blank gap row, so "blank or absent
        // directly below the thumb" means the thumb reaches the track bottom.
        let thumb_state = |focus: usize| -> (u16, bool) {
            let mut form = Form::new("x", fields.clone()).with_submit_focusable(true);
            form.focus = focus;
            let mut term = Terminal::new(TestBackend::new(40, 18)).unwrap();
            term.draw(|f| render_inline(f, f.area(), &form, true))
                .unwrap();
            let buf = term.backend().buffer();
            let (w, h) = (buf.area.width, buf.area.height);
            // Column that holds the thumb glyph (the scrollbar's right-edge column).
            let col = (0..w)
                .find(|&x| (0..h).any(|y| buf[(x, y)].symbol() == "\u{2588}"))
                .expect("scrollbar column present");
            let thumb_bottom = (0..h)
                .rev()
                .find(|&y| buf[(col, y)].symbol() == "\u{2588}")
                .expect("thumb present");
            let below = thumb_bottom + 1;
            let flush_with_track_bottom = below >= h || buf[(col, below)].symbol() == " ";
            (thumb_bottom, flush_with_track_bottom)
        };
        let (top_thumb, _) = thumb_state(0); // first field → scrolled to top
        let (bottom_thumb, flush) = thumb_state(20); // submit → scrolled to bottom
        assert!(
            bottom_thumb > top_thumb,
            "thumb moves down when scrolled to the bottom (top={top_thumb}, bottom={bottom_thumb})"
        );
        assert!(
            flush,
            "thumb reaches the bottom of the track when fully scrolled down (no track below it)"
        );
    }

    #[test]
    fn test_render_inline_hides_optional_empty_row_when_collapsed() {
        let required = FormField {
            label: "ns".into(),
            field_type: FieldType::Scalar,
            required: true,
            value: FieldValue::Scalar("x".into()),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("ns".into()),
            schema: serde_json::json!({"type":"string"}),
            read_only: false,
            dynamic: None,
        };
        let optional = FormField {
            label: "zebra".into(),
            field_type: FieldType::Scalar,
            required: false,
            value: FieldValue::Empty,
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Input("zebra".into()),
            schema: serde_json::json!({"type":"string"}),
            read_only: false,
            dynamic: None,
        };
        let form = Form::new("x", vec![required, optional])
            .with_submit_focusable(true)
            .with_optional_filter(true)
            .with_mark_required(true);
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| render_inline(f, f.area(), &form, true))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        // The `[o] show optional (+N)` affordance now lives in the nav bar (driven by
        // `drive_form`), not the Parameters box — so it is NOT rendered here.
        assert!(s.contains("ns"), "required row visible: {s}");
        assert!(
            !s.contains("zebra"),
            "optional-empty row hidden when collapsed: {s}"
        );
    }

    #[test]
    fn test_render_shows_run_mode_buttons() {
        // When `run_mode_buttons` is set, the fullscreen `render` path must draw
        // the three-button group, not the single Confirm button.
        let field = FormField {
            label: "season-name".into(),
            field_type: FieldType::Scalar,
            required: true,
            value: FieldValue::Scalar("Season 1".into()),
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type":"string"}),
            read_only: false,
            dynamic: None,
        };
        let mut form = Form::new("x", vec![field])
            .with_submit_focusable(true)
            .with_run_mode_buttons(true);
        form.focus_submit_if_available();
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| render(f, f.area(), "Inputs", &form, true))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains("Run & Review"),
            "run-mode button group must appear on fullscreen path: {s}"
        );
        assert!(
            !s.contains("[ Confirm ]"),
            "single Confirm button must NOT appear when run_mode_buttons is set: {s}"
        );
    }

    #[test]
    fn test_render_single_confirm_button_without_run_mode() {
        // Without `run_mode_buttons`, the single Confirm button must still render.
        let field = FormField {
            label: "user-id".into(),
            field_type: FieldType::Scalar,
            required: true,
            value: FieldValue::Empty,
            description: String::new(),
            source: FieldSource::UserInput,
            key: FieldKey::Slot(GatherSlotId(0)),
            schema: serde_json::json!({"type":"string"}),
            read_only: false,
            dynamic: None,
        };
        let form = Form::new("x", vec![field]).with_submit_focusable(true);
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| render(f, f.area(), "Inputs", &form, true))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains("[ Confirm ]"),
            "single Confirm button must appear when run_mode_buttons is not set: {s}"
        );
    }
}
