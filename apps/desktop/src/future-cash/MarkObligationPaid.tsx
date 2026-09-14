import { useState } from "react";
import { CheckCircle2, Loader2 } from "lucide-react";

import type { ForecastEventDto } from "@/bindings";
import { useAccounts } from "@/accounts/useAccounts";
import { useBills } from "@/bills/useBills";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { dollarsToMinorUnits, todayInTimezone } from "@/lib/format";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { useHouseholdTimezone } from "@/settings/useHouseholdTimezone";
import { describeIpcError } from "@/vault/useVault";

import { useMarkObligation } from "./useMarkObligation";
import { useMarkObligationUndoPublish } from "./markObligationUndo";

const SELECT_CLASS =
  "flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

function minorUnitsToInput(minorUnits: number): string {
  return (Math.abs(minorUnits) / 100).toString();
}

/// The "mark this bill occurrence paid" control for the Future Cash projected-activity panel
/// (personal-cfo-5ie.9). Only rendered for a recurring bill / loan-payment occurrence. Posts a
/// real outflow from a liquid account and removes the occurrence from the forecast. Card/loan
/// liability-crediting isn't supported yet (r7sb), so a card-charged bill is refused by the
/// backend — we surface that message.
export function MarkObligationPaid({
  event,
  scheduledDate,
}: {
  event: ForecastEventDto;
  scheduledDate: string;
}) {
  const { accounts } = useAccounts();
  const { bills } = useBills();
  const { confirm } = useMarkObligation();
  const publishConfirmed = useMarkObligationUndoPublish();
  // The default/max pay date (ADR 0021 §1, personal-cfo-ku2hn): the household-local
  // calendar date, matching the backend's future-date guard on ConfirmObligationEarly —
  // not the browser's own local date, which `toISOString()` (UTC) used to fall back to
  // here and could disagree with by a day near either midnight.
  const { timezone } = useHouseholdTimezone();
  const todayIso = todayInTimezone(timezone);

  const liquid = (accounts ?? []).filter(
    (a) => a.cashflow_role === "liquid_cash" && a.active,
  );
  const bill = bills?.find((b) => b.id === event.source_event_id);
  const autopay = bill?.autopay_account_id ?? null;
  // An autopay bill pays itself, so the action reads "confirm it cleared" (ADR 0041, mc7f).
  const isAutopay = bill?.autopay_enabled ?? false;
  const actionLabel = isAutopay ? "Confirm it cleared" : "Mark paid";
  const defaultAccount =
    (autopay && liquid.some((a) => a.id === autopay)
      ? autopay
      : liquid[0]?.id) ?? "";

  const [open, setOpen] = useState(false);
  const [amount, setAmount] = useState(() =>
    minorUnitsToInput(event.amount.minor_units),
  );
  // Derive the paying account from the user's choice, falling back to the default — so it fills
  // in once accounts load (the choice state alone would stay empty).
  const [payFromChoice, setPayFromChoice] = useState("");
  const payFrom = payFromChoice || defaultAccount;
  // Default the paid-date to the DUE date, not today: confirming a bill due
  // months ago should record it when it was due (owner dogfooding, 6zk3). A
  // future occurrence (early confirm) clamps to today to satisfy max=today.
  //
  // Derived like `payFrom` above, not a one-time `useState` default: `todayIso` depends on
  // `useHouseholdTimezone()`'s query, which is still "UTC" (the loading fallback) on the
  // very first render — a one-time default would lock that in permanently even after the
  // real household timezone loads moments later (personal-cfo-q329).
  const [whenDraft, setWhenDraft] = useState<string | null>(null);
  const when = whenDraft ?? (scheduledDate <= todayIso ? scheduledDate : todayIso);
  const [error, setError] = useState<string | null>(null);

  if (liquid.length === 0) return null;

  async function onConfirm() {
    setError(null);
    // Zero is allowed — "nothing due this cycle" still clears the occurrence. Reject only a
    // blank/unparseable or negative amount.
    const minor = dollarsToMinorUnits(amount);
    if (minor === null || minor < 0) {
      setError("Enter a valid amount (0 or more).");
      return;
    }
    const result = await confirm.mutateAsync({
      recurring_event_id: event.source_event_id,
      scheduled_date: scheduledDate,
      actual_amount: { minor_units: minor, currency: event.amount.currency },
      actual_date: when,
      paying_account_id: payFrom,
      idempotency_key: mintIdempotencyKey(),
    });
    if (result.status === "error") {
      setError(describeIpcError(result.error));
      return;
    }
    // On success the forecast invalidates and this row disappears — surface an Undo at the
    // view level (the row is gone, so it can't live here).
    publishConfirmed?.({
      eventId: event.source_event_id,
      scheduledDate,
      name: event.name,
    });
  }

  if (!open) {
    return (
      <div className="px-4 pb-3">
        <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
          <CheckCircle2 aria-hidden />
          {actionLabel}
        </Button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 px-4 pb-4">
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <div className="flex flex-col gap-1">
          <Label htmlFor="mp-amount" className="text-xs">
            Amount paid
          </Label>
          <Input
            id="mp-amount"
            inputMode="decimal"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1">
          <Label htmlFor="mp-account" className="text-xs">
            Paid from
          </Label>
          <select
            id="mp-account"
            className={SELECT_CLASS}
            value={payFrom}
            onChange={(e) => setPayFromChoice(e.target.value)}
          >
            {liquid.map((a) => (
              <option key={a.id} value={a.id}>
                {a.name}
              </option>
            ))}
          </select>
        </div>
        <div className="flex flex-col gap-1">
          <Label htmlFor="mp-date" className="text-xs">
            Date paid
          </Label>
          <Input
            id="mp-date"
            type="date"
            max={todayIso}
            value={when}
            onChange={(e) => setWhenDraft(e.target.value)}
          />
        </div>
      </div>
      {error && (
        <p role="alert" className="text-sm text-loss">
          {error}
        </p>
      )}
      {/* Right-aligned with Confirm outermost: the confirm action lands in
          the same screen position as the trigger button it replaced, so rapid
          review keeps the pointer still (owner dogfooding, 6zk3). */}
      <div className="flex justify-end gap-2">
        <Button variant="ghost" size="sm" onClick={() => setOpen(false)}>
          Cancel
        </Button>
        <Button
          size="sm"
          onClick={() => void onConfirm()}
          disabled={confirm.isPending || !payFrom}
        >
          {confirm.isPending ? (
            <Loader2 className="animate-spin" aria-hidden />
          ) : null}
          Confirm payment
        </Button>
      </div>
    </div>
  );
}
