//! TTY topology coverage — 8 combinations.
//!
//! Exhaustively walks the (stdin × stdout × stderr) TTY-status truth
//! table to verify the invariant that drives prompt eligibility and
//! surface choice: prompts require both stdin AND stderr to be
//! terminals; stdout's TTY status only affects stdout-buffering
//! semantics, never prompt eligibility (invariants 2 + 5).
//!
//! Per-shape surface resolution is also exercised across every
//! combination so a future refactor that introduces a panic in the
//! decision matrix gets caught here regardless of what TTY state the
//! test process happens to inherit.

use ags::invocation::context::TerminalCapabilities;
use ags::invocation::policy::{base_surface, Surface};
use ags::invocation::shape::{RouteKind, Shape};

fn caps(stdin: bool, stdout: bool, stderr: bool) -> TerminalCapabilities {
    TerminalCapabilities {
        stdin_is_tty: stdin,
        stdout_is_tty: stdout,
        stderr_is_tty: stderr,
        color_force_off: false,
    }
}

const ALL_ROUTES: &[RouteKind] = &[
    RouteKind::Auth,
    RouteKind::Service,
    RouteKind::Workflow,
    RouteKind::Builtin,
];

const ALL_SHAPES: &[Shape] = &[
    Shape::Zero,
    Shape::Small,
    Shape::Form,
    Shape::Multi,
    Shape::Static,
    Shape::AuthSp,
];

#[test]
fn test_all_eight_tty_topologies_resolve_a_surface_without_panic() {
    for stdin in [false, true] {
        for stdout in [false, true] {
            for stderr in [false, true] {
                let c = caps(stdin, stdout, stderr);
                // Sanity: allows_interactive_prompts matches the spec rule.
                assert_eq!(c.allows_interactive_prompts(), stdin && stderr);
                // Surface resolves for every (route, shape) without panic.
                for &route in ALL_ROUTES {
                    for &shape in ALL_SHAPES {
                        let _ = base_surface(route, shape);
                    }
                }
            }
        }
    }
}

#[test]
fn test_stderr_not_tty_forbids_prompts_regardless_of_stdin() {
    assert!(!caps(true, true, false).allows_interactive_prompts());
    assert!(!caps(false, true, false).allows_interactive_prompts());
    assert!(!caps(true, false, false).allows_interactive_prompts());
    assert!(!caps(false, false, false).allows_interactive_prompts());
}

#[test]
fn test_stdin_not_tty_forbids_prompts_regardless_of_stderr() {
    assert!(!caps(false, true, true).allows_interactive_prompts());
    assert!(!caps(false, false, true).allows_interactive_prompts());
}

#[test]
fn test_stdout_tty_status_does_not_affect_prompt_eligibility() {
    let with = caps(true, true, true);
    let without = caps(true, false, true);
    assert_eq!(
        with.allows_interactive_prompts(),
        without.allows_interactive_prompts()
    );
}

#[test]
fn test_only_stdin_and_stderr_both_tty_enables_prompts() {
    // The single topology that enables interactive prompts.
    assert!(caps(true, true, true).allows_interactive_prompts());
    assert!(caps(true, false, true).allows_interactive_prompts());
    // Seven topologies that forbid them.
    let forbidden = [
        (true, true, false),
        (true, false, false),
        (false, true, true),
        (false, true, false),
        (false, false, true),
        (false, false, false),
    ];
    for (i, o, e) in forbidden {
        assert!(
            !caps(i, o, e).allows_interactive_prompts(),
            "stdin={i} stdout={o} stderr={e} must forbid prompts"
        );
    }
}

#[test]
fn test_workflow_route_returns_fullscreen_for_every_shape() {
    // Workflow row is always fullscreen.
    for &shape in ALL_SHAPES {
        assert_eq!(
            base_surface(RouteKind::Workflow, shape),
            Surface::Fullscreen,
            "shape {shape:?} on workflow route must map to fullscreen"
        );
    }
}

#[test]
fn test_auth_route_returns_plain_for_every_shape() {
    // Auth row is always plain.
    for &shape in ALL_SHAPES {
        assert_eq!(
            base_surface(RouteKind::Auth, shape),
            Surface::Plain,
            "shape {shape:?} on auth route must map to plain"
        );
    }
}
