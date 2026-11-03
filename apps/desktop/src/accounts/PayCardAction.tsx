import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";

import type { CardStatementForecastDto } from "@/bindings";
import { useAccounts } from "@/accounts/useAccounts";
import { useTransactions } from "@/transactions/useTransactions";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { dollarsToMinorUnits, formatMoney } from "@/lib/format";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { queryKeys } from "@/lib/query";
import { describeIpcError } from "@/vault/useVault";

const SELECT_CLASS =
  "flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

/// Shown after the payment call threw: nobody knows whether the transfer was recorded. Confirming
/// the same payment again reuses its idempotency key, so it cannot be recorded twice.
const PAYMENT_OUTCOME_UNKNOWN =
  "Couldn’t confirm whether the payment was recorded. Confirm again and it won’t be recorded twice.";

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
  const { accounts, error: accountsError } = useAccounts();
  const { transfer } = useTransactions();
  const queryClient = useQueryClient();

  const [open, setOpen] = useState(false);
  const [amountDraft, setAmountDraft] = useState<string | null>(null);
  const [sourceChoice, setSourceChoice] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const panel = useRef<HTMLDivElement>(null);
  const payButton = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef(false);
  // A retry of an accounts read that never produced data puts the query back to pending, which clears
  // its error and would flip the panel to "Loading…" and hide the Retrying… button; the failure being
  // retried stays on screen.
  const [retrying, setRetrying] = useState(false);
  const [retriedFailure, setRetriedFailure] = useState<string | null>(null);
  // One idempotency key per draft: confirming the same payment again (after a call that threw, so
  // nobody knows whether it was recorded) replays instead of recording a second transfer. A
  // different amount or source is a different draft.
  const draftKey = useRef<{ identity: string; key: string } | null>(null);

  // An opened tool owns the keyboard: the Pay card button is gone, so focus moves in (and back in if
  // a control it was on disappears), which is what lets Escape reach it. Only focus that was lost is
  // taken back; a control the person chose elsewhere keeps it.
  const isOpen = open && next !== undefined;
  useEffect(() => {
    if (!isOpen || !panel.current) return;
    const active = document.activeElement;
    if (!active || active === document.body || active === document.documentElement) {
      panel.current.focus();
    }
  });
  // Closing returns focus to the button that opened the tool.
  useEffect(() => {
    if (!isOpen && returnFocus.current) {
      returnFocus.current = false;
      payButton.current?.focus();
    }
  }, [isOpen]);

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
    const identity = `${source}|${card.account_id}|${currency}|${minor}`;
    if (draftKey.current?.identity !== identity) {
      draftKey.current = { identity, key: mintIdempotencyKey() };
    }
    setSaving(true);
    try {
      const failure = await transfer({
        source_account_id: source,
        dest_account_id: card.account_id,
        amount: { minor_units: minor, currency },
        occurred_at: new Date().toISOString(),
        idempotency_key: draftKey.current.key,
      });
      if (failure) {
        setError(describeIpcError(failure));
      } else {
        // The next payment is a new draft, even for the same amount and source.
        draftKey.current = null;
        setAmountDraft(null);
        closeTool();
      }
    } catch {
      // The call itself failed, so the outcome is not known. The key is kept, so confirming this
      // draft again cannot record a second transfer, and what is stored is read again before
      // Confirm is offered (it stays disabled while saving).
      setError(PAYMENT_OUTCOME_UNKNOWN);
      await Promise.allSettled([
        queryClient.refetchQueries({ queryKey: queryKeys.accounts }),
        queryClient.refetchQueries({ queryKey: ["forecast"] }),
      ]);
    } finally {
      setSaving(false);
    }
  }

  async function retryAccounts() {
    setRetriedFailure(accountsError);
    setRetrying(true);
    try {
      await queryClient.refetchQueries({ queryKey: queryKeys.accounts });
    } finally {
      setRetrying(false);
    }
  }

  function closeTool() {
    returnFocus.current = true;
    setOpen(false);
  }

  /// Cancel and Escape are refused while a payment is being recorded: the outcome has not come
  /// back, and the tool is where it will be reported.
  function dismiss() {
    if (!saving) closeTool();
  }

  if (!open) {
    return (
      <Button
        ref={payButton}
        variant="outline"
        size="sm"
        className="self-start"
        onClick={() => setOpen(true)}
      >
        Pay card
      </Button>
    );
  }

  // What the tool says about the accounts it pays from, in order: a failed read (which outranks a
  // cached list, since nothing here can be confirmed on accounts that could not just be read), a read
  // still on its way, and only after a read that worked, no account to pay from.
  const accountsFailure = accountsError ?? (retrying ? retriedFailure : null);
  if (accountsFailure !== null || accounts === null || sources.length === 0) {
    return (
      <div
        ref={panel}
        tabIndex={-1}
        className="flex flex-wrap items-center gap-3 outline-none"
        onKeyDown={(e) => {
          if (e.key === "Escape") dismiss();
        }}
      >
        {error === PAYMENT_OUTCOME_UNKNOWN && (
          <p role="alert" className="w-full text-sm text-loss">
            {error}
          </p>
        )}
        {accountsFailure !== null ? (
          <p role="alert" className="text-sm text-loss">
            {accountsFailure}
          </p>
        ) : accounts === null ? (
          <p role="status" className="text-sm text-muted-foreground">
            Loading your accounts…
          </p>
        ) : (
          <p className="text-sm text-muted-foreground">
            No liquid account holds funds in {currency} to pay from.
          </p>
        )}
        {accountsFailure !== null && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => void retryAccounts()}
            disabled={retrying}
          >
            {retrying ? <Loader2 className="animate-spin" aria-hidden /> : null}
            {retrying ? "Retrying…" : "Retry"}
          </Button>
        )}
        <Button variant="ghost" size="sm" onClick={dismiss} disabled={saving}>
          Cancel
        </Button>
      </div>
    );
  }

  return (
    <div
      ref={panel}
      tabIndex={-1}
      className="flex flex-col gap-3 rounded-md border bg-muted/30 p-3 outline-none"
      onKeyDown={(e) => {
        if (e.key === "Escape") dismiss();
      }}
    >
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
        <Button variant="ghost" size="sm" onClick={dismiss} disabled={saving}>
          Cancel
        </Button>
      </div>
    </div>
  );
}
