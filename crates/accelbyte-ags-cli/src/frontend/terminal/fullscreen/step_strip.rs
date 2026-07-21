//! Step strip — top of the fullscreen layout.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
use ratatui::Frame;

/// Identity of a step-strip row, separating the leading Inputs pseudo-row from
/// real workflow steps. Workflow rows carry the runtime `CompiledStep.index`
/// used to route `StepStarted`/`StepFinished` events; the Inputs row is never
/// matched by those events (its state is driven by Phase 1 directly).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepRowKind {
    /// The leading "step 0 — Inputs" collection row.
    Inputs,
    /// A real workflow step, keyed by its runtime index.
    Workflow {
        /// 0-based `CompiledStep.index`.
        runtime_index: usize,
    },
}

/// What the frame title calls the run: a registered multi-step `workflow run`
/// or a single synthesised service `command`. Only changes the header prefix
/// (`ags workflow:` vs `ags command:`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeaderKind {
    /// `ags workflow: <name>` — a registered multi-step workflow run.
    #[default]
    Workflow,
    /// `ags command: <name>` — a single synthesised service command.
    Command,
}

#[derive(Debug, Clone)]
pub struct Step {
    pub kind: StepRowKind,
    pub title: String,
    pub state: StepState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Pending,
    Current,
    Complete,
    Failed,
    Skipped,
}

/// Render the header as a single bordered box whose frame title carries
/// `ags workflow: <name>` (or `ags command: <name>` for a single synthesised
/// command) — drawn once, on the border, no duplication; the left-aligned step
/// strip sits inside with top/bottom breathing room.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    header_title: &str,
    header_kind: HeaderKind,
    status: &str,
    steps: &[Step],
    _workflow_description: Option<&str>,
) {
    let prefix = match header_kind {
        HeaderKind::Workflow => "ags workflow",
        HeaderKind::Command => "ags command",
    };
    let title = if status.is_empty() {
        format!(" {prefix}: {header_title} ")
    } else {
        format!(" {prefix}: {header_title}  \u{00B7}  {status} ")
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::new(2, 2, 1, 1))
        .title(title)
        .title_alignment(Alignment::Center);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Step strip on the first inner line. Left-aligned, NOT centered: the
    // windowed line's width changes as steps gain/lose their state prefix
    // (Pending has none; Current/Complete add "● "/"✔ "), and centering would
    // re-centre the whole strip on every such change — making the leading
    // gather-inputs row visibly jump sideways when the run starts. A fixed left
    // edge keeps it put; only its colour and the current-marker move.
    let strip_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let line = build_strip_line(steps, inner.width);
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Left), strip_area);
}

/// Display-column cost of the separator between two step entries.
const SEP_WIDTH: usize = 5; // "  ·  " = 2 + 1 + 2
const SEP: &str = "  \u{00B7}  ";

/// Index of the step to centre the window on.
/// Priority: first `Current` → first `Pending` → last → 0.
fn current_step_index(steps: &[Step]) -> usize {
    steps
        .iter()
        .position(|s| s.state == StepState::Current)
        .or_else(|| steps.iter().position(|s| s.state == StepState::Pending))
        .unwrap_or_else(|| steps.len().saturating_sub(1))
}

/// Whether window `[lo, hi]` (inclusive) fits within `avail` columns, including
/// the `‹ +K` / `+M ›` affordance spans shown when steps are hidden.
fn window_fits(step_widths: &[usize], lo: usize, hi: usize, n: usize, avail: u16) -> bool {
    let hidden_left = lo;
    let hidden_right = n.saturating_sub(hi + 1);
    let left_cost = if hidden_left > 0 {
        format!("\u{2039} +{hidden_left}").chars().count() + SEP_WIDTH
    } else {
        0
    };
    let right_cost = if hidden_right > 0 {
        SEP_WIDTH + format!("+{hidden_right} \u{203a}").chars().count()
    } else {
        0
    };
    let visible_count = hi - lo + 1;
    let content: usize =
        step_widths[lo..=hi].iter().sum::<usize>() + SEP_WIDTH * visible_count.saturating_sub(1);
    left_cost + content + right_cost <= avail as usize
}

/// Build the step-strip [`Line`] for `width` display columns. All steps when
/// they fit; otherwise a sliding window centred on the current step with
/// `‹ +K` / `+M ›` hidden-count affordances. The current step is always
/// included (ratatui clips at the edge if it alone exceeds `width`).
pub(crate) fn build_strip_line(steps: &[Step], width: u16) -> Line<'static> {
    let n = steps.len();
    if n == 0 {
        return Line::from(vec![]);
    }
    let step_widths: Vec<usize> = steps.iter().map(|s| step_span(s).width()).collect();

    let total: usize = step_widths.iter().sum::<usize>() + SEP_WIDTH * n.saturating_sub(1);
    if total <= width as usize {
        let mut spans = Vec::with_capacity(n * 2);
        for (i, step) in steps.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(SEP));
            }
            spans.push(step_span(step));
        }
        return Line::from(spans);
    }

    let current = current_step_index(steps);
    let mut lo = current;
    let mut hi = current;
    loop {
        let can_left = lo > 0;
        let can_right = hi < n - 1;
        if !can_left && !can_right {
            break;
        }
        let mut expanded = false;
        if can_left && window_fits(&step_widths, lo - 1, hi, n, width) {
            lo -= 1;
            expanded = true;
        }
        if can_right && window_fits(&step_widths, lo, hi + 1, n, width) {
            hi += 1;
            expanded = true;
        }
        if !expanded {
            break;
        }
    }

    let affordance_style = Style::default().fg(Color::Indexed(244));
    let hidden_left = lo;
    let hidden_right = n.saturating_sub(hi + 1);
    let visible_count = hi - lo + 1;
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(visible_count * 2 + 4);
    if hidden_left > 0 {
        spans.push(Span::styled(
            format!("\u{2039} +{hidden_left}"),
            affordance_style,
        ));
        spans.push(Span::raw(SEP));
    }
    for (j, step) in steps[lo..=hi].iter().enumerate() {
        if j > 0 {
            spans.push(Span::raw(SEP));
        }
        spans.push(step_span(step));
    }
    if hidden_right > 0 {
        spans.push(Span::raw(SEP));
        spans.push(Span::styled(
            format!("+{hidden_right} \u{203a}"),
            affordance_style,
        ));
    }
    Line::from(spans)
}

/// Build the styled span for one step in the header step strip, coloured by its state.
fn step_span(step: &Step) -> Span<'static> {
    match step.state {
        StepState::Pending => {
            Span::styled(step.title.clone(), Style::default().fg(Color::Indexed(244)))
        }
        StepState::Current => Span::styled(
            format!("\u{25CF} {}", step.title),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        StepState::Complete => Span::styled(
            format!("\u{2714} {}", step.title),
            Style::default().fg(Color::Green),
        ),
        StepState::Failed => Span::styled(
            format!(
                "{} {}",
                crate::frontend::style::text::SYMBOL_ERROR,
                step.title
            ),
            Style::default().fg(Color::Red),
        ),
        StepState::Skipped => Span::styled(
            format!(
                "{} {}",
                crate::frontend::style::text::SYMBOL_SKIPPED,
                step.title
            ),
            Style::default().fg(Color::Indexed(244)),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }
    fn wf_step(title: &str, state: StepState) -> Step {
        Step {
            kind: StepRowKind::Workflow { runtime_index: 0 },
            title: title.into(),
            state,
        }
    }

    #[test]
    fn test_build_strip_line_all_steps_fit_no_affordances() {
        let steps = vec![
            wf_step("A", StepState::Current),
            wf_step("B", StepState::Pending),
            wf_step("C", StepState::Pending),
        ];
        let line = build_strip_line(&steps, 80);
        assert!(line.width() <= 80);
        let t = line_text(&line);
        assert!(t.contains('A') && t.contains('B') && t.contains('C'));
        assert!(
            !t.contains('\u{2039}') && !t.contains('\u{203a}'),
            "no affordances: {t}"
        );
    }

    #[test]
    fn test_build_strip_line_14_steps_fits_within_width_and_shows_affordances() {
        let steps: Vec<Step> = (0..14)
            .map(|i| {
                wf_step(
                    &format!("Step {}", i + 1),
                    if i == 7 {
                        StepState::Current
                    } else {
                        StepState::Pending
                    },
                )
            })
            .collect();
        let line = build_strip_line(&steps, 74);
        assert!(line.width() <= 74, "width {} > 74", line.width());
        let t = line_text(&line);
        assert!(t.contains("Step 8"), "current step visible: {t}");
        assert!(t.contains('\u{2039}'), "left affordance: {t}");
        assert!(t.contains('\u{203a}'), "right affordance: {t}");
    }

    #[test]
    fn test_build_strip_line_current_at_index_zero_no_left_affordance() {
        let steps: Vec<Step> = (0..14)
            .map(|i| {
                wf_step(
                    &format!("Step {}", i + 1),
                    if i == 0 {
                        StepState::Current
                    } else {
                        StepState::Pending
                    },
                )
            })
            .collect();
        let line = build_strip_line(&steps, 40);
        assert!(line.width() <= 40);
        let t = line_text(&line);
        assert!(t.contains("Step 1"));
        assert!(
            !t.contains('\u{2039}'),
            "no left affordance at index 0: {t}"
        );
        assert!(t.contains('\u{203a}'), "right affordance present: {t}");
    }

    #[test]
    fn test_build_strip_line_single_step_no_separators() {
        let line = build_strip_line(&[wf_step("Init", StepState::Current)], 40);
        let t = line_text(&line);
        assert!(t.contains("Init"));
        assert!(!t.contains('\u{00b7}'), "no separator: {t}");
        assert!(!t.contains('\u{2039}') && !t.contains('\u{203a}'));
    }

    #[test]
    fn test_build_strip_line_empty_returns_empty() {
        assert!(build_strip_line(&[], 80).spans.is_empty());
    }

    #[test]
    fn test_step_span_complete_uses_check_glyph_and_title() {
        let s = Step {
            kind: StepRowKind::Workflow { runtime_index: 0 },
            title: "Stat".into(),
            state: StepState::Complete,
        };
        let span = step_span(&s);
        assert!(span.content.contains("\u{2714}"));
        assert!(span.content.contains("Stat"));
        assert!(!span.content.contains("1. "), "numbering dropped");
    }

    #[test]
    fn test_step_span_failed_uses_cross_glyph() {
        let s = Step {
            kind: StepRowKind::Workflow { runtime_index: 2 },
            title: "Session".into(),
            state: StepState::Failed,
        };
        let span = step_span(&s);
        assert!(span
            .content
            .contains(crate::frontend::style::text::SYMBOL_ERROR));
        assert!(span.content.contains("Session"));
        assert!(!span.content.contains("3. "), "numbering dropped");
    }

    #[test]
    fn test_step_span_current_uses_filled_circle() {
        let s = Step {
            kind: StepRowKind::Workflow { runtime_index: 1 },
            title: "Build".into(),
            state: StepState::Current,
        };
        let span = step_span(&s);
        assert!(span.content.contains("\u{25CF}"));
        assert!(span.content.contains("Build"));
    }

    #[test]
    fn test_step_span_skipped_uses_em_dash() {
        let s = Step {
            kind: StepRowKind::Workflow { runtime_index: 4 },
            title: "Cleanup".into(),
            state: StepState::Skipped,
        };
        assert!(step_span(&s).content.contains("\u{2014}"));
    }

    #[test]
    fn test_inputs_row_renders_title_only() {
        let s = Step {
            kind: StepRowKind::Inputs,
            title: "Inputs".into(),
            state: StepState::Current,
        };
        // Inputs is in `Current` state, so it shows the filled circle + title.
        assert!(step_span(&s).content.contains("Inputs"));
        assert!(!step_span(&s).content.contains("0. "), "no numbering");
    }

    #[test]
    fn test_workflow_row_renders_title_only() {
        let s = Step {
            kind: StepRowKind::Workflow { runtime_index: 0 },
            title: "Create the MMR skill stat".into(),
            state: StepState::Current,
        };
        let span = step_span(&s);
        assert!(span.content.contains("Create the MMR skill stat"));
        assert!(!span.content.contains("1. "), "no numbering");
    }

    #[test]
    fn test_step_span_pending_renders_title_without_glyph() {
        let s = Step {
            kind: StepRowKind::Workflow { runtime_index: 0 },
            title: "create-stat".into(),
            state: StepState::Pending,
        };
        let span = step_span(&s);
        assert!(
            !span.content.contains("\u{00B7}"),
            "no leading dot: {}",
            span.content
        );
        assert!(!span.content.contains("\u{25CF}"), "no filled circle");
        assert!(
            span.content.trim_start() == "create-stat",
            "title only: {}",
            span.content
        );
    }

    #[test]
    fn test_render_description_argument_is_ignored() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let steps = vec![Step {
            kind: StepRowKind::Workflow { runtime_index: 0 },
            title: "create-stat".into(),
            state: StepState::Current,
        }];
        // Passing a description has no visual effect — the strip always
        // occupies a single inner line regardless.
        let mut term = Terminal::new(TestBackend::new(80, 7)).unwrap();
        term.draw(|f| {
            render(
                f,
                f.area(),
                "wf",
                HeaderKind::Workflow,
                "",
                &steps,
                Some("Sets up matchmaking."),
            )
        })
        .unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            !buf.contains("Sets up matchmaking."),
            "description must not be rendered: {buf}"
        );
        assert!(buf.contains("create-stat"), "strip still rendered: {buf}");
    }

    #[test]
    fn test_render_omits_description_when_absent() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let steps = vec![Step {
            kind: StepRowKind::Workflow { runtime_index: 0 },
            title: "create-stat".into(),
            state: StepState::Current,
        }];
        let mut term = Terminal::new(TestBackend::new(80, 5)).unwrap();
        term.draw(|f| render(f, f.area(), "wf", HeaderKind::Workflow, "", &steps, None))
            .unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(buf.contains("create-stat"), "strip still rendered: {buf}");
    }

    #[test]
    fn test_render_header_prefix_reflects_kind() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let steps = vec![Step {
            kind: StepRowKind::Workflow { runtime_index: 0 },
            title: "gather-inputs".into(),
            state: StepState::Current,
        }];
        let frame_text = |kind: HeaderKind| -> String {
            let mut term = Terminal::new(TestBackend::new(80, 7)).unwrap();
            term.draw(|f| render(f, f.area(), "iam users create", kind, "", &steps, None))
                .unwrap();
            term.backend()
                .buffer()
                .content()
                .iter()
                .map(|c| c.symbol())
                .collect()
        };
        let workflow = frame_text(HeaderKind::Workflow);
        assert!(
            workflow.contains("ags workflow: iam users create"),
            "workflow prefix: {workflow}"
        );
        let command = frame_text(HeaderKind::Command);
        assert!(
            command.contains("ags command: iam users create"),
            "command prefix: {command}"
        );
        assert!(
            !command.contains("ags workflow:"),
            "command run must not say workflow: {command}"
        );
    }

    #[test]
    fn test_gather_inputs_row_does_not_shift_when_run_starts() {
        use ratatui::{backend::TestBackend, Terminal};

        // Leading Inputs row + several long-titled steps at a width where the
        // window is active. When the run starts, the first step gains a "● "
        // prefix and one fewer row fits, changing the line width — which a
        // centred strip would re-centre, shifting gather-inputs sideways.
        let titles = [
            "create-category",
            "create-pass-item-free",
            "create-pass-item-premium",
            "create-tier-item",
            "publish-store",
        ];
        let build = |inputs: StepState, first: StepState| -> Vec<Step> {
            let mut steps = vec![Step {
                kind: StepRowKind::Inputs,
                title: "gather-inputs".into(),
                state: inputs,
            }];
            for (i, t) in titles.iter().enumerate() {
                steps.push(Step {
                    kind: StepRowKind::Workflow { runtime_index: i },
                    title: (*t).into(),
                    state: if i == 0 { first } else { StepState::Pending },
                });
            }
            steps
        };
        let column_of_gather_inputs = |steps: &[Step]| -> usize {
            let mut term = Terminal::new(TestBackend::new(120, 5)).unwrap();
            term.draw(|f| {
                render(
                    f,
                    f.area(),
                    "season-pass",
                    HeaderKind::Workflow,
                    "",
                    steps,
                    None,
                )
            })
            .unwrap();
            let buf = term.backend().buffer().clone();
            for y in 0..buf.area.height {
                let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
                if let Some(idx) = row.find("gather-inputs") {
                    return idx;
                }
            }
            panic!("gather-inputs row not found in the rendered strip");
        };

        // During gather the Inputs row is Current; at run start it is Complete
        // and the first workflow step is Current. The gather-inputs text must
        // start at the same column in both — a stable left edge, no jump.
        let during_gather = build(StepState::Current, StepState::Pending);
        let run_started = build(StepState::Complete, StepState::Current);
        assert_eq!(
            column_of_gather_inputs(&during_gather),
            column_of_gather_inputs(&run_started),
            "gather-inputs must not shift horizontally when the run starts"
        );
    }
}
