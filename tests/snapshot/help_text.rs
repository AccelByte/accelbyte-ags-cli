use crate::common::cli_helpers::ags;

// ── Root help ──

/// Root --help lists all 24 services, standalone commands, flags, examples, and exit codes
#[test]
fn test_root_help() {
    let output = ags().arg("--help").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout, @"
    AccelByte Gaming Services CLI

    Manage your AccelByte backend from the terminal. Authenticate,
    call any admin API, and automate cross-service workflows.

    Usage:
      ags [FLAGS] <COMMAND> [OPTIONS]
      ags [FLAGS] <SERVICE> <RESOURCE> <METHOD> [OPTIONS]

    Commands (standalone):
      auth           Authentication management
      completions    Generate a shell completion script
      config         Configuration management
      profile        Profile management
      describe       Machine-readable command discovery and introspection (JSON)
      doctor         Check environment, configuration, and connectivity
      refresh-specs  Rebuild the parsed-schema cache from bundled specs

    Services (API groups):
      achievement        Player achievements, badges, and progression tracking
      ams                Multiplayer server fleet management and orchestration
      basic              User profiles, namespaces, and file uploads
      challenge          Challenge definitions, goals, schedules, and player progression
      chat               Chat messaging, moderation, inbox, and profanity filtering
      cloud-save         Cloud save records for games and players (binary and JSON)
      csm                Custom service management, deployments, and container images
      game-telemetry     Game telemetry event ingestion and querying
      gdpr               GDPR data deletion, retrieval, and platform account closure
      group              Player groups, memberships, roles, and join requests
      iam                Identity, authentication, authorization, and user management
      inventory          Player inventories, items, item types, and tags
      leaderboard        Leaderboard configuration, data, and user rankings
      legal              Legal agreements, policies, eligibility, and consent tracking
      lobby              Lobby connections, friends, presence, party, and notifications
      login-queue        Login queue management and ticket processing
      matchmaking        Matchmaking pools, rulesets, tickets, and backfill
      platform           Store, items, entitlements, wallets, payments, and IAP
      reporting          Player reporting, moderation rules, reasons, and tickets
      season-pass        Season passes, tiers, rewards, and seasonal content
      session            Game sessions, parties, matchmaking templates, and DS config
      session-history    Session analytics and X-Ray diagnostics
      social             Stats, stat cycles, game profiles, and social slots
      ugc                User-generated content: uploads, moderation, and discovery

    Flags:
          --dry-run                  Show HTTP request without executing
          --format <format>          Output format for automation [json]
      -n, --namespace <namespace>    Override namespace (default from config)
          --no-color                 Disable colored output
          --no-input                 Disable all interactive prompts
          --output <output>          Write response body to <path> (use '-' for stdout)
      -q, --quiet                    Suppress non-essential output
      -v, --verbose                  Show HTTP request/response details
      -y, --yes                      Skip confirmation prompts
          --skeleton                 Output a JSON request body template (for operations with --json)
          --timeout <timeout>        Request timeout in seconds (default 60)
          --page-all                 Fetch all pages of paginated results
          --page-limit <page-limit>  Max pages to fetch with --page-all (default 10, max 100)
      -h, --help                     Print help (see more with '--help')
      -V, --version                  Print version

    Examples:
      ags auth login
      ags iam users search --namespace my-game
      ags platform items create --namespace my-game --store-id main --json @item.json

    Exit codes:
      0 = success
      1 = usage error
      2 = auth error
      3 = API error
      4 = network error
      5 = internal error

    Links:
      Docs:      https://docs.accelbyte.io/
      Feedback:  https://github.com/AccelByte/accelbyte-ags-cli/issues
    ");
}

// ── refresh-specs help ──

/// `refresh-specs --help` prints subcommand-level help describing the optional service argument
#[test]
fn test_refresh_specs_help() {
    let output = ags().args(["refresh-specs", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout, @r"
    Rebuild the parsed-schema cache from bundled specs.

    Without an argument, clears the cache directory and rebuilds
    every service. With a service argument, rebuilds just that
    service's cache.

    Use this after updating the bundled specs or when the CLI
    reports a stale or corrupt cache.

    Usage:
      refresh-specs [service]

    Arguments:
      [service]
              Optional service name; omit to refresh all services

    Options:
      -h, --help
              Print help (see a summary with '-h')
    ");
}

// ── Service-level help (IAM) ──

/// Service-level help lists all resources within that service with one-line descriptions
#[test]
fn test_iam_service_help() {
    let output = ags().args(["iam", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout, @"
    Identity, authentication, authorization, and user management

    Usage:
      ags iam <RESOURCE> <METHOD> [OPTIONS]

    Resources:
      age-restrictions           Age restriction settings per country
      allow-list                 Login allowlist for restricted access
      bans                       Ban management and ban type configuration
      client-config              OAuth client configuration templates
      clients                    OAuth client application management
      config                     IAM service configuration
      countries                  Country and region settings
      devices                    Device bans and device management
      input-validations          Input validation rules for user fields
      oauth2                     OAuth 2.0 token and authorization management
      platform-credentials       Login provider credentials, including Device ID
      profile-update-strategies  Profile field update strategy settings
      role-override              Role override configuration per namespace
      roles                      Role definitions, permissions, and assignment
      sso                        Single sign-on authentication flows
      sso-credentials            SSO credential and platform login configuration
      tags                       User account tag management
      user-mfa                   User multi-factor authentication configuration
      users                      User accounts, profiles, bans, and permissions

    Options:
      -h, --help  Print help
    ");
}

// ── Resource-level help (IAM users) ──

/// Resource-level help lists all available methods with their summaries
#[test]
fn test_iam_users_resource_help() {
    let output = ags().args(["iam", "users", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout, @"
    User accounts, profiles, bans, and permissions

    Usage:
      ags iam users <METHOD> [OPTIONS]

    Methods:
      add-permission                          Adds permissions to the user
      assign-role                             Adds roles to a user's current role assignments.
      ban                                     Bans a user
      bulk-check-valid                        Checks whether user IDs are valid
      bulk-get                                Retrieves users by user ID
      bulk-get-bans                           Retrieves bans for a list of users
      bulk-get-by-email                       Retrieves users by email address in bulk
      bulk-get-platforms                      Retrieves platform information for a list of users
      bulk-remove-permissions                 Deletes user permissions
      bulk-update                             Updates users in bulk
      bulk-update-account-type                Updates user account types in bulk.
      check-availability                      Checks whether a user account field value is available
      create                                  Creates a new user account
      create-from-invitation                  Creates a user account from an invitation
      create-from-publisher                   Creates a Justice user from a publisher user
      create-test                             Creates test users without sending verification emails
      delete                                  Deletes a user's information
      delete-platform-linking-restriction     Removes a user's platform linking restriction
      force-link-platform                     Links the current user's account to a platform, overriding any existing link.
      force-link-platform-with-progression    Links a platform account and transfers game progression, overriding any exist...
      force-verify                            Verifies a user account
      get                                     Retrieves a user by user ID
      get-ban-summary                         Retrieves a summary of ban records for a user
      get-by-email                            Retrieves a user by email address
      get-by-platform-user-id                 Retrieves a user by platform user ID
      get-deletion-status                     Retrieves deletion status for a user
      get-information                         Retrieves a user's profile and linked platform accounts
      get-invitation                          Retrieves a user invitation
      get-invitation-history                  Retrieves invitation history for a specific namespace
      get-linking-status                      Retrieves the async account linking progress status
      get-login-history                       Retrieves login history for a user
      get-mapping                             Retrieves the namespace user ID mapping for a user
      get-mfa-status                          Retrieves a user's 2FA status
      get-my                                  Retrieves the current user account
      get-my-link-redirection                 Retrieves the redirect URI after account linking
      get-my-linking-conflict                 Retrieves linking conflicts when linking a headless account to a full account
      get-my-profile-update-policy            Retrieves profile update status for the current user
      get-openid-info                         Retrieves the current user's OpenID Connect user info
      get-platform-account-metadata           Retrieves platform account metadata for a user
      get-platform-link-status                Checks the link status of a third-party platform token
      get-platform-linking-history            Lists platform link history for a user
      get-platform-web-link                   Generates a third-party platform login URL for web account linking
      get-publisher                           Retrieves the publisher account for a user
      get-state                               Retrieves the state of a user account
      get-user-invitation-history             Retrieves user invitation history for a specific namespace
      handle-web-auth-callback                Creates the third-party platform account link after the authorization redirect.
      invite                                  Invites a user and assigns a role
      invite-admin                            Invites a game studio admin user to a new namespace
      link-my-headless-account                Links a headless account to the current full account
      link-platform-account                   Links a user's account to a platform
      link-platform-account-forced            Links a platform account to a user
      list-accelbyte-accounts                 Lists Justice platform accounts for a user
      list-admins                             Lists admin users in a namespace
      list-bans                               Retrieves ban records for a user
      list-by-cursor                          Retrieves users using cursor-based pagination
      list-by-platform-id                     Lists user IDs by platform user ID
      list-distinct-active-platform-accounts  Retrieves distinct platform accounts linked to a user
      list-distinct-platform-accounts         Lists distinct third-party platforms linked to a user
      list-invitation-histories               Lists invitation histories for studio namespaces
      list-linked-platforms                   Retrieves users' basic info and linked third-party platform info
      list-platform-accounts                  Lists platform accounts linked to a user
      list-roles                              Lists roles assigned to a user
      list-users-with-accelbyte-account       Lists users with Justice platform accounts
      list-verification-codes                 Retrieves active verification codes for a user
      remove-permission                       Deletes a permission from a user
      request-password-reset                  Requests a password reset code
      request-password-reset-global           Requests a password reset code
      reset-password                          Resets a user's password
      resolve-web-auth-redirect               Processes the third-party web link callback and returns link status
      revoke-role                             Removes roles from a user
      revoke-roles                            Removes roles from a user
      revoke-trusted-device                   Removes the trusted device for the current user
      search                                  Searches users in the namespace
      search-platform-linking-history         Searches link history for a platform user ID
      send-my-verification-code               Sends a verification code to the current user's email address
      send-verification-code                  Sends a verification code to a user
      send-verification-code-forward          Sends a verification code using an upgrade token
      send-verification-link                  Sends a verification link to the current user's email address
      set-my                                  Updates the current user's profile
      set-permissions                         Replaces a user's permissions
      set-roles                               Replaces all roles assigned to a user
      start-upgrade-my                        Upgrades a headless account to a full account
      unlink-platform-all                     Unlinks a user's account from a platform across all namespaces
      update                                  Updates a user's profile
      update-ban                              Enables or disables a ban for a user
      update-deletion-status                  Updates deletion status for a user
      update-email                            Updates a user's email address
      update-my                               Updates the current admin user's profile
      update-my-email                         Updates the current user's email address
      update-my-password                      Updates the current user's password
      update-password                         Updates a user's password
      update-status                           Enables or disables a user account
      update-trusted-identity                 Updates a user's identity without email notification
      upgrade-and-validate                    Verifies or consumes a verification code
      upgrade-and-validate-forward            Upgrades a headless account and verifies the email address with redirect
      upgrade-and-validate-my                 Verifies or consumes a code to upgrade a headless account
      validate-code                           Verifies or consumes a verification code for a user
      validate-input                          Validates user input against configured validation rules
      validate-password                       Validates a user's password
      validate-registration-code              Verifies the registration code
      verify-email                            Verifies a user's email using a verification link code

    Options:
      -h, --help  Print help
    ");
}

// ── Method-level help (IAM users list-users-with-accelbyte-account) ──

/// Method-level help shows the full description, required permissions, options, and an example command
#[test]
fn test_iam_users_list_method_help() {
    let output = ags()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--help",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout, @"
    Requires publisher namespace.

    Returns a list of user IDs and namespaces with their Justice platform accounts.
    If a user does not have a Justice platform account, the linkedPlatforms field is an empty array.

    Requires permission:
      READ on ADMIN:NAMESPACE:{namespace}:USER

    Default contract:
      scope:   admin
      version: v3

    Usage:
      ags iam users list-users-with-accelbyte-account [OPTIONS] --namespace <namespace>

    Options:
          --namespace <namespace>
              Game namespace

          --limit <limit>
              Number of results to return

          --offset <offset>
              Number of results to skip for pagination

      -h, --help
              Print help (see a summary with '-h')

    Example:
      ags iam users list-users-with-accelbyte-account --namespace 'my-namespace' --limit 68 --offset 12
    ");
}

// ── Auth help ──

/// Auth subcommand help lists login, logout, and status commands
#[test]
fn test_auth_help() {
    let output = ags().args(["auth", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout, @"
    Authentication management

    Usage:
      auth <COMMAND>

    Commands:
      login   Log in to AccelByte
      logout  Log out and clear credentials
      status  Show current authentication status

    Options:
      -h, --help  Print help
    ");
}

// ── Config help ──

/// Config subcommand help lists get, set, and unset commands
#[test]
fn test_config_help() {
    let output = ags().args(["config", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout);
}

// ── Profile help ──

/// Profile subcommand help lists all profile management commands
#[test]
fn test_profile_help() {
    let output = ags().args(["profile", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout);
}

/// Profile create subcommand help shows the name argument
#[test]
fn test_profile_create_help() {
    let output = ags()
        .args(["profile", "create", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout);
}

// ── Doctor help ──

/// Doctor --help shows description, usage, and flags
#[test]
fn test_doctor_help() {
    let output = ags().args(["doctor", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout);
}

// ── Leaf-level help: --api-scope / --api-version injection ──

/// Case A: multi-scope multi-version leaf — default help shows Default contract (admin/v4),
/// both --api-scope and --api-version flags with their possible values.
/// Uses `iam roles list` (admin v3/v4, public v3).
#[test]
fn test_leaf_help_multi_scope_multi_version_default() {
    let output = ags()
        .args(["iam", "roles", "list", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout, @"
    Lists admin roles available in the system.

    Requires permission:
      READ on ADMIN:ROLE

    Default contract:
      scope:   admin
      version: v4

    Usage:
      ags iam roles list [OPTIONS]

    Options:
          --admin-role <admin-role>
              - true if the expected result should only returns records with adminRole = true
              - false if the expected result should only returns records with adminRole = false
              - empty (omitted) if the expected result should returns records with no wildcard filter at all

          --is-wildcard <is-wildcard>
              - true if the expected result should only returns records with wildcard = true
              - false if the expected result should only returns records with wildcard = false
              - empty (omitted) if the expected result should returns records with no wildcard filter at all

          --limit <limit>
              Number of results to return

          --offset <offset>
              Number of results to skip for pagination

          --api-scope <api-scope>
              Select the CLI API scope for this command
              
              [default: admin]
              [possible values: admin, public]

          --api-version <api-version>
              Select the CLI API version for this command
              
              [default: v4]
              [possible values: v3, v4]

      -h, --help
              Print help (see a summary with '-h')

    Example:
      ags iam roles list --admin-role true --is-wildcard false --limit 68 --offset 12
    ");
}

/// Case B: multi-scope leaf with --api-scope public — Default contract shows public/v3,
/// --api-version flag absent (public only has v3 for this command).
/// Uses `iam roles list --api-scope public`.
#[test]
fn test_leaf_help_multi_scope_explicit_public() {
    let output = ags()
        .args(["iam", "roles", "list", "--api-scope", "public", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout, @"
    Returns all non-admin roles.

    Default contract:
      scope:   public
      version: v3

    Usage:
      ags iam roles list [OPTIONS]

    Options:
          --after <after>
              Cursor for forward pagination

          --before <before>
              Cursor for backward pagination

          --is-wildcard <is-wildcard>
              - true if the expected result should only returns records with wildcard = true
              - false if the expected result should only returns records with wildcard = false
              - empty (omitted) if the expected result should returns records with no wildcard filter at all

          --limit <limit>
              Number of results to return

          --api-scope <api-scope>
              Select the CLI API scope for this command
              
              [default: admin]
              [possible values: admin, public]

      -h, --help
              Print help (see a summary with '-h')

    Example:
      ags iam roles list --after 'my-after' --before 'my-before' --is-wildcard false --limit 68
    ");
}

/// Case C: multi-scope leaf with public scope pinned to a non-default version — Default contract
/// shows public/v3, --api-version default locks to v3, both flags shown.
/// Uses `iam users create --api-scope public --api-version v3`.
#[test]
fn test_leaf_help_multi_scope_fully_pinned() {
    let output = ags()
        .args([
            "iam",
            "users",
            "create",
            "--api-scope",
            "public",
            "--api-version",
            "v3",
            "--help",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout, @r#"
    Available Authentication Types: 1. EMAILPASSWD: an authentication type used for new user registration through email.
    
    Note: * uniqueDisplayName: required when uniqueDisplayNameEnabled/UNIQUE_DISPLAY_NAME_ENABLED is true. * code: required when mandatoryEmailVerificationEnabled config is true.
    Refer to the configuration from /iam/v3/public/namespaces/{namespace}/config/{configKey}.
    
    Country uses ISO3166-1 alpha-2 two letter, e.g. US. Date of Birth format: YYYY-MM-DD, e.g. 2019-04-29.
    Supports accepting agreements for the created user; supply the accepted agreements in acceptedPolicies.
    
    Default contract:
      scope:   public
      version: v3
    
    Usage:
      ags iam users create [OPTIONS] --namespace <namespace> --json <json>
    
    Options:
          --namespace <namespace>
              Game namespace
    
          --json <json>
              JSON request body (UserCreateRequestV3)
              
              Input:
                --json @path/to.json   read JSON from a file
                --json @-              read JSON from stdin (avoids shell quoting issues)
                --json '{...}'         inline JSON
              
              Schema:
                {
                  *"authType": <string>,
                  *"code": <string>,
                  *"country": <string>,
                  *"displayName": <string>,
                  *"emailAddress": <string>,
                  *"password": <string>,
                  *"reachMinimumAge": <boolean>,
                   "PasswordMD5Sum": <string>,
                   "acceptedPolicies": [  <array[AcceptedPoliciesRequest]>
                    {
                      *"isAccepted": <boolean>,
                      *"localizedPolicyVersionId": <string>,
                      *"policyId": <string>,
                      *"policyVersionId": <string>
                    }
                  ],
                   "dateOfBirth": <string>,
                   "uniqueDisplayName": <string>
                }
              
              Example:
                  {
                    "PasswordMD5Sum": "my-password-md5-sum",
                    "acceptedPolicies": [
                      {
                        "isAccepted": true,
                        "localizedPolicyVersionId": "my-localized-policy-version-id",
                        "policyId": "my-policy-id",
                        "policyVersionId": "my-policy-version-id"
                      }
                    ],
                    "authType": "my-auth-type",
                    "code": "my-code",
                    "country": "my-country",
                    "dateOfBirth": "my-date-of-birth",
                    "displayName": "my-display-name",
                    "emailAddress": "my-email-address",
                    "password": "my-password",
                    "reachMinimumAge": false,
                    "uniqueDisplayName": "my-unique-display-name"
                  }
    
          --api-scope <api-scope>
              Select the CLI API scope for this command
              
              [default: admin]
              [possible values: admin, public]
    
          --api-version <api-version>
              Select the CLI API version for this command
              
              [default: v4]
              [possible values: v3, v4]
    
      -h, --help
              Print help (see a summary with '-h')
    
    Example:
      ags iam users create --namespace 'my-namespace' --json '{...}'
    "#);
}

/// Case D: single-scope single-version leaf — Default contract header present, no --api-scope
/// or --api-version flags injected. Uses `iam bans bulk-ban-users` (admin/v3 only).
#[test]
fn test_leaf_help_single_scope_single_version() {
    let output = ags()
        .args(["iam", "bans", "bulk-ban-users", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout, @r#"
    Bans up to 100 users with a specified ban type. Retrieve available ban types and reasons from the bans list.
    
    Requires permission:
      CREATE on ADMIN:NAMESPACE:{namespace}:BAN
    
    Default contract:
      scope:   admin
      version: v3
    
    Usage:
      ags iam bans bulk-ban-users --namespace <namespace> --json <json>
    
    Options:
          --namespace <namespace>
              Game namespace
    
          --json <json>
              JSON request body (BulkBanCreateRequestV3)
              
              Input:
                --json @path/to.json   read JSON from a file
                --json @-              read JSON from stdin (avoids shell quoting issues)
                --json '{...}'         inline JSON
              
              Schema:
                {
                  *"ban": <string>,
                  *"comment": <string>,
                  *"endDate": <string>,
                  *"reason": <string>,
                  *"skipNotif": <boolean>,
                  *"userIds": <array[string]>
                }
              
              Example:
                  {
                    "ban": "my-ban",
                    "comment": "my-comment",
                    "endDate": "my-end-date",
                    "reason": "my-reason",
                    "skipNotif": true,
                    "userIds": [
                      "my-user-ids"
                    ]
                  }
    
      -h, --help
              Print help (see a summary with '-h')
    
    Example:
      ags iam bans bulk-ban-users --namespace 'my-namespace' --json '{...}'
    "#);
}

/// Case E: admin-only multi-version leaf (iam roles add-permissions) — Default contract header
/// present (multi-version), no --api-scope flag (single scope), --api-version flag present
#[test]
fn test_leaf_help_admin_only_multi_version() {
    let output = ags()
        .args(["iam", "roles", "add-permissions", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout, @r#"
    Attaches the specified permissions to a role.

    Permissions can include a schedule as a cron string or UTC date range.
    Both schedule types accept quartz-compatible cron syntax.
    In a ranged schedule, the first element is the start date and the second is the end date.
    When a schedule is defined, the action value must be between 1 and 15, inclusive.

    Requires permission:
      UPDATE on ADMIN:ROLE

    Default contract:
      scope:   admin
      version: v4

    Usage:
      ags iam roles add-permissions [OPTIONS] --role-id <role-id> --json <json>

    Options:
          --role-id <role-id>
              IAM role ID

          --json <json>
              JSON request body (PermissionsV3)
              
              Input:
                --json @path/to.json   read JSON from a file
                --json @-              read JSON from stdin (avoids shell quoting issues)
                --json '{...}'         inline JSON
              
              Schema:
                {
                  *"permissions": [  <array[PermissionV3]>
                    {
                      *"action": <integer>,
                      *"resource": <string>,
                       "schedAction": <integer>,
                       "schedCron": <string>,
                       "schedRange": <array[string]>
                    }
                  ]
                }
              
              Example:
                  {
                    "permissions": [
                      {
                        "action": 99,
                        "resource": "my-resource",
                        "schedAction": 10,
                        "schedCron": "my-sched-cron",
                        "schedRange": [
                          "my-sched-range"
                        ]
                      }
                    ]
                  }

          --api-version <api-version>
              Select the CLI API version for this command
              
              [default: v4]
              [possible values: v3, v4]

      -h, --help
              Print help (see a summary with '-h')

    Example:
      ags iam roles add-permissions --role-id 'my-role-id' --json '{...}'
    "#);
}
