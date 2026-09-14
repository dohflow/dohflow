import { useState, type FormEvent } from "react";
import { CalendarPlus, Pencil, Trash2 } from "lucide-react";

import type { ManualFutureEntryDto } from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  dollarsToMinorUnits,
  formatIsoDate,
  formatSignedMoney,
  signedAmountClass,
} from "@/lib/format";
import { cn } from "@/lib/utils";
import { describeIpcError } from "@/vault/useVault";
import { useAccounts } from "@/accounts/useAccounts";
import { useManualEntries } from "./useManualEntries";

const SELECT_CLASS =
  "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

/// Today as `YYYY-MM-DD` in local time (the `<input type="date">` default).
function todayIso(): string {
  const now = new Date();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${now.getFullYear()}-${month}-${day}`;
}

type Direction = "in" | "out";

interface EntryDraft {
  amount: { minor_units: number; currency: string };
  date: string;
  label: string;
  account_id: string | null;
}

/// The user's manual future entries (personal-cfo-q6gh): an add/edit form plus an
/// editable list. Adding, editing, or deleting an entry refreshes the forecast (the
/// hook invalidates the forecast cache), so the chart and ledger update live.
export function FutureCashEntries({ currency }: { currency: string }) {
  const { entries, error, addEntry, updateEntry, deleteEntry } =
    useManualEntries();
  const [open, setOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);

  function close() {
    setOpen(false);
    setEditingId(null);
  }

  const editing =
    editingId && entries ? (entries.find((e) => e.id === editingId) ?? null) : null;

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-3">
        <CardTitle className="text-sm">Your future entries</CardTitle>
        {!open && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setEditingId(null);
              setOpen(true);
            }}
          >
            <CalendarPlus aria-hidden />
            Add entry
          </Button>
        )}
      </CardHeader>
      <CardContent className="flex flex-col gap-3 p-0">
        {error && (
          <p role="alert" className="px-6 text-sm text-loss">
            {error}
          </p>
        )}

        {open && (
          <EntryForm
            key={editingId ?? "new"}
            currency={currency}
            initial={editing}
            onCancel={close}
            onSubmit={async (draft) => {
              const failure = editingId
                ? await updateEntry({ id: editingId, ...draft })
                : await addEntry(draft);
              if (!failure) close();
              return failure ? describeIpcError(failure) : null;
            }}
          />
        )}

        {entries && entries.length > 0 ? (
          <ul>
            {entries.map((entry) => (
              <li
                key={entry.id}
                className="flex items-center justify-between gap-4 border-t px-6 py-2.5 text-sm"
              >
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="truncate font-medium">
                      {entry.label || "—"}
                    </span>
                    {entry.matched_transaction_id ? (
                      <Badge variant="gain">Matched</Badge>
                    ) : null}
                  </div>
                  <div className="text-xs text-muted-foreground">
                    {entry.matched_transaction_id
                      ? `${formatIsoDate(entry.date)} · a recorded transaction matches this entry — it no longer counts in the forecast`
                      : formatIsoDate(entry.date)}
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  <span
                    className={cn("tabular-nums", signedAmountClass(entry.amount))}
                  >
                    {formatSignedMoney(entry.amount)}
                  </span>
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label={`Edit ${entry.label}`}
                    onClick={() => {
                      setEditingId(entry.id);
                      setOpen(true);
                    }}
                  >
                    <Pencil aria-hidden />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label={`Delete ${entry.label}`}
                    onClick={() => void deleteEntry(entry.id)}
                  >
                    <Trash2 aria-hidden />
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        ) : (
          !open && (
            <p className="px-6 pb-6 text-sm text-muted-foreground">
              Add a one-time future inflow or outflow to fold it into your projection.
            </p>
          )
        )}
      </CardContent>
    </Card>
  );
}

function EntryForm({
  currency,
  initial,
  onCancel,
  onSubmit,
}: {
  currency: string;
  initial: ManualFutureEntryDto | null;
  onCancel: () => void;
  onSubmit: (draft: EntryDraft) => Promise<string | null>;
}) {
  const [amount, setAmount] = useState(
    initial ? (Math.abs(initial.amount.minor_units) / 100).toString() : "",
  );
  const [direction, setDirection] = useState<Direction>(
    initial && initial.amount.minor_units < 0 ? "out" : "in",
  );
  const [date, setDate] = useState(initial?.date ?? todayIso());
  const [label, setLabel] = useState(initial?.label ?? "");
  const [accountId, setAccountId] = useState(initial?.account_id ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Only liquid accounts can be a from/to for cash (ADR 0028); the rest of the model
  // (cards/loans/investments) isn't a cash source.
  const { accounts } = useAccounts();
  const liquidAccounts =
    accounts?.filter((a) => a.cashflow_role === "liquid_cash") ?? [];

  async function submit(event: FormEvent) {
    event.preventDefault();
    const magnitude = dollarsToMinorUnits(amount);
    if (magnitude === null || magnitude <= 0) {
      setError("Enter an amount greater than zero.");
      return;
    }
    if (!label.trim()) {
      setError("Add a short label.");
      return;
    }
    setBusy(true);
    setError(null);
    const minor = direction === "out" ? -magnitude : magnitude;
    const failure = await onSubmit({
      amount: { minor_units: minor, currency },
      date,
      label: label.trim(),
      account_id: accountId === "" ? null : accountId,
    });
    setBusy(false);
    if (failure) setError(failure);
  }

  return (
    <form
      onSubmit={submit}
      className="flex flex-col gap-3 border-y bg-muted/30 px-6 py-4"
    >
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="entry-label">Label</Label>
          <Input
            id="entry-label"
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            placeholder="e.g. Bonus"
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="entry-date">Date</Label>
          <Input
            id="entry-date"
            type="date"
            value={date}
            onChange={(e) => setDate(e.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="entry-amount">Amount ({currency})</Label>
          <Input
            id="entry-amount"
            inputMode="decimal"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            placeholder="0.00"
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <span className="text-sm font-medium">Direction</span>
          <div
            className="inline-flex rounded-md border p-0.5"
            role="group"
            aria-label="Direction"
          >
            {(["in", "out"] as const).map((option) => (
              <button
                key={option}
                type="button"
                onClick={() => setDirection(option)}
                aria-pressed={direction === option}
                className={cn(
                  "flex-1 rounded px-2.5 py-1 text-xs font-medium transition-colors",
                  direction === option
                    ? "bg-secondary text-secondary-foreground"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {option === "in" ? "Money in" : "Money out"}
              </button>
            ))}
          </div>
        </div>
        <div className="flex flex-col gap-1.5 sm:col-span-2">
          <Label htmlFor="entry-account">
            {direction === "out" ? "From account" : "To account"}
          </Label>
          <select
            id="entry-account"
            aria-label={direction === "out" ? "From account" : "To account"}
            className={SELECT_CLASS}
            value={accountId}
            onChange={(e) => setAccountId(e.target.value)}
          >
            <option value="">Unallocated (no account)</option>
            {liquidAccounts.map((account) => (
              <option key={account.id} value={account.id}>
                {account.name}
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

      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" disabled={busy}>
          {initial ? "Save changes" : "Add entry"}
        </Button>
      </div>
    </form>
  );
}
