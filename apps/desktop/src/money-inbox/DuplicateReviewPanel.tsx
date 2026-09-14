import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  AlertTriangle,
  ArrowDown,
  ArrowRight,
  Check,
  ChevronRight,
  FileText,
  Info,
  Trash2,
  X,
} from "lucide-react";

import {
  commands,
  type CategoryDto,
  type IpcError,
  type MoneyDto,
  type TransactionRowDto,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import { describeIpcError } from "@/vault/useVault";
import { ipcQuery, queryKeys } from "@/lib/query";
import {
  formatDate,
  formatIsoDate,
  formatMoney,
  signedAmountClass,
} from "@/lib/format";
import { categoryLabels } from "@/categories/labels";
import { useCategories } from "@/categories/useCategories";
import { useTransactions } from "@/transactions/useTransactions";

const SELECT_CLASS =
  "h-9 flex-1 rounded-md border border-input bg-background px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

/// The incoming (not-yet-committed) staged transaction, extracted from the inbox item.
export type IncomingDuplicate = {
  merchant: string;
  amount: MoneyDto;
  /// `posted_at` as `YYYY-MM-DD`.
  date: string;
  account: string | null;
  source: string | null;
};

/// The side-by-side duplicate Review panel (ADR 0032 §4, personal-cfo-4d8.8). Shows the
/// incoming transaction beside its committed counterpart(s) (fetched by fingerprint via
/// `duplicate_candidates`) so the user can decide: Skip (it is a duplicate) or Import
/// anyway (it is different). Counterpart edit / void is a tracked follow-on.
export function DuplicateReviewPanel({
  stagedTxnId,
  incoming,
  reason,
  onSkip,
  onImportAnyway,
  onClose,
}: {
  stagedTxnId: string;
  incoming: IncomingDuplicate;
  reason: string | null;
  onSkip: () => Promise<IpcError | null>;
  onImportAnyway: () => Promise<IpcError | null>;
  onClose: () => void;
}) {
  const candidatesQuery = useQuery({
    queryKey: queryKeys.duplicateCandidates(stagedTxnId),
    queryFn: () =>
      ipcQuery(
        commands.duplicateCandidates(stagedTxnId),
        "Could not load the matching transactions.",
      ),
  });
  const { categories } = useCategories();
  const { recategorize, deleteTransaction } = useTransactions();

  // Recategorize / void act on the COMMITTED counterpart (never the incoming); both
  // refetch the candidates so the panel reflects the change (personal-cfo-4d8.21).
  async function recategorizeCounterpart(
    txnId: string,
    categoryId: string | null,
  ) {
    const failure = await recategorize(txnId, categoryId);
    if (!failure) await candidatesQuery.refetch();
    return failure;
  }
  async function voidCounterpart(txnId: string) {
    const failure = await deleteTransaction(txnId);
    if (!failure) await candidatesQuery.refetch();
    return failure;
  }

  const [pending, setPending] = useState<"skip" | "import" | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const candidates = candidatesQuery.data ?? [];
  const single = candidates.length === 1 ? candidates[0] : null;
  const amountDiffers = single
    ? single.amount.minor_units !== incoming.amount.minor_units
    : false;
  // The mock's suggestion logic: a single exact match leans Skip; anything else
  // (amount mismatch, multiple matches, or no match left) leans Import anyway.
  const importPrimary =
    candidates.length !== 1 || amountDiffers || candidatesQuery.isLoading;

  const suggestion = candidatesQuery.isLoading
    ? null
    : candidates.length === 0
      ? "The matching entry is no longer in your ledger — safe to import."
      : candidates.length > 1
        ? `${candidates.length} close matches — confirm before skipping.`
        : amountDiffers
          ? "Suggested: Import anyway — the amount doesn't match."
          : "Suggested: Skip — every field matches.";

  async function run(which: "skip" | "import", fn: () => Promise<IpcError | null>) {
    setPending(which);
    setError(null);
    const failure = await fn();
    setPending(null);
    // On success the inbox re-fetches and the panel closes.
    if (failure) setError(describeIpcError(failure));
    else onClose();
  }

  function counterpartMerchant(memo: string | null, counterparty: string | null) {
    return memo ?? counterparty ?? "—";
  }

  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      <div
        className="absolute inset-0 bg-foreground/50"
        aria-hidden
        onClick={onClose}
      />
      <aside
        role="dialog"
        aria-label="Duplicate review"
        className="relative flex h-full w-full max-w-xl flex-col border-l bg-background shadow-xl"
      >
        {/* header */}
        <div className="flex items-start gap-3 border-b p-5">
          <div className="mt-0.5 flex size-9 shrink-0 items-center justify-center rounded-lg bg-warning/10 text-warning">
            <AlertTriangle className="size-4" aria-hidden />
          </div>
          <div className="min-w-0 flex-1">
            <div className="font-semibold">Possible duplicate</div>
            <div className="mt-1 text-sm leading-relaxed text-muted-foreground">
              {reason ?? "This looks like a transaction you already have."}
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

        {/* body */}
        <div className="flex-1 overflow-auto p-5">
          {candidatesQuery.isLoading ? (
            <p className="text-sm text-muted-foreground">
              Loading the matching transaction…
            </p>
          ) : candidates.length === 0 ? (
            <div className="rounded-md border bg-muted/30 p-3 text-sm text-muted-foreground">
              The matching entry is no longer in your ledger. You can import this
              safely or skip it.
            </div>
          ) : single ? (
            <div className="flex flex-col gap-4">
              {/* column captions */}
              <div className="grid grid-cols-[1fr_auto_1fr] items-end gap-2 text-xs">
                <div>
                  <div className="flex items-center gap-1.5 font-semibold uppercase tracking-wide text-warning">
                    <span className="size-1.5 rounded-full bg-warning" aria-hidden />
                    Incoming
                  </div>
                  <div className="mt-0.5 text-muted-foreground">
                    not yet committed
                  </div>
                </div>
                <div className="w-16" />
                <div>
                  <div className="flex items-center gap-1.5 font-semibold uppercase tracking-wide text-gain">
                    <Check className="size-3" aria-hidden />
                    In your ledger
                  </div>
                  <div className="mt-0.5 text-muted-foreground">1 match</div>
                </div>
              </div>

              {/* aligned compare rows */}
              <div className="overflow-hidden rounded-lg border">
                {[
                  {
                    label: "Merchant",
                    a: incoming.merchant,
                    b: counterpartMerchant(single.memo, single.counterparty),
                    differs:
                      incoming.merchant !==
                      counterpartMerchant(single.memo, single.counterparty),
                  },
                  {
                    label: "Amount",
                    a: formatMoney(incoming.amount),
                    b: formatMoney(single.amount),
                    differs: amountDiffers,
                    aClass: signedAmountClass(incoming.amount),
                    bClass: signedAmountClass(single.amount),
                  },
                  {
                    label: "Date",
                    a: formatIsoDate(incoming.date),
                    b: formatDate(single.occurred_at),
                    differs:
                      formatIsoDate(incoming.date) !==
                      formatDate(single.occurred_at),
                  },
                  {
                    label: "Account",
                    a: incoming.account ?? "—",
                    b: single.account_name,
                    differs: (incoming.account ?? "") !== single.account_name,
                  },
                ].map((field) => (
                  <div
                    key={field.label}
                    className={`grid grid-cols-[1fr_auto_1fr] items-center border-t first:border-t-0 ${
                      field.differs ? "bg-warning/5" : ""
                    }`}
                  >
                    <div className="px-3 py-2.5">
                      <span
                        className={`inline-block rounded px-1.5 py-0.5 text-sm tabular-nums ${
                          field.aClass ?? ""
                        } ${field.differs ? "bg-warning/10 ring-1 ring-warning/40" : ""}`}
                      >
                        {field.a}
                      </span>
                    </div>
                    <div className="w-16 text-center text-[11px] uppercase tracking-wide text-muted-foreground">
                      {field.label}
                    </div>
                    <div className="px-3 py-2.5">
                      <span
                        className={`inline-block rounded px-1.5 py-0.5 text-sm tabular-nums ${
                          field.bClass ?? ""
                        } ${field.differs ? "bg-warning/10 ring-1 ring-warning/40" : ""}`}
                      >
                        {field.b}
                      </span>
                    </div>
                  </div>
                ))}
              </div>

              {amountDiffers && (
                <div className="flex items-center gap-2 rounded-md border border-warning/30 bg-warning/5 px-3 py-2 text-sm">
                  <AlertTriangle
                    className="size-4 shrink-0 text-warning"
                    aria-hidden
                  />
                  <span>
                    Amount differs by{" "}
                    <span className="font-semibold tabular-nums text-warning">
                      {formatMoney({
                        minor_units: Math.abs(
                          incoming.amount.minor_units - single.amount.minor_units,
                        ),
                        currency: incoming.amount.currency,
                      })}
                    </span>{" "}
                    — this may be a separate charge, not a duplicate.
                  </span>
                </div>
              )}

              {/* per-column context */}
              <div className="grid grid-cols-[1fr_auto_1fr] items-start gap-2">
                <div className="flex min-w-0 flex-col gap-1.5">
                  <div className="text-[11px] uppercase tracking-wide text-muted-foreground">
                    Source
                  </div>
                  <div className="flex min-w-0 items-center gap-1.5 rounded-md border bg-muted/30 px-2 py-1.5">
                    <FileText
                      className="size-3 shrink-0 text-muted-foreground"
                      aria-hidden
                    />
                    <span className="truncate font-mono text-xs text-muted-foreground">
                      {incoming.source ?? "imported"}
                    </span>
                  </div>
                </div>
                <div className="w-16" />
                <div className="flex flex-col gap-1.5">
                  <div className="text-[11px] uppercase tracking-wide text-muted-foreground">
                    In your ledger
                  </div>
                  <CounterpartActions
                    counterpart={single}
                    categories={categories}
                    onRecategorize={(categoryId) =>
                      recategorizeCounterpart(single.transaction_id, categoryId)
                    }
                    onVoid={() => voidCounterpart(single.transaction_id)}
                  />
                </div>
              </div>
            </div>
          ) : (
            // MULTI: incoming card + stacked candidate list
            <div className="flex flex-col gap-3">
              <div className="grid grid-cols-[1fr_auto_1fr] items-start gap-2">
                <div className="flex flex-col gap-2 rounded-lg border bg-muted/30 p-3">
                  <div className="font-semibold">{incoming.merchant}</div>
                  <div
                    className={`text-lg font-bold tabular-nums ${signedAmountClass(incoming.amount)}`}
                  >
                    {formatMoney(incoming.amount)}
                  </div>
                  <div className="flex justify-between text-xs">
                    <span className="text-muted-foreground">Date</span>
                    <span>{formatIsoDate(incoming.date)}</span>
                  </div>
                  <div className="flex justify-between text-xs">
                    <span className="text-muted-foreground">Account</span>
                    <span>{incoming.account ?? "—"}</span>
                  </div>
                </div>
                <div className="flex w-16 items-center justify-center pt-12">
                  <ArrowRight className="size-4 text-muted-foreground" aria-hidden />
                </div>
                <div className="flex flex-col gap-2">
                  {candidates.map((candidate) => (
                    <div
                      key={candidate.transaction_id}
                      className="rounded-lg border bg-muted/30 p-3"
                    >
                      <div className="font-medium">
                        {counterpartMerchant(
                          candidate.memo,
                          candidate.counterparty,
                        )}
                      </div>
                      <div
                        className={`mt-1 font-bold tabular-nums ${signedAmountClass(candidate.amount)}`}
                      >
                        {formatMoney(candidate.amount)}
                      </div>
                      <div className="mt-1.5 flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
                        <span>{formatDate(candidate.occurred_at)}</span>
                        <span>{candidate.account_name}</span>
                      </div>
                      <div className="mt-2">
                        <CounterpartActions
                          counterpart={candidate}
                          categories={categories}
                          onRecategorize={(categoryId) =>
                            recategorizeCounterpart(
                              candidate.transaction_id,
                              categoryId,
                            )
                          }
                          onVoid={() => voidCounterpart(candidate.transaction_id)}
                        />
                      </div>
                    </div>
                  ))}
                </div>
              </div>
              <div className="flex items-center gap-2 rounded-md border bg-muted/20 px-3 py-2 text-sm text-muted-foreground">
                <Info className="size-4 shrink-0" aria-hidden />
                <span>
                  {candidates.length} ledger entries already match. Skip only if
                  this import repeats one of them.
                </span>
              </div>
            </div>
          )}

          {error && (
            <p role="alert" className="mt-3 text-sm text-loss">
              {error}
            </p>
          )}
        </div>

        {/* footer */}
        <div className="border-t p-5">
          {suggestion && (
            <div className="mb-3 flex items-center gap-1.5 text-sm text-muted-foreground">
              <ChevronRight className="size-3.5 shrink-0" aria-hidden />
              {suggestion}
            </div>
          )}
          <div className="flex flex-col gap-2">
            <Button
              variant={importPrimary ? "outline" : "default"}
              disabled={pending !== null}
              onClick={() => void run("skip", onSkip)}
            >
              <Check aria-hidden />
              {pending === "skip" ? "Skipping…" : "Skip (it's a duplicate)"}
            </Button>
            <Button
              variant={importPrimary ? "default" : "outline"}
              disabled={pending !== null}
              onClick={() => void run("import", onImportAnyway)}
            >
              <ArrowDown aria-hidden />
              {pending === "import"
                ? "Importing…"
                : "Import anyway (it's different)"}
            </Button>
          </div>
        </div>
      </aside>
    </div>
  );
}

/// The committed counterpart's editable category + a void action (personal-cfo-4d8.21).
/// Both act only on the existing ledger entry — never the incoming (not yet committed).
function CounterpartActions({
  counterpart,
  categories,
  onRecategorize,
  onVoid,
}: {
  counterpart: TransactionRowDto;
  categories: CategoryDto[] | null;
  onRecategorize: (categoryId: string | null) => Promise<IpcError | null>;
  onVoid: () => Promise<IpcError | null>;
}) {
  const { options } = categoryLabels(categories ?? []);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function pickCategory(categoryId: string | null) {
    setBusy(true);
    setError(null);
    const failure = await onRecategorize(categoryId);
    setBusy(false);
    if (failure) setError(describeIpcError(failure));
  }
  async function confirmVoid() {
    setBusy(true);
    setError(null);
    const failure = await onVoid();
    setBusy(false);
    // On success the candidates refetch and this card unmounts (or stays if others remain).
    if (failure) {
      setError(describeIpcError(failure));
      setConfirming(false);
    }
  }

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2">
        <select
          aria-label="Counterpart category"
          value={counterpart.category_id ?? ""}
          disabled={busy}
          onChange={(event) => void pickCategory(event.target.value || null)}
          className={SELECT_CLASS}
        >
          <option value="">Uncategorized</option>
          {options.map((option) => (
            <option key={option.id} value={option.id}>
              {option.label}
            </option>
          ))}
        </select>
        {confirming ? (
          <>
            <Button
              variant="ghost"
              size="sm"
              disabled={busy}
              onClick={() => void confirmVoid()}
            >
              {busy ? "Voiding…" : "Confirm void"}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={busy}
              onClick={() => setConfirming(false)}
            >
              Cancel
            </Button>
          </>
        ) : (
          <button
            type="button"
            aria-label="Void this entry"
            disabled={busy}
            onClick={() => setConfirming(true)}
            className="rounded-md p-1.5 text-muted-foreground hover:bg-loss/10 hover:text-loss"
          >
            <Trash2 className="size-4" aria-hidden />
          </button>
        )}
      </div>
      {error && (
        <p role="alert" className="text-xs text-loss">
          {error}
        </p>
      )}
    </div>
  );
}
