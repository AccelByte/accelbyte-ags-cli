//! Cloudsave service error codes.
//!
//! Cloudsave's error space is large but highly redundant: dozens of codes
//! correspond to the same underlying condition (record-not-found,
//! record-decode-failed, request-too-large, etc.) on different endpoints.
//! Codes that share a meaning are grouped under a single arm here.

use super::ErrorMapping;

/// Look up a curated error mapping for this service.
pub(super) fn lookup(error_code: i64) -> Option<ErrorMapping> {
    match error_code {
        // ── Record not found (404) ──
        18003 | 18022 | 18081 | 18122 | 18133 | 18140 | 18152 | 18167 | 18171 | 18177 | 18186
        | 18303 | 18313 | 18317 | 18322 | 18325 | 18333 | 18338 | 18340 | 18361 => {
            Some(ErrorMapping {
                message: "Cloudsave record not found.",
                reason: Some("No record exists for the supplied key, namespace, or user."),
                suggestion: Some("Verify the key/namespace/user and retry."),
                tip: None,
            })
        }

        // ── Invalid input (400) ──
        18011 | 18030 | 18050 | 18060 | 18083 | 18090 | 18100 | 18113 | 18125 | 18128 | 18134
        | 18144 | 18149 | 18150 | 18156 | 18159 | 18168 | 18174 | 18184 | 18304 | 18305 | 18311
        | 18316 | 18326 | 18327 | 18332 | 18342 | 18347 | 18350 | 18353 | 18408 => {
            Some(ErrorMapping {
                message: "Cloudsave request is invalid.",
                reason: Some("The request body or parameters did not pass validation."),
                suggestion: Some("Check the request payload and retry."),
                tip: None,
            })
        }

        // ── Validation errors (400) ──
        18064 | 18102 | 18181 => Some(ErrorMapping {
            message: "Cloudsave validation failed.",
            reason: Some("The record contents did not pass schema validation."),
            suggestion: Some("Check the record against the configured schema and retry."),
            tip: None,
        }),

        // ── Request size limits (400) ──
        18015 | 18052 | 18356 => Some(ErrorMapping {
            message: "Request exceeds the allowed size limit.",
            reason: Some("The total request payload is larger than Cloudsave permits."),
            suggestion: Some("Reduce the payload size and retry."),
            tip: None,
        }),
        18136 | 18146 => Some(ErrorMapping {
            message: "Record exceeds the allowed size limit.",
            reason: Some("The individual record payload is larger than Cloudsave permits."),
            suggestion: Some("Reduce the record size and retry."),
            tip: None,
        }),
        18126 | 18129 | 18169 | 18175 | 18351 => Some(ErrorMapping {
            message: "Record key list exceeds the size limit.",
            reason: Some("The list of keys in this bulk request is too long."),
            suggestion: Some("Split the keys across multiple requests."),
            tip: None,
        }),
        18354 => Some(ErrorMapping {
            message: "Too many records in a single request.",
            reason: Some("The request exceeds the per-call record limit."),
            suggestion: Some("Reduce the number of records per request and retry."),
            tip: None,
        }),

        // ── Optimistic concurrency (412) ──
        18056 | 18066 | 18103 | 18180 | 18183 => Some(ErrorMapping {
            message: "Cloudsave record changed since you fetched it.",
            reason: Some("The record was modified concurrently by another caller."),
            suggestion: Some("Fetch the latest version and retry your update."),
            tip: None,
        }),

        // ── Forbidden access to another user's record (403) ──
        18023 | 18035 | 18063 | 18072 => Some(ErrorMapping {
            message: "Cannot access another user's record.",
            reason: Some("This action is not permitted on records belonging to other users."),
            suggestion: Some(
                "Use a record key that belongs to the requesting user, or use the admin endpoint.",
            ),
            tip: None,
        }),

        // ── Conflict (409) ──
        18309 | 18330 => Some(ErrorMapping {
            message: "Cloudsave key already exists.",
            reason: Some("A record with this key already exists in this namespace."),
            suggestion: Some("Use a different key, or update the existing record instead."),
            tip: None,
        }),

        // ── Server-side failures (500) ──
        18001 | 18020 | 18080 | 18084 | 18124 | 18130 | 18139 | 18151 | 18170 | 18176 | 18185
        | 18301 | 18312 | 18323 | 18339 | 18343 | 18349 => Some(ErrorMapping {
            message: "Cloudsave was unable to read the record.",
            reason: Some("The server failed to retrieve the requested record."),
            suggestion: Some("Retry the command."),
            tip: Some("If this persists, contact AccelByte support."),
        }),
        18013 | 18033 | 18091 | 18307 | 18328 => Some(ErrorMapping {
            message: "Cloudsave was unable to save the record.",
            reason: Some("The server failed to persist the record."),
            suggestion: Some("Retry the command."),
            tip: Some("If this persists, contact AccelByte support."),
        }),
        18053 | 18061 | 18065 | 18101 | 18147 | 18182 | 18318 | 18334 | 18362 => {
            Some(ErrorMapping {
                message: "Cloudsave was unable to update the record.",
                reason: Some("The server failed to apply the update."),
                suggestion: Some("Retry the command."),
                tip: Some("If this persists, contact AccelByte support."),
            })
        }
        18040 | 18070 | 18120 | 18142 | 18154 | 18320 | 18336 => Some(ErrorMapping {
            message: "Cloudsave was unable to delete the record.",
            reason: Some("The server failed to delete the record."),
            suggestion: Some("Retry the command."),
            tip: Some("If this persists, contact AccelByte support."),
        }),
        18012 | 18051 | 18135 | 18145 | 18355 => Some(ErrorMapping {
            message: "Cloudsave was unable to encode the record.",
            reason: Some("The server could not serialise the record payload."),
            suggestion: Some("Check that the record contents are valid JSON and retry."),
            tip: None,
        }),
        18005 | 18006 | 18131 | 18138 | 18157 | 18162 | 18163 | 18164 | 18165 | 18172 | 18178
        | 18187 => Some(ErrorMapping {
            message: "Cloudsave was unable to decode the record.",
            reason: Some("The stored record could not be parsed."),
            suggestion: Some("The record may be corrupt — inspect or re-create it."),
            tip: Some("If this persists, contact AccelByte support."),
        }),
        18004 | 18114 | 18160 | 18345 => Some(ErrorMapping {
            message: "Cloudsave was unable to list records.",
            reason: Some("The server failed to retrieve the record list."),
            suggestion: Some("Retry the command."),
            tip: Some("If this persists, contact AccelByte support."),
        }),
        18310 | 18314 | 18331 => Some(ErrorMapping {
            message: "Cloudsave was unable to issue a presigned URL.",
            reason: Some("The server failed to generate a presigned upload/download URL."),
            suggestion: Some("Retry the command."),
            tip: Some("If this persists, contact AccelByte support."),
        }),

        // ── Plugins (18401–18409) ──
        18401 => Some(ErrorMapping {
            message: "Plugin configuration is invalid.",
            reason: Some("The supplied plugin configuration did not pass validation."),
            suggestion: Some("Check the plugin payload and retry."),
            tip: None,
        }),
        18402 => Some(ErrorMapping {
            message: "Plugin already configured.",
            reason: Some("A plugin configuration already exists for this namespace."),
            suggestion: Some("Update the existing configuration instead of creating a new one."),
            tip: None,
        }),
        18404 | 18406 | 18409 => Some(ErrorMapping {
            message: "Cloudsave plugin configuration not found.",
            reason: Some("No plugin configuration exists for this namespace."),
            suggestion: Some("Configure the plugin first, then retry."),
            tip: None,
        }),

        // ── Tags (18502–18510) ──
        18502 => Some(ErrorMapping {
            message: "Cloudsave was unable to list tags.",
            reason: Some("The server failed to retrieve the tag list."),
            suggestion: Some("Retry the command."),
            tip: Some("If this persists, contact AccelByte support."),
        }),
        18503 => Some(ErrorMapping {
            message: "Tag list request is invalid.",
            reason: Some("The tag listing parameters did not pass validation."),
            suggestion: Some("Check the request parameters and retry."),
            tip: None,
        }),
        18505 => Some(ErrorMapping {
            message: "Tag request is invalid.",
            reason: Some("The supplied tag did not pass validation."),
            suggestion: Some("Check the tag value and retry."),
            tip: None,
        }),
        18506 => Some(ErrorMapping {
            message: "Tag already exists.",
            reason: Some("A tag with this name is already configured."),
            suggestion: Some("Use a different tag name."),
            tip: None,
        }),
        18507 => Some(ErrorMapping {
            message: "Cloudsave was unable to create the tag.",
            reason: Some("The server failed to persist the tag."),
            suggestion: Some("Retry the command."),
            tip: Some("If this persists, contact AccelByte support."),
        }),
        18509 => Some(ErrorMapping {
            message: "Cloudsave was unable to delete the tag.",
            reason: Some("The server failed to delete the tag."),
            suggestion: Some("Retry the command."),
            tip: Some("If this persists, contact AccelByte support."),
        }),
        18510 => Some(ErrorMapping {
            message: "Tag not found.",
            reason: Some("No tag exists with the supplied name."),
            suggestion: Some("Run 'ags cloudsave tags list' to see configured tags."),
            tip: None,
        }),

        // ── Misc ──
        18201 => Some(ErrorMapping {
            message: "Invalid record operator.",
            reason: Some("The supplied record operator is not recognised."),
            suggestion: Some("Use a supported operator (e.g., $set, $inc)."),
            tip: None,
        }),
        _ => None,
    }
}
