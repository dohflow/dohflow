import { useState, useId } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { Loader2, Plus, Receipt, Search } from "lucide-react";

import type {
  AccountViewDto,
  IpcError,
  MoneyDto,
  RecurringBillDto,
  RecurringCandidateDto,
  TransactionRowDto,
} from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent } from "@/components/ui/card";
import { NativeSelect } from "@/components/ui/native-select";
import { describeIpcError } from "@/vault/useVault";
import { AppliedOverrideNote } from "@/future-cash/AppliedOverrideNote";
import {
  useAppliedOverrides,
  type AppliedOverride,
} from "@/future-cash/useAppliedOverrides";
import {
  dollarsToMinorUnits,
  formatDate,
  formatIsoDate,
  formatMoney,
} from "@/lib/format";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { useAccounts } from "@/accounts/useAccounts";
import {
  CLASSIC_FREQUENCIES,
  INTERVAL_UNITS,
  buildIntervalToken,
  frequencyLabel,
  isValidIntervalCount,
  parseIntervalToken,
  type IntervalUnit,
} from "@/lib/frequency";
import { useCategories } from "@/categories/useCategories";
import { categoryLabels } from "@/categories/labels";
import { useTags } from "@/tags/useTags";
import { cn } from "@/lib/utils";
import { useBaseCurrency } from "@/settings/useBaseCurrency";
import { BillDetailDrawer } from "./BillDetailDrawer";
import { useBills } from "./useBills";

const SELECT_CLASS =
  "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

const TEXTAREA_CLASS =
  "flex min-h-16 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

// The bill-contract type tokens the backend accepts, paired with display labels.
const BILL_TYPES: { value: string; label: string }[] = [
  { value: "subscription", label: "Subscription" },
  { value: "utility", label: "Utility" },
  { value: "rent_mortgage", label: "Rent / mortgage" },
  { value: "insurance", label: "Insurance" },
  { value: "loan_payment", label: "Loan payment" },
  { value: "tax", label: "Tax" },
  { value: "membership", label: "Membership" },
  { value: "childcare", label: "Childcare" },
  { value: "other", label: "Other" },
];

export const BILL_TYPE_LABELS: Record<string, string> = Object.fromEntries(
  BILL_TYPES.map((type) => [type.value, type.label]),
);

// One anchor meaning across every creation path (ADR 0047 §3): a date the cadence is
// measured from — ideally a real occurrence. Past and future both work; the displayed
// next-due always derives forward from today.
const ANCHOR_HELP =
  "The schedule counts from this date — past and future occurrences derive from it. Any date the bill actually happened works.";

// The classic pay-frequency tokens plus the custom-interval escape hatch
// (ADR 0048): "custom" is a FORM-ONLY value resolved to an every_<n>_<unit>
// token on submit — it never reaches the backend.
const FREQUENCIES: { value: string; label: string }[] = [
  ...CLASSIC_FREQUENCIES,
  { value: "custom", label: "Custom interval…" },
];

/// Seed the form's frequency fields from a stored token: an interval token selects
/// "custom" with its parts; anything else selects the token itself.
export function frequencyFormDefaults(token: string): {
  frequency: string;
  interval_count: string;
  interval_unit: string;
} {
  const interval = parseIntervalToken(token);
  if (interval) {
    return {
      frequency: "custom",
      interval_count: String(interval.n),
      interval_unit: interval.unit,
    };
  }
  return { frequency: token, interval_count: "", interval_unit: "months" };
}

/// How the bill lists are ordered (personal-cfo-yequ, mirroring the transactions
/// filter bar). "Next due" is the default — the soonest obligation tops the list.
type BillSort = "next_due" | "name" | "amount_desc" | "amount_asc";

const BILL_SORT_OPTIONS: { value: BillSort; label: string }[] = [
  { value: "next_due", label: "Next due" },
  { value: "name", label: "Name A–Z" },
  { value: "amount_desc", label: "Amount high" },
  { value: "amount_asc", label: "Amount low" },
];

/// Case-insensitive match on the two free-text fields a user remembers a bill by.
function filterBills(bills: RecurringBillDto[], query: string): RecurringBillDto[] {
  const needle = query.trim().toLowerCase();
  if (needle === "") return bills;
  return bills.filter((bill) =>
    [bill.name, bill.description].some(
      (field) => field !== null && field.toLowerCase().includes(needle),
    ),
  );
}

/// Order a (filtered) bill list; sorts a copy so the backend order is untouched.
/// Bills without a next due date (e.g. archived) sink to the end of "Next due".
function sortBills(bills: RecurringBillDto[], sort: BillSort): RecurringBillDto[] {
  const sorted = [...bills];
  switch (sort) {
    case "next_due":
      sorted.sort((a, b) => {
        if (a.next_due_date === null) return b.next_due_date === null ? 0 : 1;
        if (b.next_due_date === null) return -1;
        return a.next_due_date.localeCompare(b.next_due_date);
      });
      break;
    case "name":
      sorted.sort((a, b) => a.name.localeCompare(b.name));
      break;
    case "amount_desc":
      sorted.sort((a, b) => b.amount.minor_units - a.amount.minor_units);
      break;
    case "amount_asc":
      sorted.sort((a, b) => a.amount.minor_units - b.amount.minor_units);
      break;
  }
  return sorted;
}

/// Today as `YYYY-MM-DD` in the user's locale (the `<input type="date">` value).
function today(): string {
  return new Date().toLocaleDateString("en-CA");
}

/// A wire minor-units amount as an editable major-unit string, e.g. 1250 → "12.5".
/// Both supported currencies (USD/EUR) are 2-decimal, matching `dollarsToMinorUnits`.
export function minorUnitsToInput(minorUnits: number): string {
  return (minorUnits / 100).toString();
}

/// `embedded` hides the section's own heading inside the unified Transactions
/// hub (personal-cfo-xdbm), where it is the "Recurring" sub-view.
export function BillsView({
  embedded = false,
  onOpenScenario,
}: {
  embedded?: boolean;
  /// Take the user to the scenario that overrode a bill (personal-cfo-abhr). Optional so
  /// an embedded Bills list without a router still renders.
  onOpenScenario?: (scenarioId: string) => void;
} = {}) {
  // What an APPLIED scenario changed about each bill (ADR 0055). Applying promotes
  // assumption events into base rather than editing the bill, so the stored amount and
  // the forecast's amount legitimately differ — the row has to say so.
  const { overrideFor } = useAppliedOverrides();
  const { bills, error, addBill, updateBill, deleteBill, archiveBill, restoreBill } =
    useBills();
  const { accounts } = useAccounts();
  const { baseCurrency } = useBaseCurrency();
  const [adding, setAdding] = useState(false);
  // Which bill's detail drawer is open (personal-cfo-4d8.24.7.2). Held by id, then
  // re-derived from the live `bills` so an edit-save refresh flows into the open drawer.
  const [selectedBillId, setSelectedBillId] = useState<string | null>(null);
  // Search + sort over the lists (personal-cfo-yequ); local view state only.
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<BillSort>("next_due");

  const activeBills = bills?.filter((bill) => bill.active) ?? [];
  const archivedBills = bills?.filter((bill) => !bill.active) ?? [];
  const selectedBill = bills?.find((bill) => bill.id === selectedBillId) ?? null;
  const visibleActive = sortBills(filterBills(activeBills, query), sort);
  const visibleArchived = sortBills(filterBills(archivedBills, query), sort);

  return (
    <div className={embedded ? "flex flex-col gap-4" : "mx-auto flex w-full max-w-2xl flex-col gap-4"}>
      <div className="flex items-center justify-between">
        {embedded ? (
          <span />
        ) : (
          <h2 className="text-lg font-semibold tracking-tight">Bills</h2>
        )}
        {!adding && (
          <Button size="sm" onClick={() => setAdding(true)}>
            <Plus aria-hidden />
            Add bill
          </Button>
        )}
      </div>

      {adding && (
        <Card>
          <CardContent className="pt-6">
            <BillForm
              accounts={accounts ?? []}
              submitLabel="Add bill"
              fallbackCurrency={baseCurrency}
              defaultValues={{
                name: "",
                bill_type: "subscription",
                amount: "",
                frequency: "monthly",
                interval_count: "",
                interval_unit: "months",
                anchor: today(),
                autopay_account_id: "",
                autopay: false,
                description: "",
                category_id: "",
                tag_ids: [],
              }}
              onCancel={() => setAdding(false)}
              onSubmit={async (draft) => {
                const { error: failure } = await addBill({
                  ...draft,
                  source_merchant_key: null,
                  idempotency_key: mintIdempotencyKey(),
                });
                if (!failure) setAdding(false);
                return failure ? describeIpcError(failure) : null;
              }}
            />
          </CardContent>
        </Card>
      )}

      {error && (
        <p role="alert" className="text-sm text-loss">
          {error}
        </p>
      )}

      {bills !== null && bills.length > 0 && (
        <div className="flex items-center gap-2">
          <div className="relative flex-1">
            <Search
              className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
              aria-hidden
            />
            <Input
              type="search"
              role="searchbox"
              aria-label="Search bills"
              placeholder="Search bills…"
              className="pl-9"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
          <NativeSelect
            aria-label="Sort bills"
            value={sort}
            onChange={(e) => setSort(e.target.value as BillSort)}
            className="w-40 shrink-0"
          >
            {BILL_SORT_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </NativeSelect>
        </div>
      )}

      {bills === null ? (
        <div className="flex items-center justify-center gap-2 py-10 text-muted-foreground">
          <Loader2 className="size-5 animate-spin" aria-hidden />
          Loading bills…
        </div>
      ) : bills.length === 0 ? (
        !adding && (
          <Card>
            <CardContent className="flex flex-col items-center gap-2 py-10 text-center">
              <Receipt className="size-8 text-muted-foreground" aria-hidden />
              <p className="font-medium">No bills yet</p>
              <p className="text-sm text-muted-foreground">
                Add a recurring bill to project your upcoming cash flow.
              </p>
            </CardContent>
          </Card>
        )
      ) : (
        <>
          {visibleActive.length > 0 ? (
            <Card>
              <CardContent className="p-0">
                <ul>
                  {visibleActive.map((bill) => (
                    <BillRow
                      key={bill.id}
                      bill={bill}
                      onOpenDrawer={(b) => setSelectedBillId(b.id)}
                      override={overrideFor(bill.id)}
                      onOpenScenario={onOpenScenario}
                      onDelete={deleteBill}
                      onArchive={archiveBill}
                      onRestore={restoreBill}
                    />
                  ))}
                </ul>
              </CardContent>
            </Card>
          ) : activeBills.length > 0 ? (
            // Bills exist but the search hid them all — say so, not "no active bills".
            <Card>
              <CardContent className="py-8 text-center text-sm text-muted-foreground">
                No bills match — clear the search.
              </CardContent>
            </Card>
          ) : (
            !adding && (
              <Card>
                <CardContent className="py-8 text-center text-sm text-muted-foreground">
                  No active bills.
                </CardContent>
              </Card>
            )
          )}

          {visibleArchived.length > 0 && (
            <div className="flex flex-col gap-2">
              <h3 className="text-sm font-medium text-muted-foreground">
                Archived
              </h3>
              <Card>
                <CardContent className="p-0">
                  <ul>
                    {visibleArchived.map((bill) => (
                      <BillRow
                        key={bill.id}
                        bill={bill}
                        onOpenDrawer={(b) => setSelectedBillId(b.id)}
                      override={overrideFor(bill.id)}
                      onOpenScenario={onOpenScenario}
                        onDelete={deleteBill}
                        onArchive={archiveBill}
                        onRestore={restoreBill}
                      />
                    ))}
                  </ul>
                </CardContent>
              </Card>
            </div>
          )}
        </>
      )}

      {selectedBill && (
        <BillDetailDrawer
          bill={selectedBill}
          accounts={accounts ?? []}
          onClose={() => setSelectedBillId(null)}
          onUpdate={updateBill}
        />
      )}
    </div>
  );
}

// A single bill row. Active bills show Edit / Archive / Delete (Delete behind a
// two-click confirm). Archived bills (personal-cfo-4d8.2) are dimmed, show their
// created + archived dates, and offer Restore.
function BillRow({
  bill,
  onOpenDrawer,
  onDelete,
  onArchive,
  onRestore,
  override,
  onOpenScenario,
}: {
  bill: RecurringBillDto;
  onOpenDrawer: (bill: RecurringBillDto) => void;
  /// Set when an applied scenario overrides this bill's forecast figure.
  override?: AppliedOverride;
  onOpenScenario?: (scenarioId: string) => void;
  onDelete: (billId: string) => Promise<IpcError | null>;
  onArchive: (billId: string) => Promise<IpcError | null>;
  onRestore: (billId: string) => Promise<IpcError | null>;
}) {
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function runAction(action: () => Promise<IpcError | null>) {
    setBusy(true);
    setError(null);
    const failure = await action();
    setBusy(false);
    if (failure) setError(describeIpcError(failure));
  }

  // Archived bills: dimmed, with their history dates + a Restore action.
  if (!bill.active) {
    return (
      <li className="flex flex-col gap-1 border-b px-5 py-3 opacity-60 last:border-0">
        <div className="flex items-center justify-between gap-3">
          <button
            type="button"
            onClick={() => onOpenDrawer(bill)}
            className="min-w-0 flex-1 text-left"
            aria-label={`Open ${bill.name} detail`}
          >
            <div className="font-medium">{bill.name}</div>
            <div className="text-xs text-muted-foreground">
              {BILL_TYPE_LABELS[bill.bill_type] ?? bill.bill_type}
              {` · ${frequencyLabel(bill.frequency)}`}
              {` · added ${formatDate(bill.created_at)}`}
              {bill.archived_at && ` · archived ${formatDate(bill.archived_at)}`}
            </div>
            {bill.description && (
              <div className="mt-0.5 text-xs text-muted-foreground">
                {bill.description}
              </div>
            )}
          </button>
          <div className="flex items-center gap-3">
            <span className="font-medium tabular-nums text-muted-foreground">
              {formatMoney(bill.amount)}
            </span>
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() => runAction(() => onRestore(bill.id))}
              aria-label={`Restore ${bill.name}`}
            >
              Restore
            </Button>
          </div>
        </div>
        {error && (
          <p role="alert" className="text-xs text-loss">
            {error}
          </p>
        )}
      </li>
    );
  }

  async function runDelete() {
    setBusy(true);
    setError(null);
    const failure = await onDelete(bill.id);
    setBusy(false);
    if (failure) {
      setError(describeIpcError(failure));
      setConfirmingDelete(false);
    }
    // On success the row vanishes (the bills query is invalidated).
  }

  return (
    <li className="flex flex-col gap-1 border-b px-5 py-3 last:border-0">
      <div className="flex items-center justify-between gap-3">
        <button
          type="button"
          onClick={() => onOpenDrawer(bill)}
          className="min-w-0 flex-1 text-left"
          aria-label={`Open ${bill.name} detail`}
        >
          <div className="flex items-center gap-2">
            <span className="font-medium">{bill.name}</span>
            {bill.autopay_enabled && (
              <Badge variant="gain" className="text-[10px] uppercase tracking-wide">
                Autopay
              </Badge>
            )}
          </div>
          <div className="text-xs text-muted-foreground">
            {BILL_TYPE_LABELS[bill.bill_type] ?? bill.bill_type}
            {` · ${frequencyLabel(bill.frequency)}`}
            {bill.next_due_date && ` · due ${formatIsoDate(bill.next_due_date)}`}
            {bill.autopay_account_name && ` · ${bill.autopay_account_name}`}
          </div>
          {bill.description && (
            <div className="mt-0.5 text-xs text-muted-foreground">
              {bill.description}
            </div>
          )}
        </button>
        <div className="flex items-center gap-3">
          <span className="font-medium tabular-nums text-loss">
            {formatMoney(bill.amount)}
          </span>
          <div className="flex gap-1">
            {confirmingDelete ? (
              <>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={runDelete}
                  aria-label={`Confirm delete ${bill.name}`}
                >
                  Confirm
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  type="button"
                  onClick={() => setConfirmingDelete(false)}
                >
                  Cancel
                </Button>
              </>
            ) : (
              <>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={() => runAction(() => onArchive(bill.id))}
                  aria-label={`Archive ${bill.name}`}
                >
                  Archive
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={() => setConfirmingDelete(true)}
                  aria-label={`Delete ${bill.name}`}
                >
                  Delete
                </Button>
              </>
            )}
          </div>
        </div>
      </div>
      {/* Outside the row's open-detail button on purpose: this note contains its own
          button, and a button inside a button is invalid. */}
      {override && onOpenScenario && (
        <AppliedOverrideNote
          override={override}
          currency={bill.amount.currency}
          onOpenScenario={onOpenScenario}
        />
      )}
      {error && (
        <p role="alert" className="text-xs text-loss">
          {error}
        </p>
      )}
    </li>
  );
}

// The normalized fields a bill form emits; the caller wraps it into the create or
// update IPC input (adding `idempotency_key`, and `bill_id` for an edit).
type BillDraft = {
  name: string;
  amount: MoneyDto;
  bill_type: string;
  frequency: string;
  anchor_date: string;
  autopay_account_id: string | null;
  autopay: boolean;
  description: string | null;
  category_id: string | null;
  tag_ids: string[];
};

// Rust remains authoritative for validation (ADR 0003); this schema only gates
// the form UI and shapes display copy.
const billFormSchema = z.object({
  name: z.string().trim().min(1, "Enter a bill name."),
  bill_type: z.string().min(1),
  amount: z.string().refine((value) => {
    const minor = dollarsToMinorUnits(value);
    return minor !== null && minor > 0;
  }, "Enter an amount above zero."),
  frequency: z.string().min(1),
  // Parts of a custom interval (ADR 0048); only read when frequency === "custom".
  interval_count: z.string(),
  interval_unit: z.string(),
  anchor: z.string().min(1, "Pick an anchor date."),
  // "" means no autopay account is linked.
  autopay_account_id: z.string(),
  // Whether this bill pays itself (ADR 0041) — distinct from which account it uses.
  autopay: z.boolean(),
  // Optional free-text note.
  description: z.string(),
  // "" means uncategorized (personal-cfo-4d8.24.5).
  category_id: z.string(),
  // Tag ids applied to the bill (personal-cfo-4d8.24.5.1).
  tag_ids: z.array(z.string()),
});
type BillFormValues = z.infer<typeof billFormSchema>;

// Shared add/edit form. `fallbackCurrency` is used when no autopay account is
// linked (USD for a new bill, the bill's existing currency for an edit) so an edit
// never silently changes the currency of an accountless bill.
// Exported so the First Forecast Wizard (personal-cfo-uipt) reuses it verbatim.
export function BillForm({
  accounts,
  defaultValues,
  submitLabel,
  fallbackCurrency,
  onCancel,
  onSubmit,
  // Category is chosen when creating/promoting (personal-cfo-4d8.24.5); the edit form
  // hides it (editing a bill's category is a follow-up), but a hidden field keeps the
  // pre-filled value so it round-trips unchanged.
  showCategory = true,
}: {
  accounts: AccountViewDto[];
  defaultValues: BillFormValues;
  submitLabel: string;
  fallbackCurrency: string;
  onCancel: () => void;
  onSubmit: (draft: BillDraft) => Promise<string | null>;
  showCategory?: boolean;
}) {
  const {
    register,
    handleSubmit,
    watch,
    setValue,
    formState: { errors, isSubmitting },
  } = useForm<BillFormValues>({
    resolver: zodResolver(billFormSchema),
    defaultValues,
  });
  // Unique per instance: the wizard's own form and a promoted suggestion's
  // form can be mounted together (nhmsg review; mirrors IncomeForm).
  const uid = useId();
  const [error, setError] = useState<string | null>(null);
  // Categorize + tag the bill up front so its forecasted occurrences + confirmed
  // payments inherit them (personal-cfo-4d8.24.5 / .5.1).
  const { categories } = useCategories();
  const { options: categoryOptions } = categoryLabels(categories ?? []);
  const { tags } = useTags();
  const selectedTagIds = watch("tag_ids");
  const toggleTag = (tagId: string) => {
    const next = selectedTagIds.includes(tagId)
      ? selectedTagIds.filter((id) => id !== tagId)
      : [...selectedTagIds, tagId];
    setValue("tag_ids", next, { shouldDirty: true });
  };

  const submit = handleSubmit(async (values) => {
    setError(null);
    const magnitude = dollarsToMinorUnits(values.amount);
    if (magnitude === null) return;
    // The bill currency must match the autopay account when one is linked
    // (enforced by the db worker); without an account, keep the fallback.
    const autopayAccount = accounts.find(
      (account) => account.id === values.autopay_account_id,
    );
    const currency = autopayAccount?.balance.currency ?? fallbackCurrency;
    const description = values.description.trim();
    // Resolve the form-only "custom" selection into the wire token (ADR 0048),
    // mirroring the backend bounds so rejection happens before the IPC call.
    let frequency = values.frequency;
    if (frequency === "custom") {
      const n = Number(values.interval_count);
      const unit = values.interval_unit as IntervalUnit;
      if (!isValidIntervalCount(n, unit)) {
        const max = INTERVAL_UNITS.find((u) => u.value === unit)?.max;
        setError(`Enter a whole number of ${unit} between 1 and ${max}.`);
        return;
      }
      frequency = buildIntervalToken(n, unit);
    }
    const draft: BillDraft = {
      name: values.name,
      amount: { minor_units: magnitude, currency },
      bill_type: values.bill_type,
      frequency,
      anchor_date: values.anchor,
      autopay_account_id:
        values.autopay_account_id === "" ? null : values.autopay_account_id,
      autopay: values.autopay,
      description: description === "" ? null : description,
      category_id: values.category_id === "" ? null : values.category_id,
      tag_ids: values.tag_ids,
    };
    const failure = await onSubmit(draft);
    if (failure) setError(failure);
  });

  return (
    <form onSubmit={submit} className="flex flex-col gap-4">
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${uid}-name`}>Bill name</Label>
        <Input
          id={`${uid}-name`}
          autoFocus
          {...register("name")}
          placeholder="e.g. Netflix"
          aria-invalid={!!errors.name}
        />
        {errors.name && (
          <p className="text-xs text-loss">{errors.name.message}</p>
        )}
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${uid}-type`}>Type</Label>
          <select
            id={`${uid}-type`}
            className={SELECT_CLASS}
            {...register("bill_type")}
          >
            {BILL_TYPES.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${uid}-amount`}>Amount</Label>
          <Input
            id={`${uid}-amount`}
            inputMode="decimal"
            {...register("amount")}
            placeholder="0.00"
            aria-invalid={!!errors.amount}
          />
          {errors.amount && (
            <p className="text-xs text-loss">{errors.amount.message}</p>
          )}
        </div>
      </div>

      {showCategory ? (
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${uid}-category`}>Category</Label>
          <select
            id={`${uid}-category`}
            className={SELECT_CLASS}
            {...register("category_id")}
          >
            <option value="">Uncategorized</option>
            {categoryOptions.map((option) => (
              <option key={option.id} value={option.id}>
                {option.label}
              </option>
            ))}
          </select>
        </div>
      ) : (
        // Keep the pre-filled category in the form so it round-trips unchanged on edit.
        <input type="hidden" {...register("category_id")} />
      )}

      {showCategory && (tags ?? []).length > 0 && (
        <div className="flex flex-col gap-1.5">
          <Label>Tags</Label>
          <div className="flex flex-wrap gap-1.5" role="group" aria-label="Bill tags">
            {(tags ?? []).map((tag) => {
              const active = selectedTagIds.includes(tag.id);
              return (
                <button
                  key={tag.id}
                  type="button"
                  aria-pressed={active}
                  onClick={() => toggleTag(tag.id)}
                  className={cn(
                    "rounded-full border px-3 py-0.5 text-xs transition-colors",
                    active
                      ? "border-primary bg-primary/10 text-primary"
                      : "border-input text-muted-foreground hover:bg-muted",
                  )}
                >
                  {tag.name}
                </button>
              );
            })}
          </div>
        </div>
      )}

      <div className="grid grid-cols-2 gap-3">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${uid}-frequency`}>Frequency</Label>
          <select
            id={`${uid}-frequency`}
            className={SELECT_CLASS}
            {...register("frequency")}
          >
            {FREQUENCIES.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
          {watch("frequency") === "custom" && (
            <div className="flex items-center gap-1.5">
              <span className="text-sm text-muted-foreground">Every</span>
              <Input
                aria-label="Interval count"
                inputMode="numeric"
                className="h-9 w-16"
                {...register("interval_count")}
              />
              <select
                aria-label="Interval unit"
                className={SELECT_CLASS + " w-auto"}
                {...register("interval_unit")}
              >
                {INTERVAL_UNITS.map((unit) => (
                  <option key={unit.value} value={unit.value}>
                    {unit.label}
                  </option>
                ))}
              </select>
            </div>
          )}
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${uid}-anchor`}>Anchor date</Label>
          <Input id={`${uid}-anchor`} type="date" {...register("anchor")} />
          <p className="text-xs text-muted-foreground">{ANCHOR_HELP}</p>
        </div>
      </div>

      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${uid}-account`}>Autopay account</Label>
        <select
          id={`${uid}-account`}
          className={SELECT_CLASS}
          {...register("autopay_account_id")}
        >
          <option value="">No autopay account</option>
          {accounts.map((account) => (
            <option key={account.id} value={account.id}>
              {account.name}
            </option>
          ))}
        </select>
      </div>

      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          className="size-4 rounded border-input"
          {...register("autopay")}
        />
        This bill pays itself (autopay)
      </label>

      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${uid}-description`}>Description (optional)</Label>
        <textarea
          id={`${uid}-description`}
          className={TEXTAREA_CLASS}
          {...register("description")}
          placeholder="e.g. Family plan, renews each spring"
        />
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
        <Button type="submit" disabled={isSubmitting}>
          {isSubmitting ? "Saving…" : submitLabel}
        </Button>
      </div>
    </form>
  );
}

/// Promote a transaction to a recurring bill (personal-cfo-5n4.6): a pre-filled
/// [`BillForm`] opened from a transaction-review surface. Name comes from the
/// merchant/memo, amount + currency from the transaction, and pay-from defaults to
/// the transaction's own account — liquid OR a credit card (per personal-cfo-6wk.8).
/// The user edits before confirming; on submit it creates the recurring event via
/// the standard `CreateRecurringBill` path. Self-contained so any surface can host it.
export function MakeRecurringBillForm({
  transaction,
  onCancel,
  onCreated,
}: {
  transaction: TransactionRowDto;
  onCancel: () => void;
  onCreated: () => void;
}) {
  const { accounts } = useAccounts();
  const { addBill } = useBills();

  // Wait for accounts so BillForm mounts with the pay-from option already present —
  // its autopay <select> is uncontrolled, so a late-arriving option would leave the
  // defaulted account unshown even though it is set (personal-cfo-5n4.6 review).
  if (accounts === null) {
    return (
      <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
        <Loader2 className="size-4 animate-spin" aria-hidden /> Loading…
      </div>
    );
  }

  return (
    <BillForm
      accounts={accounts}
      submitLabel="Create recurring bill"
      // The transaction's own currency is the right fallback: it matches the pay-from
      // account, and holds even if that account isn't in the loaded list.
      fallbackCurrency={transaction.amount.currency}
      defaultValues={{
        name: (transaction.counterparty ?? transaction.memo ?? "").trim(),
        bill_type: "subscription",
        // A transaction amount is signed from the account's view; a bill amount is a
        // positive magnitude.
        amount: minorUnitsToInput(Math.abs(transaction.amount.minor_units)),
        frequency: "monthly",
        interval_count: "",
        interval_unit: "months",
        // RFC 3339 → the `<input type="date">` YYYY-MM-DD value.
        anchor: transaction.occurred_at.slice(0, 10),
        autopay_account_id: transaction.account_id,
        autopay: false,
        description: "",
        // Pre-fill the bill's category from the source transaction (4d8.24.5).
        category_id: transaction.category_id ?? "",
        tag_ids: transaction.tag_ids,
      }}
      onCancel={onCancel}
      onSubmit={async (draft) => {
        // Promoted from a single transaction, not a detected merchant candidate.
        const { error: failure } = await addBill({
          ...draft,
          source_merchant_key: null,
          idempotency_key: mintIdempotencyKey(),
        });
        if (!failure) onCreated();
        return failure ? describeIpcError(failure) : null;
      }}
    />
  );
}

/// Promote a detected recurring candidate (personal-cfo-98ql) to a bill: a pre-filled
/// [`BillForm`] seeded from the candidate's inferred amount / frequency / last observed
/// occurrence (the anchor, ADR 0047 §3). Pay-from prefills when the series is single-account
/// (ADR 0047 §4); a multi-account merchant leaves it for the user. Reuses the standard
/// create path; on success the
/// candidate drops out (it is now tracked) and `onCreated` receives the new bill's id so
/// the caller can surface its retro-attached history (ADR 0047 §1).
export function RecurringCandidateForm({
  candidate,
  onCancel,
  onCreated,
}: {
  candidate: RecurringCandidateDto;
  onCancel: () => void;
  onCreated: (eventId: string | null) => void;
}) {
  const { accounts } = useAccounts();
  const { addBill } = useBills();

  if (accounts === null) {
    return (
      <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
        <Loader2 className="size-4 animate-spin" aria-hidden /> Loading…
      </div>
    );
  }

  return (
    <BillForm
      accounts={accounts}
      submitLabel="Create recurring bill"
      fallbackCurrency={candidate.currency}
      defaultValues={{
        name: candidate.display,
        bill_type: "subscription",
        amount: minorUnitsToInput(candidate.amount_minor),
        ...frequencyFormDefaults(candidate.frequency),
        // The last OBSERVED occurrence, not the synthetic next-expected date (ADR 0047
        // §3): the anchor is "a real occurrence the cadence counts from", and the
        // schedule lattice is identical either way — a real date is better provenance.
        anchor: candidate.last_seen,
        // Prefill pay-from when every observation posted to ONE account (ADR 0047 §4,
        // personal-cfo-4d8.25.11) — "always on Venture X" makes the account knowable;
        // a multi-account series stays blank for the user to pick.
        autopay_account_id: candidate.source_account_id ?? "",
        autopay: false,
        description: "",
        // Pre-fill from the candidate's dominant category when the detector found one
        // (personal-cfo-4d8.24.5); the user can still change or clear it before creating.
        category_id: candidate.dominant_category_id ?? "",
        tag_ids: [],
      }}
      onCancel={onCancel}
      onSubmit={async (draft) => {
        // Persist the candidate's merchant key so the suggestion stays suppressed even
        // if the bill is later renamed (personal-cfo-5n4.8).
        const { error: failure, eventId } = await addBill({
          ...draft,
          source_merchant_key: candidate.merchant_key,
          idempotency_key: mintIdempotencyKey(),
        });
        if (!failure) onCreated(eventId);
        return failure ? describeIpcError(failure) : null;
      }}
    />
  );
}
