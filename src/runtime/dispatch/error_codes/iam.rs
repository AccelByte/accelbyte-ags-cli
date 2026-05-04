//! IAM service error codes.
//!
//! Covers user account state (`10130–10240`), OAuth client management
//! (`10364–10365`), role management (`10456–10470`), the extended
//! user-management range (`1014001–1015073`) used by some IAM endpoints,
//! and the legacy `1094028` user-mapping code.

use super::ErrorMapping;

/// Look up a curated error mapping for this service.
pub(super) fn lookup(error_code: i64) -> Option<ErrorMapping> {
    match error_code {
        // ── User account (10130–10240) ──

        10130 => Some(ErrorMapping {
            message: "User is under the minimum age.",
            reason: Some("The user does not meet the age requirement for this namespace."),
            suggestion: Some("Check the namespace age restriction settings."),
            tip: None,
        }),
        10131 => Some(ErrorMapping {
            message: "Invalid date of birth.",
            reason: Some("The date of birth format or value is invalid."),
            suggestion: Some("Provide a valid date of birth."),
            tip: None,
        }),
        10132 => Some(ErrorMapping {
            message: "Invalid email address.",
            reason: Some("The email address format is not valid."),
            suggestion: Some("Provide a valid email address."),
            tip: None,
        }),
        10133 => Some(ErrorMapping {
            message: "Email address already in use.",
            reason: Some("Another account is already registered with this email."),
            suggestion: Some("Use a different email address or recover the existing account."),
            tip: None,
        }),
        10136 => Some(ErrorMapping {
            message: "Verification code already used or invalid.",
            reason: Some("The code has been consumed or was never valid."),
            suggestion: Some("Request a new verification code and retry."),
            tip: None,
        }),
        10137 => Some(ErrorMapping {
            message: "Verification code expired.",
            reason: Some("The code is no longer valid."),
            suggestion: Some("Request a new verification code and retry."),
            tip: None,
        }),
        10138 => Some(ErrorMapping {
            message: "Verification code does not match.",
            reason: Some("The provided code does not match the expected value."),
            suggestion: Some("Check the code and retry, or request a new one."),
            tip: None,
        }),
        10139 => Some(ErrorMapping {
            message: "Platform account not found.",
            reason: Some("The specified platform account does not exist for this user."),
            suggestion: Some(
                "Run 'ags iam users get-platform-accounts --userId <id>' to see linked platforms.",
            ),
            tip: None,
        }),
        10140 => Some(ErrorMapping {
            message: "User is already verified.",
            reason: Some("The user account has already been verified."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        10141 => Some(ErrorMapping {
            message: "Email is already verified.",
            reason: Some("The email address has already been verified."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        10142 => Some(ErrorMapping {
            message: "New password cannot be the same as the current password.",
            reason: Some("The new password must differ from the existing one."),
            suggestion: Some("Choose a different password."),
            tip: None,
        }),
        10143 => Some(ErrorMapping {
            message: "Password does not match.",
            reason: Some("The provided password is incorrect."),
            suggestion: Some("Check the password and retry."),
            tip: None,
        }),
        10144 => Some(ErrorMapping {
            message: "User has no bans.",
            reason: Some("No ban records exist for this user."),
            suggestion: None,
            tip: None,
        }),
        10145 => Some(ErrorMapping {
            message: "Cannot ban a publisher user from game access.",
            reason: Some("Publisher users cannot be banned at the game namespace level."),
            suggestion: Some("Manage publisher user bans in the publisher namespace."),
            tip: None,
        }),
        10146 => Some(ErrorMapping {
            message: "User ID does not match.",
            reason: Some("The user ID in the request does not match the authenticated user."),
            suggestion: Some("Verify the user ID and retry."),
            tip: None,
        }),
        10148 => Some(ErrorMapping {
            message: "Verification code context does not match.",
            reason: Some("The verification code was issued for a different context."),
            suggestion: Some("Request a new code for this specific operation."),
            tip: None,
        }),
        10149 => Some(ErrorMapping {
            message: "Verification contact type does not match.",
            reason: Some("The code was sent to a different contact type than expected."),
            suggestion: Some("Request a new code using the correct contact method."),
            tip: None,
        }),
        10152 => Some(ErrorMapping {
            message: "Verification code not found.",
            reason: Some("No verification code exists for this request."),
            suggestion: Some("Request a new verification code."),
            tip: None,
        }),
        10153 => Some(ErrorMapping {
            message: "User already exists.",
            reason: Some("An account with these details already exists."),
            suggestion: Some("Use a different identifier or recover the existing account."),
            tip: None,
        }),
        10154 => Some(ErrorMapping {
            message: "Country not found.",
            reason: Some("The specified country code is not recognized."),
            suggestion: Some("Check the country code and retry."),
            tip: Some("Use standard ISO 3166-1 alpha-2 country codes."),
        }),
        10155 => Some(ErrorMapping {
            message: "Country is not defined.",
            reason: Some("The country is not configured for this namespace."),
            suggestion: Some("Check the namespace configuration."),
            tip: None,
        }),
        10156 => Some(ErrorMapping {
            message: "Role not found.",
            reason: Some("The specified role does not exist."),
            suggestion: Some("Run 'ags iam roles list' to see available roles."),
            tip: None,
        }),
        10157 => Some(ErrorMapping {
            message: "Not an admin role.",
            reason: Some("The specified role is not an admin role."),
            suggestion: Some("Use an admin role for this operation."),
            tip: None,
        }),
        10158 => Some(ErrorMapping {
            message: "Ban not found.",
            reason: Some("The specified ban does not exist."),
            suggestion: None,
            tip: None,
        }),
        10159 => Some(ErrorMapping {
            message: "Not a role manager.",
            reason: Some("Your account is not a manager of this role."),
            suggestion: Some("Contact a role manager to perform this operation."),
            tip: None,
        }),
        10160 => Some(ErrorMapping {
            message: "User already has this role.",
            reason: Some("The role is already assigned to this user."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        10161 => Some(ErrorMapping {
            message: "User is already a role member.",
            reason: Some("The user is already a member of this role."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        10162 => Some(ErrorMapping {
            message: "Invalid verification.",
            reason: Some("The verification attempt is invalid."),
            suggestion: Some("Request a new verification code and retry."),
            tip: None,
        }),
        10163 => Some(ErrorMapping {
            message: "Platform already linked to this account.",
            reason: Some("This platform is already associated with the user account."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        10169 => Some(ErrorMapping {
            message: "Age restriction not found.",
            reason: Some("No age restriction is configured for this namespace or country."),
            suggestion: None,
            tip: None,
        }),
        10170 => Some(ErrorMapping {
            message: "Account is already a full account.",
            reason: Some("This account has already been upgraded from headless."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        10171 => Some(ErrorMapping {
            message: "Email address not found.",
            reason: Some("No account is associated with this email address."),
            suggestion: Some("Check the email address and retry."),
            tip: None,
        }),
        10172 => Some(ErrorMapping {
            message: "Platform user already linked to this account.",
            reason: Some("This platform identity is already linked to the account."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        10173 => Some(ErrorMapping {
            message: "Platform already linked to another account.",
            reason: Some("This platform identity is linked to a different account."),
            suggestion: Some(
                "Unlink the platform from the other account first, or use a different platform identity.",
            ),
            tip: None,
        }),
        10174 => Some(ErrorMapping {
            message: "Platform client not found.",
            reason: Some("The platform client configuration does not exist."),
            suggestion: Some("Check the platform ID and namespace."),
            tip: None,
        }),
        10175 => Some(ErrorMapping {
            message: "Third-party credential not found.",
            reason: Some("The third-party platform credential does not exist."),
            suggestion: Some("Check the platform ID and configuration."),
            tip: None,
        }),
        10177 => Some(ErrorMapping {
            message: "Username already in use.",
            reason: Some("Another account is already using this username."),
            suggestion: Some("Choose a different username."),
            tip: None,
        }),
        10180 => Some(ErrorMapping {
            message: "Admin invitation not found or expired.",
            reason: Some("The invitation does not exist or has expired."),
            suggestion: Some("Request a new admin invitation."),
            tip: None,
        }),
        10182 => Some(ErrorMapping {
            message: "Namespace cannot be assigned to this role.",
            reason: Some("The specified namespace is not allowed for this role."),
            suggestion: Some("Check the role's allowed namespaces."),
            tip: None,
        }),
        10183 => Some(ErrorMapping {
            message: "Unprocessable entity.",
            reason: Some("The request could not be processed."),
            suggestion: Some("Check the request fields and retry."),
            tip: None,
        }),
        10185 => Some(ErrorMapping {
            message: "Publisher namespace not allowed.",
            reason: Some("This operation cannot be performed in a publisher namespace."),
            suggestion: Some("Use a game namespace instead."),
            tip: None,
        }),
        10188 => Some(ErrorMapping {
            message: "Input validation field not found.",
            reason: Some("The validation rule for the specified field does not exist."),
            suggestion: Some("Check the field name and retry."),
            tip: None,
        }),
        10189 => Some(ErrorMapping {
            message: "Invalid MFA factor.",
            reason: Some("The specified MFA factor is not valid."),
            suggestion: Some("Use a supported factor (e.g., email, authenticator)."),
            tip: None,
        }),
        10190 => Some(ErrorMapping {
            message: "Auth secret key expired.",
            reason: Some("The authenticator secret key has expired."),
            suggestion: Some("Generate a new secret key."),
            tip: None,
        }),
        10191 => Some(ErrorMapping {
            message: "Email address not verified.",
            reason: Some("This operation requires a verified email address."),
            suggestion: Some("Verify the email address first, then retry."),
            tip: Some("MFA operations require email verification."),
        }),
        10192 => Some(ErrorMapping {
            message: "MFA factor not enabled.",
            reason: Some("The specified MFA factor is not enabled for this account."),
            suggestion: Some("Enable the factor first, then retry."),
            tip: None,
        }),
        10193 => Some(ErrorMapping {
            message: "MFA not enabled.",
            reason: Some("Multi-factor authentication is not enabled for this account."),
            suggestion: Some("Enable MFA first."),
            tip: None,
        }),
        10194 => Some(ErrorMapping {
            message: "MFA factor already enabled.",
            reason: Some("The specified MFA factor is already active."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        10195 => Some(ErrorMapping {
            message: "No valid backup codes found.",
            reason: Some("All backup codes have been used or none were generated."),
            suggestion: Some("Generate new backup codes."),
            tip: None,
        }),
        10200 => Some(ErrorMapping {
            message: "Cannot link to a different platform account.",
            reason: Some("Changing the linked platform account is not allowed."),
            suggestion: None,
            tip: Some("The previously linked platform account is returned in the response."),
        }),
        10202 => Some(ErrorMapping {
            message: "Device ban configuration already exists.",
            reason: Some("An active device ban config already exists for this combination."),
            suggestion: None,
            tip: None,
        }),
        10204 => Some(ErrorMapping {
            message: "Device cannot be banned.",
            reason: Some("This device is not eligible for banning."),
            suggestion: None,
            tip: None,
        }),
        10207 => Some(ErrorMapping {
            message: "User namespace is not available.",
            reason: Some("The namespace for this user is not available."),
            suggestion: None,
            tip: None,
        }),
        10208 => Some(ErrorMapping {
            message: "Platform token expired.",
            reason: Some("The platform authentication token has expired."),
            suggestion: Some("Re-authenticate with the platform and retry."),
            tip: None,
        }),
        10213 => Some(ErrorMapping {
            message: "Country is blocked.",
            reason: Some("Access from this country is restricted."),
            suggestion: None,
            tip: None,
        }),
        10215 => Some(ErrorMapping {
            message: "Simultaneous ticket is required.",
            reason: Some("A simultaneous platform ticket must be provided."),
            suggestion: Some("Provide both native and simultaneous platform tickets."),
            tip: None,
        }),
        10216 => Some(ErrorMapping {
            message: "Native ticket is required.",
            reason: Some("A native platform ticket must be provided."),
            suggestion: Some("Provide both native and simultaneous platform tickets."),
            tip: None,
        }),
        10217..=10221 => Some(ErrorMapping {
            message: "Platform account linking conflict.",
            reason: Some("The platform accounts have conflicting linking histories."),
            suggestion: Some("Check existing platform links before retrying."),
            tip: None,
        }),
        10222 => Some(ErrorMapping {
            message: "Display name already in use.",
            reason: Some("Another account is using this unique display name."),
            suggestion: Some("Choose a different display name."),
            tip: None,
        }),
        10226 => Some(ErrorMapping {
            message: "Third-party platform not supported.",
            reason: Some("The specified platform is not supported."),
            suggestion: Some("Check the list of supported platforms."),
            tip: None,
        }),
        10228 => Some(ErrorMapping {
            message: "Invalid MFA token.",
            reason: Some("The MFA token is invalid or expired."),
            suggestion: Some("Request a new MFA token and retry."),
            tip: None,
        }),
        10229 => Some(ErrorMapping {
            message: "Request body exceeds size limit.",
            reason: Some("The request payload is too large."),
            suggestion: Some("Reduce the request body size and retry."),
            tip: None,
        }),
        10235 => Some(ErrorMapping {
            message: "Date of birth cannot be updated.",
            reason: Some("Updating the date of birth is not allowed."),
            suggestion: None,
            tip: None,
        }),
        10236 => Some(ErrorMapping {
            message: "Username cannot be updated.",
            reason: Some("Updating the username is not allowed."),
            suggestion: None,
            tip: None,
        }),
        10237 => Some(ErrorMapping {
            message: "Display name cannot be updated.",
            reason: Some("Updating the display name is not allowed."),
            suggestion: None,
            tip: None,
        }),
        10238 => Some(ErrorMapping {
            message: "Country cannot be updated.",
            reason: Some("Updating the country is not allowed."),
            suggestion: None,
            tip: None,
        }),
        10240 => Some(ErrorMapping {
            message: "Not a game namespace.",
            reason: Some("This operation requires a game namespace."),
            suggestion: Some("Use a game namespace, not a publisher namespace."),
            tip: None,
        }),

        // ── OAuth / client (10364–10365) ──

        10364 => Some(ErrorMapping {
            message: "Client already exists.",
            reason: Some("A client with this ID already exists."),
            suggestion: Some("Use a different client ID."),
            tip: None,
        }),
        10365 => Some(ErrorMapping {
            message: "Client not found.",
            reason: Some("The specified client does not exist."),
            suggestion: Some("Check the client ID and retry."),
            tip: None,
        }),

        // ── Role management (10456–10470) ──

        10456 => Some(ErrorMapping {
            message: "Role not found.",
            reason: Some("The specified role does not exist."),
            suggestion: Some("Run 'ags iam roles list' to see available roles."),
            tip: None,
        }),
        10457 => Some(ErrorMapping {
            message: "Not an admin role.",
            reason: Some("The specified role is not an admin role."),
            suggestion: Some("Use an admin role for this operation."),
            tip: None,
        }),
        10459 => Some(ErrorMapping {
            message: "Not a role manager.",
            reason: Some("Your account is not a manager of this role."),
            suggestion: Some("Contact a role manager to perform this operation."),
            tip: None,
        }),
        10466 => Some(ErrorMapping {
            message: "Invalid role members.",
            reason: Some("The role member list is invalid."),
            suggestion: Some("Check the member list and retry."),
            tip: None,
        }),
        10467 => Some(ErrorMapping {
            message: "Role has no manager.",
            reason: Some("The role does not have an assigned manager."),
            suggestion: Some("Assign a manager to the role first."),
            tip: None,
        }),
        10468 => Some(ErrorMapping {
            message: "Role manager already exists.",
            reason: Some("This user is already a manager of the role."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        10469 => Some(ErrorMapping {
            message: "Role member already exists.",
            reason: Some("This user is already a member of the role."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        10470 => Some(ErrorMapping {
            message: "Role is empty.",
            reason: Some("The role has no members or permissions configured."),
            suggestion: Some("Add members or permissions to the role."),
            tip: None,
        }),

        // ── User management extended (1014001–1015073) ──

        1014001 => Some(ErrorMapping {
            message: "Unable to parse request body.",
            reason: Some("The request body format is invalid."),
            suggestion: Some("Check the JSON body and retry."),
            tip: None,
        }),
        1014002 => Some(ErrorMapping {
            message: "User already exists.",
            reason: Some("An account with these details already exists."),
            suggestion: Some("Use a different identifier or recover the existing account."),
            tip: None,
        }),
        1014016 => Some(ErrorMapping {
            message: "Unable to parse request body.",
            reason: Some("The request body format is invalid."),
            suggestion: Some("Check the JSON body and retry."),
            tip: None,
        }),
        1014017 => Some(ErrorMapping {
            message: "User not found.",
            reason: Some("The user does not exist or is not accessible in this context."),
            suggestion: Some("Run 'ags iam users list' to see available users."),
            tip: None,
        }),
        1014018 => Some(ErrorMapping {
            message: "Verification code not found.",
            reason: Some("No verification code exists for this request."),
            suggestion: Some("Request a new verification code."),
            tip: None,
        }),
        1014019 => Some(ErrorMapping {
            message: "Verification code already used.",
            reason: Some("The code has already been consumed."),
            suggestion: Some("Request a new verification code and retry."),
            tip: None,
        }),
        1014020 => Some(ErrorMapping {
            message: "Verification code invalid.",
            reason: Some("The provided code is not valid."),
            suggestion: Some("Check the code and retry, or request a new one."),
            tip: None,
        }),
        1014021 => Some(ErrorMapping {
            message: "Verification code expired.",
            reason: Some("The code is no longer valid."),
            suggestion: Some("Request a new verification code and retry."),
            tip: None,
        }),
        1015073 => Some(ErrorMapping {
            message: "New password cannot be the same as the old password.",
            reason: Some("The new password must differ from the existing one."),
            suggestion: Some("Choose a different password."),
            tip: None,
        }),

        // ── Legacy ──

        1094028 => Some(ErrorMapping {
            message: "Invalid user mapping request.",
            reason: Some("The target namespace is invalid or the user has no mapping."),
            suggestion: Some(
                "Check that the target namespace exists and the user has a mapping to it.",
            ),
            tip: None,
        }),

        _ => None,
    }
}
