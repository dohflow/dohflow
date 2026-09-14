import { useState } from "react";
import { AlertTriangle, CreditCard as CreditCardIcon, Loader2, Pencil } from "lucide-react";

import type {
  CardStatementForecastDto,
  ForecastViewDto,
} from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { describeIpcError } from "@/vault/useVault";
import { dollarsToMinorUnits, formatIsoDate, formatMoney } from "@/lib/format";
import { useFutureCash } from "@/future-cash/useFutureCash";
import {
  useCardStatementForecast,
  useCardStatementHistory,
  useSetStatementBalance,
} from "./useCardStatementForecast";
import { PayCardAction } from "./PayCardAction";

/// Descriptive labels for the estimate's signal tier (ADR 0039 addendum 2026-07-10 §2 —
/// forecast explainability; ADR 0018 neutral phrasing).
const BASIS_LABELS: Record<string, string> = {
  card_history: "from this card's transaction history",
  statement_history: "from your recorded statements",
  categorized_average: "from categorized spending",
};

/// Descriptive labels for the repayment philosophy (ADR 0018 — neutral, never advisory).
const PHILOSOPHY_LABELS: Record<string, string> = {
  pay_in_full: "Projected payment (pay in full)",
  pay_statement_balance: "Projected payment (statement balance)",
  pay_current_balance: "Projected payment (current balance)",
  pay_minimum: "Projected payment (minimum)",
  pay_fixed_amount: "Projected payment (fixed amount)",
  unknown: "Projected payment (assumes the minimum)",
};

function money(minor: number, currency: string): string {
  return formatMoney({ minor_units: minor, currency });
}

/// The credit-card view (personal-cfo-4piy): per card, its utilization, the upcoming
/// statement + payment, projected revolving interest, and a note when the projected statement
/// is larger than projected cash on the due date. Descriptive copy only (ADR 0018). Lives in
/// the Accounts surface's Debt sub-view (ADR 0037).
export function CreditCardsView({ accountIds }: { accountIds?: string[] } = {}) {
  const { cards: allCards, error, loading } = useCardStatementForecast();
  // Scoped by the Debt page's selector when it supplies one (ADR 0057 §1 — selection is
  // a filter over the page). Undefined means unscoped, which is how every other caller
  // renders it.
  const cards =
    accountIds === undefined
      ? allCards
      : (allCards ?? []).filter((card) => accountIds.includes(card.account_id));
  // Projected liquid cash by date, for the statement-vs-cash comparison on each due date.
  const { forecast } = useFutureCash(90);

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="size-4 animate-spin" aria-hidden /> Loading…
      </div>
    );
  }
  if (error) {
    return <div className="text-sm text-destructive">{error}</div>;
  }
  if (!cards || cards.length === 0) {
    return (
      <div className="text-sm text-muted-foreground">
        No credit cards with a billing cycle yet. Add a credit-card account and set its statement
        days in &ldquo;Debt details&rdquo; to see its projected statement here.
      </div>
    );
  }

  const cashOn = buildCashByDate(forecast);
  return (
    <div className="flex flex-col gap-4">
      {cards.map((card) => (
        <CardPanel
          key={card.account_id}
          card={card}
          cashOn={cashOn}
          forecastCurrency={forecast?.currency ?? null}
        />
      ))}
    </div>
  );
}

function buildCashByDate(forecast: ForecastViewDto | null): Map<string, number> {
  const map = new Map<string, number>();
  for (const day of forecast?.days ?? []) {
    map.set(day.date, day.closing.p50.minor_units);
  }
  return map;
}

function CardPanel({
  card,
  cashOn,
  forecastCurrency,
}: {
  card: CardStatementForecastDto;
  cashOn: Map<string, number>;
  forecastCurrency: string | null;
}) {
  const next = card.cycles[0];
  const owed = next?.carried_opening_balance_minor ?? 0;
  const utilization =
    card.credit_limit_minor > 0
      ? Math.round((owed / card.credit_limit_minor) * 100)
      : null;
  const cashOnDue = next ? cashOn.get(next.due_date) : undefined;
  // The forecast cash series is in the household base currency; only compare when the card
  // shares it, so a foreign-currency card never compares raw minor units across currencies.
  const overCash =
    next !== undefined &&
    cashOnDue !== undefined &&
    card.currency === forecastCurrency &&
    next.statement_balance_minor > cashOnDue;

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 p-4">
        <div className="flex items-start justify-between">
          <div className="flex items-center gap-2">
            <CreditCardIcon className="size-4 text-muted-foreground" aria-hidden />
            <span className="font-semibold">{card.account_name}</span>
          </div>
          {utilization !== null && (
            <span className="text-xs text-muted-foreground">
              {money(owed, card.currency)} of {money(card.credit_limit_minor, card.currency)} used
            </span>
          )}
        </div>

        {utilization !== null && (
          <div>
            <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
              <div
                className={
                  utilization >= 70
                    ? "h-full rounded-full bg-loss"
                    : utilization >= 30
                      ? "h-full rounded-full bg-warning"
                      : "h-full rounded-full bg-gain"
                }
                style={{ width: `${Math.min(utilization, 100)}%` }}
                role="progressbar"
                aria-valuenow={Math.min(utilization, 100)}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-label="Credit utilization"
              />
            </div>
            <div className="mt-1 text-xs text-muted-foreground">{utilization}% utilization</div>
          </div>
        )}

        {next && (
          <div className="flex flex-col gap-1 text-sm">
            <StatementRow card={card} />
            <Row label="Closes" value={formatIsoDate(next.close_date)} />
            <Row label="Payment due" value={formatIsoDate(next.due_date)} />
            <Row
              label={PHILOSOPHY_LABELS[card.repayment_philosophy] ?? "Projected payment"}
              value={money(next.forecast_payment_minor, card.currency)}
            />
            <Row label="Minimum due" value={money(next.minimum_due_minor, card.currency)} />
            {next.accrued_interest_minor > 0 && (
              <Row
                label="Projected interest"
                value={money(next.accrued_interest_minor, card.currency)}
              />
            )}
          </div>
        )}

        {overCash && next && (
          <div className="flex items-start gap-2 rounded-md border border-warning/30 bg-warning/10 p-2 text-xs text-warning">
            <AlertTriangle className="mt-0.5 size-4 shrink-0" aria-hidden />
            <span>
              The projected statement of {money(next.statement_balance_minor, card.currency)} is
              larger than your projected cash of{" "}
              {money(cashOn.get(next.due_date) ?? 0, forecastCurrency ?? card.currency)} on{" "}
              {formatIsoDate(next.due_date)} — a carried balance would accrue interest.
            </span>
          </div>
        )}

        {card.cycles.length > 1 && (
          <div className="text-xs text-muted-foreground">
            Later statements:{" "}
            {card.cycles.slice(1).map((c, i) => (
              <span key={c.close_date}>
                {i > 0 ? " · " : ""}
                {formatIsoDate(c.close_date)}: {money(c.statement_balance_minor, card.currency)}
              </span>
            ))}
          </div>
        )}

        {card.estimate_basis !== "none" && (
          <div className="text-xs text-muted-foreground">
            Projected spend {BASIS_LABELS[card.estimate_basis] ?? card.estimate_basis}
            {card.estimate_sample_cycles > 0 &&
              ` (${card.estimate_sample_cycles} ${
                card.estimate_basis === "statement_history" ? "statements" : "cycles"
              })`}
          </div>
        )}

        <StoredStatements card={card} />

        <StatementHistory card={card} />

        <PayCardAction card={card} />
      </CardContent>
    </Card>
  );
}

/// Every user-recorded statement row for the card, each with its own Clear (ADR 0039
/// addendum 2026-07-10 §1, personal-cfo-4d8.25.2): a row keyed to a close date the
/// derivation no longer leads with — e.g. recorded before the closed-cycles-only guard
/// existed — would otherwise be invisible and silently replay when its date arrives.
function StoredStatements({ card }: { card: CardStatementForecastDto }) {
  const setStatement = useSetStatementBalance();
  const [busyClose, setBusyClose] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  if (card.stored_statements.length === 0) return null;

  async function clear(closeDate: string) {
    setBusyClose(closeDate);
    setError(null);
    const failure = await setStatement(card.account_id, closeDate, null);
    setBusyClose(null);
    if (failure) setError(describeIpcError(failure));
  }

  return (
    <div className="flex flex-col gap-1 text-xs">
      <span className="text-muted-foreground">Recorded statements</span>
      {card.stored_statements.map((row) => (
        <div key={row.close_date} className="flex items-center justify-between gap-2">
          <span className="flex items-center gap-1.5 text-muted-foreground">
            {formatIsoDate(row.close_date)}
            {!row.applied && <Badge variant="warning">Not yet closed — ignored</Badge>}
          </span>
          <span className="flex items-center gap-1.5">
            <span className="font-medium tabular-nums">
              {money(row.statement_balance_minor, card.currency)}
            </span>
            <Button
              size="sm"
              variant="ghost"
              className="h-6 px-1.5 text-xs"
              disabled={busyClose !== null}
              onClick={() => void clear(row.close_date)}
            >
              {busyClose === row.close_date ? "Clearing…" : "Clear"}
            </Button>
          </span>
        </div>
      ))}
      {error && (
        <p role="alert" className="text-right text-xs text-loss">
          {error}
        </p>
      )}
    </div>
  );
}

/// The statement line with the set-the-real-number affordance (feedback 2026-07-03): when
/// the actual statement is known it replaces the estimate — badged "Actual" — and Clear
/// falls back to the projection.
function StatementRow({ card }: { card: CardStatementForecastDto }) {
  const next = card.cycles[0];
  const setStatement = useSetStatementBalance();
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (!next) return null;

  async function save(minor: number | null) {
    if (!next) return;
    setBusy(true);
    setError(null);
    const failure = await setStatement(card.account_id, next.close_date, minor);
    setBusy(false);
    if (failure) {
      setError(describeIpcError(failure));
    } else {
      setEditing(false);
      setValue("");
    }
  }

  if (editing) {
    const minor = dollarsToMinorUnits(value);
    return (
      <div className="flex flex-col gap-1">
        <div className="flex items-center justify-between gap-2">
          <span className="text-muted-foreground">Actual statement</span>
          <div className="flex items-center gap-1.5">
            <Input
              aria-label="Actual statement balance"
              inputMode="decimal"
              placeholder="0.00"
              autoFocus
              className="h-7 w-28 text-right text-sm tabular-nums"
              value={value}
              onChange={(e) => setValue(e.target.value)}
            />
            <Button
              size="sm"
              className="h-7"
              disabled={busy || minor === null || minor < 0}
              onClick={() => void save(minor)}
            >
              Save
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="h-7"
              disabled={busy}
              onClick={() => {
                setEditing(false);
                setError(null);
              }}
            >
              Cancel
            </Button>
          </div>
        </div>
        {error && (
          <p role="alert" className="text-right text-xs text-loss">
            {error}
          </p>
        )}
      </div>
    );
  }

  // A statement can only be recorded once its cycle has closed (ADR 0039 addendum
  // 2026-07-10 §1) — outside the grace window the leading cycle is still open, and
  // recording against its shifting close date is how stale rows were born. The flag is
  // computed server-side on the household calendar day, so it can never disagree with
  // the backend write guard across timezones.
  const closed = next.is_closed;
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="flex items-center gap-1.5 text-muted-foreground">
        {next.statement_is_actual ? "Statement" : "Projected statement"}
        {next.statement_is_actual && <Badge variant="gain">Actual</Badge>}
      </span>
      <span className="flex items-center gap-1.5">
        <span className="font-medium tabular-nums">
          {money(next.statement_balance_minor, card.currency)}
        </span>
        {closed && (
          <button
            type="button"
            title={
              next.statement_is_actual
                ? "Edit the actual statement balance"
                : "Enter the actual statement balance"
            }
            aria-label="Set actual statement balance"
            onClick={() => setEditing(true)}
            className="rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <Pencil className="size-3.5" aria-hidden />
            <span className="sr-only">Set actual statement balance</span>
          </button>
        )}
        {next.statement_is_actual && (
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-1.5 text-xs"
            disabled={busy}
            onClick={() => void save(null)}
          >
            Clear
          </Button>
        )}
      </span>
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-medium tabular-nums">{value}</span>
    </div>
  );
}

/// Past billing-cycle windows with derived-from-imports totals and recorded actuals — the
/// statement-history capture surface (ADR 0039 addendum 2026-07-10 §2, personal-cfo-4d8.25.4).
/// Collapsed by default; fetches on expand. Each row can record a past statement: prefilled
/// from the derived import total when available, always editable before saving.
function StatementHistory({ card }: { card: CardStatementForecastDto }) {
  const [open, setOpen] = useState(false);
  const { history, error, loading } = useCardStatementHistory(card.account_id, open);
  const setStatement = useSetStatementBalance();
  const [editingClose, setEditingClose] = useState<string | null>(null);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  async function save(closeDate: string, minor: number) {
    setBusy(true);
    setSaveError(null);
    const failure = await setStatement(card.account_id, closeDate, minor);
    setBusy(false);
    if (failure) {
      setSaveError(describeIpcError(failure));
    } else {
      setEditingClose(null);
      setValue("");
    }
  }

  return (
    <div className="flex flex-col gap-1 text-xs">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="self-start text-muted-foreground underline-offset-2 hover:underline"
      >
        {open ? "Hide statement history" : "Statement history…"}
      </button>
      {open && loading && <span className="text-muted-foreground">Loading…</span>}
      {open && error && (
        <p role="alert" className="text-loss">
          {error}
        </p>
      )}
      {open && history && history.length === 0 && (
        <span className="text-muted-foreground">No past cycles derivable yet.</span>
      )}
      {open &&
        history?.map((row) => {
          const minor = dollarsToMinorUnits(value);
          const editing = editingClose === row.close_date;
          return (
            <div key={row.close_date} className="flex items-center justify-between gap-2">
              <span className="text-muted-foreground">{formatIsoDate(row.close_date)}</span>
              {editing ? (
                <span className="flex items-center gap-1.5">
                  <Input
                    aria-label={`Actual statement for ${row.close_date}`}
                    inputMode="decimal"
                    autoFocus
                    className="h-6 w-24 text-right text-xs tabular-nums"
                    value={value}
                    onChange={(e) => setValue(e.target.value)}
                  />
                  <Button
                    size="sm"
                    className="h-6 px-1.5 text-xs"
                    disabled={busy || minor === null || minor < 0}
                    onClick={() => minor !== null && void save(row.close_date, minor)}
                  >
                    Save
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-6 px-1.5 text-xs"
                    disabled={busy}
                    onClick={() => setEditingClose(null)}
                  >
                    Cancel
                  </Button>
                </span>
              ) : (
                <span className="flex items-center gap-1.5">
                  {row.stored_statement_minor !== null ? (
                    <>
                      <span className="font-medium tabular-nums">
                        {money(row.stored_statement_minor, card.currency)}
                      </span>
                      <Badge variant="gain">Actual</Badge>
                    </>
                  ) : row.derived_charges_minor !== null ? (
                    <span className="tabular-nums text-muted-foreground">
                      {money(row.derived_charges_minor, card.currency)} from imports
                    </span>
                  ) : (
                    <span className="text-muted-foreground">no import coverage</span>
                  )}
                  <button
                    type="button"
                    aria-label={`Record actual statement for ${row.close_date}`}
                    title="Record the actual statement for this cycle"
                    onClick={() => {
                      setEditingClose(row.close_date);
                      const prefill = row.stored_statement_minor ?? row.derived_charges_minor;
                      setValue(prefill !== null ? (prefill / 100).toFixed(2) : "");
                    }}
                    className="rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                  >
                    <Pencil className="size-3" aria-hidden />
                  </button>
                </span>
              )}
            </div>
          );
        })}
      {saveError && (
        <p role="alert" className="text-right text-xs text-loss">
          {saveError}
        </p>
      )}
    </div>
  );
}
