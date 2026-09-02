//! Static registration table mapping `extend-helper-cli` invocation names
//! to their canonical `ags csm` service operations.
//!
//! This table is the SOLE place any migration shortcut name is written as
//! a literal. The Clap subcommands, help text, and forwarding logic all
//! derive from it by iterating the slice. A rename costs one string edit
//! in one file — this one.

use clap::Command;

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
    },
    ExtendShim {
        name: "get-app-info",
        parent: None,
        go_invocation: "get-app-info",
        summary: "Show Extend app details",
        service: "csm",
        resource: "apps",
        method: "get",
    },
    ExtendShim {
        name: "list-images",
        parent: None,
        go_invocation: "list-images",
        summary: "List container images for an Extend app",
        service: "csm",
        resource: "images",
        method: "list",
    },
    ExtendShim {
        name: "deploy-app",
        parent: None,
        go_invocation: "deploy-app",
        summary: "Deploy an Extend app",
        service: "csm",
        resource: "deployments",
        method: "create",
    },
    ExtendShim {
        name: "start-app",
        parent: None,
        go_invocation: "start-app",
        summary: "Start an Extend app",
        service: "csm",
        resource: "apps",
        method: "start",
    },
    ExtendShim {
        name: "stop-app",
        parent: None,
        go_invocation: "stop-app",
        summary: "Stop an Extend app",
        service: "csm",
        resource: "apps",
        method: "stop",
    },
    ExtendShim {
        name: "delete-app",
        parent: None,
        go_invocation: "delete-app",
        summary: "Delete an Extend app",
        service: "csm",
        resource: "apps",
        method: "delete",
    },
    ExtendShim {
        name: "create",
        parent: Some("app-ui"),
        go_invocation: "appui create",
        summary: "Create an Extend app UI",
        service: "csm",
        resource: "app-ui",
        method: "create",
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

/// If `remaining` starts with `"extend"` followed by a registered shim
/// address, return the rewritten service arguments for `run_service`.
/// Returns `None` when the invocation is not a shim (e.g. `extend --help`,
/// `extend clone-template`, or `extend no-such-command`).
pub(crate) fn try_rewrite_to_service_args(remaining: &[String]) -> Option<Vec<String>> {
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
            return Some(build_service_args(shim, &extend_args[1..]));
        }
    }

    // Subgroup shims: `extend <parent> <name> [flags...]`
    if extend_args.len() >= 2 {
        let second = extend_args[1].as_str();
        for shim in SHIMS {
            if shim.parent == Some(first) && shim.name == second {
                return Some(build_service_args(shim, &extend_args[2..]));
            }
        }
    }

    None
}

/// Construct the service dispatch arguments from a matched shim entry
/// and the user's remaining flags. User-supplied flags are forwarded
/// verbatim after the service/resource/method triple.
fn build_service_args(shim: &ExtendShim, user_args: &[String]) -> Vec<String> {
    let mut args = vec![
        shim.service.to_string(),
        shim.resource.to_string(),
        shim.method.to_string(),
    ];
    args.extend_from_slice(user_args);
    args
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
            cmd = cmd.subcommand(
                Command::new(shim.name)
                    .about(shim_about(shim))
                    .disable_help_flag(true)
                    .hide(false),
            );
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
            sub = sub.subcommand(
                Command::new(shim.name)
                    .about(shim_about(shim))
                    .disable_help_flag(true)
                    .hide(false),
            );
        }

        cmd = cmd.subcommand(sub.hide(true));
    }

    cmd
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

            let result = try_rewrite_to_service_args(&remaining(&input)).unwrap_or_else(|| {
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
        let result = try_rewrite_to_service_args(&args).expect("shim must match");
        // The --json and value are forwarded verbatim.
        assert!(result.contains(&"--json".to_string()));
        assert!(result.contains(&r#"{"a":1}"#.to_string()));
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
}
