/// Shown for a balance whose save call threw: nobody knows whether it was saved, which is never
/// reported as "not saved". Saving the same figure again reuses its idempotency key, so it cannot
/// add a second observation (personal-cfo-g3m.37).
export const BALANCE_OUTCOME_UNKNOWN =
  "Couldn’t confirm whether this balance was saved. Save again and it won’t be added twice.";

/// Shown for a balance that was saved but whose new figure could not be read back.
export const BALANCE_SAVED_UNCONFIRMED =
  "Saved, but the new balance couldn’t be confirmed.";
