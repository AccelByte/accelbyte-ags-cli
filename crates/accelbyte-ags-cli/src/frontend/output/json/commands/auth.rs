//! JSON rendering for auth command output.

use serde_json::Value;

use crate::errors::CliError;
use crate::frontend::output::json::format_json;
use crate::frontend::presenters::auth as auth_presenter;
use crate::frontend::RenderOptions;
use crate::frontend::RenderedOutput;
use ags_protocol::output::{
    AuthActionStatus, AuthOutput, AuthStatusData, AuthView, Presence, TokenState,
};

/// Render auth command output as JSON
pub(crate) fn render_auth_output(
    output: &AuthOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    Ok(RenderedOutput {
        stdout: Some(render_auth_view_json(&output.view)?),
        stderr: None,
        is_stdout_first: true,
    })
}

/// Serialize an auth view to pretty-printed JSON
fn render_auth_view_json(view: &AuthView) -> Result<String, CliError> {
    let value = match view {
        AuthView::Authenticated(data) => {
            let mut json_object = serde_json::Map::new();
            json_object.insert(
                "status".to_string(),
                Value::String("authenticated".to_string()),
            );
            insert_auth_status_fields(&mut json_object, data);
            Value::Object(json_object)
        }
        AuthView::RequiresAttention(data) => {
            let mut json_object = serde_json::Map::new();
            json_object.insert(
                "status".to_string(),
                Value::String("requires_attention".to_string()),
            );
            insert_auth_status_fields(&mut json_object, data);
            Value::Object(json_object)
        }
        AuthView::LoginSuccess(data) => {
            let status = match data.status {
                AuthActionStatus::LoggedIn => "logged_in",
                AuthActionStatus::AlreadyAuthenticated => "already_authenticated",
                AuthActionStatus::Refreshed => "refreshed",
            };
            serde_json::json!({ "status": status })
        }
        AuthView::RefreshSuccess(data) => {
            let mut object = serde_json::Map::new();
            object.insert("status".to_string(), Value::String("refreshed".to_string()));
            if let Some(base_url) = &data.base_url {
                object.insert("base_url".to_string(), Value::String(base_url.clone()));
            }
            if let Some(login_type) = &data.login_type {
                object.insert("login_type".to_string(), Value::String(login_type.clone()));
            }
            if let Some(client_id) = &data.client_id {
                object.insert("client_id".to_string(), Value::String(client_id.clone()));
            }
            if let Some(secs) = data.token_expires_in_secs {
                object.insert(
                    "token_expires_in_secs".to_string(),
                    Value::Number(secs.into()),
                );
            }
            if let Some(tip) = &data.tip {
                object.insert("tip".to_string(), Value::String(tip.clone()));
            }
            Value::Object(object)
        }
        AuthView::LogoutSuccess(_) => {
            serde_json::json!({
                "status": "cleared",
            })
        }
        AuthView::LogoutAllSuccess(data) => {
            serde_json::json!({
                "status": "cleared",
                "profiles": data.profiles_cleared,
            })
        }
        AuthView::NotAuthenticated { .. } => {
            serde_json::json!({ "status": "not_authenticated" })
        }
    };

    format_json(&value)
}

/// Insert credential fields into a JSON object for the status JSON output
fn insert_auth_status_fields(obj: &mut serde_json::Map<String, Value>, data: &AuthStatusData) {
    if let Some(source_label) = auth_presenter::json_source_label(data.source) {
        obj.insert(
            "source".to_string(),
            Value::String(source_label.to_string()),
        );
    }

    if let Some(base_url) = &data.base_url {
        obj.insert("base_url".to_string(), Value::String(base_url.clone()));
    }
    if let Some(login_type) = &data.login_type {
        obj.insert("login_type".to_string(), Value::String(login_type.clone()));
    }
    if let Some(client_id) = &data.client_id {
        obj.insert("client_id".to_string(), Value::String(client_id.clone()));
    }

    if !matches!(data.client_secret, Presence::Unknown) {
        obj.insert(
            "client_secret".to_string(),
            Value::String(auth_presenter::presence_label(data.client_secret).to_string()),
        );
    }

    insert_token_state_fields(obj, "access_token", "token_expires_in", &data.access_token);
    insert_token_state_fields(
        obj,
        "refresh_token",
        "refresh_expires_in",
        &data.refresh_token,
    );

    if let Some(namespace) = &data.namespace {
        obj.insert("namespace".to_string(), Value::String(namespace.clone()));
    }
}

/// Insert token state and optional expiry into a JSON object
fn insert_token_state_fields(
    obj: &mut serde_json::Map<String, Value>,
    state_key: &str,
    expires_key: &str,
    state: &TokenState,
) {
    if matches!(state, TokenState::Unknown) {
        return;
    }

    obj.insert(
        state_key.to_string(),
        Value::String(auth_presenter::token_state_label(state).to_string()),
    );

    if let Some(expires_in_secs) = auth_presenter::token_expiry_secs(state) {
        obj.insert(
            expires_key.to_string(),
            Value::Number(expires_in_secs.into()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_refresh_success_json_carries_fields() {
        use ags_protocol::output::{AuthActionData, AuthActionStatus, AuthView};
        let view = AuthView::RefreshSuccess(AuthActionData {
            status: AuthActionStatus::Refreshed,
            base_url: Some("https://dev".to_string()),
            login_type: Some("client credentials".to_string()),
            client_id: Some("cid".to_string()),
            token_expires_in_secs: Some(3600),
            tip: None,
        });
        let rendered = render_auth_view_json(&view).expect("json render");
        let json: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(json["status"], "refreshed");
        assert_eq!(json["base_url"], "https://dev");
        assert_eq!(json["login_type"], "client credentials");
        assert_eq!(json["client_id"], "cid");
        assert_eq!(json["token_expires_in_secs"], 3600);
        assert!(
            json.get("tip").is_none(),
            "absent optional fields must be omitted"
        );
    }

    #[test]
    fn test_login_success_json_unchanged_still_status_only() {
        use ags_protocol::output::{AuthActionData, AuthActionStatus, AuthView};
        let view = AuthView::LoginSuccess(AuthActionData {
            status: AuthActionStatus::LoggedIn,
            base_url: Some("https://dev".to_string()),
            login_type: Some("client credentials".to_string()),
            client_id: Some("cid".to_string()),
            token_expires_in_secs: Some(3600),
            tip: None,
        });
        let rendered = render_auth_view_json(&view).expect("json render");
        let json: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        // LoginSuccess must remain status-only (no cross-command change).
        assert_eq!(json["status"], "logged_in");
        assert!(
            json.get("base_url").is_none(),
            "login JSON must not gain fields"
        );
    }
}
