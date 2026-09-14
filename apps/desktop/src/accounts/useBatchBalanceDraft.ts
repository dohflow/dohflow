import { useState } from "react";

import type { AccountViewDto } from "@/bindings";
import { dollarsToMinorUnits } from "@/lib/format";
import { describeIpcError } from "@/vault/useVault";

import { useAccounts } from "./useAccounts";
import { enteredToStoredMinor, storedToShownMinor } from "./balanceSign";

/// Roles that carry a balance the user reconciles each cycle (banks, cards, loans,
/// investments, real assets) — virtual/clearing accounts are excluded.
const BALANCE_ROLES = new Set([
  "liquid_cash",
  "credit_facility",
  "loan_liability",
  "investment_asset",
  "real_asset",
]);

/// Today as `YYYY-MM-DD` in the user's locale (matches SetBalanceModal).
function today(): string {
  return new Date().toLocaleDateString("en-CA");
}

/// A wire minor-units amount as an editable major-unit string (2-dp currencies).
function minorUnitsToInput(minorUnits: number): string {
  return (minorUnits / 100).toString();
}

export type RowResult = { kind: "ok" } | { kind: "error"; message: string };

/// The batch balance-update draft machinery (personal-cfo-wgpb), extracted so it can
/// drive the **inline** edit mode in the real grouped Accounts layout rather than a
/// separate flat surface (personal-cfo-4d8.25.24). Each editable row is pre-filled
/// with the account's current (signed) balance; the user overwrites the ones that
/// changed and saves them together through the additive `assertBalance` model
/// (ADR 0027). All amounts respect the positive-owed sign convention (4d8.23.3) via
/// `balanceSign`.
export function useBatchBalanceDraft() {
  const { assertBalance } = useAccounts();
  const [date, setDate] = useState(today());
  // account_id → the in-progress input value (absent until the row is touched).
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [results, setResults] = useState<Record<string, RowResult>>({});
  const [saving, setSaving] = useState(false);

  const currentOf = (a: AccountViewDto) =>
    minorUnitsToInput(storedToShownMinor(a.cashflow_role, a.balance.minor_units));
  const valueOf = (a: AccountViewDto) => drafts[a.id] ?? currentOf(a);
  const isEdited = (a: AccountViewDto) =>
    drafts[a.id] !== undefined && drafts[a.id] !== currentOf(a);
  const resultOf = (id: string): RowResult | undefined => results[id];
  const isEditable = (a: AccountViewDto) =>
    a.active && BALANCE_ROLES.has(a.cashflow_role);

  function setDraft(id: string, value: string) {
    setDrafts((d) => ({ ...d, [id]: value }));
    setResults((r) => {
      if (!(id in r)) return r;
      const next = { ...r };
      delete next[id];
      return next;
    });
  }

  /// Save every edited row through `assertBalance`, then drop the drafts that saved
  /// cleanly (the account list refetches and the row falls back to the new balance).
  async function saveAll(editable: AccountViewDto[]) {
    const edited = editable.filter(isEdited);
    if (edited.length === 0) return;
    setSaving(true);
    const outcomes = await Promise.all(
      edited.map(async (a): Promise<[string, RowResult]> => {
        const entered = dollarsToMinorUnits(valueOf(a));
        if (entered === null) {
          return [a.id, { kind: "error", message: "Enter a valid amount." }];
        }
        const { error } = await assertBalance({
          account_id: a.id,
          amount: {
            minor_units: enteredToStoredMinor(a.cashflow_role, entered),
            currency: a.balance.currency,
          },
          as_of_date: date,
        });
        return [
          a.id,
          error ? { kind: "error", message: describeIpcError(error) } : { kind: "ok" },
        ];
      }),
    );
    setSaving(false);
    setResults((r) => ({ ...r, ...Object.fromEntries(outcomes) }));
    setDrafts((d) => {
      const next = { ...d };
      for (const [id, res] of outcomes) if (res.kind === "ok") delete next[id];
      return next;
    });
  }

  const editedCount = (editable: AccountViewDto[]) =>
    editable.filter(isEdited).length;
  const savedCount = Object.values(results).filter((r) => r.kind === "ok").length;

  /// Discard all in-progress drafts + results (adversarial review of 4d8.25.24):
  /// the flat component used to unmount on exit, discarding edits — since the hook
  /// now outlives batch mode, leaving it must clear state so a stale draft can't
  /// resurrect and silently overwrite a balance changed by another path.
  function reset() {
    setDrafts({});
    setResults({});
    setSaving(false);
    setDate(today());
  }

  return {
    date,
    setDate,
    valueOf,
    isEdited,
    isEditable,
    resultOf,
    setDraft,
    saveAll,
    editedCount,
    savedCount,
    saving,
    reset,
  };
}

export type BatchBalanceDraft = ReturnType<typeof useBatchBalanceDraft>;
