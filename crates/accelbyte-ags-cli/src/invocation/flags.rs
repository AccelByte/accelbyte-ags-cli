//! Global flag pre-scanning and CLI state.

use std::collections::HashMap;

/// Presentation preference selected by `--ui`. All four values serve a human;
/// the axis is plain line output vs inline (in-cursor) vs fullscreen
/// (alternate-screen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum UiFlag {
    /// Auto-detect from the decision matrix (default).
    Auto,
    /// Force the plain line-oriented surface.
    Plain,
    /// Force the inline (in-cursor) surface.
    Inline,
    /// Force the fullscreen (alternate-screen) surface.
    Fullscreen,
}

impl std::str::FromStr for UiFlag {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(UiFlag::Auto),
            "plain" => Ok(UiFlag::Plain),
            "inline" => Ok(UiFlag::Inline),
            "fullscreen" => Ok(UiFlag::Fullscreen),
            _ => Err(format!(
                "unknown --ui value '{s}' (expected one of: auto, plain, inline, fullscreen)"
            )),
        }
    }
}

/// Global flags that can appear anywhere in the argument list.
#[derive(Debug, Default, Clone)]
pub struct GlobalFlags {
    pub verbosity: ags_protocol::request::Verbosity,
    pub is_no_input: bool,
    pub is_no_color: bool,
    pub is_auto_confirmed: bool,
    pub format: Option<ags_protocol::request::OutputFormat>,
    pub ui: Option<UiFlag>,
    pub namespace: Option<String>,
    pub profile: Option<String>,
    pub is_dry_run: bool,
    pub is_skeleton: bool,
    pub timeout: Option<u64>,
    pub is_page_all: bool,
    pub page_limit: Option<u64>,
    /// Raw --page-limit value before validation (validated in mod.rs)
    pub page_limit_raw: Option<String>,
    pub output: Option<ags_protocol::request::OutputDestination>,
}

impl GlobalFlags {
    /// Derive the dispatch pagination hint from `--page-all` / `--page-limit`:
    /// `--page-all` with a limit caps page count, without one fetches all pages,
    /// and absent `--page-all` leaves pagination on its automatic default.
    pub fn pagination_hint(&self) -> ags_protocol::request::PaginationHint {
        use ags_protocol::request::PaginationHint;
        if self.is_page_all {
            match self.page_limit {
                Some(limit) => PaginationHint::Limit(limit),
                None => PaginationHint::All,
            }
        } else {
            PaginationHint::Auto
        }
    }

    /// Reconstruct `(flag_name, value)` pairs for every global flag that was
    /// set, so telemetry's `command_flag` capture sees them —
    /// `pre_scan_global_flags` strips these out of argv before
    /// `extract_flags` ever runs, so without this they are silently absent
    /// from telemetry rather than merely redacted (the 2026-08-14 bug).
    /// Value-carrying fields reconstruct the exact string the user would
    /// have typed (verified against each type's own `FromStr`), so
    /// telemetry's `VALUE_SAFE_FLAGS` allowlist sees the same shape it would
    /// see from raw argv.
    pub fn telemetry_pairs(&self) -> Vec<(String, Option<String>)> {
        let mut pairs = Vec::new();
        if matches!(self.verbosity, ags_protocol::request::Verbosity::Verbose) {
            pairs.push(("--verbose".to_string(), None));
        }
        if matches!(self.verbosity, ags_protocol::request::Verbosity::Quiet) {
            pairs.push(("--quiet".to_string(), None));
        }
        if self.is_no_input {
            pairs.push(("--no-input".to_string(), None));
        }
        if self.is_no_color {
            pairs.push(("--no-color".to_string(), None));
        }
        if self.is_auto_confirmed {
            pairs.push(("--yes".to_string(), None));
        }
        if let Some(format) = &self.format {
            let value = match format {
                ags_protocol::request::OutputFormat::Human => "human",
                ags_protocol::request::OutputFormat::Json => "json",
            };
            pairs.push(("--format".to_string(), Some(value.to_string())));
        }
        if let Some(ui) = &self.ui {
            let value = match ui {
                UiFlag::Auto => "auto",
                UiFlag::Plain => "plain",
                UiFlag::Inline => "inline",
                UiFlag::Fullscreen => "fullscreen",
            };
            pairs.push(("--ui".to_string(), Some(value.to_string())));
        }
        if let Some(namespace) = &self.namespace {
            pairs.push(("--namespace".to_string(), Some(namespace.clone())));
        }
        if let Some(profile) = &self.profile {
            pairs.push(("--profile".to_string(), Some(profile.clone())));
        }
        if self.is_dry_run {
            pairs.push(("--dry-run".to_string(), None));
        }
        if self.is_skeleton {
            pairs.push(("--skeleton".to_string(), None));
        }
        if let Some(timeout) = self.timeout {
            pairs.push(("--timeout".to_string(), Some(timeout.to_string())));
        }
        if self.is_page_all {
            pairs.push(("--page-all".to_string(), None));
        }
        if let Some(page_limit) = &self.page_limit_raw {
            pairs.push(("--page-limit".to_string(), Some(page_limit.clone())));
        }
        if let Some(output) = &self.output {
            let value = match output {
                ags_protocol::request::OutputDestination::Stdout => "-".to_string(),
                ags_protocol::request::OutputDestination::File(path) => path.display().to_string(),
            };
            pairs.push(("--output".to_string(), Some(value)));
        }
        pairs
    }
}

impl From<&GlobalFlags> for crate::frontend::RenderOptions {
    fn from(flags: &GlobalFlags) -> Self {
        Self {
            verbosity: flags.verbosity,
            is_page_all: flags.is_page_all,
            output: flags.output.clone(),
        }
    }
}

/// Every flag string the global prescan recognizes, paired with whether it
/// takes a value. This is the single source of truth consumed by
/// `pre_scan_global_flags`; the collision-prohibition test in `builder.rs`
/// also reads it so that new global flags automatically invalidate any
/// subcommand that shadows them.
pub(crate) const KNOWN_GLOBAL_FLAGS: &[(&str, bool)] = &[
    ("--verbose", false),
    ("-v", false),
    ("--quiet", false),
    ("-q", false),
    ("--no-input", false),
    ("--no-color", false),
    ("--yes", false),
    ("-y", false),
    ("--format", true),
    ("--ui", true),
    ("--namespace", true),
    ("-n", true),
    ("--output", true),
    ("--profile", true),
    ("--dry-run", false),
    ("--skeleton", false),
    ("--timeout", true),
    ("--page-all", false),
    ("--page-limit", true),
];

/// Pre-scan argv to extract global flags before two-phase parsing.
/// Returns (extracted flags, remaining args).
pub fn pre_scan_global_flags(
    args: &[String],
) -> Result<(GlobalFlags, Vec<String>), crate::errors::CliError> {
    let known_flags: HashMap<&str, bool> = KNOWN_GLOBAL_FLAGS.iter().copied().collect();

    let mut flags = GlobalFlags::default();
    let mut remaining = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];

        // Handle --flag=value syntax
        if let Some((flag_part, value_part)) = arg.split_once('=') {
            if let Some(&takes_value) = known_flags.get(flag_part) {
                if takes_value {
                    apply_flag(&mut flags, flag_part, Some(value_part))?;
                    i += 1;
                    continue;
                }
            }
        }

        if let Some(&takes_value) = known_flags.get(arg.as_str()) {
            if takes_value {
                // A value-taking flag needs a real value next: reject when it
                // is the last token, or when the next token is itself a flag
                // (so `--profile --dry-run` cannot silently swallow `--dry-run`,
                // and a bare `--format` reports "value required"). The lone `-`
                // stdout/stdin sentinel is a value, not a flag, so allow it.
                let next = args.get(i + 1);
                let looks_like_flag = next.is_some_and(|v| v.starts_with('-') && v.as_str() != "-");
                if next.is_none() || looks_like_flag {
                    return Err(crate::errors::CliError::Usage {
                        message: format!("A value is required for '{arg}' but none was supplied"),
                        metadata: None,
                    });
                }
                apply_flag(&mut flags, arg, next.map(|s| s.as_str()))?;
                i += 2;
            } else {
                apply_flag(&mut flags, arg, None)?;
                i += 1;
            }
        } else {
            remaining.push(arg.clone());
            i += 1;
        }
    }

    Ok((flags, remaining))
}

// ── Helpers ──

/// Set the appropriate GlobalFlags field for a matched CLI flag.
fn apply_flag(
    flags: &mut GlobalFlags,
    flag: &str,
    value: Option<&str>,
) -> Result<(), crate::errors::CliError> {
    match flag {
        "--verbose" | "-v" => flags.verbosity = ags_protocol::request::Verbosity::Verbose,
        "--quiet" | "-q" => flags.verbosity = ags_protocol::request::Verbosity::Quiet,
        "--no-input" => flags.is_no_input = true,
        "--no-color" => flags.is_no_color = true,
        "--yes" | "-y" => flags.is_auto_confirmed = true,
        "--format" => {
            if let Some(v) = value {
                flags.format =
                    Some(
                        v.parse()
                            .map_err(|message: String| crate::errors::CliError::Usage {
                                message,
                                metadata: None,
                            })?,
                    );
            }
        }
        "--ui" => {
            if let Some(v) = value {
                flags.ui =
                    Some(
                        v.parse()
                            .map_err(|message: String| crate::errors::CliError::Usage {
                                message,
                                metadata: None,
                            })?,
                    );
            }
        }
        "--namespace" | "-n" => {
            if let Some(v) = value {
                if v.trim().is_empty() {
                    return Err(crate::errors::CliError::Usage {
                        message: "--namespace value cannot be empty".to_string(),
                        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                            "Pass a non-empty namespace, e.g. --namespace my-game",
                        ))),
                    });
                }
                flags.namespace = Some(v.to_string());
            }
        }
        "--output" => {
            if let Some(v) = value {
                // FromStr is infallible — every string is a valid path or "-".
                flags.output = v.parse().ok();
            }
        }
        "--profile" => {
            if let Some(v) = value {
                flags.profile = Some(v.to_string());
            }
        }
        "--dry-run" => flags.is_dry_run = true,
        "--skeleton" => flags.is_skeleton = true,
        "--timeout" => {
            if let Some(v) = value {
                let parsed: u64 = v.parse().map_err(|_| crate::errors::CliError::Usage {
                    message: format!("Invalid --timeout value '{v}'"),
                    metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                        "Pass a positive integer number of seconds, e.g. --timeout 60",
                    ))),
                })?;
                if parsed == 0 {
                    return Err(crate::errors::CliError::Usage {
                        message: "--timeout must be at least 1 second".to_string(),
                        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                            "Pass a positive integer, e.g. --timeout 60",
                        ))),
                    });
                }
                flags.timeout = Some(parsed);
            }
        }
        "--page-all" => flags.is_page_all = true,
        "--page-limit" => {
            if let Some(v) = value {
                flags.page_limit_raw = Some(v.to_string());
            }
        }
        _ => {}
    }
    Ok(())
}

/// Leaf-level selectors that narrow which operation variant to use.
#[derive(Debug, Default, Clone)]
pub struct LeafSelectors {
    pub api_scope: Option<String>,
    pub api_version: Option<String>,
}

/// Pre-scan argv to extract `--api-scope` and `--api-version` before building
/// the Clap leaf command. Returns (extracted selectors, remaining args).
///
/// Returns an error when one of the flags is supplied with no following
/// value (e.g. `--api-scope` as the final token, or followed only by another
/// flag). This gives a consistent message across methods that do and don't
/// have the flag registered on their Clap command — otherwise single-scope
/// methods would report "Unexpected argument" because the flag isn't in
/// their Clap arg list.
///
/// Duplicate flags (e.g. `--api-scope admin --api-scope public`) follow
/// last-writer-wins semantics — the final value supplied takes effect and
/// earlier values are silently discarded. This matches Clap's default for
/// non-repeatable options and avoids penalising users who, for example,
/// override a flag from a wrapping shell alias.
pub fn pre_scan_leaf_selectors(
    args: &[String],
) -> Result<(LeafSelectors, Vec<String>), crate::errors::CliError> {
    let mut sel = LeafSelectors::default();
    let mut remaining = Vec::with_capacity(args.len());
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];

        if let Some((flag, value)) = arg.split_once('=') {
            match flag {
                "--api-scope" => {
                    sel.api_scope = Some(value.to_string());
                    i += 1;
                    continue;
                }
                "--api-version" => {
                    sel.api_version = Some(value.to_string());
                    i += 1;
                    continue;
                }
                _ => {}
            }
        }

        match arg.as_str() {
            "--api-scope" | "--api-version" => {
                let flag = arg.as_str();
                let next = args.get(i + 1);
                let has_value = next.is_some_and(|v| !v.starts_with('-'));
                if !has_value {
                    return Err(crate::errors::CliError::Usage {
                        message: format!(
                            "A value is required for '{flag} <{}>' but none was supplied",
                            flag.trim_start_matches("--")
                        ),
                        metadata: None,
                    });
                }
                let value = args[i + 1].clone();
                if flag == "--api-scope" {
                    sel.api_scope = Some(value);
                } else {
                    sel.api_version = Some(value);
                }
                i += 2;
                continue;
            }
            _ => {}
        }

        remaining.push(arg.clone());
        i += 1;
    }

    Ok((sel, remaining))
}

/// Apply global config defaults for flags not set on the command line.
/// Resolution order: CLI flag (already set) → global config → built-in default.
/// Config errors are silently ignored to avoid blocking CLI startup.
pub fn apply_config_defaults(flags: &mut GlobalFlags) {
    let config = match ags_runtime::runtime::config::GlobalConfig::load() {
        Ok(c) => c,
        Err(_) => return,
    };

    if flags.format.is_none() {
        if let Some(format) = config.format {
            flags.format = Some(format);
        }
    }

    if !flags.is_no_color {
        if let Some(true) = config.no_color {
            flags.is_no_color = true;
        }
    }

    if flags.timeout.is_none() {
        flags.timeout = config.timeout;
    }

    if flags.page_limit.is_none() {
        flags.page_limit = config.page_limit;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::request::{OutputDestination, OutputFormat, Verbosity};

    /// Boolean flags like --verbose are extracted and removed from the remaining args
    #[test]
    fn test_pre_scan_extracts_verbose() {
        let args: Vec<String> = vec!["--verbose", "iam", "users", "list"]
            .into_iter()
            .map(String::from)
            .collect();
        let (flags, remaining) = pre_scan_global_flags(&args).unwrap();
        assert_eq!(flags.verbosity, Verbosity::Verbose);
        assert_eq!(remaining, vec!["iam", "users", "list"]);
    }

    /// Value-taking flags consume the next argument and both are removed from remaining args
    #[test]
    fn test_pre_scan_extracts_format_with_value() {
        let args: Vec<String> = vec!["iam", "--format", "json", "users", "list"]
            .into_iter()
            .map(String::from)
            .collect();
        let (flags, remaining) = pre_scan_global_flags(&args).unwrap();
        assert_eq!(flags.format, Some(OutputFormat::Json));
        assert_eq!(remaining, vec!["iam", "users", "list"]);
    }

    /// The --flag=value syntax is accepted as an alternative to --flag value
    #[test]
    fn test_pre_scan_handles_equals_syntax() {
        let args: Vec<String> = vec!["iam", "--format=json", "users", "list"]
            .into_iter()
            .map(String::from)
            .collect();
        let (flags, remaining) = pre_scan_global_flags(&args).unwrap();
        assert_eq!(flags.format, Some(OutputFormat::Json));
        assert_eq!(remaining, vec!["iam", "users", "list"]);
    }

    /// An unknown --format value is rejected at flag-parse time as a Usage error
    #[test]
    fn test_pre_scan_rejects_unknown_format() {
        let args: Vec<String> = vec!["iam", "--format=table", "users", "list"]
            .into_iter()
            .map(String::from)
            .collect();
        let err = pre_scan_global_flags(&args).unwrap_err();
        let crate::errors::CliError::Usage { message, .. } = err else {
            panic!("expected Usage error");
        };
        assert!(message.contains("unknown --format value 'table'"));
    }

    /// `--ui plain` and `--ui fullscreen` parse into the matching UiFlag value
    #[test]
    fn test_pre_scan_extracts_ui_space_form() {
        let plain: Vec<String> = vec!["iam", "--ui", "plain", "users", "list"]
            .into_iter()
            .map(String::from)
            .collect();
        let (flags, remaining) = pre_scan_global_flags(&plain).unwrap();
        assert_eq!(flags.ui, Some(UiFlag::Plain));
        assert_eq!(remaining, vec!["iam", "users", "list"]);

        let full: Vec<String> = vec!["iam", "--ui", "fullscreen", "users", "list"]
            .into_iter()
            .map(String::from)
            .collect();
        let (flags, _) = pre_scan_global_flags(&full).unwrap();
        assert_eq!(flags.ui, Some(UiFlag::Fullscreen));
    }

    /// `--ui=plain` and `--ui=inline` equals-form parse into the matching UiFlag value
    #[test]
    fn test_pre_scan_extracts_ui_equals_form() {
        let plain: Vec<String> = vec!["iam", "--ui=plain", "users", "list"]
            .into_iter()
            .map(String::from)
            .collect();
        let (flags, _) = pre_scan_global_flags(&plain).unwrap();
        assert_eq!(flags.ui, Some(UiFlag::Plain));

        let inline: Vec<String> = vec!["iam", "--ui=inline", "users", "list"]
            .into_iter()
            .map(String::from)
            .collect();
        let (flags, _) = pre_scan_global_flags(&inline).unwrap();
        assert_eq!(flags.ui, Some(UiFlag::Inline));
    }

    /// The removed `tui` alias is now rejected as an unknown `--ui` value.
    #[test]
    fn test_pre_scan_rejects_removed_tui_alias() {
        let args: Vec<String> = vec!["iam", "--ui=tui", "users", "list"]
            .into_iter()
            .map(String::from)
            .collect();
        let err = pre_scan_global_flags(&args).unwrap_err();
        let crate::errors::CliError::Usage { message, .. } = err else {
            panic!("expected Usage error");
        };
        assert!(message.contains("unknown --ui value 'tui'"));
    }

    /// An unknown --ui value is rejected at flag-parse time as a Usage error
    #[test]
    fn test_pre_scan_rejects_unknown_ui() {
        let args: Vec<String> = vec!["iam", "--ui=foo", "users", "list"]
            .into_iter()
            .map(String::from)
            .collect();
        let err = pre_scan_global_flags(&args).unwrap_err();
        let crate::errors::CliError::Usage { message, .. } = err else {
            panic!("expected Usage error");
        };
        assert!(message.contains("unknown --ui value 'foo'"));
        assert!(message.contains("expected one of: auto, plain, inline, fullscreen"));
    }

    /// Multiple global flags of different types can appear together and all get extracted
    #[test]
    fn test_pre_scan_multiple_flags() {
        let args: Vec<String> = vec![
            "--verbose",
            "--dry-run",
            "--namespace",
            "my-ns",
            "iam",
            "users",
            "list",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let (flags, remaining) = pre_scan_global_flags(&args).unwrap();
        assert_eq!(flags.verbosity, Verbosity::Verbose);
        assert!(flags.is_dry_run);
        assert_eq!(flags.namespace, Some("my-ns".to_string()));
        assert_eq!(remaining, vec!["iam", "users", "list"]);
    }

    /// The --skeleton flag is extracted and sets is_skeleton to true
    #[test]
    fn test_pre_scan_extracts_skeleton() {
        let args: Vec<String> = vec!["--skeleton", "iam", "roles", "create"]
            .into_iter()
            .map(String::from)
            .collect();
        let (flags, remaining) = pre_scan_global_flags(&args).unwrap();
        assert!(flags.is_skeleton);
        assert_eq!(remaining, vec!["iam", "roles", "create"]);
    }

    /// The --page-all and --page-limit flags are extracted correctly
    #[test]
    fn test_pre_scan_extracts_page_all() {
        let args: Vec<String> = vec!["--page-all", "--page-limit", "5", "iam", "users", "list"]
            .into_iter()
            .map(String::from)
            .collect();
        let (flags, remaining) = pre_scan_global_flags(&args).unwrap();
        assert!(flags.is_page_all);
        assert_eq!(flags.page_limit_raw, Some("5".to_string()));
        assert_eq!(remaining, vec!["iam", "users", "list"]);
    }

    #[test]
    fn test_pre_scan_leaf_selectors_extracts_both() {
        let args: Vec<String> = [
            "iam",
            "users",
            "get",
            "abc",
            "--api-scope",
            "public",
            "--api-version",
            "v2",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let (sel, remaining) = pre_scan_leaf_selectors(&args).unwrap();

        assert_eq!(sel.api_scope.as_deref(), Some("public"));
        assert_eq!(sel.api_version.as_deref(), Some("v2"));
        assert_eq!(remaining, vec!["iam", "users", "get", "abc"]);
    }

    #[test]
    fn test_pre_scan_leaf_selectors_accepts_equals_form() {
        let args: Vec<String> = [
            "iam",
            "users",
            "get",
            "--api-scope=public",
            "--api-version=v2",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let (sel, remaining) = pre_scan_leaf_selectors(&args).unwrap();

        assert_eq!(sel.api_scope.as_deref(), Some("public"));
        assert_eq!(sel.api_version.as_deref(), Some("v2"));
        assert_eq!(remaining, vec!["iam", "users", "get"]);
    }

    #[test]
    fn test_pre_scan_leaf_selectors_returns_none_when_absent() {
        let args: Vec<String> = ["iam", "users", "get", "abc"]
            .into_iter()
            .map(String::from)
            .collect();

        let (sel, _) = pre_scan_leaf_selectors(&args).unwrap();

        assert!(sel.api_scope.is_none());
        assert!(sel.api_version.is_none());
    }

    /// `--api-scope` as the last token (no value) must produce a clean
    /// "value required" error, not be silently forwarded to Clap where
    /// methods without the flag would render "Unexpected argument".
    #[test]
    fn test_pre_scan_leaf_selectors_errors_when_scope_missing_value() {
        let args: Vec<String> = ["iam", "users", "list", "--namespace", "ns", "--api-scope"]
            .into_iter()
            .map(String::from)
            .collect();

        let err = pre_scan_leaf_selectors(&args).unwrap_err();
        let crate::errors::CliError::Usage { message, .. } = err else {
            panic!("expected Usage error");
        };
        assert!(message.contains("--api-scope"));
        assert!(message.contains("value is required"));
    }

    /// Same for `--api-version` with no value.
    #[test]
    fn test_pre_scan_leaf_selectors_errors_when_version_missing_value() {
        let args: Vec<String> = ["iam", "roles", "list", "--api-version"]
            .into_iter()
            .map(String::from)
            .collect();

        let err = pre_scan_leaf_selectors(&args).unwrap_err();
        let crate::errors::CliError::Usage { message, .. } = err else {
            panic!("expected Usage error");
        };
        assert!(message.contains("--api-version"));
    }

    /// When `--api-scope` is followed by another flag (not a value), it
    /// must be treated as missing a value — not consume the next flag.
    #[test]
    fn test_pre_scan_leaf_selectors_errors_when_scope_followed_by_flag() {
        let args: Vec<String> = ["iam", "roles", "list", "--api-scope", "--api-version", "v4"]
            .into_iter()
            .map(String::from)
            .collect();

        let err = pre_scan_leaf_selectors(&args).unwrap_err();
        let crate::errors::CliError::Usage { message, .. } = err else {
            panic!("expected Usage error");
        };
        assert!(message.contains("--api-scope"));
    }

    #[test]
    fn test_pre_scan_global_flags_extracts_output_path() {
        let args: Vec<String> = ["iam", "users", "list", "--output", "/tmp/foo.json"]
            .into_iter()
            .map(String::from)
            .collect();
        let (flags, remaining) = pre_scan_global_flags(&args).unwrap();
        assert_eq!(
            flags.output,
            Some(OutputDestination::File(std::path::PathBuf::from(
                "/tmp/foo.json"
            )))
        );
        assert_eq!(remaining, vec!["iam", "users", "list"]);
    }

    #[test]
    fn test_pre_scan_global_flags_extracts_output_stdout_alias() {
        let args: Vec<String> = ["iam", "users", "list", "--output", "-"]
            .into_iter()
            .map(String::from)
            .collect();
        let (flags, _) = pre_scan_global_flags(&args).unwrap();
        assert_eq!(flags.output, Some(OutputDestination::Stdout));
    }

    #[test]
    fn test_pre_scan_rejects_value_flag_with_missing_value() {
        // A bare value-taking flag at the end of argv must report "value
        // required" rather than being silently swallowed.
        let args: Vec<String> = ["iam", "users", "list", "--format"]
            .into_iter()
            .map(String::from)
            .collect();
        let err = pre_scan_global_flags(&args).unwrap_err();
        let crate::errors::CliError::Usage { message, .. } = err else {
            panic!("expected Usage error");
        };
        assert!(
            message.contains("A value is required for '--format'"),
            "got: {message}"
        );
    }

    #[test]
    fn test_pre_scan_rejects_value_flag_followed_by_another_flag() {
        // `--profile --dry-run` must not consume `--dry-run` as the profile
        // value; the missing profile value is a usage error.
        let args: Vec<String> = ["iam", "--profile", "--dry-run", "users", "list"]
            .into_iter()
            .map(String::from)
            .collect();
        let err = pre_scan_global_flags(&args).unwrap_err();
        let crate::errors::CliError::Usage { message, .. } = err else {
            panic!("expected Usage error");
        };
        assert!(
            message.contains("A value is required for '--profile'"),
            "got: {message}"
        );
    }

    #[test]
    fn test_pre_scan_global_flags_extracts_output_equals_form() {
        let args: Vec<String> = ["iam", "users", "list", "--output=file.png"]
            .into_iter()
            .map(String::from)
            .collect();
        let (flags, _) = pre_scan_global_flags(&args).unwrap();
        assert_eq!(
            flags.output,
            Some(OutputDestination::File(std::path::PathBuf::from(
                "file.png"
            )))
        );
    }

    // ── Global flags consumed regardless of command position ──

    /// `--namespace` and `--format` are consumed by the prescan regardless
    /// of where they appear — even after `extend docker-login`. There is no
    /// command-specific boundary; the route reads namespace from
    /// `flags.namespace` via its fallback path.
    #[test]
    fn test_pre_scan_consumes_namespace_after_extend_docker_login() {
        let args: Vec<String> = [
            "extend",
            "docker-login",
            "--namespace",
            "game-ns",
            "--app",
            "myapp",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let (flags, remaining) = pre_scan_global_flags(&args).unwrap();
        assert_eq!(
            flags.namespace,
            Some("game-ns".to_string()),
            "--namespace must be consumed globally"
        );
        assert_eq!(
            remaining,
            vec!["extend", "docker-login", "--app", "myapp"],
            "--namespace and its value must not appear in remaining"
        );
    }

    /// The `--namespace=value` equals form is consumed globally after
    /// `extend docker-login`, exactly as the `--namespace value` space
    /// form is. The two syntaxes must agree.
    #[test]
    fn test_pre_scan_consumes_namespace_equals_form_after_extend_docker_login() {
        let args: Vec<String> = [
            "extend",
            "docker-login",
            "--namespace=game-ns",
            "--app",
            "myapp",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let (flags, remaining) = pre_scan_global_flags(&args).unwrap();
        assert_eq!(
            flags.namespace,
            Some("game-ns".to_string()),
            "--namespace=value must be consumed globally (equals form)"
        );
        assert_eq!(
            remaining,
            vec!["extend", "docker-login", "--app", "myapp"],
            "--namespace=value must not appear in remaining"
        );
    }

    // ── Route-local flags pass through the prescan ──

    /// `--print-format <value>` (space form) is NOT a global flag; the
    /// prescan must leave it in remaining so the route's clap parser
    /// receives it.
    #[test]
    fn test_pre_scan_passes_through_print_format_space_form() {
        let args: Vec<String> = [
            "extend",
            "docker-login",
            "--namespace",
            "game-ns",
            "--app",
            "myapp",
            "--print-format",
            "token",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let (flags, remaining) = pre_scan_global_flags(&args).unwrap();
        assert_eq!(
            flags.namespace,
            Some("game-ns".to_string()),
            "--namespace is still consumed"
        );
        assert!(
            remaining.contains(&"--print-format".to_string()),
            "--print-format must pass through to remaining: {remaining:?}"
        );
        assert!(
            remaining.contains(&"token".to_string()),
            "the --print-format value must pass through to remaining: {remaining:?}"
        );
    }

    /// `--print-format=<value>` (equals form) is NOT a global flag; the
    /// prescan must leave it in remaining so the route's clap parser
    /// receives it. Must agree with the space form above.
    #[test]
    fn test_pre_scan_passes_through_print_format_equals_form() {
        let args: Vec<String> = [
            "extend",
            "docker-login",
            "--namespace=game-ns",
            "--app",
            "myapp",
            "--print-format=token",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let (flags, remaining) = pre_scan_global_flags(&args).unwrap();
        assert_eq!(
            flags.namespace,
            Some("game-ns".to_string()),
            "--namespace=value is still consumed (equals form)"
        );
        assert!(
            remaining.contains(&"--print-format=token".to_string()),
            "--print-format=value must pass through to remaining: {remaining:?}"
        );
    }

    #[test]
    fn test_telemetry_pairs_reconstructs_set_global_flags() {
        let flags = GlobalFlags {
            namespace: Some("ns1".to_string()),
            format: Some(ags_protocol::request::OutputFormat::Json),
            is_no_color: true,
            ..GlobalFlags::default()
        };
        let pairs = flags.telemetry_pairs();
        assert_eq!(
            pairs,
            vec![
                ("--no-color".to_string(), None),
                ("--format".to_string(), Some("json".to_string())),
                ("--namespace".to_string(), Some("ns1".to_string())),
            ]
        );
    }

    #[test]
    fn test_telemetry_pairs_empty_for_default_flags() {
        assert_eq!(GlobalFlags::default().telemetry_pairs(), Vec::new());
    }

    #[test]
    fn test_telemetry_pairs_reconstructs_output_stdout_and_file() {
        let stdout_flags = GlobalFlags {
            output: Some(ags_protocol::request::OutputDestination::Stdout),
            ..GlobalFlags::default()
        };
        assert_eq!(
            stdout_flags.telemetry_pairs(),
            vec![("--output".to_string(), Some("-".to_string()))]
        );

        let file_flags = GlobalFlags {
            output: Some(ags_protocol::request::OutputDestination::File(
                "out.json".into(),
            )),
            ..GlobalFlags::default()
        };
        assert_eq!(
            file_flags.telemetry_pairs(),
            vec![("--output".to_string(), Some("out.json".to_string()))]
        );
    }
}
