import { useEffect, useState } from "react";
import { Loader2, X } from "lucide-react";

import type {
  AccountViewDto,
  IpcError,
  RecurringBillDto,
  RecurringBillOccurrenceDto,
  UpdateRecurringBillInput,
} from "@/bindings";
import { commands } from "@/bindings";
import { Input } from "@/components/ui/input";
import { PaginationControls } from "@/components/ui/pagination";
import {
  formatDate,
  formatIsoDate,
  formatMoney,
  formatSignedMoney,
  signedAmountClass,
} from "@/lib/format";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { cn } from "@/lib/utils";
import { describeIpcError } from "@/vault/useVault";
import { EMPTY_FILTERS } from "@/transactions/filters";
import { usePagedTransactions } from "@/transactions/usePagedTransactions";
import { frequencyLabel } from "@/lib/frequency";
import {
  BILL_TYPE_LABELS,
  BillForm,
  frequencyFormDefaults,
  minorUnitsToInput,
} from "./BillsView";

/// A bill's detail side-panel (personal-cfo-4d8.24.7.2): a right-side drawer to edit
/// the bill and browse the history of transactions confirmed as paying it. Mirrors
/// `TransactionDetailDrawer`'s bespoke fixed-panel pattern (backdrop + right panel,
/// Escape / backdrop-click to close). Controlled by `BillsView`, which owns which bill
/// is open and threads `updateBill` in.
export function BillDetailDrawer({
  bill,
  accounts,
  onClose,
  onUpdate,
}: {
  bill: RecurringBillDto;
  accounts: AccountViewDto[];
  onClose: () => void;
  onUpdate: (input: UpdateRecurringBillInput) => Promise<IpcError | null>;
}) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const subtitle = [
    BILL_TYPE_LABELS[bill.bill_type] ?? bill.bill_type,
    frequencyLabel(bill.frequency),
    bill.next_due_date ? `due ${formatIsoDate(bill.next_due_date)}` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      <div className="absolute inset-0 bg-foreground/40" aria-hidden onClick={onClose} />
      <div
        role="dialog"
        aria-label="Bill detail"
        className="relative flex h-full w-full max-w-md flex-col gap-4 overflow-y-auto border-l bg-background p-6 shadow-xl"
      >
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <h2 className="truncate text-lg font-semibold tracking-tight">
              {bill.name}
            </h2>
            <p className="text-xs text-muted-foreground">{subtitle}</p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <span className="font-medium tabular-nums text-loss">
              {formatMoney(bill.amount)}
            </span>
            <button
              type="button"
              onClick={onClose}
              aria-label="Close"
              className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
            >
              <X className="size-4" aria-hidden />
            </button>
          </div>
        </div>

        <section className="flex flex-col gap-2" aria-label="Edit bill">
          <h3 className="text-sm font-semibold">Edit</h3>
          <BillForm
            // Remount per bill so the form's defaultValues reset when a different bill
            // opens (RHF only reads defaultValues on mount).
            key={bill.id}
            accounts={accounts}
            submitLabel="Save"
            fallbackCurrency={bill.amount.currency}
            showCategory
            defaultValues={{
              name: bill.name,
              bill_type: bill.bill_type,
              amount: minorUnitsToInput(bill.amount.minor_units),
              ...frequencyFormDefaults(bill.frequency),
              anchor: bill.anchor_date,
              autopay_account_id: bill.autopay_account_id ?? "",
              autopay: bill.autopay_enabled,
              description: bill.description ?? "",
              category_id: bill.category_id ?? "",
              tag_ids: bill.tag_ids,
            }}
            onCancel={onClose}
            onSubmit={async (draft) => {
              const failure = await onUpdate({
                bill_id: bill.id,
                ...draft,
                idempotency_key: mintIdempotencyKey(),
              });
              if (!failure) onClose();
              return failure ? describeIpcError(failure) : null;
            }}
          />
        </section>

        <BillPayments billId={bill.id} />

        <BillMatchedHistory billId={bill.id} />
      </div>
    </div>
  );
}

/// The bill's retro-attached history (ADR 0047 §1, personal-cfo-4d8.25.8): the schedule's
/// occurrences that matched a REALIZED transaction via the drift-tolerant instance seam —
/// automatic evidence, distinct from the explicit confirmed payments above. Fetched on
/// open (the read refreshes the seam, so a just-created bill shows its matches
/// immediately).
function BillMatchedHistory({ billId }: { billId: string }) {
  const [rows, setRows] = useState<RecurringBillOccurrenceDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void commands.recurringBillHistory(billId).then((result) => {
      if (cancelled) return;
      if (result.status === "ok") setRows(result.data);
      else setError(describeIpcError(result.error));
    });
    return () => {
      cancelled = true;
    };
  }, [billId]);

  const linked = (rows ?? []).filter(
    (occurrence) => occurrence.linked_transaction_id !== null,
  );
  // Newest first, capped for the panel; the count line carries the full total.
  const shown = [...linked].reverse().slice(0, 8);

  return (
    <section className="flex flex-col gap-2" aria-label="Matched history">
      <h3 className="text-sm font-semibold">Matched history</h3>
      {rows === null && !error && (
        <p className="flex items-center gap-2 py-1 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" aria-hidden /> Matching this bill&apos;s
          schedule against your history…
        </p>
      )}
      {error && (
        <p role="alert" className="text-sm text-loss">
          {error}
        </p>
      )}
      {rows !== null && linked.length === 0 && (
        <p className="py-1 text-sm text-muted-foreground">
          No past transactions matched this bill&apos;s schedule yet.
        </p>
      )}
      {linked.length > 0 && (
        <>
          <p className="text-xs text-muted-foreground">
            {linked.length} past transaction{linked.length === 1 ? "" : "s"} matched this
            bill&apos;s schedule.
          </p>
          <ul className="flex flex-col">
            {shown.map((occurrence) => (
              <li
                key={occurrence.scheduled_date}
                className="flex items-center justify-between gap-3 border-b py-1.5 text-sm last:border-0"
              >
                <span className="text-muted-foreground">
                  {formatIsoDate(occurrence.scheduled_date)}
                </span>
                <span className="tabular-nums">
                  {formatMoney({
                    minor_units: occurrence.expected_amount_minor,
                    currency: occurrence.currency,
                  })}
                </span>
              </li>
            ))}
          </ul>
          {linked.length > shown.length && (
            <p className="text-xs text-muted-foreground">
              +{linked.length - shown.length} more
            </p>
          )}
        </>
      )}
    </section>
  );
}

/// The bill's confirmed-payment history (personal-cfo-4d8.24.7.1/.2): a paginated,
/// searchable list scoped server-side to `recurring_event_id = bill.id` via
/// `confirmed_obligations` — the transactions the user confirmed as paying this bill,
/// not a memo/counterparty guess. (`filterTransactions` on the client can't mirror the
/// join, so the list is authoritative only through the server query.)
function BillPayments({ billId }: { billId: string }) {
  const [search, setSearch] = useState("");
  const { rows, error, pagination } = usePagedTransactions(
    { ...EMPTY_FILTERS, recurringEventId: billId, query: search },
    "newest",
    "pcfo.pageSize.billPayments",
  );

  return (
    <section className="flex flex-col gap-2" aria-label="Payments">
      <h3 className="text-sm font-semibold">Payments</h3>
      <Input
        type="search"
        value={search}
        onChange={(event) => setSearch(event.target.value)}
        placeholder="Search payments…"
        aria-label="Search payments"
      />
      {error && (
        <p role="alert" className="text-sm text-loss">
          {error}
        </p>
      )}
      {rows.length === 0 && !error ? (
        <p className="py-2 text-sm text-muted-foreground">
          {search.trim() === ""
            ? "No linked payments yet. Confirm a bill as paid to see it here."
            : "No payments match your search."}
        </p>
      ) : (
        <ul className="flex flex-col">
          {rows.map((txn) => (
            <li
              key={txn.transaction_id}
              className="flex items-center justify-between gap-3 border-b py-2 text-sm last:border-0"
            >
              <span className="min-w-0">
                <span className="block truncate">
                  {txn.memo ?? txn.counterparty ?? txn.account_name}
                </span>
                <span className="block text-xs text-muted-foreground">
                  {formatDate(txn.occurred_at)}
                </span>
              </span>
              <span className={cn("shrink-0 tabular-nums", signedAmountClass(txn.amount))}>
                {formatSignedMoney(txn.amount)}
              </span>
            </li>
          ))}
        </ul>
      )}
      <PaginationControls pagination={pagination} noun="payments" />
    </section>
  );
}
