//! Hand-authored catalogue metadata: service ids, display names, and
//! descriptions for services and resources.

/// A validated service id — guaranteed to refer to an entry in [`SERVICES`].
/// Construct only via [`find_id`] or [`Catalogue::find_id`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ServiceId(&'static str);

impl ServiceId {
    /// Return the canonical internal id as a `&'static str`.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for ServiceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl serde::Serialize for ServiceId {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.0)
    }
}

impl<'de> serde::Deserialize<'de> for ServiceId {
    fn deserialize<D: serde::Deserializer<'de>>(deser: D) -> Result<Self, D::Error> {
        let s = <&str>::deserialize(deser)?;
        find_id(s).ok_or_else(|| serde::de::Error::custom(format!("unknown service id '{s}'")))
    }
}

/// Hand-authored metadata for one CLI service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ServiceManifest {
    pub internal: &'static str,
    pub display: &'static str,
    pub description: &'static str,
}

/// All supported services, in CLI display order.
pub(crate) const SERVICES: &[ServiceManifest] = &[
    ServiceManifest {
        internal: "achievement",
        display: "achievement",
        description: "Player achievements, badges, and progression tracking",
    },
    ServiceManifest {
        internal: "ams",
        display: "ams",
        description: "Multiplayer server fleet management and orchestration",
    },
    ServiceManifest {
        internal: "basic",
        display: "basic",
        description: "User profiles, namespaces, and file uploads",
    },
    ServiceManifest {
        internal: "challenge",
        display: "challenge",
        description: "Challenge definitions, goals, schedules, and player progression",
    },
    ServiceManifest {
        internal: "chat",
        display: "chat",
        description: "Chat messaging, moderation, inbox, and profanity filtering",
    },
    ServiceManifest {
        internal: "cloudsave",
        display: "cloud-save",
        description: "Cloud save records for games and players (binary and JSON)",
    },
    ServiceManifest {
        internal: "csm",
        display: "csm",
        description: "Custom service management, deployments, and container images",
    },
    ServiceManifest {
        internal: "gametelemetry",
        display: "game-telemetry",
        description: "Game telemetry event ingestion and querying",
    },
    ServiceManifest {
        internal: "gdpr",
        display: "gdpr",
        description: "GDPR data deletion, retrieval, and platform account closure",
    },
    ServiceManifest {
        internal: "group",
        display: "group",
        description: "Player groups, memberships, roles, and join requests",
    },
    ServiceManifest {
        internal: "iam",
        display: "iam",
        description: "Identity, authentication, authorization, and user management",
    },
    ServiceManifest {
        internal: "inventory",
        display: "inventory",
        description: "Player inventories, items, item types, and tags",
    },
    ServiceManifest {
        internal: "leaderboard",
        display: "leaderboard",
        description: "Leaderboard configuration, data, and user rankings",
    },
    ServiceManifest {
        internal: "legal",
        display: "legal",
        description: "Legal agreements, policies, eligibility, and consent tracking",
    },
    ServiceManifest {
        internal: "lobby",
        display: "lobby",
        description: "Lobby connections, friends, presence, party, and notifications",
    },
    ServiceManifest {
        internal: "loginqueue",
        display: "login-queue",
        description: "Login queue management and ticket processing",
    },
    ServiceManifest {
        internal: "match2",
        display: "matchmaking",
        description: "Matchmaking pools, rulesets, tickets, and backfill",
    },
    ServiceManifest {
        internal: "platform",
        display: "platform",
        description: "Store, items, entitlements, wallets, payments, and IAP",
    },
    ServiceManifest {
        internal: "reporting",
        display: "reporting",
        description: "Player reporting, moderation rules, reasons, and tickets",
    },
    ServiceManifest {
        internal: "seasonpass",
        display: "season-pass",
        description: "Season passes, tiers, rewards, and seasonal content",
    },
    ServiceManifest {
        internal: "session",
        display: "session",
        description: "Game sessions, parties, matchmaking templates, and DS config",
    },
    ServiceManifest {
        internal: "sessionhistory",
        display: "session-history",
        description: "Session analytics and X-Ray diagnostics",
    },
    ServiceManifest {
        internal: "social",
        display: "social",
        description: "Stats, stat cycles, game profiles, and social slots",
    },
    ServiceManifest {
        internal: "ugc",
        display: "ugc",
        description: "User-generated content: uploads, moderation, and discovery",
    },
];

/// Look up the manifest entry for a service by internal id.
pub fn find_service(internal: &str) -> Option<&'static ServiceManifest> {
    SERVICES.iter().find(|entry| entry.internal == internal)
}

/// Look up a service by display or internal name; return its `ServiceId`.
pub fn find_id(name: &str) -> Option<ServiceId> {
    SERVICES
        .iter()
        .find(|entry| entry.display == name || entry.internal == name)
        .map(|m| ServiceId(m.internal))
}

/// Iterate the known internal service ids in manifest order.
pub fn service_ids() -> impl Iterator<Item = &'static str> {
    SERVICES.iter().map(|entry| entry.internal)
}

/// Resolve an internal service name to the CLI display name.
pub fn display_name(internal: &str) -> Option<&'static str> {
    find_service(internal).map(|entry| entry.display)
}

/// Resolve a CLI display name or internal id to the internal service name.
pub fn internal_name(name: &str) -> Option<&'static str> {
    SERVICES
        .iter()
        .find(|entry| entry.display == name || entry.internal == name)
        .map(|entry| entry.internal)
}

/// Return the one-line description for a service, or an empty string if unknown.
pub fn service_description(internal: &str) -> &'static str {
    find_service(internal)
        .map(|entry| entry.description)
        .unwrap_or("")
}

/// Hand-authored one-line descriptions for each (service, resource) pair.
/// Used in `ags <service> --help` to describe each resource.
pub static RESOURCE_DESCRIPTIONS: &[(&str, &str, &str)] = &[
    // achievement
    (
        "achievement",
        "achievements",
        "Manage achievement definitions and configuration",
    ),
    (
        "achievement",
        "collaborative-progress",
        "Collaborative achievement progress tracking",
    ),
    (
        "achievement",
        "platform-achievements",
        "Platform-specific achievement events (e.g. PSN)",
    ),
    (
        "achievement",
        "tags",
        "Achievement tag management for categorization",
    ),
    (
        "achievement",
        "user-progress",
        "Per-user achievement progress and unlocks",
    ),
    // ams
    ("ams", "account", "AMS account settings and configuration"),
    ("ams", "artifacts", "Server build artifacts and logs"),
    (
        "ams",
        "dedicated-servers",
        "Dedicated server instances and lifecycle",
    ),
    (
        "ams",
        "dev-server-config",
        "Development server configuration",
    ),
    (
        "ams",
        "fleet-commander",
        "Fleet commander orchestration and status",
    ),
    (
        "ams",
        "fleets",
        "Manage dedicated server fleets and scaling",
    ),
    ("ams", "images", "Container images for dedicated servers"),
    ("ams", "info", "AMS regional info and supported instances"),
    ("ams", "qos", "Quality of service regions and latency"),
    (
        "ams",
        "watchdogs",
        "Server health monitoring and watchdog heartbeats",
    ),
    // basic
    ("basic", "config", "Publisher and namespace configuration"),
    (
        "basic",
        "files",
        "File upload management and URL generation",
    ),
    (
        "basic",
        "locale",
        "Country, language, and time zone lookups",
    ),
    (
        "basic",
        "namespaces",
        "Namespace creation, configuration, and management",
    ),
    (
        "basic",
        "profiles",
        "User profile data and custom attributes",
    ),
    // challenge
    (
        "challenge",
        "assignment-plugins",
        "Challenge plugin configuration and assignment",
    ),
    (
        "challenge",
        "challenges",
        "Challenge definitions and configuration",
    ),
    (
        "challenge",
        "goals",
        "Challenge goal definitions and requirements",
    ),
    (
        "challenge",
        "progress",
        "Player challenge progression tracking",
    ),
    (
        "challenge",
        "schedules",
        "Challenge scheduling and rotation",
    ),
    ("challenge", "utilities", "Challenge utility operations"),
    // chat
    ("chat", "config", "Chat service configuration"),
    ("chat", "inbox", "Admin inbox messages and categories"),
    (
        "chat",
        "moderation",
        "Chat moderation, snapshots, and topic management",
    ),
    (
        "chat",
        "profanity",
        "Profanity filter configuration and word lists",
    ),
    (
        "chat",
        "service-messages",
        "System and service message management",
    ),
    ("chat", "topics", "Chat topic management and membership"),
    // cloudsave
    (
        "cloudsave",
        "game-files",
        "Binary game file storage and management",
    ),
    ("cloudsave", "game-records", "JSON game record management"),
    (
        "cloudsave",
        "game-records-protected",
        "Protected game records with restricted access",
    ),
    (
        "cloudsave",
        "plugin-config",
        "Cloud save plugin configuration and certificates",
    ),
    ("cloudsave", "tags", "Record tag management"),
    (
        "cloudsave",
        "user-files",
        "Binary user file storage and management",
    ),
    ("cloudsave", "user-records", "JSON user record management"),
    (
        "cloudsave",
        "user-records-protected",
        "Protected user records with restricted access",
    ),
    // csm
    ("csm", "app-ui", "App UI hosting and asset management"),
    ("csm", "apps", "Custom service application lifecycle"),
    (
        "csm",
        "config",
        "Service configuration variables and secrets",
    ),
    ("csm", "deployments", "Service deployment management"),
    ("csm", "files", "Hosted static file access"),
    ("csm", "images", "Container image registry and management"),
    (
        "csm",
        "key-value",
        "Key-Value store configuration and management",
    ),
    (
        "csm",
        "nosql",
        "NoSQL database configuration and management",
    ),
    ("csm", "resource-limits", "Resource limits and quotas"),
    (
        "csm",
        "service-messages",
        "Service messaging and notifications",
    ),
    ("csm", "sql", "SQL database configuration and management"),
    (
        "csm",
        "subscriptions",
        "Notification subscription management",
    ),
    (
        "csm",
        "topics",
        "Async messaging topics and subscriber management",
    ),
    // gametelemetry
    (
        "gametelemetry",
        "events",
        "Telemetry event ingestion and querying",
    ),
    (
        "gametelemetry",
        "namespaces",
        "Telemetry namespace configuration",
    ),
    (
        "gametelemetry",
        "steam-playtime",
        "Steam playtime data and analytics",
    ),
    // gdpr
    ("gdpr", "config", "GDPR service plugin configuration"),
    (
        "gdpr",
        "platform-closure",
        "Platform account closure operations",
    ),
    (
        "gdpr",
        "user-access-requests",
        "User data access requests and downloads",
    ),
    (
        "gdpr",
        "user-access-requests-s2s",
        "Server-to-server data access operations",
    ),
    (
        "gdpr",
        "user-deletion-requests",
        "User data deletion requests and status",
    ),
    (
        "gdpr",
        "user-deletion-requests-s2s",
        "Server-to-server data deletion operations",
    ),
    // group
    (
        "group",
        "group-definitions",
        "Global group type and configuration definitions",
    ),
    (
        "group",
        "groups",
        "Create, update, and manage player groups",
    ),
    ("group", "membership", "Group membership management"),
    (
        "group",
        "requests",
        "Group join and invite request handling",
    ),
    ("group", "roles", "Group role definitions and permissions"),
    // iam
    (
        "iam",
        "age-restrictions",
        "Age restriction settings per country",
    ),
    ("iam", "allow-list", "Login allowlist for restricted access"),
    ("iam", "bans", "Ban management and ban type configuration"),
    (
        "iam",
        "client-config",
        "OAuth client configuration templates",
    ),
    ("iam", "clients", "OAuth client application management"),
    ("iam", "config", "IAM service configuration"),
    ("iam", "countries", "Country and region settings"),
    ("iam", "devices", "Device bans and device management"),
    (
        "iam",
        "input-validations",
        "Input validation rules for user fields",
    ),
    (
        "iam",
        "oauth2",
        "OAuth 2.0 token and authorization management",
    ),
    (
        "iam",
        "platform-credentials",
        "Login provider credentials, including Device ID",
    ),
    (
        "iam",
        "profile-update-strategies",
        "Profile field update strategy settings",
    ),
    (
        "iam",
        "role-override",
        "Role override configuration per namespace",
    ),
    (
        "iam",
        "roles",
        "Role definitions, permissions, and assignment",
    ),
    ("iam", "sso", "Single sign-on authentication flows"),
    (
        "iam",
        "sso-credentials",
        "SSO credential and platform login configuration",
    ),
    ("iam", "tags", "User account tag management"),
    (
        "iam",
        "user-mfa",
        "User multi-factor authentication configuration",
    ),
    (
        "iam",
        "users",
        "User accounts, profiles, bans, and permissions",
    ),
    // inventory
    (
        "inventory",
        "chaining",
        "Chained inventory operations (multi-step)",
    ),
    (
        "inventory",
        "integrations",
        "Inventory integration configuration",
    ),
    (
        "inventory",
        "inventories",
        "Manage player inventory instances",
    ),
    (
        "inventory",
        "inventory-definitions",
        "Inventory type and schema definitions",
    ),
    ("inventory", "item-types", "Item type definitions"),
    ("inventory", "items", "Inventory item management"),
    ("inventory", "tags", "Inventory tag management"),
    // leaderboard
    (
        "leaderboard",
        "global-rankings",
        "Global leaderboard entries, scores, and rankings",
    ),
    (
        "leaderboard",
        "leaderboards",
        "Leaderboard configuration and definitions",
    ),
    (
        "leaderboard",
        "user-rankings",
        "Per-user leaderboard data and rankings",
    ),
    (
        "leaderboard",
        "visibility",
        "User leaderboard visibility settings",
    ),
    // legal
    (
        "legal",
        "agreements",
        "Agreement acceptance and status tracking",
    ),
    (
        "legal",
        "base-policies",
        "Base legal policy templates and management",
    ),
    (
        "legal",
        "eligibilities",
        "User eligibility checks for agreements",
    ),
    (
        "legal",
        "localized-versions",
        "Localized policy version content",
    ),
    ("legal", "policies", "Policy management and publishing"),
    (
        "legal",
        "policy-versions",
        "Policy version lifecycle management",
    ),
    ("legal", "utility", "Legal utility operations"),
    ("legal", "versions", "Legal policy version management"),
    // lobby
    ("lobby", "blocks", "Player block list management"),
    ("lobby", "config", "Lobby service configuration"),
    (
        "lobby",
        "friends",
        "Friend list management and friend requests",
    ),
    ("lobby", "notifications", "Push notifications and templates"),
    ("lobby", "presence", "Online presence and status tracking"),
    (
        "lobby",
        "service-messages",
        "System and service message management",
    ),
    ("lobby", "users", "Lobby user state and settings"),
    // loginqueue
    (
        "loginqueue",
        "queue",
        "Login queue administration, configuration, and tickets",
    ),
    // match2
    (
        "match2",
        "backfill",
        "Match backfill proposals and acceptance",
    ),
    ("match2", "config", "Matchmaking service configuration"),
    (
        "match2",
        "feature-flags",
        "Matchmaking feature flag management",
    ),
    (
        "match2",
        "match-functions",
        "Custom match function management",
    ),
    (
        "match2",
        "match-pools",
        "Match pool configuration and metrics",
    ),
    (
        "match2",
        "match-tickets",
        "Match ticket creation and tracking",
    ),
    ("match2", "rule-sets", "Matchmaking rule set definitions"),
    (
        "match2",
        "xray-config",
        "Matchmaking X-Ray diagnostic configuration",
    ),
    // platform
    (
        "platform",
        "campaign-codes",
        "License code and key management",
    ),
    (
        "platform",
        "campaigns",
        "Campaign and redemption code management",
    ),
    (
        "platform",
        "catalog-changes",
        "Store catalog change tracking and publishing",
    ),
    ("platform", "categories", "Store item category management"),
    (
        "platform",
        "clawback",
        "Payment clawback handling and notifications",
    ),
    (
        "platform",
        "currencies",
        "Virtual currency definitions and management",
    ),
    (
        "platform",
        "dlc-config",
        "Downloadable content configuration",
    ),
    ("platform", "dlc-records", "Platform DLC sync and records"),
    (
        "platform",
        "entitlements",
        "User entitlement grants, revocations, and queries",
    ),
    (
        "platform",
        "fulfillment-script",
        "Custom fulfillment script management",
    ),
    (
        "platform",
        "fulfillments",
        "Order fulfillment and item granting",
    ),
    (
        "platform",
        "iap",
        "In-app purchase configuration and validation",
    ),
    ("platform", "iap-apple", "Apple App Store IAP configuration"),
    (
        "platform",
        "iap-epicgames",
        "Epic Games Store IAP configuration",
    ),
    ("platform", "iap-google", "Google Play IAP configuration"),
    (
        "platform",
        "iap-notifications",
        "IAP notification processing",
    ),
    ("platform", "iap-oculus", "Meta/Oculus IAP configuration"),
    (
        "platform",
        "iap-playstation",
        "PlayStation Store IAP configuration",
    ),
    ("platform", "iap-steam", "Steam IAP configuration"),
    (
        "platform",
        "iap-subscriptions",
        "IAP subscription management and sync",
    ),
    ("platform", "iap-twitch", "Twitch IAP configuration"),
    ("platform", "iap-xbox", "Xbox IAP configuration"),
    (
        "platform",
        "invoices",
        "Payment invoice queries and management",
    ),
    ("platform", "items", "Store item definitions and management"),
    (
        "platform",
        "key-groups",
        "License key group management and distribution",
    ),
    (
        "platform",
        "orders",
        "Purchase order management and processing",
    ),
    (
        "platform",
        "payment",
        "Payment processing and transaction management",
    ),
    (
        "platform",
        "payment-accounts",
        "User payment account management",
    ),
    (
        "platform",
        "payment-config",
        "Payment provider configuration",
    ),
    (
        "platform",
        "payment-station",
        "Payment station UI and flow management",
    ),
    ("platform", "payments", "Dedicated payment processing"),
    (
        "platform",
        "platform-achievements",
        "Platform achievement event processing",
    ),
    (
        "platform",
        "platform-closure",
        "Platform account closure configuration",
    ),
    (
        "platform",
        "platform-sessions",
        "Platform session management",
    ),
    (
        "platform",
        "plugin-config",
        "Platform service plugin configuration",
    ),
    ("platform", "revocations", "Entitlement and item revocation"),
    (
        "platform",
        "rewards",
        "Reward definitions and condition configuration",
    ),
    (
        "platform",
        "sections",
        "Store section layout and content management",
    ),
    (
        "platform",
        "stores",
        "Store management, cloning, and publishing",
    ),
    (
        "platform",
        "subscriptions",
        "Subscription plan management and billing",
    ),
    ("platform", "tickets", "Support ticket and key distribution"),
    ("platform", "trades", "Trade action processing and history"),
    ("platform", "views", "Store view definitions and layout"),
    (
        "platform",
        "wallets",
        "User wallet balances and transactions",
    ),
    // reporting
    ("reporting", "config", "Reporting service configuration"),
    (
        "reporting",
        "extensions",
        "Report extension categories and auto-moderation",
    ),
    ("reporting", "reasons", "Reporting reason management"),
    ("reporting", "reports", "Report queries and management"),
    ("reporting", "rules", "Auto-moderation rule definitions"),
    (
        "reporting",
        "tickets",
        "Moderation ticket management and resolution",
    ),
    // seasonpass
    (
        "seasonpass",
        "passes",
        "Season pass definitions and management",
    ),
    (
        "seasonpass",
        "progress",
        "Player season pass progress and rewards",
    ),
    ("seasonpass", "rewards", "Season pass reward configuration"),
    (
        "seasonpass",
        "seasons",
        "Season lifecycle, publishing, and tiers",
    ),
    (
        "seasonpass",
        "tiers",
        "Season tier definitions and ordering",
    ),
    ("seasonpass", "utilities", "Season pass utility operations"),
    // session
    (
        "session",
        "alerts",
        "Session alert configuration and management",
    ),
    ("session", "config", "Session service log configuration"),
    (
        "session",
        "game-sessions",
        "Game session lifecycle and management",
    ),
    ("session", "parties", "Party session management"),
    (
        "session",
        "platform-credentials",
        "Platform credential configuration for sessions",
    ),
    ("session", "recent-users", "Recent player history"),
    ("session", "storage", "Session storage management"),
    ("session", "templates", "Session configuration templates"),
    ("session", "users", "Player session attributes and queries"),
    // sessionhistory
    (
        "sessionhistory",
        "xray-tickets",
        "Matchmaking X-Ray analytics and diagnostics",
    ),
    // social
    (
        "social",
        "global-stat-values",
        "Global statistic configuration and values",
    ),
    (
        "social",
        "stat-cycles",
        "Statistic cycle configuration and lifecycle",
    ),
    (
        "social",
        "stat-definitions",
        "Statistic definitions and configuration",
    ),
    (
        "social",
        "user-stat-cycles",
        "Per-user statistic cycle data",
    ),
    (
        "social",
        "user-stat-values",
        "Per-user statistic values and operations",
    ),
    // ugc
    (
        "ugc",
        "channels",
        "UGC channel management and configuration",
    ),
    ("ugc", "config", "UGC service configuration"),
    (
        "ugc",
        "content",
        "User-generated content items and discovery",
    ),
    ("ugc", "creators", "UGC creator profiles and statistics"),
    (
        "ugc",
        "engagements",
        "Content engagement (likes, downloads, follows)",
    ),
    ("ugc", "groups", "UGC group management"),
    ("ugc", "staging", "UGC staging area management"),
    ("ugc", "staging-content", "UGC staged content items"),
    ("ugc", "tags", "UGC content tag management"),
    ("ugc", "types", "UGC content type definitions"),
];

/// Look up the hand-authored description for a (service, resource) pair
pub fn get_resource_description(service: &str, resource: &str) -> Option<&'static str> {
    RESOURCE_DESCRIPTIONS
        .iter()
        .find(|(service_name, resource_name, _)| {
            *service_name == service && *resource_name == resource
        })
        .map(|(_, _, description)| *description)
}

#[cfg(test)]
mod new_service_description_tests {
    use super::{display_name, internal_name, service_description, SERVICES};

    /// Newly added services should ship with a non-empty one-line description.
    #[test]
    fn test_gametelemetry_has_description() {
        assert!(
            !service_description("gametelemetry").is_empty(),
            "gametelemetry should have a description"
        );
    }

    /// Newly added services should ship with a non-empty one-line description.
    #[test]
    fn test_ugc_has_description() {
        assert!(
            !service_description("ugc").is_empty(),
            "ugc should have a description"
        );
    }

    /// Internal names that match a manifest entry resolve to themselves.
    #[test]
    fn test_internal_name_direct_match() {
        assert_eq!(internal_name("iam"), Some("iam"));
        assert_eq!(internal_name("platform"), Some("platform"));
        assert_eq!(internal_name("achievement"), Some("achievement"));
    }

    /// CLI display names resolve back to their internal ids.
    #[test]
    fn test_internal_name_display_to_internal() {
        assert_eq!(internal_name("matchmaking"), Some("match2"));
        assert_eq!(internal_name("cloud-save"), Some("cloudsave"));
        assert_eq!(internal_name("login-queue"), Some("loginqueue"));
        assert_eq!(internal_name("season-pass"), Some("seasonpass"));
        assert_eq!(internal_name("session-history"), Some("sessionhistory"));
        assert_eq!(internal_name("game-telemetry"), Some("gametelemetry"));
        assert_eq!(internal_name("ugc"), Some("ugc"));
    }

    /// Unknown service names return None to prevent invalid API calls.
    #[test]
    fn test_internal_name_rejects_unknown() {
        assert_eq!(internal_name("badservice"), None);
        assert_eq!(internal_name(""), None);
    }

    /// Path traversal attempts are rejected because they are not valid service names.
    #[test]
    fn test_internal_name_rejects_path_traversal() {
        assert_eq!(internal_name("../../../etc/passwd"), None);
        assert_eq!(internal_name("/etc/passwd"), None);
        assert_eq!(internal_name("iam/../secret"), None);
    }

    /// Services without a display override use their internal name directly.
    #[test]
    fn test_display_name_identity() {
        assert_eq!(display_name("iam"), Some("iam"));
        assert_eq!(display_name("platform"), Some("platform"));
    }

    /// Services with display overrides map to user-friendly CLI names.
    #[test]
    fn test_display_name_overrides() {
        assert_eq!(display_name("match2"), Some("matchmaking"));
        assert_eq!(display_name("cloudsave"), Some("cloud-save"));
        assert_eq!(display_name("loginqueue"), Some("login-queue"));
        assert_eq!(display_name("seasonpass"), Some("season-pass"));
        assert_eq!(display_name("sessionhistory"), Some("session-history"));
        assert_eq!(display_name("gametelemetry"), Some("game-telemetry"));
        assert_eq!(display_name("ugc"), Some("ugc"));
    }

    /// Unknown internal names return None rather than a fabricated display name.
    #[test]
    fn test_display_name_unknown_returns_none() {
        assert_eq!(display_name("nonexistent"), None);
    }

    /// Every service can round-trip from internal to display and back without data loss.
    #[test]
    fn test_all_services_round_trip() {
        for service in SERVICES {
            let back = internal_name(service.display);
            assert_eq!(
                back,
                Some(service.internal),
                "Round-trip failed for {} → {} → {back:?}",
                service.internal,
                service.display
            );
        }
    }
}
