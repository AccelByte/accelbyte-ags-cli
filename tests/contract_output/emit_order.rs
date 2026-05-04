use ags::frontend::human::render;
use ags::frontend::RenderOptions;
use ags::protocol::output::{
    ApiBody, ApiOutput, ApiSuccess, AuthOutput, AuthView, CommandOutput, ExecutionTrace,
    RequestTrace, ResponseTrace,
};
use ags::protocol::result::{CommandResult, RawResult};

fn dummy_operation() -> ags::protocol::catalogue::OperationSchema {
    ags::protocol::catalogue::OperationSchema {
        id: ags::protocol::catalogue::OperationId::new("AdminListUsers"),
        name: "list".to_string(),
        summary: "List users".to_string(),
        description: None,
        mutation_class: ags::protocol::catalogue::MutationClass::ReadOnly,
        http_method: ags::protocol::catalogue::HttpMethod::Get,
        path_template: "/iam/v3/admin/namespaces/{namespace}/users".to_string(),
        parameters: vec![],
        request_body: None,
        response: None,
        permissions: vec![],
        scope: "admin".to_string(),
        api_version: ags::protocol::catalogue::ApiVersion(3),
        deprecated: false,
        response_content_type: None,
    }
}

// ── Auth output: status line before detail lines ──

/// Auth status headline prints before detail lines so the user sees the verdict immediately
#[test]
fn test_auth_not_authenticated_is_stdout_first() {
    let output = CommandOutput::Auth(AuthOutput {
        view: AuthView::NotAuthenticated {
            next_step: Some("Run 'ags auth login'.".to_string()),
            tip: Some("Set AGS_BASE_URL for non-interactive workflows.".to_string()),
        },
    });

    let rendered = render(&output, &RenderOptions::default()).unwrap();
    assert!(
        rendered.is_stdout_first,
        "Auth output must set is_stdout_first so the status headline prints before detail lines"
    );
}

/// Login success headline prints before detail lines so the user sees confirmation first
#[test]
fn test_auth_login_success_is_stdout_first() {
    let output = CommandOutput::Auth(AuthOutput {
        view: AuthView::LoginSuccess(ags::protocol::output::AuthActionData {
            status: ags::protocol::output::AuthActionStatus::LoggedIn,
            base_url: Some("https://example.com".to_string()),
            login_type: Some("client_credentials".to_string()),
            client_id: Some("cid".to_string()),
            token_expires_in_secs: Some(3600),
            tip: None,
        }),
    });

    let rendered = render(&output, &RenderOptions::default()).unwrap();
    assert!(
        rendered.is_stdout_first,
        "Login success must set is_stdout_first so the status headline prints before detail lines"
    );
}

// ── Service output: verbose trace before response data ──

/// Verbose HTTP trace prints before response data so the request context appears above the result
#[test]
fn test_service_verbose_trace_before_data() {
    let output = CommandOutput::Service(Box::new(ApiOutput {
        operation: dummy_operation(),
        resource_name: "users".to_string(),
        body: ApiBody::Shaped(Box::new(CommandResult::Raw(RawResult {
            value: serde_json::json!({"data": [], "paging": {}}),
        }))),
        success: None,
        trace: Some(ExecutionTrace {
            resolution: None,
            request: RequestTrace {
                http_method: "GET".to_string(),
                url: "https://example.com/iam/v3/admin/namespaces/test/users".to_string(),
                query_params: vec![],
                has_auth_header: true,
                body_size: None,
            },
            response: Some(ResponseTrace {
                status: 200,
                reason: Some("OK".to_string()),
                body_size: None,
            }),
        }),
    }));

    let rendered = render(&output, &RenderOptions::default()).unwrap();
    assert!(
        !rendered.is_stdout_first,
        "Service output must not set is_stdout_first so verbose trace prints before response data"
    );
}

/// Mutation success banner prints before response data so the outcome is visible before the payload
#[test]
fn test_service_mutation_success_trace_before_data() {
    let output = CommandOutput::Service(Box::new(ApiOutput {
        operation: ags::protocol::catalogue::OperationSchema {
            id: ags::protocol::catalogue::OperationId::new("AdminCreateRole"),
            name: "create".to_string(),
            summary: "Create role".to_string(),
            description: None,
            mutation_class: ags::protocol::catalogue::MutationClass::Mutating,
            http_method: ags::protocol::catalogue::HttpMethod::Post,
            path_template: "/iam/v4/admin/roles".to_string(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: "admin".to_string(),
            api_version: ags::protocol::catalogue::ApiVersion(4),
            deprecated: false,
            response_content_type: None,
        },
        resource_name: "roles".to_string(),
        body: ApiBody::Shaped(Box::new(CommandResult::Raw(RawResult {
            value: serde_json::json!({"roleId": "r1"}),
        }))),
        success: Some(ApiSuccess {
            summary: "Role created.".to_string(),
        }),
        trace: None,
    }));

    let rendered = render(&output, &RenderOptions::default()).unwrap();
    assert!(
        !rendered.is_stdout_first,
        "Service mutation must not set is_stdout_first so success banner prints before response data"
    );
}
