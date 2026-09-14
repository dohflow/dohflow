// Shared sync-outcome vocabulary (personal-cfo-ul5d/-zfyo): the Settings card
// and the Money Inbox retry must read the same outcome the same way. The
// status union is the ConnectorSyncResultDto contract (bindings.ts).

/// Whether an ok-envelope sync outcome is good news or needs attention.
/// Rate-limited and debounced are HEALTHY states (ADR 0060 §5) — pacing,
/// never a fault.
export function syncOutcomeTone(status: string): "notice" | "problem" {
  switch (status) {
    case "synced":
    case "partially_committed":
    case "rate_limited":
    case "skipped_debounced":
    case "sync_in_progress":
    case "discovered_accounts":
    case "no_mapped_accounts":
      // no_mapped_accounts: since the discovery fix (personal-cfo-k025) this
      // status only means "the provider reported no accounts yet" — a
      // healthy waiting state, never a fault.
      return "notice";
    default:
      return "problem";
  }
}

function capitalize(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/// Human copy for a sync outcome.
export function syncOutcomeCopy(
  status: string,
  message: string | null,
): string {
  switch (status) {
    case "synced":
      // The backend may report work done alongside the success (e.g. an
      // auto-categorized count, kz88).
      return message ? `Synced — ${message}.` : "Synced.";
    case "partially_committed":
      // Keep the kz88 auto-categorized note even when rows also flagged.
      return message
        ? `Synced — some rows are waiting in the Money Inbox. ${capitalize(message)}.`
        : "Synced — some rows are waiting in the Money Inbox.";
    case "rate_limited":
      return "The provider is pacing requests — this connection will sync later.";
    case "sync_in_progress":
      return "A sync is already running for this connection.";
    case "skipped_debounced":
      return "Recently synced — skipped.";
    case "discovered_accounts":
      return message ?? "Found new accounts — map them in Settings, then sync.";
    case "no_mapped_accounts":
      // The backend message says whether the provider has no accounts yet;
      // the fallback covers older payloads.
      return message ?? "Map at least one account in Settings, then sync.";
    case "expired":
      return "The connection expired — re-link it with a fresh setup token.";
    case "needs_user_action":
      return message ?? "The provider needs attention at the Bridge.";
    default:
      return message ?? "The sync did not complete.";
  }
}
