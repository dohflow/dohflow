import type { IpcError } from "@/bindings";
import { describeIpcError } from "@/vault/useVault";

type IpcResult<T> =
  | { status: "ok"; data: T }
  | { status: "error"; error: IpcError };

/// Adapt a typed IPC `Result` for a TanStack Query `queryFn`: return the data, or
/// **throw** a friendly message so `query.error` carries it. The `fallback`
/// covers a thrown IPC rejection (the vault service being unavailable), matching
/// the previous hand-rolled hooks' behaviour. Rust remains authoritative for
/// validation (ADR 0003); this only shapes display copy.
export async function ipcQuery<T>(
  call: Promise<IpcResult<T>>,
  fallback: string,
): Promise<T> {
  let result: IpcResult<T>;
  try {
    result = await call;
  } catch {
    throw new Error(fallback);
  }
  if (result.status === "ok") return result.data;
  throw new Error(describeIpcError(result.error));
}

/// Stable query keys — one per read command. Mutations invalidate these to keep
/// every screen sharing the data consistent (ADR 0020).
export const queryKeys = {
  accounts: ["accounts"] as const,
  /// The type-based cash-tier rollups (ADR 0028); invalidated whenever accounts,
  /// subtypes, or balances change.
  cashTiers: ["cash-tiers"] as const,
  /// The household cash-availability snapshot (ADR 0029, personal-cfo-fqbm):
  /// ledger/available/committed/headroom + the floor status. Derived from the
  /// same canonical data as the forecast, so every mutation that invalidates
  /// `["forecast"]` invalidates this too.
  cashAvailability: ["cash-availability"] as const,
  /// The liquid-cash comfort band (lower = the floor, optional upper) — ADR 0018
  /// addendum 915.1, personal-cfo-3v6d. Setting either edge invalidates this.
  comfortBand: ["comfort-band"] as const,
  /// The descriptive band-drift signal (why the projection heads below the band) — ADR 0018
  /// §915.1, personal-cfo-5ie.8. Derived from the forecast + band + spend, so forecast-,
  /// band-, and transaction-affecting mutations invalidate it.
  bandDrift: ["band-drift"] as const,
  /// The R1 Forecast Readiness score (ADR 0026 §13, personal-cfo-6vj9): coverage +
  /// balance freshness + explained ratio. Derived from accounts / income / bills /
  /// balances / transactions, so those mutations invalidate it.
  forecastReadiness: ["forecast-readiness"] as const,
  /// Forecast capabilities that have self-activated but whose one-time unlock notice
  /// the user hasn't acknowledged (ADR 0026 §10, personal-cfo-egon). Acknowledging
  /// invalidates this so the notice drops; otherwise it refreshes on mount / focus.
  capabilityUnlocks: ["capability-unlocks"] as const,
  /// The vault health check (personal-cfo-n9w/-5ivp). On-demand in Settings; a
  /// read-model rebuild invalidates it so the verdict refreshes.
  vaultHealth: ["vault-health"] as const,
  /// Bank connections + their account links (personal-cfo-ul5d, ADR 0060).
  /// Linking, mapping, syncing, and forgetting invalidate this; a sync also
  /// writes the ledger, so it invalidates transactions / inbox / forecast too.
  connections: ["connections"] as const,
  transactions: ["transactions"] as const,
  /// Spend rolled up by category (ADR 0052, personal-cfo-4d8.27.8.4). Under the
  /// `["transactions"]` prefix on purpose: it is derived from transactions, their
  /// categorizations and their split lines, so every mutation that already invalidates
  /// the list keeps the chart above it honest. A separate prefix would let the two
  /// disagree after a recategorize, which is the one thing this surface must never do.
  spendByCategory: (input: unknown) => ["transactions", "spend", input] as const,
  /// The Money Inbox triage items (ADR 0014 §7, personal-cfo-tknc). Resolving an
  /// item (import-anyway / skip) invalidates this; import-anyway also writes the
  /// ledger, so it invalidates the financial caches below as well.
  moneyInbox: ["money-inbox"] as const,
  income: ["income"] as const,
  bills: ["bills"] as const,
  /// Recurring account-to-account transfers (ADR 0026 §14, personal-cfo-npoe).
  /// They project in the per-account forecast, so a mutation invalidates the
  /// forecast too.
  recurringTransfers: ["recurring-transfers"] as const,
  /// The category taxonomy (plan §9.6, ADR 0030, personal-cfo-bac). Reference data
  /// for the management UI + the categorization picker; a category mutation
  /// (create / edit / move / archive) invalidates only this.
  categories: ["categories"] as const,
  /// The tag vocabulary (ADR 0033, personal-cfo-2ryf). Reference data for the tag
  /// picker; a tag mutation (create) invalidates only this.
  tags: ["tags"] as const,
  /// One transaction's split lines (ADR 0034, personal-cfo-kr9). Keyed by transaction
  /// id; a SetSplits invalidates the affected transaction's lines + the list.
  transactionSplits: (transactionId: string) =>
    ["transaction-splits", transactionId] as const,
  /// A flagged staged transaction's committed duplicate counterpart(s) (ADR 0032 §4,
  /// personal-cfo-4d8.20). Keyed by the staged txn id; backs the Review panel.
  duplicateCandidates: (stagedTransactionId: string) =>
    ["duplicate-candidates", stagedTransactionId] as const,
  manualEntries: ["manual-entries"] as const,
  /// Device-local verified backup receipts and the active vault's schedule.
  backupHistory: ["backup-history"] as const,
  backupSchedule: ["backup-schedule"] as const,
  baseCurrency: ["base-currency"] as const,
  /// The household's IANA timezone (ADR 0021 §1, personal-cfo-q329) — the calendar-boundary
  /// authority for "today". Setting it invalidates the `["forecast"]` prefix (which already
  /// covers `unconfirmedPastDue`, nested under it) and Money Inbox, so the day boundary
  /// updates everywhere without a relaunch.
  householdTimezone: ["household-timezone"] as const,
  /// Whether merchant memory auto-applies after an import (ADR 0030 addendum,
  /// personal-cfo-5n4.2). Toggled in Settings; defaults on.
  autoCategorizeOnImport: ["auto-categorize-on-import"] as const,
  /// The Future Cash chart's persisted series selection (personal-cfo-4d8.25.26).
  futureCashSeries: ["future-cash-series"] as const,
  scenarios: ["scenarios"] as const,
  /// The active assumption events for one scenario (`null` = base). All share the
  /// `["scenario-events"]` prefix, so a scenario change can invalidate them all.
  scenarioEvents: (scenarioId: string | null) =>
    ["scenario-events", scenarioId] as const,
  /// All Future Cash forecasts (any horizon, any scenario) share this prefix, so a
  /// mutation can invalidate them with `{ queryKey: ["forecast"] }`. `scenarioId`
  /// is `null` for the base forecast — base and a scenario cache independently so
  /// the compare-vs-base view holds both at once (personal-cfo-6zep).
  forecast: (horizonDays: number, scenarioId: string | null = null) =>
    ["forecast", horizonDays, scenarioId] as const,
  /// The per-account/per-group projection (personal-cfo-l8oh). Shares the
  /// `["forecast"]` prefix so the same mutations invalidate it as the aggregate.
  forecastByAccount: (horizonDays: number, scenarioId: string | null = null) =>
    ["forecast", "by-account", horizonDays, scenarioId] as const,
};
