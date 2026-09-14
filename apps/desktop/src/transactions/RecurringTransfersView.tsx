import { useState } from "react";
import { ArrowLeftRight, Loader2, Plus } from "lucide-react";

import type { IpcError, RecurringTransferDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { describeIpcError } from "@/vault/useVault";
import { formatIsoDate, formatMoney } from "@/lib/format";
import { frequencyLabel } from "@/lib/frequency";
import { useAccounts } from "@/accounts/useAccounts";
import { AddTransferForm } from "./TransactionsView";
import { useRecurringTransfers } from "./useRecurringTransfers";

/// The scheduled account-to-account transfers (ADR 0026 §14, personal-cfo-npoe),
/// shown under the Recurring sub-tab beside the bills. Created right here (the
/// "Add recurring transfer" button, personal-cfo-4d8.25.12 — the shared
/// [`AddTransferForm`] locked to recurring mode) or from Activity → Add transfer;
/// each row can be deleted (which stops future projection).
export function RecurringTransfersView() {
  const { recurringTransfers, error, deleteRecurringTransfer, addRecurringTransfer } =
    useRecurringTransfers();
  const { accounts } = useAccounts();
  const [adding, setAdding] = useState(false);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold tracking-tight">
          Recurring transfers
        </h3>
        {!adding && (
          <Button
            size="sm"
            // The locked form skips the one-off/recurring toggle, so this button
            // carries the toggle's own guard: a recurring transfer must be funded
            // from cash (adversarial review of 4d8.25.12).
            disabled={accounts !== null && !accounts.some((a) => a.cashflow_role === "liquid_cash")}
            title={
              accounts !== null && !accounts.some((a) => a.cashflow_role === "liquid_cash")
                ? "Recurring transfers need a cash account"
                : undefined
            }
            onClick={() => setAdding(true)}
          >
            <Plus aria-hidden />
            Add recurring transfer
          </Button>
        )}
      </div>

      {error && (
        <p role="alert" className="text-sm text-loss">
          {error}
        </p>
      )}

      {adding &&
        // Gate on loaded accounts: the form computes its defaults once on mount,
        // so mounting it with an empty list would leave dead selects that never
        // re-seed when the accounts arrive (same guard as RecurringCandidateForm).
        (accounts === null ? (
          <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" aria-hidden /> Loading…
          </div>
        ) : (
          <AddTransferForm
            accounts={accounts}
            defaultRecurring
            lockRecurring
            onCancel={() => setAdding(false)}
            onCreateRecurring={async (input) => {
              const failure = await addRecurringTransfer(input);
              if (!failure) setAdding(false);
              return failure ? describeIpcError(failure) : null;
            }}
          />
        ))}

      {recurringTransfers === null ? (
        <div className="flex items-center justify-center gap-2 py-6 text-muted-foreground">
          <Loader2 className="size-5 animate-spin" aria-hidden /> Loading…
        </div>
      ) : recurringTransfers.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No scheduled transfers yet. Add one here, or from Activity → Add
          transfer.
        </p>
      ) : (
        <Card>
          <CardContent className="p-0">
            <ul>
              {recurringTransfers.map((transfer) => (
                <RecurringTransferRow
                  key={transfer.id}
                  transfer={transfer}
                  onDelete={deleteRecurringTransfer}
                />
              ))}
            </ul>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

function RecurringTransferRow({
  transfer,
  onDelete,
}: {
  transfer: RecurringTransferDto;
  onDelete: (id: string) => Promise<IpcError | null>;
}) {
  const [busy, setBusy] = useState(false);
  const [rowError, setRowError] = useState<string | null>(null);

  async function remove() {
    setBusy(true);
    setRowError(null);
    const failure = await onDelete(transfer.id);
    setBusy(false);
    if (failure) setRowError(describeIpcError(failure));
  }

  return (
    <li className="flex flex-col gap-1 border-b px-5 py-3 last:border-0">
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-0.5">
          <div className="flex items-center gap-1.5 font-medium">
            <span className="truncate">{transfer.source_account_name}</span>
            <ArrowLeftRight
              className="size-3.5 shrink-0 text-muted-foreground"
              aria-hidden
            />
            <span className="truncate">{transfer.dest_account_name}</span>
          </div>
          <div className="text-xs text-muted-foreground">
            {frequencyLabel(transfer.frequency)}
            {transfer.next_date && ` · next ${formatIsoDate(transfer.next_date)}`}
          </div>
        </div>
        <div className="flex items-center gap-3">
          <span className="font-medium tabular-nums">
            {formatMoney(transfer.amount)}
          </span>
          <Button
            size="sm"
            variant="ghost"
            disabled={busy}
            onClick={remove}
            aria-label={`Delete transfer from ${transfer.source_account_name} to ${transfer.dest_account_name}`}
          >
            Delete
          </Button>
        </div>
      </div>
      {rowError && (
        <p role="alert" className="text-xs text-loss">
          {rowError}
        </p>
      )}
    </li>
  );
}
