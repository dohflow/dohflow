import { useEffect, useState } from "react";
import { Loader2, X } from "lucide-react";

import type { AccountViewDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { describeIpcError } from "@/vault/useVault";
import { dollarsToMinorUnits, formatMoney, formatSignedMoney } from "@/lib/format";
import {
  enteredToStoredMinor,
  figureLabelForRole,
  storedToShownMinor,
} from "./balanceSign";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { useTransactions } from "@/transactions/useTransactions";

const SELECT_CLASS =
  "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

/// Today as `YYYY-MM-DD` in the user's locale (the `<input type="date">` value).
function today(): string {
  return new Date().toLocaleDateString("en-CA");
}

/// A wire minor-units amount as an editable major-unit string (USD/EUR are 2-dp).
function minorUnitsToInput(minorUnits: number): string {
  return (minorUnits / 100).toString();
}

/// Reconcile an account's balance to a target by recording the real transactions
/// that explain the gap (personal-cfo-4d8.5). Rather than force-setting a number,
/// it nudges the user to enter the postings they forgot: each one is a genuine
/// `record_transaction` ledger entry — the reconciling transactions ARE the
/// record. The remaining delta counts down as they're added.
export function ReconcileBalanceModal({
  account,
  onClose,
}: {
  account: AccountViewDto;
  onClose: () => void;
}) {
  const { addTransaction } = useTransactions();
  const currency = account.balance.currency;
  const role = account.cashflow_role;
  // Balance at open time; the live balance is `openingMinor + applied`. The
  // `account` prop is a snapshot, but capture it so the math stays stable.
  const [openingMinor] = useState(account.balance.minor_units);
  const [applied, setApplied] = useState(0);

  const [target, setTarget] = useState(
    minorUnitsToInput(storedToShownMinor(account.cashflow_role, openingMinor)),
  );
  const [amount, setAmount] = useState("");
  const [directionOverride, setDirectionOverride] = useState<"in" | "out" | null>(
    null,
  );
  const [date, setDate] = useState(today());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const targetEntered = dollarsToMinorUnits(target);
  const targetMinor =
    targetEntered === null ? null : enteredToStoredMinor(role, targetEntered);
  const remainingMinor =
    targetMinor === null ? null : targetMinor - openingMinor - applied;
  const reconciled = remainingMinor === 0;

  // Default the direction to whatever closes the gap; the user can override it.
  const gapDirection: "in" | "out" =
    remainingMinor !== null && remainingMinor < 0 ? "out" : "in";
  const direction = directionOverride ?? gapDirection;

  // Close on Escape — this is a modal surface over the app shell.
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function addReconciling() {
    const magnitude = dollarsToMinorUnits(amount);
    if (magnitude === null || magnitude <= 0) {
      setError("Enter an amount above zero.");
      return;
    }
    const signed = direction === "in" ? magnitude : -magnitude;
    setBusy(true);
    setError(null);
    const failure = await addTransaction({
      account_id: account.id,
      amount: { minor_units: signed, currency },
      occurred_at: `${date}T12:00:00Z`,
      idempotency_key: mintIdempotencyKey(),
    });
    setBusy(false);
    if (failure) {
      setError(describeIpcError(failure));
      return;
    }
    setApplied((prev) => prev + signed);
    setAmount("");
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div
        className="absolute inset-0 bg-foreground/40"
        aria-hidden
        onClick={onClose}
      />
      <div
        role="dialog"
        aria-label={`Set balance for ${account.name}`}
        className="relative flex w-full max-w-md flex-col gap-4 rounded-lg border bg-background p-6 shadow-xl"
      >
        <div className="flex items-start justify-between">
          <div>
            <div className="font-semibold">Set balance · {account.name}</div>
            <div className="text-xs text-muted-foreground">
              Record the transactions that explain the difference.
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X className="size-4" aria-hidden />
          </button>
        </div>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="reconcile-target">
            Target {figureLabelForRole(role).toLowerCase()}
          </Label>
          <Input
            id="reconcile-target"
            inputMode="decimal"
            value={target}
            onChange={(event) => setTarget(event.target.value)}
          />
          <p className="text-xs text-muted-foreground">
            Starting from{" "}
            {formatMoney({
              minor_units: storedToShownMinor(role, openingMinor),
              currency,
            })}
            .
          </p>
        </div>

        <div className="flex items-center justify-between rounded-md bg-muted px-3 py-2 text-sm">
          <span className="text-muted-foreground">Remaining to reconcile</span>
          {remainingMinor === null ? (
            <span className="text-muted-foreground">—</span>
          ) : reconciled ? (
            <span className="font-medium text-gain">Reconciled ✓</span>
          ) : (
            <span className="font-medium tabular-nums">
              {/* The remaining gap stays in the stored frame so its sign lines up with the
                  in/out direction picker below (a positive gap ⇒ "money in" closes it). */}
              {formatSignedMoney({ minor_units: remainingMinor, currency })}
            </span>
          )}
        </div>

        {!reconciled && (
          <div className="flex flex-col gap-3">
            <div className="grid grid-cols-2 gap-3">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="reconcile-amount">Amount</Label>
                <Input
                  id="reconcile-amount"
                  inputMode="decimal"
                  value={amount}
                  onChange={(event) => setAmount(event.target.value)}
                  placeholder="0.00"
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="reconcile-direction">Type</Label>
                <select
                  id="reconcile-direction"
                  className={SELECT_CLASS}
                  value={direction}
                  onChange={(event) =>
                    setDirectionOverride(event.target.value as "in" | "out")
                  }
                >
                  <option value="in">Money in</option>
                  <option value="out">Money out</option>
                </select>
              </div>
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="reconcile-date">Date</Label>
              <Input
                id="reconcile-date"
                type="date"
                value={date}
                onChange={(event) => setDate(event.target.value)}
              />
            </div>
          </div>
        )}

        {error && (
          <p role="alert" className="text-sm text-loss">
            {error}
          </p>
        )}

        <div className="flex justify-end gap-2">
          {!reconciled && (
            <Button type="button" disabled={busy} onClick={addReconciling}>
              {busy && <Loader2 className="size-4 animate-spin" aria-hidden />}
              Add &amp; new
            </Button>
          )}
          <Button type="button" variant="ghost" onClick={onClose}>
            Done
          </Button>
        </div>
      </div>
    </div>
  );
}
