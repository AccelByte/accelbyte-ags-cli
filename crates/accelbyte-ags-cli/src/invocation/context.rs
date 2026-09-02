//! Frontend context resolution.
//!
//! Resolves *who* an invocation is for (`ConsumerKind`), *what interaction*
//! is permitted (`InteractionPolicy`), and *what the terminal can do*
//! (`TerminalCapabilities`), independently of the concrete presentation surface.
//!
//! `resolve_frontend_context` is the single place that interprets `--format`,
//! `--no-input`, `--no-color`, and TTY detection. Invocation handlers that
//! gate on promptability read the resolved `FrontendContext` — via
//! `allows_input()` and `is_automation()` — rather than re-examining raw
//! flags. `surface_backend()` derives the `PhaseBackend` selector
//! from the same context.
//!
//! Not every modelled field has a consumer yet: the colour capabilities
//! (`stdout_color` / `stderr_color`) are populated but unread — `ansi::init`
//! still owns the render-time colour decision.

use crate::errors::CliError;
use crate::invocation::flags::{GlobalFlags, UiFlag};
use ags_protocol::request::OutputFormat;

/// Who the invocation is for. Drives the output contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumerKind {
    /// A person at an interactive terminal.
    Human,
    /// A script, CI job, or other automation consuming machine-readable output.
    Automation,
}

/// What kinds of interaction are permitted or preferred for this invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionPolicy {
    /// Whether the CLI may prompt for missing input or confirmation.
    pub allow_input: bool,
    /// Whether a richer terminal UI is preferred over plain line output.
    pub prefer_rich_ui: bool,
    /// Whether a fullscreen/alternate-screen UI is preferred. Set by
    /// `finalize_surface` for workflow runs and explicit `--ui=fullscreen`.
    pub prefer_fullscreen: bool,
}

/// What the attached terminal can do. Captured once, up front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCapabilities {
    /// Stdin is an interactive terminal.
    pub stdin_is_tty: bool,
    /// Stdout is an interactive terminal.
    pub stdout_is_tty: bool,
    /// Stderr is an interactive terminal.
    pub stderr_is_tty: bool,
    /// Colour is force-disabled (`--no-color` flag or `NO_COLOR` env).
    pub color_force_off: bool,
}

impl TerminalCapabilities {
    /// Capture the real process terminal state.
    pub fn detect(no_color_flag: bool) -> Self {
        Self {
            stdin_is_tty: ags_runtime::support::is_stdin_tty(),
            stdout_is_tty: ags_runtime::support::is_stdout_tty(),
            stderr_is_tty: ags_runtime::support::is_stderr_tty(),
            color_force_off: crate::frontend::style::color_force_off(no_color_flag),
        }
    }

    /// Whether stdin and stderr together support interactive prompts.
    /// Prompts need both stdin and stderr, regardless of stdout state.
    pub fn allows_interactive_prompts(&self) -> bool {
        self.stdin_is_tty && self.stderr_is_tty
    }

    /// Whether colour should be emitted on stdout.
    // modelled for a later stage; render-time colour still goes through ansi::init
    #[allow(dead_code)]
    pub fn stdout_color(&self) -> bool {
        !self.color_force_off && self.stdout_is_tty
    }

    /// Whether colour should be emitted on stderr.
    // modelled for a later stage; render-time colour still goes through ansi::init
    #[allow(dead_code)]
    pub fn stderr_color(&self) -> bool {
        !self.color_force_off && self.stderr_is_tty
    }
}

/// The fully resolved frontend context for one invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontendContext {
    pub consumer: ConsumerKind,
    pub interaction: InteractionPolicy,
    pub terminal: TerminalCapabilities,
    /// The canonicalised `--ui` intent captured at resolve time. `Auto` when
    /// no flag was passed. Drives `finalize_surface` once the route + shape
    /// are known.
    pub ui_intent: crate::invocation::flags::UiFlag,
}

impl FrontendContext {
    /// Whether this invocation may prompt for missing input or confirmation.
    pub fn allows_input(&self) -> bool {
        self.interaction.allow_input
    }

    /// Whether the consumer is an automation (machine-readable output contract).
    pub fn is_automation(&self) -> bool {
        matches!(self.consumer, ConsumerKind::Automation)
    }

    /// The resolved protocol output format for this invocation.
    ///
    /// Returns `OutputFormat::Json` for an automation consumer and
    /// `OutputFormat::Human` otherwise. Terminal UI is a presentation surface
    /// choice (`--ui`), not a protocol output format. The
    /// runtime/protocol-facing request structures consume this rather than
    /// raw `flags.format`.
    pub fn protocol_output_format(&self) -> OutputFormat {
        match self.consumer {
            ConsumerKind::Automation => OutputFormat::Json,
            ConsumerKind::Human => OutputFormat::Human,
        }
    }

    /// Resolve the [`PhaseBackend`] for a single workflow [`InteractionPhase`].
    ///
    /// Derived from [`surface_backend`](Self::surface_backend):
    /// - `PlainTerminal` → every phase renders on `PlainTerminal`.
    /// - `StructuredJson` → every phase renders on `StructuredJson`.
    ///   `Input`/`Confirmation` are unreachable for a valid JSON run — the
    ///   `no_input_precheck` rejects any workflow that would need interactive
    ///   input, and confirmations are pre-approved by `--yes` or skipped under
    ///   `--dry-run`. The arm exists so the match is exhaustive; `JsonInteraction`
    ///   provides a defensive `Usage` error if the precheck is ever bypassed.
    /// - `InlineTerminalUi` → `Input`/`Confirmation`/`Progress` render on
    ///   `InlineTerminalUi`; `Result`/`Error` fall back to `PlainTerminal`, since
    ///   the inline surface final result/error routes through stable plain rendering.
    pub fn backend_for_phase(&self, phase: InteractionPhase) -> PhaseBackend {
        match self.surface_backend() {
            PhaseBackend::PlainTerminal => PhaseBackend::PlainTerminal,
            PhaseBackend::StructuredJson => PhaseBackend::StructuredJson,
            PhaseBackend::InlineTerminalUi => match phase {
                InteractionPhase::Input
                | InteractionPhase::Confirmation
                | InteractionPhase::Progress => PhaseBackend::InlineTerminalUi,
                InteractionPhase::Result | InteractionPhase::Error => PhaseBackend::PlainTerminal,
            },
            // Fullscreen keeps every interactive phase on the same alt-screen
            // surface so the user sees one coherent layout for the whole run;
            // the dismiss loop owns the post-run result + error display in-
            // surface, then `finish()` flushes any deferred stdout emission.
            PhaseBackend::FullscreenTerminalUi => PhaseBackend::FullscreenTerminalUi,
        }
    }

    /// Apply the decision matrix now that the route and shape are
    /// known. Precedence: automation (JSON) > explicit `--ui` > matrix.
    /// Returns a context whose `surface_backend()` reflects the decision.
    ///
    /// Also publishes the resolved surface's telemetry label (see
    /// [`crate::invocation::publish_finalized_ui_surface`]) so
    /// `cli.command.invoked` — whose context is gathered before any route runs,
    /// while `--ui=auto` still reads as plain — reports the surface the user
    /// actually got, matching the run and step events. Publishing here rather
    /// than at each call site is deliberate: every future route inherits the
    /// invariant for free.
    pub fn finalize_surface(
        mut self,
        route: crate::invocation::shape::RouteKind,
        shape: crate::invocation::shape::Shape,
    ) -> Self {
        use crate::invocation::flags::UiFlag;
        use crate::invocation::policy::{base_surface, Surface};
        if matches!(self.consumer, ConsumerKind::Automation) {
            // StructuredJson; surface fields irrelevant. Still published: an
            // automation consumer's finalized surface is `structured_json`,
            // which is also what the pre-finalize label already says.
            crate::invocation::publish_finalized_ui_surface(
                self.surface_backend().telemetry_label(),
            );
            return self;
        }
        let surface = match self.ui_intent {
            UiFlag::Plain => Surface::Plain,
            UiFlag::Inline => Surface::Inline,
            UiFlag::Fullscreen => Surface::Fullscreen,
            UiFlag::Auto => base_surface(route, shape),
        };
        // Graceful degrade for the auto path: the inline/fullscreen surfaces
        // enter raw mode and read keys on an interactive terminal (rendering on
        // stderr, plus the gather + fullscreen dismiss loops). When stdin+stderr
        // aren't both TTYs (piped / non-interactive, e.g. CI or `cmd 2>file`),
        // an auto-selected TUI can't run, so fall back to plain line output —
        // the command still runs and renders. `stdout` may still be a pipe
        // (`workflow run X | tee` keeps fullscreen on stderr). An explicit
        // `--ui` is honoured as the user asked (resolve already rejects an
        // explicit TUI flag without a TTY).
        let surface = if matches!(self.ui_intent, UiFlag::Auto)
            && matches!(surface, Surface::Inline | Surface::Fullscreen)
            && !self.terminal.allows_interactive_prompts()
        {
            Surface::Plain
        } else {
            surface
        };
        match surface {
            Surface::Plain => {
                self.interaction.prefer_rich_ui = false;
                self.interaction.prefer_fullscreen = false;
            }
            Surface::Inline => {
                self.interaction.prefer_rich_ui = true;
                self.interaction.prefer_fullscreen = false;
            }
            Surface::Fullscreen => {
                self.interaction.prefer_rich_ui = false;
                self.interaction.prefer_fullscreen = true;
            }
            Surface::Json => {
                debug_assert!(false, "Json surface unreachable for a human consumer");
            }
        }
        crate::invocation::publish_finalized_ui_surface(self.surface_backend().telemetry_label());
        self
    }

    /// Resolve the pre-surface presentation backend: `StructuredJson` for an
    /// automation consumer (so JSON output stays byte-identical), `PlainTerminal`
    /// otherwise. Used wherever the CLI must render before any owned phase
    /// surface exists.
    pub fn pre_surface_backend(&self) -> PhaseBackend {
        if self.is_automation() {
            PhaseBackend::StructuredJson
        } else {
            PhaseBackend::PlainTerminal
        }
    }

    /// Resolve the overall presentation surface for this invocation.
    /// Precedence: machine-readable wins for automation consumers; among
    /// humans, fullscreen wins over inline, which wins over plain.
    pub fn surface_backend(&self) -> PhaseBackend {
        match self.consumer {
            ConsumerKind::Automation => PhaseBackend::StructuredJson,
            ConsumerKind::Human => {
                if self.interaction.prefer_fullscreen {
                    PhaseBackend::FullscreenTerminalUi
                } else if self.interaction.prefer_rich_ui {
                    PhaseBackend::InlineTerminalUi
                } else {
                    PhaseBackend::PlainTerminal
                }
            }
        }
    }
}

/// A distinct phase of a workflow run, each of which may be presented by a
/// different surface. Inline runs use the inline-terminal surface for
/// interaction/progress and the plain-terminal surface for final output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionPhase {
    /// Gathering missing workflow input values.
    Input,
    /// Per-step confirmation prompts.
    Confirmation,
    /// Lifecycle and progress reporting during the run.
    Progress,
    /// Rendering the final successful result payload.
    Result,
    /// Rendering a workflow failure.
    Error,
}

/// The concrete presentation surface resolved for a single
/// [`InteractionPhase`]. A workflow may use one surface for
/// interaction/progress and another for its final result/error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseBackend {
    /// Plain line-oriented human terminal output.
    PlainTerminal,
    /// Inline (non-fullscreen) terminal UI.
    InlineTerminalUi,
    /// Fullscreen alternate-screen workflow surface.
    FullscreenTerminalUi,
    /// Machine-readable JSON output.
    StructuredJson,
}

impl PhaseBackend {
    /// Stable telemetry label for the surface the user actually got.
    pub fn telemetry_label(&self) -> &'static str {
        match self {
            PhaseBackend::PlainTerminal => "plain",
            PhaseBackend::InlineTerminalUi => "inline",
            PhaseBackend::FullscreenTerminalUi => "fullscreen",
            PhaseBackend::StructuredJson => "structured_json",
        }
    }
}

/// Resolve the frontend context from CLI flags, capturing the real terminal.
pub fn resolve_frontend_context(flags: &GlobalFlags) -> Result<FrontendContext, CliError> {
    let terminal = TerminalCapabilities::detect(flags.is_no_color);
    resolve_with_terminal(flags, terminal)
}

/// Resolve a context for meta-commands (`--version`, `--help`) that never
/// render on a rich UI surface. Identical to [`resolve_frontend_context`] but
/// with the `--ui` intent neutralised, so the rich-UI gate
/// (which rejects an explicit TUI request on a non-interactive terminal) can
/// never fire and suppress version/help output. `--format=json` is preserved
/// so `ags --version --format=json` still emits the machine-readable form.
pub fn resolve_plain_meta_context(flags: &GlobalFlags) -> FrontendContext {
    let terminal = TerminalCapabilities::detect(flags.is_no_color);
    resolve_plain_meta_with_terminal(flags, terminal)
}

/// Pure core of [`resolve_plain_meta_context`], with the terminal injected so
/// it can be unit-tested without a real TTY. Neutralising `--ui` to `Plain`
/// means `resolve_with_terminal` can never take the rich-UI gate, so the
/// result is always `Ok`.
fn resolve_plain_meta_with_terminal(
    flags: &GlobalFlags,
    terminal: TerminalCapabilities,
) -> FrontendContext {
    let mut flags = flags.clone();
    flags.ui = Some(UiFlag::Plain);
    resolve_with_terminal(&flags, terminal)
        .expect("plain meta context neutralises the rich-UI gate, so it cannot fail")
}

/// Why interactive input is unavailable, for inclusion in usage-error
/// messages. The interactivity rule distinguishes stdin-missing from
/// stderr-missing because the right remediation differs.
pub fn input_unavailable_reason(caps: &TerminalCapabilities) -> &'static str {
    match (caps.stdin_is_tty, caps.stderr_is_tty) {
        (false, false) => {
            "Attempting to run interactively, but stdin and stderr are not terminals."
        }
        (false, true) => "Attempting to run interactively, but stdin is not a terminal.",
        (true, false) => {
            "Attempting to run interactively, but stderr is not a terminal (prompts would not be visible)."
        }
        // Both channels are TTYs, so a flag (--no-input or --format=json) disabled
        // interactive input rather than the terminal itself.
        (true, true) => "Interactive input is disabled.",
    }
}

/// Resolve the presentation surface from `--format`, `--ui`, and the
/// (route, shape) decision matrix. Pure mirror of the precedence logic that
/// `finalize_surface` applies inline on the live path; kept as a standalone
/// reference exercised by `tests/integration/format_precedence.rs`.
///
/// `--format=json` silently wins over `--ui`.
/// `UiFlag::Auto` defers to the `(route, shape)` matrix; explicit
/// `Plain`/`Inline`/`Fullscreen` override it.
#[allow(dead_code)] // pure reference fn; the runtime path uses finalize_surface
pub fn resolve_surface(
    format: OutputFormat,
    ui: crate::invocation::flags::UiFlag,
    route: crate::invocation::shape::RouteKind,
    shape: crate::invocation::shape::Shape,
) -> crate::invocation::policy::Surface {
    use crate::invocation::flags::UiFlag;
    use crate::invocation::policy::{base_surface, Surface};
    if format == OutputFormat::Json {
        return Surface::Json;
    }
    match ui {
        UiFlag::Plain => Surface::Plain,
        UiFlag::Inline => Surface::Inline,
        UiFlag::Fullscreen => Surface::Fullscreen,
        UiFlag::Auto => base_surface(route, shape),
    }
}

/// Pure resolver core: maps flags + already-captured capabilities to a context.
/// Separated from `resolve_frontend_context` so it can be unit-tested without
/// depending on whether the test process has a real TTY. Also used by test
/// helpers that must produce only resolver-valid contexts.
pub(crate) fn resolve_with_terminal(
    flags: &GlobalFlags,
    terminal: TerminalCapabilities,
) -> Result<FrontendContext, CliError> {
    let is_json = matches!(flags.format, Some(OutputFormat::Json));
    // Inline maps to the inline-terminal surface; Fullscreen maps to the
    // alt-screen surface. `ui_intent` carries the choice forward so
    // `finalize_surface` can apply the decision matrix once the route and
    // shape are known. Auto defers entirely to that matrix.
    let ui_intent = flags.ui.unwrap_or(UiFlag::Auto);
    let (is_tui, is_fullscreen) = match ui_intent {
        UiFlag::Inline => (true, false),
        UiFlag::Fullscreen => (true, true),
        UiFlag::Plain => (false, false),
        UiFlag::Auto => (false, false),
    };

    // `--format=json` silently ignores `--ui`.
    // The combination is not a usage error; JSON wins and `--ui` is dropped.
    let is_tui = is_tui && !is_json;
    let is_fullscreen = is_fullscreen && !is_json;

    // Both TUI surfaces read keys from stdin and render on stderr — stdout
    // stays reserved for `CommandOutput`, so `… | tee` keeps piped output
    // clean. The gate therefore requires stdin+stderr, NOT stdout, mirroring
    // the auto-path degrade in `finalize_surface`: an explicit `--ui=fullscreen`
    // with piped stdout now behaves the same as the auto selection.
    if is_tui && !terminal.allows_interactive_prompts() {
        // is_tui is only set by an explicit `--ui=inline` / `--ui=fullscreen`
        // (Auto defers to the matrix), so name the spelling the user passed.
        let requested = if is_fullscreen {
            "--ui=fullscreen"
        } else {
            "--ui=inline"
        };
        return Err(CliError::Usage {
            message: format!("A rich terminal UI ({requested}) cannot be shown"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata {
                reason: Some(input_unavailable_reason(&terminal).to_string()),
                suggestion: Some(format!(
                    "Omit {requested} for plain human output, or use --format=json for scripting."
                )),
                ..Default::default()
            })),
        });
    }

    let consumer = if is_json {
        ConsumerKind::Automation
    } else {
        ConsumerKind::Human
    };

    let interaction = InteractionPolicy {
        // `allow_input`: JSON is a machine contract (never prompt), `--no-input`
        // opts out explicitly, and prompting requires both stdin
        // and stderr to be terminals so prompts are actually visible.
        allow_input: !is_json && !flags.is_no_input && terminal.allows_interactive_prompts(),
        // `prefer_rich_ui` and `prefer_fullscreen` are exclusive sibling
        // intents — fullscreen subsumes inline, so only the higher one
        // is set. `surface_backend` picks fullscreen first then inline.
        prefer_rich_ui: is_tui && !is_fullscreen,
        prefer_fullscreen: is_fullscreen,
    };

    Ok(FrontendContext {
        consumer,
        interaction,
        terminal,
        ui_intent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build `TerminalCapabilities` with stdin, stdout, and stderr sharing one TTY state.
    fn caps(stdout_tty: bool) -> TerminalCapabilities {
        TerminalCapabilities {
            stdin_is_tty: stdout_tty,
            stdout_is_tty: stdout_tty,
            stderr_is_tty: stdout_tty, // mirrors stdout; construct TerminalCapabilities directly when streams differ
            color_force_off: false,
        }
    }

    /// Build `GlobalFlags` carrying only a format and no-input setting.
    fn flags_with(format: Option<OutputFormat>, no_input: bool) -> GlobalFlags {
        GlobalFlags {
            format,
            is_no_input: no_input,
            ..GlobalFlags::default()
        }
    }

    /// Build `GlobalFlags` carrying a format, `--ui` value, and no-input setting.
    fn flags_with_ui(
        format: Option<OutputFormat>,
        ui: Option<UiFlag>,
        no_input: bool,
    ) -> GlobalFlags {
        GlobalFlags {
            format,
            ui,
            is_no_input: no_input,
            ..GlobalFlags::default()
        }
    }

    #[test]
    fn test_json_resolves_to_automation_consumer() {
        let ctx = resolve_with_terminal(&flags_with(Some(OutputFormat::Json), false), caps(false))
            .unwrap();
        assert_eq!(ctx.consumer, ConsumerKind::Automation);
        assert_eq!(ctx.surface_backend(), PhaseBackend::StructuredJson);
    }

    #[test]
    fn test_json_disallows_input() {
        let ctx = resolve_with_terminal(&flags_with(Some(OutputFormat::Json), false), caps(false))
            .unwrap();
        assert!(!ctx.interaction.allow_input);
    }

    #[test]
    fn test_human_resolves_to_human_kind_and_allows_input() {
        let ctx = resolve_with_terminal(&flags_with(Some(OutputFormat::Human), false), caps(true))
            .unwrap();
        assert_eq!(ctx.consumer, ConsumerKind::Human);
        assert_eq!(ctx.surface_backend(), PhaseBackend::PlainTerminal);
        assert!(ctx.interaction.allow_input);
        assert!(!ctx.interaction.prefer_rich_ui);
    }

    #[test]
    fn test_unset_format_resolves_to_human_kind() {
        let ctx = resolve_with_terminal(&flags_with(None, false), caps(true)).unwrap();
        assert_eq!(ctx.surface_backend(), PhaseBackend::PlainTerminal);
    }

    #[test]
    fn test_no_input_disables_input_for_human() {
        let ctx = resolve_with_terminal(&flags_with(Some(OutputFormat::Human), true), caps(true))
            .unwrap();
        assert!(!ctx.interaction.allow_input);
    }

    // --- --ui resolution ---

    #[test]
    fn test_ui_inline_resolves_to_inline_surface_with_rich_ui() {
        let ctx = resolve_with_terminal(
            &flags_with_ui(None, Some(UiFlag::Inline), false),
            caps(true),
        )
        .unwrap();
        assert_eq!(ctx.surface_backend(), PhaseBackend::InlineTerminalUi);
        assert!(ctx.interaction.prefer_rich_ui);
        assert!(!ctx.interaction.prefer_fullscreen);
    }

    #[test]
    fn test_ui_inline_with_piped_stdout_but_tty_stdin_stderr_is_accepted() {
        // The TUI surfaces read stdin and render on stderr; stdout may be a
        // pipe (`workflow run X | tee`). An explicit `--ui` must not be rejected
        // just because stdout is redirected, matching the auto-path degrade.
        let piped_stdout = TerminalCapabilities {
            stdin_is_tty: true,
            stdout_is_tty: false,
            stderr_is_tty: true,
            color_force_off: false,
        };
        let ctx = resolve_with_terminal(
            &flags_with_ui(None, Some(UiFlag::Inline), false),
            piped_stdout,
        )
        .unwrap();
        assert_eq!(ctx.surface_backend(), PhaseBackend::InlineTerminalUi);
    }

    #[test]
    fn test_ui_inline_with_non_tty_stderr_is_rejected_even_with_tty_stdout() {
        // Rendering goes to stderr; without it the TUI can't draw, so reject
        // even though stdout is a TTY (the old stdout-only gate would allow it).
        let no_stderr = TerminalCapabilities {
            stdin_is_tty: true,
            stdout_is_tty: true,
            stderr_is_tty: false,
            color_force_off: false,
        };
        let err =
            resolve_with_terminal(&flags_with_ui(None, Some(UiFlag::Inline), false), no_stderr)
                .unwrap_err();
        assert!(matches!(err, CliError::Usage { .. }));
    }

    #[test]
    fn test_ui_plain_resolves_to_human_kind() {
        let ctx =
            resolve_with_terminal(&flags_with_ui(None, Some(UiFlag::Plain), false), caps(true))
                .unwrap();
        assert_eq!(ctx.surface_backend(), PhaseBackend::PlainTerminal);
        assert!(!ctx.interaction.prefer_rich_ui);
    }

    #[test]
    fn test_plain_meta_context_survives_fullscreen_on_non_tty() {
        // `ags --version --ui=fullscreen | cat`: the rich-UI gate would reject
        // an explicit fullscreen request on a non-interactive terminal, but the
        // meta path must neutralise the intent and resolve to plain instead.
        let ctx = resolve_plain_meta_with_terminal(
            &flags_with_ui(None, Some(UiFlag::Fullscreen), false),
            caps(false),
        );
        assert_eq!(ctx.surface_backend(), PhaseBackend::PlainTerminal);
        assert!(!ctx.interaction.prefer_rich_ui);
        assert!(!ctx.interaction.prefer_fullscreen);
    }

    #[test]
    fn test_plain_meta_context_preserves_json_format() {
        // `--format=json` is a machine contract, not a rich-UI request, so the
        // meta path leaves it intact (e.g. `ags --version --format=json`).
        let ctx = resolve_plain_meta_with_terminal(
            &flags_with(Some(OutputFormat::Json), false),
            caps(false),
        );
        assert_eq!(ctx.consumer, ConsumerKind::Automation);
        assert_eq!(ctx.surface_backend(), PhaseBackend::StructuredJson);
    }

    #[test]
    fn test_ui_with_json_is_silently_ignored_per_spec_4_1_3() {
        // `--format=json` silently wins over --ui.
        // No usage error; the resolver downgrades to the JSON path.
        let ctx = resolve_with_terminal(
            &flags_with_ui(Some(OutputFormat::Json), Some(UiFlag::Inline), false),
            caps(true),
        )
        .unwrap();
        assert!(
            ctx.is_automation(),
            "JSON must produce an automation consumer"
        );
        assert!(
            !ctx.interaction.prefer_rich_ui,
            "rich UI must be dropped when --format=json wins"
        );
    }

    #[test]
    fn test_ui_without_tty_is_rejected_naming_the_requested_mode() {
        // The refusal names the exact `--ui` spelling the user passed.
        for (ui, expected) in [
            (
                UiFlag::Fullscreen,
                "rich terminal UI (--ui=fullscreen) cannot be shown",
            ),
            (
                UiFlag::Inline,
                "rich terminal UI (--ui=inline) cannot be shown",
            ),
        ] {
            let err = resolve_with_terminal(&flags_with_ui(None, Some(ui), false), caps(false))
                .unwrap_err();
            match err {
                CliError::Usage { message, .. } => {
                    assert!(message.contains(expected), "unexpected message: {message}");
                }
                other => panic!("expected CliError::Usage, got {other:?}"),
            }
        }
    }

    // --- allows_input() ---

    #[test]
    fn test_allows_input_false_for_json() {
        let ctx = resolve_with_terminal(&flags_with(Some(OutputFormat::Json), false), caps(false))
            .unwrap();
        assert!(!ctx.allows_input());
    }

    #[test]
    fn test_allows_input_true_for_human_and_false_with_no_input() {
        let ctx_on =
            resolve_with_terminal(&flags_with(Some(OutputFormat::Human), false), caps(true))
                .unwrap();
        assert!(ctx_on.allows_input());

        let ctx_off =
            resolve_with_terminal(&flags_with(Some(OutputFormat::Human), true), caps(true))
                .unwrap();
        assert!(!ctx_off.allows_input());
    }

    #[test]
    fn test_allows_input_true_for_inline_ui_on_tty() {
        let ctx = resolve_with_terminal(
            &flags_with_ui(None, Some(UiFlag::Inline), false),
            caps(true),
        )
        .unwrap();
        assert!(ctx.allows_input());
    }

    // --- is_automation() ---

    #[test]
    fn test_is_automation_true_for_json() {
        let ctx = resolve_with_terminal(&flags_with(Some(OutputFormat::Json), false), caps(false))
            .unwrap();
        assert!(ctx.is_automation());
    }

    #[test]
    fn test_is_automation_false_for_human() {
        let ctx = resolve_with_terminal(&flags_with(Some(OutputFormat::Human), false), caps(true))
            .unwrap();
        assert!(!ctx.is_automation());
    }

    #[test]
    fn test_is_automation_false_for_inline_ui() {
        let ctx = resolve_with_terminal(
            &flags_with_ui(None, Some(UiFlag::Inline), false),
            caps(true),
        )
        .unwrap();
        assert!(!ctx.is_automation());
    }

    // --- protocol_output_format() ---

    #[test]
    fn test_protocol_output_format_ui_inline_is_human() {
        let ctx = resolve_with_terminal(
            &flags_with_ui(None, Some(UiFlag::Inline), false),
            caps(true),
        )
        .unwrap();
        assert_eq!(ctx.protocol_output_format(), OutputFormat::Human);
    }

    #[test]
    fn test_protocol_output_format_json_is_json() {
        let ctx = resolve_with_terminal(&flags_with(Some(OutputFormat::Json), false), caps(false))
            .unwrap();
        assert_eq!(ctx.protocol_output_format(), OutputFormat::Json);
    }

    #[test]
    fn test_protocol_output_format_default_is_human() {
        let ctx = resolve_with_terminal(&flags_with(None, false), caps(true)).unwrap();
        assert_eq!(ctx.protocol_output_format(), OutputFormat::Human);
    }

    // --- backend_for_phase() ---

    const ALL_PHASES: [InteractionPhase; 5] = [
        InteractionPhase::Input,
        InteractionPhase::Confirmation,
        InteractionPhase::Progress,
        InteractionPhase::Result,
        InteractionPhase::Error,
    ];

    #[test]
    fn test_backend_for_phase_human_is_plain_for_every_phase() {
        let ctx = resolve_with_terminal(&flags_with(Some(OutputFormat::Human), false), caps(true))
            .unwrap();
        for phase in ALL_PHASES {
            assert_eq!(
                ctx.backend_for_phase(phase),
                PhaseBackend::PlainTerminal,
                "human phase {phase:?}"
            );
        }
    }

    #[test]
    fn test_backend_for_phase_json_is_structured_for_every_phase() {
        let ctx = resolve_with_terminal(&flags_with(Some(OutputFormat::Json), false), caps(false))
            .unwrap();
        for phase in ALL_PHASES {
            assert_eq!(
                ctx.backend_for_phase(phase),
                PhaseBackend::StructuredJson,
                "json phase {phase:?}"
            );
        }
    }

    #[test]
    fn test_backend_for_phase_inline_ui_for_interaction_and_progress() {
        let ctx = resolve_with_terminal(
            &flags_with_ui(None, Some(UiFlag::Inline), false),
            caps(true),
        )
        .unwrap();
        assert_eq!(
            ctx.backend_for_phase(InteractionPhase::Input),
            PhaseBackend::InlineTerminalUi
        );
        assert_eq!(
            ctx.backend_for_phase(InteractionPhase::Confirmation),
            PhaseBackend::InlineTerminalUi
        );
        assert_eq!(
            ctx.backend_for_phase(InteractionPhase::Progress),
            PhaseBackend::InlineTerminalUi
        );
    }

    #[test]
    fn test_backend_for_phase_inline_ui_plain_for_result_and_error() {
        let ctx = resolve_with_terminal(
            &flags_with_ui(None, Some(UiFlag::Inline), false),
            caps(true),
        )
        .unwrap();
        assert_eq!(
            ctx.backend_for_phase(InteractionPhase::Result),
            PhaseBackend::PlainTerminal
        );
        assert_eq!(
            ctx.backend_for_phase(InteractionPhase::Error),
            PhaseBackend::PlainTerminal
        );
    }

    // --- pre_surface_backend() ---

    #[test]
    fn test_pre_surface_backend_automation_is_json() {
        let ctx = resolve_with_terminal(&flags_with(Some(OutputFormat::Json), false), caps(false))
            .unwrap();
        assert_eq!(ctx.pre_surface_backend(), PhaseBackend::StructuredJson);
    }

    #[test]
    fn test_pre_surface_backend_human_is_human() {
        let ctx = resolve_with_terminal(&flags_with(Some(OutputFormat::Human), false), caps(true))
            .unwrap();
        assert_eq!(ctx.pre_surface_backend(), PhaseBackend::PlainTerminal);
    }

    #[test]
    fn test_pre_surface_backend_inline_ui_request_stays_human() {
        // A `--ui=inline` invocation resolves a *human* pre-surface backend:
        // prologue access-token warnings and cold-cache `Preparing specs...`
        // progress render on plain human and never acquire the TUI terminal.
        let ctx = resolve_with_terminal(
            &flags_with_ui(None, Some(UiFlag::Inline), false),
            caps(true),
        )
        .unwrap();
        assert_eq!(ctx.pre_surface_backend(), PhaseBackend::PlainTerminal);
    }

    #[test]
    fn test_terminal_color_derivation() {
        let on = TerminalCapabilities {
            stdin_is_tty: true,
            stdout_is_tty: true,
            stderr_is_tty: false,
            color_force_off: false,
        };
        assert!(on.stdout_color());
        assert!(!on.stderr_color());

        let forced = TerminalCapabilities {
            color_force_off: true,
            ..on
        };
        assert!(!forced.stdout_color());
        assert!(!forced.stderr_color());
    }

    #[test]
    fn test_allows_interactive_prompts_requires_stdin_and_stderr_both_tty() {
        let caps = |stdin, stderr| TerminalCapabilities {
            stdin_is_tty: stdin,
            stdout_is_tty: true,
            stderr_is_tty: stderr,
            color_force_off: false,
        };
        assert!(caps(true, true).allows_interactive_prompts());
        assert!(!caps(false, true).allows_interactive_prompts());
        assert!(!caps(true, false).allows_interactive_prompts());
        assert!(!caps(false, false).allows_interactive_prompts());
    }

    #[test]
    fn test_allow_input_false_when_stderr_not_tty_even_with_stdin_tty() {
        let caps = TerminalCapabilities {
            stdin_is_tty: true,
            stdout_is_tty: true,
            stderr_is_tty: false,
            color_force_off: false,
        };
        assert!(!caps.allows_interactive_prompts());
    }

    #[test]
    fn test_input_unavailable_reason_names_specific_missing_channel() {
        let no_stdin = TerminalCapabilities {
            stdin_is_tty: false,
            stdout_is_tty: true,
            stderr_is_tty: true,
            color_force_off: false,
        };
        let no_stderr = TerminalCapabilities {
            stdin_is_tty: true,
            stdout_is_tty: true,
            stderr_is_tty: false,
            color_force_off: false,
        };
        let neither = TerminalCapabilities {
            stdin_is_tty: false,
            stdout_is_tty: true,
            stderr_is_tty: false,
            color_force_off: false,
        };
        let both = TerminalCapabilities {
            stdin_is_tty: true,
            stdout_is_tty: true,
            stderr_is_tty: true,
            color_force_off: false,
        };
        assert_eq!(
            input_unavailable_reason(&no_stdin),
            "Attempting to run interactively, but stdin is not a terminal."
        );
        assert!(input_unavailable_reason(&no_stderr)
            .starts_with("Attempting to run interactively, but stderr is not a terminal"));
        assert_eq!(
            input_unavailable_reason(&neither),
            "Attempting to run interactively, but stdin and stderr are not terminals."
        );
        assert_eq!(
            input_unavailable_reason(&both),
            "Interactive input is disabled."
        );
    }

    #[test]
    fn test_allows_interactive_prompts_ignores_stdout_status() {
        let with_stdout = TerminalCapabilities {
            stdin_is_tty: true,
            stdout_is_tty: true,
            stderr_is_tty: true,
            color_force_off: false,
        };
        let without_stdout = TerminalCapabilities {
            stdin_is_tty: true,
            stdout_is_tty: false,
            stderr_is_tty: true,
            color_force_off: false,
        };
        assert_eq!(
            with_stdout.allows_interactive_prompts(),
            without_stdout.allows_interactive_prompts()
        );
    }

    // --- finalize_surface() ---

    fn sample_human_auto_ctx() -> FrontendContext {
        FrontendContext {
            consumer: ConsumerKind::Human,
            interaction: InteractionPolicy {
                allow_input: true,
                prefer_rich_ui: false,
                prefer_fullscreen: false,
            },
            terminal: TerminalCapabilities {
                stdin_is_tty: true,
                stdout_is_tty: true,
                stderr_is_tty: true,
                color_force_off: false,
            },
            ui_intent: crate::invocation::flags::UiFlag::Auto,
        }
    }

    #[test]
    fn test_finalize_surface_auto_service_form_resolves_inline() {
        use crate::invocation::shape::{RouteKind, Shape};
        let ctx = FrontendContext {
            consumer: ConsumerKind::Human,
            interaction: InteractionPolicy {
                allow_input: true,
                prefer_rich_ui: false,
                prefer_fullscreen: false,
            },
            terminal: TerminalCapabilities {
                stdin_is_tty: true,
                stdout_is_tty: true,
                stderr_is_tty: true,
                color_force_off: false,
            },
            ui_intent: crate::invocation::flags::UiFlag::Auto,
        }
        .finalize_surface(RouteKind::Service, Shape::Form);
        assert_eq!(ctx.surface_backend(), PhaseBackend::InlineTerminalUi);
    }

    #[test]
    fn test_finalize_surface_auto_workflow_resolves_fullscreen() {
        use crate::invocation::shape::{RouteKind, Shape};
        let ctx = sample_human_auto_ctx().finalize_surface(RouteKind::Workflow, Shape::Multi);
        assert_eq!(ctx.surface_backend(), PhaseBackend::FullscreenTerminalUi);
    }

    #[test]
    fn test_finalize_surface_explicit_plain_overrides_matrix() {
        use crate::invocation::shape::{RouteKind, Shape};
        let mut ctx = sample_human_auto_ctx();
        ctx.ui_intent = crate::invocation::flags::UiFlag::Plain;
        let ctx = ctx.finalize_surface(RouteKind::Service, Shape::Form);
        assert_eq!(ctx.surface_backend(), PhaseBackend::PlainTerminal);
    }

    #[test]
    fn test_finalize_surface_automation_stays_json() {
        use crate::invocation::shape::{RouteKind, Shape};
        let mut ctx = sample_human_auto_ctx();
        ctx.consumer = ConsumerKind::Automation;
        let ctx = ctx.finalize_surface(RouteKind::Service, Shape::Form);
        assert_eq!(ctx.surface_backend(), PhaseBackend::StructuredJson);
    }

    #[test]
    fn test_finalize_surface_auto_non_tty_degrades_tui_to_plain() {
        use crate::invocation::shape::{RouteKind, Shape};
        let mut ctx = sample_human_auto_ctx();
        ctx.terminal = TerminalCapabilities {
            stdin_is_tty: false,
            stdout_is_tty: false,
            stderr_is_tty: false,
            color_force_off: false,
        };
        // Workflow would auto-select fullscreen, but a non-interactive terminal
        // can't run a TUI, so it degrades to plain line output.
        let ctx = ctx.finalize_surface(RouteKind::Workflow, Shape::Multi);
        assert_eq!(ctx.surface_backend(), PhaseBackend::PlainTerminal);
    }

    #[test]
    fn test_finalize_surface_auto_piped_stdout_keeps_fullscreen() {
        use crate::invocation::shape::{RouteKind, Shape};
        // `workflow run X | tee`: stdout is a pipe, but stdin+stderr are TTYs,
        // so the fullscreen surface still runs (it renders on stderr; the result
        // streams to the piped stdout).
        let mut ctx = sample_human_auto_ctx();
        ctx.terminal = TerminalCapabilities {
            stdin_is_tty: true,
            stdout_is_tty: false,
            stderr_is_tty: true,
            color_force_off: false,
        };
        let ctx = ctx.finalize_surface(RouteKind::Workflow, Shape::Multi);
        assert_eq!(ctx.surface_backend(), PhaseBackend::FullscreenTerminalUi);
    }

    /// Every `PhaseBackend` variant must map to its exact telemetry label —
    /// these strings are a closed vocabulary transmitted verbatim.
    #[test]
    fn test_phase_backend_telemetry_label_matches_each_variant() {
        assert_eq!(PhaseBackend::PlainTerminal.telemetry_label(), "plain");
        assert_eq!(PhaseBackend::InlineTerminalUi.telemetry_label(), "inline");
        assert_eq!(
            PhaseBackend::FullscreenTerminalUi.telemetry_label(),
            "fullscreen"
        );
        assert_eq!(
            PhaseBackend::StructuredJson.telemetry_label(),
            "structured_json"
        );
    }
}
