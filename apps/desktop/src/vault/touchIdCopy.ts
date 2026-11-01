/// Plain-language copy for each Touch ID reason code the app can return (personal-cfo-xuu, K5).
/// The codes are the stable strings of `touch_id::Reason`; each one reads on its own, on the unlock
/// screen and in Settings, and none names a key, a path or an error. An unknown code (a newer host)
/// falls back to the password.

const COPY: Record<string, string> = {
  after_restart:
    "Your Mac restarted. Enter your master password once and Touch ID works again.",
  too_many_failed_prompts:
    "Touch ID didn't work three times in a row. Enter your master password to use Touch ID again.",
  password_too_old:
    "It has been 30 days since you entered your master password. Enter it to keep using Touch ID.",
  markers_unreadable:
    "DohFlow couldn't confirm that Touch ID is safe to use right now. Enter your master password.",
  retired:
    "Touch ID was turned off because this vault's key or the fingerprints on this Mac changed. Enter your master password, then turn Touch ID on again in Settings.",
  biometry_not_present: "Touch ID isn't available on this Mac right now.",
  biometry_not_enrolled:
    "No fingerprint is set up on this Mac. Add one in System Settings, then turn Touch ID on here.",
  biometry_locked_out:
    "Touch ID is locked after too many tries. Enter your master password.",
  biometry_unknown:
    "DohFlow couldn't tell whether Touch ID is available. Enter your master password.",
  signed_build_needed:
    "Touch ID needs the signed release of DohFlow, so it isn't available in this build. Use your master password.",
  store_unavailable:
    "DohFlow can't reach your Mac's secure storage right now. Enter your master password.",
  not_enabled: "Touch ID is off for this vault.",
  prompt_not_completed:
    "Touch ID didn't unlock the vault. Try again, or enter your master password.",
  unlock_unavailable:
    "Touch ID couldn't open the vault this time. Enter your master password.",
  attempt_in_progress:
    "Touch ID is already waiting for you. Finish that, or enter your master password.",
};

const FALLBACK = "Enter your master password.";

/// The sentence that explains `code` to the person looking at the screen.
export function touchIdReasonCopy(code: string | null | undefined): string {
  return (code && COPY[code]) || FALLBACK;
}

/// Every code this build has copy for (the tests check the list against the host's codes).
export const TOUCH_ID_REASON_CODES = Object.keys(COPY);
