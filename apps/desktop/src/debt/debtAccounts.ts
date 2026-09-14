/// Pure helpers for the Debt page's account scope.
///
/// Kept out of the component file so they can be shared without a fast-refresh warning —
/// the same reason `future-cash/seriesKeys.ts` exists.
import type { AccountViewDto } from "@/bindings";

/// The account roles the Debt page covers.
export const DEBT_ROLES = ["credit_facility", "loan_liability"] as const;

/// The debt accounts a household actually has: active, and one of the two debt roles.
///
/// Archived accounts are excluded for the same reason they left the forecast (ADR 0056) —
/// a retired debt is not part of the picture, and offering it here would invite selecting
/// a debt that no longer exists.
export function debtAccounts(accounts: AccountViewDto[]): AccountViewDto[] {
  return accounts.filter(
    (a) => a.active && (DEBT_ROLES as readonly string[]).includes(a.cashflow_role),
  );
}

/// The account ids the page is actually scoped to.
///
/// An empty selection means "all", which keeps the default free of a list that would go
/// stale the moment an account is added. Callers want the concrete set, so this resolves
/// it once rather than each surface re-deriving the same rule (and one of them getting it
/// wrong).
export function effectiveSelection(
  accounts: AccountViewDto[],
  selected: string[],
): string[] {
  return selected.length === 0 ? accounts.map((a) => a.id) : selected;
}

/// A sentence naming what every figure below the bar refers to.
///
/// The chips already say *which* debts are on; this says it in words, because a row of
/// toggles is read as a control and a sentence is read as a statement — and the thing the
/// user needs to trust is the statement.
export function scopeSentence(
  accounts: AccountViewDto[],
  selected: string[],
): string {
  if (accounts.length === 0) return "No debts to scope yet.";
  const on = selected.length === 0 ? accounts : accounts.filter((a) => selected.includes(a.id));
  if (on.length === accounts.length) {
    return accounts.length === 1
      ? "Everything below covers your one debt."
      : `Everything below covers all ${accounts.length} of your debts.`;
  }
  if (on.length === 1) {
    return `Everything below covers ${on[0]?.name ?? "one debt"} only.`;
  }
  return `Everything below covers ${on.length} of your ${accounts.length} debts.`;
}
