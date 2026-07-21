use crate::errors::CliError;
use crate::invocation::{flags, routes};

/// Parse and validate the raw `--page-limit` flag into `flags.page_limit`.
/// Shared by every self-owned execution path (`workflow run`, service, the
/// builtin route), so all paths reject an out-of-range limit
/// identically.
pub(crate) fn parse_page_limit(flags: &mut flags::GlobalFlags) -> Result<(), CliError> {
    if let Some(raw) = flags.page_limit_raw.take() {
        match raw.parse::<u64>() {
            Ok(limit) if (1..=100).contains(&limit) => flags.page_limit = Some(limit),
            _ => {
                return Err(CliError::Usage {
                    message: format!("Invalid page limit '{raw}'"),
                    metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                        "Use a value between 1 and 100",
                    ))),
                });
            }
        }
    }
    Ok(())
}

/// Which root invocation owns its own presentation surfaces.
///
/// This is the single authoritative classifier: `WorkflowRun` and `Service`
/// both run the self-owned, phase-aware lifecycle (constructing surfaces only
/// after their runtime prologue succeeds); `Auth` runs the bespoke (non-
/// executor) self-owned auth-path lifecycle; `Builtin` runs the builtin-route
/// lifecycle via [`routes::builtin::route_builtin`].
pub(crate) enum RootDispatch {
    Builtin,
    WorkflowRun,
    Service,
    /// The `auth` path, classified at the raw `auth` token so its
    /// bespoke wrapper owns frontend construction and decides — *after* clap
    /// parsing — whether a real `auth login` run lifecycle starts. The token
    /// alone cannot distinguish `auth login`, `auth login --help`, and
    /// `auth login --badflag`, so the parse/help/run decision is deferred. It
    /// is never dispatched by the builtin route.
    Auth,
}

/// Classify a root invocation from its post-prescan args.
///
/// The builtin-command name list MUST stay in sync with the commands the
/// builtin route dispatches; that single-sourcing is provided by
/// [`routes::builtin::is_builtin_command`], which this consults. `auth`
/// is NOT in that list — it classifies as [`RootDispatch::Auth`] and runs
/// its bespoke self-owned wrapper.
pub(crate) fn classify_root(remaining: &[String]) -> RootDispatch {
    if routes::workflow::is_workflow_run(remaining) {
        return RootDispatch::WorkflowRun;
    }
    let Some(first) = remaining.first() else {
        return RootDispatch::Builtin;
    };
    if first == "auth" {
        return RootDispatch::Auth;
    }
    if routes::builtin::is_builtin_command(first) {
        return RootDispatch::Builtin;
    }
    RootDispatch::Service
}

#[cfg(test)]
mod classify_root_tests {
    use super::{classify_root, RootDispatch};

    /// Build the post-prescan `remaining` slice from a list of tokens.
    fn remaining(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn test_classify_root_builtin_command_is_builtin() {
        for name in [
            "config",
            "profile",
            "describe",
            "doctor",
            "completions",
            "refresh-specs",
            "help",
        ] {
            assert!(
                matches!(classify_root(&remaining(&[name])), RootDispatch::Builtin),
                "'{name}' must classify as Builtin"
            );
        }
    }

    #[test]
    fn test_classify_root_auth_token_is_auth() {
        // The `auth` path classifies at the raw token: bare `auth` and
        // every subcommand/flag form classifies as `Auth`, never `Builtin`.
        for tokens in [
            vec!["auth"],
            vec!["auth", "login"],
            vec!["auth", "login", "--help"],
            vec!["auth", "login", "--badflag"],
            vec!["auth", "status"],
            vec!["auth", "logout"],
            vec!["auth", "--help"],
        ] {
            assert!(
                matches!(classify_root(&remaining(&tokens)), RootDispatch::Auth),
                "'{tokens:?}' must classify as Auth"
            );
        }
    }

    #[test]
    fn test_classify_root_workflow_run_is_workflow_run() {
        assert!(matches!(
            classify_root(&remaining(&["workflow", "run", "some-id"])),
            RootDispatch::WorkflowRun
        ));
    }

    #[test]
    fn test_classify_root_workflow_list_is_builtin() {
        assert!(matches!(
            classify_root(&remaining(&["workflow", "list"])),
            RootDispatch::Builtin
        ));
    }

    #[test]
    fn test_classify_root_bare_workflow_is_builtin() {
        assert!(matches!(
            classify_root(&remaining(&["workflow"])),
            RootDispatch::Builtin
        ));
    }

    #[test]
    fn test_classify_root_dynamic_service_token_is_service() {
        assert!(matches!(
            classify_root(&remaining(&["iam", "users", "list"])),
            RootDispatch::Service
        ));
    }

    #[test]
    fn test_classify_root_leading_long_help_is_builtin() {
        assert!(matches!(
            classify_root(&remaining(&["--help"])),
            RootDispatch::Builtin
        ));
    }

    #[test]
    fn test_classify_root_leading_short_help_is_builtin() {
        assert!(matches!(
            classify_root(&remaining(&["-h"])),
            RootDispatch::Builtin
        ));
    }

    #[test]
    fn test_classify_root_leading_unknown_flag_is_builtin() {
        // An unknown leading flag stays Builtin so `route` renders the usage
        // error through the top-level frontend, not the self-owned lifecycle.
        assert!(matches!(
            classify_root(&remaining(&["--foo"])),
            RootDispatch::Builtin
        ));
    }

    #[test]
    fn test_classify_root_empty_is_builtin() {
        assert!(matches!(classify_root(&[]), RootDispatch::Builtin));
    }

    #[test]
    fn test_classify_root_version_is_builtin() {
        assert!(matches!(
            classify_root(&remaining(&["version"])),
            RootDispatch::Builtin
        ));
    }
}

#[cfg(test)]
mod page_limit_tests {
    use super::parse_page_limit;
    use crate::errors::CliError;
    use crate::invocation::flags::GlobalFlags;

    #[test]
    fn test_parse_page_limit_accepts_in_range() {
        let mut flags = GlobalFlags {
            page_limit_raw: Some("50".to_string()),
            ..GlobalFlags::default()
        };
        parse_page_limit(&mut flags).expect("50 is in range");
        assert_eq!(flags.page_limit, Some(50));
        assert!(flags.page_limit_raw.is_none());
    }

    #[test]
    fn test_parse_page_limit_rejects_zero() {
        let mut flags = GlobalFlags {
            page_limit_raw: Some("0".to_string()),
            ..GlobalFlags::default()
        };
        assert!(matches!(
            parse_page_limit(&mut flags),
            Err(CliError::Usage { .. })
        ));
    }

    #[test]
    fn test_parse_page_limit_rejects_above_max() {
        let mut flags = GlobalFlags {
            page_limit_raw: Some("101".to_string()),
            ..GlobalFlags::default()
        };
        assert!(matches!(
            parse_page_limit(&mut flags),
            Err(CliError::Usage { .. })
        ));
    }

    #[test]
    fn test_parse_page_limit_rejects_non_numeric() {
        let mut flags = GlobalFlags {
            page_limit_raw: Some("abc".to_string()),
            ..GlobalFlags::default()
        };
        assert!(matches!(
            parse_page_limit(&mut flags),
            Err(CliError::Usage { .. })
        ));
    }

    #[test]
    fn test_parse_page_limit_noop_when_absent() {
        let mut flags = GlobalFlags::default();
        parse_page_limit(&mut flags).expect("no raw flag is fine");
        assert!(flags.page_limit.is_none());
    }
}
