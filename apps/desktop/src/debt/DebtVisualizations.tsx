import type { AccountViewDto } from "@/bindings";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { AccountBalanceChart } from "@/accounts/AccountBalanceChart";
import {
  buildCardChartRows,
  buildLiquidChartRows,
} from "@/accounts/accountDetailSeries";
import { figureLabelForRole } from "@/accounts/balanceSign";
import { useCardStatementForecast } from "@/accounts/useCardStatementForecast";
import { useCashFlowHistory } from "@/accounts/useCashFlowHistory";
import { useFutureCashByAccount } from "@/future-cash/useFutureCash";

/// History lookback + forward horizon, matching Account detail's 6M default so the two
/// surfaces draw the same shape for the same account.
const LOOKBACK_DAYS = 180;
const HORIZON_DAYS = 90;

/// Per-debt-type visualizations for the Debt page (personal-cfo-4d8.27.9.6, ADR 0057 §2).
///
/// Visualizations are chosen by **account role and composed per role**. A selection
/// spanning cards and loans renders BOTH sections rather than one chart for two shapes —
/// a revolving balance and an amortizing balance share an axis but not a meaning.
///
/// Both roles get balance-over-time, and for a loan that curve *is* its amortization: the
/// row builders are role-aware, so a liability's balance is shown as amount owed. The
/// per-category card-spending breakdown that once lived here was removed as redundant
/// with the scoped Activity section below (owner dogfooding, emgh; ADR 0057 addendum).
///
/// No new primitive is built here. `AccountBalanceChart` already ships — which is why
/// the heatmap deferred in `personal-cfo-azeb` was never actually required.
export function DebtVisualizations({ accounts }: { accounts: AccountViewDto[] }) {
  const { history } = useCashFlowHistory(LOOKBACK_DAYS);
  const { projection } = useFutureCashByAccount(HORIZON_DAYS);
  const { cards } = useCardStatementForecast();
  const todayIso = history?.end_date ?? "";

  const cardAccounts = accounts.filter((a) => a.cashflow_role === "credit_facility");
  const loanAccounts = accounts.filter((a) => a.cashflow_role === "loan_liability");

  function chartFor(account: AccountViewDto) {
    const historySeries = history?.accounts.find((a) => a.account_id === account.id);
    const forwardSeries = projection?.accounts.find((a) => a.account_id === account.id);
    const card = cards?.find((c) => c.account_id === account.id);
    const rows =
      account.cashflow_role === "credit_facility"
        ? buildCardChartRows(
            account.cashflow_role,
            historySeries,
            card,
            todayIso,
            HORIZON_DAYS,
          )
        : buildLiquidChartRows(account.cashflow_role, historySeries, forwardSeries);
    return (
      <div key={account.id} className="flex flex-col gap-1">
        <h4 className="text-sm font-medium">{account.name}</h4>
        <AccountBalanceChart
          rows={rows}
          currency={account.balance.currency}
          todayIso={todayIso}
          figureLabel={figureLabelForRole(account.cashflow_role)}
        />
      </div>
    );
  }

  return (
    <>
      {cardAccounts.length > 0 && (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-base">Credit cards</CardTitle>
            <p className="text-sm text-muted-foreground">
              What you have owed, and where the balance is headed as statements close and
              payments post.
            </p>
          </CardHeader>
          <CardContent className="flex flex-col gap-5">
            {/* The per-category card-spending breakdown that used to sit here
                duplicated the where-the-money-went section in the debts
                activity list below — removed per owner dogfooding (emgh). */}
            {cardAccounts.map(chartFor)}
          </CardContent>
        </Card>
      )}

      {loanAccounts.length > 0 && (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-base">Loans</CardTitle>
            <p className="text-sm text-muted-foreground">
              What is still owed on each loan, and how it pays down.
            </p>
          </CardHeader>
          <CardContent className="flex flex-col gap-5">
            {loanAccounts.map(chartFor)}
          </CardContent>
        </Card>
      )}
    </>
  );
}
