//! Interaction-shape classification.
//!
//! `classify_shape` is the live runtime path — `routes/service` calls it to
//! pick the surface. The route only ever constructs the `Service`/`Workflow`
//! `RouteKind`s and their reachable `Shape`s, so the `Auth`/`Builtin` arms,
//! the `AuthSp`/`Static` shapes, and the `AuthSubcommand`/`OutputOnly` flags
//! are exercised only by tests; the module-level allowance keeps those
//! still-modelled variants from warning rather than scattering per-item
//! attributes.
#![allow(dead_code)]

/// One of the four invocation routes (matches `invocation/routes/` on disk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteKind {
    Auth,
    Service,
    Workflow,
    Builtin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Zero,
    Small,
    Form,
    Multi,
    AuthSp, // special: auth login with OAuth browser callback
    Static, // output-only routes (help, list, describe)
}

/// Number of required missing inputs after flag resolution.
#[derive(Debug, Clone, Copy)]
pub struct MissingInputs {
    pub required_scalars: usize,
    pub has_body_field: bool,
}

/// Whether the auth login subcommand triggers the OAuth browser-callback flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSubcommand {
    /// `ags auth login` without `--grant client-credentials` — browser flow.
    LoginBrowser,
    /// `ags auth login --grant client-credentials` or `ags auth status`/`logout`.
    Other,
}

/// Whether a builtin/auth command emits output without prompting the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputOnly {
    Yes,
    No,
}

/// Classify a route-and-context into a [`Shape`].
pub fn classify_shape(
    route: RouteKind,
    missing: MissingInputs,
    step_count: usize,
    auth_sub: AuthSubcommand,
    output_only: OutputOnly,
) -> Shape {
    match route {
        RouteKind::Auth if auth_sub == AuthSubcommand::LoginBrowser => Shape::AuthSp,
        RouteKind::Auth => classify_service_like(missing),

        RouteKind::Workflow if step_count >= 2 => Shape::Multi,
        RouteKind::Workflow => classify_service_like(missing),

        RouteKind::Service => classify_service_like(missing),

        RouteKind::Builtin if output_only == OutputOnly::Yes => Shape::Static,
        RouteKind::Builtin => classify_service_like(missing),
    }
}

/// Classify a service-like route's interaction shape from its missing inputs
/// (body field or several required scalars → form; none → static; else small).
fn classify_service_like(missing: MissingInputs) -> Shape {
    if missing.has_body_field || missing.required_scalars >= 3 {
        Shape::Form
    } else if missing.required_scalars == 0 {
        Shape::Zero
    } else {
        Shape::Small
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_missing() -> MissingInputs {
        MissingInputs {
            required_scalars: 0,
            has_body_field: false,
        }
    }
    fn small_missing() -> MissingInputs {
        MissingInputs {
            required_scalars: 2,
            has_body_field: false,
        }
    }
    fn form_missing() -> MissingInputs {
        MissingInputs {
            required_scalars: 5,
            has_body_field: false,
        }
    }
    fn body_missing() -> MissingInputs {
        MissingInputs {
            required_scalars: 1,
            has_body_field: true,
        }
    }

    #[test]
    fn test_service_zero_when_no_missing_inputs_and_no_body() {
        let s = classify_shape(
            RouteKind::Service,
            no_missing(),
            0,
            AuthSubcommand::Other,
            OutputOnly::No,
        );
        assert_eq!(s, Shape::Zero);
    }

    #[test]
    fn test_service_small_for_one_or_two_scalars() {
        for n in [1, 2] {
            let m = MissingInputs {
                required_scalars: n,
                has_body_field: false,
            };
            assert_eq!(
                classify_shape(
                    RouteKind::Service,
                    m,
                    0,
                    AuthSubcommand::Other,
                    OutputOnly::No
                ),
                Shape::Small,
                "n = {n}"
            );
        }
    }

    #[test]
    fn test_service_form_for_three_or_more_scalars() {
        let m = MissingInputs {
            required_scalars: 3,
            has_body_field: false,
        };
        assert_eq!(
            classify_shape(
                RouteKind::Service,
                m,
                0,
                AuthSubcommand::Other,
                OutputOnly::No
            ),
            Shape::Form
        );
    }

    #[test]
    fn test_service_form_when_body_field_present_regardless_of_count() {
        assert_eq!(
            classify_shape(
                RouteKind::Service,
                body_missing(),
                0,
                AuthSubcommand::Other,
                OutputOnly::No
            ),
            Shape::Form
        );
    }

    #[test]
    fn test_workflow_multi_when_two_or_more_steps() {
        assert_eq!(
            classify_shape(
                RouteKind::Workflow,
                no_missing(),
                2,
                AuthSubcommand::Other,
                OutputOnly::No
            ),
            Shape::Multi
        );
    }

    #[test]
    fn test_single_step_workflow_treated_as_underlying_service_shape() {
        assert_eq!(
            classify_shape(
                RouteKind::Workflow,
                form_missing(),
                1,
                AuthSubcommand::Other,
                OutputOnly::No
            ),
            Shape::Form
        );
    }

    #[test]
    fn test_builtin_static_for_output_only_subcommands() {
        assert_eq!(
            classify_shape(
                RouteKind::Builtin,
                no_missing(),
                0,
                AuthSubcommand::Other,
                OutputOnly::Yes
            ),
            Shape::Static
        );
    }

    #[test]
    fn test_builtin_input_subcommand_falls_back_to_service_like() {
        // e.g. `ags profile create` with multiple required fields.
        assert_eq!(
            classify_shape(
                RouteKind::Builtin,
                form_missing(),
                0,
                AuthSubcommand::Other,
                OutputOnly::No
            ),
            Shape::Form
        );
    }

    #[test]
    fn test_auth_login_browser_is_authsp_shape() {
        assert_eq!(
            classify_shape(
                RouteKind::Auth,
                no_missing(),
                0,
                AuthSubcommand::LoginBrowser,
                OutputOnly::No
            ),
            Shape::AuthSp
        );
    }

    #[test]
    fn test_auth_status_treated_as_service_like_for_shape() {
        // `ags auth status` typically zero missing inputs.
        assert_eq!(
            classify_shape(
                RouteKind::Auth,
                no_missing(),
                0,
                AuthSubcommand::Other,
                OutputOnly::No
            ),
            Shape::Zero
        );
    }

    #[test]
    fn test_auth_login_client_credentials_treated_as_service_like() {
        // Prompts for client-id + client-secret = 2 scalars.
        assert_eq!(
            classify_shape(
                RouteKind::Auth,
                small_missing(),
                0,
                AuthSubcommand::Other,
                OutputOnly::No
            ),
            Shape::Small
        );
    }
}
