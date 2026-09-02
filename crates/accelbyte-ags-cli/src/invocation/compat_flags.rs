//! Backward-compatible flags accepted for migration from another CLI.
//!
//! When a command is ported from a predecessor CLI (e.g. `extend-helper-cli`),
//! flags the predecessor accepted but this CLI does not honor are registered
//! as compat flags. They are visible in `--help`, accepted by clap, and
//! produce a one-time stderr notice when the user explicitly supplies them.
//!
//! [`CompatFlag`] is the single definition of both the clap `Arg` and the
//! notice, so they cannot drift apart. The structural test
//! `test_extend_hidden_compat_flags_registered_through_compat_flag` (in
//! `builder.rs`) walks the live clap tree and verifies that every hidden or
//! compat-marked flag is present in [`all_compat_flag_ids`], so a route that
//! adds a silent no-op flag without a [`CompatFlag`] entry fails the suite.

use clap::Arg;

/// A backward-compatible flag accepted for migration but not honored.
///
/// One struct produces both the clap [`Arg`] (via [`to_arg`](Self::to_arg))
/// and the presence check (via [`was_supplied`](Self::was_supplied)), so the
/// two cannot drift apart.
pub struct CompatFlag {
    /// The clap argument ID (matches the `--long` name).
    pub id: &'static str,
    /// Optional single-character short alias.
    pub short: Option<char>,
    /// Help text shown in `--help`.
    pub help: &'static str,
    /// Whether this flag takes a value (true) or is a boolean toggle (false).
    pub takes_value: bool,
    /// Value placeholder shown in help (e.g. `"level"`). Only for valued flags.
    pub value_name: Option<&'static str>,
    /// Default value so clap always has a value. Only for valued flags.
    pub default_value: Option<&'static str>,
}

impl CompatFlag {
    /// Build the clap [`Arg`] for this compat flag. The resulting argument is
    /// visible in `--help` with the compatibility note.
    pub fn to_arg(&self) -> Arg {
        let mut arg = Arg::new(self.id).long(self.id).help(self.help);
        if let Some(short) = self.short {
            arg = arg.short(short);
        }
        if self.takes_value {
            if let Some(vn) = self.value_name {
                arg = arg.value_name(vn);
            }
            if let Some(dv) = self.default_value {
                arg = arg.default_value(dv);
            }
        } else {
            arg = arg.action(clap::ArgAction::SetTrue);
        }
        arg
    }

    /// Whether the user explicitly supplied this flag on the command line.
    /// Returns `false` for a default value that clap injected.
    pub fn was_supplied(&self, matches: &clap::ArgMatches) -> bool {
        matches.value_source(self.id) == Some(clap::parser::ValueSource::CommandLine)
    }
}

/// Collect the IDs of compat flags the user explicitly supplied on the
/// command line. Pure function: no emission, no side effects.
pub(crate) fn collect_supplied_flags<'a>(
    matches: &clap::ArgMatches,
    flags: &[&'a CompatFlag],
) -> Vec<&'a str> {
    flags
        .iter()
        .filter(|flag| flag.was_supplied(matches))
        .map(|flag| flag.id)
        .collect()
}

/// All compat-flag IDs registered across all extend routes.
///
/// The structural test `test_extend_hidden_compat_flags_registered_through_compat_flag`
/// compares this list against the live clap tree so a hidden no-op flag added
/// without a [`CompatFlag`] entry fails the suite.
#[cfg(test)]
pub(crate) fn all_compat_flag_ids() -> &'static [&'static str] {
    // APP_UI_UPLOAD_VERBOSITY has the same id ("verbosity") as
    // DOCKER_LOGIN_VERBOSITY, so it does not need a separate entry.
    &[DOCKER_LOGIN_LOGIN.id, DOCKER_LOGIN_VERBOSITY.id]
}

// ── Docker-login compat flags ──

/// `--login` / `-l`: legacy boolean flag from the Go `extend-helper-cli`.
pub const DOCKER_LOGIN_LOGIN: CompatFlag = CompatFlag {
    id: "login",
    short: Some('l'),
    help: "Accepted for backward compatibility, ignored",
    takes_value: false,
    value_name: None,
    default_value: None,
};

/// `--verbosity`: log verbosity level from the Go `extend-helper-cli`.
pub const DOCKER_LOGIN_VERBOSITY: CompatFlag = CompatFlag {
    id: "verbosity",
    short: None,
    help: "Accepted for backward compatibility, ignored",
    takes_value: true,
    value_name: Some("level"),
    default_value: Some("info"),
};

// ── App-ui upload compat flags ──

/// `--verbosity`: log verbosity level from the Go `extend-helper-cli`.
pub const APP_UI_UPLOAD_VERBOSITY: CompatFlag = CompatFlag {
    id: "verbosity",
    short: None,
    help: "Accepted for backward compatibility, ignored",
    takes_value: true,
    value_name: Some("level"),
    default_value: Some("info"),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bool_compat_flag_builds_set_true_arg() {
        let arg = DOCKER_LOGIN_LOGIN.to_arg();
        let cmd = clap::Command::new("test").arg(arg);
        let matches = cmd
            .try_get_matches_from(["test", "--login"])
            .expect("--login must be accepted");
        assert!(matches.get_flag("login"));
    }

    #[test]
    fn test_valued_compat_flag_accepts_value() {
        let arg = DOCKER_LOGIN_VERBOSITY.to_arg();
        let cmd = clap::Command::new("test").arg(arg);
        let matches = cmd
            .try_get_matches_from(["test", "--verbosity", "debug"])
            .expect("--verbosity debug must be accepted");
        assert_eq!(
            matches.get_one::<String>("verbosity").map(|s| s.as_str()),
            Some("debug")
        );
    }

    #[test]
    fn test_was_supplied_true_when_flag_on_command_line() {
        let arg = DOCKER_LOGIN_VERBOSITY.to_arg();
        let cmd = clap::Command::new("test").arg(arg);
        let matches = cmd
            .try_get_matches_from(["test", "--verbosity", "debug"])
            .expect("must parse");
        assert!(
            DOCKER_LOGIN_VERBOSITY.was_supplied(&matches),
            "flag explicitly passed must be detected as supplied"
        );
    }

    #[test]
    fn test_was_supplied_false_for_default_value() {
        let arg = DOCKER_LOGIN_VERBOSITY.to_arg();
        let cmd = clap::Command::new("test").arg(arg);
        let matches = cmd
            .try_get_matches_from(["test"])
            .expect("must parse without --verbosity");
        assert!(
            !DOCKER_LOGIN_VERBOSITY.was_supplied(&matches),
            "default value must not be detected as supplied"
        );
    }

    #[test]
    fn test_was_supplied_false_for_bool_flag_not_passed() {
        let arg = DOCKER_LOGIN_LOGIN.to_arg();
        let cmd = clap::Command::new("test").arg(arg);
        let matches = cmd
            .try_get_matches_from(["test"])
            .expect("must parse without --login");
        assert!(
            !DOCKER_LOGIN_LOGIN.was_supplied(&matches),
            "bool flag not passed must not be detected as supplied"
        );
    }

    #[test]
    fn test_collect_supplied_returns_supplied_flag_names() {
        let login_arg = DOCKER_LOGIN_LOGIN.to_arg();
        let verbosity_arg = DOCKER_LOGIN_VERBOSITY.to_arg();
        let cmd = clap::Command::new("test").arg(login_arg).arg(verbosity_arg);
        let matches = cmd
            .try_get_matches_from(["test", "--login", "--verbosity", "debug"])
            .expect("must parse");
        let supplied =
            collect_supplied_flags(&matches, &[&DOCKER_LOGIN_LOGIN, &DOCKER_LOGIN_VERBOSITY]);
        assert!(
            supplied.contains(&"login"),
            "supplied must contain 'login': {supplied:?}"
        );
        assert!(
            supplied.contains(&"verbosity"),
            "supplied must contain 'verbosity': {supplied:?}"
        );
        assert_eq!(
            supplied.len(),
            2,
            "exactly two flags supplied: {supplied:?}"
        );
    }

    #[test]
    fn test_collect_supplied_returns_empty_when_none_supplied() {
        let login_arg = DOCKER_LOGIN_LOGIN.to_arg();
        let verbosity_arg = DOCKER_LOGIN_VERBOSITY.to_arg();
        let cmd = clap::Command::new("test").arg(login_arg).arg(verbosity_arg);
        let matches = cmd
            .try_get_matches_from(["test"])
            .expect("must parse without flags");
        let supplied =
            collect_supplied_flags(&matches, &[&DOCKER_LOGIN_LOGIN, &DOCKER_LOGIN_VERBOSITY]);
        assert!(
            supplied.is_empty(),
            "no flags supplied, but got: {supplied:?}"
        );
    }
}
