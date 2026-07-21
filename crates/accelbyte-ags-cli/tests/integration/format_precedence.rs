//! `--format=json` silently wins over `--ui` (invariant 3);
//! otherwise an explicit `--ui` overrides the decision matrix, and `auto`
//! defers to it.

use ags::invocation::context::resolve_surface;
use ags::invocation::flags::UiFlag;
use ags::invocation::policy::Surface;
use ags::invocation::shape::{RouteKind, Shape};
use ags_protocol::request::OutputFormat;

#[test]
fn test_json_wins_over_ui_fullscreen() {
    let s = resolve_surface(
        OutputFormat::Json,
        UiFlag::Fullscreen,
        RouteKind::Workflow,
        Shape::Multi,
    );
    assert_eq!(s, Surface::Json);
}

#[test]
fn test_explicit_ui_overrides_matrix_when_format_not_json() {
    // Workflow + Multi normally maps to Fullscreen via the matrix;
    // an explicit --ui=plain must win.
    let s = resolve_surface(
        OutputFormat::Human,
        UiFlag::Plain,
        RouteKind::Workflow,
        Shape::Multi,
    );
    assert_eq!(s, Surface::Plain);
}

#[test]
fn test_auto_falls_back_to_matrix() {
    // Service + Form maps to Inline.
    let s = resolve_surface(
        OutputFormat::Human,
        UiFlag::Auto,
        RouteKind::Service,
        Shape::Form,
    );
    assert_eq!(s, Surface::Inline);
}
