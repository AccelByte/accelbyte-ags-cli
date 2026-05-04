use ags::errors::SuggestionKind;
use ags::frontend::human::templates;
use ags::frontend::human::templates::render_error_text;
use ags::protocol::output::FieldEntry;

// ── Error rendering ──

/// Compact errors show the error symbol and a Fix line with no extra chrome
#[test]
fn test_compact_error_with_fix() {
    let output = render_error_text(
        "Request was not authorized.",
        None,
        None,
        Some("Run 'ags auth login'."),
        SuggestionKind::Fix,
        None,
        false,
    );
    insta::assert_snapshot!(output, @"
    ✕ Request was not authorized.
    → Fix: Run 'ags auth login'.
    ");
}

/// Expanded errors render Reason, Detail, and Fix lines in the correct order
#[test]
fn test_expanded_error_with_reason_detail_fix() {
    let output = render_error_text(
        "You do not have permission for this operation.",
        Some("client does not have the required scope"),
        Some("Error code 20013"),
        Some("Check that your account has the required role and permissions."),
        SuggestionKind::Fix,
        None,
        false,
    );
    insta::assert_snapshot!(output, @"
    ✕ You do not have permission for this operation.
        Reason: client does not have the required scope
        Detail: Error code 20013
    → Fix: Check that your account has the required role and permissions.
    ");
}

/// Errors with a tip render all four lines: Reason, Detail, Fix, and Tip
#[test]
fn test_error_with_tip() {
    let output = render_error_text(
        "Email address not verified.",
        Some("This operation requires a verified email address."),
        Some("Error code 10191"),
        Some("Verify the email address first, then retry."),
        SuggestionKind::Fix,
        Some("MFA operations require email verification."),
        false,
    );
    insta::assert_snapshot!(output, @"
    ✕ Email address not verified.
        Reason: This operation requires a verified email address.
        Detail: Error code 10191
    → Fix: Verify the email address first, then retry.
        Tip: MFA operations require email verification.
    ");
}

// ── Tip rendering ──

/// Standalone tip renders as an indented "Tip:" line with no prefix symbol
#[test]
fn test_tip_rendering() {
    let output = templates::render_tip_text("Set AGS_AUTH_TIMEOUT to change the timeout", false);
    insta::assert_snapshot!(output, @"    Tip: Set AGS_AUTH_TIMEOUT to change the timeout");
}

// ── Inspect rendering ──

/// Inspect view renders a heading, aligned field pairs, and nested sections with sub-headings
#[test]
fn test_inspect_heading_with_fields_and_section() {
    use ags::protocol::output::Section;

    let fields = vec![
        FieldEntry {
            label: "ID".to_string(),
            value: "user-001".to_string(),
        },
        FieldEntry {
            label: "Display Name".to_string(),
            value: "Jane Doe".to_string(),
        },
        FieldEntry {
            label: "Status".to_string(),
            value: "active".to_string(),
        },
    ];
    let sections = vec![Section {
        heading: "Permissions".to_string(),
        fields: vec![
            FieldEntry {
                label: "Role".to_string(),
                value: "admin".to_string(),
            },
            FieldEntry {
                label: "Scope".to_string(),
                value: "global".to_string(),
            },
        ],
    }];
    let output = templates::render_inspect_text("User: Jane Doe", &fields, &sections, false, false);
    insta::assert_snapshot!(output, @"
    › User: Jane Doe
        ID:            user-001
        Display Name:  Jane Doe
        Status:        active

        Permissions
            Role:   admin
            Scope:  global
    ");
}

// ── List rendering ──

/// List template renders a count heading, column headers, and aligned data rows
#[test]
fn test_list_output() {
    let headers = vec!["ID".to_string(), "Email".to_string()];
    let rows = vec![
        vec!["user-001".to_string(), "jane@example.com".to_string()],
        vec!["user-002".to_string(), "john@example.com".to_string()],
        vec!["user-003".to_string(), "alice@example.com".to_string()],
    ];
    let output = templates::render_list_text(3, "user", &headers, &rows, None, false, false, false);
    insta::assert_snapshot!(output, @"
    › Found 3 users
        ID        Email
        user-001  jane@example.com
        user-002  john@example.com
        user-003  alice@example.com
    ");
}

/// Already-plural nouns like "clients" are not double-pluralised in the count heading
#[test]
fn test_list_output_already_plural_noun() {
    let headers = vec!["ID".to_string()];
    let rows = vec![
        vec!["client-001".to_string()],
        vec!["client-002".to_string()],
    ];
    let output =
        templates::render_list_text(2, "clients", &headers, &rows, None, false, false, false);
    insta::assert_snapshot!(output, @"
    › Found 2 clients
        ID
        client-001
        client-002
    ");
}

// ── Pagination hints ──

/// When a total is known and --page-all is not set, a Tip points at --page-all with the total.
#[test]
fn test_list_output_pagination_total_known_tip() {
    use ags::frontend::PaginationHint;
    let headers = vec!["ID".to_string()];
    let rows = vec![vec!["role-001".to_string()], vec!["role-002".to_string()]];
    let hint = PaginationHint {
        total: Some(347),
        has_next: true,
    };
    let output =
        templates::render_list_text(2, "role", &headers, &rows, Some(hint), false, false, false);
    insta::assert_snapshot!(output, @r"
    › Showing 2 of 347 roles
        ID
        role-001
        role-002
        Tip: Use --page-all to fetch all 347 roles
    ");
}

/// When has_next is set but total is unknown and --page-all is not set, a generic tip appears.
#[test]
fn test_list_output_pagination_has_next_only_tip() {
    use ags::frontend::PaginationHint;
    let headers = vec!["ID".to_string()];
    let rows = vec![vec!["role-001".to_string()], vec!["role-002".to_string()]];
    let hint = PaginationHint {
        total: None,
        has_next: true,
    };
    let output =
        templates::render_list_text(2, "role", &headers, &rows, Some(hint), false, false, false);
    insta::assert_snapshot!(output, @r"
    › Found 2 roles (more available)
        ID
        role-001
        role-002
        Tip: Use --page-all to fetch additional pages
    ");
}

/// When --page-all is already set, the trailing tip is suppressed (the runtime emits a page-cap message through the sink instead).
#[test]
fn test_list_output_pagination_tip_suppressed_when_page_all() {
    use ags::frontend::PaginationHint;
    let headers = vec!["ID".to_string()];
    let rows = vec![vec!["role-001".to_string()], vec!["role-002".to_string()]];
    let hint = PaginationHint {
        total: Some(347),
        has_next: true,
    };
    let output =
        templates::render_list_text(2, "role", &headers, &rows, Some(hint), true, false, false);
    insta::assert_snapshot!(output, @"
    › Showing 2 of 347 roles
        ID
        role-001
        role-002
    ");
}

// ── JSON passthrough ──

/// JSON passthrough emits pretty-printed JSON with no decorations or headings
#[test]
fn test_json_passthrough() {
    let value: serde_json::Value = serde_json::json!({"id": "abc", "name": "test"});
    let output = serde_json::to_string_pretty(&value).unwrap();
    insta::assert_snapshot!(output, @r#"
    {
      "id": "abc",
      "name": "test"
    }
    "#);
}
