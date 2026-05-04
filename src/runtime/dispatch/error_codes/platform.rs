//! Platform / commerce service error codes.
//!
//! Platform owns the largest error-code surface — store, catalog, items,
//! categories, entitlements, orders, payments, rewards, wallets, currencies,
//! IAP, subscriptions, fulfillment, DLC, and ticket booths. Codes are
//! organised by domain. Codes shared with the season pass service
//! (`30141`, `30142`, `30341`, `36141`, `49147`, `49183–49187`) are owned
//! by [`super::seasonpass`]; this module does not re-define them.

use super::ErrorMapping;

/// Look up a curated error mapping for this service.
pub(super) fn lookup(error_code: i64) -> Option<ErrorMapping> {
    match error_code {
        // ── Standard codes with platform-specific meaning ──
        20016 => Some(ErrorMapping {
            message: "Action is banned for this user.",
            reason: Some("This user is banned from performing the action."),
            suggestion: Some("Check the user's ban status before retrying."),
            tip: None,
        }),
        20024 => Some(ErrorMapping {
            message: "Insufficient inventory capacity.",
            reason: Some("The user has reached the maximum number of inventory slots."),
            suggestion: Some("Free up inventory slots, or raise the per-user slot cap in the namespace config."),
            tip: None,
        }),
        20027 => Some(ErrorMapping {
            message: "Invalid time range.",
            reason: Some("The supplied start/end time range is not valid."),
            suggestion: Some("Ensure the start time precedes the end time and both are in ISO-8601 format."),
            tip: None,
        }),

        // ── Catalog config (30021–30025) ──
        30021 => Some(ErrorMapping {
            message: "Default language is required.",
            reason: Some("The default language is missing from the catalog configuration."),
            suggestion: Some("Set a default language and retry."),
            tip: None,
        }),
        30022 => Some(ErrorMapping {
            message: "Default region is required.",
            reason: Some("The default region is missing from the catalog configuration."),
            suggestion: Some("Set a default region and retry."),
            tip: None,
        }),
        30023 => Some(ErrorMapping {
            message: "Catalog plugin gRPC server address is required.",
            reason: Some("The plugin endpoint is not configured."),
            suggestion: Some("Provide a gRPC server address for the catalog plugin."),
            tip: None,
        }),
        30024 => Some(ErrorMapping {
            message: "Unable to parse CSV cell.",
            reason: Some("A cell in the imported CSV could not be parsed."),
            suggestion: Some("Check the offending cell and retry the import."),
            tip: None,
        }),
        30025 => Some(ErrorMapping {
            message: "Required CSV header is missing.",
            reason: Some("The CSV import is missing a header required for this catalog type."),
            suggestion: Some("Add the required header column and retry the import."),
            tip: None,
        }),

        // ── Changelog ──
        30041 => Some(ErrorMapping {
            message: "Changelog not found.",
            reason: Some("The specified changelog does not exist in this namespace."),
            suggestion: Some("Verify the changelog ID and retry."),
            tip: None,
        }),

        // ── Catalog selection conflicts (30071–30076) ──
        30071 => Some(ErrorMapping {
            message: "Cannot unselect item — bound item is selected.",
            reason: Some("Another item bound to this one is currently selected in the catalog."),
            suggestion: Some("Unselect the bound item first."),
            tip: None,
        }),
        30072 => Some(ErrorMapping {
            message: "Cannot unselect category — items in it are selected.",
            reason: Some("The category contains selected items."),
            suggestion: Some("Unselect items in this category before unselecting the category."),
            tip: None,
        }),
        30073 => Some(ErrorMapping {
            message: "Cannot unselect store change.",
            reason: Some("This store change is required by other selected changes."),
            suggestion: Some("Unselect the dependent changes first."),
            tip: None,
        }),
        30074 => Some(ErrorMapping {
            message: "Cannot unselect subscription content while the subscription is selected.",
            reason: Some("Subscription content is bound to a selected subscription."),
            suggestion: Some("Unselect the parent subscription first."),
            tip: None,
        }),
        30076 => Some(ErrorMapping {
            message: "CSV header is not supported for this catalog type.",
            reason: Some("The header is not valid for the supplied catalog type."),
            suggestion: Some("Remove the unsupported header and retry the import."),
            tip: None,
        }),

        // ── Store (30121–30175) ──
        30121 => Some(ErrorMapping {
            message: "Store data is invalid.",
            reason: Some("The supplied store payload did not pass validation."),
            suggestion: Some("Check the store payload and retry."),
            tip: None,
        }),
        30122 => Some(ErrorMapping {
            message: "Item not found.",
            reason: Some("The specified item does not exist in this namespace."),
            suggestion: Some("Verify the item ID and retry."),
            tip: None,
        }),
        30143 => Some(ErrorMapping {
            message: "Published store backup not found.",
            reason: Some("No backup exists for the requested published store."),
            suggestion: Some("Verify the store ID and retry."),
            tip: None,
        }),
        30171 => Some(ErrorMapping {
            message: "Store default language cannot be changed.",
            reason: Some("Changing the default language is not allowed for this store."),
            suggestion: Some("Create a new store with the desired default language."),
            tip: None,
        }),
        30172 => Some(ErrorMapping {
            message: "Store default region cannot be changed.",
            reason: Some("Changing the default region is not allowed for this store."),
            suggestion: Some("Create a new store with the desired default region."),
            tip: None,
        }),
        30173 => Some(ErrorMapping {
            message: "Published store cannot be modified.",
            reason: Some("The store is in a published state and is immutable."),
            suggestion: Some("Make changes to the draft store and republish."),
            tip: None,
        }),
        30174 => Some(ErrorMapping {
            message: "Draft store already exists.",
            reason: Some("A draft store already exists in this namespace."),
            suggestion: Some("Edit the existing draft, or publish it before creating a new draft."),
            tip: None,
        }),
        30175 => Some(ErrorMapping {
            message: "Duplicate currency code in this region.",
            reason: Some("Two prices use the same currency code for the same region."),
            suggestion: Some("Remove the duplicate currency entry."),
            tip: None,
        }),

        // ── Categories (30241–30272) ──
        30241 => Some(ErrorMapping {
            message: "Category not found.",
            reason: Some("The specified category does not exist in this namespace."),
            suggestion: Some("Run 'ags platform categories list' to see configured categories."),
            tip: None,
        }),
        30271 => Some(ErrorMapping {
            message: "Category already exists.",
            reason: Some("A category with this path is already configured."),
            suggestion: Some("Use a different path, or update the existing category."),
            tip: None,
        }),
        30272 => Some(ErrorMapping {
            message: "Category is not empty.",
            reason: Some("The category still contains items."),
            suggestion: Some("Remove or move all items in the category before deleting it."),
            tip: None,
        }),

        // ── Items (30301–30387) ──
        30301 => Some(ErrorMapping {
            message: "Unsupported item type for box item with expiration.",
            reason: Some("The contained item type cannot be used inside an expiring box."),
            suggestion: Some("Remove the unsupported item from the box, or remove the expiration."),
            tip: None,
        }),
        30321 => Some(ErrorMapping {
            message: "Invalid item discount amount.",
            reason: Some("The discount value is out of range."),
            suggestion: Some("Use a non-negative discount that does not exceed the item price."),
            tip: None,
        }),
        30322 | 30325 | 30326 | 30334 | 30339 => Some(ErrorMapping {
            message: "Item cannot be bundled.",
            reason: Some("This item type is not allowed inside a bundle."),
            suggestion: Some("Remove the item from the bundle."),
            tip: None,
        }),
        30323 => Some(ErrorMapping {
            message: "Target namespace is required.",
            reason: Some("The operation requires a target namespace."),
            suggestion: Some("Provide a target namespace and retry."),
            tip: None,
        }),
        30324 => Some(ErrorMapping {
            message: "Invalid namespace.",
            reason: Some("The supplied namespace is not valid."),
            suggestion: Some("Use a configured namespace."),
            tip: None,
        }),
        30327 => Some(ErrorMapping {
            message: "Invalid item trial price.",
            reason: Some("The trial price configuration is invalid."),
            suggestion: Some("Check the trial price values and retry."),
            tip: None,
        }),
        30329 => Some(ErrorMapping {
            message: "Invalid bundled item quantity.",
            reason: Some("The quantity for a bundled item is not valid."),
            suggestion: Some("Use a positive integer quantity."),
            tip: None,
        }),
        30330 => Some(ErrorMapping {
            message: "Invalid currency for region price.",
            reason: Some("The currency is not allowed for this region price."),
            suggestion: Some("Use a currency that is configured for the namespace and region."),
            tip: None,
        }),
        30331 => Some(ErrorMapping {
            message: "Invalid purchase condition.",
            reason: Some("The purchase condition is not valid."),
            suggestion: Some("Check the purchase condition fields and retry."),
            tip: None,
        }),
        30332 => Some(ErrorMapping {
            message: "Invalid option box item quantity.",
            reason: Some("The option box item quantity is not valid."),
            suggestion: Some("Use a positive integer quantity."),
            tip: None,
        }),
        30333 | 30338 => Some(ErrorMapping {
            message: "Item cannot be added to this container type.",
            reason: Some("The item type is not permitted inside this option box or loot box."),
            suggestion: Some("Remove the unsupported item from the container."),
            tip: None,
        }),
        30335 => Some(ErrorMapping {
            message: "Published item cannot be deleted in non-forced mode.",
            reason: Some("Deleting a published item requires the force flag."),
            suggestion: Some("Use --force to delete a published item, or unpublish it first."),
            tip: None,
        }),
        30336 => Some(ErrorMapping {
            message: "Item type does not support this operation.",
            reason: Some("The requested operation is not valid for this item type."),
            suggestion: Some("Use a supported item type, or perform a different operation."),
            tip: None,
        }),
        30337 => Some(ErrorMapping {
            message: "Invalid loot box item quantity.",
            reason: Some("The loot box item quantity is not valid."),
            suggestion: Some("Use a positive integer quantity."),
            tip: None,
        }),
        30342 => Some(ErrorMapping {
            message: "Item with this app ID does not exist.",
            reason: Some("No item maps to the supplied appId."),
            suggestion: Some("Verify the app ID and retry."),
            tip: None,
        }),
        30343 => Some(ErrorMapping {
            message: "Item with this SKU does not exist.",
            reason: Some("No item maps to the supplied SKU."),
            suggestion: Some("Verify the SKU and retry."),
            tip: None,
        }),
        30371 => Some(ErrorMapping {
            message: "Item type config already exists.",
            reason: Some("A config already exists for this item type and class."),
            suggestion: Some("Update the existing config instead of creating a new one."),
            tip: None,
        }),
        30372 => Some(ErrorMapping {
            message: "Item type is not updatable.",
            reason: Some("The item type cannot be modified once created."),
            suggestion: None,
            tip: None,
        }),
        30373 => Some(ErrorMapping {
            message: "Item type is not allowed in this namespace.",
            reason: Some("The item type is not enabled for the requested namespace."),
            suggestion: Some("Enable the item type in the namespace configuration first."),
            tip: None,
        }),
        30374 => Some(ErrorMapping {
            message: "Item SKU already exists.",
            reason: Some("Another item already uses this SKU."),
            suggestion: Some("Choose a unique SKU."),
            tip: None,
        }),
        30375 => Some(ErrorMapping {
            message: "Item SKU conflicts with an unpublished deleted item.",
            reason: Some("A deleted draft item still owns this SKU until the change is published."),
            suggestion: Some("Publish the deletion before reusing the SKU."),
            tip: None,
        }),
        30376 => Some(ErrorMapping {
            message: "Sellback is not allowed in publisher namespaces.",
            reason: Some("Items in a publisher namespace cannot be sold back."),
            suggestion: Some("Configure sellback in a game namespace."),
            tip: None,
        }),
        30377 => Some(ErrorMapping {
            message: "Sellback is not allowed for this item type.",
            reason: Some("This item type does not permit sellback."),
            suggestion: None,
            tip: None,
        }),
        30378 => Some(ErrorMapping {
            message: "Sale price cannot use real currency.",
            reason: Some("Sale prices must use virtual currency for this item."),
            suggestion: Some("Use a virtual currency for the sale price."),
            tip: None,
        }),
        30379 => Some(ErrorMapping {
            message: "Item SKU is not updatable.",
            reason: Some("Changing an item's SKU is not allowed."),
            suggestion: None,
            tip: None,
        }),
        30380 => Some(ErrorMapping {
            message: "Box item cannot have both duration and end date.",
            reason: Some("Set either a duration or an explicit end date, not both."),
            suggestion: Some("Use one of duration or endDate."),
            tip: None,
        }),
        30381 => Some(ErrorMapping {
            message: "Currency is not configured for this region.",
            reason: Some("The bundle item is missing a price for this currency/region pair."),
            suggestion: Some("Add the missing region price to the bundle item."),
            tip: None,
        }),
        30382 => Some(ErrorMapping {
            message: "Duplicate item SKU.",
            reason: Some("The same SKU is used by more than one item in the request."),
            suggestion: Some("Ensure SKUs are unique within the payload."),
            tip: None,
        }),
        30383 => Some(ErrorMapping {
            message: "Item app ID already exists.",
            reason: Some("Another item already uses this app ID in the namespace."),
            suggestion: Some("Choose a unique app ID."),
            tip: None,
        }),
        30386 | 30387 => Some(ErrorMapping {
            message: "Item is referenced by other resources.",
            reason: Some("The item is in use by bundles, rewards, or other items."),
            suggestion: Some("Remove the references before deleting or disabling the item."),
            tip: None,
        }),

        // ── Item type config / view / section (30541–30772) ──
        30541 => Some(ErrorMapping {
            message: "Item type config not found.",
            reason: Some("The specified item type config does not exist."),
            suggestion: Some("Verify the config ID and retry."),
            tip: None,
        }),
        30641 => Some(ErrorMapping {
            message: "View not found.",
            reason: Some("The specified view does not exist in this namespace."),
            suggestion: Some("Run 'ags platform views list' to see configured views."),
            tip: None,
        }),
        30741 => Some(ErrorMapping {
            message: "Section not found.",
            reason: Some("The specified section does not exist in this namespace."),
            suggestion: Some("Run 'ags platform sections list' to see configured sections."),
            tip: None,
        }),
        30771 => Some(ErrorMapping {
            message: "Item not found in user section.",
            reason: Some("The item is not part of the user's section."),
            suggestion: Some("Verify the item, section, and user."),
            tip: None,
        }),
        30772 => Some(ErrorMapping {
            message: "Section is not available or has expired.",
            reason: Some("The section is outside its active time window."),
            suggestion: Some("Use a section that is currently active."),
            tip: None,
        }),

        // ── Entitlements (31121–31185) ──
        31121 => Some(ErrorMapping {
            message: "OptionBox entitlement use count must be 1.",
            reason: Some("OptionBox entitlements support only a single use."),
            suggestion: Some("Use a use count of 1 for this entitlement."),
            tip: None,
        }),
        31122 => Some(ErrorMapping {
            message: "OptionBox entitlement options size must be 1.",
            reason: Some("OptionBox entitlements support only a single option."),
            suggestion: Some("Provide exactly one option."),
            tip: None,
        }),
        31123 => Some(ErrorMapping {
            message: "Box item has expired.",
            reason: Some("The box item is past its end date and cannot be opened."),
            suggestion: None,
            tip: None,
        }),
        31141 => Some(ErrorMapping {
            message: "Entitlement not found.",
            reason: Some("The specified entitlement does not exist."),
            suggestion: Some("Verify the entitlement ID and retry."),
            tip: None,
        }),
        31142..=31144 => Some(ErrorMapping {
            message: "Entitlement not found.",
            reason: Some("No entitlement matches the supplied lookup key."),
            suggestion: Some("Verify the appId, SKU, or itemId and retry."),
            tip: None,
        }),
        31145 => Some(ErrorMapping {
            message: "Option does not exist in this OptionBox entitlement.",
            reason: Some("The supplied option is not configured for this entitlement."),
            suggestion: Some("Use one of the configured options."),
            tip: None,
        }),
        31147 => Some(ErrorMapping {
            message: "Required platform origin is missing.",
            reason: Some("The allowed platform origins must include both Steam and System."),
            suggestion: Some("Update the allowed platform origins to include Steam and System."),
            tip: None,
        }),
        31171 => Some(ErrorMapping {
            message: "Entitlement is already revoked.",
            reason: Some("This entitlement has previously been revoked."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        31172 => Some(ErrorMapping {
            message: "Entitlement is not active.",
            reason: Some("The entitlement is in an inactive state."),
            suggestion: Some("Activate the entitlement before retrying."),
            tip: None,
        }),
        31173 => Some(ErrorMapping {
            message: "Entitlement is not consumable.",
            reason: Some("This entitlement type cannot be consumed."),
            suggestion: None,
            tip: None,
        }),
        31174 => Some(ErrorMapping {
            message: "Entitlement is already consumed.",
            reason: Some("This entitlement has previously been consumed."),
            suggestion: None,
            tip: None,
        }),
        31176 => Some(ErrorMapping {
            message: "Entitlement use count is insufficient.",
            reason: Some("Not enough uses remain on this entitlement."),
            suggestion: Some("Use fewer units, or grant additional uses."),
            tip: None,
        }),
        31177 => Some(ErrorMapping {
            message: "Permanent item is already owned.",
            reason: Some("The user already owns this permanent item."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        31178 => Some(ErrorMapping {
            message: "Entitlement is out of its valid time range.",
            reason: Some("The entitlement is outside its activation window."),
            suggestion: None,
            tip: None,
        }),
        31179 => Some(ErrorMapping {
            message: "Duplicate entitlement.",
            reason: Some("An equivalent entitlement already exists for this user."),
            suggestion: None,
            tip: None,
        }),
        31180 => Some(ErrorMapping {
            message: "Duplicate request ID.",
            reason: Some("A request with this ID has already been processed."),
            suggestion: Some("Use a fresh request ID for new operations."),
            tip: None,
        }),
        31181 => Some(ErrorMapping {
            message: "Entitlement is not sellable.",
            reason: Some("This entitlement cannot be sold back."),
            suggestion: None,
            tip: None,
        }),
        31182 => Some(ErrorMapping {
            message: "Entitlement is already sold.",
            reason: Some("The entitlement has previously been sold back."),
            suggestion: None,
            tip: None,
        }),
        31183 => Some(ErrorMapping {
            message: "Entitlement origin operation not allowed on this platform.",
            reason: Some("The operation is not permitted for the entitlement's platform of origin."),
            suggestion: None,
            tip: None,
        }),
        31184 => Some(ErrorMapping {
            message: "Source and target entitlements must match.",
            reason: Some(
                "Source and target entitlements must share the same collectionId, time range, origin, and itemId.",
            ),
            suggestion: Some("Choose entitlements that share these attributes."),
            tip: None,
        }),
        31185 => Some(ErrorMapping {
            message: "Source and target entitlements cannot be the same.",
            reason: Some("The source and target must be different entitlements."),
            suggestion: None,
            tip: None,
        }),

        // ── Orders (32121–32178) ──
        32121 => Some(ErrorMapping {
            message: "Order price mismatch.",
            reason: Some("The supplied price does not match the catalog price."),
            suggestion: Some("Refresh the catalog and retry with the current price."),
            tip: None,
        }),
        32122 => Some(ErrorMapping {
            message: "Item type does not support this operation.",
            reason: Some("This order operation is not valid for the item type."),
            suggestion: None,
            tip: None,
        }),
        32123 => Some(ErrorMapping {
            message: "Item is not purchasable.",
            reason: Some("The item is configured as non-purchasable."),
            suggestion: None,
            tip: None,
        }),
        32124 => Some(ErrorMapping {
            message: "Invalid currency namespace.",
            reason: Some("The currency is not configured for this namespace."),
            suggestion: Some("Use a currency configured for the namespace."),
            tip: None,
        }),
        32125 => Some(ErrorMapping {
            message: "User does not meet purchase conditions.",
            reason: Some("The user fails one or more configured purchase conditions."),
            suggestion: Some("Check the item's purchase conditions against the user's state."),
            tip: None,
        }),
        32126 => Some(ErrorMapping {
            message: "Section ID is required.",
            reason: Some("Placing this order requires a section ID."),
            suggestion: Some("Include the section ID and retry."),
            tip: None,
        }),
        32127 => Some(ErrorMapping {
            message: "Discount code cannot be used on this item.",
            reason: Some("This discount code is not applicable to the supplied item."),
            suggestion: Some("Check the code's eligibility rules, or use a different code."),
            tip: None,
        }),
        32128 => Some(ErrorMapping {
            message: "Discount code cannot be combined.",
            reason: Some("This code is not stackable with other discount codes."),
            suggestion: Some("Use this code on its own, or use a different combinable code."),
            tip: None,
        }),
        32129 => Some(ErrorMapping {
            message: "Discount code cannot be applied to a free order.",
            reason: Some("Free orders are not eligible for discount codes."),
            suggestion: None,
            tip: None,
        }),
        32130 => Some(ErrorMapping {
            message: "Total discount exceeds the order price.",
            reason: Some("The combined discounts would push the order below zero."),
            suggestion: Some("Reduce the discount amount or remove a discount."),
            tip: None,
        }),
        32141 => Some(ErrorMapping {
            message: "Order not found.",
            reason: Some("The specified order does not exist."),
            suggestion: Some("Verify the order number and retry."),
            tip: None,
        }),
        32171 => Some(ErrorMapping {
            message: "Order is not refundable.",
            reason: Some("The order is in a state that does not permit refund."),
            suggestion: None,
            tip: None,
        }),
        32172 => Some(ErrorMapping {
            message: "Invalid order status for this operation.",
            reason: Some("The order's current status does not permit this action."),
            suggestion: Some("Check the order status before retrying."),
            tip: None,
        }),
        32173 => Some(ErrorMapping {
            message: "Order receipt is not downloadable.",
            reason: Some("The receipt is not available for this order."),
            suggestion: None,
            tip: None,
        }),
        32175 | 32176 => Some(ErrorMapping {
            message: "Item purchase limit reached.",
            reason: Some("The maximum purchase count for this item has been reached."),
            suggestion: None,
            tip: None,
        }),
        32177 => Some(ErrorMapping {
            message: "Order cannot be cancelled.",
            reason: Some("The order's current state does not permit cancellation."),
            suggestion: None,
            tip: None,
        }),
        32178 => Some(ErrorMapping {
            message: "All durable items in this bundle are already owned.",
            reason: Some("The user already owns every durable item in this flexible bundle."),
            suggestion: None,
            tip: None,
        }),

        // ── Payments (33045–33333) ──
        33045 => Some(ErrorMapping {
            message: "Payment merchant config not found.",
            reason: Some("No merchant config is set for this namespace."),
            suggestion: Some("Configure the payment merchant first."),
            tip: None,
        }),
        33121 => Some(ErrorMapping {
            message: "Recurring payment failed at the provider.",
            reason: Some("The payment provider returned an error during the recurring charge."),
            suggestion: Some("Check the provider's response and retry the payment."),
            tip: None,
        }),
        33122 => Some(ErrorMapping {
            message: "Subscription mismatch when creating payment order.",
            reason: Some("The supplied subscription does not match the payment order."),
            suggestion: Some("Use the matching subscription ID."),
            tip: None,
        }),
        33123 => Some(ErrorMapping {
            message: "Invalid zip code.",
            reason: Some("The supplied billing zip code is not valid."),
            suggestion: Some("Provide a valid zip/postal code and retry."),
            tip: None,
        }),
        33141 => Some(ErrorMapping {
            message: "Payment order not found.",
            reason: Some("The specified payment order does not exist."),
            suggestion: Some("Verify the payment order number and retry."),
            tip: None,
        }),
        33145 => Some(ErrorMapping {
            message: "Recurring token not found.",
            reason: Some("No recurring token exists for this subscription."),
            suggestion: Some("Configure a recurring token for the subscription."),
            tip: None,
        }),
        33171 => Some(ErrorMapping {
            message: "Invalid payment order status for this operation.",
            reason: Some("The payment order's current status does not permit this action."),
            suggestion: None,
            tip: None,
        }),
        33172 => Some(ErrorMapping {
            message: "Payment order is not refundable.",
            reason: Some("The payment order is in a state that does not permit refund."),
            suggestion: None,
            tip: None,
        }),
        33173 => Some(ErrorMapping {
            message: "External order number already exists.",
            reason: Some("Another payment order already uses this extOrderNo in this namespace."),
            suggestion: Some("Use a unique external order number."),
            tip: None,
        }),
        33221 => Some(ErrorMapping {
            message: "Failed to update payment provider config.",
            reason: Some("The provider rejected the updated configuration."),
            suggestion: Some("Inspect the error message and retry with corrected values."),
            tip: None,
        }),
        33241 => Some(ErrorMapping {
            message: "Payment provider config not found.",
            reason: Some("No payment provider config exists with this ID."),
            suggestion: Some("Verify the provider config ID and retry."),
            tip: None,
        }),
        33242 => Some(ErrorMapping {
            message: "Payment merchant config not found.",
            reason: Some("No merchant config exists with this ID."),
            suggestion: Some("Verify the merchant config ID and retry."),
            tip: None,
        }),
        33243 => Some(ErrorMapping {
            message: "Payment callback config not found.",
            reason: Some("No callback config is set for this namespace."),
            suggestion: Some("Configure a payment callback for this namespace."),
            tip: None,
        }),
        33271 => Some(ErrorMapping {
            message: "Payment provider config already exists.",
            reason: Some("A provider config already exists for this namespace and region."),
            suggestion: Some("Update the existing config or remove it before creating a new one."),
            tip: None,
        }),
        33321 => Some(ErrorMapping {
            message: "Payment provider does not support this currency.",
            reason: Some("The provider has no support for the requested currency."),
            suggestion: Some("Use a supported currency or a different provider."),
            tip: None,
        }),
        33322 => Some(ErrorMapping {
            message: "Payment provider is not supported.",
            reason: Some("The supplied provider is not configured."),
            suggestion: Some("Use one of the configured payment providers."),
            tip: None,
        }),
        33332 => Some(ErrorMapping {
            message: "Payment amount is too small.",
            reason: Some("The amount is below the provider's minimum."),
            suggestion: Some("Increase the amount, or contact the administrator to lower the minimum."),
            tip: None,
        }),
        33333 => Some(ErrorMapping {
            message: "Neon Pay checkout failed.",
            reason: Some("The Neon Pay provider rejected the checkout."),
            suggestion: Some("Inspect the provider error message and retry."),
            tip: None,
        }),

        // ── Rewards (34021–34076) ──
        34021 => Some(ErrorMapping {
            message: "Reward data is invalid.",
            reason: Some("The supplied reward payload did not pass validation."),
            suggestion: Some("Check the reward fields and retry."),
            tip: None,
        }),
        34023 | 34027 => Some(ErrorMapping {
            message: "Reward item type does not support duration or endDate.",
            reason: Some("This item type cannot be granted with a duration or end date."),
            suggestion: Some("Remove the duration/endDate, or use a supported item type."),
            tip: None,
        }),
        34041 => Some(ErrorMapping {
            message: "Reward not found.",
            reason: Some("The specified reward does not exist in this namespace."),
            suggestion: Some("Run 'ags platform rewards list' to see configured rewards."),
            tip: None,
        }),
        34042 | 34044 => Some(ErrorMapping {
            message: "Reward item not found.",
            reason: Some("The reward references an item that does not exist."),
            suggestion: Some("Verify the item ID/SKU and retry."),
            tip: None,
        }),
        34043 => Some(ErrorMapping {
            message: "Reward not found by code.",
            reason: Some("No reward exists with the supplied code."),
            suggestion: Some("Verify the reward code and retry."),
            tip: None,
        }),
        34071 => Some(ErrorMapping {
            message: "Reward code already exists.",
            reason: Some("Another reward already uses this code."),
            suggestion: Some("Choose a unique reward code."),
            tip: None,
        }),
        34072 => Some(ErrorMapping {
            message: "Duplicate reward condition.",
            reason: Some("The reward already has a condition with this name."),
            suggestion: Some("Use a unique condition name."),
            tip: None,
        }),
        34074 | 34076 => Some(ErrorMapping {
            message: "Reward item cannot have both duration and end date.",
            reason: Some("Set either a duration or an explicit end date, not both."),
            suggestion: Some("Use one of duration or endDate."),
            tip: None,
        }),

        // ── Wallets (35123–35141) ──
        35123 => Some(ErrorMapping {
            message: "Wallet is inactive.",
            reason: Some("The wallet is currently disabled."),
            suggestion: Some("Activate the wallet before performing this operation."),
            tip: None,
        }),
        35124 => Some(ErrorMapping {
            message: "Wallet has insufficient balance.",
            reason: Some("The wallet does not have enough balance for this operation."),
            suggestion: Some("Top up the wallet or reduce the requested amount."),
            tip: None,
        }),
        35141 => Some(ErrorMapping {
            message: "Wallet not found.",
            reason: Some("The specified wallet does not exist."),
            suggestion: Some("Verify the wallet ID and retry."),
            tip: None,
        }),

        // ── Currencies (36171–36172) ──
        36171 => Some(ErrorMapping {
            message: "Currency already exists.",
            reason: Some("A currency with this code is already configured for this namespace."),
            suggestion: Some("Use a different code, or update the existing currency."),
            tip: None,
        }),
        36172 => Some(ErrorMapping {
            message: "Real currency is not allowed in this game namespace.",
            reason: Some("The game namespace is configured to disallow real currencies."),
            suggestion: Some("Use a virtual currency, or change the namespace configuration."),
            tip: None,
        }),

        // ── Ticket booth (37041–37071) ──
        37041 => Some(ErrorMapping {
            message: "Ticket booth not found.",
            reason: Some("No ticket booth exists with the supplied name in this namespace."),
            suggestion: Some("Verify the booth name and namespace."),
            tip: None,
        }),
        37071 => Some(ErrorMapping {
            message: "Insufficient tickets in booth.",
            reason: Some("The booth does not have enough tickets remaining."),
            suggestion: Some("Refill the booth or reduce the requested ticket count."),
            tip: None,
        }),

        // ── Discount codes / campaigns / key groups (37121–37271) ──
        37121 => Some(ErrorMapping {
            message: "Invalid currency namespace in discount config.",
            reason: Some("The discount config references a currency that is not configured for the namespace."),
            suggestion: Some("Use a configured currency."),
            tip: None,
        }),
        37141 => Some(ErrorMapping {
            message: "Campaign not found.",
            reason: Some("The specified campaign does not exist."),
            suggestion: Some("Run 'ags platform campaigns list' to see configured campaigns."),
            tip: None,
        }),
        37142 => Some(ErrorMapping {
            message: "Code not found.",
            reason: Some("The specified code does not exist in this namespace."),
            suggestion: Some("Verify the code and retry."),
            tip: None,
        }),
        37143 | 37144 => Some(ErrorMapping {
            message: "Campaign batch not found.",
            reason: Some("No batch matches the supplied name or number for this campaign."),
            suggestion: Some("Verify the batch name/number and retry."),
            tip: None,
        }),
        37171 => Some(ErrorMapping {
            message: "Campaign already exists.",
            reason: Some("A campaign with this name is already configured."),
            suggestion: Some("Use a different campaign name."),
            tip: None,
        }),
        37172 => Some(ErrorMapping {
            message: "Campaign is inactive.",
            reason: Some("The campaign is not currently active."),
            suggestion: Some("Activate the campaign before redeeming codes."),
            tip: None,
        }),
        37173 => Some(ErrorMapping {
            message: "Code is inactive.",
            reason: Some("The code is currently disabled."),
            suggestion: None,
            tip: None,
        }),
        37174 | 37175 | 37179 => Some(ErrorMapping {
            message: "Code redemption limit reached.",
            reason: Some("The configured redemption count for this code or campaign has been reached."),
            suggestion: None,
            tip: None,
        }),
        37176 => Some(ErrorMapping {
            message: "Code has already been redeemed.",
            reason: Some("This code was previously redeemed."),
            suggestion: None,
            tip: None,
        }),
        37177 => Some(ErrorMapping {
            message: "Code redemption has not started.",
            reason: Some("The code is not yet within its redeemable window."),
            suggestion: Some("Wait until the campaign's start time before redeeming."),
            tip: None,
        }),
        37178 => Some(ErrorMapping {
            message: "Code redemption has ended.",
            reason: Some("The code's redemption window has closed."),
            suggestion: None,
            tip: None,
        }),
        37180 => Some(ErrorMapping {
            message: "Code already exists.",
            reason: Some("Another code with this value already exists in the namespace."),
            suggestion: Some("Use a different code value."),
            tip: None,
        }),
        37221 => Some(ErrorMapping {
            message: "Invalid key file.",
            reason: Some("The supplied key file did not pass validation."),
            suggestion: Some("Check the key file format and retry."),
            tip: None,
        }),
        37241 => Some(ErrorMapping {
            message: "Key group not found.",
            reason: Some("The specified key group does not exist."),
            suggestion: Some("Run 'ags platform key-groups list' to see configured groups."),
            tip: None,
        }),
        37271 => Some(ErrorMapping {
            message: "Key group already exists.",
            reason: Some("A key group with this name is already configured."),
            suggestion: Some("Use a different key group name."),
            tip: None,
        }),

        // ── Fulfillment (38121–38171) ──
        38121 => Some(ErrorMapping {
            message: "Duplicate permanent item.",
            reason: Some("The fulfillment payload contains a permanent item the user already owns."),
            suggestion: Some("Remove the duplicate from the payload."),
            tip: None,
        }),
        38122 => Some(ErrorMapping {
            message: "Subscription end date is required.",
            reason: Some("Subscription fulfillments must include an end date."),
            suggestion: Some("Provide a subscription end date and retry."),
            tip: None,
        }),
        38128 => Some(ErrorMapping {
            message: "Cannot retry fulfillment with a different payload.",
            reason: Some("Retried fulfillments must keep the same items list."),
            suggestion: Some("Resubmit with the original items list, or use a fresh transaction ID."),
            tip: None,
        }),
        38129 => Some(ErrorMapping {
            message: "Cannot combine the same item with conflicting fields.",
            reason: Some("Two entries reference the same item but disagree on a field."),
            suggestion: Some("Reconcile the field values or split into separate fulfillments."),
            tip: None,
        }),
        38130 => Some(ErrorMapping {
            message: "Item type cannot be fulfilled.",
            reason: Some("This item type does not support fulfillment."),
            suggestion: None,
            tip: None,
        }),
        38141 => Some(ErrorMapping {
            message: "Fulfillment script not found.",
            reason: Some("No fulfillment script is configured for this namespace."),
            suggestion: Some("Configure a fulfillment script before retrying."),
            tip: None,
        }),
        38145 => Some(ErrorMapping {
            message: "Fulfillment transaction not found.",
            reason: Some("No fulfillment matches the supplied transaction ID."),
            suggestion: Some("Verify the transaction ID and retry."),
            tip: None,
        }),
        38171 => Some(ErrorMapping {
            message: "Fulfillment script already exists.",
            reason: Some("A fulfillment script is already configured for this namespace."),
            suggestion: Some("Update the existing script instead of creating a new one."),
            tip: None,
        }),

        // ── IAP (39121–39245) ──
        39121 => Some(ErrorMapping {
            message: "Apple IAP receipt verification failed.",
            reason: Some("Apple's verification service rejected the receipt."),
            suggestion: Some("Check the receipt and retry."),
            tip: None,
        }),
        39122 => Some(ErrorMapping {
            message: "Google IAP receipt is invalid.",
            reason: Some("Google's verification service rejected the receipt."),
            suggestion: Some("Check the receipt and retry."),
            tip: None,
        }),
        39123 => Some(ErrorMapping {
            message: "IAP request is not for a valid application.",
            reason: Some("The application identifier in the IAP request is not registered."),
            suggestion: Some("Verify the application ID matches the configured IAP app."),
            tip: None,
        }),
        39124 => Some(ErrorMapping {
            message: "IAP platform user is not linked to this user.",
            reason: Some("The platform user ID does not match the requesting user."),
            suggestion: Some("Link the platform account first, then retry."),
            tip: None,
        }),
        39125 => Some(ErrorMapping {
            message: "Invalid platform user token.",
            reason: Some("The supplied platform token is invalid."),
            suggestion: Some("Re-authenticate with the platform and retry."),
            tip: None,
        }),
        39126 => Some(ErrorMapping {
            message: "User is not linked to this platform.",
            reason: Some("The user has no link to the requested platform."),
            suggestion: Some("Link the platform account first, then retry."),
            tip: None,
        }),
        39127 => Some(ErrorMapping {
            message: "Invalid service label.",
            reason: Some("The supplied service label is not configured."),
            suggestion: Some("Use one of the configured service labels."),
            tip: None,
        }),
        39128 => Some(ErrorMapping {
            message: "Steam publisher key is invalid.",
            reason: Some("Steam rejected the configured publisher key."),
            suggestion: Some("Update the Steam publisher key in the IAP configuration."),
            tip: None,
        }),
        39129 => Some(ErrorMapping {
            message: "Steam app ID is invalid.",
            reason: Some("Steam rejected the configured app ID."),
            suggestion: Some("Update the Steam app ID in the IAP configuration."),
            tip: None,
        }),
        39130 | 39131 | 39134 | 39135 => Some(ErrorMapping {
            message: "Invalid IAP configuration.",
            reason: Some("The platform's IAP configuration did not pass validation."),
            suggestion: Some("Check the configuration values and retry."),
            tip: None,
        }),
        39132 | 39133 => Some(ErrorMapping {
            message: "Bad request to platform IAP service.",
            reason: Some("The platform IAP service rejected the request."),
            suggestion: Some("Check the request payload and retry."),
            tip: None,
        }),
        39136 | 39137 => Some(ErrorMapping {
            message: "Apple API request failed.",
            reason: Some("Apple's API returned an error."),
            suggestion: Some("Inspect the response message and retry."),
            tip: None,
        }),
        39138 => Some(ErrorMapping {
            message: "Apple IAP version mismatch.",
            reason: Some("The configuration and API versions do not match."),
            suggestion: Some("Align the IAP configuration version with the API version."),
            tip: None,
        }),
        39141 => Some(ErrorMapping {
            message: "Apple IAP receipt not found.",
            reason: Some("No Apple IAP receipt exists for the supplied transaction."),
            suggestion: Some("Verify the transaction ID and product ID."),
            tip: None,
        }),
        39142..=39148 => Some(ErrorMapping {
            message: "Platform IAP config not found in this namespace.",
            reason: Some("The platform's IAP configuration is missing."),
            suggestion: Some("Configure the platform IAP for this namespace."),
            tip: None,
        }),
        39149 => Some(ErrorMapping {
            message: "Third-party subscription transaction not found.",
            reason: Some("No third-party subscription transaction matches the request."),
            suggestion: Some("Verify the transaction ID and user."),
            tip: None,
        }),
        39150 => Some(ErrorMapping {
            message: "Third-party user subscription not found.",
            reason: Some("No third-party subscription matches the request."),
            suggestion: Some("Verify the subscription ID and user."),
            tip: None,
        }),
        39151 => Some(ErrorMapping {
            message: "IAP order not found.",
            reason: Some("No IAP order matches the supplied number in this namespace."),
            suggestion: Some("Verify the IAP order number and retry."),
            tip: None,
        }),
        39152 => Some(ErrorMapping {
            message: "Third-party subscription group not found.",
            reason: Some("No subscription group matches the supplied SKU in this namespace."),
            suggestion: Some("Configure the subscription group, or verify the SKU."),
            tip: None,
        }),
        39154 => Some(ErrorMapping {
            message: "Meta Quest subscription SKU not found in config.",
            reason: Some("The SKU is not configured in the namespace's subscription group."),
            suggestion: Some("Add the SKU to the subscription group configuration."),
            tip: None,
        }),
        39171 => Some(ErrorMapping {
            message: "Bundle ID does not match expected value.",
            reason: Some("The verified bundle ID differs from the configured one."),
            suggestion: Some("Align the configured bundle ID with the platform's value."),
            tip: None,
        }),
        39172 => Some(ErrorMapping {
            message: "Order ID mismatch.",
            reason: Some("The order ID returned by the platform does not match the request."),
            suggestion: None,
            tip: None,
        }),
        39173 => Some(ErrorMapping {
            message: "Google Play order has unexpected purchase status.",
            reason: Some("The purchase status reported by Google Play is not one we can process."),
            suggestion: Some("Inspect the order status and retry once it stabilises."),
            tip: None,
        }),
        39174 => Some(ErrorMapping {
            message: "Google IAP purchase time is out of range.",
            reason: Some("The purchase time falls outside the accepted window."),
            suggestion: None,
            tip: None,
        }),
        39175 => Some(ErrorMapping {
            message: "Duplicate IAP item mapping.",
            reason: Some("Another mapping already exists for this IAP type and ID."),
            suggestion: Some("Remove the duplicate mapping."),
            tip: None,
        }),
        39183 => Some(ErrorMapping {
            message: "Steam transaction is pending or failed.",
            reason: Some("Steam reported the transaction as pending or failed."),
            suggestion: Some("Retry the operation later."),
            tip: None,
        }),
        39184 => Some(ErrorMapping {
            message: "Steam API exception.",
            reason: Some("Steam returned an error while processing the request."),
            suggestion: Some("Inspect the response and retry."),
            tip: None,
        }),
        39185 => Some(ErrorMapping {
            message: "Steam IAP sync mode mismatch.",
            reason: Some("The operation requires a different sync mode than is configured."),
            suggestion: Some("Update the Steam IAP sync mode in the configuration."),
            tip: None,
        }),
        39187 => Some(ErrorMapping {
            message: "Duplicate subscription group SKU on this platform.",
            reason: Some("Another subscription group already uses this SKU on the platform."),
            suggestion: Some("Use a unique SKU."),
            tip: None,
        }),
        39188 => Some(ErrorMapping {
            message: "Subscription group is already linked to the user.",
            reason: Some("The user already has this subscription group linked on this platform."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        39221 => Some(ErrorMapping {
            message: "Invalid Xbox Business Partner Certificate or password.",
            reason: Some("The certificate or password did not pass validation."),
            suggestion: Some("Update the Xbox certificate and/or password and retry."),
            tip: None,
        }),
        39244 => Some(ErrorMapping {
            message: "Steam config not found.",
            reason: Some("No Steam configuration is set."),
            suggestion: Some("Configure Steam IAP first."),
            tip: None,
        }),
        39245 => Some(ErrorMapping {
            message: "Steam app ID not found.",
            reason: Some("The supplied Steam app ID is not configured."),
            suggestion: Some("Verify the Steam app ID and retry."),
            tip: None,
        }),
        39321 => Some(ErrorMapping {
            message: "Invalid IAP item config namespace.",
            reason: Some("The IAP item config references an invalid namespace."),
            suggestion: Some("Use a configured namespace."),
            tip: None,
        }),
        39341 | 39441 => Some(ErrorMapping {
            message: "Platform DLC config not found.",
            reason: Some("No DLC config is set for this namespace."),
            suggestion: Some("Configure the platform DLC for this namespace."),
            tip: None,
        }),
        39442 => Some(ErrorMapping {
            message: "DLC item config not found.",
            reason: Some("No DLC item config is set for this namespace."),
            suggestion: Some("Configure DLC items for this namespace."),
            tip: None,
        }),
        39471 => Some(ErrorMapping {
            message: "Duplicate DLC reward ID.",
            reason: Some("Another DLC reward already uses this ID in the namespace."),
            suggestion: Some("Use a unique DLC reward ID."),
            tip: None,
        }),
        39621 => Some(ErrorMapping {
            message: "Steam API exception.",
            reason: Some("Steam returned an error while processing the request."),
            suggestion: Some("Inspect the response and retry."),
            tip: None,
        }),

        // ── Subscriptions (40121–40173) ──
        40121 => Some(ErrorMapping {
            message: "Item type does not support subscriptions.",
            reason: Some("This item type cannot be subscribed to."),
            suggestion: None,
            tip: None,
        }),
        40122 => Some(ErrorMapping {
            message: "User is already subscribed.",
            reason: Some("An active subscription already exists for this user and item."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        40123 => Some(ErrorMapping {
            message: "Currency is not supported for subscriptions.",
            reason: Some("The supplied currency cannot be used for subscriptions."),
            suggestion: Some("Use a currency that supports recurring billing."),
            tip: None,
        }),
        40125 => Some(ErrorMapping {
            message: "Subscription has no real currency billing account.",
            reason: Some("Real-currency billing is not configured for this subscription."),
            suggestion: Some("Configure a real-currency billing account."),
            tip: None,
        }),
        40141 => Some(ErrorMapping {
            message: "Subscription not found.",
            reason: Some("The specified subscription does not exist."),
            suggestion: Some("Verify the subscription ID and retry."),
            tip: None,
        }),
        40171 => Some(ErrorMapping {
            message: "Subscription is not active.",
            reason: Some("The subscription is in an inactive state."),
            suggestion: None,
            tip: None,
        }),
        40172 => Some(ErrorMapping {
            message: "Subscription is awaiting payment notification.",
            reason: Some("A charge is in flight for this subscription."),
            suggestion: Some("Wait for the payment notification before retrying."),
            tip: None,
        }),
        40173 => Some(ErrorMapping {
            message: "Subscription currency does not match request.",
            reason: Some("The subscription's current currency differs from the request currency."),
            suggestion: Some("Use the subscription's current currency."),
            tip: None,
        }),

        // ── Idempotency (41171–41172) ──
        41171 => Some(ErrorMapping {
            message: "Request body differs from a previous call with the same idempotency key.",
            reason: Some("Idempotent requests must keep the same body across retries."),
            suggestion: Some("Use a fresh idempotency key, or resubmit with the original body."),
            tip: None,
        }),
        41172 => Some(ErrorMapping {
            message: "User ID differs from a previous call with the same idempotency key.",
            reason: Some("Idempotent requests must keep the same user across retries."),
            suggestion: Some("Use a fresh idempotency key for a different user."),
            tip: None,
        }),

        _ => None,
    }
}
