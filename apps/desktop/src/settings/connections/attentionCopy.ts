// The words of the re-authentication notice (personal-cfo-pxi.22; ADR 0060 addendum 2026-10-23).
//
// Only copy the owner approved lives here; there is deliberately no generic fallback line. An
// adapter or kind with no approved wording returns null and the notice shows nothing for it (the
// Settings card keeps showing the connection's last error as before), rather than shipping
// invented words. The bank name arrives already cleaned by the backend and is rendered as text.

import type { ConnectorAttentionDto } from "@/bindings";

const REAUTH_INSTITUTION = "reauth_institution";
const RELINK_CONNECTION = "relink_connection";

/// The provider's name as the notice's link names it, or null when the adapter has no approved
/// notice.
export function providerSiteName(adapterId: string): string | null {
  switch (adapterId) {
    case "simplefin":
      return "SimpleFIN Bridge";
    case "lunchflow":
      return "LunchFlow";
    default:
      return null;
  }
}

/// The approved sentence for one thing a connection needs, or null when there is none.
export function attentionMessage(
  adapterId: string,
  entry: ConnectorAttentionDto,
): string | null {
  if (adapterId === "simplefin") {
    if (entry.kind === REAUTH_INSTITUTION) {
      return entry.institution
        ? `Sign in to ${entry.institution} again in SimpleFIN Bridge so its accounts keep refreshing.`
        : "One of your banks needs you to sign in again in SimpleFIN Bridge.";
    }
    if (entry.kind === RELINK_CONNECTION) {
      return "Your SimpleFIN connection needs to be set up again.";
    }
  }
  if (adapterId === "lunchflow" && entry.kind === RELINK_CONNECTION) {
    return "Your LunchFlow connection needs a new API key.";
  }
  return null;
}
