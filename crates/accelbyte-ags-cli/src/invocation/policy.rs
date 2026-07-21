//! Decision matrix: (route, shape) -> base surface.
//!
//! `base_surface` is the live runtime path — `context::finalize_surface` calls
//! it for the `--ui=auto` selection. `context::resolve_surface` is a pure
//! reference mirror used by the precedence tests.

use crate::invocation::shape::{RouteKind, Shape};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Plain,
    Inline,
    Fullscreen,
    // Not produced by `base_surface`; an automation consumer is mapped to JSON
    // upstream in `finalize_surface`/`resolve_surface`, so this variant is only
    // constructed there.
    #[allow(dead_code)]
    Json,
}

/// Map a `(route, shape)` pair to its base interaction surface, before any
/// context-driven overrides are applied.
pub fn base_surface(route: RouteKind, shape: Shape) -> Surface {
    use RouteKind::*;
    use Shape::*;
    match (route, shape) {
        // Auth row: always plain.
        (Auth, _) => Surface::Plain,

        // Workflow row: always fullscreen.
        (Workflow, _) => Surface::Fullscreen,

        // Service / Builtin: shape-driven.
        (Service | Builtin, Form) => Surface::Inline,
        (Service | Builtin, Zero | Small | Static) => Surface::Plain,

        // Workflow w/ a non-Multi shape (1-step) already returned above; the
        // Multi shape is only reachable from Workflow.
        (_, Multi) => Surface::Fullscreen,

        // AuthSp shape only reachable from Auth route, handled above.
        (_, AuthSp) => Surface::Plain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_row_is_always_plain() {
        for shape in [Shape::Zero, Shape::Small, Shape::AuthSp] {
            assert_eq!(
                base_surface(RouteKind::Auth, shape),
                Surface::Plain,
                "shape = {shape:?}"
            );
        }
    }

    #[test]
    fn test_workflow_row_is_always_fullscreen() {
        for shape in [Shape::Zero, Shape::Small, Shape::Form, Shape::Multi] {
            assert_eq!(
                base_surface(RouteKind::Workflow, shape),
                Surface::Fullscreen,
                "shape = {shape:?}"
            );
        }
    }

    #[test]
    fn test_service_zero_and_small_are_plain() {
        assert_eq!(
            base_surface(RouteKind::Service, Shape::Zero),
            Surface::Plain
        );
        assert_eq!(
            base_surface(RouteKind::Service, Shape::Small),
            Surface::Plain
        );
    }

    #[test]
    fn test_service_form_is_inline() {
        assert_eq!(
            base_surface(RouteKind::Service, Shape::Form),
            Surface::Inline
        );
    }

    #[test]
    fn test_builtin_form_is_inline_static_is_plain() {
        assert_eq!(
            base_surface(RouteKind::Builtin, Shape::Form),
            Surface::Inline
        );
        assert_eq!(
            base_surface(RouteKind::Builtin, Shape::Static),
            Surface::Plain
        );
    }
}
