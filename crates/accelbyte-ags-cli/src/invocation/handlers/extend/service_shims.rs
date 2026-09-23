//! Static registration table mapping an `ags extend` shortcut name to its
//! canonical `ags csm` service operation — either an `extend-helper-cli`
//! migration shortcut, or a newer command that simply has no logic beyond
//! a single API call (e.g. `security-assessment list`).
//!
//! This table is the SOLE place any such shortcut name is written as
//! a literal. The Clap subcommands, help text, and forwarding logic all
//! derive from it by iterating the slice. A rename costs one string edit
//! in one file — this one.

use clap::Command;

use crate::errors::CliError;

/// One migration shortcut entry mapping an `extend-helper-cli` invocation
/// to its canonical `ags csm` service operation.
// Public for integration-test access; not a supported API surface.
#[doc(hidden)]
pub struct ExtendShim {
    /// Command name as typed under `ags extend` (or under a parent subgroup).
    pub name: &'static str,
    /// Parent subgroup under `extend`, if any. `None` for top-level entries.
    pub parent: Option<&'static str>,
    /// The historical `extend-helper-cli` invocation this shim preserves,
    /// WITHOUT the program name. For example `"create-app"` or
    /// `"appui create"`. Exists for discoverability in `--help` output
    /// only — routing never uses it.
    pub go_invocation: &'static str,
    /// Hand-written product copy shown in `--help` next to the command name.
    ///
    /// This is intentionally NOT derived from the bundled CSM spec: the
    /// spec's operation summaries are written for API operations and read
    /// wrong next to customer-facing commands like "Clone a starter
    /// template for Extend apps". Hand-written summaries also prevent
    /// user-facing help text from changing silently when a spec is
    /// refreshed.
    pub summary: &'static str,
    /// Canonical service id (always `"csm"` for Phase 1).
    pub service: &'static str,
    /// Canonical resource name.
    pub resource: &'static str,
    /// Canonical method name.
    pub method: &'static str,
    /// Optional async-wait target. `Some` for app lifecycle commands that
    /// support `--wait` (poll until the app reaches a terminal state);
    /// `None` for shims that return as soon as the API call completes.
    pub(crate) wait: Option<&'static super::app_lifecycle::wait::WaitSpec>,
    /// Flag renames applied when forwarding user-supplied args: pairs of
    /// `(extend-facing long flag, canonical operation flag)`, each without
    /// the leading `--`. Lets a shim keep an established `ags extend`-
    /// family flag name (e.g. `--app`) even when the underlying operation's
    /// own kebab-cased parameter name differs (e.g. `--app-name` for an
    /// `appName` parameter). Empty when the shim's flags already match the
    /// canonical operation's flag names verbatim.
    pub arg_renames: &'static [(&'static str, &'static str)],
}

/// The static registration table. Every extend migration shortcut is
/// defined here and nowhere else — all consumers iterate this slice.
// Public for integration-test access; not a supported API surface.
#[doc(hidden)]
pub static SHIMS: &[ExtendShim] = &[
    ExtendShim {
        name: "create-app",
        parent: None,
        go_invocation: "create-app",
        summary: "Create an Extend app",
        service: "csm",
        resource: "apps",
        method: "create",
        wait: Some(&super::app_lifecycle::wait::CREATE_APP_WAIT),
        arg_renames: &[],
    },
    ExtendShim {
        name: "get-app-info",
        parent: None,
        go_invocation: "get-app-info",
        summary: "Show Extend app details",
        service: "csm",
        resource: "apps",
        method: "get",
        wait: None,
        arg_renames: &[],
    },
    ExtendShim {
        name: "list-images",
        parent: None,
        go_invocation: "list-images",
        summary: "List container images for an Extend app",
        service: "csm",
        resource: "images",
        method: "list",
        wait: None,
        arg_renames: &[],
    },
    ExtendShim {
        name: "deploy-app",
        parent: None,
        go_invocation: "deploy-app",
        summary: "Deploy an Extend app",
        service: "csm",
        resource: "deployments",
        method: "create",
        wait: Some(&super::app_lifecycle::wait::DEPLOY_APP_WAIT),
        arg_renames: &[],
    },
    ExtendShim {
        name: "start-app",
        parent: None,
        go_invocation: "start-app",
        summary: "Start an Extend app",
        service: "csm",
        resource: "apps",
        method: "start",
        wait: Some(&super::app_lifecycle::wait::START_APP_WAIT),
        arg_renames: &[],
    },
    ExtendShim {
        name: "stop-app",
        parent: None,
        go_invocation: "stop-app",
        summary: "Stop an Extend app",
        service: "csm",
        resource: "apps",
        method: "stop",
        wait: Some(&super::app_lifecycle::wait::STOP_APP_WAIT),
        arg_renames: &[],
    },
    ExtendShim {
        name: "delete-app",
        parent: None,
        go_invocation: "delete-app",
        summary: "Delete an Extend app",
        service: "csm",
        resource: "apps",
        method: "delete",
        wait: Some(&super::app_lifecycle::wait::DELETE_APP_WAIT),
        arg_renames: &[],
    },
    ExtendShim {
        name: "create",
        parent: Some("app-ui"),
        go_invocation: "appui create",
        summary: "Create an Extend app UI",
        service: "csm",
        resource: "app-ui",
        method: "create",
        wait: None,
        arg_renames: &[],
    },
    ExtendShim {
        name: "list",
        parent: Some("security-assessment"),
        // No historical Go invocation — this is a new command, not a
        // migration shortcut. Equal to its own extend address so
        // `shim_about` omits the "(was: ...)" suffix.
        go_invocation: "security-assessment list",
        summary: "List security-assessment engagements for a namespace",
        service: "csm",
        resource: "security-assessment",
        method: "list",
        wait: None,
        arg_renames: &[],
    },
    ExtendShim {
        name: "list-endpoints",
        parent: Some("security-assessment"),
        go_invocation: "security-assessment list-endpoints",
        summary: "Discover an Extend app's testable endpoints and required permissions",
        service: "csm",
        resource: "security-assessment",
        method: "get-app-endpoints",
        wait: None,
        // The get-app-endpoints operation's own parameter is `appName`
        // (kebab-cased to `--app-name` by the generic dynamic command
        // builder), but every other `security-assessment` command uses
        // `--app` — keep that family convention here too.
        arg_renames: &[("app", "app-name")],
    },
];

/// The path segments under `extend` for a shim. The caller prepends the
/// `"extend"` root to form a full path.
pub(crate) fn shim_path_segments(shim: &ExtendShim) -> Vec<String> {
    match shim.parent {
        Some(parent) => vec![parent.to_string(), shim.name.to_string()],
        None => vec![shim.name.to_string()],
    }
}

/// Unique parent-group names from the SHIMS table, deduplicated and sorted.
///
/// Used by the describe handler (suggestion builder), the collision guard,
/// and describe tests. Extracting this derivation once prevents the
/// BTreeSet-over-filter_map pattern from being duplicated at every call site.
pub(crate) fn parent_group_names() -> std::collections::BTreeSet<&'static str> {
    SHIMS.iter().filter_map(|s| s.parent).collect()
}

/// Test-only: return the canonical (non-shim) top-level extend subcommand
/// names by enumerating the real clap tree. Shim subcommands and their
/// parent groups are excluded so this set contains only hand-written
/// commands (`clone-template`, `docker-login`, etc.) — it tracks reality
/// as commands are added or removed without a hand-maintained list.
///
/// Used by the collision-guard and non-emptiness tests; production callers
/// in describe each build the tree and derive names inline.
#[cfg(test)]
fn canonical_extend_subcommand_names() -> Vec<String> {
    let shim_top_level: std::collections::BTreeSet<&str> = SHIMS
        .iter()
        .filter_map(|s| {
            if s.parent.is_none() {
                Some(s.name)
            } else {
                None
            }
        })
        .collect();
    let parents = parent_group_names();
    let cmd = crate::invocation::builder::build_extend_command();
    cmd.get_subcommands()
        .filter(|sub| !sub.is_hide_set())
        .map(|sub| sub.get_name().to_string())
        .filter(|name| !shim_top_level.contains(name.as_str()) && !parents.contains(name.as_str()))
        .collect()
}

/// Test-only: resolve a shim's (resource, method) pair against a service
/// schema. Returns `Ok(())` when the resource exists and carries the named
/// method. Returns `Err` with a diagnostic message describing the miss.
///
/// Used by the live-schema guard and its negative-coverage siblings to
/// verify both resolution directions without relying on panic-message
/// string matching.
#[cfg(test)]
fn resolve_shim_triple(
    schema: &ags_protocol::catalogue::ServiceSchema,
    shim: &ExtendShim,
) -> Result<(), String> {
    let resource = schema
        .resources
        .iter()
        .find(|r| r.name == shim.resource)
        .ok_or_else(|| {
            let available: Vec<&str> = schema.resources.iter().map(|r| r.name.as_str()).collect();
            format!(
                "schema has no resource '{}' (referenced by shim '{}'); \
                 available resources: {available:?}",
                shim.resource, shim.name,
            )
        })?;

    if !resource.methods.iter().any(|m| m.name == shim.method) {
        let available: Vec<&str> = resource.methods.iter().map(|m| m.name.as_str()).collect();
        return Err(format!(
            "resource '{}' has no method '{}' (referenced by shim '{}'); \
             available methods: {available:?}",
            shim.resource, shim.method, shim.name,
        ));
    }

    Ok(())
}

/// The result of rewriting a shim invocation: the service-dispatch arguments
/// (with any `--wait-*` flags stripped), an optional resolved wait request for
/// the lifecycle commands that support `--wait`, and the shortcut presentation
/// used to render the shim's own `--help` page.
pub(crate) struct ShimDispatch {
    pub(crate) service_args: Vec<String>,
    pub(crate) wait: Option<super::app_lifecycle::WaitRequest>,
    pub(crate) presentation: ShimPresentation,
}

/// Presentation metadata for a matched shim, carried from the shim match
/// through to the help printer so the shortcut's own help page can be
/// rendered instead of the canonical CSM operation's page.
// Public for integration-test access; not a supported API surface.
#[doc(hidden)]
pub struct ShimPresentation {
    /// Hand-written summary (first line of the shortcut help page).
    pub summary: &'static str,
    /// The display address the user typed, e.g. `"ags extend deploy-app"`.
    pub display_address: String,
    /// The canonical `ags <service> <resource> <method>` address.
    pub canonical_address: String,
    /// True when the shim supports `--wait`. The help printer adds the
    /// `--wait*` flags to the shortcut's help page for these
    /// (see `routes/service/help.rs::apply_shim_overrides`).
    pub wait_capable: bool,
}

/// Match `remaining` against the shim table. Returns the matched shim and the
/// user args that follow the shim address (`None` when not a shim invocation,
/// e.g. `extend --help`, `extend clone-template`, `extend no-such-command`).
fn match_shim(remaining: &[String]) -> Option<(&'static ExtendShim, &[String])> {
    if remaining.first().map(String::as_str) != Some("extend") {
        return None;
    }
    let extend_args = &remaining[1..];
    if extend_args.is_empty() {
        return None;
    }

    let first = extend_args[0].as_str();

    // Top-level shims: `extend <name> [flags...]`
    for shim in SHIMS {
        if shim.parent.is_none() && shim.name == first {
            return Some((shim, &extend_args[1..]));
        }
    }

    // Subgroup shims: `extend <parent> <name> [flags...]`
    if extend_args.len() >= 2 {
        let second = extend_args[1].as_str();
        for shim in SHIMS {
            if shim.parent == Some(first) && shim.name == second {
                return Some((shim, &extend_args[2..]));
            }
        }
    }

    None
}

/// If `remaining` starts with `"extend"` followed by a registered shim
/// address, return the rewritten dispatch (service args + optional wait).
/// Returns `None` when the invocation is not a shim, or `Some(Err(..))` when
/// it is a shim but the `--wait-*` flag values are malformed.
pub(crate) fn try_rewrite_shim(remaining: &[String]) -> Option<Result<ShimDispatch, CliError>> {
    let (shim, user_args) = match_shim(remaining)?;
    Some(build_dispatch(shim, user_args))
}

/// If `remaining` starts with `"extend"` followed by a registered shim
/// address, return the rewritten service arguments for `run_service`.
/// Returns `None` when the invocation is not a shim.
///
/// Convenience wrapper over [`try_rewrite_shim`] for callers that only need
/// the service address (collision guards, tests). The `--wait-*` flags are
/// still stripped; a malformed wait value collapses to `None` here since those
/// callers never pass wait flags.
#[cfg(test)]
pub(crate) fn try_rewrite_to_service_args(
    remaining: &[String],
) -> Option<(Vec<String>, ShimPresentation)> {
    match try_rewrite_shim(remaining) {
        Some(Ok(dispatch)) => Some((dispatch.service_args, dispatch.presentation)),
        _ => None,
    }
}

/// Build the dispatch for a matched shim. For wait-capable shims, the
/// `--wait` / `--wait-interval` / `--wait-limit` flags are parsed and stripped
/// from the forwarded args (the generic service Clap tree would reject them),
/// and `--app` / `-a` is captured as the poll target.
///
/// The `--wait` status poll uses a fixed `/csm/v5/` endpoint (see
/// [`super::app_lifecycle::api::get_app_status`]) and therefore ignores
/// `--api-scope` / `--api-version`, even though the primary operation built here
/// honours them. The two can diverge — an explicit `--api-version`, or a stale
/// parse cache resolving the operation to v2, leaves the operation on v2 while
/// the poll stays v5 (benign in practice, same record). See the reference doc's
/// `--wait` caveat and the follow-up to resolve the poll through the spec path.
fn build_dispatch(
    shim: &'static ExtendShim,
    user_args: &[String],
) -> Result<ShimDispatch, CliError> {
    let Some(spec) = shim.wait else {
        return Ok(ShimDispatch {
            service_args: build_service_args(shim, user_args),
            wait: None,
            presentation: shim_presentation(shim),
        });
    };

    let parsed = parse_wait_flags(user_args)?;
    let wait = if parsed.wait {
        let app = parsed.app.clone().ok_or_else(|| CliError::Usage {
            message: "--wait requires --app to identify the app to poll".to_string(),
            metadata: None,
        })?;
        Some(super::app_lifecycle::WaitRequest {
            spec,
            app,
            interval_secs: parsed.interval,
            limit_secs: parsed.limit,
            // Populated after the primary call returns, from its create
            // response — see the shim dispatch in `invocation::run`.
            expected_deployment_id: None,
        })
    } else {
        None
    };

    Ok(ShimDispatch {
        service_args: build_service_args(shim, &parsed.rest),
        wait,
        presentation: shim_presentation(shim),
    })
}

/// Defaults mirror `extend-helper-cli`: poll every 10s, up to 600s.
const DEFAULT_WAIT_INTERVAL_SECS: u64 = 10;
const DEFAULT_WAIT_LIMIT_SECS: u64 = 600;

/// Parsed `--wait-*` state plus the args to forward to the service dispatch.
struct ParsedWaitFlags {
    wait: bool,
    interval: u64,
    limit: u64,
    /// The `--app` / `-a` value, captured for polling (still forwarded too).
    app: Option<String>,
    /// User args with the `--wait-*` flags removed.
    rest: Vec<String>,
}

/// Strip and parse the `--wait-*` flags out of a shim's user args. `--app` /
/// `-a` is captured but left in `rest` so the primary call still receives it.
fn parse_wait_flags(user_args: &[String]) -> Result<ParsedWaitFlags, CliError> {
    let mut wait = false;
    let mut interval = DEFAULT_WAIT_INTERVAL_SECS;
    let mut interval_explicit = false;
    let mut limit = DEFAULT_WAIT_LIMIT_SECS;
    let mut app: Option<String> = None;
    let mut rest: Vec<String> = Vec::with_capacity(user_args.len());

    let parse_secs = |flag: &str, raw: &str| -> Result<u64, CliError> {
        raw.parse::<u64>().map_err(|_| CliError::Usage {
            message: format!("{flag} expects a non-negative integer, got '{raw}'"),
            metadata: None,
        })
    };

    let mut i = 0;
    while i < user_args.len() {
        let arg = user_args[i].as_str();
        if arg == "--wait" {
            wait = true;
        } else if arg == "--wait-interval" || arg == "--wait-limit" {
            let raw = user_args.get(i + 1).ok_or_else(|| CliError::Usage {
                message: format!("{arg} requires a value"),
                metadata: None,
            })?;
            let secs = parse_secs(arg, raw)?;
            if arg == "--wait-interval" {
                interval = secs;
                interval_explicit = true;
            } else {
                limit = secs;
            }
            i += 1; // consume the value token (not forwarded)
        } else if let Some(raw) = arg.strip_prefix("--wait-interval=") {
            interval = parse_secs("--wait-interval", raw)?;
            interval_explicit = true;
        } else if let Some(raw) = arg.strip_prefix("--wait-limit=") {
            limit = parse_secs("--wait-limit", raw)?;
        } else {
            // Capture the app name for polling, but keep the flag+value in the
            // forwarded args so the primary service call still receives it.
            if arg == "--app" || arg == "-a" {
                if let Some(v) = user_args.get(i + 1) {
                    app = Some(v.clone());
                }
            } else if let Some(v) = arg.strip_prefix("--app=") {
                app = Some(v.to_string());
            }
            rest.push(user_args[i].clone());
        }
        i += 1;
    }

    if interval == 0 {
        return Err(CliError::Usage {
            message: "--wait-interval must be greater than 0".to_string(),
            metadata: None,
        });
    }

    // A 0 limit would enter the poll loop zero times and report an instant
    // timeout without ever polling. Reject it rather than "wait" for nothing.
    if limit == 0 {
        return Err(CliError::Usage {
            message: "--wait-limit must be greater than 0".to_string(),
            metadata: None,
        });
    }

    // The loop sleeps a full interval before checking the clock, so an interval
    // larger than the limit would block past the promised maximum wait (e.g.
    // interval 300 / limit 30 sleeps 300s). Reject it; equal is fine — a single
    // poll at exactly the limit.
    if interval > limit {
        // Name the default explicitly — the common way to hit this is passing a
        // small `--wait-limit` while `--wait-interval` is still its default 10s.
        let interval_display = if interval_explicit {
            format!("{interval}s")
        } else {
            format!("{interval}s, the default")
        };
        return Err(CliError::Usage {
            message: format!(
                "--wait-interval ({interval_display}) must not be greater than \
                 --wait-limit ({limit}s); pass a smaller --wait-interval or a larger --wait-limit"
            ),
            metadata: None,
        });
    }

    Ok(ParsedWaitFlags {
        wait,
        interval,
        limit,
        app,
        rest,
    })
}

/// Build a [`ShimPresentation`] from a matched shim entry.
fn shim_presentation(shim: &ExtendShim) -> ShimPresentation {
    ShimPresentation {
        summary: shim.summary,
        display_address: match shim.parent {
            Some(parent) => format!("ags extend {} {}", parent, shim.name),
            None => format!("ags extend {}", shim.name),
        },
        canonical_address: format!("ags {} {} {}", shim.service, shim.resource, shim.method),
        wait_capable: shim.wait.is_some(),
    }
}

/// Construct the service dispatch arguments from a matched shim entry
/// and the user's remaining flags. User-supplied flags are forwarded after
/// the service/resource/method triple, renamed per `shim.arg_renames`.
fn build_service_args(shim: &ExtendShim, user_args: &[String]) -> Vec<String> {
    let mut args = vec![
        shim.service.to_string(),
        shim.resource.to_string(),
        shim.method.to_string(),
    ];
    args.extend(rename_args(user_args, shim.arg_renames));
    args
}

/// Rewrite each `--<from>` (or `--<from>=value`) flag in `args` to
/// `--<to>`, per `renames`. Every other token — including the flag's own
/// value, when given as a separate argv element — passes through
/// unchanged. A no-op when `renames` is empty.
fn rename_args(args: &[String], renames: &[(&str, &str)]) -> Vec<String> {
    if renames.is_empty() {
        return args.to_vec();
    }
    args.iter()
        .map(|arg| {
            for (from, to) in renames {
                if arg == &format!("--{from}") {
                    return format!("--{to}");
                }
                if let Some(value) = arg.strip_prefix(&format!("--{from}=")) {
                    return format!("--{to}={value}");
                }
            }
            arg.clone()
        })
        .collect()
}

/// Build the `about` string for a shim subcommand.
///
/// Format: `{summary} (\u{2192} ags {service} {resource} {method})`
///
/// When the historical Go invocation differs from the shim's `ags extend`
/// address (e.g. `appui create` vs `app-ui create`), a
/// `(was: {go_invocation})` suffix is appended so migrating customers can
/// find their old command name. The condition is derived from the data, not
/// hard-coded to a specific shim.
fn shim_about(shim: &ExtendShim) -> String {
    let base = format!(
        "{} (\u{2192} ags {} {} {})",
        shim.summary, shim.service, shim.resource, shim.method
    );
    let extend_addr = match shim.parent {
        Some(parent) => format!("{parent} {}", shim.name),
        None => shim.name.to_string(),
    };
    if shim.go_invocation != extend_addr {
        format!("{base} (was: {})", shim.go_invocation)
    } else {
        base
    }
}

/// Add migration shortcut subcommands to the extend Clap tree by iterating
/// the registration table. Shim subcommands are registered as visible clap
/// subcommands and listed in the `Commands:` section alongside the
/// canonical commands (`clone-template`, `docker-login`, etc.). Each
/// shim's `about` shows its hand-written summary followed by the
/// canonical `ags csm` address.
///
/// Parent subgroups (e.g. `app-ui`, `remote-debug`) are registered here
/// as hidden placeholders. The caller promotes them to visible via
/// `mut_subcommand` and adds native subcommands (e.g. `setup-env`,
/// `connect`, `enable`).
pub(crate) fn add_shim_subcommands(mut cmd: Command) -> Command {
    // Register top-level shim subcommands as visible entries in the
    // clap-generated Commands: section.
    for shim in SHIMS {
        if shim.parent.is_none() {
            cmd = cmd.subcommand(with_wait_args(
                Command::new(shim.name)
                    .about(shim_about(shim))
                    .disable_help_flag(true)
                    .hide(false),
                shim,
            ));
        }
    }

    // Group child shims by parent and register parent subgroups.
    let mut subgroups: std::collections::BTreeMap<&str, Vec<&ExtendShim>> =
        std::collections::BTreeMap::new();
    for shim in SHIMS {
        if let Some(parent) = shim.parent {
            subgroups.entry(parent).or_default().push(shim);
        }
    }

    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    for (parent_name, children) in &subgroups {
        let mut sub = Command::new(*parent_name)
            .help_template(template.clone())
            .about(format!("{parent_name} commands"))
            .disable_help_subcommand(true)
            .arg_required_else_help(true)
            .subcommand_required(true);

        for shim in children {
            sub = sub.subcommand(with_wait_args(
                Command::new(shim.name)
                    .about(shim_about(shim))
                    .disable_help_flag(true)
                    .hide(false),
                shim,
            ));
        }

        cmd = cmd.subcommand(sub.hide(true));
    }

    cmd
}

/// Register the `--wait` / `--wait-interval` / `--wait-limit` flags on a
/// wait-capable shim command so they appear in `--help` and shell completions.
///
/// The flags are actually consumed by [`parse_wait_flags`], which scans argv
/// before the generic service dispatch — clap on the shim command never parses
/// them at runtime (the shim routing intercepts first). Declaring them here is
/// purely for discoverability: without it the three flags are silently accepted
/// by the pre-clap scanner but invisible in `--help` and offer no completions.
/// A no-op for shims that do not support `--wait` (`shim.wait` is `None`).
fn with_wait_args(command: Command, shim: &ExtendShim) -> Command {
    if shim.wait.is_none() {
        return command;
    }
    add_wait_flag_args(command)
}

/// Add the `--wait` / `--wait-interval` / `--wait-limit` flags to a clap
/// command so they appear in `--help` and shell completions. Shared by the
/// extend command tree (`with_wait_args`, for completions) and the shortcut
/// help-page override (`routes/service/help.rs::apply_shim_overrides`), so the
/// wording and shape stay in one place.
pub(crate) fn add_wait_flag_args(command: Command) -> Command {
    command
        .arg(
            clap::Arg::new("wait")
                .long("wait")
                .action(clap::ArgAction::SetTrue)
                .help("Wait until the app reaches a terminal state before returning"),
        )
        .arg(
            clap::Arg::new("wait-interval")
                .long("wait-interval")
                .value_name("SECONDS")
                .help("Seconds between status polls while waiting (default 10)"),
        )
        .arg(
            clap::Arg::new("wait-limit")
                .long("wait-limit")
                .value_name("SECONDS")
                .help("Maximum seconds to wait before giving up (default 600)"),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build `remaining` from string slices.
    fn remaining(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|s| s.to_string()).collect()
    }

    /// Strip ANSI escape sequences from a string for assertion checks.
    /// Help text may contain ANSI styling codes when color is enabled;
    /// stripping ensures tests compare semantic content, not styling.
    fn strip_ansi_codes(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            if let Some('[') = chars.next() {
                for inner in chars.by_ref() {
                    if inner.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        }
        out
    }

    // ── Routing parity (Test Plan 1) ──

    #[test]
    fn every_shim_rewrites_to_correct_service_triple() {
        for shim in SHIMS {
            let mut input = vec!["extend"];
            if let Some(parent) = shim.parent {
                input.push(parent);
            }
            input.push(shim.name);
            // Append a flag to verify it passes through.
            input.push("--namespace");
            input.push("test-ns");

            let (result, presentation) = try_rewrite_to_service_args(&remaining(&input))
                .unwrap_or_else(|| {
                    panic!("shim '{}' (parent {:?}) must match", shim.name, shim.parent)
                });

            assert_eq!(
                result[0], shim.service,
                "shim '{}': service mismatch",
                shim.name
            );
            assert_eq!(
                result[1], shim.resource,
                "shim '{}': resource mismatch",
                shim.name
            );
            assert_eq!(
                result[2], shim.method,
                "shim '{}': method mismatch",
                shim.name
            );

            // User flags are forwarded after the triple.
            assert!(
                result.contains(&"--namespace".to_string()),
                "shim '{}': user flags must be forwarded",
                shim.name
            );
            assert!(
                result.contains(&"test-ns".to_string()),
                "shim '{}': user flag value must be forwarded",
                shim.name
            );

            // Presentation carries the correct addresses.
            assert_eq!(
                presentation.summary, shim.summary,
                "shim '{}': presentation summary mismatch",
                shim.name
            );
            let expected_canonical =
                format!("ags {} {} {}", shim.service, shim.resource, shim.method);
            assert_eq!(
                presentation.canonical_address, expected_canonical,
                "shim '{}': canonical address mismatch",
                shim.name
            );
        }
    }

    // ── Canonical address unchanged (Test Plan 4) ──

    #[test]
    fn canonical_csm_address_not_intercepted() {
        // For every triple in the table, verify that the canonical
        // `ags csm <resource> <method>` invocation is NOT intercepted
        // by the shim layer (it does not start with "extend").
        for shim in SHIMS {
            let canonical = remaining(&[shim.service, shim.resource, shim.method]);
            assert!(
                try_rewrite_to_service_args(&canonical).is_none(),
                "canonical address 'ags {} {} {}' must not be intercepted",
                shim.service,
                shim.resource,
                shim.method
            );
        }
    }

    // ── --help visibility (Test Plan 8, 9) ──

    /// Every shim name appears in the appropriate help page: top-level
    /// shims in `extend --help`, subgroup shims in their parent's help.
    #[test]
    fn extend_help_shows_all_shim_names_in_correct_context() {
        let mut cmd = crate::invocation::builder::build_extend_command();
        let extend_help = cmd.render_long_help().to_string();

        for shim in SHIMS {
            if shim.parent.is_none() {
                // Top-level shims appear in `extend --help`.
                assert!(
                    extend_help.contains(shim.name),
                    "extend help must list top-level shim '{}'\n{extend_help}",
                    shim.name
                );
            }
        }

        // Subgroup shims appear in their parent's --help.
        let subcommands: Vec<_> = cmd.get_subcommands().collect();
        for shim in SHIMS {
            if let Some(parent) = shim.parent {
                let parent_cmd = subcommands
                    .iter()
                    .find(|c| c.get_name() == parent)
                    .unwrap_or_else(|| panic!("extend must have a '{parent}' subcommand"));
                let parent_help = (*parent_cmd).clone().render_help().to_string();
                assert!(
                    parent_help.contains(shim.name),
                    "{parent} help must list shim '{}'\n{parent_help}",
                    shim.name
                );
            }
        }
    }

    // ── CSM help unaffected (Test Plan 10) ──

    #[test]
    fn csm_help_unaffected_by_extend_shims() {
        // Build the extend Clap tree and verify no shim name leaks into
        // a CSM-like context. Structural check: the extend help must
        // NOT contain "csm" as a top-level subcommand name — the shim
        // entries reference CSM in their about text only.
        let cmd = crate::invocation::builder::build_extend_command();
        let subs: Vec<String> = cmd
            .get_subcommands()
            .map(|c| c.get_name().to_string())
            .collect();

        // No subcommand should be named after a CSM resource.
        for shim in SHIMS {
            if shim.parent.is_none() {
                assert!(
                    !subs.contains(&shim.resource.to_string()),
                    "extend must not have a subcommand named '{}' (CSM resource)",
                    shim.resource
                );
            }
        }
    }

    // ── Collision guard (Test Plan 11) ──

    #[test]
    fn no_shim_name_collides_with_canonical_extend_subcommands() {
        let canonical_names = canonical_extend_subcommand_names();

        // Top-level shim names must not collide with canonical subcommands.
        for shim in SHIMS {
            if shim.parent.is_none() {
                assert!(
                    !canonical_names.contains(&shim.name.to_string()),
                    "shim name '{}' collides with canonical extend subcommand",
                    shim.name
                );
            }
        }

        // Subgroup parent names are registered at the same Clap namespace
        // level as top-level shims; they must not collide either — unless the
        // parent group was intentionally promoted to a visible canonical
        // subcommand by `build_extend_command` (e.g. `app-ui` hosts both the
        // `create` migration shim and the native `setup-env` command).
        let parent_names: Vec<String> = parent_group_names()
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        assert!(
            !parent_names.is_empty(),
            "parent_names must not be empty; the SHIMS table must contain \
             at least one entry with a parent subgroup"
        );
        // Parents that appear as visible subcommands were promoted intentionally.
        let promoted_parents: Vec<&String> = parent_names
            .iter()
            .filter(|p| canonical_names.contains(p))
            .collect();
        for parent in &parent_names {
            if promoted_parents.contains(&parent) {
                continue;
            }
            assert!(
                !canonical_names.contains(parent),
                "subgroup parent name '{}' collides with canonical extend subcommand",
                parent
            );
        }

        // Reverse direction: no canonical name matches a shim or parent name.
        // Promoted parent groups are excluded — they share the Clap subcommand
        // intentionally.
        let all_shim_level_names: Vec<String> = SHIMS
            .iter()
            .filter(|s| s.parent.is_none())
            .map(|s| s.name.to_string())
            .chain(
                parent_names
                    .iter()
                    .filter(|p| !promoted_parents.contains(p))
                    .cloned(),
            )
            .collect();
        for canonical in &canonical_names {
            assert!(
                !all_shim_level_names.contains(canonical),
                "canonical extend subcommand '{}' collides with a shim or parent name",
                canonical
            );
        }
    }

    /// The derived canonical list actually contains real commands. An
    /// empty derived set would make the collision guard vacuously true,
    /// defeating its purpose.
    #[test]
    fn canonical_names_derived_from_clap_tree_are_non_empty() {
        let names = canonical_extend_subcommand_names();
        assert!(
            !names.is_empty(),
            "derived canonical list must not be empty; enumeration returned nothing"
        );
        assert!(
            names.iter().any(|n| n == "clone-template"),
            "derived canonical list must include 'clone-template'; got: {names:?}"
        );
    }

    // ── Spec-regeneration guard (Test Plan 12) ──

    /// Validate every shim triple against the LIVE bundled CSM service schema
    /// — the same source `ags csm` uses to build its command tree. If the CSM
    /// spec is regenerated and a referenced resource or method is renamed or
    /// removed, this test fails immediately without requiring a separate
    /// fixture update.
    #[test]
    fn shim_triples_all_resolve_in_live_csm_schema() {
        use ags_runtime::catalogue::Catalogue;

        let csm_schema = Catalogue::load_bundled("csm")
            .expect("bundled CSM spec must load — this is the same path `ags csm` uses");

        // Guard against vacuous success: the schema must expose resources,
        // exactly as `canonical_names_derived_from_clap_tree_are_non_empty`
        // guards the collision check.
        assert!(
            !csm_schema.resources.is_empty(),
            "CSM schema must have at least one resource; load returned an empty set"
        );

        let mut resolved_count: usize = 0;

        for shim in SHIMS {
            resolve_shim_triple(&csm_schema, shim).unwrap_or_else(|msg| {
                panic!(
                    "shim '{}' did not resolve in the live CSM schema: {msg}",
                    shim.name,
                )
            });
            resolved_count += 1;
        }

        // Non-vacuity: at least as many triples resolved as the table has entries.
        assert_eq!(
            resolved_count,
            SHIMS.len(),
            "every shim triple must resolve; got {resolved_count}/{} resolved",
            SHIMS.len(),
        );
    }

    /// The resolution guard must actually fail when a shim references a
    /// resource that does not exist in the schema. Without this negative
    /// test, a refactor that short-circuits `resolve_shim_triple` to always
    /// return `Ok` would leave the positive test green.
    #[test]
    fn resolve_rejects_absent_resource() {
        use ags_runtime::catalogue::Catalogue;

        let csm_schema = Catalogue::load_bundled("csm").expect("bundled CSM spec must load");

        let bad_shim = ExtendShim {
            name: "fake-shim",
            parent: None,
            go_invocation: "fake-shim",
            summary: "Fake shim for testing",
            service: "csm",
            resource: "no-such-resource",
            method: "list",
            wait: None,
            arg_renames: &[],
        };

        let result = resolve_shim_triple(&csm_schema, &bad_shim);
        assert!(
            result.is_err(),
            "resolve must fail for an absent resource; got Ok"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("no-such-resource"),
            "error message must name the missing resource; got: {msg}"
        );
    }

    /// The resolution guard must fail when the resource exists but does not
    /// carry the requested method — the subtler of the two miss modes.
    #[test]
    fn resolve_rejects_absent_method_on_existing_resource() {
        use ags_runtime::catalogue::Catalogue;

        let csm_schema = Catalogue::load_bundled("csm").expect("bundled CSM spec must load");

        // Pick a resource that genuinely exists in the CSM schema (the first
        // shim entry's resource) but pair it with a method name that does not.
        let real_resource = SHIMS[0].resource;
        let bad_shim = ExtendShim {
            name: "fake-shim",
            parent: None,
            go_invocation: "fake-shim",
            summary: "Fake shim for testing",
            service: "csm",
            resource: real_resource,
            method: "no-such-method",
            wait: None,
            arg_renames: &[],
        };

        let result = resolve_shim_triple(&csm_schema, &bad_shim);
        assert!(
            result.is_err(),
            "resolve must fail for an absent method on resource '{real_resource}'; got Ok"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("no-such-method"),
            "error message must name the missing method; got: {msg}"
        );
    }

    // ── Unknown-command error (Test Plan 13) ──

    #[test]
    fn unknown_extend_address_returns_none() {
        let args = remaining(&["extend", "no-such-command"]);
        assert!(
            try_rewrite_to_service_args(&args).is_none(),
            "unknown extend address must not match any shim"
        );
    }

    #[test]
    fn extend_bare_returns_none() {
        assert!(try_rewrite_to_service_args(&remaining(&["extend"])).is_none());
    }

    #[test]
    fn non_extend_prefix_returns_none() {
        assert!(try_rewrite_to_service_args(&remaining(&["csm", "apps", "create"])).is_none());
    }

    #[test]
    fn extend_help_returns_none() {
        assert!(try_rewrite_to_service_args(&remaining(&["extend", "--help"])).is_none());
    }

    /// A shim forwards a user's `--json` through unchanged.
    #[test]
    fn shim_passes_json_through() {
        let shim = SHIMS
            .iter()
            .find(|s| s.parent.is_none())
            .expect("at least one top-level shim");
        let args = remaining(&["extend", shim.name, "--json", r#"{"a":1}"#]);
        let (result, _) = try_rewrite_to_service_args(&args).expect("shim must match");
        // The --json and value are forwarded verbatim.
        assert!(result.contains(&"--json".to_string()));
        assert!(result.contains(&r#"{"a":1}"#.to_string()));
    }

    // ── Flag renames ──

    #[test]
    fn rename_args_rewrites_bare_flag_and_leaves_its_value_alone() {
        let args = remaining(&["--app", "playground"]);
        let renamed = rename_args(&args, &[("app", "app-name")]);
        assert_eq!(renamed, vec!["--app-name", "playground"]);
    }

    #[test]
    fn rename_args_rewrites_equals_form() {
        let args = remaining(&["--app=playground"]);
        let renamed = rename_args(&args, &[("app", "app-name")]);
        assert_eq!(renamed, vec!["--app-name=playground"]);
    }

    #[test]
    fn rename_args_leaves_unmatched_flags_untouched() {
        let args = remaining(&["--namespace", "ns", "--app", "playground"]);
        let renamed = rename_args(&args, &[("app", "app-name")]);
        assert_eq!(
            renamed,
            vec!["--namespace", "ns", "--app-name", "playground"]
        );
    }

    #[test]
    fn rename_args_is_noop_with_no_renames() {
        let args = remaining(&["--app", "playground"]);
        assert_eq!(rename_args(&args, &[]), args);
    }

    #[test]
    fn list_endpoints_shim_rewrites_app_to_app_name() {
        let args = remaining(&[
            "extend",
            "security-assessment",
            "list-endpoints",
            "--app",
            "playground",
        ]);
        let (result, _presentation) = try_rewrite_to_service_args(&args).expect("shim must match");
        assert_eq!(
            result,
            vec![
                "csm",
                "security-assessment",
                "get-app-endpoints",
                "--app-name",
                "playground",
            ]
        );
    }

    #[test]
    fn list_shim_rewrites_to_csm_security_assessment_list() {
        let args = remaining(&["extend", "security-assessment", "list"]);
        let (result, _presentation) = try_rewrite_to_service_args(&args).expect("shim must match");
        assert_eq!(result, vec!["csm", "security-assessment", "list"]);
    }

    // ── Help section rendering ──

    /// Extract command names from the "Commands:" section of rendered help.
    ///
    /// Scans `help` for the `"Commands:"` heading, then collects the first
    /// whitespace-delimited word from each indented line until a
    /// non-indented, non-empty line is encountered.
    fn command_names_from_help(help: &str) -> Vec<String> {
        let help = strip_ansi_codes(help);
        let Some(section_start) = help.find("Commands:") else {
            return Vec::new();
        };
        let section = &help[section_start + "Commands:".len()..];
        let mut names = Vec::new();
        for line in section.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !line.starts_with(' ') && !line.starts_with('\t') {
                break;
            }
            if trimmed.starts_with('-') {
                continue;
            }
            if let Some(name) = trimmed.split_whitespace().next() {
                names.push(name.to_string());
            }
        }
        names
    }

    /// The rendered extend help lists every top-level shim in the
    /// Commands: section with its summary and canonical address.
    /// Replaces `rendered_extend_help_section_contains_shim_names` from
    /// round 4, which asserted the description-body placement.
    #[test]
    fn rendered_extend_help_lists_all_top_level_shims_in_commands() {
        let mut cmd = crate::invocation::builder::build_extend_command();
        let help = strip_ansi_codes(&cmd.render_long_help().to_string());
        let command_names = command_names_from_help(&help);
        for shim in SHIMS {
            if shim.parent.is_none() {
                assert!(
                    command_names.contains(&shim.name.to_string()),
                    "Commands section must list top-level shim '{}'; \
                     Commands: {command_names:?}\nhelp:\n{help}",
                    shim.name,
                );
            }
        }
    }

    /// Every shim in the SHIMS table must appear in the rendered help
    /// with its summary text and canonical `ags csm` address. Top-level
    /// shims appear in `ags extend --help`; subgroup shims appear in
    /// their parent's `--help`. The about string is compared by equality
    /// (split on the command name) to prevent drift between the
    /// registration table and the rendered output.
    #[test]
    fn extend_help_shows_all_shims_with_summary_and_address() {
        let mut cmd = crate::invocation::builder::build_extend_command();
        let extend_help = strip_ansi_codes(&cmd.render_long_help().to_string());

        // Verify top-level shims appear in extend help with full about.
        let mut top_level_count = 0;
        for shim in SHIMS {
            if shim.parent.is_none() {
                let expected_about = shim_about(shim);
                assert!(
                    extend_help.contains(&expected_about),
                    "extend help must contain about for '{}': '{}'\nhelp:\n{extend_help}",
                    shim.name,
                    expected_about,
                );
                top_level_count += 1;
            }
        }
        assert!(
            top_level_count > 0,
            "at least one top-level shim must exist"
        );

        // Verify subgroup shims appear in their parent's help.
        let subcommands: Vec<_> = cmd.get_subcommands().collect();
        for shim in SHIMS {
            if let Some(parent) = shim.parent {
                let parent_cmd = subcommands
                    .iter()
                    .find(|c| c.get_name() == parent)
                    .unwrap_or_else(|| panic!("extend must have a '{parent}' subcommand"));
                let parent_help =
                    strip_ansi_codes(&(*parent_cmd).clone().render_help().to_string());
                let expected_about = shim_about(shim);
                assert!(
                    parent_help.contains(&expected_about),
                    "{parent} help must contain about for '{}': '{}'\nhelp:\n{parent_help}",
                    shim.name,
                    expected_about,
                );
            }
        }
    }

    /// Subgroup children appear in their parent's clap-generated
    /// Commands: section. Replaces
    /// `promoted_parent_subgroups_still_show_own_shortcut_section` from
    /// round 4, which asserted a hand-rendered "Migration shortcuts"
    /// section.
    #[test]
    fn subgroup_children_appear_in_parent_commands_section() {
        let mut cmd = crate::invocation::builder::build_extend_command();

        // Check app-ui --help
        let app_ui = cmd
            .find_subcommand_mut("app-ui")
            .expect("app-ui must exist under extend");
        let app_ui_help = strip_ansi_codes(&app_ui.render_help().to_string());
        let app_ui_cmds = command_names_from_help(&app_ui_help);
        assert!(
            app_ui_cmds.contains(&"create".to_string()),
            "app-ui Commands: must list 'create'; got: {app_ui_cmds:?}\nhelp:\n{app_ui_help}"
        );

        // Check remote-debug --help
        let remote_debug = cmd
            .find_subcommand_mut("remote-debug")
            .expect("remote-debug must exist under extend");
        let remote_debug_help = strip_ansi_codes(&remote_debug.render_help().to_string());
        let remote_debug_cmds = command_names_from_help(&remote_debug_help);
        assert!(
            remote_debug_cmds.contains(&"disable".to_string()),
            "remote-debug Commands: must list 'disable'; got: {remote_debug_cmds:?}\nhelp:\n{remote_debug_help}"
        );
    }

    /// The `(was: ...)` suffix appears ONLY on shims whose Go invocation
    /// differs from their `ags extend` address. Today exactly one entry
    /// qualifies: `app-ui create` (Go spelling: `appui create`).
    /// Replaces `subgroup_help_shows_go_invocations_on_left` from round
    /// 4, which asserted Go invocations in a hand-rolled section.
    #[test]
    fn was_suffix_appears_only_when_go_invocation_differs() {
        for shim in SHIMS {
            let about = shim_about(shim);
            let extend_addr = match shim.parent {
                Some(parent) => format!("{parent} {}", shim.name),
                None => shim.name.to_string(),
            };
            if shim.go_invocation != extend_addr {
                assert!(
                    about.contains(&format!("(was: {})", shim.go_invocation)),
                    "shim '{}' has divergent Go invocation '{}' vs address '{}' \
                     but about lacks (was:) suffix: {about}",
                    shim.name,
                    shim.go_invocation,
                    extend_addr,
                );
            } else {
                assert!(
                    !about.contains("(was:"),
                    "shim '{}' has identical Go invocation and address '{}' \
                     but about contains (was:) suffix: {about}",
                    shim.name,
                    extend_addr,
                );
            }
        }
        // Pin the exact entry that currently diverges.
        let app_ui_create = SHIMS
            .iter()
            .find(|s| s.parent == Some("app-ui") && s.name == "create")
            .expect("app-ui create shim must exist");
        let about = shim_about(app_ui_create);
        assert!(
            about.contains("(was: appui create)"),
            "app-ui create must carry (was: appui create); got: {about}"
        );
    }

    // ── Uniqueness: every shim appears exactly once ──

    /// Every shim in the SHIMS table appears exactly once in the
    /// rendered help. A shim that is accidentally hidden or registered
    /// twice would break this invariant. Replaces
    /// `no_name_appears_in_both_commands_and_shortcuts_sections` from
    /// round 4, which asserted partition between two separate sections.
    #[test]
    fn every_shim_appears_exactly_once_in_help() {
        let mut cmd = crate::invocation::builder::build_extend_command();
        let help = strip_ansi_codes(&cmd.render_long_help().to_string());
        let command_names = command_names_from_help(&help);

        assert!(
            !command_names.is_empty(),
            "Commands section must not be empty; help:\n{help}"
        );

        for shim in SHIMS {
            if shim.parent.is_none() {
                let count = command_names
                    .iter()
                    .filter(|n| n.as_str() == shim.name)
                    .count();
                assert_eq!(
                    count, 1,
                    "top-level shim '{}' must appear exactly once in Commands; \
                     found {count} times; Commands: {command_names:?}",
                    shim.name,
                );
            }
        }
    }

    /// Every shim's about text in the Commands: section contains the
    /// arrow character pointing at its canonical `ags csm` address.
    /// A shim whose about was accidentally cleared or mangled would
    /// fail this check. Replaces `every_shortcuts_row_names_concrete_target`
    /// from round 4, which asserted arrow presence in a hand-rolled section.
    #[test]
    fn every_shim_about_names_canonical_address() {
        for shim in SHIMS {
            let about = shim_about(shim);
            assert!(
                about.contains('\u{2192}'),
                "shim '{}' about must contain \u{2192}; got: {about}",
                shim.name,
            );
            let expected_addr = format!("ags {} {} {}", shim.service, shim.resource, shim.method);
            assert!(
                about.contains(&expected_addr),
                "shim '{}' about must contain canonical address '{}'; got: {about}",
                shim.name,
                expected_addr,
            );
        }
    }

    // ── Description body wording ──

    /// The description body must NOT contain stale wording from earlier
    /// rounds: no "Non-API commands" and no "Migration shortcuts" heading.
    /// Replaces `description_references_migration_shortcuts` (round 4).
    #[test]
    fn description_body_contains_no_stale_wording() {
        let mut cmd = crate::invocation::builder::build_extend_command();
        let help = strip_ansi_codes(&cmd.render_long_help().to_string());

        assert!(
            !help.contains("Non-API commands"),
            "description must not contain old wording 'Non-API commands':\n{help}"
        );
        assert!(
            !help.contains("Migration shortcuts"),
            "help must not contain 'Migration shortcuts' heading:\n{help}"
        );
    }

    // ── Wait-flag parsing and dispatch ──

    fn strings(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_wait_defaults_when_only_wait() {
        let parsed = parse_wait_flags(&strings(&["--app", "my-app", "--wait"])).unwrap();
        assert!(parsed.wait);
        assert_eq!(parsed.interval, 10);
        assert_eq!(parsed.limit, 600);
        assert_eq!(parsed.app.as_deref(), Some("my-app"));
        // --app and its value are still forwarded to the primary call.
        assert_eq!(parsed.rest, strings(&["--app", "my-app"]));
    }

    #[test]
    fn parse_wait_strips_interval_and_limit_space_form() {
        let parsed = parse_wait_flags(&strings(&[
            "--app",
            "a",
            "--wait",
            "--wait-interval",
            "5",
            "--wait-limit",
            "60",
        ]))
        .unwrap();
        assert_eq!(parsed.interval, 5);
        assert_eq!(parsed.limit, 60);
        // Wait flags and their values must not leak into the forwarded args.
        assert_eq!(parsed.rest, strings(&["--app", "a"]));
    }

    #[test]
    fn parse_wait_equals_form() {
        let parsed =
            parse_wait_flags(&strings(&["--wait-interval=15", "--wait-limit=120"])).unwrap();
        assert_eq!(parsed.interval, 15);
        assert_eq!(parsed.limit, 120);
        assert!(parsed.rest.is_empty());
    }

    #[test]
    fn parse_wait_captures_app_from_short_and_equals() {
        assert_eq!(
            parse_wait_flags(&strings(&["-a", "x"]))
                .unwrap()
                .app
                .as_deref(),
            Some("x")
        );
        assert_eq!(
            parse_wait_flags(&strings(&["--app=y"]))
                .unwrap()
                .app
                .as_deref(),
            Some("y")
        );
    }

    #[test]
    fn parse_wait_absent_means_no_wait() {
        let parsed = parse_wait_flags(&strings(&["--app", "a"])).unwrap();
        assert!(!parsed.wait);
    }

    #[test]
    fn parse_wait_rejects_non_integer() {
        assert!(parse_wait_flags(&strings(&["--wait-interval", "abc"])).is_err());
    }

    #[test]
    fn parse_wait_rejects_zero_interval() {
        assert!(parse_wait_flags(&strings(&["--wait-interval", "0"])).is_err());
    }

    #[test]
    fn parse_wait_rejects_zero_limit() {
        // A 0 limit would enter the poll loop zero times and report an instant
        // timeout without ever polling — reject it up front, like interval 0.
        assert!(parse_wait_flags(&strings(&["--wait-limit", "0"])).is_err());
    }

    #[test]
    fn parse_wait_rejects_interval_greater_than_limit() {
        // The loop sleeps a full interval before checking the clock, so an
        // interval larger than the limit overshoots the promised maximum wait
        // (e.g. interval 300 / limit 30 blocks for 300s). Reject it.
        assert!(
            parse_wait_flags(&strings(&["--wait-interval", "300", "--wait-limit", "30"])).is_err()
        );
        // Equal is fine — a single poll at exactly the limit.
        assert!(
            parse_wait_flags(&strings(&["--wait-interval", "30", "--wait-limit", "30"])).is_ok()
        );
    }

    #[test]
    fn interval_over_limit_error_names_the_default_when_interval_not_passed() {
        // The common case: only `--wait-limit` passed, so the offending
        // interval is the default 10 — the message must say so and suggest
        // the fix, rather than blame a flag the user never typed.
        let Err(CliError::Usage { message, .. }) =
            parse_wait_flags(&strings(&["--wait-limit", "5"]))
        else {
            panic!("expected a usage error");
        };
        assert!(
            message.contains("10s, the default"),
            "must reveal the interval is the default: {message}"
        );
        assert!(
            message.contains("pass a smaller --wait-interval"),
            "must suggest the fix: {message}"
        );
        // When the interval WAS passed, it is not labelled a default.
        let Err(CliError::Usage { message, .. }) =
            parse_wait_flags(&strings(&["--wait-interval", "300", "--wait-limit", "30"]))
        else {
            panic!("expected a usage error");
        };
        assert!(
            !message.contains("the default"),
            "an explicit interval must not be labelled a default: {message}"
        );
    }

    #[test]
    fn rewrite_create_app_with_wait_builds_request_and_strips_flags() {
        let dispatch = try_rewrite_shim(&remaining(&[
            "extend",
            "create-app",
            "--namespace",
            "ns",
            "--app",
            "my-app",
            "--wait",
            "--wait-interval",
            "5",
        ]))
        .expect("create-app is a shim")
        .expect("wait flags are valid");

        // Service args carry the canonical triple and the surviving flags,
        // with the wait flags removed.
        assert_eq!(
            dispatch.service_args,
            strings(&[
                "csm",
                "apps",
                "create",
                "--namespace",
                "ns",
                "--app",
                "my-app"
            ])
        );
        let wait = dispatch.wait.expect("--wait must produce a wait request");
        assert_eq!(wait.app, "my-app");
        assert_eq!(wait.interval_secs, 5);
        assert_eq!(wait.limit_secs, 600);
    }

    #[test]
    fn rewrite_create_app_without_wait_has_no_request() {
        let dispatch = try_rewrite_shim(&remaining(&[
            "extend",
            "create-app",
            "--namespace",
            "ns",
            "--app",
            "my-app",
        ]))
        .expect("create-app is a shim")
        .expect("no wait flags is valid");
        assert!(dispatch.wait.is_none());
    }

    #[test]
    fn rewrite_wait_without_app_is_usage_error() {
        let result = try_rewrite_shim(&remaining(&["extend", "create-app", "--wait"]))
            .expect("create-app is a shim");
        assert!(matches!(result, Err(CliError::Usage { .. })));
    }

    // ── shim presentation + --wait help capability ──

    /// A wait-capable shim's dispatch carries a presentation marked
    /// `wait_capable`, so the help page adds the `--wait*` flags.
    #[test]
    fn wait_capable_shim_presentation_is_wait_capable() {
        for addr in [
            "deploy-app",
            "start-app",
            "stop-app",
            "delete-app",
            "create-app",
        ] {
            let dispatch = try_rewrite_shim(&remaining(&["extend", addr]))
                .expect("is a shim")
                .expect("valid");
            assert!(
                dispatch.presentation.wait_capable,
                "{addr} presentation must be wait_capable"
            );
        }
    }

    /// A non-wait shim's presentation is not `wait_capable`, so its help page
    /// gets no `--wait*` flags.
    #[test]
    fn non_wait_shim_presentation_is_not_wait_capable() {
        for addr in ["get-app-info", "list-images"] {
            let dispatch = try_rewrite_shim(&remaining(&["extend", addr]))
                .expect("is a shim")
                .expect("valid");
            assert!(
                !dispatch.presentation.wait_capable,
                "{addr} presentation must not be wait_capable"
            );
        }
    }

    /// The presentation carries the display and canonical addresses used to
    /// rewrite the shortcut help page's header and usage line.
    #[test]
    fn presentation_carries_display_and_canonical_addresses() {
        let dispatch = try_rewrite_shim(&remaining(&["extend", "deploy-app"]))
            .expect("is a shim")
            .expect("valid");
        assert_eq!(
            dispatch.presentation.display_address,
            "ags extend deploy-app"
        );
        assert_eq!(
            dispatch.presentation.canonical_address,
            "ags csm deployments create"
        );
    }

    #[test]
    fn rewrite_non_wait_shim_ignores_wait_field() {
        // get-app-info has no wait spec; it must dispatch normally.
        let dispatch = try_rewrite_shim(&remaining(&[
            "extend",
            "get-app-info",
            "--namespace",
            "ns",
            "--app",
            "a",
        ]))
        .expect("get-app-info is a shim")
        .expect("valid");
        assert!(dispatch.wait.is_none());
        assert_eq!(
            dispatch.service_args,
            strings(&["csm", "apps", "get", "--namespace", "ns", "--app", "a"])
        );
    }
}
