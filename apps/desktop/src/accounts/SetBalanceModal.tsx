import { useEffect, useState } from "react";
import { Loader2, X } from "lucide-react";

import { commands } from "@/bindings";
import type { AccountViewDto, IpcError, MoneyDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { describeIpcError } from "@/vault/useVault";
import { dollarsToMinorUnits, formatMoney, formatSignedMoney } from "@/lib/format";
import { useAccounts } from "./useAccounts";
import {
  enteredToStoredMinor,
  figureLabelForRole,
  storedToShownMinor,
} from "./balanceSign";

/// Today as `YYYY-MM-DD` in the user's locale (the `<input type="date">` value).
function today(): string {
  return new Date().toLocaleDateString("en-CA");
}

/// A wire minor-units amount as an editable major-unit string (USD/EUR are 2-dp).
function minorUnitsToInput(minorUnits: number): string {
  return (minorUnits / 100).toString();
}

/// An unexplained plug counts as "fully explained" when it is absent or zero.
function isExplained(plug: MoneyDto | null): boolean {
  return plug === null || plug.minor_units === 0;
}

/// Set an account's balance directly (personal-cfo-hxjj, ADR 0027 additive model).
/// The user asserts "this is what the account holds, as of this date" with no
/// required transaction; the kernel records a balance assertion and returns the
/// still-unexplained adjustment — the auto-reconciling "plug" (asserted minus the
/// transactions recorded so far). The plug is shown for transparency and shrinks on
/// its own as real transactions get recorded. The user can explain it now via the
/// optional reconcile-with-transactions path, or later. This is the primary balance
/// path; `ReconcileBalanceModal` (4d8.5) is the secondary one.
export function SetBalanceModal({
  account,
  onClose,
  onExplainWithTransactions,
}: {
  account: AccountViewDto;
  onClose: () => void;
  onExplainWithTransactions: () => void;
}) {
  const { assertBalance, convertUnexplained } = useAccounts();
  const currency = account.balance.currency;

  const role = account.cashflow_role;
  const [balance, setBalance] = useState(
    minorUnitsToInput(storedToShownMinor(role, account.balance.minor_units)),
  );
  const [date, setDate] = useState(today());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // `null` until the balance has been asserted; afterwards holds the new balance
  // and the unexplained plug so the body can switch to the outcome view.
  const [outcome, setOutcome] = useState<{
    balance: MoneyDto;
    unexplained: MoneyDto | null;
  } | null>(null);

  // Close on Escape — this is a modal surface over the app shell.
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function save() {
    const entered = dollarsToMinorUnits(balance);
    if (entered === null) {
      setError("Enter a valid amount.");
      return;
    }
    const minorUnits = enteredToStoredMinor(role, entered);
    setBusy(true);
    setError(null);
    const { result, error: failure } = await assertBalance({
      account_id: account.id,
      amount: { minor_units: minorUnits, currency },
      as_of_date: date,
    });
    setBusy(false);
    if (failure || result === null) {
      setError(failure ? describeIpcError(failure) : "Could not set the balance.");
      return;
    }
    setOutcome({ balance: result.balance, unexplained: result.unexplained });
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
              {outcome
                ? "Balance updated."
                : "Enter what the account holds — no transactions required."}
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

        {outcome ? (
          <SetBalanceOutcome
            role={role}
            outcome={outcome}
            onExplainWithTransactions={onExplainWithTransactions}
            onConvert={async () => {
              const failure = await convertUnexplained(account.id);
              // Re-read the plug rather than assuming it cleared: whether the
              // conversion fully explains the balance is the backend's call
              // (ADR 0027; the yl53 review caught the optimistic null hiding
              // a residual that had NOT cleared).
              if (!failure) {
                const fresh = await commands.accountUnexplained(account.id);
                setOutcome((current) =>
                  current
                    ? {
                        ...current,
                        unexplained:
                          fresh.status === "ok" ? fresh.data : current.unexplained,
                      }
                    : current,
                );
              }
              return failure;
            }}
            onDone={onClose}
          />
        ) : (
          <>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="set-balance-amount">
                New {figureLabelForRole(role).toLowerCase()}
              </Label>
              <Input
                id="set-balance-amount"
                inputMode="decimal"
                value={balance}
                onChange={(event) => setBalance(event.target.value)}
                placeholder="0.00"
                autoFocus
              />
              <p className="text-xs text-muted-foreground">
                Currently{" "}
                {formatMoney({
                  minor_units: storedToShownMinor(role, account.balance.minor_units),
                  currency,
                })}
                .
              </p>
            </div>

            <div className="flex flex-col gap-1.5">
              <Label htmlFor="set-balance-date">As of</Label>
              <Input
                id="set-balance-date"
                type="date"
                value={date}
                onChange={(event) => setDate(event.target.value)}
              />
            </div>

            {error && (
              <p role="alert" className="text-sm text-loss">
                {error}
              </p>
            )}

            <div className="flex justify-end gap-2">
              <Button type="button" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button type="button" disabled={busy} onClick={save}>
                {busy && <Loader2 className="size-4 animate-spin" aria-hidden />}
                Set balance
              </Button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/// The post-assert view: confirm the new balance and, if anything is still
/// unexplained, surface the plug with the option to explain it with transactions
/// now (the 4d8.5 path) or later.
function SetBalanceOutcome({
  role,
  outcome,
  onExplainWithTransactions,
  onConvert,
  onDone,
}: {
  role: string;
  outcome: { balance: MoneyDto; unexplained: MoneyDto | null };
  onExplainWithTransactions: () => void;
  onConvert: () => Promise<IpcError | null>;
  onDone: () => void;
}) {
  const explained = isExplained(outcome.unexplained);
  const [converting, setConverting] = useState(false);
  const [convertError, setConvertError] = useState<string | null>(null);

  async function convert() {
    setConverting(true);
    setConvertError(null);
    const failure = await onConvert();
    setConverting(false);
    if (failure) setConvertError(describeIpcError(failure));
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center justify-between rounded-md bg-muted px-3 py-2 text-sm">
        <span className="text-muted-foreground">New balance</span>
        <span className="font-medium tabular-nums">
          {formatMoney({
            minor_units: storedToShownMinor(role, outcome.balance.minor_units),
            currency: outcome.balance.currency,
          })}
        </span>
      </div>

      {explained ? (
        <p className="text-sm text-gain">Fully explained by your transactions ✓</p>
      ) : (
        <div className="flex flex-col gap-2">
          <div className="flex items-center justify-between rounded-md border border-dashed px-3 py-2 text-sm">
            <span className="text-muted-foreground">Unexplained adjustment</span>
            <span className="font-medium tabular-nums">
              {/* `explained` guarantees a non-null plug here. */}
              {formatSignedMoney({
                minor_units: storedToShownMinor(
                  role,
                  (outcome.unexplained as MoneyDto).minor_units,
                ),
                currency: (outcome.unexplained as MoneyDto).currency,
              })}
            </span>
          </div>
          <p className="text-xs text-muted-foreground">
            This is the gap between the balance you set and the transactions
            recorded so far. It shrinks on its own as you add the transactions that
            explain it — or record it as one transaction now.
          </p>
          {convertError && (
            <p role="alert" className="text-xs text-loss">
              {convertError}
            </p>
          )}
        </div>
      )}

      <div className="flex flex-wrap justify-end gap-2">
        {!explained && (
          <>
            <Button
              type="button"
              variant="ghost"
              onClick={onExplainWithTransactions}
            >
              Explain with transactions
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={converting}
              onClick={convert}
            >
              {converting && (
                <Loader2 className="size-4 animate-spin" aria-hidden />
              )}
              Record as a transaction
            </Button>
          </>
        )}
        <Button type="button" onClick={onDone}>
          Done
        </Button>
      </div>
    </div>
  );
}
