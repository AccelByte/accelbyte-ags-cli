# Config & State — AGS CLI Review Conventions

Part of the [AGS CLI Review Conventions](../review-conventions.md) (RULE-12–RULE-13, 2 rules).

### RULE-12 — Every new config key must have a `ConfigKeyDef` entry in `KNOWN_KEYS`

**Rule:** All writable config keys must be declared in the `KNOWN_KEYS: &[ConfigKeyDef]`
static in `crates/ags-runtime/src/runtime/config/keys.rs`. A key absent from this array
cannot be set or retrieved via `ags config` and will not appear in resolved config views.

**Adding a key requires three changes:**
1. A `ConfigKeyDef` entry in `KNOWN_KEYS` with `cli_name` (kebab-case), `json_name`
   (snake_case), and `scope`.
2. A matching struct field in `GlobalConfig` or `ProfileConfig`.
3. Handling in `apply_config_defaults` (`invocation/flags.rs`) if the key should influence
   flag resolution at startup.

**Special case:** `client-secret` is intentionally absent from `KNOWN_KEYS`; it is
keychain-managed and handled via `CLIENT_SECRET_ALIASES`. Do not add keychain-managed
secrets to `KNOWN_KEYS`.

**House pattern:**
```rust
// crates/ags-runtime/src/runtime/config/keys.rs:26-74
pub static KNOWN_KEYS: &[ConfigKeyDef] = &[
    ConfigKeyDef { cli_name: "active-profile", json_name: "active_profile", scope: ConfigScope::Global },
    ConfigKeyDef { cli_name: "base-url",        json_name: "base_url",       scope: ConfigScope::Profile },
    ConfigKeyDef { cli_name: "client-id",       json_name: "client_id",      scope: ConfigScope::Profile },
    // … (ten entries total)
];
```

**Self-check:**
```
grep -rn "KNOWN_KEYS" crates/ --include="*.rs"
```
Expected: one definition site and callers that read it. Any bypass is a violation.

**Provenance:** `crates/ags-runtime/src/runtime/config/keys.rs:26-74, 89-91`

---

### RULE-13 — Every config key must declare its `ConfigScope`; global and profile keys must not be stored interchangeably

**Rule:** Each `ConfigKeyDef` entry must assign `scope: ConfigScope::Global` (stored in
top-level config, shared across profiles) or `scope: ConfigScope::Profile` (stored
per-profile). The scope determines which storage path is used; mixing them corrupts the
config layout.

**Current scope assignments:** `format`, `no-color`, `timeout`, `page-limit`,
`active-profile`, `first-run-hint-seen` are Global (`keys.rs:28-57`); `base-url`,
`client-id`, `namespace`, `grant-type` are Profile.

**House pattern:**
```rust
// crates/ags-runtime/src/runtime/config/keys.rs:8-12
pub enum ConfigScope {
    Global,
    Profile,
}
```

**Self-check:**
```
grep -rn "ConfigScope" crates/ --include="*.rs"
```
Verify each usage routes correctly to `GlobalConfig` vs `ProfileConfig` storage.

**Provenance:** `crates/ags-runtime/src/runtime/config/keys.rs:8-23`

---
