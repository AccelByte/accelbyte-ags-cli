//! Build dynamic Clap command trees from `ServiceSchema`.

use clap::{Arg, Command};

use crate::catalogue::Catalogue;
use crate::frontend::style;
use crate::invocation::flags::LeafSelectors;
use crate::protocol::catalogue::ServiceSchema;

// ── Public entry points ──

/// Build the root command structure (name, help, about, args, and auxiliary
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
         ags iam users list --namespace my-game\n  \
         ags platform item create --namespace my-game --json @item.json\n\
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

    for internal in Catalogue::service_ids() {
        let display = Catalogue::display_name_or_panic(internal);
        let desc = Catalogue::service_description(internal);
        let service_command = match Catalogue::load_bundled(internal) {
            Ok(schema) => {
                crate::invocation::commands::service::clap_tree::build_service_command_tree(
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
    crate::invocation::commands::service::clap_tree::build_service_command_tree(schema, selectors)
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
                .help_template(auth_help_template)
                .about("Show current authentication status")
                .long_about(
                    "Show current authentication status\n\n\
                     \x20 Displays the active authentication source (environment variables\n\
                     \x20 or stored credentials), base URL, client ID, token expiry time,\n\
                     \x20 and whether a refresh token is available.\n\n\
                     \x20 With --format json, outputs full auth state as machine-readable JSON.",
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
                .long_about(
                    "Set a configuration value\n\n\
                     \x20 Profile keys: base-url, client-id, namespace, grant-type\n\
                     \x20 Global keys:  active-profile, format, no-color, timeout, page-limit\n\n\
                     \x20 Scope is auto-detected from the key. Use --global or --profile to override.",
                )
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
                .long_about(
                    "Remove a configuration value\n\n\
                     \x20 Profile keys: base-url, client-id, namespace, grant-type\n\
                     \x20 Global keys:  active-profile, format, no-color, timeout, page-limit\n\n\
                     \x20 Scope is auto-detected from the key. Use --global or --profile to override.",
                )
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
             \x20   ags describe iam users list        outputs full parameter schema for a method\n\n\
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
             \x20   AGS_ACCESS_TOKEN=<token> ags iam users list ...\n\n\
             \x20   Bypass login entirely with a pre-obtained token. No\n\
             \x20   credentials are stored.\n\n\
             \x20 Resolution order:\n\n\
             \x20   Base URL:       --base-url → AGS_BASE_URL → config → prompt\n\
             \x20   Client ID:      --client-id → AGS_CLIENT_ID → config → prompt\n\
             \x20   Client Secret:  --client-secret → AGS_CLIENT_SECRET → keychain → prompt\n\n\
             \x20 With --format json, outputs {\"status\": \"authenticated\"} on success.",
        )
        .arg(
            Arg::new("grant")
                .long("grant")
                .help("Login method")
                .value_name("GRANT")
                .value_parser(clap::value_parser!(crate::protocol::request::GrantType))
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
            .help("Show HTTP request/response details")
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
