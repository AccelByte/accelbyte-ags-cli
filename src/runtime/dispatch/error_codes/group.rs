//! Group service error codes.

use super::ErrorMapping;

/// Look up a curated error mapping for this service.
pub(super) fn lookup(error_code: i64) -> Option<ErrorMapping> {
    match error_code {
        73034 => Some(ErrorMapping {
            message: "User does not belong to any group.",
            reason: Some("The user is not a member of any group in this namespace."),
            suggestion: Some("Have the user join a group first."),
            tip: None,
        }),
        73036 => Some(ErrorMapping {
            message: "Insufficient member role permission.",
            reason: Some("The user's group role does not permit this operation."),
            suggestion: Some("Have a group admin or higher-privileged role perform this action."),
            tip: None,
        }),
        73130 => Some(ErrorMapping {
            message: "Global group configuration already exists.",
            reason: Some("A global configuration is already set for this namespace."),
            suggestion: Some("Update the existing configuration instead of creating a new one."),
            tip: None,
        }),
        73131 => Some(ErrorMapping {
            message: "Global group configuration not found.",
            reason: Some("No global configuration is set for this namespace."),
            suggestion: Some("Initialise the global configuration first."),
            tip: None,
        }),
        73232 => Some(ErrorMapping {
            message: "Group member role not found.",
            reason: Some("The specified member role does not exist."),
            suggestion: Some("Run 'ags group member-roles list' to see available roles."),
            tip: None,
        }),
        73333 => Some(ErrorMapping {
            message: "Group not found.",
            reason: Some("The specified group does not exist in this namespace."),
            suggestion: Some("Run 'ags group groups list' to see available groups."),
            tip: None,
        }),
        73342 => Some(ErrorMapping {
            message: "User has already joined this group.",
            reason: Some("The user is already a member of the group."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        73433 => Some(ErrorMapping {
            message: "Group member not found.",
            reason: Some("The user is not a member of the requested group."),
            suggestion: Some("Verify the user ID and group, then retry."),
            tip: None,
        }),
        73437 => Some(ErrorMapping {
            message: "User has already been invited.",
            reason: Some("An invitation for this user is already pending."),
            suggestion: None,
            tip: Some("Wait for the user to accept or decline the existing invitation."),
        }),
        73438 => Some(ErrorMapping {
            message: "User has already requested to join.",
            reason: Some("A join request from this user is already pending."),
            suggestion: None,
            tip: Some("Approve or decline the existing request."),
        }),
        73440 => Some(ErrorMapping {
            message: "Group admin cannot leave the group.",
            reason: Some("Admins must transfer ownership or disband before leaving."),
            suggestion: Some("Promote another member to admin first, or delete the group."),
            tip: None,
        }),
        73442 => Some(ErrorMapping {
            message: "User is already a member of another group.",
            reason: Some("The user can only belong to one group at a time."),
            suggestion: Some("Have the user leave their current group before joining a new one."),
            tip: None,
        }),
        73443 => Some(ErrorMapping {
            message: "Group join request not found.",
            reason: Some("No pending join request matches this user and group."),
            suggestion: Some("Verify the user ID and group, then retry."),
            tip: None,
        }),
        73444 => Some(ErrorMapping {
            message: "Group member must have a role.",
            reason: Some("Members cannot exist without an assigned role."),
            suggestion: Some("Assign a role when adding the member."),
            tip: None,
        }),
        _ => None,
    }
}
