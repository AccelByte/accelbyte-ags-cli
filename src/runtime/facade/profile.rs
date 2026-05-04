//! Profile facade — `Runtime` methods for CRUD operations on named profiles
//! (create, list, switch, show, delete, rename).

use crate::protocol::error::{RuntimeError, RuntimeErrorKind};

impl crate::runtime::Runtime {
    /// List every known profile and indicate which one is currently active.
    pub fn profile_list(&self) -> Result<crate::protocol::output::ProfileView, RuntimeError> {
        use crate::protocol::output::{ProfileSummary, ProfileView};
        use crate::runtime::config::{self, GlobalConfig};

        let global = GlobalConfig::load()?;
        let active = global.active_profile.clone();
        let profiles_dir = config::profiles_dir()?;
        let mut profiles = Vec::new();

        if profiles_dir.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(&profiles_dir)
                .map_err(|error| RuntimeError {
                    kind: RuntimeErrorKind::Internal,
                    message: format!("Failed to read profiles directory: {error}"),
                    details: None,
                    hint: None,
                    trace: None,
                })?
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.path().is_dir())
                .collect();

            entries.sort_by_key(|entry| entry.file_name());

            for entry in entries {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_active = active.as_deref() == Some(&name);
                profiles.push(ProfileSummary { name, is_active });
            }
        }

        Ok(ProfileView::List { profiles, active })
    }

    /// Create a new named profile with an empty config file.
    pub fn profile_create(
        &self,
        name: &str,
    ) -> Result<crate::protocol::output::ProfileView, RuntimeError> {
        use crate::protocol::output::ProfileView;
        use crate::runtime::config;

        let name = config::validate_profile_name(name)?;

        if config::profile_exists(&name)? {
            return Err(RuntimeError {
                kind: RuntimeErrorKind::Validation,
                message: format!("Profile '{name}' already exists."),
                details: None,
                hint: Some(format!(
                    "Use 'ags profile show {name}' to inspect it or choose a different name."
                )),
                trace: None,
            });
        }

        config::ensure_profile_exists(&name)?;
        Ok(ProfileView::Created { name })
    }

    /// Switch the global active-profile pointer to an existing named profile.
    pub fn profile_use(
        &self,
        name: &str,
    ) -> Result<crate::protocol::output::ProfileView, RuntimeError> {
        use crate::protocol::output::ProfileView;
        use crate::runtime::config;

        let name = config::validate_profile_name(name)?;

        if !config::profile_exists(&name)? {
            return Err(RuntimeError {
                kind: RuntimeErrorKind::Validation,
                message: format!("Profile '{name}' does not exist."),
                details: None,
                hint: Some(format!("Run 'ags profile create {name}' first.")),
                trace: None,
            });
        }

        crate::runtime::config::GlobalConfig::update(|global| {
            global.active_profile = Some(name.clone());
            Ok(())
        })?;

        Ok(ProfileView::Switched { name })
    }

    /// Show the stored configuration and auth state for one profile.
    pub fn profile_show(
        &self,
        name: Option<&str>,
    ) -> Result<crate::protocol::output::ProfileView, RuntimeError> {
        use crate::protocol::output::{ProfileShowData, ProfileView};
        use crate::runtime::auth::store;
        use crate::runtime::config::{self, GlobalConfig, ProfileConfig};

        let name = match name {
            Some(name) => config::validate_profile_name(name)?,
            None => match config::resolve_profile_name(None) {
                Ok(name) => name,
                Err(_) => return Ok(ProfileView::NoActiveProfile),
            },
        };

        if !config::profile_exists(&name)? {
            return Err(RuntimeError {
                kind: RuntimeErrorKind::Validation,
                message: format!("Profile '{name}' does not exist."),
                details: None,
                hint: Some(format!("Run 'ags profile create {name}' first.")),
                trace: None,
            });
        }

        let global = GlobalConfig::load()?;
        let is_active = global.active_profile.as_deref() == Some(&name);
        let profile_config = ProfileConfig::load(&name)?;
        let has_secret = store::get_secret(&name).ok().flatten().is_some();
        let has_token = store::get_token_data(&name).ok().flatten().is_some();

        Ok(ProfileView::Show {
            name,
            is_active,
            config: ProfileShowData {
                base_url: profile_config.base_url,
                client_id: profile_config.client_id,
                namespace: profile_config.namespace,
                grant_type: profile_config.grant_type.map(|g| g.to_string()),
                has_secret,
                has_token,
            },
        })
    }

    /// Delete one named profile and clean up any stored credentials.
    pub fn profile_delete(
        &self,
        name: &str,
    ) -> Result<crate::protocol::output::ProfileView, RuntimeError> {
        use crate::protocol::output::{OperationWarning, ProfileView};
        use crate::runtime::auth::store;
        use crate::runtime::config;

        let name = config::validate_profile_name(name)?;

        if !config::profile_exists(&name)? {
            return Err(RuntimeError {
                kind: RuntimeErrorKind::Validation,
                message: format!("Profile '{name}' does not exist."),
                details: None,
                hint: None,
                trace: None,
            });
        }

        let mut warnings = Vec::new();

        if let Err(error) = store::delete_secret(&name) {
            warnings.push(OperationWarning {
                message: "Could not remove stored secret from keychain".into(),
                reason: Some(error.to_string()),
                fix: "The orphaned keychain entry is harmless and can be ignored.".into(),
            });
        }
        if let Err(error) = store::delete_token_data(&name) {
            warnings.push(OperationWarning {
                message: "Could not remove stored tokens from keychain".into(),
                reason: Some(error.to_string()),
                fix: "The orphaned keychain entry is harmless and can be ignored.".into(),
            });
        }

        let dir = config::profile_dir(&name)?;
        std::fs::remove_dir_all(&dir).map_err(|error| RuntimeError {
            kind: RuntimeErrorKind::Internal,
            message: format!("Failed to delete profile directory: {error}"),
            details: None,
            hint: None,
            trace: None,
        })?;

        let mut tips = Vec::new();
        let mut is_active_profile = false;

        if let Err(error) = crate::runtime::config::GlobalConfig::update(|global| {
            if global.active_profile.as_deref() == Some(&name) {
                is_active_profile = true;
                global.active_profile = None;
            }
            Ok(())
        }) {
            warnings.push(OperationWarning {
                message: "Could not clear active profile reference".into(),
                reason: Some(error.to_string()),
                fix: "Run 'ags profile use <name>' to set a new active profile.".into(),
            });
        } else if is_active_profile {
            warnings.push(OperationWarning {
                message: format!("'{name}' was the active profile"),
                reason: None,
                fix: "Run 'ags profile use <name>' to set a new active profile.".into(),
            });
        }

        if name == config::DEFAULT_PROFILE {
            tips.push(
                "The default profile will not be recreated automatically. Run 'ags profile create default' if needed.".into(),
            );
        }

        Ok(ProfileView::Deleted {
            name,
            warnings,
            tips,
        })
    }

    /// Rename one profile and migrate any stored credentials to the new name.
    pub fn profile_rename(
        &self,
        old: &str,
        new: &str,
    ) -> Result<crate::protocol::output::ProfileView, RuntimeError> {
        use crate::protocol::output::{OperationWarning, ProfileView};
        use crate::runtime::auth::{locking, store};
        use crate::runtime::config;

        let old = config::validate_profile_name(old)?;
        let new = config::validate_profile_name(new)?;

        if !config::profile_exists(&old)? {
            return Err(RuntimeError {
                kind: RuntimeErrorKind::Validation,
                message: format!("Profile '{old}' does not exist."),
                details: None,
                hint: None,
                trace: None,
            });
        }

        if config::profile_exists(&new)? {
            return Err(RuntimeError {
                kind: RuntimeErrorKind::Validation,
                message: format!("Profile '{new}' already exists."),
                details: None,
                hint: None,
                trace: None,
            });
        }

        let old_dir = config::profile_dir(&old)?;
        let new_dir = config::profile_dir(&new)?;
        // On Unix, `std::fs::rename` silently overwrites an empty destination
        // directory; on Windows it returns `AlreadyExists`. The
        // `profile_exists(&new)` pre-check above makes both cases unlikely in
        // normal single-user operation, but a concurrent `ags profile create
        // <new>` could race between the check and the rename. This narrow
        // TOCTOU window is accepted; the surrounding `ags profile` commands
        // are not designed for concurrent invocation.
        // (Cross-compile validation only — no Windows runtime verification yet.)
        std::fs::rename(&old_dir, &new_dir).map_err(|error| RuntimeError {
            kind: RuntimeErrorKind::Internal,
            message: format!("Failed to rename profile directory: {error}"),
            details: None,
            hint: None,
            trace: None,
        })?;

        let mut warnings = Vec::new();

        if let Ok(Some(secret)) = store::get_secret(&old) {
            if let Err(error) = store::store_secret(&new, &secret) {
                warnings.push(OperationWarning {
                    message: "Could not migrate stored secret to new profile".into(),
                    reason: Some(error.to_string()),
                    fix: format!("Run 'ags auth login --profile {new}' to re-authenticate."),
                });
            } else if let Err(error) = store::delete_secret(&old) {
                warnings.push(OperationWarning {
                    message: "Could not remove old keychain entry for secret".into(),
                    reason: Some(error.to_string()),
                    fix: "The orphaned entry is harmless and can be ignored.".into(),
                });
            }
        }

        if let Ok(Some(token_data)) = store::get_token_data(&old) {
            if let Err(error) = locking::with_token_locks(&[&old, &new], || {
                store::store_token_data_unlocked(&new, &token_data)?;
                store::delete_token_data_unlocked(&old)
            }) {
                warnings.push(OperationWarning {
                    message: "Could not migrate stored tokens to new profile".into(),
                    reason: Some(error.to_string()),
                    fix: format!("Run 'ags auth login --profile {new}' to re-authenticate."),
                });
            }
        }

        if let Err(error) = crate::runtime::config::GlobalConfig::update(|global| {
            if global.active_profile.as_deref() == Some(&old) {
                global.active_profile = Some(new.clone());
            }
            Ok(())
        }) {
            warnings.push(OperationWarning {
                message: "Could not update active profile reference".into(),
                reason: Some(error.to_string()),
                fix: format!("Run 'ags profile use {new}'."),
            });
        }

        Ok(ProfileView::Renamed { old, new, warnings })
    }
}
