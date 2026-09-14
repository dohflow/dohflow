import { useState } from "react";
import { Loader2 } from "lucide-react";

import type { CardStatementForecastDto } from "@/bindings";
import { useAccounts } from "@/accounts/useAccounts";
import { useTransactions } from "@/transactions/useTransactions";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { dollarsToMinorUnits, formatMoney } from "@/lib/format";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { describeIpcError } from "@/vault/useVault";

const SELECT_CLASS =
  "flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

function minorToInput(minorUnits: number): string {
  return (minorUnits / 100).toString();
}

/// A user-opened tool to record a credit-card payment (r7sb mixed-role transfer, personal-cfo-6wk.9):
/// move money from a liquid account to the card, reducing the owed balance. (Paying from a
/// brokerage is a two-step flow — liquidate to checking first via cover-it, then pay from there —
/// so an investment isn't offered here.) Quick-picks fill the
/// statement balance / minimum due / current balance; the amount and source stay the user's choice
/// and the payment is only recorded on confirm. Descriptive copy only (ADR 0018).
export function PayCardAction({ card }: { card: CardStatementForecastDto }) {
  const next = card.cycles[0];
  const { accounts } = useAccounts();
  const { transfer } = useTransactions();

  const [open, setOpen] = useState(false);
  const [amountDraft, setAmountDraft] = useState<string | null>(null);
  const [sourceChoice, setSourceChoice] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!next) return null;

  const currency = card.currency;
  // A card payment comes from a liquid account holding funds in the card's currency. (An
  // investment is liquidated to cash first — via cover-it — then the card is paid from cash.)
  const sources = (accounts ?? []).filter(
    (a) =>
      a.cashflow_role === "liquid_cash" &&
      a.active &&
      a.balance.minor_units > 0 &&
      a.balance.currency === currency,
  );
  const source = sourceChoice || sources[0]?.id || "";
  const amountValue = amountDraft ?? minorToInput(next.statement_balance_minor);

  const quickPicks: { label: string; minor: number }[] = [
    { label: "Statement balance", minor: next.statement_balance_minor },
    { label: "Minimum due", minor: next.minimum_due_minor },
    { label: "Current balance", minor: next.carried_opening_balance_minor },
  ];

  async function onConfirm() {
    setError(null);
    const minor = dollarsToMinorUnits(amountValue);
    if (minor === null || minor <= 0) {
      setError("Enter an amount above zero.");
      return;
    }
    setSaving(true);
    const failure = await transfer({
      source_account_id: source,
      dest_account_id: card.account_id,
      amount: { minor_units: minor, currency },
      occurred_at: new Date().toISOString(),
      idempotency_key: mintIdempotencyKey(),
    });
    setSaving(false);
    if (failure) {
      setError(describeIpcError(failure));
    } else {
      setOpen(false);
      setAmountDraft(null);
    }
  }

  if (!open) {
    return (
      <Button variant="outline" size="sm" className="self-start" onClick={() => setOpen(true)}>
        Pay card
      </Button>
    );
  }

  if (sources.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No liquid account holds funds in {currency} to pay from.
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-3 rounded-md border bg-muted/30 p-3">
      <div className="flex flex-wrap gap-2">
        {quickPicks.map((q) => (
          <Button
            key={q.label}
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => setAmountDraft(minorToInput(q.minor))}
          >
            {q.label} ({formatMoney({ minor_units: q.minor, currency })})
          </Button>
        ))}
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div className="flex flex-col gap-1">
          <Label htmlFor={`pay-amount-${card.account_id}`} className="text-xs">
            Amount
          </Label>
          <Input
            id={`pay-amount-${card.account_id}`}
            inputMode="decimal"
            value={amountValue}
            onChange={(e) => setAmountDraft(e.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1">
          <Label htmlFor={`pay-source-${card.account_id}`} className="text-xs">
            From account
          </Label>
          <select
            id={`pay-source-${card.account_id}`}
            className={SELECT_CLASS}
            value={source}
            onChange={(e) => setSourceChoice(e.target.value)}
          >
            {sources.map((a) => (
              <option key={a.id} value={a.id}>
                {a.name} ({formatMoney(a.balance)})
              </option>
            ))}
          </select>
        </div>
      </div>
      {error && (
        <p role="alert" className="text-sm text-loss">
          {error}
        </p>
      )}
      <div className="flex gap-2">
        <Button size="sm" onClick={() => void onConfirm()} disabled={saving || !source}>
          {saving ? <Loader2 className="animate-spin" aria-hidden /> : null}
          Confirm payment
        </Button>
        <Button variant="ghost" size="sm" onClick={() => setOpen(false)}>
          Cancel
        </Button>
      </div>
    </div>
  );
}
