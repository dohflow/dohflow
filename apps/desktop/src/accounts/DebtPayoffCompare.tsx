import { useEffect, useRef, useState } from "react";
import { CreditCard } from "lucide-react";

import type { DebtPayoffPlanDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { describeIpcError } from "@/vault/useVault";
import { DataTable, type DataTableColumn } from "@/components/ui/data-table";
import { dollarsToMinorUnits, formatMoney, monthsToDuration } from "@/lib/format";
import { useBaseCurrency } from "@/settings/useBaseCurrency";
import { DebtBurndownChart } from "./DebtBurndownChart";
import { DebtPerDebtChart } from "./DebtPerDebtChart";
import { STRATEGY_LABEL, STRATEGY_ORDER, type Strategy } from "./debtStrategies";
import { useCreatePayoffScenario } from "./useCreatePayoffScenario";
import { useDebtPayoffComparison } from "./useDebtPayoffComparison";

/// Neutral, descriptive one-line detail per strategy (ADR 0018 — none is labelled
/// best/recommended). The display label is shared with the chart legend via STRATEGY_LABEL.
const STRATEGY_DETAIL: Record<string, string> = {
  minimum_only: "Pay just each debt's minimum.",
  snowball: "Extra goes to the smallest balance first.",
  avalanche: "Extra goes to the highest APR first.",
};

/// Debounce a value so a typed extra-budget doesn't refetch on every keystroke.
function useDebounced<T>(value: T, ms: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return debounced;
}

/// The debt-payoff strategy compare (personal-cfo-od07, ADR 0036 debt_payoff). Given an extra
/// monthly budget, shows how the household's carry-debts pay down under minimum-only, snowball,
/// and avalanche — the debt-free duration + total interest per approach. Descriptive only
/// (ADR 0018); lives in the Accounts surface's Debt sub-view (ADR 0037). Single-currency for now
/// (personal-cfo-6wk.13): interest is formatted in the base currency.
export function DebtPayoffCompare({ accountIds }: { accountIds?: string[] } = {}) {
  const [extraInput, setExtraInput] = useState("0");
  const extraMinor = Math.max(0, dollarsToMinorUnits(extraInput) ?? 0);
  const debouncedExtra = useDebounced(extraMinor, 300);
  // Scoped by the Debt page's selector when it supplies one (ADR 0057 §1). The scope
  // reaches the simulation, not its output — see the hook.
  const { plans, error, loading } = useDebtPayoffComparison(
    debouncedExtra,
    accountIds ?? [],
  );
  const { baseCurrency } = useBaseCurrency();
  const { createPayoffScenario, pending } = useCreatePayoffScenario();
  const [scenarioNote, setScenarioNote] = useState<string | null>(null);
  // Which plan's per-debt breakdown to show (the cash outflow is the same across strategies; they
  // differ in which debt clears first).
  const [perDebtStrategy, setPerDebtStrategy] = useState<Strategy>("snowball");
  // Synchronous guard so a rapid double-click can't submit twice before `pending` flips.
  const submitting = useRef(false);

  const addToForecast = async () => {
    if (submitting.current || extraMinor <= 0) return;
    submitting.current = true;
    setScenarioNote(null);
    const perMonth = formatMoney({ minor_units: extraMinor, currency: baseCurrency });
    const name = `Extra ${perMonth}/mo toward debt`;
    try {
      const failure = await createPayoffScenario({
        name,
        extraBudgetMinor: extraMinor,
        currency: baseCurrency,
      });
      setScenarioNote(
        failure
          ? describeIpcError(failure)
          : `Added “${name}” — open Cash Flow to compare it against your base forecast.`,
      );
    } catch {
      setScenarioNote("Could not add the scenario to Cash Flow. Please try again.");
    } finally {
      submitting.current = false;
    }
  };

  // Column defs for the shared DataTable (ADR 0053). Not sortable: three bounded rows in
  // a fixed, deliberately neutral order (ADR 0018 — no approach is labelled best), and
  // re-ranking them by interest would read as exactly the recommendation we don't make.
  const planColumns: DataTableColumn<DebtPayoffPlanDto>[] = [
    {
      key: "approach",
      header: "Approach",
      cell: (plan) => (
        <>
          <div className="font-medium">
            {STRATEGY_LABEL[plan.strategy as Strategy] ?? plan.strategy}
          </div>
          <div className="text-xs text-muted-foreground">
            {STRATEGY_DETAIL[plan.strategy] ?? ""}
          </div>
        </>
      ),
    },
    {
      key: "debt_free",
      header: "Debt-free in",
      cell: (plan) =>
        plan.debt_free_month === null
          ? "Not within 50 years"
          : monthsToDuration(plan.debt_free_month),
    },
    {
      key: "interest",
      header: "Total interest",
      align: "right",
      className: "tabular-nums",
      cell: (plan) =>
        formatMoney({
          minor_units: plan.total_interest_minor,
          currency: plan.currency,
        }),
    },
  ];

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Debt payoff plans</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <p className="text-sm text-muted-foreground">
          Projected paydown for your credit cards and loans under each approach, from your current
          balances, APRs, and minimums.
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <Label htmlFor="payoff-extra" className="text-sm">
            Extra toward debt each month
          </Label>
          <Input
            id="payoff-extra"
            inputMode="decimal"
            value={extraInput}
            onChange={(event) => {
              setExtraInput(event.target.value);
              setScenarioNote(null);
            }}
            className="w-28"
          />
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={extraMinor <= 0 || pending}
            onClick={() => void addToForecast()}
          >
            Add to Cash Flow
          </Button>
        </div>
        {scenarioNote && <p className="text-sm text-muted-foreground">{scenarioNote}</p>}

        {!loading && !error && plans && plans.length > 0 && (
          <DebtBurndownChart
            plans={plans}
            currency={plans[0]?.currency ?? baseCurrency}
          />
        )}
        {/* The table owns its loading / empty / error treatments (ADR 0053 §1). This
            screen used to hand-roll a spinner, a bare sentence and a bespoke error line
            beside a table that only existed once data landed — three of the four states
            re-invented, which is what the primitive exists to end. The header and the
            skeleton rows now hold the layout while the comparison loads. */}
        <DataTable
          columns={planColumns}
          rows={plans ?? []}
          rowKey={(plan) => plan.strategy}
          label="Debt payoff plans by approach"
          // "ready" is gated on plans having actually ARRIVED, not on `loading` being
          // false. `isLoading` is false for a query TanStack has paused (it pauses on
          // navigator.onLine even though every read here is local IPC), and a paused
          // query with no data would otherwise render as "no revolving debt" — telling
          // the household they have no debt when we simply have not read it yet.
          status={error ? "error" : plans === null ? "loading" : "ready"}
          error={error}
          empty={{
            icon: CreditCard,
            title: "No revolving debt to plan a payoff for.",
            description:
              "Add a credit card or loan with a balance and repayment terms to see estimates.",
          }}
        />

        {!loading &&
          !error &&
          plans &&
          plans.length > 0 &&
          (() => {
            const perDebtPlan =
              plans.find((p) => p.strategy === perDebtStrategy) ?? plans[0];
            if (!perDebtPlan) return null;
            return (
              <div className="flex flex-col gap-2">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium">Per debt</span>
                  <div
                    role="group"
                    aria-label="Per-debt paydown strategy"
                    className="flex gap-1 rounded-md bg-muted p-0.5 text-xs"
                  >
                    {STRATEGY_ORDER.map((s) => (
                      <button
                        key={s}
                        type="button"
                        aria-pressed={perDebtStrategy === s}
                        onClick={() => setPerDebtStrategy(s)}
                        className={`rounded px-2 py-1 ${
                          perDebtStrategy === s
                            ? "bg-background font-medium shadow-sm"
                            : "text-muted-foreground"
                        }`}
                      >
                        {STRATEGY_LABEL[s]}
                      </button>
                    ))}
                  </div>
                </div>
                <DebtPerDebtChart plan={perDebtPlan} currency={perDebtPlan.currency} />
              </div>
            );
          })()}
      </CardContent>
    </Card>
  );
}
