import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, Loader2 } from "lucide-react";

import type { MultiSeriesForecastDto } from "@/bindings";
import { useAccounts } from "@/accounts/useAccounts";
import { useTransactions } from "@/transactions/useTransactions";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { dollarsToMinorUnits, formatIsoDate, formatMoney } from "@/lib/format";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { queryKeys } from "@/lib/query";
import { describeIpcError } from "@/vault/useVault";

import { detectShortfall } from "./coverIt";
import {
  accountLabel,
  seriesAsLabelled,
  type LabelledAccount,
} from "./seriesKeys";

const SELECT_CLASS =
  "flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

/// Shown after a transfer call that threw: the call failed, so the ledger may or may not hold the
/// transfer, and the screen re-reads before offering Confirm again (the 5ie.12 pattern).
const TRANSFER_OUTCOME_UNKNOWN =
  "Couldn’t confirm whether the transfer was recorded. Check your accounts and the forecast before trying again.";

/// Shown when the forecast read behind this notice failed and the cache has no message of its own.
const FORECAST_READ_FAILED = "Could not load your forecast.";

function minorToInput(minorUnits: number): string {
  return (minorUnits / 100).toString();
}

/// The user-steered "cover it" tool (ADR 0018 addendum, personal-cfo-j0cg.1). When a liquid account
/// is projected to dip below $0 in the near term, it states the shortfall as a fact and offers a
/// tool the user *opens* to move money from another liquid account and cover it. Descriptive
/// outside the tool; the amount proposal only appears inside it; the app never picks the source or
/// acts unprompted (the copy stays within ADR 0018 §915.1).
export function CoverItNotice({
  projection,
}: {
  projection: MultiSeriesForecastDto;
}) {
  const shortfall = detectShortfall(projection);
  const { accounts, error: accountsError } = useAccounts();
  const { transfer } = useTransactions();
  const queryClient = useQueryClient();

  const [open, setOpen] = useState(false);
  const [amountDraft, setAmountDraft] = useState<string | null>(null);
  const [sourceChoice, setSourceChoice] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The forecast re-read after an unknown outcome failed: its message, until a retry succeeds.
  const [forecastFailure, setForecastFailure] = useState<string | null>(null);
  // Which read a Retry is re-running right now.
  const [retrying, setRetrying] = useState<"accounts" | "forecast" | null>(null);
  // A retry of a read that never produced data puts the query back to pending, which would flip the
  // panel to "Loading…" and hide the Retrying… button; the failure being retried stays on screen.
  const [retriedFailure, setRetriedFailure] = useState<string | null>(null);
  const panel = useRef<HTMLDivElement>(null);
  const coverButton = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef(false);
  // One idempotency key per draft: a retry of the same transfer replays instead of recording a
  // second one, and a different amount or source is a different draft with a new key.
  const draftKey = useRef<{ identity: string; key: string } | null>(null);

  // An opened panel owns the keyboard: the button that opened it is gone, so focus moves in (and
  // back in if a control it was on disappears), which is what lets Escape reach it. Only focus that
  // was lost is taken back; a control the person chose outside the panel keeps it across re-renders.
  useEffect(() => {
    if (!open || !panel.current) return;
    const active = document.activeElement;
    if (!active || active === document.body || active === document.documentElement) {
      panel.current.focus();
    }
  });
  // Closing returns focus to the button that opened the tool.
  useEffect(() => {
    if (!open && returnFocus.current) {
      returnFocus.current = false;
      coverButton.current?.focus();
    }
  }, [open]);

  if (!shortfall) return null;

  const currency = projection.currency;
  // Eligible sources: other active accounts you can draw cash from — liquid cash or an investment
  // (j0cg.2) — holding funds in the projection's currency (record_transfer requires the amount's
  // currency to match both legs).
  const sources = (accounts ?? []).filter(
    (a) =>
      (a.cashflow_role === "liquid_cash" ||
        a.cashflow_role === "investment_asset") &&
      a.active &&
      a.id !== shortfall.accountId &&
      a.balance.minor_units > 0 &&
      a.balance.currency === currency,
  );
  const source = sourceChoice || sources[0]?.id || "";
  const amountValue = amountDraft ?? minorToInput(shortfall.shortfallMinor);
  // Every account this notice names is labelled against all of them, so two accounts called
  // "Savings" never read alike (the Future Cash picker's rule).
  const known: LabelledAccount[] = [
    ...projection.accounts.map(seriesAsLabelled),
    ...(accounts ?? []).filter(
      (a) => !projection.accounts.some((s) => s.account_id === a.id),
    ),
  ];
  const labelFor = (id: string | null, fallback: string) => {
    const account = known.find((a) => a.id === id);
    return account ? accountLabel(account, known) : fallback;
  };
  const zero = { minor_units: 0, currency };
  const shortfallName = labelFor(shortfall.accountId, shortfall.accountName);
  const sourceName = labelFor(source, "another account");

  function close() {
    returnFocus.current = true;
    setOpen(false);
  }

  const accountsFailure = accountsError ?? (retrying === "accounts" ? retriedFailure : null);

  /// The message of a forecast read that is on screen and failed, or null when none is.
  function currentForecastFailure(): string | null {
    const failed = queryClient
      .getQueryCache()
      .findAll({ queryKey: ["forecast"] })
      .find((q) => q.getObserversCount() > 0 && q.state.status === "error");
    if (!failed) return null;
    return (failed.state.error as Error | null)?.message || FORECAST_READ_FAILED;
  }

  async function retry(which: "accounts" | "forecast") {
    if (which === "accounts") setRetriedFailure(accountsError);
    setRetrying(which);
    try {
      await queryClient.refetchQueries({
        queryKey: which === "accounts" ? queryKeys.accounts : ["forecast"],
      });
      if (which === "forecast") setForecastFailure(currentForecastFailure());
    } finally {
      setRetrying(null);
    }
  }

  async function onConfirm() {
    if (!shortfall) return;
    setError(null);
    const minor = dollarsToMinorUnits(amountValue);
    if (minor === null || minor <= 0) {
      setError("Enter an amount above zero.");
      return;
    }
    const identity = `${source}|${shortfall.accountId}|${currency}|${minor}`;
    if (draftKey.current?.identity !== identity) {
      draftKey.current = { identity, key: mintIdempotencyKey() };
    }
    setSaving(true);
    let failure;
    try {
      failure = await transfer({
        source_account_id: source,
        dest_account_id: shortfall.accountId,
        amount: { minor_units: minor, currency },
        occurred_at: new Date().toISOString(),
        idempotency_key: draftKey.current.key,
      });
    } catch {
      // The call itself failed, so the outcome is not known. The key is kept, so a retry of this
      // draft cannot record a second transfer; the screen reads what is stored first, and
      // Confirm stays disabled (`saving`) until that is done.
      setError(TRANSFER_OUTCOME_UNKNOWN);
      // Settling only means the reads were attempted. Confirm comes back only once both succeeded:
      // a failed accounts read shows as such (`accountsError`), and a failed forecast read is
      // looked up here.
      await Promise.allSettled([
        queryClient.refetchQueries({ queryKey: queryKeys.accounts }),
        queryClient.refetchQueries({ queryKey: ["forecast"] }),
      ]);
      setForecastFailure(currentForecastFailure());
      setSaving(false);
      return;
    }
    setSaving(false);
    if (failure) {
      setError(describeIpcError(failure));
    } else {
      // The next transfer is a new draft, even for the same amount and source.
      draftKey.current = null;
      // On success the forecast refetches; if it's covered, this notice disappears.
      close();
    }
  }

  return (
    <div className="flex flex-col gap-2 rounded-md border border-loss/30 bg-loss/5 px-3 py-2 text-sm">
      <div className="flex items-start gap-2">
        <AlertTriangle
          className="mt-0.5 size-4 shrink-0 text-loss"
          aria-hidden
        />
        <span className="flex-1">
          <span className="font-medium">{shortfallName}</span> is projected to
          be below{" "}
          <span className="font-medium tabular-nums">{formatMoney(zero)}</span>{" "}
          on <span className="font-medium">{formatIsoDate(shortfall.date)}</span>.
          Its near-term projected low is about{" "}
          <span className="font-medium tabular-nums text-loss">
            {formatMoney({ minor_units: -shortfall.shortfallMinor, currency })}
          </span>
          .
        </span>
        {!open && (
          <Button
            ref={coverButton}
            variant="outline"
            size="sm"
            onClick={() => setOpen(true)}
          >
            Cover it
          </Button>
        )}
      </div>

      {open && (
        <div
          ref={panel}
          tabIndex={-1}
          className="flex flex-col gap-3 pt-1 outline-none"
          onKeyDown={(e) => {
            if (e.key === "Escape" && !saving) close();
          }}
        >
          {accountsFailure ? (
            // A failed accounts read outranks a cached list: nothing here can be confirmed on
            // accounts the app could not just read.
            <>
              {error === TRANSFER_OUTCOME_UNKNOWN && (
                <p role="alert" className="text-sm text-loss">
                  {error}
                </p>
              )}
              <p role="alert" className="text-sm text-loss">
                {accountsFailure}
              </p>
              <div className="flex gap-2">
                <RetryButton
                  pending={retrying === "accounts"}
                  onClick={() => void retry("accounts")}
                />
                <CancelButton disabled={saving} onClick={close} />
              </div>
            </>
          ) : accounts === null ? (
            <>
              <p role="status" className="text-muted-foreground">
                Loading your accounts…
              </p>
              <div className="flex gap-2">
                <CancelButton disabled={saving} onClick={close} />
              </div>
            </>
          ) : forecastFailure ? (
            <>
              {error === TRANSFER_OUTCOME_UNKNOWN && (
                <p role="alert" className="text-sm text-loss">
                  {error}
                </p>
              )}
              <p role="alert" className="text-sm text-loss">
                {forecastFailure}
              </p>
              <div className="flex gap-2">
                <RetryButton
                  pending={retrying === "forecast"}
                  onClick={() => void retry("forecast")}
                />
                <CancelButton disabled={saving} onClick={close} />
              </div>
            </>
          ) : sources.length === 0 ? (
            <>
              <p className="text-muted-foreground">
                No other account has funds available to cover it.
              </p>
              <div className="flex gap-2">
                <CancelButton disabled={saving} onClick={close} />
              </div>
            </>
          ) : (
            <>
              <p className="text-muted-foreground">
                A transfer of about{" "}
                <span className="font-medium tabular-nums text-foreground">
                  {formatMoney({
                    minor_units: shortfall.shortfallMinor,
                    currency,
                  })}
                </span>{" "}
                from{" "}
                <span className="font-medium text-foreground">{sourceName}</span>{" "}
                would bring {shortfallName}’s near-term projected low to{" "}
                <span className="font-medium tabular-nums text-foreground">
                  {formatMoney(zero)}
                </span>
                .
              </p>
              <div className="grid grid-cols-2 gap-3">
                <div className="flex flex-col gap-1">
                  <Label htmlFor="cover-amount" className="text-xs">
                    Amount to transfer
                  </Label>
                  <Input
                    id="cover-amount"
                    inputMode="decimal"
                    value={amountValue}
                    onChange={(e) => setAmountDraft(e.target.value)}
                  />
                </div>
                <div className="flex flex-col gap-1">
                  <Label htmlFor="cover-source" className="text-xs">
                    From account
                  </Label>
                  <select
                    id="cover-source"
                    className={SELECT_CLASS}
                    value={source}
                    onChange={(e) => setSourceChoice(e.target.value)}
                  >
                    {sources.map((a) => (
                      <option key={a.id} value={a.id}>
                        {labelFor(a.id, a.name)} ({formatMoney(a.balance)})
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
              <div className="flex gap-2">
                <Button
                  size="sm"
                  onClick={() => void onConfirm()}
                  disabled={saving || !source}
                >
                  {saving ? (
                    <Loader2 className="animate-spin" aria-hidden />
                  ) : null}
                  Confirm transfer
                </Button>
                <CancelButton disabled={saving} onClick={close} />
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}

function CancelButton({ disabled, onClick }: { disabled: boolean; onClick: () => void }) {
  return (
    <Button variant="ghost" size="sm" onClick={onClick} disabled={disabled}>
      Cancel
    </Button>
  );
}

/// Retry for a failed read: disabled while it runs, and the failure stays on screen if it fails again.
function RetryButton({ pending, onClick }: { pending: boolean; onClick: () => void }) {
  return (
    <Button variant="outline" size="sm" onClick={onClick} disabled={pending}>
      {pending ? <Loader2 className="animate-spin" aria-hidden /> : null}
      {pending ? "Retrying…" : "Retry"}
    </Button>
  );
}
