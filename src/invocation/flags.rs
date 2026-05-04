//! Global flag pre-scanning and CLI state.

use std::collections::HashMap;

/// Global flags that can appear anywhere in the argument list.
#[derive(Debug, Default)]
pub struct GlobalFlags {
    pub verbosity: crate::protocol::request::Verbosity,
    pub is_no_input: bool,
    pub is_no_color: bool,
    pub is_auto_confirmed: bool,
    pub format: Option<crate::protocol::request::OutputFormat>,
    pub namespace: Option<String>,
    pub profile: Option<String>,
    pub is_dry_run: bool,
    pub is_skeleton: bool,
    pub timeout: Option<u64>,
    pub is_page_all: bool,
    pub page_limit: Option<u64>,
    /// Raw --page-limit value before validation (validated in mod.rs)
    pub page_limit_raw: Option<String>,
    pub output: Option<crate::protocol::request::OutputDestination>,
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

/// Pre-scan argv to extract global flags before two-phase parsing.
/// Returns (extracted flags, remaining args).
pub fn pre_scan_global_flags(
    args: &[String],
) -> Result<(GlobalFlags, Vec<String>), crate::errors::CliError> {
    // Map of known flags and whether they take a value
    let mut known_flags: HashMap<&str, bool> = HashMap::new();
    known_flags.insert("--verbose", false);
    known_flags.insert("-v", false);
    known_flags.insert("--quiet", false);
    known_flags.insert("-q", false);
    known_flags.insert("--no-input", false);
    known_flags.insert("--no-color", false);
    known_flags.insert("--yes", false);
    known_flags.insert("-y", false);
    known_flags.insert("--format", true);
    known_flags.insert("--namespace", true);
    known_flags.insert("-n", true);
    known_flags.insert("--output", true);
    known_flags.insert("--profile", true);
    known_flags.insert("--dry-run", false);
    known_flags.insert("--skeleton", false);
    known_flags.insert("--timeout", true);
    known_flags.insert("--page-all", false);
    known_flags.insert("--page-limit", true);

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
                let value = args.get(i + 1).map(|s| s.as_str());
                apply_flag(&mut flags, arg, value)?;
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
        "--verbose" | "-v" => flags.verbosity = crate::protocol::request::Verbosity::Verbose,
        "--quiet" | "-q" => flags.verbosity = crate::protocol::request::Verbosity::Quiet,
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
    let config = match crate::runtime::config::GlobalConfig::load() {
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
    use crate::protocol::request::{OutputDestination, OutputFormat, Verbosity};

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
}
