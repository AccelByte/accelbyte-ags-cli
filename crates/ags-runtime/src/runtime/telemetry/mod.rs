//! Best-effort CLI usage telemetry.
//!
//! Emits a single `cli.command.invoked` event to PostHog for every invocation,
//! authenticated or not — see [`ResolvedIdentity`]. An authenticated
//! invocation is keyed on the CLI's own AGS user id (`sub`, decoded from the
//! stored access token); an unauthenticated one (no stored token yet, e.g.
//! before the first successful `ags auth login`) is keyed on this install's
//! own random, machine-local anonymous id instead of being dropped — see
//! `identity resolution` in [`resolve_identity`].
//!
//! AGS issues per-namespace user ids, so this `sub` (game/namespace-scoped) is not the
//! id the Admin Portal identifies on for the same person, and cross-namespace id
//! resolution is subdomain-gated (unavailable from the CLI's context). A shared person
//! property can't bridge two different PostHog persons for funnel/group queries either
//! — that's why the actual cross-surface key is the `studio`/`game_namespace` `$group`s
//! (see [`build_event`]), matching the Admin Portal's group type keys. `studio` costs no
//! network round-trip: it's the token's own `parent_namespace` claim, decoded alongside
//! `sub` — see [`decode_parent_namespace`]. Email is still attached, best-effort via a
//! self `users/me` lookup, but only as a `$set` **person** property, never a top-level
//! event property (PII-minimization), for a human to look a person up by in the PostHog
//! UI — not as an aggregation key. An anonymous invocation never carries an email or a
//! studio — there is no user or token to look either up from, and skipping the lookup
//! entirely also spares that invocation the `users/me` network round-trip.
//!
//! Every path here is strictly fire-and-forget: telemetry is a cheap no-op
//! unless [`ENV_POSTHOG_KEY`] is set, and no failure ever propagates to the
//! caller or affects a command's behaviour or exit code.

use base64::Engine;
use posthog_rs::{ClientOptionsBuilder, Event};

/// Env var: PostHog project API key. Telemetry is disabled unless this is set
/// to a non-empty value. The key is never compiled in.
pub const ENV_POSTHOG_KEY: &str = "AGS_TELEMETRY_POSTHOG_KEY";

/// Env var: PostHog host override. Defaults to the US ingestion endpoint
/// ([`DEFAULT_POSTHOG_HOST`]).
pub const ENV_POSTHOG_HOST: &str = "AGS_TELEMETRY_POSTHOG_HOST";

/// Env var: universal telemetry opt-out (<https://consoledonottrack.com>). Any
/// non-empty value disables telemetry regardless of [`ENV_POSTHOG_KEY`].
pub const ENV_DO_NOT_TRACK: &str = "DO_NOT_TRACK";

/// Env var: operator kill switch for `input_fields` value transmission. Any
/// non-empty value forces every `StepInputField.value` to `None` for a
/// bundled workflow's failed step — field name, location, source and
/// required-ness still transmit, exactly as an external (non-bundled)
/// workflow already behaves — while leaving every other telemetry event
/// untouched. Lets a security reviewer disable this one behaviour without
/// disabling telemetry outright (see §4.1 of the CLI telemetry observability
/// design).
pub const ENV_TELEMETRY_NO_INPUT_VALUES: &str = "AGS_TELEMETRY_NO_INPUT_VALUES";

/// Env var: when set (non-empty), print the telemetry decision path to stderr.
/// Prints the decoded `sub` and a domain-only redacted `email` (see
/// [`redact_email_for_debug`]), never the key.
const ENV_DEBUG: &str = "AGS_TELEMETRY_DEBUG";

/// Process-global hook that receives formatted `tdbg!` diagnostic lines.
///
/// The runtime layer must never write to stdout/stderr directly (enforced by
/// `test_runtime_layer_has_no_user_facing_io`), so this mirrors
/// `crate::support::register_lock_contention_reporter`: the `accelbyte-ags-cli`
/// binary registers a stderr-writing callback at startup, and `tdbg!` below
/// calls through it instead of doing I/O itself. Until registered (e.g. in a
/// test binary that never wires one up), `tdbg!` is simply inert.
static DEBUG_REPORTER: std::sync::OnceLock<fn(&str)> = std::sync::OnceLock::new();

/// Register the process-global hook that receives `tdbg!` diagnostic lines.
/// Subsequent registrations are ignored; the first reporter wins for the
/// life of the process.
pub fn register_debug_reporter(reporter: fn(&str)) {
    let _ = DEBUG_REPORTER.set(reporter);
}

/// Format and forward a `[telemetry]` diagnostic line to the registered
/// debug reporter when [`ENV_DEBUG`] is set. `posthog-rs` never surfaces
/// delivery errors, so this is the only visibility into why an event was
/// (or was not) sent. A no-op (aside from the env lookup) whenever the flag
/// is unset or no reporter has been registered.
macro_rules! tdbg {
    ($($arg:tt)*) => {{
        if std::env::var_os(ENV_DEBUG).is_some_and(|value| !value.is_empty()) {
            if let Some(reporter) = DEBUG_REPORTER.get().copied() {
                reporter(&format!("[telemetry] {}", format_args!($($arg)*)));
            }
        }
    }};
}

/// Ingestion host, used unless [`ENV_POSTHOG_HOST`] overrides it. Defaults to
/// the US region; it must point at the same region as the Admin Portal's
/// PostHog project for the identity join to land there. Override via
/// [`ENV_POSTHOG_HOST`] to target a different region's project.
const DEFAULT_POSTHOG_HOST: &str = "https://us.i.posthog.com";

/// Event name emitted once per CLI invocation.
const EVENT_COMMAND_INVOKED: &str = "cli.command.invoked";

/// PostHog group type used to attach the AccelByte game namespace as a `$group`.
/// Named to match the Admin Portal's group type key so the two surfaces'
/// group analytics resolve to the same PostHog group.
const NAMESPACE_GROUP: &str = "game_namespace";

/// PostHog group type used to attach the AccelByte studio/publisher namespace
/// as a `$group`. Named to match the Admin Portal's group type key.
const STUDIO_GROUP: &str = "studio";

/// Decode an AccelByte access token's JWT claims to a raw JSON value. Pure, no I/O.
///
/// AccelByte IAM access tokens minted by the interactive `authorization_code`
/// grant are JWTs whose middle segment carries the claims. The signature is
/// intentionally NOT verified: the CLI already stored and trusts this token,
/// so this only base64url-decodes the claims for a specific field to be read
/// out of afterwards.
///
/// Returns `None` for anything that is not a 3-segment JWT with a JSON object
/// payload (e.g. an opaque `AGS_ACCESS_TOKEN` passthrough token that never was
/// a JWT).
///
/// `pub(crate)`: the no-signature-verification trust assumption only holds
/// for tokens this crate already retrieved from the local credential store.
pub(crate) fn decode_claims(access_token: &str) -> Option<serde_json::Value> {
    let mut segments = access_token.split('.');
    let _header = segments.next()?;
    let payload = segments.next()?;
    let _signature = segments.next()?;
    if segments.next().is_some() {
        return None;
    }

    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&decoded).ok()
}

/// Decode the IAM user id (`sub`) from an AccelByte access token.
///
/// A `client_credentials`-grant token IS a valid 3-segment JWT (its `sub` is
/// the IAM Client's id, not a person's) — it does NOT get skipped here; see
/// `gather_context`'s doc comment for what happens to it downstream.
pub(crate) fn decode_sub(access_token: &str) -> Option<String> {
    match decode_claims(access_token)?.get("sub") {
        Some(serde_json::Value::String(sub)) => Some(sub.clone()),
        _ => None,
    }
}

/// Decode the studio/publisher namespace (`parent_namespace`) from an
/// AccelByte access token, distinct from the game-scoped `namespace` claim.
/// Absent on tokens that were never scoped under a publisher namespace (e.g.
/// some `client_credentials` grants), in which case there is no studio to
/// group by and the caller should omit the group entirely rather than
/// substitute the game namespace for it.
pub(crate) fn decode_parent_namespace(access_token: &str) -> Option<String> {
    match decode_claims(access_token)?.get("parent_namespace") {
        Some(serde_json::Value::String(parent_namespace)) => Some(parent_namespace.clone()),
        _ => None,
    }
}

/// Gather everything needed for a `cli.command.invoked` event, ahead of
/// command dispatch. Best-effort and fully non-fatal: returns `None` only
/// when telemetry is disabled ([`ENV_POSTHOG_KEY`]/[`ENV_DO_NOT_TRACK`], see
/// [`resolve_identity`]) or `command_path` is empty (a meta/builtin
/// invocation). Every other invocation — authenticated or not — produces a
/// context: see [`ResolvedIdentity`] for the two attribution branches.
///
/// `command_path` is the raw post-global-flag argument vector; only its
/// leading bare-word command names are recorded (see [`command_path_names`])
/// and only allowlisted flag values are kept (see [`extract_flags`]), so no
/// unsafe flag or argument value is ever transmitted.
///
/// The event itself is not sent here — call [`emit_with_outcome`] once the
/// command's outcome is known, immediately before the process exits.
#[allow(clippy::too_many_arguments)]
pub async fn gather_context(
    profile_flag: Option<&str>,
    namespace_flag: Option<&str>,
    global_flags: &[(String, Option<String>)],
    command_path: &[String],
    cli_version: &str,
    workflow_run_id: Option<&str>,
    ui_surface: &'static str,
    workflow_id: Option<&str>,
) -> Option<CommandTelemetry> {
    let started_at = std::time::Instant::now();
    if !is_enabled_by_env() {
        tdbg!("disabled — AGS_TELEMETRY_POSTHOG_KEY is unset/blank or DO_NOT_TRACK is set");
        return None;
    }

    let command_names = command_path_names(command_path);
    if command_names.is_empty() {
        // A meta/builtin invocation (e.g. `ags --version`, `ags --help`) whose
        // `command_path` starts with a flag: an empty `command_path` event is
        // near-zero-information noise, and skipping it here — before ever
        // resolving an identity — also means we never pay for the token-store
        // read, never create this install's anon-id file on first run, and
        // never pay for the `fetch_email` network round-trip below.
        tdbg!("skipping — command_path is empty (meta/builtin invocation)");
        return None;
    }

    let Some(identity) = resolve_identity(profile_flag).await else {
        tdbg!("could not resolve an identity to attribute this event to");
        return None;
    };

    let namespace = crate::runtime::execution::resolve_namespace(namespace_flag, profile_flag)
        .map(|(namespace, _source)| namespace);

    // An anonymous (pre-login) invocation has no user to look up: skip the
    // `users/me` round-trip entirely, and leave `auth_grant` `None` — there is
    // no token to classify a grant type from.
    let (email, studio, auth_grant) = match &identity {
        ResolvedIdentity::Authenticated { .. } => resolve_authenticated_extras(profile_flag).await,
        ResolvedIdentity::Anonymous { .. } => {
            tdbg!("anonymous — skipping the users/me email lookup, no user to look up");
            (None, None, None)
        }
    };
    tdbg!(
        "enabled; identity={}, distinct_id='{}', email={:?}, command='{command_names}', namespace={namespace:?}, studio={studio:?}",
        identity.label(),
        identity.distinct_id(),
        email.as_deref().map(redact_email_for_debug)
    );

    let mut flags = extract_flags(command_path);
    merge_global_flags(&mut flags, global_flags);

    Some(CommandTelemetry {
        distinct_id: identity.distinct_id().to_string(),
        identity: identity.label(),
        ui_surface,
        email,
        namespace,
        studio,
        command_path: command_names,
        flags,
        cli_version: cli_version.to_string(),
        auth_grant,
        started_at,
        workflow_run_id: workflow_run_id.map(str::to_string),
        workflow_id: workflow_id.map(str::to_string),
    })
}

/// Resolve the email (via cache or a best-effort `users/me` lookup), the
/// studio namespace, and the auth-grant classification for an authenticated
/// invocation. Only called once [`gather_context`] already knows the
/// identity is [`ResolvedIdentity::Authenticated`] — an anonymous identity
/// has no token to look any of these up from, so this is never called in
/// that case. Best-effort: any resolution failure (profile, token store)
/// yields `(None, None, None)` rather than propagating.
///
/// Cross-surface join key: AGS issues per-namespace user ids, so the CLI's
/// `sub` (a game/namespace-scoped id) is not the Admin Portal's user id for
/// the same person, and cross-namespace id resolution is subdomain-gated
/// (unavailable here). Email is the one identifier shared and reachable on
/// both surfaces, so the Portal identifies on email and the CLI keys on it
/// too. Cached on disk keyed to a fingerprint of this exact access token (see
/// `read_cached_email`) so the `users/me` round-trip only fires once per
/// token, not once per invocation; fetched best-effort on a cache miss, and
/// on failure we fall back to no email so the event still emits (it just may
/// not join).
///
/// The studio namespace, unlike email, needs no network round-trip: it's the
/// token's own `parent_namespace` claim, decoded alongside `sub` from the
/// same JWT the CLI already holds — see [`decode_parent_namespace`].
async fn resolve_authenticated_extras(
    profile_flag: Option<&str>,
) -> (Option<String>, Option<String>, Option<&'static str>) {
    let Ok(profile) = crate::runtime::config::resolve_profile_name(profile_flag) else {
        return (None, None, None);
    };
    // Held live only until `read_cached_email`/`fetch_email`/`decode_parent_namespace`
    // read it; `TokenData` is zeroized on drop.
    let Ok(Some(token)) = crate::runtime::auth::store::get_token_data_async(&profile).await else {
        return (None, None, None);
    };
    let email = match read_cached_email(&profile, &token.access_token) {
        Some(email) => {
            tdbg!("email cache hit for profile '{profile}'");
            Some(email)
        }
        None => {
            let fetched = fetch_email(&profile, &token.access_token).await;
            if let Some(email) = &fetched {
                write_cached_email(&profile, &token.access_token, email);
            }
            fetched
        }
    };
    let studio = decode_parent_namespace(&token.access_token);
    (email, studio, classify_auth_grant(token.grant_type))
}

/// Redact an email to `***@domain` for [`ENV_DEBUG`] tracing, so a shared or
/// CI log never carries the full address. Falls back to `"***"` if there's
/// no `@`.
fn redact_email_for_debug(email: &str) -> String {
    match email.split_once('@') {
        Some((_, domain)) => format!("***@{domain}"),
        None => "***".to_string(),
    }
}

/// Which person a telemetry event is attributed to: a logged-in user, or —
/// pre-login — this install's own anonymous id.
pub enum ResolvedIdentity {
    /// A logged-in user: the IAM `sub` decoded from the stored access token.
    Authenticated {
        /// The decoded IAM `sub`.
        sub: String,
    },
    /// No usable token: a random, machine-local anon id, merged into the real
    /// person by `$identify` on the next successful login.
    Anonymous {
        /// This install's anon id (see [`load_or_create_install_identity`]).
        install_id: String,
    },
}

impl ResolvedIdentity {
    /// The PostHog `distinct_id`.
    pub fn distinct_id(&self) -> &str {
        match self {
            ResolvedIdentity::Authenticated { sub } => sub,
            ResolvedIdentity::Anonymous { install_id } => install_id,
        }
    }

    /// Stable label distinguishing the two cases on the event itself, so a
    /// query can filter pre-login traffic out (or in).
    pub fn label(&self) -> &'static str {
        match self {
            ResolvedIdentity::Authenticated { .. } => "authenticated",
            ResolvedIdentity::Anonymous { .. } => "anonymous",
        }
    }
}

/// Resolve the identity to attribute events to: the stored token's `sub` when
/// there is one, otherwise this install's anon id. `None` only when telemetry
/// is disabled. Used both by [`gather_context`] and by a registered workflow
/// run to tag its `cli.workflow.step_*` events with the same identity
/// `cli.command.invoked` will use — cheap (no network), so callers can
/// `.await` it directly instead of backgrounding it the way `gather_context`'s
/// email fetch is backgrounded.
pub async fn resolve_identity(profile_flag: Option<&str>) -> Option<ResolvedIdentity> {
    if !is_enabled_by_env() {
        return None;
    }
    let profile = crate::runtime::config::resolve_profile_name(profile_flag).ok()?;
    if let Ok(Some(token)) = crate::runtime::auth::store::get_token_data_async(&profile).await {
        if let Some(sub) = decode_sub(&token.access_token) {
            return Some(ResolvedIdentity::Authenticated { sub });
        }
    }
    load_or_create_install_identity(&profile).map(|identity| ResolvedIdentity::Anonymous {
        install_id: identity.install_id,
    })
}

/// Upper bound on [`emit_with_outcome`]'s flush, so a stalled connection to
/// the ingestion host can't add unbounded latency to a command's exit.
const EMIT_FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Build the event from `ctx` and `outcome`, capture it, and flush.
///
/// Explicit flush is needed because `posthog-rs` sends on a background
/// worker and the CLI's self-owned routes exit via `std::process::exit`,
/// bypassing `Drop`. Bounded by [`EMIT_FLUSH_TIMEOUT`]. No-op when telemetry
/// is disabled.
pub async fn emit_with_outcome(ctx: &CommandTelemetry, outcome: Outcome) {
    let client = TelemetryClient::from_env().await;
    client.capture(build_event(ctx, &outcome));
    let _ = tokio::time::timeout(EMIT_FLUSH_TIMEOUT, client.flush()).await;
}

/// Upper bound on the `users/me` lookup so a slow or hanging IAM endpoint
/// cannot add unbounded latency ahead of the event this feeds
/// (`gather_context` now runs concurrently with command dispatch — see its
/// caller in `accelbyte-ags-cli`'s `run()` — but its result is still joined
/// before the process exits, so an unbounded call there could still delay
/// exit past the command's own completion).
const FETCH_EMAIL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(800);

/// Fetch the authenticated user's email via `GET {base_url}/iam/v3/admin/users/me`.
///
/// A self-lookup of the token's own user, so it needs no cross-namespace access and
/// sidesteps the subdomain-gated mapping endpoints. Best-effort: any failure (including
/// a [`FETCH_EMAIL_TIMEOUT`] timeout) yields `None` and the caller falls back to the
/// game-scoped `sub`.
///
/// Uses [`crate::runtime::dispatch::http::build_http_client`] rather than a bare
/// `reqwest::Client`, so this call gets the same User-Agent, redirect policy,
/// and response-size cap as every other AGS API call.
async fn fetch_email(profile: &str, access_token: &str) -> Option<String> {
    let base_url = crate::runtime::auth::credentials::resolve_base_url_value(profile)?;
    let url = format!("{}/iam/v3/admin/users/me", base_url.trim_end_matches('/'));
    let client = crate::runtime::dispatch::http::build_http_client(None).ok()?;
    let response = client
        .get(url)
        .bearer_auth(access_token)
        .timeout(FETCH_EMAIL_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        tdbg!("users/me lookup returned HTTP {}", response.status());
        return None;
    }
    let body_text = crate::runtime::dispatch::http::read_response_body(response)
        .await
        .ok()?;
    let body: serde_json::Value = serde_json::from_str(&body_text).ok()?;
    body.get("emailAddress")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// On-disk cache entry mapping a fingerprint of the access token that
/// produced `email` to that `email` — see [`read_cached_email`].
#[derive(serde::Serialize, serde::Deserialize)]
struct EmailCache {
    token_fingerprint: u64,
    email: String,
}

/// Path to the per-profile email cache file. Lives alongside the profile's
/// other on-disk state ([`crate::runtime::config::profile_dir`]) rather than
/// inside `auth::store::TokenData` itself: caching a telemetry lookup result
/// is not an auth-store concern, and keeping it out of `TokenData` means its
/// schema can evolve freely without touching the credential-store schema
/// every authenticated command depends on. `None` only when the profile
/// directory itself cannot be resolved.
fn email_cache_path(profile: &str) -> Option<std::path::PathBuf> {
    Some(
        crate::runtime::config::profile_dir(profile)
            .ok()?
            .join("telemetry_email_cache.json"),
    )
}

/// Cheap, non-cryptographic fingerprint of an access token — used only to
/// detect "the stored token changed since the email was cached", never as a
/// security boundary, so `DefaultHasher`'s lack of cross-version stability
/// guarantees is harmless: a changed fingerprint after a toolchain upgrade
/// just costs one extra cache-miss fetch, not an incorrect result.
fn token_fingerprint(access_token: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    access_token.hash(&mut hasher);
    hasher.finish()
}

/// Read the cached email for `profile`, but only if it was cached against
/// exactly this `access_token` — a token refresh or re-login changes the
/// fingerprint, so the stale entry is treated as a cache miss (never served)
/// rather than actively invalidated; the next successful fetch overwrites it.
/// Best-effort: any read/parse failure (including "no cache file yet") is
/// simply a cache miss, never an error.
///
/// `pub(crate)`: also read directly by `auth::operations`'s login path to
/// pass an already-cached email into `emit_identity_merge` without ever
/// triggering a fresh `users/me` fetch from that latency-sensitive path.
pub(crate) fn read_cached_email(profile: &str, access_token: &str) -> Option<String> {
    let path = email_cache_path(profile)?;
    let json = std::fs::read_to_string(path).ok()?;
    let cache: EmailCache = serde_json::from_str(&json).ok()?;
    (cache.token_fingerprint == token_fingerprint(access_token)).then_some(cache.email)
}

/// Persist `email` as the cached value for `access_token`. Best-effort: any
/// failure (directory creation, serialization, write) is silently dropped —
/// caching is purely an optimization, never load-bearing for telemetry itself.
fn write_cached_email(profile: &str, access_token: &str, email: &str) {
    let Some(path) = email_cache_path(profile) else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if crate::support::file_system::create_dir_restricted(parent).is_err() {
        return;
    }
    let cache = EmailCache {
        token_fingerprint: token_fingerprint(access_token),
        email: email.to_string(),
    };
    if let Ok(json) = serde_json::to_string(&cache) {
        let _ = crate::support::file_system::write_file_restricted(&path, &json);
    }
}

/// Remove the cached email for `profile`, if any. Called on `ags auth
/// logout` so a future login re-fetches rather than reusing a stale value.
/// Best-effort: a missing file is not an error.
pub(crate) fn clear_cached_email(profile: &str) {
    if let Some(path) = email_cache_path(profile) {
        let _ = std::fs::remove_file(path);
    }
}

/// Anonymous identity for pre-login telemetry, plus the `sub` it has already
/// been merged into (if any). Later tasks will `$identify` this id into the
/// logged-in `sub`'s PostHog person on first login, then read
/// `merged_into_sub` here to avoid ever repeating that merge for the same
/// `sub`.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct InstallIdentity {
    /// Random, machine-local id of the form `anon-<32 hex chars>`. Never
    /// derived from hostname, username, or any hardware identifier — see
    /// [`new_install_id`].
    pub install_id: String,
    /// The `sub` this anon id was merged into via `$identify`, if a merge has
    /// happened yet. `Some` means a merge already happened and must not be
    /// repeated for that same `sub`; a *different* `sub` rotates the id
    /// instead (see [`record_identity_merge`]).
    pub merged_into_sub: Option<String>,
}

/// Number of random bytes in a freshly generated install id — 128 bits, the
/// same collision-resistance budget `generate_workflow_run_id` uses for its
/// non-secret random ids.
const INSTALL_ID_RANDOM_BYTES: usize = 16;

/// Prefix stamped on every generated install id, so a PostHog `distinct_id`
/// or `$anon_distinct_id` is recognisable at a glance as this CLI's
/// machine-local anonymous identity rather than a real `sub`.
const INSTALL_ID_PREFIX: &str = "anon-";

/// Filename of the per-profile install-identity file, alongside
/// `telemetry_email_cache.json` in the same profile directory.
const INSTALL_IDENTITY_FILE_NAME: &str = "telemetry_install_id.json";

/// Path to the per-profile install-identity file. `None` only when the
/// profile directory itself cannot be resolved.
fn install_identity_path(profile: &str) -> Option<std::path::PathBuf> {
    Some(
        crate::runtime::config::profile_dir(profile)
            .ok()?
            .join(INSTALL_IDENTITY_FILE_NAME),
    )
}

/// Generate a fresh random anon id: 128 bits ([`INSTALL_ID_RANDOM_BYTES`]) of
/// randomness from `rand`, hex-encoded and prefixed with
/// [`INSTALL_ID_PREFIX`]. Collision resistance is all that is needed here —
/// this is not a secret, and it is deliberately NEVER derived from hostname,
/// username, MAC address, or any other machine/user identifier (the same
/// class of value this codebase has already rejected for `hostname` and
/// `--output` paths as PII).
fn new_install_id() -> String {
    use rand::Rng;
    let bytes: [u8; INSTALL_ID_RANDOM_BYTES] = rand::rng().random();
    let mut id = String::from(INSTALL_ID_PREFIX);
    for byte in bytes {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}

/// Load the install identity for `profile`, creating and persisting one on
/// first use. Returns `None` up front when telemetry is disabled
/// ([`is_enabled_by_env`]), so a user who never sets [`ENV_POSTHOG_KEY`] (or
/// who sets [`ENV_DO_NOT_TRACK`]) never has a file written for them at all —
/// this check happens before any path resolution or filesystem access.
/// Best-effort otherwise: any read/parse failure is treated as "no identity
/// yet" and a fresh one is created, exactly as the email cache treats a
/// missing/corrupt file as a plain miss.
pub(crate) fn load_or_create_install_identity(profile: &str) -> Option<InstallIdentity> {
    if !is_enabled_by_env() {
        return None;
    }
    let path = install_identity_path(profile)?;
    if let Ok(json) = std::fs::read_to_string(&path) {
        if let Ok(identity) = serde_json::from_str::<InstallIdentity>(&json) {
            return Some(identity);
        }
    }
    let identity = InstallIdentity {
        install_id: new_install_id(),
        merged_into_sub: None,
    };
    write_install_identity(profile, &identity);
    Some(identity)
}

/// Record that this install's anon id was merged into `sub` via `$identify`.
/// Leaves the id unchanged (just recording the merge) when it was never
/// merged before, or was already merged into this same `sub`; rotates to a
/// brand-new id with `merged_into_sub: None` when the stored
/// `merged_into_sub` names a *different* `sub` — PostHog refuses to re-merge
/// an already-identified anonymous id, so on a shared machine rotating keeps
/// the next user's pre-login events on a clean anonymous person instead of
/// silently attributing them to the previous user. Best-effort and a no-op
/// when telemetry is disabled or the identity can't be loaded.
pub(crate) fn record_identity_merge(profile: &str, sub: &str) {
    let Some(current) = load_or_create_install_identity(profile) else {
        return;
    };
    let identity = match &current.merged_into_sub {
        Some(existing) if existing != sub => InstallIdentity {
            install_id: new_install_id(),
            merged_into_sub: None,
        },
        _ => InstallIdentity {
            install_id: current.install_id,
            merged_into_sub: Some(sub.to_string()),
        },
    };
    write_install_identity(profile, &identity);
}

/// Persist `identity` for `profile` using the same restricted (0600) file
/// helpers as the email cache. Best-effort: any failure (directory creation,
/// serialization, write) is silently dropped — this is an optimization/cache,
/// never load-bearing for telemetry itself.
fn write_install_identity(profile: &str, identity: &InstallIdentity) {
    let Some(path) = install_identity_path(profile) else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if crate::support::file_system::create_dir_restricted(parent).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string(identity) {
        let _ = crate::support::file_system::write_file_restricted(&path, &json);
    }
}

/// PostHog's identity-merge event name — see [`build_identify_event`].
const EVENT_IDENTIFY: &str = "$identify";

/// Build the `$identify` event that merges this install's anonymous person
/// into the authenticated one. `sub` becomes the event's `distinct_id`;
/// `$anon_distinct_id` is set to `anon_id`, the id the pre-login events were
/// attributed to — this pair is what tells PostHog to merge the two persons.
/// `email`, when present, is attached as a `$set` **person** property (never
/// a top-level event property), matching [`build_event`]'s PII handling.
fn build_identify_event(sub: &str, anon_id: &str, email: Option<&str>) -> posthog_rs::Event {
    let mut event = posthog_rs::Event::new(EVENT_IDENTIFY, sub);
    let _ = event.insert_prop("$lib", LIB_OVERRIDE);
    let _ = event.insert_prop("$anon_distinct_id", anon_id.to_string());
    if let Some(email) = email {
        let _ = event.insert_prop("$set", serde_json::json!({ "email": email }));
    }
    event
}

/// Merge this install's anonymous person into `sub` after a successful login,
/// once per install. A no-op when telemetry is disabled ([`load_or_create_install_identity`]
/// returns `None`) or when no anon identity exists yet for this profile. When
/// a merge into `sub` already happened, this skips capturing a second
/// `$identify` but still calls [`record_identity_merge`] — that call is what
/// performs the id rotation when a *different* `sub` logs in on the same
/// machine afterward; skipping it here would silently keep attributing the
/// new user's pre-login events to the previous one. Fire-and-forget: bounded
/// by [`EMIT_FLUSH_TIMEOUT`], every failure swallowed, never propagates an
/// error or otherwise affects the caller.
pub async fn emit_identity_merge(profile: &str, sub: &str, email: Option<&str>) {
    emit_identity_merge_with(TelemetryClient::from_env(), profile, sub, email).await;
}

/// [`emit_identity_merge`] against a caller-supplied client, so the
/// first-ever-merge path — capture, flush, then `record_identity_merge` — can
/// be driven in a test with a disabled client instead of one built from the
/// environment.
///
/// `client` is taken as an un-awaited future and awaited only on the branch
/// that actually captures: `TelemetryClient::from_env` spawns a background
/// transport thread per instance, and every login after the first takes the
/// skip branch, which never sends anything.
async fn emit_identity_merge_with(
    client: impl std::future::Future<Output = TelemetryClient>,
    profile: &str,
    sub: &str,
    email: Option<&str>,
) {
    let Some(identity) = load_or_create_install_identity(profile) else {
        return;
    };
    if identity.merged_into_sub.is_some() {
        tdbg!("install identity already merged; not re-identifying");
        record_identity_merge(profile, sub);
        return;
    }
    let client = client.await;
    client.capture(build_identify_event(sub, &identity.install_id, email));
    let _ = tokio::time::timeout(EMIT_FLUSH_TIMEOUT, client.flush()).await;
    record_identity_merge(profile, sub);
}

/// Map a stored grant type to the PostHog property value; `None` if unknown.
///
/// `None` covers e.g. an opaque `AGS_ACCESS_TOKEN` passthrough, where no
/// `GrantType` was ever recorded for the stored token.
pub fn classify_auth_grant(
    grant: Option<ags_protocol::request::GrantType>,
) -> Option<&'static str> {
    grant.map(ags_protocol::request::GrantType::as_oauth_param)
}

/// Upper bound on how many leading tokens of `command_path` are routing
/// names, keyed on the first token. Most routes are 3 deep (`<service>
/// <resource> <method>`), but `workflow` and `profile` are only 2 deep —
/// their 3rd token is always a value (a workflow id/path, or a profile
/// name), never a routing word.
fn max_routing_depth(command_path: &[String]) -> usize {
    match command_path.first().map(String::as_str) {
        Some("workflow") | Some("profile") => 2,
        _ => 3,
    }
}

/// Space-joined leading command names from a post-global-flag argument vector.
///
/// Stops at the first token beginning with `-`, and at [`max_routing_depth`],
/// whichever comes first — only routing names like `iam users list` or
/// `workflow run`, never a positional value (a workflow id, file path, or
/// config value).
fn command_path_names(command_path: &[String]) -> String {
    command_path
        .iter()
        .take_while(|token| !token.starts_with('-'))
        .take(max_routing_depth(command_path))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
}

use std::collections::BTreeMap;

/// Flags safe to record with their values; every other flag's value is redacted.
/// `--user-id`/`--client-id` (added 2026-08-14) name AGS resource identifiers,
/// not secrets — useful for correlating which record an operation touched.
///
/// `--output` is deliberately NOT here: its value is a filesystem path
/// (`path.display().to_string()`), which typically embeds the OS username
/// (e.g. `/home/alice/report.json`) — the same class of identifier already
/// rejected for `hostname` as effectively PII. It was present here before
/// the flag-capture pipeline fix, but was dead code until then: `--output`
/// was always stripped out of argv by `pre_scan_global_flags` before
/// `extract_flags` could ever see it, so this entry never actually fired in
/// production. Now that the pipeline fix makes global flags reach this
/// allowlist for real, `--output`'s value must stay redacted.
const VALUE_SAFE_FLAGS: &[&str] = &[
    "--namespace",
    "-n",
    "--format",
    "--ui",
    "--user-id",
    "--client-id",
];

#[derive(Debug, Default, PartialEq, Eq)]
pub struct FlagCapture {
    pub names: Vec<String>,
    pub values: BTreeMap<String, String>,
}

/// Whether `name_part` has the shape every real long flag in this codebase
/// actually has: lowercase ASCII letters, optionally hyphen-joined into
/// words (`namespace`, `client-secret`, `no-color`, `page-limit`, ...) —
/// never empty, never leading/trailing/doubled hyphens, never a digit or
/// an uppercase letter.
///
/// This is deliberately narrower than "starts with a letter": a dash-leading
/// secret can itself start with a letter right after the dash (base64url and
/// hex alphabets both include letters), so checking only the first character
/// still misclassifies a secret like `--abcSECRETvalue123` as a flag. Real
/// secrets, however, contain a digit or an uppercase letter with near
/// certainty within any realistic length — requiring the *whole* name to be
/// lowercase-kebab is the boundary that actually keeps them out.
fn looks_like_flag_name(name_part: &str) -> bool {
    !name_part.is_empty()
        && !name_part.starts_with('-')
        && !name_part.ends_with('-')
        && !name_part.contains("--")
        && name_part
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '-')
}

/// Whether `tok` looks like a real flag: a long flag (`--<lowercase-kebab>`,
/// optionally with `=value`) or a single-character short flag (`-<alpha>`,
/// optionally with `=value`) — but NOT a bare `-` followed by an arbitrary
/// multi-character token.
///
/// Used only to decide whether `tok` itself starts a new flag (both for the
/// top-level scan in [`extract_flags`] and for the inline-`=` split). It is
/// deliberately NOT used to decide whether the *value token following* a
/// flag is eligible to be consumed — see [`BOOLEAN_FLAGS`] for why a
/// shape-based lookahead there is unsafe.
///
/// Trade-off: combined short flags (`-vvv`) and any multi-character short
/// form are NOT recognised as flags here — they fall through and are
/// treated as a *value* instead. That is the conservative direction for a
/// security boundary (it leans toward redacting/consuming a token rather
/// than risking a secret being classified as flag metadata), at the cost of
/// not recording `-vvv`-style combined flags as their own name.
fn looks_like_flag(tok: &str) -> bool {
    let rest = match tok.strip_prefix("--") {
        Some(rest) => {
            let name_part = rest.split('=').next().unwrap_or(rest);
            return looks_like_flag_name(name_part);
        }
        None => match tok.strip_prefix('-') {
            Some(rest) => rest,
            None => return false,
        },
    };
    let name_part = rest.split('=').next().unwrap_or(rest);
    // Single-char short flag only (`-n`, `-n=x`) — not `-vvv`, not a
    // dash-leading secret value.
    name_part.len() == 1
        && name_part
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
}

/// Long/short flag names known to take no value in this codebase (built with
/// clap's `ArgAction::SetTrue`) — the complete set, per `builder.rs`. Every
/// *other* recognised flag is treated as value-taking by default, including
/// every dynamically-generated per-operation flag from the bundled OpenAPI
/// specs: `routes::service::clap_tree::build_service_command_tree` always
/// builds parameter args with clap's default value-taking `Set` action, so
/// there is no dynamic boolean flag this list needs to (or safely could)
/// anticipate.
///
/// This is the actual safety boundary for `extract_flags`'s value handling
/// (see its doc comment): a flag NOT in this list always consumes the next
/// argv token as its value, regardless of that token's own shape. A prior
/// version instead decided consumption by asking whether the next token
/// "looks like a flag" ([`looks_like_flag`]) — but a secret can itself be
/// lowercase-kebab-shaped (e.g. a passphrase like `my-secret-words`), which
/// made it indistinguishable from a real flag name. Such a secret would fail
/// that lookahead, get left unconsumed, and be re-parsed as its own "flag"
/// on the next loop iteration — pushed into `names` verbatim with no
/// redaction. Defaulting new/dynamic flags to value-taking is the safe
/// direction: at worst it swallows a genuinely boolean flag's *name* as the
/// redacted value of the flag before it (a data-quality loss, not a leak,
/// since that swallowed value is redacted unless the preceding flag's name
/// is itself in [`VALUE_SAFE_FLAGS`]).
const BOOLEAN_FLAGS: &[&str] = &[
    "--all",
    "--global",
    "--offline",
    "--validate-only",
    "--client-secret-stdin",
    "--dry-run",
    "--no-color",
    "--no-input",
    "--quiet",
    "-q",
    "--verbose",
    "-v",
    "--yes",
    "-y",
    "--skeleton",
    "--page-all",
    "--symbol-files",
    "--skip-script-validation",
    "--help",
    "-h",
    "--version",
    "-V",
];

/// Extract flag names (always) and values (allowlisted, else `<redacted>`) from raw argv.
///
/// A token is only ever recorded in `names`, or consumed as a flag's value,
/// when [`looks_like_flag`] recognises it as the flag itself. The following
/// token is that flag's value whenever the flag is not in [`BOOLEAN_FLAGS`]
/// — unconditionally, regardless of the value token's own shape — which is
/// what actually closes the leak where a dash-leading (or plain
/// lowercase-kebab) secret value would otherwise be re-parsed as its own
/// flag name on the next iteration and emitted verbatim with no redaction.
/// A token never adjacent to a flag (or following a boolean flag) is simply
/// skipped — never pushed into `names`.
pub fn extract_flags(argv: &[String]) -> FlagCapture {
    let mut capture = FlagCapture::default();
    let mut i = 0;
    while i < argv.len() {
        let tok = argv[i].as_str();
        if looks_like_flag(tok) {
            let (name, inline_val) = match tok.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (tok.to_string(), None),
            };
            if !capture.names.contains(&name) {
                capture.names.push(name.clone());
            }
            let is_boolean = BOOLEAN_FLAGS.contains(&name.as_str());
            let value = match inline_val {
                Some(v) => Some(v),
                None if is_boolean => None,
                None => argv.get(i + 1).map(|next| {
                    i += 1; // consume the value token, whatever it looks like
                    next.clone()
                }),
            };
            if let Some(v) = value {
                let recorded = if VALUE_SAFE_FLAGS.contains(&name.as_str()) {
                    v
                } else {
                    "<redacted>".to_string()
                };
                capture.values.insert(name, recorded);
            }
        }
        i += 1;
    }
    capture
}

/// Fold global flags already stripped out of argv by the caller (e.g.
/// `accelbyte-ags-cli`'s `pre_scan_global_flags`) into a `FlagCapture`
/// already built from the remaining command-specific argv via
/// [`extract_flags`]. Applies the same [`VALUE_SAFE_FLAGS`] allowlist as
/// `extract_flags` itself, so a global flag's value is redacted under
/// exactly the same rule a command-specific flag's value would be. A name
/// already present in `capture.names` (e.g. `-n` appearing in both the
/// caller's stripped-flags list and, in some edge case, the remaining argv)
/// is not duplicated.
pub fn merge_global_flags(capture: &mut FlagCapture, pairs: &[(String, Option<String>)]) {
    for (name, value) in pairs {
        if !capture.names.contains(name) {
            capture.names.push(name.clone());
        }
        if let Some(v) = value {
            let recorded = if VALUE_SAFE_FLAGS.contains(&name.as_str()) {
                v.clone()
            } else {
                "<redacted>".to_string()
            };
            capture.values.insert(name.clone(), recorded);
        }
    }
}

/// Whether telemetry is switched on by the environment: an API key is present
/// and the universal `DO_NOT_TRACK` opt-out is not set. Kept cheap so the
/// disabled path never touches the credential store or network.
fn is_enabled_by_env() -> bool {
    if crate::runtime::config::is_env_var_set(ENV_DO_NOT_TRACK) {
        return false;
    }
    crate::runtime::config::is_env_var_set(ENV_POSTHOG_KEY)
}

/// `$lib` override: PostHog's own SDK auto-fills `$lib`; this stamps a
/// human-recognisable value for the CLI's hand-rolled `posthog-rs` events.
const LIB_OVERRIDE: &str = "ags-cli";

/// Everything gathered before dispatch; the outcome is attached at send time.
pub struct CommandTelemetry {
    /// The PostHog `distinct_id`: the authenticated `sub`, or — pre-login —
    /// this install's anonymous id (see [`ResolvedIdentity`]).
    pub distinct_id: String,
    /// Which [`ResolvedIdentity`] branch produced `distinct_id`:
    /// `"authenticated"` or `"anonymous"`.
    pub identity: &'static str,
    /// Which terminal UI surface rendered this invocation (e.g. `"plain"`,
    /// `"inline"`, `"fullscreen"`), matching `PhaseBackend::telemetry_label`'s
    /// vocabulary in the CLI crate.
    pub ui_surface: &'static str,
    pub email: Option<String>,
    pub namespace: Option<String>,
    /// The studio/publisher namespace decoded from the access token's
    /// `parent_namespace` claim, distinct from the game-scoped `namespace`
    /// above. `None` for an anonymous identity, or when the token was never
    /// scoped under a publisher namespace.
    pub studio: Option<String>,
    pub command_path: String,
    pub flags: FlagCapture,
    pub cli_version: String,
    pub auth_grant: Option<&'static str>,
    /// Captured at the top of `gather_context`, before any awaited work —
    /// `build_event` computes `started_at.elapsed()` as `duration_ms`, so
    /// this approximates total command wall-clock time (arg-parsing done →
    /// process about to exit), matching what `finish_self_owned`'s single
    /// `emit_with_outcome` call actually observes.
    pub started_at: std::time::Instant,
    /// Correlates this `cli.command.invoked` event with the `cli.workflow.step_*`
    /// events emitted by the same `ags workflow run` invocation, when this was
    /// one. `None` for every non-workflow-run command.
    pub workflow_run_id: Option<String>,
    /// The registered workflow's id (e.g. `competitive-multiplayer`), sent
    /// verbatim. `None` for every non-workflow-run command.
    pub workflow_id: Option<String>,
}

/// Command result, attached to the single end-of-command event.
pub struct Outcome {
    pub status: &'static str, // "completed" | "failed" | "cancelled"
    pub exit_code: i32,
    /// Coarse failure classification (see `CliError::telemetry_class`).
    /// `None` when `status` is not `"failed"`.
    pub error_class: Option<&'static str>,
    /// HTTP status from the failing upstream response, from
    /// `CliError::metadata`. `None` when `status` is not `"failed"` or the
    /// failure never reached an upstream response.
    pub http_status: Option<u16>,
    /// AccelByte error code (or a client-side `<domain>.<kind>` constant),
    /// from `CliError::metadata`. `None` when `status` is not `"failed"` or
    /// no code was recorded.
    pub error_code: Option<String>,
}

/// Build the single `command_flag` JSON object PostHog property from a
/// `FlagCapture`: one key per flag name present in the invocation (mirrors
/// the old `flag_names`), mapped to its value — the allowlisted real value,
/// `"<redacted>"`, or `null` for a valueless (boolean) flag (mirrors the old
/// `flag_values`). Collapsing into one object keeps every flag's name and
/// value paired together, so it reads directly alongside `command_path`
/// instead of needing two separately-indexed properties cross-referenced by
/// key.
fn command_flag_json(flags: &FlagCapture) -> serde_json::Value {
    let map: serde_json::Map<String, serde_json::Value> = flags
        .names
        .iter()
        .map(|name| {
            let value = flags
                .values
                .get(name)
                .cloned()
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null);
            (name.clone(), value)
        })
        .collect();
    serde_json::Value::Object(map)
}

/// Event name for the start of one workflow step.
const EVENT_WORKFLOW_STEP_STARTED: &str = "cli.workflow.step_started";

/// Event name for a step reaching any terminal outcome (success, failure,
/// cancellation, or skip). A step that emits `step_started` but never
/// `step_completed` is the drop-off signal the dashboard's friction panel
/// wants (see `ags-telemetry-metrics-dashboard-design.md` §5) — a crash or
/// hang, distinct from a step that ran and failed (which DOES get a
/// `step_completed` with `outcome: "failed"`).
const EVENT_WORKFLOW_STEP_COMPLETED: &str = "cli.workflow.step_completed";

/// Event name for the start of one registered workflow run.
const EVENT_WORKFLOW_RUN_STARTED: &str = "cli.workflow.run_started";

/// Event name for a workflow run reaching any terminal outcome — including a
/// `--no-input` rejection, which the executor rejects before it emits any
/// step event at all.
const EVENT_WORKFLOW_RUN_COMPLETED: &str = "cli.workflow.run_completed";

/// Fields shared by every `cli.workflow.step_*` event for one `ags workflow
/// run` invocation — everything except the per-step `step_index`/`step_id`.
pub struct WorkflowStepContext {
    /// Correlates every step event with each other and with the parent
    /// `cli.command.invoked` event's own `workflow_run_id` property.
    pub run_id: String,
    /// The registered workflow's id (e.g. `competitive-multiplayer`).
    pub workflow_id: String,
    /// Total step count for the run, for computing progress fractions.
    pub steps_total: usize,
    /// CLI binary version that produced this run.
    pub cli_version: String,
    /// Whether this run is a `--dry-run` preview. Without it a step funnel
    /// silently mixes previews with live API calls.
    pub is_dry_run: bool,
    /// Which interaction surface the user actually got.
    pub ui_surface: &'static str,
}

/// Pure builder for a `cli.workflow.step_started` event — split out from
/// [`capture_workflow_step_started`] so tests can assert on the built
/// event's properties directly, mirroring [`build_event`] /
/// `test_build_event_includes_duration_ms_elapsed_since_started_at`.
fn build_step_started_event(
    sub: &str,
    ctx: &WorkflowStepContext,
    step_index: usize,
    step_id: &str,
    service: &str,
    operation: &str,
) -> posthog_rs::Event {
    let mut event = posthog_rs::Event::new(EVENT_WORKFLOW_STEP_STARTED, sub);
    let _ = event.insert_prop("$lib", LIB_OVERRIDE);
    let _ = event.insert_prop("run_id", ctx.run_id.clone());
    let _ = event.insert_prop("workflow_id", ctx.workflow_id.clone());
    let _ = event.insert_prop("step_index", step_index as i64);
    let _ = event.insert_prop("step_id", step_id.to_string());
    let _ = event.insert_prop("steps_total", ctx.steps_total as i64);
    let _ = event.insert_prop("cli_version", ctx.cli_version.clone());
    let _ = event.insert_prop("is_dry_run", ctx.is_dry_run);
    let _ = event.insert_prop("service", service.to_string());
    let _ = event.insert_prop("operation", operation.to_string());
    event
}

/// Build and queue a `cli.workflow.step_started` event. Synchronous — safe to
/// call from `WorkflowFrontend::on_event`, which is not `async`.
pub fn capture_workflow_step_started(
    client: &TelemetryClient,
    sub: &str,
    ctx: &WorkflowStepContext,
    step_index: usize,
    step_id: &str,
    service: &str,
    operation: &str,
) {
    let event = build_step_started_event(sub, ctx, step_index, step_id, service, operation);
    tdbg!("queuing '{EVENT_WORKFLOW_STEP_STARTED}' step_id='{step_id}' index={step_index}");
    client.capture(event);
}

/// Everything a `cli.workflow.step_completed` event reports beyond the step's
/// identity. All fields are integers, bools, or closed vocabularies — never
/// free text (see the module's redaction boundary).
pub struct StepCompletedFacts {
    /// Terminal outcome label (`success` | `failed` | `cancelled` | `skipped`).
    pub outcome: &'static str,
    /// Why the step reached `outcome`, from `StepOutcomeReason::as_label()`;
    /// `None` for a plain success.
    pub reason: Option<&'static str>,
    /// Dispatch attempts; 1 for a step that was never retried. `0` means the
    /// step never dispatched at all (it ended at schema load, gather,
    /// assembly, review, or the confirm gate). A `--dry-run` step reports `1`
    /// without dispatching, so reading `attempts` as a count of API calls
    /// needs an `is_dry_run == false` filter.
    pub attempts: u32,
    /// Wall-clock time for this step, prompts included.
    pub duration_ms: u64,
    /// Coarse failure classification (`StepErrorFacts::class`); `None` unless
    /// the step failed.
    pub error_class: Option<&'static str>,
    /// HTTP status, when the failure came from an upstream response.
    pub http_status: Option<u16>,
    /// AccelByte error code, or a client-side `<domain>.<kind>` constant.
    pub error_code: Option<String>,
    /// The failed step's resolved input fields, redacted per the §4.1
    /// amendment (real values for a bundled workflow, `value: None` for
    /// every field of an external one). Empty for a successful step — the
    /// `input_fields` event property is omitted entirely in that case.
    pub input_fields: Vec<ags_protocol::workflow::StepInputField>,
    /// AGS service the step's operation belongs to. `String`, not
    /// `&'static str`: it comes from a runtime-loaded `CompiledWorkflow`.
    pub service: String,
    /// Operation id the step actually called. `String` for the same reason
    /// as `service`.
    pub operation: String,
}

/// Pure builder for a `cli.workflow.step_completed` event — split out from
/// [`capture_workflow_step_completed`] so tests can assert on the built
/// event's properties directly.
fn build_step_completed_event(
    sub: &str,
    ctx: &WorkflowStepContext,
    step_index: usize,
    step_id: &str,
    facts: &StepCompletedFacts,
) -> posthog_rs::Event {
    let mut event = posthog_rs::Event::new(EVENT_WORKFLOW_STEP_COMPLETED, sub);
    let _ = event.insert_prop("$lib", LIB_OVERRIDE);
    let _ = event.insert_prop("run_id", ctx.run_id.clone());
    let _ = event.insert_prop("workflow_id", ctx.workflow_id.clone());
    let _ = event.insert_prop("step_index", step_index as i64);
    let _ = event.insert_prop("step_id", step_id.to_string());
    let _ = event.insert_prop("steps_total", ctx.steps_total as i64);
    let _ = event.insert_prop("cli_version", ctx.cli_version.clone());
    let _ = event.insert_prop("outcome", facts.outcome);
    if let Some(reason) = facts.reason {
        let _ = event.insert_prop("outcome_reason", reason);
    }
    let _ = event.insert_prop("attempts", facts.attempts);
    let _ = event.insert_prop("duration_ms", facts.duration_ms);
    if let Some(class) = facts.error_class {
        let _ = event.insert_prop("error_class", class);
    }
    if let Some(status) = facts.http_status {
        let _ = event.insert_prop("http_status", status);
    }
    if let Some(code) = &facts.error_code {
        let _ = event.insert_prop("error_code", code.clone());
    }
    if !facts.input_fields.is_empty() {
        let _ = event.insert_prop(
            "input_fields",
            step_input_fields_to_json(&facts.input_fields),
        );
    }
    let _ = event.insert_prop("is_dry_run", ctx.is_dry_run);
    let _ = event.insert_prop("service", facts.service.clone());
    let _ = event.insert_prop("operation", facts.operation.clone());
    event
}

/// Project `input_fields` to a JSON array for the `input_fields` event
/// property. `StepInputField` is deliberately not `Serialize` (it lives
/// alongside `StepErrorFacts`, which must stay unserialized so `source` can
/// be a `&'static str`), so this builds the wire shape by hand. `location`
/// is the one sub-value that already implements `Serialize`.
fn step_input_fields_to_json(
    fields: &[ags_protocol::workflow::StepInputField],
) -> serde_json::Value {
    let items = fields
        .iter()
        .map(|f| {
            serde_json::json!({
                "field": f.field,
                "location": serde_json::to_value(f.location).unwrap_or(serde_json::Value::Null),
                "source": f.source,
                "required": f.required,
                "value": f.value.clone().unwrap_or(serde_json::Value::Null),
            })
        })
        .collect();
    serde_json::Value::Array(items)
}

/// Build and queue a `cli.workflow.step_completed` event.
pub fn capture_workflow_step_completed(
    client: &TelemetryClient,
    sub: &str,
    ctx: &WorkflowStepContext,
    step_index: usize,
    step_id: &str,
    facts: &StepCompletedFacts,
) {
    let event = build_step_completed_event(sub, ctx, step_index, step_id, facts);
    tdbg!(
        "queuing '{EVENT_WORKFLOW_STEP_COMPLETED}' step_id='{step_id}' index={step_index} outcome={}",
        facts.outcome
    );
    client.capture(event);
}

/// Pure builder for a `cli.workflow.run_started` event — split out from
/// [`capture_workflow_run_started`] so tests can assert on the built event's
/// properties directly, mirroring [`build_step_started_event`].
fn build_run_started_event(
    sub: &str,
    ctx: &WorkflowStepContext,
    assume_yes: bool,
    no_input: bool,
) -> posthog_rs::Event {
    let mut event = posthog_rs::Event::new(EVENT_WORKFLOW_RUN_STARTED, sub);
    let _ = event.insert_prop("$lib", LIB_OVERRIDE);
    let _ = event.insert_prop("run_id", ctx.run_id.clone());
    let _ = event.insert_prop("workflow_id", ctx.workflow_id.clone());
    let _ = event.insert_prop("steps_total", ctx.steps_total as i64);
    let _ = event.insert_prop("cli_version", ctx.cli_version.clone());
    let _ = event.insert_prop("is_dry_run", ctx.is_dry_run);
    let _ = event.insert_prop("ui_surface", ctx.ui_surface);
    let _ = event.insert_prop("assume_yes", assume_yes);
    let _ = event.insert_prop("no_input", no_input);
    event
}

/// Build and queue a `cli.workflow.run_started` event. Synchronous — safe to
/// call from `WorkflowFrontend::on_event`, which is not `async`.
pub fn capture_workflow_run_started(
    client: &TelemetryClient,
    sub: &str,
    ctx: &WorkflowStepContext,
    assume_yes: bool,
    no_input: bool,
) {
    let event = build_run_started_event(sub, ctx, assume_yes, no_input);
    tdbg!(
        "queuing '{EVENT_WORKFLOW_RUN_STARTED}' run_id='{}'",
        ctx.run_id
    );
    client.capture(event);
}

/// Everything a `cli.workflow.run_completed` event reports beyond the run's
/// shared context. All fields are integers, bools, or closed vocabularies —
/// never free text (see the module's redaction boundary).
pub struct RunCompletedFacts {
    /// Terminal outcome label for the whole run: `completed` | `failed` |
    /// `cancelled`. Note the asymmetry with step events, which report a
    /// success as `success` — that spelling is kept for back-compat with
    /// dashboards built before the run events existed, so a query must not
    /// assume one vocabulary across both.
    pub outcome: &'static str,
    /// Why the run reached `outcome`, emitted as `outcome_reason`; `None` for a plain success.
    pub reason: Option<&'static str>,
    /// Wall-clock time for the entire run.
    pub duration_ms: u64,
    /// Which confirmation/review mode the run executed under, when applicable.
    pub run_mode: Option<&'static str>,
    /// Count of steps that were started.
    pub steps_started: usize,
    /// Count of steps that succeeded.
    pub steps_succeeded: usize,
    /// Count of steps that failed.
    pub steps_failed: usize,
    /// Count of steps that were skipped.
    pub steps_skipped: usize,
    /// Count of steps that were cancelled.
    pub steps_cancelled: usize,
    /// Index of the last step reached before the run ended, when applicable.
    pub last_step_index: Option<usize>,
    /// Coarse failure classification; `None` unless the run failed.
    pub error_class: Option<&'static str>,
    /// HTTP status, when the failure came from an upstream response.
    pub http_status: Option<u16>,
    /// AccelByte error code, or a client-side `<domain>.<kind>` constant.
    pub error_code: Option<String>,
    /// Count of inputs supplied via a CLI flag.
    pub inputs_from_flag: usize,
    /// Count of inputs supplied via an interactive prompt.
    pub inputs_from_prompt: usize,
    /// Count of inputs left at their default value.
    pub inputs_from_default: usize,
    /// Count of inputs edited in the structured JSON/form editor. An input
    /// that went from *unset* to set counts as edited too — the comparison is
    /// against the pre-gather value, and "absent" is one of the values it can
    /// differ from.
    pub inputs_edited_in_form: usize,
}

/// Pure builder for a `cli.workflow.run_completed` event — split out from
/// [`capture_workflow_run_completed`] so tests can assert on the built
/// event's properties directly, mirroring [`build_step_completed_event`].
fn build_run_completed_event(
    sub: &str,
    ctx: &WorkflowStepContext,
    facts: &RunCompletedFacts,
    assume_yes: bool,
    no_input: bool,
) -> posthog_rs::Event {
    let mut event = posthog_rs::Event::new(EVENT_WORKFLOW_RUN_COMPLETED, sub);
    let _ = event.insert_prop("$lib", LIB_OVERRIDE);
    let _ = event.insert_prop("run_id", ctx.run_id.clone());
    let _ = event.insert_prop("workflow_id", ctx.workflow_id.clone());
    let _ = event.insert_prop("steps_total", ctx.steps_total as i64);
    let _ = event.insert_prop("cli_version", ctx.cli_version.clone());
    let _ = event.insert_prop("is_dry_run", ctx.is_dry_run);
    let _ = event.insert_prop("ui_surface", ctx.ui_surface);
    // Spec §3.2: `run_completed` carries everything `run_started` does. Both
    // matter here in their own right — `no_input` is the segmentation for the
    // `no_input` `outcome_reason` this same event emits, and `assume_yes`
    // explains away every confirm/review stage that never fired.
    let _ = event.insert_prop("assume_yes", assume_yes);
    let _ = event.insert_prop("no_input", no_input);
    let _ = event.insert_prop("outcome", facts.outcome);
    if let Some(reason) = facts.reason {
        let _ = event.insert_prop("outcome_reason", reason);
    }
    let _ = event.insert_prop("duration_ms", facts.duration_ms);
    if let Some(run_mode) = facts.run_mode {
        let _ = event.insert_prop("run_mode", run_mode);
    }
    let _ = event.insert_prop("steps_started", facts.steps_started as i64);
    let _ = event.insert_prop("steps_succeeded", facts.steps_succeeded as i64);
    let _ = event.insert_prop("steps_failed", facts.steps_failed as i64);
    let _ = event.insert_prop("steps_skipped", facts.steps_skipped as i64);
    let _ = event.insert_prop("steps_cancelled", facts.steps_cancelled as i64);
    if let Some(last_step_index) = facts.last_step_index {
        let _ = event.insert_prop("last_step_index", last_step_index as i64);
    }
    if let Some(class) = facts.error_class {
        let _ = event.insert_prop("error_class", class);
    }
    if let Some(status) = facts.http_status {
        let _ = event.insert_prop("http_status", status);
    }
    if let Some(code) = &facts.error_code {
        let _ = event.insert_prop("error_code", code.clone());
    }
    let _ = event.insert_prop("inputs_from_flag", facts.inputs_from_flag as i64);
    let _ = event.insert_prop("inputs_from_prompt", facts.inputs_from_prompt as i64);
    let _ = event.insert_prop("inputs_from_default", facts.inputs_from_default as i64);
    let _ = event.insert_prop("inputs_edited_in_form", facts.inputs_edited_in_form as i64);
    event
}

/// Build and queue a `cli.workflow.run_completed` event.
pub fn capture_workflow_run_completed(
    client: &TelemetryClient,
    sub: &str,
    ctx: &WorkflowStepContext,
    facts: &RunCompletedFacts,
    assume_yes: bool,
    no_input: bool,
) {
    let event = build_run_completed_event(sub, ctx, facts, assume_yes, no_input);
    tdbg!(
        "queuing '{EVENT_WORKFLOW_RUN_COMPLETED}' run_id='{}' outcome={}",
        ctx.run_id,
        facts.outcome
    );
    client.capture(event);
}

/// Build the `cli.command.invoked` event from gathered context and the
/// command's outcome. Pure — does no I/O and never fails; property
/// serialization errors from `insert_prop` are swallowed (`let _ =`) exactly
/// as the existing capture path already does.
///
/// `email`, when present, is attached as a `$set` **person** property (never
/// a top-level event property) so PostHog updates the person profile rather
/// than treating it as per-event data. `namespace`, when present, is attached
/// only via `add_group` — the group key is the source of truth, so no
/// redundant top-level `namespace` property is written.
fn build_event(ctx: &CommandTelemetry, outcome: &Outcome) -> posthog_rs::Event {
    let mut event = posthog_rs::Event::new(EVENT_COMMAND_INVOKED, &ctx.distinct_id);
    // Email as a PERSON property, not an event property.
    if let Some(email) = &ctx.email {
        let _ = event.insert_prop("$set", serde_json::json!({ "email": email }));
    }
    let _ = event.insert_prop("$lib", LIB_OVERRIDE);
    let _ = event.insert_prop("identity", ctx.identity);
    let _ = event.insert_prop("ui_surface", ctx.ui_surface);
    let _ = event.insert_prop("command_path", ctx.command_path.clone());
    let _ = event.insert_prop("command_flag", command_flag_json(&ctx.flags));
    let _ = event.insert_prop("cli_version", ctx.cli_version.clone());
    let _ = event.insert_prop(
        "duration_ms",
        i64::try_from(ctx.started_at.elapsed().as_millis()).unwrap_or(i64::MAX),
    );
    let _ = event.insert_prop("outcome", outcome.status);
    let _ = event.insert_prop("exit_code", outcome.exit_code);
    if let Some(class) = outcome.error_class {
        let _ = event.insert_prop("error_class", class);
    }
    if let Some(run_id) = &ctx.workflow_run_id {
        let _ = event.insert_prop("workflow_run_id", run_id.clone());
    }
    if let Some(workflow_id) = &ctx.workflow_id {
        let _ = event.insert_prop("workflow_id", workflow_id.clone());
    }
    if let Some(status) = outcome.http_status {
        let _ = event.insert_prop("http_status", status);
    }
    if let Some(code) = &outcome.error_code {
        let _ = event.insert_prop("error_code", code.clone());
    }
    if let Some(grant) = ctx.auth_grant {
        let _ = event.insert_prop("auth_grant", grant);
    }
    if let Some(namespace) = &ctx.namespace {
        event.add_group(NAMESPACE_GROUP, namespace);
    }
    if let Some(studio) = &ctx.studio {
        event.add_group(STUDIO_GROUP, studio);
    }
    event
}

/// Thin wrapper over the `posthog-rs` client. When telemetry is disabled the
/// inner client is `None` and every method is a cheap no-op.
pub struct TelemetryClient {
    inner: Option<posthog_rs::Client>,
}

impl TelemetryClient {
    /// A disabled client usable from another crate's tests (e.g. the CLI
    /// crate's `ExecutionFrontendAdapter` tests) without going through the
    /// environment. Identical to what `from_env()` returns when telemetry is
    /// off.
    pub fn disabled_for_test() -> Self {
        Self { inner: None }
    }

    /// Build a client from the environment, or a disabled no-op client when the
    /// key is unset/blank or `DO_NOT_TRACK` is set. Configures the ingestion host
    /// ([`DEFAULT_POSTHOG_HOST`], US by default) so events land in the same
    /// PostHog project the Admin Portal identifies into.
    pub async fn from_env() -> Self {
        if !is_enabled_by_env() {
            return Self { inner: None };
        }
        let Ok(api_key) = std::env::var(ENV_POSTHOG_KEY) else {
            return Self { inner: None };
        };
        let host = std::env::var(ENV_POSTHOG_HOST)
            .ok()
            .filter(|host| !host.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_POSTHOG_HOST.to_string());
        tdbg!(
            "building client: host='{host}', api_key length={}",
            api_key.len()
        );

        // `is_server(false)` suppresses the `$is_server` stamp so the CLI's
        // host OS isn't attributed to the user — but that also makes PostHog
        // geolocate the event by IP, so `disable_geoip(true)` turns that back off.
        let Ok(options) = ClientOptionsBuilder::default()
            .api_key(api_key)
            .host(host)
            .is_server(false)
            .disable_geoip(true)
            .build()
        else {
            tdbg!("failed to build posthog client options");
            return Self { inner: None };
        };

        Self {
            inner: Some(posthog_rs::client(options).await),
        }
    }

    /// Queue `event` for delivery. No-op when disabled.
    pub fn capture(&self, event: Event) {
        let Some(client) = &self.inner else {
            tdbg!("telemetry client is disabled; not capturing");
            return;
        };
        // Callers (`capture_workflow_step_started`, `capture_workflow_step_completed`,
        // `emit_with_outcome`, …) each log their own specific `tdbg!` before
        // calling this shared method, so this message stays generic rather
        // than naming a specific event (this method is shared by more than
        // just `EVENT_COMMAND_INVOKED`).
        tdbg!("queuing event");
        client.capture(event);
    }

    /// Flush any queued event(s). No-op when disabled; every failure is
    /// swallowed so telemetry never affects the command.
    pub async fn flush(&self) {
        let Some(client) = &self.inner else {
            return;
        };
        tdbg!("flushing…");
        client.flush().await;
        tdbg!("flush returned (posthog-rs does not surface delivery errors)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::test_helpers::TempEnvGuard;

    /// Profile name used by the anonymous-fallback tests below.
    ///
    /// `ENV_HOME` redirects the config directory, but it cannot isolate the
    /// OS keyring: credentials are looked up by `keyring_service(profile)`
    /// (`"ags:accelbyte.io:{profile}"`), which is independent of `$HOME`. If
    /// these tests used the default profile, a developer machine (or CI
    /// runner) with a real stored token for `"default"` would make the
    /// lookup succeed, resolve an authenticated identity, and silently
    /// falsify the "no stored token" precondition the tests exist to check.
    /// Using a profile name no real login flow would ever create keeps the
    /// keyring lookup a genuine miss.
    const ANON_FALLBACK_TEST_PROFILE: &str = "ags-telemetry-anon-test";

    /// Encode a claims object as a base64url (no-pad) JWT payload segment.
    fn payload_segment(claims: &serde_json::Value) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap())
    }

    #[test]
    fn test_decode_sub_reads_sub_from_three_segment_token() {
        let payload = payload_segment(&serde_json::json!({ "sub": "user-123" }));
        let token = format!("header.{payload}.sig");
        assert_eq!(decode_sub(&token), Some("user-123".to_string()));
    }

    #[test]
    fn test_decode_sub_rejects_two_segment_token() {
        let payload = payload_segment(&serde_json::json!({ "sub": "user-123" }));
        let token = format!("header.{payload}");
        assert_eq!(decode_sub(&token), None);
    }

    #[test]
    fn test_decode_sub_rejects_non_jwt_string() {
        assert_eq!(decode_sub("not-a-jwt"), None);
    }

    #[test]
    fn test_decode_sub_returns_none_when_claims_lack_sub() {
        let payload = payload_segment(&serde_json::json!({ "client_id": "abc" }));
        let token = format!("header.{payload}.sig");
        assert_eq!(decode_sub(&token), None);
    }

    #[test]
    fn test_decode_parent_namespace_reads_parent_namespace_from_three_segment_token() {
        let payload = payload_segment(&serde_json::json!({
            "sub": "user-123",
            "namespace": "mygame",
            "parent_namespace": "mystudio"
        }));
        let token = format!("header.{payload}.sig");
        assert_eq!(
            decode_parent_namespace(&token),
            Some("mystudio".to_string())
        );
    }

    #[test]
    fn test_decode_parent_namespace_returns_none_when_claims_lack_it() {
        let payload = payload_segment(&serde_json::json!({ "sub": "user-123" }));
        let token = format!("header.{payload}.sig");
        assert_eq!(decode_parent_namespace(&token), None);
    }

    #[test]
    fn test_decode_parent_namespace_rejects_non_jwt_string() {
        assert_eq!(decode_parent_namespace("not-a-jwt"), None);
    }

    /// Build a `String` argument vector from string slices.
    fn args(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|token| token.to_string()).collect()
    }

    #[test]
    fn test_command_path_names_joins_leading_command_names() {
        assert_eq!(
            command_path_names(&args(&["iam", "users", "list"])),
            "iam users list"
        );
    }

    #[test]
    fn test_command_path_names_stops_at_first_flag_dropping_values() {
        assert_eq!(
            command_path_names(&args(&["iam", "users", "get", "--user-id", "secret-abc"])),
            "iam users get"
        );
    }

    #[test]
    fn test_command_path_names_empty_for_leading_flag() {
        assert_eq!(command_path_names(&args(&["--help"])), "");
    }

    #[test]
    fn test_command_path_names_excludes_workflow_run_positional_value() {
        assert_eq!(
            command_path_names(&args(&["workflow", "run", "/home/alice/wf.yaml"])),
            "workflow run"
        );
    }

    #[test]
    fn test_command_path_names_excludes_workflow_add_path_value() {
        assert_eq!(
            command_path_names(&args(&["workflow", "add", "/home/alice/wf.yaml"])),
            "workflow add"
        );
    }

    /// The key is kept (small fixed vocabulary); the value is not.
    #[test]
    fn test_command_path_names_excludes_config_set_value_but_keeps_key() {
        assert_eq!(
            command_path_names(&args(&[
                "config",
                "set",
                "base-url",
                "https://internal.example.com"
            ])),
            "config set base-url"
        );
    }

    #[test]
    fn test_command_path_names_excludes_profile_create_name_value() {
        assert_eq!(
            command_path_names(&args(&["profile", "create", "alice-personal"])),
            "profile create"
        );
    }

    /// Service routes have no positional values, so all 3 tokens pass through.
    #[test]
    fn test_command_path_names_service_route_keeps_all_three_routing_tokens() {
        assert_eq!(
            command_path_names(&args(&["iam", "users", "get"])),
            "iam users get"
        );
    }

    #[test]
    fn test_extract_flags_collects_all_names_in_order() {
        let fc = extract_flags(&args(&[
            "iam",
            "users",
            "get",
            "--namespace",
            "ns1",
            "--verbose",
        ]));
        assert_eq!(
            fc.names,
            vec!["--namespace".to_string(), "--verbose".to_string()]
        );
    }

    #[test]
    fn test_extract_flags_redacts_non_allowlisted_values() {
        let fc = extract_flags(&args(&["login", "--client-secret", "shhh"]));
        assert_eq!(fc.names, vec!["--client-secret".to_string()]);
        assert_eq!(
            fc.values.get("--client-secret"),
            Some(&"<redacted>".to_string())
        );
    }

    #[test]
    fn test_extract_flags_keeps_allowlisted_value() {
        let fc = extract_flags(&args(&["-n", "ammarabtestsa55"]));
        assert_eq!(fc.values.get("-n"), Some(&"ammarabtestsa55".to_string()));
    }

    #[test]
    fn test_extract_flags_handles_equals_form_and_redacts() {
        let fc = extract_flags(&args(&["run", "--token=abc123"]));
        assert_eq!(fc.names, vec!["--token".to_string()]);
        assert_eq!(fc.values.get("--token"), Some(&"<redacted>".to_string()));
    }

    #[test]
    fn test_extract_flags_boolean_flag_has_no_value() {
        let fc = extract_flags(&args(&["build", "--dry-run"]));
        assert_eq!(fc.names, vec!["--dry-run".to_string()]);
        assert!(fc.values.is_empty());
    }

    /// Regression for the leak this finding closes: a dash-leading secret
    /// passed as a flag's value must never surface verbatim in `names` (as
    /// if it were re-parsed as its own flag) nor in `values` — it must be
    /// redacted.
    #[test]
    fn test_extract_flags_redacts_dash_leading_value_and_never_leaks_it() {
        let secret = "-XyZsecretBase64url9f8";
        let fc = extract_flags(&args(&["login", "--client-secret", secret]));
        assert_eq!(fc.names, vec!["--client-secret".to_string()]);
        assert!(
            !fc.names.contains(&secret.to_string()),
            "secret must not appear in flag_names: {:?}",
            fc.names
        );
        for value in fc.values.values() {
            assert_ne!(
                value, secret,
                "secret must not appear verbatim in flag_values"
            );
        }
        assert_eq!(
            fc.values.get("--client-secret"),
            Some(&"<redacted>".to_string())
        );
    }

    /// Same leak, equals-form: `--token=-dashvalue`. The dash-leading value
    /// is inline (not a separate token) but must still be redacted, not kept.
    #[test]
    fn test_extract_flags_redacts_equals_form_dash_leading_value() {
        let fc = extract_flags(&args(&["run", "--token=-dashvalue"]));
        assert_eq!(fc.names, vec!["--token".to_string()]);
        assert!(!fc.names.iter().any(|n| n.contains("dashvalue")));
        assert_eq!(fc.values.get("--token"), Some(&"<redacted>".to_string()));
    }

    /// Allowlisted flags still keep their value even though the parsing rule
    /// changed — `-n` is a recognised single-char short flag, so its
    /// non-flag-looking value is captured normally.
    #[test]
    fn test_extract_flags_allowlisted_still_keeps_value_after_leak_fix() {
        let fc = extract_flags(&args(&["-n", "ammarabtestsa55"]));
        assert_eq!(fc.values.get("-n"), Some(&"ammarabtestsa55".to_string()));
    }

    /// Regression for a re-review-caught gap in the first leak fix: a
    /// double-dash-leading secret whose first character (right after `--`)
    /// happens to be a letter (e.g. mixed-case base64url) must not be
    /// misread as its own flag name — `looks_like_flag_name` requires the
    /// *whole* name to be lowercase-kebab, so an uppercase letter or digit
    /// anywhere in it disqualifies it as a flag.
    #[test]
    fn test_extract_flags_redacts_double_dash_leading_value_and_never_leaks_it() {
        let secret = "--abcSECRETvalue123";
        let fc = extract_flags(&args(&["login", "--client-secret", secret]));
        assert_eq!(fc.names, vec!["--client-secret".to_string()]);
        assert!(
            !fc.names.contains(&secret.to_string()),
            "secret must not appear in flag_names: {:?}",
            fc.names
        );
        assert_eq!(
            fc.values.get("--client-secret"),
            Some(&"<redacted>".to_string())
        );
    }

    /// Regression for the residual gap in the first two leak fixes: a
    /// purely lowercase-kebab secret (e.g. a word-based passphrase) passed
    /// as a flag's value is itself shaped exactly like a real flag name, so
    /// the old shape-based lookahead misclassified it as "not a value" and
    /// let it fall through to be re-parsed as its own flag on the next
    /// iteration — leaked verbatim into `names`. `--client-secret` is not in
    /// `BOOLEAN_FLAGS`, so it must now consume the next token unconditionally.
    #[test]
    fn test_extract_flags_redacts_lowercase_kebab_value_and_never_leaks_it() {
        let secret = "--my-secret-passphrase";
        let fc = extract_flags(&args(&["login", "--client-secret", secret]));
        assert_eq!(fc.names, vec!["--client-secret".to_string()]);
        assert!(
            !fc.names.contains(&secret.to_string()),
            "secret must not appear in flag_names: {:?}",
            fc.names
        );
        for value in fc.values.values() {
            assert_ne!(
                value, secret,
                "secret must not appear verbatim in flag_values"
            );
        }
        assert_eq!(
            fc.values.get("--client-secret"),
            Some(&"<redacted>".to_string())
        );
    }

    /// A boolean flag placed immediately before another real flag must not
    /// swallow that flag as its own "value" — it takes no value at all, so
    /// the following token is left for the next loop iteration to parse
    /// as its own flag, exactly as before this fix.
    #[test]
    fn test_extract_flags_boolean_flag_does_not_consume_following_flag() {
        let fc = extract_flags(&args(&["build", "--dry-run", "--namespace", "ns1"]));
        assert_eq!(
            fc.names,
            vec!["--dry-run".to_string(), "--namespace".to_string()]
        );
        assert!(!fc.values.contains_key("--dry-run"));
        assert_eq!(fc.values.get("--namespace"), Some(&"ns1".to_string()));
    }

    /// `--symbol-files` is a boolean flag (SetTrue) added by the ams-upload
    /// route. Without its entry in BOOLEAN_FLAGS, extract_flags treats it as
    /// value-taking, consumes `--format` as its redacted value, and `--format`
    /// never appears in the flag list at all — a data-quality loss.
    #[test]
    fn test_extract_flags_treats_symbol_files_as_boolean() {
        let fc = extract_flags(&args(&[
            "ams",
            "upload",
            "--symbol-files",
            "--format",
            "json",
        ]));
        assert!(
            fc.names.contains(&"--symbol-files".to_string()),
            "flag names must include --symbol-files: {:?}",
            fc.names
        );
        assert!(
            fc.names.contains(&"--format".to_string()),
            "--format must not be swallowed as --symbol-files's value: {:?}",
            fc.names
        );
        assert!(
            !fc.values.contains_key("--symbol-files"),
            "--symbol-files is boolean and must carry no value"
        );
        assert_eq!(
            fc.values.get("--format"),
            Some(&"json".to_string()),
            "--format is value-safe and its value must be captured"
        );
    }

    /// `--skip-script-validation` is a boolean flag (SetTrue) added by the
    /// ams-upload route. Same swallowing risk as `--symbol-files` above.
    #[test]
    fn test_extract_flags_treats_skip_script_validation_as_boolean() {
        let fc = extract_flags(&args(&[
            "ams",
            "upload",
            "--skip-script-validation",
            "--dry-run",
        ]));
        assert!(
            fc.names.contains(&"--skip-script-validation".to_string()),
            "flag names must include --skip-script-validation: {:?}",
            fc.names
        );
        assert!(
            fc.names.contains(&"--dry-run".to_string()),
            "--dry-run must not be swallowed as --skip-script-validation's value: {:?}",
            fc.names
        );
        assert!(
            !fc.values.contains_key("--skip-script-validation"),
            "--skip-script-validation is boolean and must carry no value"
        );
    }

    #[test]
    fn test_classify_auth_grant_authorization_code() {
        use ags_protocol::request::GrantType;
        assert_eq!(
            classify_auth_grant(Some(GrantType::AuthorizationCode)),
            Some("authorization_code")
        );
    }

    #[test]
    fn test_classify_auth_grant_client_credentials() {
        use ags_protocol::request::GrantType;
        assert_eq!(
            classify_auth_grant(Some(GrantType::ClientCredentials)),
            Some("client_credentials")
        );
    }

    #[test]
    fn test_classify_auth_grant_none_when_absent() {
        assert_eq!(classify_auth_grant(None), None);
    }

    #[test]
    fn test_merge_global_flags_adds_names_and_applies_value_safe_allowlist() {
        let mut capture = extract_flags(&args(&["iam", "users", "get", "--user-id", "abc123"]));
        let global_pairs = vec![
            ("--namespace".to_string(), Some("ns1".to_string())),
            (
                "--profile".to_string(),
                Some("secret-profile-name".to_string()),
            ),
            ("--no-color".to_string(), None),
        ];

        merge_global_flags(&mut capture, &global_pairs);

        assert_eq!(
            capture.names,
            vec![
                "--user-id".to_string(),
                "--namespace".to_string(),
                "--profile".to_string(),
                "--no-color".to_string(),
            ]
        );
        // --namespace is value-safe: real value kept.
        assert_eq!(capture.values.get("--namespace"), Some(&"ns1".to_string()));
        // --profile is NOT value-safe: redacted even though the pair carried a real value.
        assert_eq!(
            capture.values.get("--profile"),
            Some(&"<redacted>".to_string())
        );
        // --no-color has no value: nothing inserted into `values` for it.
        assert!(!capture.values.contains_key("--no-color"));
    }

    #[test]
    fn test_merge_global_flags_does_not_duplicate_a_name_already_present() {
        let mut capture = extract_flags(&args(&["iam", "users", "get", "-n", "ns1"]));
        let global_pairs = vec![("-n".to_string(), Some("ns1".to_string()))];

        merge_global_flags(&mut capture, &global_pairs);

        assert_eq!(
            capture.names.iter().filter(|n| n.as_str() == "-n").count(),
            1
        );
    }

    #[test]
    fn test_extract_flags_keeps_user_id_and_client_id_values() {
        let fc = extract_flags(&args(&["iam", "users", "get", "--user-id", "abc123"]));
        assert_eq!(fc.values.get("--user-id"), Some(&"abc123".to_string()));

        let fc = extract_flags(&args(&["iam", "clients", "get", "--client-id", "xyz789"]));
        assert_eq!(fc.values.get("--client-id"), Some(&"xyz789".to_string()));
    }

    /// `--output`'s value is a filesystem path that typically embeds the OS
    /// username (e.g. `/home/alice/report.json`) — the same PII class
    /// already rejected for `hostname`. It must be redacted like any other
    /// non-allowlisted flag, even though the flag name itself is recorded.
    #[test]
    fn test_extract_flags_redacts_output_value() {
        let fc = extract_flags(&args(&[
            "iam",
            "users",
            "list",
            "--output",
            "/home/alice/out.json",
        ]));
        assert_eq!(fc.names, vec!["--output".to_string()]);
        assert_eq!(fc.values.get("--output"), Some(&"<redacted>".to_string()));
    }

    /// Same redaction rule applies when `--output` arrives via
    /// `merge_global_flags` (its real path through `pre_scan_global_flags` +
    /// `GlobalFlags::telemetry_pairs`), not just via raw-argv `extract_flags`.
    #[test]
    fn test_merge_global_flags_redacts_output_value() {
        let mut capture = extract_flags(&args(&["iam", "users", "list"]));
        let global_pairs = vec![(
            "--output".to_string(),
            Some("/home/alice/out.json".to_string()),
        )];

        merge_global_flags(&mut capture, &global_pairs);

        assert_eq!(
            capture.values.get("--output"),
            Some(&"<redacted>".to_string())
        );
    }

    #[test]
    fn test_command_flag_json_pairs_each_name_with_its_value_or_null() {
        let mut capture = extract_flags(&args(&[
            "iam",
            "users",
            "get",
            "--user-id",
            "abc123",
            "--dry-run",
        ]));
        merge_global_flags(
            &mut capture,
            &[("--client-secret".to_string(), Some("shhh".to_string()))],
        );

        assert_eq!(
            command_flag_json(&capture),
            serde_json::json!({
                "--user-id": "abc123",
                "--dry-run": null,
                "--client-secret": "<redacted>",
            })
        );
    }

    /// Regression for the most likely silent-regression shape called out in
    /// review: a typo'd/inverted env-var check in `is_enabled_by_env` (or in
    /// `gather_context`'s early-return on it) would make telemetry silently
    /// stop firing with no test catching it, since fire-and-forget swallows
    /// all failures by design.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_gather_context_returns_none_when_posthog_key_is_unset() {
        let _key = TempEnvGuard::remove(ENV_POSTHOG_KEY);
        let _dnt = TempEnvGuard::remove(ENV_DO_NOT_TRACK);

        let ctx = gather_context(
            None,
            None,
            &[],
            &args(&["iam", "users", "list"]),
            "1.0.0",
            None,
            "plain",
            None,
        )
        .await;

        assert!(ctx.is_none());
    }

    /// `DO_NOT_TRACK` must win even when a PostHog key is present — the
    /// universal opt-out takes priority over the feature being configured.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_gather_context_returns_none_when_do_not_track_overrides_posthog_key() {
        let _key = TempEnvGuard::set(ENV_POSTHOG_KEY, "test-key");
        let _dnt = TempEnvGuard::set(ENV_DO_NOT_TRACK, "1");

        let ctx = gather_context(
            None,
            None,
            &[],
            &args(&["iam", "users", "list"]),
            "1.0.0",
            None,
            "plain",
            None,
        )
        .await;

        assert!(ctx.is_none());
    }

    /// A meta/builtin invocation (`command_path` starts with a flag, so
    /// `command_path_names` yields "") must be skipped before ever touching
    /// the profile/token store, resolving an identity, or firing
    /// `fetch_email` — not just at `build_event` time.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_gather_context_returns_none_when_command_path_is_empty() {
        let _key = TempEnvGuard::set(ENV_POSTHOG_KEY, "test-key");
        let _dnt = TempEnvGuard::remove(ENV_DO_NOT_TRACK);

        let ctx = gather_context(
            None,
            None,
            &[],
            &args(&["--version"]),
            "1.0.0",
            None,
            "plain",
            None,
        )
        .await;

        assert!(ctx.is_none());
    }

    /// The central regression this task closes: with no stored access token
    /// (the state of every invocation before a first successful `ags auth
    /// login`), `gather_context` must still produce a context — attributed to
    /// this install's anonymous id — instead of silently emitting nothing.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_gather_context_falls_back_to_anonymous_without_a_stored_token() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _key = TempEnvGuard::set(ENV_POSTHOG_KEY, "phc_test");
        let _dnt = TempEnvGuard::remove(ENV_DO_NOT_TRACK);

        let ctx = gather_context(
            Some(ANON_FALLBACK_TEST_PROFILE),
            None,
            &[],
            &args(&["config", "set"]),
            "1.2.3",
            None,
            "plain",
            None,
        )
        .await
        .expect("an unauthenticated invocation must still produce a context");

        assert!(ctx.distinct_id.starts_with("anon-"));
        assert_eq!(ctx.identity, "anonymous");
        assert_eq!(ctx.ui_surface, "plain");
        assert_eq!(
            ctx.email, None,
            "an anonymous event must never carry an email"
        );
        assert_eq!(
            ctx.auth_grant, None,
            "an anonymous event must never carry an auth_grant"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_telemetry_client_from_env_is_disabled_when_key_is_unset() {
        let _key = TempEnvGuard::remove(ENV_POSTHOG_KEY);
        let _dnt = TempEnvGuard::remove(ENV_DO_NOT_TRACK);

        let client = TelemetryClient::from_env().await;

        assert!(client.inner.is_none());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_telemetry_client_from_env_is_disabled_when_do_not_track_is_set() {
        let _key = TempEnvGuard::set(ENV_POSTHOG_KEY, "test-key");
        let _dnt = TempEnvGuard::set(ENV_DO_NOT_TRACK, "1");

        let client = TelemetryClient::from_env().await;

        assert!(client.inner.is_none());
    }

    /// `DO_NOT_TRACK` must disable every enablement entry point
    /// (`gather_context`, `resolve_identity`, `load_or_create_install_identity`,
    /// `TelemetryClient::from_env`) identically in the same run — and the two
    /// workflow-run capture entry points must be no-ops against that same
    /// disabled client (never bypassing it to build their own), so a silent
    /// regression in any one of them can't slip past this opt-out coverage.
    /// `load_or_create_install_identity`'s inclusion matters most: were it to
    /// leak past this opt-out, it would write a machine-identifying file to
    /// disk for a user who explicitly asked not to be tracked.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_do_not_track_disables_every_entry_point_identically() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _key = TempEnvGuard::set(ENV_POSTHOG_KEY, "test-key");
        let _dnt = TempEnvGuard::set(ENV_DO_NOT_TRACK, "1");

        let ctx = gather_context(
            None,
            None,
            &[],
            &args(&["iam", "users", "list"]),
            "1.0.0",
            None,
            "plain",
            None,
        )
        .await;
        let identity = resolve_identity(None).await;
        let install_identity = load_or_create_install_identity("default");
        let client = TelemetryClient::from_env().await;

        assert!(ctx.is_none(), "gather_context must be disabled");
        assert!(identity.is_none(), "resolve_identity must be disabled");
        assert!(
            install_identity.is_none(),
            "load_or_create_install_identity must be disabled"
        );
        assert!(
            install_identity_path("default").is_some_and(|path| !path.exists()),
            "no install-identity file may be written while DO_NOT_TRACK is set"
        );
        assert!(
            client.inner.is_none(),
            "TelemetryClient::from_env must be disabled"
        );

        let run_ctx = WorkflowStepContext {
            run_id: "run-abc".into(),
            workflow_id: "competitive-multiplayer".into(),
            steps_total: 3,
            cli_version: "1.2.3".into(),
            is_dry_run: false,
            ui_surface: "fullscreen",
        };
        capture_workflow_run_started(&client, "user-123", &run_ctx, false, false);
        let facts = RunCompletedFacts {
            // Production emits `completed` for a successful run (step events
            // keep `success` for back-compat); the fixture must not drift.
            outcome: "completed",
            reason: None,
            duration_ms: 10,
            run_mode: None,
            steps_started: 1,
            steps_succeeded: 1,
            steps_failed: 0,
            steps_skipped: 0,
            steps_cancelled: 0,
            last_step_index: None,
            error_class: None,
            http_status: None,
            error_code: None,
            inputs_from_flag: 0,
            inputs_from_prompt: 0,
            inputs_from_default: 0,
            inputs_edited_in_form: 0,
        };
        capture_workflow_run_completed(&client, "user-123", &run_ctx, &facts, false, false);
    }

    #[test]
    fn test_redact_email_for_debug_keeps_only_the_domain() {
        assert_eq!(
            redact_email_for_debug("alice@example.com"),
            "***@example.com"
        );
    }

    #[test]
    fn test_redact_email_for_debug_falls_back_for_missing_at_sign() {
        assert_eq!(redact_email_for_debug("not-an-email"), "***");
    }

    /// A write followed by a read with the SAME access token must hit the cache.
    #[test]
    #[serial_test::serial]
    fn test_email_cache_round_trips_for_the_same_access_token() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        write_cached_email("default", "token-abc", "dev@example.com");

        assert_eq!(
            read_cached_email("default", "token-abc"),
            Some("dev@example.com".to_string())
        );
    }

    /// A cache entry written for one access token must be treated as a miss
    /// once the stored token changes (re-login/refresh) — it must never be
    /// served stale, since a rotated token can belong to a different person.
    #[test]
    #[serial_test::serial]
    fn test_email_cache_misses_after_the_access_token_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        write_cached_email("default", "token-old", "dev@example.com");

        assert_eq!(read_cached_email("default", "token-new"), None);
    }

    /// No cache file yet (fresh profile) is a plain miss, not an error.
    #[test]
    #[serial_test::serial]
    fn test_email_cache_misses_when_no_cache_file_exists_yet() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        assert_eq!(read_cached_email("default", "token-abc"), None);
    }

    #[test]
    #[serial_test::serial]
    fn test_clear_cached_email_removes_a_written_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        write_cached_email("default", "token-abc", "dev@example.com");
        assert!(read_cached_email("default", "token-abc").is_some());

        clear_cached_email("default");

        assert_eq!(read_cached_email("default", "token-abc"), None);
    }

    #[test]
    #[serial_test::serial]
    fn test_clear_cached_email_is_a_no_op_when_no_cache_file_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        clear_cached_email("default");
    }

    /// The id is created on first use, is stably reloaded on a second call,
    /// and carries the `anon-` prefix required by the privacy contract.
    #[test]
    #[serial_test::serial]
    fn test_install_identity_round_trips_and_is_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _key = TempEnvGuard::set(ENV_POSTHOG_KEY, "phc_test");
        let _dnt = TempEnvGuard::remove(ENV_DO_NOT_TRACK);

        let first = load_or_create_install_identity("default").expect("identity must be created");
        assert!(first.install_id.starts_with("anon-"));
        assert_eq!(first.merged_into_sub, None);

        let second = load_or_create_install_identity("default").expect("identity must load");
        assert_eq!(second.install_id, first.install_id, "id must be stable");
    }

    /// A user who never enables telemetry (no `AGS_TELEMETRY_POSTHOG_KEY`)
    /// must never get a file written for them — this is a hard privacy
    /// requirement, not just an optimization.
    #[test]
    #[serial_test::serial]
    fn test_install_identity_is_not_created_when_telemetry_is_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _key = TempEnvGuard::remove(ENV_POSTHOG_KEY);
        let _dnt = TempEnvGuard::remove(ENV_DO_NOT_TRACK);

        assert!(
            load_or_create_install_identity("default").is_none(),
            "a user who never enables telemetry must never get a file written"
        );
        assert!(
            install_identity_path("default").is_some_and(|path| !path.exists()),
            "no install-identity file may exist when telemetry is disabled"
        );
    }

    /// Recording a merge into the same `sub` twice leaves the id untouched;
    /// recording a merge into a *different* `sub` afterward rotates to a
    /// brand-new id with a cleared `merged_into_sub`.
    #[test]
    #[serial_test::serial]
    fn test_recording_a_merge_then_a_different_sub_rotates_the_id() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _key = TempEnvGuard::set(ENV_POSTHOG_KEY, "phc_test");
        let _dnt = TempEnvGuard::remove(ENV_DO_NOT_TRACK);

        let first = load_or_create_install_identity("default").unwrap();
        record_identity_merge("default", "user-a");
        let after_merge = load_or_create_install_identity("default").unwrap();
        assert_eq!(after_merge.install_id, first.install_id);
        assert_eq!(after_merge.merged_into_sub, Some("user-a".to_string()));

        record_identity_merge("default", "user-b");
        let rotated = load_or_create_install_identity("default").unwrap();
        assert_ne!(
            rotated.install_id, first.install_id,
            "a different sub must rotate the anon id, not re-merge"
        );
        assert_eq!(rotated.merged_into_sub, None);
    }

    /// Pure builder test: `$anon_distinct_id` links the anon id to the sub,
    /// and email — when present — lands only inside `$set`, never as a
    /// top-level `email` property.
    #[test]
    fn test_build_identify_event_links_the_anon_id_to_the_sub() {
        let event = build_identify_event("user-a", "anon-deadbeef", Some("dev@example.com"));
        let props = event.properties();
        assert_eq!(
            props.get("$anon_distinct_id").and_then(|v| v.as_str()),
            Some("anon-deadbeef")
        );
        assert_eq!(
            props.get("$lib").and_then(|v| v.as_str()),
            Some(LIB_OVERRIDE)
        );
        assert!(
            props.get("email").is_none(),
            "email must never be a top-level event property"
        );
        assert_eq!(
            props
                .get("$set")
                .and_then(|set| set.get("email"))
                .and_then(|v| v.as_str()),
            Some("dev@example.com"),
            "email must be carried as a $set person property"
        );
    }

    /// When no email is at hand — the common case, since login never fetches
    /// one — the builder must omit `$set` entirely rather than send an empty
    /// or null person-property patch.
    #[test]
    fn test_build_identify_event_omits_set_when_email_is_none() {
        let event = build_identify_event("user-a", "anon-deadbeef", None);
        let props = event.properties();
        assert!(
            props.get("$set").is_none(),
            "no $set patch should be sent when there is no email to carry"
        );
    }

    /// A user who never enables telemetry must see zero side effects from a
    /// successful login: no install-identity file is created or touched.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_emit_identity_merge_is_a_no_op_when_telemetry_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _key = TempEnvGuard::remove(ENV_POSTHOG_KEY);
        let _dnt = TempEnvGuard::remove(ENV_DO_NOT_TRACK);

        emit_identity_merge("default", "user-a", None).await;

        assert!(
            install_identity_path("default").is_some_and(|path| !path.exists()),
            "no install-identity file may be written when telemetry is disabled"
        );
    }

    /// Once a merge has already happened for this install (into any sub),
    /// `emit_identity_merge` must never queue a second `$identify` — but it
    /// must still delegate to `record_identity_merge`, because that is what
    /// rotates the anon id when a *different* user logs in on the same
    /// machine afterward. Getting this wrong would silently attribute the
    /// second user's pre-login events to the first. The prior merge is
    /// seeded directly via `record_identity_merge` (no network involved) so
    /// this test never has to exercise the real capture/flush path.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_emit_identity_merge_skips_capture_but_still_rotates_for_a_different_sub() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _key = TempEnvGuard::set(ENV_POSTHOG_KEY, "phc_test");
        let _dnt = TempEnvGuard::remove(ENV_DO_NOT_TRACK);

        let seeded = load_or_create_install_identity("default").unwrap();
        record_identity_merge("default", "user-a");

        // Re-merging the same sub: the skip branch fires, no rotation.
        emit_identity_merge("default", "user-a", None).await;
        let unchanged = load_or_create_install_identity("default").unwrap();
        assert_eq!(unchanged.install_id, seeded.install_id);
        assert_eq!(unchanged.merged_into_sub, Some("user-a".to_string()));

        // A different user logs in on the same machine: the skip branch
        // still fires (no second `$identify`), but `record_identity_merge`
        // rotates the id underneath it.
        emit_identity_merge("default", "user-b", None).await;
        let rotated = load_or_create_install_identity("default").unwrap();
        assert_ne!(
            rotated.install_id, seeded.install_id,
            "a different sub logging in must rotate the anon id"
        );
        assert_eq!(rotated.merged_into_sub, None);
    }

    /// The first-ever merge — the primary path of the whole identity feature,
    /// and the one whose failure mode is unrecoverable (PostHog cannot repair
    /// a missing or wrong merge after the fact). Drives the branch the
    /// skip/rotate test never reaches: capture, flush, then
    /// `record_identity_merge`, which must leave `merged_into_sub` naming this
    /// exact sub while keeping the anon id it just merged from. A disabled
    /// client makes the capture/flush pair a no-op, so no network is touched.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_emit_identity_merge_records_the_sub_on_the_first_ever_merge() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _key = TempEnvGuard::set(ENV_POSTHOG_KEY, "phc_test");
        let _dnt = TempEnvGuard::remove(ENV_DO_NOT_TRACK);

        let before = load_or_create_install_identity("default").expect("identity must be created");
        assert_eq!(
            before.merged_into_sub, None,
            "a fresh install must start un-merged"
        );

        emit_identity_merge_with(
            std::future::ready(TelemetryClient::disabled_for_test()),
            "default",
            "user-a",
            None,
        )
        .await;

        let after = load_or_create_install_identity("default").expect("identity must persist");
        assert_eq!(
            after.merged_into_sub,
            Some("user-a".to_string()),
            "the first merge must record the sub it merged into"
        );
        assert_eq!(
            after.install_id, before.install_id,
            "a first merge records, it never rotates the anon id it just merged from"
        );
    }

    #[test]
    fn test_build_event_includes_duration_ms_elapsed_since_started_at() {
        let ctx = CommandTelemetry {
            distinct_id: "user-123".into(),
            identity: "authenticated",
            ui_surface: "plain",
            email: None,
            namespace: None,
            studio: None,
            command_path: "iam users get".into(),
            flags: FlagCapture::default(),
            cli_version: "1.2.3".into(),
            auth_grant: None,
            started_at: std::time::Instant::now() - std::time::Duration::from_millis(50),
            workflow_run_id: None,
            workflow_id: None,
        };
        let outcome = Outcome {
            status: "completed",
            exit_code: 0,
            error_class: None,
            http_status: None,
            error_code: None,
        };
        let event = build_event(&ctx, &outcome);
        let duration = event
            .properties()
            .get("duration_ms")
            .and_then(|v| v.as_i64())
            .expect("duration_ms must be present and an integer");
        assert!(
            duration >= 50,
            "duration_ms should be at least the 50ms backdated into started_at, got {duration}"
        );
    }

    /// Both namespace levels attach as PostHog `$group`s under the same group
    /// type keys the Admin Portal uses (`game_namespace`, `studio`), not as
    /// top-level event properties — see [`build_event`].
    #[test]
    fn test_build_event_attaches_namespace_and_studio_as_groups() {
        let ctx = CommandTelemetry {
            distinct_id: "user-123".into(),
            identity: "authenticated",
            ui_surface: "plain",
            email: None,
            namespace: Some("mygame".into()),
            studio: Some("mystudio".into()),
            command_path: "iam users get".into(),
            flags: FlagCapture::default(),
            cli_version: "1.2.3".into(),
            auth_grant: None,
            started_at: std::time::Instant::now(),
            workflow_run_id: None,
            workflow_id: None,
        };
        let outcome = Outcome {
            status: "completed",
            exit_code: 0,
            error_class: None,
            http_status: None,
            error_code: None,
        };
        let event = build_event(&ctx, &outcome);
        // `Event::groups()` is `pub(crate)` in `posthog-rs`; serializing the
        // whole event is the only way to observe its private `groups` field
        // from outside the crate.
        let serialized = serde_json::to_value(&event).expect("event must serialize");
        let groups = serialized
            .get("groups")
            .and_then(|v| v.as_object())
            .expect("groups must be present");
        assert_eq!(
            groups.get("game_namespace").and_then(|v| v.as_str()),
            Some("mygame")
        );
        assert_eq!(
            groups.get("studio").and_then(|v| v.as_str()),
            Some("mystudio")
        );
    }

    #[test]
    fn test_build_event_is_constructible() {
        let ctx = CommandTelemetry {
            distinct_id: "user-123".into(),
            identity: "authenticated",
            ui_surface: "fullscreen",
            email: Some("dev@example.com".into()),
            namespace: Some("ns1".into()),
            studio: None,
            command_path: "iam users get".into(),
            flags: extract_flags(&args(&["iam", "users", "get", "-n", "ns1"])),
            cli_version: "1.2.3".into(),
            auth_grant: Some("authorization_code"),
            started_at: std::time::Instant::now(),
            workflow_run_id: None,
            workflow_id: None,
        };
        let outcome = Outcome {
            status: "completed",
            exit_code: 0,
            error_class: None,
            http_status: None,
            error_code: None,
        };
        let _event = build_event(&ctx, &outcome); // must not panic
    }

    /// A failed `workflow run <id>` invocation must carry the workflow's id
    /// plus the failing HTTP status and AccelByte error code as top-level
    /// properties — the exact trio a "which workflow, which upstream error"
    /// dashboard query needs.
    #[test]
    fn test_build_event_includes_workflow_id_http_status_and_error_code_on_failure() {
        let ctx = CommandTelemetry {
            distinct_id: "user-1".into(),
            identity: "authenticated",
            ui_surface: "fullscreen",
            email: None,
            namespace: None,
            studio: None,
            command_path: "workflow run".into(),
            flags: FlagCapture::default(),
            cli_version: "1.2.3".into(),
            auth_grant: None,
            started_at: std::time::Instant::now(),
            workflow_run_id: Some("run-1".into()),
            workflow_id: Some("in-game-store".into()),
        };
        let outcome = Outcome {
            status: "failed",
            exit_code: 3,
            error_class: Some("upstream"),
            http_status: Some(500),
            error_code: Some("20013".to_string()),
        };
        let event = build_event(&ctx, &outcome);
        let props = event.properties();
        assert_eq!(
            props.get("workflow_id").and_then(|v| v.as_str()),
            Some("in-game-store")
        );
        assert_eq!(props.get("http_status").and_then(|v| v.as_u64()), Some(500));
        assert_eq!(
            props.get("error_code").and_then(|v| v.as_str()),
            Some("20013")
        );
    }

    /// A successful invocation of a plain service command — not a workflow
    /// run, no failure — must omit `workflow_id`, `http_status`, and
    /// `error_code` entirely rather than emitting them as `null`.
    #[test]
    fn test_build_event_omits_workflow_id_http_status_and_error_code_on_success() {
        let ctx = CommandTelemetry {
            distinct_id: "user-1".into(),
            identity: "authenticated",
            ui_surface: "plain",
            email: None,
            namespace: None,
            studio: None,
            command_path: "iam users get".into(),
            flags: FlagCapture::default(),
            cli_version: "1.2.3".into(),
            auth_grant: None,
            started_at: std::time::Instant::now(),
            workflow_run_id: None,
            workflow_id: None,
        };
        let outcome = Outcome {
            status: "completed",
            exit_code: 0,
            error_class: None,
            http_status: None,
            error_code: None,
        };
        let event = build_event(&ctx, &outcome);
        let props = event.properties();
        assert!(props.get("workflow_id").is_none());
        assert!(props.get("http_status").is_none());
        assert!(props.get("error_code").is_none());
    }

    #[test]
    fn test_flag_capture_pipeline_recovers_global_and_command_flags_together() {
        // Reproduces the exact 2026-08-14 bug: `ags iam users get-information
        // --namespace X --user-id Y --format json` only captured `--user-id`,
        // because `--namespace`/`--format` were already stripped into
        // `GlobalFlags` before `extract_flags` ever saw the remaining argv.
        let mut capture = extract_flags(&args(&[
            "iam",
            "users",
            "get-information",
            "--user-id",
            "abc123",
        ]));
        let global_pairs = vec![
            (
                "--namespace".to_string(),
                Some("ammarabtestsa55-game".to_string()),
            ),
            ("--format".to_string(), Some("json".to_string())),
        ];

        merge_global_flags(&mut capture, &global_pairs);

        assert_eq!(
            capture.names,
            vec![
                "--user-id".to_string(),
                "--namespace".to_string(),
                "--format".to_string(),
            ]
        );
        assert_eq!(
            capture.values.get("--namespace"),
            Some(&"ammarabtestsa55-game".to_string())
        );
        assert_eq!(capture.values.get("--format"), Some(&"json".to_string()));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_resolve_identity_returns_none_when_disabled() {
        let _key = TempEnvGuard::remove(ENV_POSTHOG_KEY);
        let _dnt = TempEnvGuard::remove(ENV_DO_NOT_TRACK);

        assert!(resolve_identity(None).await.is_none());
    }

    /// With telemetry enabled and no stored token, `resolve_identity` must
    /// fall back to this install's anonymous id rather than returning `None`
    /// — the same fallback [`gather_context`] relies on.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_resolve_identity_falls_back_to_anonymous_without_a_stored_token() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _key = TempEnvGuard::set(ENV_POSTHOG_KEY, "phc_test");
        let _dnt = TempEnvGuard::remove(ENV_DO_NOT_TRACK);

        let identity = resolve_identity(Some(ANON_FALLBACK_TEST_PROFILE))
            .await
            .expect("telemetry is enabled, so an identity must resolve");

        assert_eq!(identity.label(), "anonymous");
        assert!(identity.distinct_id().starts_with("anon-"));
    }

    /// Proves the actual property values baked into a `step_started` event by
    /// [`build_step_started_event`] — not just that building/capturing
    /// doesn't panic. This is the assertion that would have caught the
    /// `step_completed` `step_id` reverse-engineering bug (Finding 1): had an
    /// equivalent assertion existed for `step_completed`, the mismatch
    /// between `StepStarted`'s real `id` and `StepFinished`'s parsed
    /// `summary` token would have been visible in the expected `step_id`.
    #[test]
    fn test_build_step_started_event_has_expected_properties() {
        let ctx = WorkflowStepContext {
            run_id: "run-abc".into(),
            workflow_id: "competitive-multiplayer".into(),
            steps_total: 3,
            cli_version: "1.2.3".into(),
            is_dry_run: false,
            ui_surface: "fullscreen",
        };
        let event =
            build_step_started_event("user-123", &ctx, 1, "create-lobby", "platform", "getStore");
        let props = event.properties();
        assert_eq!(
            props.get("run_id").and_then(|v| v.as_str()),
            Some("run-abc")
        );
        assert_eq!(
            props.get("workflow_id").and_then(|v| v.as_str()),
            Some("competitive-multiplayer")
        );
        assert_eq!(props.get("step_index").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(
            props.get("step_id").and_then(|v| v.as_str()),
            Some("create-lobby")
        );
        assert_eq!(props.get("steps_total").and_then(|v| v.as_i64()), Some(3));
        assert_eq!(
            props.get("cli_version").and_then(|v| v.as_str()),
            Some("1.2.3")
        );
        assert_eq!(
            props.get("is_dry_run").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            props.get("service").and_then(|v| v.as_str()),
            Some("platform")
        );
        assert_eq!(
            props.get("operation").and_then(|v| v.as_str()),
            Some("getStore")
        );
    }

    /// Same as above for `step_completed`, plus the `outcome` property that
    /// `step_started` doesn't carry.
    #[test]
    fn test_build_step_completed_event_has_expected_properties() {
        let ctx = WorkflowStepContext {
            run_id: "run-abc".into(),
            workflow_id: "competitive-multiplayer".into(),
            steps_total: 3,
            cli_version: "1.2.3".into(),
            is_dry_run: false,
            ui_surface: "fullscreen",
        };
        let facts = StepCompletedFacts {
            outcome: "success",
            reason: None,
            attempts: 1,
            duration_ms: 200,
            error_class: None,
            http_status: None,
            error_code: None,
            input_fields: Vec::new(),
            service: "platform".to_string(),
            operation: "getStore".to_string(),
        };
        let event = build_step_completed_event("user-123", &ctx, 1, "create-lobby", &facts);
        let props = event.properties();
        assert_eq!(
            props.get("run_id").and_then(|v| v.as_str()),
            Some("run-abc")
        );
        assert_eq!(
            props.get("workflow_id").and_then(|v| v.as_str()),
            Some("competitive-multiplayer")
        );
        assert_eq!(props.get("step_index").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(
            props.get("step_id").and_then(|v| v.as_str()),
            Some("create-lobby")
        );
        assert_eq!(props.get("steps_total").and_then(|v| v.as_i64()), Some(3));
        assert_eq!(
            props.get("cli_version").and_then(|v| v.as_str()),
            Some("1.2.3")
        );
        assert_eq!(
            props.get("outcome").and_then(|v| v.as_str()),
            Some("success")
        );
        assert_eq!(props.get("attempts").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(props.get("duration_ms").and_then(|v| v.as_u64()), Some(200));
        assert_eq!(
            props.get("is_dry_run").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            props.get("service").and_then(|v| v.as_str()),
            Some("platform")
        );
        assert_eq!(
            props.get("operation").and_then(|v| v.as_str()),
            Some("getStore")
        );
        assert!(props.get("outcome_reason").is_none());
        assert!(props.get("error_class").is_none());
        assert!(props.get("http_status").is_none());
        assert!(props.get("error_code").is_none());
        // A successful step's facts carry no input fields — the property
        // must be omitted entirely, not sent as an empty array.
        assert!(props.get("input_fields").is_none());
    }

    /// Proves every property `build_step_completed_event` reports for a
    /// non-trivial skip: the enriched facts (Tasks 3-9) that this task
    /// finally carries into the event — `outcome_reason`, `attempts`,
    /// `duration_ms`, `http_status`, `error_code` — plus the three brand-new
    /// properties this task adds: `is_dry_run`, `service`, `operation`.
    #[test]
    fn test_build_step_completed_event_includes_reason_error_and_attempts() {
        let context = WorkflowStepContext {
            run_id: "run-1".to_string(),
            workflow_id: "in-game-store".to_string(),
            steps_total: 3,
            cli_version: "1.2.3".to_string(),
            is_dry_run: true,
            ui_surface: "fullscreen",
        };
        let facts = StepCompletedFacts {
            outcome: "skipped",
            reason: Some("already_exists"),
            attempts: 2,
            duration_ms: 1500,
            error_class: Some("upstream"),
            http_status: Some(409),
            error_code: Some("20013".to_string()),
            input_fields: Vec::new(),
            service: "platform".to_string(),
            operation: "createStore".to_string(),
        };
        let event = build_step_completed_event("user-1", &context, 1, "create-store", &facts);
        let props = event.properties();
        assert_eq!(
            props.get("outcome").and_then(|v| v.as_str()),
            Some("skipped")
        );
        assert_eq!(
            props.get("outcome_reason").and_then(|v| v.as_str()),
            Some("already_exists")
        );
        assert_eq!(props.get("attempts").and_then(|v| v.as_u64()), Some(2));
        assert_eq!(
            props.get("duration_ms").and_then(|v| v.as_u64()),
            Some(1500)
        );
        assert_eq!(
            props.get("error_class").and_then(|v| v.as_str()),
            Some("upstream")
        );
        assert_eq!(props.get("http_status").and_then(|v| v.as_u64()), Some(409));
        assert_eq!(
            props.get("error_code").and_then(|v| v.as_str()),
            Some("20013")
        );
        assert_eq!(
            props.get("is_dry_run").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            props.get("service").and_then(|v| v.as_str()),
            Some("platform")
        );
        assert_eq!(
            props.get("operation").and_then(|v| v.as_str()),
            Some("createStore")
        );
    }

    /// A failed step's non-empty `input_fields` reaches the event as a JSON
    /// array with the expected per-field shape, including a withheld
    /// (`value: null`) entry for an external workflow's field.
    #[test]
    fn test_build_step_completed_event_includes_input_fields_when_present() {
        let context = WorkflowStepContext {
            run_id: "run-1".to_string(),
            workflow_id: "external-probe".to_string(),
            steps_total: 1,
            cli_version: "1.2.3".to_string(),
            is_dry_run: false,
            ui_surface: "fullscreen",
        };
        let facts = StepCompletedFacts {
            outcome: "failed",
            reason: Some("dispatch"),
            attempts: 1,
            duration_ms: 50,
            error_class: Some("rejected"),
            http_status: Some(400),
            error_code: Some("20013".to_string()),
            input_fields: vec![
                ags_protocol::workflow::StepInputField {
                    field: "storeName".to_string(),
                    location: ags_protocol::workflow::StepFieldLocation::Body,
                    source: "flag",
                    required: true,
                    value: Some(serde_json::json!("acme")),
                },
                ags_protocol::workflow::StepInputField {
                    field: "clientSecret".to_string(),
                    location: ags_protocol::workflow::StepFieldLocation::Body,
                    source: "prompt",
                    required: true,
                    value: None,
                },
            ],
            service: "platform".to_string(),
            operation: "createStore".to_string(),
        };
        let event = build_step_completed_event("user-1", &context, 0, "create-store", &facts);
        let props = event.properties();
        let input_fields = props
            .get("input_fields")
            .and_then(|v| v.as_array())
            .expect("input_fields must be a JSON array");
        assert_eq!(input_fields.len(), 2);
        assert_eq!(
            input_fields[0].get("field").and_then(|v| v.as_str()),
            Some("storeName")
        );
        assert_eq!(
            input_fields[0].get("location").and_then(|v| v.as_str()),
            Some("body")
        );
        assert_eq!(
            input_fields[0].get("source").and_then(|v| v.as_str()),
            Some("flag")
        );
        assert_eq!(
            input_fields[0].get("required").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            input_fields[0].get("value").and_then(|v| v.as_str()),
            Some("acme")
        );
        // The second field's value was withheld — it must serialize as
        // `null`, never dropped or replaced by anything that reads as data.
        assert!(input_fields[1]
            .get("value")
            .expect("value key must always be present")
            .is_null());
    }

    /// The public `capture_*` entry points must still not panic against a
    /// disabled client — the fire-and-forget contract the CLI's
    /// `ExecutionFrontendAdapter` tests rely on.
    #[test]
    fn test_capture_workflow_step_started_and_completed_do_not_panic_when_disabled() {
        let client = TelemetryClient { inner: None }; // disabled: capture() is then a no-op tdbg! call
        let ctx = WorkflowStepContext {
            run_id: "run-abc".into(),
            workflow_id: "competitive-multiplayer".into(),
            steps_total: 3,
            cli_version: "1.2.3".into(),
            is_dry_run: false,
            ui_surface: "fullscreen",
        };
        capture_workflow_step_started(
            &client,
            "user-123",
            &ctx,
            0,
            "create-lobby",
            "platform",
            "getStore",
        );
        let facts = StepCompletedFacts {
            outcome: "success",
            reason: None,
            attempts: 1,
            duration_ms: 10,
            error_class: None,
            http_status: None,
            error_code: None,
            input_fields: Vec::new(),
            service: "platform".to_string(),
            operation: "getStore".to_string(),
        };
        capture_workflow_step_completed(&client, "user-123", &ctx, 0, "create-lobby", &facts);
    }

    /// Proves the actual property values baked into a `run_started` event by
    /// [`build_run_started_event`] — including `ui_surface`, which the
    /// step-level events deliberately omit.
    #[test]
    fn test_build_run_started_event_has_expected_properties() {
        let ctx = WorkflowStepContext {
            run_id: "run-abc".into(),
            workflow_id: "competitive-multiplayer".into(),
            steps_total: 3,
            cli_version: "1.2.3".into(),
            is_dry_run: false,
            ui_surface: "fullscreen",
        };
        let event = build_run_started_event("user-123", &ctx, true, false);
        let props = event.properties();
        assert_eq!(
            props.get("run_id").and_then(|v| v.as_str()),
            Some("run-abc")
        );
        assert_eq!(
            props.get("workflow_id").and_then(|v| v.as_str()),
            Some("competitive-multiplayer")
        );
        assert_eq!(props.get("steps_total").and_then(|v| v.as_i64()), Some(3));
        assert_eq!(
            props.get("cli_version").and_then(|v| v.as_str()),
            Some("1.2.3")
        );
        assert_eq!(
            props.get("is_dry_run").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            props.get("ui_surface").and_then(|v| v.as_str()),
            Some("fullscreen")
        );
        assert_eq!(
            props.get("assume_yes").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(props.get("no_input").and_then(|v| v.as_bool()), Some(false));
    }

    /// Proves every property `build_run_completed_event` reports for a
    /// non-trivial failed run, including the `reason` -> `outcome_reason`
    /// rename that is the one field/property name mismatch in this event.
    #[test]
    fn test_build_run_completed_event_includes_counts_and_outcome() {
        let context = WorkflowStepContext {
            run_id: "run-1".to_string(),
            workflow_id: "in-game-store".to_string(),
            steps_total: 4,
            cli_version: "1.2.3".to_string(),
            is_dry_run: false,
            ui_surface: "fullscreen",
        };
        let facts = RunCompletedFacts {
            outcome: "failed",
            reason: Some("dispatch"),
            duration_ms: 4200,
            run_mode: Some("review_input_steps"),
            steps_started: 3,
            steps_succeeded: 2,
            steps_failed: 1,
            steps_skipped: 0,
            steps_cancelled: 0,
            last_step_index: Some(2),
            error_class: Some("upstream"),
            http_status: Some(500),
            error_code: None,
            inputs_from_flag: 2,
            inputs_from_prompt: 1,
            inputs_from_default: 0,
            inputs_edited_in_form: 1,
        };
        let event = build_run_completed_event("user-1", &context, &facts, true, false);
        let props = event.properties();
        assert_eq!(props.get("run_id").and_then(|v| v.as_str()), Some("run-1"));
        assert_eq!(
            props.get("workflow_id").and_then(|v| v.as_str()),
            Some("in-game-store")
        );
        assert_eq!(props.get("steps_total").and_then(|v| v.as_i64()), Some(4));
        assert_eq!(
            props.get("cli_version").and_then(|v| v.as_str()),
            Some("1.2.3")
        );
        assert_eq!(
            props.get("is_dry_run").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            props.get("ui_surface").and_then(|v| v.as_str()),
            Some("fullscreen")
        );
        assert_eq!(
            props.get("outcome").and_then(|v| v.as_str()),
            Some("failed")
        );
        assert_eq!(
            props.get("outcome_reason").and_then(|v| v.as_str()),
            Some("dispatch")
        );
        assert_eq!(
            props.get("duration_ms").and_then(|v| v.as_i64()),
            Some(4200)
        );
        assert_eq!(
            props.get("run_mode").and_then(|v| v.as_str()),
            Some("review_input_steps")
        );
        assert_eq!(props.get("steps_started").and_then(|v| v.as_i64()), Some(3));
        assert_eq!(
            props.get("steps_succeeded").and_then(|v| v.as_i64()),
            Some(2)
        );
        assert_eq!(props.get("steps_failed").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(props.get("steps_skipped").and_then(|v| v.as_i64()), Some(0));
        assert_eq!(
            props.get("steps_cancelled").and_then(|v| v.as_i64()),
            Some(0)
        );
        assert_eq!(
            props.get("last_step_index").and_then(|v| v.as_i64()),
            Some(2)
        );
        assert_eq!(
            props.get("error_class").and_then(|v| v.as_str()),
            Some("upstream")
        );
        assert_eq!(props.get("http_status").and_then(|v| v.as_u64()), Some(500));
        assert!(props.get("error_code").is_none());
        assert_eq!(
            props.get("inputs_from_flag").and_then(|v| v.as_i64()),
            Some(2)
        );
        assert_eq!(
            props.get("inputs_from_prompt").and_then(|v| v.as_i64()),
            Some(1)
        );
        assert_eq!(
            props.get("inputs_from_default").and_then(|v| v.as_i64()),
            Some(0)
        );
        assert_eq!(
            props.get("inputs_edited_in_form").and_then(|v| v.as_i64()),
            Some(1)
        );
        // Spec §3.2: run_completed carries everything run_started does.
        assert_eq!(
            props.get("assume_yes").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(props.get("no_input").and_then(|v| v.as_bool()), Some(false));
    }

    /// Every `Option` field on `RunCompletedFacts` must be omitted (never
    /// emitted as a JSON `null`) when absent — the run-level analogue of
    /// `test_build_step_completed_event_has_expected_properties`'s `is_none`
    /// assertions.
    #[test]
    fn test_build_run_completed_event_omits_none_fields() {
        let ctx = WorkflowStepContext {
            run_id: "run-1".to_string(),
            workflow_id: "in-game-store".to_string(),
            steps_total: 2,
            cli_version: "1.2.3".to_string(),
            is_dry_run: false,
            ui_surface: "fullscreen",
        };
        let facts = RunCompletedFacts {
            // Production emits `completed` for a successful run (step events
            // keep `success` for back-compat); the fixture must not drift.
            outcome: "completed",
            reason: None,
            duration_ms: 100,
            run_mode: None,
            steps_started: 2,
            steps_succeeded: 2,
            steps_failed: 0,
            steps_skipped: 0,
            steps_cancelled: 0,
            last_step_index: None,
            error_class: None,
            http_status: None,
            error_code: None,
            inputs_from_flag: 0,
            inputs_from_prompt: 0,
            inputs_from_default: 2,
            inputs_edited_in_form: 0,
        };
        let event = build_run_completed_event("user-1", &ctx, &facts, false, true);
        let props = event.properties();
        assert!(props.get("outcome_reason").is_none());
        assert!(props.get("run_mode").is_none());
        assert!(props.get("last_step_index").is_none());
        assert!(props.get("error_class").is_none());
        assert!(props.get("http_status").is_none());
        assert!(props.get("error_code").is_none());
    }

    /// The public `capture_*` entry points for run-level events must also not
    /// panic against a disabled client — the same fire-and-forget contract
    /// already proven for the step-level entry points.
    #[test]
    fn test_capture_workflow_run_started_and_completed_do_not_panic_when_disabled() {
        let client = TelemetryClient { inner: None }; // disabled: capture() is then a no-op tdbg! call
        let ctx = WorkflowStepContext {
            run_id: "run-abc".into(),
            workflow_id: "competitive-multiplayer".into(),
            steps_total: 3,
            cli_version: "1.2.3".into(),
            is_dry_run: false,
            ui_surface: "fullscreen",
        };
        capture_workflow_run_started(&client, "user-123", &ctx, true, false);
        let facts = RunCompletedFacts {
            outcome: "failed",
            reason: Some("dispatch"),
            duration_ms: 100,
            run_mode: Some("review_input_steps"),
            steps_started: 2,
            steps_succeeded: 1,
            steps_failed: 1,
            steps_skipped: 0,
            steps_cancelled: 0,
            last_step_index: Some(1),
            error_class: Some("upstream"),
            http_status: Some(500),
            error_code: Some("20013".to_string()),
            inputs_from_flag: 1,
            inputs_from_prompt: 1,
            inputs_from_default: 0,
            inputs_edited_in_form: 0,
        };
        capture_workflow_run_completed(&client, "user-123", &ctx, &facts, true, false);
    }
}
