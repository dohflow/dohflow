import { useState, useId } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { Banknote, Loader2, Plus } from "lucide-react";

import type {
  AccountViewDto,
  IncomeSourceDto,
  IpcError,
  MoneyDto,
  UpdateIncomeSourceInput,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent } from "@/components/ui/card";
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
import { useBaseCurrency } from "@/settings/useBaseCurrency";
import { SuggestedIncome } from "./SuggestedIncome";
import { useIncome } from "./useIncome";

const SELECT_CLASS =
  "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

// The pay-frequency tokens the backend accepts (`Frequency::from_token`), paired
// with their display labels. Drives both the form dropdown and the list subline.
const FREQUENCIES: { value: string; label: string }[] = [
  { value: "weekly", label: "Weekly" },
  { value: "biweekly", label: "Biweekly" },
  { value: "semi_monthly", label: "Semimonthly" },
  { value: "monthly", label: "Monthly" },
  { value: "quarterly", label: "Quarterly" },
  { value: "annual", label: "Annual" },
];

const FREQUENCY_LABELS: Record<string, string> = Object.fromEntries(
  FREQUENCIES.map((frequency) => [frequency.value, frequency.label]),
);

/// Today as `YYYY-MM-DD` in the user's locale (the `<input type="date">` value).
function today(): string {
  return new Date().toLocaleDateString("en-CA");
}

/// A wire minor-units amount as an editable major-unit string, e.g. 1250 → "12.5".
export function minorUnitsToInput(minorUnits: number): string {
  return (minorUnits / 100).toString();
}

export function IncomeView({
  onOpenScenario,
}: {
  /// Take the user to the scenario that overrode an income source
  /// (personal-cfo-abhr). Optional so a mount without a router still renders.
  onOpenScenario?: (scenarioId: string) => void;
} = {}) {
  // What an APPLIED scenario changed about each source (ADR 0055). Applying promotes
  // assumption events into base rather than editing the source, so the stored amount and
  // the forecast's amount legitimately differ.
  const { overrideFor } = useAppliedOverrides();
  const {
    sources,
    error,
    addIncomeSource,
    updateIncomeSource,
    deleteIncomeSource,
    archiveIncomeSource,
    restoreIncomeSource,
  } = useIncome();
  const { accounts } = useAccounts();
  const { baseCurrency } = useBaseCurrency();
  const [adding, setAdding] = useState(false);

  const activeSources = sources?.filter((source) => source.active) ?? [];
  const archivedSources = sources?.filter((source) => !source.active) ?? [];

  return (
    <div className="mx-auto flex w-full max-w-2xl flex-col gap-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold tracking-tight">Income</h2>
        {!adding && (
          <Button size="sm" onClick={() => setAdding(true)}>
            <Plus aria-hidden />
            Add income
          </Button>
        )}
      </div>
      <SuggestedIncome />

      {adding && (
        <Card>
          <CardContent className="pt-6">
            <IncomeForm
              accounts={accounts ?? []}
              submitLabel="Add income"
              fallbackCurrency={baseCurrency}
              defaultValues={{
                name: "",
                amount: "",
                frequency: "biweekly",
                anchor: today(),
                deposit_account_id: "",
              }}
              onCancel={() => setAdding(false)}
              onSubmit={async (draft) => {
                const failure = await addIncomeSource({
                  ...draft,
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

      {sources === null ? (
        <div className="flex items-center justify-center gap-2 py-10 text-muted-foreground">
          <Loader2 className="size-5 animate-spin" aria-hidden />
          Loading income…
        </div>
      ) : sources.length === 0 ? (
        !adding && (
          <Card>
            <CardContent className="flex flex-col items-center gap-2 py-10 text-center">
              <Banknote className="size-8 text-muted-foreground" aria-hidden />
              <p className="font-medium">No income yet</p>
              <p className="text-sm text-muted-foreground">
                Add a paycheck or other recurring income to project your cash
                flow.
              </p>
            </CardContent>
          </Card>
        )
      ) : (
        <>
          {activeSources.length > 0 ? (
            <Card>
              <CardContent className="p-0">
                <ul>
                  {activeSources.map((source) => (
                    <IncomeRow
                      override={overrideFor(source.id)}
                      onOpenScenario={onOpenScenario}
                      key={source.id}
                      source={source}
                      accounts={accounts ?? []}
                      onUpdate={updateIncomeSource}
                      onDelete={deleteIncomeSource}
                      onArchive={archiveIncomeSource}
                      onRestore={restoreIncomeSource}
                    />
                  ))}
                </ul>
              </CardContent>
            </Card>
          ) : (
            !adding && (
              <Card>
                <CardContent className="py-8 text-center text-sm text-muted-foreground">
                  No active income.
                </CardContent>
              </Card>
            )
          )}

          {archivedSources.length > 0 && (
            <div className="flex flex-col gap-2">
              <h3 className="text-sm font-medium text-muted-foreground">
                Archived
              </h3>
              <Card>
                <CardContent className="p-0">
                  <ul>
                    {archivedSources.map((source) => (
                      <IncomeRow
                        override={overrideFor(source.id)}
                        onOpenScenario={onOpenScenario}
                        key={source.id}
                        source={source}
                        accounts={accounts ?? []}
                        onUpdate={updateIncomeSource}
                        onDelete={deleteIncomeSource}
                        onArchive={archiveIncomeSource}
                        onRestore={restoreIncomeSource}
                      />
                    ))}
                  </ul>
                </CardContent>
              </Card>
            </div>
          )}
        </>
      )}
    </div>
  );
}

// A single income row. Active sources show Edit / Archive / Delete (Delete behind a
// two-click confirm). Archived sources are dimmed, show their created + archived
// dates, and offer Restore (personal-cfo-tch0).
function IncomeRow({
  source,
  accounts,
  onUpdate,
  onDelete,
  onArchive,
  onRestore,
  override,
  onOpenScenario,
}: {
  source: IncomeSourceDto;
  /// Set when an applied scenario overrides this source's forecast figure.
  override?: AppliedOverride;
  onOpenScenario?: (scenarioId: string) => void;
  accounts: AccountViewDto[];
  onUpdate: (input: UpdateIncomeSourceInput) => Promise<IpcError | null>;
  onDelete: (id: string) => Promise<IpcError | null>;
  onArchive: (id: string) => Promise<IpcError | null>;
  onRestore: (id: string) => Promise<IpcError | null>;
}) {
  const [editing, setEditing] = useState(false);
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

  // Archived sources: dimmed, with their history dates + a Restore action.
  if (!source.active) {
    return (
      <li className="flex flex-col gap-1 border-b px-5 py-3 opacity-60 last:border-0">
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            <div className="font-medium">{source.name}</div>
            <div className="text-xs text-muted-foreground">
              {FREQUENCY_LABELS[source.frequency] ?? source.frequency}
              {` · added ${formatDate(source.created_at)}`}
              {source.archived_at &&
                ` · archived ${formatDate(source.archived_at)}`}
            </div>
          </div>
          <div className="flex items-center gap-3">
            <span className="font-medium tabular-nums text-muted-foreground">
              {formatMoney(source.net_amount)}
            </span>
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() => runAction(() => onRestore(source.id))}
              aria-label={`Restore ${source.name}`}
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

  if (editing) {
    return (
      <li className="border-b px-5 py-4 last:border-0">
        <IncomeForm
          accounts={accounts}
          submitLabel="Save"
          fallbackCurrency={source.net_amount.currency}
          defaultValues={{
            name: source.name,
            amount: minorUnitsToInput(source.net_amount.minor_units),
            frequency: source.frequency,
            anchor: source.anchor_date,
            deposit_account_id: source.deposit_account_id ?? "",
          }}
          onCancel={() => setEditing(false)}
          onSubmit={async (draft) => {
            const failure = await onUpdate({
              income_source_id: source.id,
              ...draft,
              idempotency_key: mintIdempotencyKey(),
            });
            if (!failure) setEditing(false);
            return failure ? describeIpcError(failure) : null;
          }}
        />
      </li>
    );
  }

  async function runDelete() {
    setBusy(true);
    setError(null);
    const failure = await onDelete(source.id);
    setBusy(false);
    if (failure) {
      setError(describeIpcError(failure));
      setConfirmingDelete(false);
    }
    // On success the row vanishes (the income query is invalidated).
  }

  return (
    <li className="flex flex-col gap-1 border-b px-5 py-3 last:border-0">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="font-medium">{source.name}</div>
          <div className="text-xs text-muted-foreground">
            {FREQUENCY_LABELS[source.frequency] ?? source.frequency}
            {source.next_pay_date && ` · next ${formatIsoDate(source.next_pay_date)}`}
            {source.deposit_account_name && ` · ${source.deposit_account_name}`}
          </div>
        </div>
        <div className="flex items-center gap-3">
          <span className="font-medium tabular-nums text-gain">
            {formatMoney(source.net_amount)}
          </span>
          <div className="flex gap-1">
            {confirmingDelete ? (
              <>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={runDelete}
                  aria-label={`Confirm delete ${source.name}`}
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
                  onClick={() => setEditing(true)}
                  aria-label={`Edit ${source.name}`}
                >
                  Edit
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={() => runAction(() => onArchive(source.id))}
                  aria-label={`Archive ${source.name}`}
                >
                  Archive
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={() => setConfirmingDelete(true)}
                  aria-label={`Delete ${source.name}`}
                >
                  Delete
                </Button>
              </>
            )}
          </div>
        </div>
      </div>
      {/* Outside the row's open-detail button: this note has its own button, and a
          button inside a button is invalid. */}
      {override && onOpenScenario && (
        <AppliedOverrideNote
          override={override}
          currency={source.net_amount.currency}
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

// The normalized fields an income form emits; the caller wraps it into the create
// or update IPC input (adding `idempotency_key`, and `income_source_id` for an edit).
type IncomeDraft = {
  name: string;
  net_amount: MoneyDto;
  frequency: string;
  anchor_date: string;
  deposit_account_id: string | null;
};

// Rust remains authoritative for validation (ADR 0003); this schema only gates
// the form UI and shapes display copy.
const incomeFormSchema = z.object({
  name: z.string().trim().min(1, "Enter a source name."),
  amount: z.string().refine((value) => {
    const minor = dollarsToMinorUnits(value);
    return minor !== null && minor > 0;
  }, "Enter an amount above zero."),
  frequency: z.string().min(1),
  anchor: z.string().min(1, "Pick an anchor date."),
  // "" means no deposit account is linked.
  deposit_account_id: z.string(),
});
type IncomeFormValues = z.infer<typeof incomeFormSchema>;

// Shared add/edit form. `fallbackCurrency` is used when no deposit account is
// linked (the base currency for a new source, the source's existing currency for
// an edit) so an edit never silently changes an accountless source's currency.
// Exported so the First Forecast Wizard (personal-cfo-uipt) reuses it verbatim.
export function IncomeForm({
  accounts,
  defaultValues,
  submitLabel,
  fallbackCurrency,
  onCancel,
  onSubmit,
}: {
  accounts: AccountViewDto[];
  defaultValues: IncomeFormValues;
  submitLabel: string;
  fallbackCurrency: string;
  onCancel: () => void;
  onSubmit: (draft: IncomeDraft) => Promise<string | null>;
}) {
  const {
    register,
    handleSubmit,
    formState: { errors, isSubmitting },
  } = useForm<IncomeFormValues>({
    resolver: zodResolver(incomeFormSchema),
    defaultValues,
  });
  // Unique per instance: two forms can be mounted at once (a suggestion's
  // prefilled form beside the add form, gmnk review).
  const uid = useId();
  const [error, setError] = useState<string | null>(null);

  const submit = handleSubmit(async (values) => {
    setError(null);
    const magnitude = dollarsToMinorUnits(values.amount);
    if (magnitude === null) return;
    // The income currency must match the deposit account when one is linked
    // (enforced by the db worker); without an account, keep the fallback.
    const depositAccount = accounts.find(
      (account) => account.id === values.deposit_account_id,
    );
    const currency = depositAccount?.balance.currency ?? fallbackCurrency;
    const draft: IncomeDraft = {
      name: values.name,
      net_amount: { minor_units: magnitude, currency },
      frequency: values.frequency,
      anchor_date: values.anchor,
      deposit_account_id:
        values.deposit_account_id === "" ? null : values.deposit_account_id,
    };
    const failure = await onSubmit(draft);
    if (failure) setError(failure);
  });

  return (
    <form onSubmit={submit} className="flex flex-col gap-4">
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${uid}-name`}>Source name</Label>
        <Input
          id={`${uid}-name`}
          autoFocus
          {...register("name")}
          placeholder="e.g. Acme Corp paycheck"
          aria-invalid={!!errors.name}
        />
        {errors.name && (
          <p className="text-xs text-loss">{errors.name.message}</p>
        )}
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${uid}-amount`}>Net amount</Label>
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
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${uid}-anchor`}>Anchor pay date</Label>
          <Input id={`${uid}-anchor`} type="date" {...register("anchor")} />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${uid}-account`}>Deposit account</Label>
          <select
            id={`${uid}-account`}
            className={SELECT_CLASS}
            {...register("deposit_account_id")}
          >
            <option value="">No deposit account</option>
            {/* A paycheck lands in a cash account, so only liquid-cash accounts are
                offered (personal-cfo-3b8.2); the kernel also rejects a non-liquid target. */}
            {accounts
              .filter((account) => account.cashflow_role === "liquid_cash")
              .map((account) => (
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
        <Button type="submit" disabled={isSubmitting}>
          {isSubmitting ? "Saving…" : submitLabel}
        </Button>
      </div>
    </form>
  );
}
