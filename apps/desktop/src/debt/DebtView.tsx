import { useState } from "react";
import { AlertTriangle } from "lucide-react";

import { CreditCardsView } from "@/accounts/CreditCardsView";
import { DebtPayoffCompare } from "@/accounts/DebtPayoffCompare";
import { useAccounts } from "@/accounts/useAccounts";
import { useCardStatementForecast } from "@/accounts/useCardStatementForecast";
import { TransactionsView } from "@/transactions/TransactionsView";

import { DebtScopeBar } from "./DebtScopeBar";
import { DebtStatRow } from "./DebtStatRow";
import { DebtTermsTable } from "./DebtTermsTable";
import { DebtVisualizations } from "./DebtVisualizations";
import { debtAccounts, effectiveSelection } from "./debtAccounts";
import { useScopedDebts } from "./useScopedDebts";

/// The Debt page (personal-cfo-4d8.27.9.2/.9.3, ADR 0049 §5 + ADR 0057).
///
/// Debt analysis got its own destination because the depth the owner asked for — per-account
/// selection, per-debt-type visualizations, a scoped transaction log, payoff tools — could
/// not live in a collapsed section at the bottom of Accounts without dominating it.
///
/// The division of labour is the part to keep straight: **Accounts owns account identity
/// and balances; Debt owns debt analysis** (ADR 0049 §5). The card-statement forecasts and
/// the payoff comparison MOVED here — they are not rendered in both places.
///
/// The selector is a **filter over the page, not a mode** (ADR 0057 §1): the layout does
/// not change with how many accounts are ticked, so the surfaces below each scope
/// themselves rather than switching shape.
export function DebtView() {
  const { accounts, error } = useAccounts();
  const [selected, setSelected] = useState<string[]>([]);

  const debts = debtAccounts(accounts ?? []);
  const scope = effectiveSelection(debts, selected);
  const inScope = debts.filter((a) => scope.includes(a.id));
  const { debts: scopedDebts } = useScopedDebts(inScope);
  // Card cycles feed the terms table's projected-statement column.
  const { cards } = useCardStatementForecast();
  const currency = inScope[0]?.balance.currency ?? "USD";

  return (
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-6">
      <div>
        <h2 className="text-lg font-semibold tracking-tight">Debt</h2>
        <p className="mt-0.5 text-sm text-muted-foreground">
          What you owe, what it costs, and how it pays down.
        </p>
      </div>

      {/* The scope reads left to right with nothing to open, and says in words what every
          figure below covers. Still a filter over the page, defaulting to all (ADR 0057 §1)
          — what changed is legibility, not the model. Shown for a single debt too: with one
          chip it is a statement of scope rather than a control, which is worth more here
          than the saved row. */}
      {debts.length > 0 && (
        <DebtScopeBar accounts={debts} selected={selected} onChange={setSelected} />
      )}

      {/* A failed read used to render empty surfaces with no explanation, which reads as
          "you have no debt" rather than "we could not look". */}
      {error !== null && (
        <div
          role="alert"
          className="flex items-start gap-3 rounded-lg border border-loss/30 bg-loss/[0.08] p-4"
        >
          <AlertTriangle className="mt-0.5 size-4 shrink-0 text-loss" aria-hidden />
          <div className="flex flex-col gap-1">
            <p className="text-sm font-semibold text-loss">Could not read your debt data.</p>
            <p className="text-sm text-muted-foreground">
              The vault returned an error while loading balances and terms. Nothing has
              changed on disk.
            </p>
          </div>
        </div>
      )}

      {accounts !== null && debts.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No credit cards or loans yet. Add one from Accounts and it will show up here with
          its payoff projection.
        </p>
      ) : (
        <>
          {/* What the scoped debts cost right now. Gated on accounts AND terms having
              arrived: during the accounts-loading window the scope is momentarily empty,
              and an empty terms table there reads as "no debts have terms" rather than
              "still looking". A stat row totalling a half-loaded set has the same problem
              in a worse place. */}
          {accounts !== null && scopedDebts !== null && scopedDebts.length > 0 && (
            <>
              <DebtStatRow debts={scopedDebts} currency={currency} />
              <DebtTermsTable
                debts={scopedDebts}
                currency={currency}
                cards={cards}
              />
            </>
          )}
          {/* The mock puts paydown and payoff ABOVE spending and activity: they answer
              "how does this end", which is the question the page is for, and the backward-
              looking sections are context for it.

              Kept as ONE block rather than the mock's separate "projected paydown" chart
              plus payoff table. The mock's chart is a single minimums-only curve; ours plots
              all three strategies on one axis, which is what makes the comparison legible —
              splitting them would separate the lines from the plans they belong to.

              Scoped to the selection (personal-cfo-4d8.27.9.7): the scope reaches the
              SIMULATION, not its output, since snowball and avalanche order the debts and
              route the extra budget among them. */}
          <DebtPayoffCompare accountIds={scope} />

          <CreditCardsView accountIds={scope} />
          {/* Per-role visualizations (ADR 0057 §2, 2026-09-01 addendum): both roles
              get balance-over-time — for a loan that curve IS its amortization. The
              card spend breakdown moved to the scoped Activity section only (emgh). */}
          <DebtVisualizations
            accounts={debts.filter((a) => scope.includes(a.id))}
          />
          {/* The same Transactions table, pinned to the selection (ADR 0057 §3). The
              account facet disappears from its filter bar because this page's selector
              owns it — everything else stays filterable, since the scope is a floor on
              what is shown rather than a replacement for filtering within it. */}
          <div>
            <h3 className="mb-2 text-sm font-medium">Activity on these debts</h3>
            <TransactionsView embedded accountScope={scope} />
          </div>
        </>
      )}
    </div>
  );
}
