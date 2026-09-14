import { Check } from "lucide-react";

import type { AccountViewDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { accountLabel } from "@/future-cash/seriesKeys";
import { storedToShownMinor } from "@/accounts/balanceSign";
import { formatMoney } from "@/lib/format";
import { cn } from "@/lib/utils";

import { scopeSentence } from "./debtAccounts";

/// The Debt page's scope control (personal-cfo-39xn, from the Debt Page mock).
///
/// Replaces a dropdown that read "All debts" or "3 debts" — which you had to **open** to
/// learn what the figures below referred to. Every debt is a chip carrying its own balance,
/// so the scope reads left to right with nothing to open.
///
/// The selection model is unchanged from ADR 0057 §1: a filter over the page, defaulting to
/// all, and the page's shape does not change with how many are ticked. This changes how the
/// scope *reads*, not what it means.
export function DebtScopeBar({
  accounts,
  selected,
  onChange,
}: {
  /// The selectable debt accounts, already filtered by [`debtAccounts`].
  accounts: AccountViewDto[];
  /// Selected account ids. Empty means **all**.
  selected: string[];
  onChange: (selected: string[]) => void;
}) {
  const all = selected.length === 0;
  const labelled = accounts.map((a) => ({ id: a.id, name: a.name, subtype: a.subtype }));

  function toggle(id: string) {
    // An empty selection means ALL, so the first tick starts from everything and removes —
    // otherwise unticking one card would silently leave only that card selected.
    const current = all ? accounts.map((a) => a.id) : selected;
    const next = current.includes(id)
      ? current.filter((x) => x !== id)
      : [...current, id];
    // Back to everything → collapse to the "all" representation so the sentence reads
    // "all N of your debts" rather than enumerating them.
    onChange(next.length === accounts.length ? [] : next);
  }

  return (
    <div className="flex flex-col gap-2.5 rounded-lg border bg-card px-4 py-3.5">
      <div className="flex items-center gap-3">
        <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          Showing
        </span>
        {/* Chips wrap rather than scroll: a household with a dozen cards gets a taller bar,
            which is legible, instead of a horizontal scroller that hides part of the scope
            — and hiding part of the scope is the exact failure this control exists to fix. */}
        <div className="flex flex-1 flex-wrap items-center gap-2">
          {accounts.length === 0 ? (
            <span className="text-sm text-muted-foreground">No debts to scope yet.</span>
          ) : (
            accounts.map((account) => {
              const on = all || selected.includes(account.id);
              return (
                <button
                  key={account.id}
                  type="button"
                  aria-pressed={on}
                  onClick={() => toggle(account.id)}
                  className={cn(
                    "flex items-center gap-2 whitespace-nowrap rounded-full border py-1.5 pl-2.5 pr-3 text-sm transition-colors",
                    on
                      ? "border-primary/40 bg-primary/10 font-medium text-foreground"
                      : "border-border bg-transparent text-muted-foreground hover:bg-muted",
                  )}
                >
                  <Check
                    aria-hidden
                    className={cn("size-3.5 shrink-0", on ? "opacity-100" : "opacity-0")}
                  />
                  <span>
                    {accountLabel(
                      { id: account.id, name: account.name, subtype: account.subtype },
                      labelled,
                    )}
                  </span>
                  <span className="tabular-nums opacity-75">
                    {formatMoney({
                      minor_units: storedToShownMinor(
                        account.cashflow_role,
                        account.balance.minor_units,
                      ),
                      currency: account.balance.currency,
                    })}
                  </span>
                </button>
              );
            })
          )}
        </div>
        <Button
          size="sm"
          variant="ghost"
          disabled={all || accounts.length === 0}
          onClick={() => onChange([])}
        >
          All debts
        </Button>
      </div>
      <p className="text-sm text-muted-foreground">
        {scopeSentence(accounts, selected)}
      </p>
    </div>
  );
}
