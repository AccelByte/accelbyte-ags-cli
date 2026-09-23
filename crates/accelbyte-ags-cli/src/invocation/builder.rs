//! Build dynamic Clap command trees from `ServiceSchema`.

use clap::{Arg, Command};

use crate::frontend::style;
use crate::invocation::flags::LeafSelectors;
use ags_protocol::catalogue::ServiceSchema;
use ags_runtime::catalogue::Catalogue;

// ── Public entry points ──

/// Build the root command structure (name, help, about, args, and standalone
/// subcommands) without any service subcommands attached. Both the lazy
/// routing path (`build_root_command`) and the fully-populated completion
/// path (`build_full_command`) extend this shared shell with different
/// service-subcommand strategies.
fn build_root_shell() -> Command {
    let mut services_lines: Vec<String> = Vec::new();
    let col_width = Catalogue::service_ids()
        .map(|service| Catalogue::display_name_or_panic(service).len())
        .max()
        .unwrap_or(0)
        + 4;

    for service in Catalogue::service_ids() {
        let display = Catalogue::display_name_or_panic(service);
        let desc = Catalogue::service_description(service);
        let padding = " ".repeat(col_width - display.len());
        services_lines.push(format!(
            "  {}{padding}{desc}",
            style::styled_literal(display)
        ));
    }

    let services_section = services_lines.join("\n");

    let root_after_help = format!(
        "{}:\n  \
         ags auth login\n  \
         ags iam users search --namespace my-game\n  \
         ags platform items create --namespace my-game --store-id main --json @item.json\n\
         \n\
         {}:\n  \
         0 = success\n  \
         1 = usage error\n  \
         2 = auth error\n  \
         3 = API error\n  \
         4 = network error\n  \
         5 = internal error\n\
         \n\
         {}:\n  \
         Docs:      https://docs.accelbyte.io/\n  \
         Feedback:  https://github.com/AccelByte/accelbyte-ags-cli/issues\n",
        style::styled_header("Examples"),
        style::styled_header("Exit codes"),
        style::styled_header("Links"),
    );

    let root_help_template = format!(
        "{{about-with-newline}}\n\
         {{usage-heading}}\n  {{usage}}\n\n\
         {}:\n\
         {{subcommands}}\n\n\
         {}:\n\
         {services_section}\n\n\
         {}:\n\
         {{options}}\
         {{after-help}}",
        style::styled_header("Commands (standalone)"),
        style::styled_header("Services (API groups)"),
        style::styled_header("Flags"),
    );

    let mut root = Command::new("ags")
        .version(env!("CARGO_PKG_VERSION"))
        .help_template(root_help_template)
        .about(
            "AccelByte Gaming Services CLI\n\n\
                Manage your AccelByte backend from the terminal. Authenticate,\n\
                call any admin API, and automate cross-service workflows.",
        )
        .override_usage(format!(
            "{} [FLAGS] <COMMAND> [OPTIONS]\n  {} [FLAGS] <SERVICE> <RESOURCE> <METHOD> [OPTIONS]",
            style::styled_literal("ags"),
            style::styled_literal("ags")
        ))
        .after_help(root_after_help)
        .args(global_flag_args())
        .subcommand_required(false)
        .arg_required_else_help(true)
        .disable_help_subcommand(true);

    root = root.subcommand(build_auth_command());
    root = root.subcommand(build_completions_command());
    root = root.subcommand(build_config_command());
    root = root.subcommand(build_profile_command());
    root = root.subcommand(build_describe_command());
    root = root.subcommand(build_doctor_command());
    root = root.subcommand(build_refresh_specs_command());
    root = root.subcommand(build_update_command());
    root = root.subcommand(build_extend_command());

    root
}

/// Build the root command listing all services.
///
/// Services are registered as hidden empty stubs: routing still works
/// (the router matches on the service name string), but the clap tree
/// carries no resources or operations for them. This keeps startup fast
/// because each service's full tree is built lazily only when the router
/// dispatches to it.
pub fn build_root_command() -> Command {
    let mut root = build_root_shell();
    // Flat workflow command (positional `<workflow-id>`): the routing/help path
    // parses the id and per-workflow flags itself, so it needs no expansion.
    root = root.subcommand(build_workflow_command());

    for service in Catalogue::service_ids() {
        let display = Catalogue::display_name_or_panic(service);
        let desc = Catalogue::service_description(service);
        root = root.subcommand(Command::new(display).about(desc).hide(true));
    }

    root
}

/// Build a fully-populated root command with every service's resources and
/// operations attached. Intended for shell-completion generation, where
/// `clap_complete::generate` must walk the complete tree.
///
/// If a service's bundled spec fails to load, falls back to the same hidden
/// stub used by `build_root_command` so completion generation still
/// succeeds for the remaining services.
pub fn build_full_command() -> Command {
    let mut root = build_root_shell();
    // Completion-only expansion: one `run` subcommand per registered workflow,
    // each carrying its own `--<input>` flags, so the shell can complete both
    // the workflow id and its flags.
    root = root.subcommand(build_workflow_completion_command());

    for internal in Catalogue::service_ids() {
        let display = Catalogue::display_name_or_panic(internal);
        let desc = Catalogue::service_description(internal);
        let service_command = match Catalogue::load_bundled(internal) {
            Ok(schema) => {
                crate::invocation::routes::service::clap_tree::build_service_command_tree(
                    &schema,
                    &LeafSelectors::default(),
                )
            }
            Err(e) => {
                // Bundled specs are embedded via include_bytes! — a load
                // failure means a corrupt binary, not a runtime condition.
                // Warn visibly and fall back to a stub so completions
                // succeed for the other services.
                crate::frontend::write_stderr_line(&format!(
                    "ags: warning: could not load spec for '{display}' ({e}); completions for this service will be incomplete",
                ));
                Command::new(display.to_string()).about(desc).hide(true)
            }
        };
        root = root.subcommand(service_command);
    }

    root
}

/// Build a service command with resource subcommands.
pub fn build_service_command_tree(schema: &ServiceSchema, selectors: &LeafSelectors) -> Command {
    crate::invocation::routes::service::clap_tree::build_service_command_tree(schema, selectors)
}

/// Build the auth command using Clap's default styling.
pub fn build_auth_command() -> Command {
    let auth_help_template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    Command::new("auth")
        .help_template(auth_help_template.clone())
        .about("Authentication management")
        .disable_help_subcommand(true)
        .arg_required_else_help(true)
        .subcommand_required(true)
        .subcommand(build_login_subcommand())
        .subcommand(
            Command::new("logout")
                .help_template(auth_help_template.clone())
                .about("Log out and clear credentials")
                .long_about(
                    "Log out and clear credentials\n\n\
                     \x20 Removes stored tokens, client secret, and refresh token from the\n\
                     \x20 OS keychain. Clears the client ID from the config file. The base\n\
                     \x20 URL is preserved.\n\n\
                     \x20 Use --all to log out from every profile at once.\n\n\
                     \x20 With --format json, outputs {\"status\": \"cleared\"} on success.",
                )
                .arg(
                    Arg::new("all")
                        .long("all")
                        .help("Log out from all profiles")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("status")
                .help_template(auth_help_template.clone())
                .about("Show current authentication status")
                .long_about(
                    "Show current authentication status\n\n\
                     \x20 Displays the active authentication source (environment variables\n\
                     \x20 or stored credentials), base URL, client ID, token expiry time,\n\
                     \x20 and whether a refresh token is available.\n\n\
                     \x20 With --format json, outputs full auth state as machine-readable JSON.",
                ),
        )
        .subcommand(
            Command::new("token")
                .help_template(auth_help_template.clone())
                .about("Print the current access token to stdout")
                .long_about(
                    "Print the current access token to stdout\n\n\
                     \x20 Prints a secret. The token is written to stdout on its own, with\n\
                     \x20 nothing else, so a script can reuse this session instead of running\n\
                     \x20 its own login:\n\n\
                     \x20   curl -H \"Authorization: Bearer $(ags auth token)\" ...\n\n\
                     \x20 The token is resolved exactly as an API call resolves it:\n\
                     \x20 AGS_ACCESS_TOKEN first, then the stored token, refreshing it when\n\
                     \x20 it has expired. Exits 2 with login guidance when authentication\n\
                     \x20 fails, or 4 when the identity service cannot be reached. Stdout is\n\
                     \x20 empty in both cases.\n\n\
                     \x20 --dry-run is refused because the command's only output would be a\n\
                     \x20 live credential.\n\n\
                     \x20 Redirect stdout with care: anything that captures it captures a\n\
                     \x20 live credential.\n\n\
                     \x20 With --format json, outputs the token alongside its expiry and\n\
                     \x20 source.",
                ),
        )
        .subcommand(
            Command::new("refresh")
                .help_template(auth_help_template)
                .about("Refresh the access token using stored credentials")
                .long_about(
                    "Refresh the access token using stored credentials\n\n\
                     \x20 Re-mints the access token without a full browser login. Client-credentials\n\
                     \x20 profiles re-run the client-credentials grant. Authorization-code profiles\n\
                     \x20 force a refresh-token exchange using the stored refresh token.\n\n\
                     \x20 If your permissions changed (for example new roles were added), you may\n\
                     \x20 still need to run 'ags auth login' again for them to take effect.\n\n\
                     \x20 With --format json, outputs the refreshed status, base URL, client ID, and\n\
                     \x20 token expiry.",
                ),
        )
}

/// Build the `ags profile` command tree
pub fn build_profile_command() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    Command::new("profile")
        .help_template(template.clone())
        .about("Profile management")
        .disable_help_subcommand(true)
        .arg_required_else_help(true)
        .subcommand_required(true)
        .subcommand(
            Command::new("list")
                .help_template(template.clone())
                .about("List all profiles"),
        )
        .subcommand(
            Command::new("create")
                .help_template(template.clone())
                .about("Create a new profile")
                .arg(Arg::new("name").required(true).help("Profile name")),
        )
        .subcommand(
            Command::new("use")
                .help_template(template.clone())
                .about("Set the active profile")
                .arg(Arg::new("name").required(true).help("Profile name")),
        )
        .subcommand(
            Command::new("show")
                .help_template(template.clone())
                .about("Show profile configuration")
                .arg(Arg::new("name").help("Profile name (defaults to active profile)")),
        )
        .subcommand(
            Command::new("delete")
                .help_template(template.clone())
                .about("Delete a profile and its stored configuration and credentials")
                .arg(Arg::new("name").required(true).help("Profile name")),
        )
        .subcommand(
            Command::new("rename")
                .help_template(template)
                .about("Rename a profile")
                .arg(Arg::new("old").required(true).help("Current profile name"))
                .arg(Arg::new("new").required(true).help("New profile name")),
        )
}

/// Build the `completions` subcommand for argument parsing and help display.
pub fn build_completions_command() -> Command {
    use clap::builder::{PossibleValuesParser, TypedValueParser};
    use clap_complete::Shell;

    let shell_parser = PossibleValuesParser::new(["bash", "zsh", "fish", "powershell"]).map(|s| {
        s.parse::<Shell>()
            .expect("PossibleValuesParser gates the set")
    });

    Command::new("completions")
        .about("Generate a shell completion script")
        .arg(
            Arg::new("shell")
                .value_name("SHELL")
                .help(
                    "Shell to generate completions for. \
                     If omitted, detected from $SHELL (or PowerShell on Windows).",
                )
                .value_parser(shell_parser)
                .required(false),
        )
        .after_help(
            "Examples:\n  \
             source <(ags completions zsh)\n  \
             source <(ags completions bash)\n  \
             ags completions fish | source\n  \
             ags completions powershell | Out-String | Invoke-Expression",
        )
}

/// Generate the "Profile keys: ... / Global keys: ..." help fragment from the
/// config key registry so the lists stay in sync with `KNOWN_KEYS` in
/// `ags-runtime` and never drift.
fn config_key_help_lines() -> String {
    use ags_runtime::runtime::config::{ConfigScope, KNOWN_KEYS};

    let profile_keys: Vec<&str> = KNOWN_KEYS
        .iter()
        .filter(|k| k.scope == ConfigScope::Profile)
        .map(|k| k.cli_name)
        .collect();
    let global_keys: Vec<&str> = KNOWN_KEYS
        .iter()
        .filter(|k| k.scope == ConfigScope::Global)
        .map(|k| k.cli_name)
        .collect();

    format!(
        " Profile keys: {}\n Global keys:  {}",
        profile_keys.join(", "),
        global_keys.join(", "),
    )
}

/// Build the `ags config` command tree
pub fn build_config_command() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    Command::new("config")
        .help_template(template.clone())
        .about("Configuration management")
        .disable_help_subcommand(true)
        .arg_required_else_help(true)
        .subcommand_required(true)
        .subcommand(
            Command::new("get")
                .help_template(template.clone())
                .about("Get a configuration value")
                .long_about(
                    "Get a configuration value\n\n\
                     \x20 Run without a key to show all configuration values and their sources.",
                )
                .arg(Arg::new("key").help("Config key (omit to show all)")),
        )
        .subcommand(
            Command::new("set")
                .help_template(template.clone())
                .about("Set a configuration value")
                .long_about(format!(
                    "Set a configuration value\n\n\
                     \x20{}\n\n\
                     \x20 Scope is auto-detected from the key. Use --global or --profile to override.",
                    config_key_help_lines(),
                ))
                .arg(Arg::new("key").required(true).help("Config key"))
                .arg(Arg::new("value").required(true).help("Value to set"))
                .arg(
                    Arg::new("global")
                        .long("global")
                        .help("Target global config (auto-detected for global keys)")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("unset")
                .help_template(template)
                .about("Remove a configuration value")
                .long_about(format!(
                    "Remove a configuration value\n\n\
                     \x20{}\n\n\
                     \x20 Scope is auto-detected from the key. Use --global or --profile to override.",
                    config_key_help_lines(),
                ))
                .arg(Arg::new("key").required(true).help("Config key"))
                .arg(
                    Arg::new("global")
                        .long("global")
                        .help("Target global config (auto-detected for global keys)")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
}

/// Build the standalone Clap subcommand for `ags describe`.
pub fn build_describe_command() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    Command::new("describe")
        .help_template(template)
        .about("Machine-readable command discovery and introspection (JSON)")
        .long_about(
            "Machine-readable command discovery and introspection (JSON)\n\n\
             \x20 Pass parts of a command path to narrow the output:\n\n\
             \x20   ags describe                       outputs all services\n\
             \x20   ags describe iam                   outputs resources within a service\n\
             \x20   ags describe iam users             outputs methods within a resource\n\
             \x20   ags describe iam users search      outputs full parameter schema for a method\n\n\
             \x20 Registered workflows have their own path:\n\n\
             \x20   ags describe workflow              lists registered multi-step workflows\n\
             \x20   ags describe workflow <id>         outputs a workflow's inputs and steps\n\n\
             \x20 Output is always JSON. No authentication required.",
        )
        .arg(Arg::new("service").help("Service name"))
        .arg(Arg::new("resource").help("Resource name"))
        .arg(Arg::new("method").help("Method name"))
}

/// Build the `ags doctor` command
pub fn build_doctor_command() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    Command::new("doctor")
        .help_template(template)
        .about("Check environment, configuration, and connectivity")
        .long_about(
            "Check environment, configuration, and connectivity\n\n\
             \x20 Exit code 0 if all checks pass or warn, 1 if any check fails. The\n\
             \x20 --all flag overrides --profile and runs against every profile.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("offline")
                .long("offline")
                .help("Skip network checks")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("all")
                .long("all")
                .help("Check all profiles")
                .action(clap::ArgAction::SetTrue),
        )
}

/// Build the `refresh-specs` subcommand.
pub fn build_refresh_specs_command() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    Command::new("refresh-specs")
        .help_template(template)
        .about("Rebuild the parsed-schema cache from bundled specs")
        .long_about(
            "Rebuild the parsed-schema cache from bundled specs.\n\n\
             Without an argument, clears the cache directory and rebuilds\n\
             every service. With a service argument, rebuilds just that\n\
             service's cache.\n\n\
             Use this after updating the bundled specs or when the CLI\n\
             reports a stale or corrupt cache.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("service")
                .help("Optional service name; omit to refresh all services")
                .required(false),
        )
}

/// Build the `ags update` command.
pub fn build_update_command() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    Command::new("update")
        .help_template(template)
        .about("Check for a newer release and show how to install it")
        .long_about(
            "Check for a newer release and show how to install it\n\n\
             \x20 Asks GitHub for the latest release, compares it with this build,\n\
             \x20 and prints the upgrade instruction for the way this copy was\n\
             \x20 installed. It does not download or modify anything.\n\n\
             \x20 With --install, downloads the newest release's installer script\n\
             \x20 and runs it for this copy, after asking for confirmation. Never\n\
             \x20 runs unless typed. When an update is available, refuses for a copy\n\
             \x20 installed with Homebrew; run brew upgrade instead.\n\n\
             \x20 Exit code 0 in both outcomes; 4 when GitHub cannot be reached,\n\
             \x20 answers with an error status, or answers with a tag that is not a release version.\n\n\
             \x20 With --install: exit 0 when installed or already current; 1 when\n\
             \x20 --no-input is given without --yes or the copy was installed with\n\
             \x20 Homebrew; 2 when the confirmation is declined or the upgrade is\n\
             \x20 interrupted; 4 when the installer script cannot be downloaded;\n\
             \x20 5 when it cannot be saved, the installer fails, the new binary\n\
             \x20 does not verify, or the previous binary could not be restored.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("install")
                .long("install")
                .action(clap::ArgAction::SetTrue)
                .help(
                    "Download the newest release's installer script and run it \
                     for this copy, after asking for confirmation",
                ),
        )
}

/// Build the `ags extend` command tree: Extend-platform tooling and
/// migration shortcuts.
///
/// Migration shortcut subcommands are added by iterating the static
/// registration table so that renaming a shortcut costs one string edit
/// in `service_shims.rs` and zero edits here.
pub fn build_extend_command() -> Command {
    use crate::invocation::handlers::extend::service_shims;

    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}{after-help}"
        .to_string();

    let cmd = Command::new("extend")
        .help_template(template)
        .about("Extend platform tooling")
        .long_about(
            "Extend platform tooling\n\n\
             \x20 Extend specific commands and shortcuts to ease migration.",
        )
        .disable_help_subcommand(true)
        .arg_required_else_help(true)
        .subcommand_required(true)
        .subcommand(build_clone_template_subcommand())
        .subcommand(build_docker_login_subcommand())
        .subcommand(build_image_upload_subcommand())
        .subcommand(build_tunnel_subcommand())
        .subcommand(build_update_secret_subcommand())
        .subcommand(build_update_var_subcommand());

    // The shim layer creates hidden parent groups (e.g. `app-ui` for the
    // `create` migration shortcut, `security-assessment` for `list` /
    // `list-endpoints`). After shim registration, promote each to visible
    // and add its native (non-shimmed) subcommands.
    let cmd = service_shims::add_shim_subcommands(cmd);
    let cmd = cmd.mut_subcommand("app-ui", |sub| {
        sub.hide(false)
            .about("App UI commands")
            .subcommand(build_setup_env_subcommand())
            .subcommand(build_upload_subcommand())
    });
    let cmd = cmd.mut_subcommand("security-assessment", |sub| {
        sub.hide(false)
            .about("Pen-testing engagement requests and reports for Extend apps")
            .subcommand(build_security_assessment_result_subcommand())
            .subcommand(build_security_assessment_request_subcommand())
    });

    // Build the `remote-debug` group natively with all three subcommands.
    // The `disable` shim was removed from the SHIMS table, so no shim
    // creates this group — it must be built here directly.
    cmd.subcommand(build_remote_debug_subcommand())
}

/// Build the `clone-template` subcommand under `extend`.
fn build_clone_template_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    Command::new("clone-template")
        .help_template(template)
        .about("Clone a starter template for Extend apps")
        .long_about(
            "Clone a starter template for Extend apps.\n\n\
             Without --template, prompts interactively through scenario,\n\
             template, and language selection. Use --template for CI/scripted use.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("template")
                .long("template")
                .help("Select a template by name (non-interactive)")
                .value_name("name"),
        )
        .arg(
            Arg::new("destination")
                .long("destination")
                .short('d')
                .help("Destination directory")
                .value_name("path"),
        )
        .arg(
            Arg::new("depth")
                .long("depth")
                .help("Shallow clone depth (default: 1, 0 for full)")
                .value_name("n")
                .value_parser(clap::value_parser!(u32)),
        )
}

/// Build the `setup-env` subcommand under `app-ui`.
fn build_setup_env_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    Command::new("setup-env")
        .help_template(template)
        .about("Set up the .env.local file for an App UI project")
        .long_about(
            "Set up the .env.local file for an App UI project.\n\n\
             Reads the App UI record from CSM and writes the four VITE_AB_*\n\
             environment variables into .env.local in the project directory.\n\n\
             If .env.example exists, its contents are used as a template and\n\
             only the managed keys are replaced or appended. Comments, blank\n\
             lines, and unmanaged keys are preserved.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("name")
                .long("name")
                .help("App UI name")
                .value_name("name")
                .required(true),
        )
        .arg(
            Arg::new("project-path")
                .long("project-path")
                .help("Project directory (default: current directory)")
                .value_name("path")
                .default_value("."),
        )
        .arg(
            Arg::new("force")
                .long("force")
                .help("Overwrite existing .env.local")
                .action(clap::ArgAction::SetTrue),
        )
}

/// Build the `upload` subcommand under `app-ui`.
///
/// Five flags: `--name` (required), `--project-path`, `--build-path`,
/// `--build-version`, `--no-build`. The `--verbosity` compat flag is
/// registered through the shared `CompatFlag` mechanism. `--namespace`
/// is a global flag and is NOT registered on this command.
fn build_upload_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}{after-help}"
        .to_string();

    Command::new("upload")
        .help_template(template)
        .about("Build and upload an App UI static-asset bundle")
        .long_about(
            "Build and upload an App UI static-asset bundle.\n\n\
             Runs the frontend build, archives the output directory into a zip,\n\
             and uploads the archive to CSM. With --no-build, skips the build step\n\
             and archives the existing build output directly.",
        )
        .after_help(
            "Global flags:\n  \
             -n, --namespace <namespace>  Game namespace\n      \
             --dry-run                    Preview without executing\n\n\
             Use 'ags --help' for all global flags.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("name")
                .long("name")
                .help("App UI name")
                .value_name("name")
                .required(true),
        )
        .arg(
            Arg::new("project-path")
                .long("project-path")
                .help("Project directory (default: current directory)")
                .value_name("path")
                .default_value("."),
        )
        .arg(
            Arg::new("build-path")
                .long("build-path")
                .help("Build output directory, relative to project path (default: dist)")
                .value_name("path")
                .default_value("dist"),
        )
        .arg(
            Arg::new("build-version")
                .long("build-version")
                .help("Build version identifier (default: random 8-char hex)")
                .value_name("version"),
        )
        .arg(
            Arg::new("no-build")
                .long("no-build")
                .help("Skip the frontend build step")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(crate::invocation::compat_flags::APP_UI_UPLOAD_VERBOSITY.to_arg())
}

/// Build the `docker-login` subcommand under `extend`.
///
/// Six flags matching the Go `extend-helper-cli` surface: `--namespace`,
/// `--app`, `--print`, `--print-format`, plus two compat flags (`--login`,
/// `--verbosity`) registered via the shared [`compat_flags`] mechanism so
/// a notice is emitted when either is explicitly supplied.
///
/// `--print-format` (formerly `--format`) was renamed to avoid shadowing
/// the global `--format` flag, which controls the output envelope.
fn build_docker_login_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}{after-help}"
        .to_string();

    Command::new("docker-login")
        .help_template(template)
        .about("Log in to the Extend container registry")
        .long_about(
            "Log in to the Extend container registry.\n\n\
             Fetches short-lived registry credentials from the Extend Helper\n\
             Service and passes them to `docker login --password-stdin`.\n\n\
             With --print, writes the credentials to stdout instead of\n\
             running Docker.",
        )
        .after_help(
            "Global flags:\n  \
             -n, --namespace <namespace>  Game namespace\n\n\
             Use 'ags --help' for all global flags.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("app")
                .long("app")
                .short('a')
                .help("Extend app name")
                .value_name("app")
                .required(true),
        )
        .arg(
            Arg::new("print")
                .long("print")
                .short('p')
                .help("Print credentials to stdout instead of running docker login")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("print-format")
                .long("print-format")
                .help("Output format for --print")
                .value_name("json|token")
                .default_value("json"),
        )
        .arg(crate::invocation::compat_flags::DOCKER_LOGIN_LOGIN.to_arg())
        .arg(crate::invocation::compat_flags::DOCKER_LOGIN_VERBOSITY.to_arg())
}

/// Build the `security-assessment` subcommand under `extend`, with its
/// `list`, `list-endpoints`, `result`, and `request` actions.
///
/// `list` and `list-endpoints` are registered as `service_shims` entries
/// instead, rewriting straight to `ags csm security-assessment
/// {list,get-app-endpoints}`. `result` and `request` are native.
///
/// Build the `result` subcommand under `security-assessment`.
fn build_security_assessment_result_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}{after-help}"
        .to_string();

    Command::new("result")
        .help_template(template)
        .about("Download a completed security-assessment engagement's report")
        .after_help(
            "Global flags:\n  \
             -n, --namespace <namespace>  Game namespace\n\n\
             Use 'ags --help' for all global flags.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("app")
                .long("app")
                .short('a')
                .help("Extend app name")
                .value_name("app")
                .required(true),
        )
        .arg(
            Arg::new("report-format")
                .long("report-format")
                .help("Report file format")
                .value_name("pdf|md")
                .default_value("pdf"),
        )
        .arg(
            // Named `--report-output`, not `--output` — the latter is
            // already a reserved global flag (writes a response body to a
            // file), pre-scanned out of argv before any subcommand flag is
            // parsed. Same naming-collision fix as `--report-format` above.
            Arg::new("report-output")
                .long("report-output")
                .short('o')
                .help("Local file path to write the report to (default: <app>-<engagementId>-report.<ext>)")
                .value_name("path"),
        )
        .arg(
            Arg::new("engagement-id")
                .long("engagement-id")
                .help("Engagement id to fetch the report for, skipping the interactive picker")
                .value_name("id"),
        )
}

/// Build the `request` subcommand under `security-assessment`.
///
/// Without `--all-endpoints`/`--operation-ids`, prompts interactively through
/// a single-screen endpoint checklist (requires a real terminal). `--permission`
/// is repeatable, one per endpoint needing a permission override.
fn build_security_assessment_request_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}{after-help}"
        .to_string();

    Command::new("request")
        .help_template(template)
        .about("Request a security assessment (pen-testing engagement) for an Extend app")
        .long_about(
            "Request a security assessment (pen-testing engagement) for an Extend app.\n\n\
             Discovers the app's testable endpoints and, without --all-endpoints or\n\
             --operation-ids, prompts interactively through a checklist to choose which\n\
             to include and to supply permissions where none was auto-discovered.\n\n\
             Warns before submitting if any selected endpoint can modify or delete data.\n\n\
             With --wait, blocks after submitting until the engagement reaches a terminal\n\
             state (COMPLETED or FAILED), polling every 10s up to --wait-limit seconds\n\
             (default 1800) and printing the current status each poll (suppressed by\n\
             --quiet). A Ctrl-C during the wait only stops the local wait — the engagement\n\
             keeps running; check its status later with\n\
             'ags extend security-assessment list --namespace <namespace>'.",
        )
        .after_help(
            "Global flags:\n  \
             -n, --namespace <namespace>  Game namespace\n      \
             --yes                        Skip the mutating-endpoint confirmation\n      \
             --dry-run                    Preview without requesting an assessment\n\n\
             Use 'ags --help' for all global flags.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("app")
                .long("app")
                .short('a')
                .help("Extend app name")
                .value_name("app")
                .required(true),
        )
        .arg(
            Arg::new("all-endpoints")
                .long("all-endpoints")
                .help("Select every discovered endpoint (non-interactive)")
                .action(clap::ArgAction::SetTrue)
                .conflicts_with("operation-ids"),
        )
        .arg(
            Arg::new("operation-ids")
                .long("operation-ids")
                .help("Select exactly these endpoints by operation id (non-interactive)")
                .value_name("id1,id2,..."),
        )
        .arg(
            Arg::new("permission")
                .long("permission")
                .help("Permission override for one endpoint, repeatable")
                .value_name("operationId=RESOURCE [ACTION]")
                .action(clap::ArgAction::Append),
        )
        .arg(
            Arg::new("wait")
                .long("wait")
                .help("Block until the engagement reaches a terminal state (COMPLETED or FAILED)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("wait-limit")
                .long("wait-limit")
                .help("Maximum seconds to wait before giving up (default 1800)")
                .value_name("SECONDS")
                .value_parser(clap::value_parser!(u64))
                .requires("wait"),
        )
}

/// Build the `update-var` subcommand under `extend`.
///
/// Five flags: `--key`, `--value` (both required), `--description`,
/// `--sensitive` (bool, default false — presence checked via
/// `ArgMatches::value_source` to distinguish "not supplied" from
/// "supplied"), `--force`. `--app`/`-a` is required, matching
/// `docker-login`'s pattern. `--namespace`/`-n` is the global flag and is
/// NOT registered here.
fn build_update_var_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}{after-help}"
        .to_string();

    Command::new("update-var")
        .help_template(template)
        .about("Update or create a CSM app configuration variable")
        .long_about(
            "Update or create a CSM app configuration variable.\n\n\
             Updates the variable named --key to --value if it exists. If it\n\
             does not exist, pass --force to create it. --sensitive and\n\
             --description are merged with the existing record when not\n\
             explicitly supplied: an unset --sensitive preserves the existing\n\
             mask, an unset --description preserves the existing description.\n\
             Pass --sensitive false explicitly to remove masking.\n\n\
             Prefer --value-stdin over --value to avoid exposing the value\n\
             in shell history.",
        )
        .after_help(
            "Global flags:\n  \
             -n, --namespace <namespace>  Game namespace\n      \
             --dry-run                    Preview without executing\n\n\
             Use 'ags --help' for all global flags.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("app")
                .long("app")
                .short('a')
                .help("Extend app name")
                .value_name("app")
                .required(true),
        )
        .arg(
            Arg::new("key")
                .long("key")
                .help("Variable name")
                .value_name("key")
                .required(true),
        )
        .arg(
            Arg::new("value")
                .long("value")
                .help("Variable value (visible in shell history)")
                .value_name("value")
                .required_unless_present("value-stdin")
                .conflicts_with("value-stdin")
                .allow_hyphen_values(true),
        )
        .arg(
            Arg::new("value-stdin")
                .long("value-stdin")
                .help("Read variable value from stdin")
                .action(clap::ArgAction::SetTrue)
                .conflicts_with("value"),
        )
        .arg(
            Arg::new("description")
                .long("description")
                .help("Variable description (preserved from the existing record if not supplied)")
                .value_name("text"),
        )
        .arg(
            Arg::new("sensitive")
                .long("sensitive")
                .help(
                    "Mask the variable's value (defaults to false when creating; preserved from \
                     the existing record on update if not supplied; pass 'false' explicitly to \
                     remove masking)",
                )
                .value_parser(clap::value_parser!(bool))
                .num_args(0..=1)
                .default_missing_value("true"),
        )
        .arg(
            Arg::new("force")
                .long("force")
                .help("Create the variable if --key does not name an existing one")
                .action(clap::ArgAction::SetTrue),
        )
}

/// Build the `image-upload` subcommand under `extend`.
///
/// Nine flags matching the Go `extend-helper-cli` surface: `--app`,
/// `--image-tag`, `--dockerfile`, `--platform`, `--work-dir`, `--login`,
/// `--retry-limit`, `--retry-interval`, `--retry-rate`.
///
/// `--namespace`, `--dry-run`, and `--format` are globals and are NOT
/// registered on this command. `--verbosity` is deferred to a separate PR.
fn build_image_upload_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}{after-help}"
        .to_string();

    Command::new("image-upload")
        .help_template(template)
        .about("Build and push a container image to the Extend registry")
        .long_about(
            "Build and push a container image to the Extend registry.\n\n\
             Builds a container image from a Dockerfile and pushes it to the\n\
             Extend container registry for the specified app. Requires Docker\n\
             (or Podman) to be installed and on PATH.\n\n\
             With --login, authenticates to the registry before building.\n\
             Without --login, assumes the registry is already authenticated.",
        )
        .after_help(
            "Global flags:\n  \
             -n, --namespace <namespace>  Game namespace\n      \
             --dry-run                    Preview without executing\n\n\
             Use 'ags --help' for all global flags.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("app")
                .long("app")
                .short('a')
                .help("Extend app name")
                .value_name("app")
                .required(true),
        )
        .arg(
            Arg::new("image-tag")
                .long("image-tag")
                .short('t')
                .help("Image tag to build and push")
                .value_name("tag")
                .required(true)
                .value_parser(parse_docker_tag),
        )
        .arg(
            Arg::new("dockerfile")
                .long("dockerfile")
                .short('f')
                .help("Path to the Dockerfile")
                .value_name("path")
                .default_value("Dockerfile"),
        )
        .arg(
            Arg::new("platform")
                .long("platform")
                .short('p')
                .help("Target platform(s)")
                .value_name("platform")
                .action(clap::ArgAction::Append)
                .default_value("linux/amd64"),
        )
        .arg(
            Arg::new("work-dir")
                .long("work-dir")
                .short('w')
                .help("Build context directory")
                .value_name("path"),
        )
        .arg(
            Arg::new("login")
                .long("login")
                .short('l')
                .help("Authenticate to the registry before building")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("retry-limit")
                .long("retry-limit")
                .help("Number of retries on failure (0 = no retries)")
                .value_name("n")
                .default_value("0")
                .value_parser(clap::value_parser!(u32)),
        )
        .arg(
            Arg::new("retry-interval")
                .long("retry-interval")
                .help("Base interval between retries in seconds")
                .value_name("seconds")
                .default_value("1.0")
                .allow_hyphen_values(true)
                .value_parser(parse_finite_nonneg_f64),
        )
        .arg(
            Arg::new("retry-rate")
                .long("retry-rate")
                .help("Exponential backoff rate multiplier")
                .value_name("rate")
                .default_value("2.0")
                .allow_hyphen_values(true)
                .value_parser(parse_finite_nonneg_f64),
        )
}

/// Build the `update-secret` subcommand under `extend`.
///
/// Six flags: `--key` (required), `--value` / `--value-stdin` (exactly one
/// required), `--description`, `--sensitive` (bool-valued via
/// `num_args(0..=1)` + `default_missing_value("true")` — the same shape
/// as `update-var`; only the CREATE default differs: `true` here vs `false`
/// for `update-var`), `--force`. `--app`/`-a` is required.
/// `--namespace`/`-n` is the global flag and is NOT registered here.
///
/// `--value-stdin` mirrors the `--client-secret-stdin` pattern from
/// `build_login_subcommand()`: mutually exclusive with `--value`, reads a
/// single line from stdin so the plaintext never appears in shell history.
fn build_update_secret_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}{after-help}"
        .to_string();

    Command::new("update-secret")
        .help_template(template)
        .about("Update or create a CSM app secret")
        .long_about(
            "Update or create a CSM app secret.\n\n\
             Updates the secret named --key to --value if it exists. If it\n\
             does not exist, pass --force to create it. --sensitive and\n\
             --description are merged with the existing record when not\n\
             explicitly supplied: an unset --sensitive preserves the existing\n\
             mask (or defaults to true on create), an unset --description\n\
             preserves the existing description.\n\n\
             Prefer --value-stdin over --value to avoid exposing the secret\n\
             in shell history.",
        )
        .after_help(
            "Global flags:\n  \
             -n, --namespace <namespace>  Game namespace\n      \
             --dry-run                    Preview without executing\n\n\
             Use 'ags --help' for all global flags.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("app")
                .long("app")
                .short('a')
                .help("Extend app name")
                .value_name("app")
                .required(true),
        )
        .arg(
            Arg::new("key")
                .long("key")
                .help("Secret name")
                .value_name("key")
                .required(true),
        )
        .arg(
            Arg::new("value")
                .long("value")
                .help("Secret value (insecure — visible in shell history)")
                .value_name("value")
                .required_unless_present("value-stdin")
                .conflicts_with("value-stdin")
                .allow_hyphen_values(true),
        )
        .arg(
            Arg::new("value-stdin")
                .long("value-stdin")
                .help("Read secret value from stdin")
                .action(clap::ArgAction::SetTrue)
                .conflicts_with("value"),
        )
        .arg(
            Arg::new("description")
                .long("description")
                .help("Secret description (preserved from the existing record if not supplied)")
                .value_name("text"),
        )
        .arg(
            Arg::new("sensitive")
                .long("sensitive")
                .help(
                    "Mask the secret's value (defaults to true; preserved from the \
                     existing record on update if not supplied; pass 'false' explicitly \
                     to remove masking)",
                )
                .value_parser(clap::value_parser!(bool))
                .num_args(0..=1)
                .default_missing_value("true"),
        )
        .arg(
            Arg::new("force")
                .long("force")
                .help("Create the secret if --key does not name an existing one")
                .action(clap::ArgAction::SetTrue),
        )
}

/// Build the `tunnel` subcommand under `extend`.
///
/// Three flags: `--resource-name` (required), `--local-port` (required, u16),
/// `--pod-name` (optional). `--namespace` is a global flag and is NOT
/// registered on this command.
fn build_tunnel_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}{after-help}"
        .to_string();

    Command::new("tunnel")
        .help_template(template)
        .about("Open a TCP tunnel to an Extend app pod")
        .long_about(
            "Open a TCP tunnel to an Extend app pod.\n\n\
             Binds a local TCP port and bridges connections to the CSM v2\n\
             tunnel endpoint via WebSocket. Each accepted connection resolves\n\
             a fresh access token and opens its own WebSocket session.\n\n\
             The tunnel runs until Ctrl-C.",
        )
        .after_help(
            "Global flags:\n  \
             -n, --namespace <namespace>  Game namespace\n\n\
             Use 'ags --help' for all global flags.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("resource-name")
                .long("resource-name")
                .help("Extend resource name to tunnel to")
                .value_name("name")
                .required(true),
        )
        .arg(
            Arg::new("local-port")
                .long("local-port")
                .help("Local TCP port to bind (localhost only)")
                .value_name("port")
                .required(true)
                .value_parser(clap::value_parser!(u16)),
        )
        .arg(
            Arg::new("pod-name")
                .long("pod-name")
                .help("Target pod name (optional)")
                .value_name("pod"),
        )
}

/// Build the `connect` subcommand under `remote-debug`.
///
/// Three flags: `--app` (required), `--local-grpc-port` (default
/// `"localhost:6565"`), `--local-http-port` (default `"localhost:8000"`).
/// `--namespace` is a global flag and is NOT registered on this command.
fn build_remote_debug_connect_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}{after-help}"
        .to_string();

    Command::new("connect")
        .help_template(template)
        .about("Connect to an Extend remote debug session")
        .long_about(
            "Connect to an Extend remote debug session.\n\n\
             Resolves debug info from the CSM API, evaluates preconditions,\n\
             and reconnects established sessions with exponential backoff.\n\n\
             Requires the app to be running and debug mode to be enabled\n\
             (see 'ags extend remote-debug enable').",
        )
        .after_help(
            "Global flags:\n  \
             -n, --namespace <namespace>  Game namespace\n\n\
             Use 'ags --help' for all global flags.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("app")
                .long("app")
                .short('a')
                .help("Extend app name")
                .value_name("app")
                .required(true),
        )
        .arg(
            Arg::new("local-grpc-port")
                .long("local-grpc-port")
                .help("Local gRPC address or bare port")
                .value_name("addr")
                .default_value("localhost:6565"),
        )
        .arg(
            Arg::new("local-http-port")
                .long("local-http-port")
                .help("Local HTTP address or bare port")
                .value_name("addr")
                .default_value("localhost:8000"),
        )
}

/// Build the `enable` subcommand under `remote-debug`.
///
/// One required flag: `--app`. `--namespace` is a global flag.
fn build_remote_debug_enable_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}{after-help}"
        .to_string();

    Command::new("enable")
        .help_template(template)
        .about("Enable remote debugging for an Extend app")
        .long_about(
            "Enable remote debugging for an Extend app.\n\n\
             Emits a performance warning, checks whether the app is currently\n\
             running, and prompts for confirmation when it is (because enabling\n\
             debug mode restarts the app). Use --yes to skip the prompt.",
        )
        .after_help(
            "Global flags:\n  \
             -n, --namespace <namespace>  Game namespace\n  \
             -y, --yes                    Skip confirmation prompt\n\n\
             Use 'ags --help' for all global flags.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("app")
                .long("app")
                .short('a')
                .help("Extend app name")
                .value_name("app")
                .required(true),
        )
}

/// Build the `disable` subcommand under `remote-debug`.
///
/// One required flag: `--app`. `--namespace` is a global flag.
fn build_remote_debug_disable_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}{after-help}"
        .to_string();

    Command::new("disable")
        .help_template(template)
        .about("Disable remote debugging for an Extend app")
        .long_about(
            "Disable remote debugging for an Extend app.\n\n\
             Checks whether the app is currently running and prompts for\n\
             confirmation when it is (because disabling debug mode restarts\n\
             the app). Use --yes to skip the prompt.",
        )
        .after_help(
            "Global flags:\n  \
             -n, --namespace <namespace>  Game namespace\n  \
             -y, --yes                    Skip confirmation prompt\n\n\
             Use 'ags --help' for all global flags.",
        )
        .disable_help_subcommand(true)
        .arg(
            Arg::new("app")
                .long("app")
                .short('a')
                .help("Extend app name")
                .value_name("app")
                .required(true),
        )
}

/// Build the `remote-debug` command group with all three subcommands.
///
/// Previously the group was created implicitly by the shim table and
/// promoted to visible via `mut_subcommand`. Now that `disable` is a
/// native command (no longer a shim), the group is built directly here.
fn build_remote_debug_subcommand() -> Command {
    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    Command::new("remote-debug")
        .help_template(template)
        .about("Remote debug commands")
        .disable_help_subcommand(true)
        .arg_required_else_help(true)
        .subcommand_required(true)
        .subcommand(build_remote_debug_connect_subcommand())
        .subcommand(build_remote_debug_enable_subcommand())
        .subcommand(build_remote_debug_disable_subcommand())
}

/// Build the hand-written `ags ams upload` command.
///
/// Injected into the generated `ams` service tree by
/// `routes::service::clap_tree`, so it appears in `ags ams --help` and in
/// shell completions alongside the spec-derived resources. It is hand-written
/// because no OpenAPI operation describes it: the CLI archives a directory
/// locally, then ships it through pre-signed URLs.
pub fn build_ams_upload_command() -> Command {
    use ags_runtime::runtime::ams_upload::{TargetArchitecture, DEFAULT_PART_CONCURRENCY};

    let template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    Command::new("upload")
        .help_template(template)
        .about("Upload a dedicated-server image to AMS")
        .long_about(
            "Upload a dedicated-server image to AMS.\n\n\
             \x20 Archives the build directory, uploads it, and registers it as an AMS\n\
             \x20 image ready to be referenced by a fleet. Credentials and the platform\n\
             \x20 host come from your login or profile, as with every other command.\n\n\
             \x20 The entrypoint must be a 64-bit little-endian ELF binary (x86-64 or\n\
             \x20 aarch64), or a shell script — in which case --target-arch is required.\n\n\
             \x20 Requires the AMS:UPLOAD permission with Create and Update, entered\n\
             \x20 un-namespaced. That is a different permission from the AMS:IMAGE behind\n\
             \x20 'ags ams images', so being able to list images is not enough. For CI, use\n\
             \x20 a confidential IAM client with AGS_CLIENT_ID / AGS_CLIENT_SECRET.\n\n\
             \x20 --namespace is accepted but ignored: AMS derives the destination from\n\
             \x20 the access token, so images land in the namespace the client belongs to.",
        )
        .term_width(120)
        .bin_name("ags ams upload")
        .disable_help_subcommand(true)
        .arg(
            Arg::new("path")
                .long("path")
                .value_name("path")
                .help("Directory to upload")
                .default_value("."),
        )
        .arg(
            Arg::new("executable")
                .long("executable")
                .value_name("path")
                .help("Required. Entrypoint to run, relative to --path")
                .required(true),
        )
        .arg(
            Arg::new("image-name")
                .long("image-name")
                .value_name("name")
                .help("Required. Name of the image to create")
                .required(true),
        )
        .arg(
            Arg::new("target-arch")
                .long("target-arch")
                .value_name("arch")
                .help("Target architecture; required for a shell-script entrypoint")
                .value_parser(TargetArchitecture::all()),
        )
        .arg(
            Arg::new("symbol-files")
                .long("symbol-files")
                .help("Include debug-symbol files (.pdb, .sym, .debug) in the archive")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("skip-script-validation")
                .long("skip-script-validation")
                .help("Upload a shell-script entrypoint without validating it")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("upload-url")
                .long("upload-url")
                .value_name("url")
                .help("AMS upload host to use instead of discovering one"),
        )
        .arg(
            Arg::new("part-concurrency")
                .long("part-concurrency")
                .value_name("count")
                .help("Parts to upload at once for archives over 500 MiB")
                .value_parser(clap::value_parser!(u16).range(1..=32))
                .default_value(DEFAULT_PART_CONCURRENCY.to_string()),
        )
}

/// Build the `ags workflow` command and its `run` / `list` subcommands.
pub fn build_workflow_command() -> Command {
    let subcommand_template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    let run_template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}{after-help}"
        .to_string();

    Command::new("workflow")
        .help_template(subcommand_template.clone())
        .about("Run a registered multi-step workflow")
        .disable_help_subcommand(true)
        .subcommand_required(false)
        .subcommand(
            Command::new("run")
                .help_template(run_template)
                .about("Execute a registered workflow by id")
                .after_help(
                    "Each workflow declares its own --<input> flags. Run\n  \
                     ags workflow run <workflow-id> --help\nto see a specific \
                     workflow's flags.",
                )
                .disable_help_subcommand(true)
                .arg(
                    Arg::new("workflow-id")
                        .help("Id of the registered workflow to run")
                        .required(false),
                ),
        )
        .subcommand(
            Command::new("list")
                .help_template(subcommand_template.clone())
                .about("List registered workflows")
                .disable_help_subcommand(true),
        )
        .subcommand(
            Command::new("add")
                .help_template(subcommand_template.clone())
                .about("Validate and install a workflow YAML file")
                .long_about(
                    "Validate and install a workflow YAML file.\n\n\
                     The file is parsed and compiled, then copied into the CLI's\n\
                     config directory as <id>.yaml, named after its own `id:`\n\
                     field. That installed copy — not <path> — is what\n\
                     `workflow run`/`workflow list` use afterward, so editing\n\
                     <path> later has no effect until you run `add` again.\n\
                     Every file must declare a `workflow_protocol_version` field\n\
                     naming the workflow YAML protocol version it targets;\n\
                     `ags workflow template` fills this in automatically.\n\
                     Use --validate-only to check the file without installing it.",
                )
                .disable_help_subcommand(true)
                .arg(
                    Arg::new("path")
                        .required(true)
                        .help("Path to the workflow YAML file"),
                )
                .arg(
                    Arg::new("validate-only")
                        .long("validate-only")
                        .help("Validate without installing")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("template")
                .help_template(subcommand_template.clone())
                .about("Print or write a starter workflow YAML skeleton")
                .disable_help_subcommand(true)
                .arg(
                    Arg::new("output")
                        .long("output")
                        .value_name("PATH")
                        .help("Write the skeleton to this path instead of stdout"),
                ),
        )
        .subcommand(
            Command::new("remove")
                .help_template(subcommand_template.clone())
                .about("Remove an installed external workflow YAML file")
                .long_about(
                    "Remove a workflow previously installed via `ags workflow add`.\n\n\
                     Only external workflows installed under the CLI's config\n\
                     directory can be removed this way. Built-in workflows\n\
                     (bundled into the `ags` binary, whether Rust or YAML)\n\
                     cannot be removed and produce an error naming the\n\
                     workflow as built-in.",
                )
                .disable_help_subcommand(true)
                .arg(
                    Arg::new("id")
                        .required(true)
                        .help("Id of the installed workflow to remove"),
                ),
        )
}

/// Build the `ags workflow` command tree for shell-completion generation.
///
/// Same `list` subcommand as [`build_workflow_command`], but `run` is expanded
/// into one subcommand per registered workflow, each carrying that workflow's
/// own `--<input>` flags. This lets the shell complete both the workflow id
/// (`ags workflow run <TAB>`) and its flags (`ags workflow run season-pass
/// --<TAB>`).
///
/// The expansion is completion-only. The runtime routing and `--help` path use
/// the flat [`build_workflow_command`] with a positional `<workflow-id>`,
/// because `workflow run` parses its id and input flags itself (see
/// `routes::workflow`).
pub fn build_workflow_completion_command() -> Command {
    use ags_runtime::runtime::workflows::{compile::compile_workflow, registry};

    let subcommand_template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    let mut run = Command::new("run")
        .help_template(subcommand_template.clone())
        .about("Execute a registered workflow by id")
        .disable_help_subcommand(true);

    // Seed every bundled service into the catalogue up front so the
    // `get_or_load` calls inside `compile_workflow` hit the in-memory cache and
    // never fall through to the on-disk parsed-schema cache — which would do
    // disk I/O and block on (and print progress for) its file lock. Completion
    // generation must stay pure and offline. A corrupt bundled spec is surfaced
    // by the service loop in `build_full_command`; here it is simply skipped.
    let mut catalogue = Catalogue::new();
    for service in Catalogue::service_ids() {
        let _ = catalogue.get_or_load_bundled(service);
    }

    // A workflow whose definition fails to compile is skipped so completion
    // generation still succeeds for the rest.
    for id in registry().ids() {
        let Some(workflow) = registry().resolve(id) else {
            continue;
        };
        if let Ok(compiled) = compile_workflow(workflow.definition(), &mut catalogue) {
            run = run.subcommand(
                crate::invocation::workflows::build_registered_workflow_clap(&compiled)
                    .about(compiled.name.clone()),
            );
        }
    }

    Command::new("workflow")
        .help_template(subcommand_template.clone())
        .about("Run a registered multi-step workflow")
        .disable_help_subcommand(true)
        .subcommand_required(false)
        .subcommand(run)
        .subcommand(
            Command::new("list")
                .help_template(subcommand_template.clone())
                .about("List registered workflows")
                .disable_help_subcommand(true),
        )
        .subcommand(Command::new("add"))
        .subcommand(Command::new("template"))
        .subcommand(Command::new("remove"))
}

// ── Helpers ──

// Auth

/// Build the login subcommand with all auth flags.
fn build_login_subcommand() -> Command {
    let login_help_template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    Command::new("login")
        .help_template(login_help_template)
        .about("Log in to AccelByte")
        .long_about(
            "Log in to AccelByte\n\n\
             \x20 Authenticates and stores credentials locally. The --grant flag\n\
             \x20 determines the login method.\n\n\
             \x20 Authorization code (default):\n\n\
             \x20   ags auth login\n\n\
             \x20   Browser-based login using PKCE. Requires a public IAM client\n\
             \x20   with http://127.0.0.1:<port> as a redirect URI (default port\n\
             \x20   is 8080, change with --port).\n\n\
             \x20 Client credentials (CI/service accounts):\n\n\
             \x20   ags auth login --grant client-credentials\n\n\
             \x20   Authenticates with a client secret. Requires a confidential\n\
             \x20   IAM client. You will be prompted for any values not provided\n\
             \x20   via flags or environment.\n\n\
             \x20 Access token (pre-authenticated):\n\n\
             \x20   AGS_ACCESS_TOKEN=<token> ags iam users search ...\n\n\
             \x20   Bypass login entirely with a pre-obtained token. No\n\
             \x20   credentials are stored.\n\n\
             \x20 Resolution order:\n\n\
             \x20   Base URL:       --base-url → AGS_BASE_URL → config → prompt\n\
             \x20   Client ID:      --client-id → AGS_CLIENT_ID → config → prompt\n\
             \x20   Client Secret:  --client-secret → AGS_CLIENT_SECRET → keychain → prompt\n\n\
             \x20 With --format json, outputs one of:\n\
             \x20   {\"status\": \"logged_in\"}             — fresh successful grant\n\
             \x20   {\"status\": \"already_authenticated\"} — probe found a valid session\n\
             \x20   {\"status\": \"refreshed\"}             — probe refreshed an expired token",
        )
        .arg(
            Arg::new("grant")
                .long("grant")
                .help("Login method")
                .value_name("GRANT")
                .value_parser(clap::builder::PossibleValuesParser::new([
                    "authorization-code",
                    "client-credentials",
                ]))
                .default_value("authorization-code"),
        )
        .arg(
            Arg::new("base-url")
                .long("base-url")
                .help("Base URL (e.g. https://demo.accelbyte.io)")
                .value_name("URL"),
        )
        .arg(
            Arg::new("client-id")
                .long("client-id")
                .help("OAuth client ID")
                .value_name("ID"),
        )
        .arg(
            Arg::new("client-secret")
                .long("client-secret")
                .help("Client secret (insecure — visible in shell history)")
                .value_name("SECRET")
                .conflicts_with("client-secret-stdin"),
        )
        .arg(
            Arg::new("client-secret-stdin")
                .long("client-secret-stdin")
                .help("Read client secret from stdin")
                .action(clap::ArgAction::SetTrue)
                .conflicts_with("client-secret"),
        )
        .arg(
            Arg::new("port")
                .long("port")
                .help("Callback server port (default: 8080)")
                .value_name("PORT"),
        )
}

// Command building

/// Global flags shown in help. These are pre-scanned from argv before Clap
/// parses, so they're defined here for display purposes only.
fn global_flag_args() -> Vec<Arg> {
    vec![
        Arg::new("dry-run")
            .long("dry-run")
            .help("Show HTTP request without executing")
            .action(clap::ArgAction::SetTrue)
            .global(true),
        Arg::new("format")
            .long("format")
            .help("Output format for automation [json]")
            .long_help(
                "Output format for automation.\n\n\
                 Use --format json for machine-readable output in scripts and CI pipelines.\n\
                 Human-readable output (the default) may change between releases and\n\
                 should not be parsed programmatically.",
            )
            .global(true),
        Arg::new("ui")
            .long("ui")
            .help("Presentation backend for human output [auto|plain|inline|fullscreen]")
            .long_help(
                "Presentation backend for human output.\n\n\
                 Defaults to auto, which selects plain, inline, or fullscreen from the\n\
                 command and terminal. Use plain for line-oriented output, inline for an\n\
                 in-cursor surface, or fullscreen for an alternate-screen surface. --ui\n\
                 only affects presentation and cannot be combined with --format json.\n\n\
                 inline and fullscreen apply to workflow runs and interactive commands\n\
                 that gather input; read-only one-shot commands (e.g. auth status,\n\
                 config get) render plain regardless of --ui.",
            )
            .global(true),
        Arg::new("namespace")
            .long("namespace")
            .short('n')
            .help("Override namespace (default from config)")
            .global(true),
        Arg::new("no-color")
            .long("no-color")
            .help("Disable colored output")
            .action(clap::ArgAction::SetTrue)
            .global(true),
        Arg::new("no-input")
            .long("no-input")
            .help("Disable all interactive prompts")
            .action(clap::ArgAction::SetTrue)
            .global(true),
        Arg::new("output")
            .long("output")
            .help("Write response body to <path> (use '-' for stdout)")
            .long_help(
                "Write the raw response body to <path> instead of rendering it.\n\
                 Use '-' as <path> to write to stdout.\n\n\
                 Works for any response type. For binary-producing endpoints\n\
                 (image/png, application/zip, etc.) this flag, or a redirected\n\
                 stdout, is required — the CLI refuses to dump raw bytes onto\n\
                 an interactive terminal.",
            )
            .global(true),
        Arg::new("quiet")
            .long("quiet")
            .short('q')
            .help("Suppress non-essential output")
            .action(clap::ArgAction::SetTrue)
            .global(true),
        Arg::new("verbose")
            .long("verbose")
            .short('v')
            .help("Show resolution trace and request/response details")
            .action(clap::ArgAction::SetTrue)
            .global(true),
        Arg::new("yes")
            .long("yes")
            .short('y')
            .help("Skip confirmation prompts")
            .action(clap::ArgAction::SetTrue)
            .global(true),
        Arg::new("skeleton")
            .long("skeleton")
            .help("Output a JSON request body template (for operations with --json)")
            .action(clap::ArgAction::SetTrue)
            .global(true),
        Arg::new("timeout")
            .long("timeout")
            .help("Request timeout in seconds (default 60)")
            .global(true),
        Arg::new("page-all")
            .long("page-all")
            .help("Fetch all pages of paginated results")
            .action(clap::ArgAction::SetTrue)
            .global(true),
        Arg::new("page-limit")
            .long("page-limit")
            .help("Max pages to fetch with --page-all (default 10, max 100)")
            .global(true),
    ]
}

// ── Value parsers for image-upload flags ──

/// Parse an f64 value that must be finite and non-negative.
///
/// Used as the `value_parser` for `--retry-interval` and `--retry-rate`.
/// Rejects negative, NaN, and infinite values at the CLI edge so they
/// never reach `Duration::from_secs_f64` (which panics on such inputs).
fn parse_finite_nonneg_f64(s: &str) -> Result<f64, String> {
    let val: f64 = s
        .parse()
        .map_err(|e: std::num::ParseFloatError| e.to_string())?;
    if !val.is_finite() {
        return Err("must be a finite number".to_string());
    }
    if val < 0.0 {
        return Err("must be non-negative".to_string());
    }
    Ok(val)
}

/// Parse and validate a Docker image tag for the clap `value_parser`.
///
/// Delegates to [`crate::invocation::handlers::extend::image_upload::check_docker_tag`]
/// so the charset rule has a single implementation.
fn parse_docker_tag(s: &str) -> Result<String, String> {
    crate::invocation::handlers::extend::image_upload::check_docker_tag(s)?;
    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_workflow_command_includes_add_and_template() {
        let command = build_workflow_command();
        assert!(command.find_subcommand("add").is_some());
        assert!(command.find_subcommand("template").is_some());
    }

    #[test]
    fn test_build_workflow_command_includes_remove() {
        let command = build_workflow_command();
        assert!(command.find_subcommand("remove").is_some());
    }

    #[test]
    fn test_build_workflow_completion_command_includes_remove() {
        let command = build_workflow_completion_command();
        assert!(command.find_subcommand("remove").is_some());
    }

    // ── Extend subcommand flag-collision prohibition ──

    /// No `extend` subcommand may register a long or short flag that shadows
    /// a global flag consumed by `pre_scan_global_flags`. The global set is
    /// derived from `KNOWN_GLOBAL_FLAGS` — the same constant the prescan
    /// reads — so adding a new global flag automatically fails this test if
    /// any existing subcommand already uses that name.
    #[test]
    fn test_extend_subcommands_do_not_shadow_global_flags() {
        use std::collections::HashSet;

        // Derive the global long-flag and short-flag sets from the single
        // source of truth that `pre_scan_global_flags` consumes.
        let global_longs: HashSet<&str> = crate::invocation::flags::KNOWN_GLOBAL_FLAGS
            .iter()
            .filter(|(f, _)| f.starts_with("--"))
            .map(|(f, _)| *f)
            .collect();
        let global_shorts: HashSet<char> = crate::invocation::flags::KNOWN_GLOBAL_FLAGS
            .iter()
            .filter(|(f, _)| f.starts_with('-') && !f.starts_with("--"))
            .filter_map(|(f, _)| f.chars().nth(1))
            .collect();

        let cmd = build_extend_command();
        let mut collisions: Vec<String> = Vec::new();

        // Recursive walk: check every subcommand at every depth.
        fn walk(
            cmd: &Command,
            path: &str,
            global_longs: &HashSet<&str>,
            global_shorts: &HashSet<char>,
            collisions: &mut Vec<String>,
        ) {
            for arg in cmd.get_arguments() {
                if let Some(long) = arg.get_long() {
                    let flag_str = format!("--{long}");
                    if global_longs.contains(flag_str.as_str()) {
                        collisions.push(format!("{path}: --{long} shadows global flag"));
                    }
                }
                if let Some(short) = arg.get_short() {
                    if global_shorts.contains(&short) {
                        collisions.push(format!("{path}: -{short} shadows global flag"));
                    }
                }
            }
            for sub in cmd.get_subcommands() {
                let child_path = format!("{path} {}", sub.get_name());
                walk(sub, &child_path, global_longs, global_shorts, collisions);
            }
        }

        for sub in cmd.get_subcommands() {
            let path = format!("extend {}", sub.get_name());
            walk(sub, &path, &global_longs, &global_shorts, &mut collisions);
        }

        assert!(
            collisions.is_empty(),
            "extend subcommand flags shadow global flags:\n{}",
            collisions.join("\n")
        );
    }

    /// Every flag in the `extend` command tree that is hidden or whose help
    /// text mentions "backward compatibility" must be declared through
    /// [`CompatFlag`] and registered in [`all_compat_flag_ids`]. A future
    /// route that adds a silent no-op flag via raw `Arg::new(...).hide(true)`
    /// bypassing CompatFlag will fail this test.
    ///
    /// Structural counterpart of `test_extend_subcommands_do_not_shadow_global_flags`.
    #[test]
    fn test_extend_hidden_compat_flags_registered_through_compat_flag() {
        use std::collections::HashSet;

        let known_compat_ids: HashSet<&str> =
            crate::invocation::compat_flags::all_compat_flag_ids()
                .iter()
                .copied()
                .collect();

        let cmd = build_extend_command();
        let mut unregistered: Vec<String> = Vec::new();

        fn walk(cmd: &Command, path: &str, known: &HashSet<&str>, unregistered: &mut Vec<String>) {
            for arg in cmd.get_arguments() {
                let help_text = arg.get_help().map(|h| h.to_string()).unwrap_or_default();
                let is_compat_like =
                    arg.is_hide_set() || help_text.contains("backward compatibility");
                if is_compat_like {
                    let id = arg.get_id().as_str();
                    if !known.contains(id) {
                        let long = arg.get_long().unwrap_or(id);
                        unregistered.push(format!(
                            "{path}: --{long} is hidden/compat-marked but not in CompatFlag registry"
                        ));
                    }
                }
            }
            for sub in cmd.get_subcommands() {
                let child_path = format!("{path} {}", sub.get_name());
                walk(sub, &child_path, known, unregistered);
            }
        }

        for sub in cmd.get_subcommands() {
            let path = format!("extend {}", sub.get_name());
            walk(sub, &path, &known_compat_ids, &mut unregistered);
        }

        assert!(
            unregistered.is_empty(),
            "Hidden/compat-marked flags not registered through CompatFlag:\n{}",
            unregistered.join("\n")
        );
    }
}
