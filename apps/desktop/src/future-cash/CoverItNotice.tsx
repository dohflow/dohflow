import { useRef, useState } from "react";
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
  // One idempotency key per draft: a retry of the same transfer replays instead of recording a
  // second one, and a different amount or source is a different draft with a new key.
  const draftKey = useRef<{ identity: string; key: string } | null>(null);

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
  const shortfallName = labelFor(shortfall.accountId, shortfall.accountName);
  const sourceName = labelFor(source, "another account");

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
      await Promise.allSettled([
        queryClient.refetchQueries({ queryKey: queryKeys.accounts }),
        queryClient.refetchQueries({ queryKey: ["forecast"] }),
      ]);
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
      setOpen(false);
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
          reach about{" "}
          <span className="font-medium tabular-nums text-loss">
            −{formatMoney({ minor_units: shortfall.shortfallMinor, currency })}
          </span>{" "}
          on{" "}
          <span className="font-medium">{formatIsoDate(shortfall.date)}</span>.
        </span>
        {!open && (
          <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
            Cover it
          </Button>
        )}
      </div>

      {open &&
        (accounts === null ? (
          accountsError ? (
            <p role="alert" className="text-sm text-loss">
              {accountsError}
            </p>
          ) : (
            <p role="status" className="text-muted-foreground">
              Loading your accounts…
            </p>
          )
        ) : sources.length === 0 ? (
          <p className="text-muted-foreground">
            No other account has funds available to cover it.
          </p>
        ) : (
          <div
            className="flex flex-col gap-3 pt-1"
            onKeyDown={(e) => {
              if (e.key === "Escape" && !saving) setOpen(false);
            }}
          >
            <p className="text-muted-foreground">
              Transferring about{" "}
              <span className="font-medium tabular-nums text-foreground">
                {formatMoney({
                  minor_units: shortfall.shortfallMinor,
                  currency,
                })}
              </span>{" "}
              from{" "}
              <span className="font-medium text-foreground">{sourceName}</span>{" "}
              would keep {shortfallName} above $0.
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
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setOpen(false)}
                disabled={saving}
              >
                Cancel
              </Button>
            </div>
          </div>
        ))}
    </div>
  );
}
