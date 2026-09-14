import { useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  CircleCheck,
  ListChecks,
  Plus,
  Sparkles, Upload } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { AccountEditorDrawer } from "@/accounts/AccountEditorDrawer";
import { ConnectionsCard } from "@/settings/ConnectionsCard";

import { SuggestedRecurring } from "@/bills/SuggestedRecurring";
import { ExportGuidance } from "@/imports/ExportGuidance";
import { ImportFileDialog } from "@/money-inbox/ImportFileDialog";
import { SuggestedIncome } from "@/income/SuggestedIncome";

import { BridgeEducation } from "./BridgeEducation";
import { IncomeForm } from "@/income/IncomeView";
import { BillForm } from "@/bills/BillsView";
import { useAccounts } from "@/accounts/useAccounts";
import { useIncome } from "@/income/useIncome";
import { useBills } from "@/bills/useBills";
import { useBaseCurrency } from "@/settings/useBaseCurrency";
import { useFutureCash } from "@/future-cash/useFutureCash";
import { FutureCashChart } from "@/future-cash/FutureCashChart";
import { ComfortBandCard } from "@/settings/ComfortBandCard";
import { describeIpcError } from "@/vault/useVault";
import { formatMoney } from "@/lib/format";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { cn } from "@/lib/utils";

const SELECT_CLASS =
  "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

const WIZARD_HORIZON_DAYS = 30;

/// Today as `YYYY-MM-DD` in the user's locale (the date inputs' value).
function today(): string {
  return new Date().toLocaleDateString("en-CA");
}

const STEPS = ["Welcome", "Path", "Accounts", "Income", "Bills", "Forecast"] as const;

/// Which way money gets in (personal-cfo-kdw6): connect banks through the
/// SimpleFIN Bridge, or enter and import by hand. Persisted per viewer so the
/// guide reopens on the last choice; either branch remains reachable.
export type OnboardingPath = "connected" | "manual";
const PATH_KEY = "pcfo.onboardingPath";

function readPath(): OnboardingPath | null {
  try {
    const stored = localStorage.getItem(PATH_KEY);
    return stored === "connected" || stored === "manual" ? stored : null;
  } catch {
    return null;
  }
}

/// The first-run wizard (personal-cfo-uipt): the shortest path to a first useful
/// 30-day cash projection. It reuses the exact Accounts/Income/Bills forms (no
/// drifting copies) and ends on the deterministic forecast + a "what's still
/// missing" list. Shown when the vault has no liquid accounts; skippable. Pure
/// frontend on shipped IPC; the household timezone step + the full Forecast
/// Readiness score are deferred (personal-cfo-q329 / -6vj9).
export function FirstForecastWizard({ onClose }: { onClose: () => void }) {
  const [step, setStep] = useState(0);
  const [path, setPathState] = useState<OnboardingPath | null>(readPath);
  const [addingAccount, setAddingAccount] = useState(false);
  const [importing, setImporting] = useState(false);
  function setPath(next: OnboardingPath) {
    setPathState(next);
    try {
      localStorage.setItem(PATH_KEY, next);
    } catch {
      // Per-viewer convenience only.
    }
  }
  const { baseCurrency, setBaseCurrency } = useBaseCurrency();
  const { accounts } = useAccounts();
  const { sources, addIncomeSource } = useIncome();
  const { bills, addBill } = useBills();

  // Remount keys: bump after a successful add (or cancel) to reset the form so the
  // user can immediately enter the next account / income / bill.
  const [incomeKey, setIncomeKey] = useState(0);
  const [billKey, setBillKey] = useState(0);

  const liveAccounts = (accounts ?? []).filter((account) => account.active);
  const liveIncome = (sources ?? []).filter((source) => source.active);
  const liveBills = (bills ?? []).filter((bill) => bill.active);

  const canAdvance =
    (step !== 1 || path !== null) && (step !== 2 || liveAccounts.length > 0);
  const isOptional = step === 3 || step === 4;

  function next() {
    setStep((current) => Math.min(current + 1, STEPS.length - 1));
  }
  function back() {
    setStep((current) => Math.max(current - 1, 0));
  }

  return (
    <div className="min-h-screen bg-background">
      <div className="mx-auto flex w-full max-w-2xl flex-col gap-6 px-6 py-10">
        <header className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <div className="flex size-7 items-center justify-center rounded-md bg-primary">
              <Sparkles className="size-4 text-primary-foreground" aria-hidden />
            </div>
            <span className="font-semibold tracking-tight">
              Set up your forecast
            </span>
          </div>
          <Button variant="ghost" size="sm" onClick={onClose}>
            Skip for now
          </Button>
        </header>

        <StepIndicator step={step} />

        {step === 0 && (
          <WelcomeStep
            baseCurrency={baseCurrency}
            onCurrency={(code) => void setBaseCurrency(code)}
          />
        )}

        {step === 1 && <PathStep path={path} onChoose={setPath} />}

        {step === 2 && path === "connected" && (
          <StepShell
            title="Connect your banks"
            description="Link the SimpleFIN Bridge, then map each connected account onto an account here — or create one on the spot. Accounts you add by hand work alongside them."
            items={liveAccounts.map(
              (account) =>
                `${account.name} · ${formatMoney(account.balance)}`,
            )}
          >
            <BridgeEducation />
            <ConnectionsCard />
            <div className="flex flex-col gap-2">
              {liveAccounts.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  Nothing discovered yet? Add an account by hand to continue —
                  connected accounts can be mapped onto it later.
                </p>
              ) : null}
              <Button
                variant="outline"
                className="self-start"
                onClick={() => setAddingAccount(true)}
              >
                <Plus aria-hidden />
                Add an account by hand
              </Button>
            </div>
          </StepShell>
        )}

        {step === 2 && path !== "connected" && (
          <StepShell
            title="Add your accounts"
            description="Everything that affects your cash — checking, savings, credit cards, loans, and investments. Debt accounts capture their due dates and terms right here. The accounts you add also start your recorded history: after 30 days of it, the Cash Flow chart shows where your money has actually been, not just where it's headed."
            items={liveAccounts.map(
              (account) =>
                `${account.name} · ${formatMoney(account.balance)}`,
            )}
          >
            <Button
              variant="outline"
              className="self-start"
              onClick={() => setAddingAccount(true)}
            >
              <Plus aria-hidden />
              Add account
            </Button>
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-base">Bring in your history</CardTitle>
                <CardDescription>
                  Optional. A transaction export from your bank gives the forecast
                  real history to learn from — download one, then import it here.
                  You can always do this later from the Money Inbox.
                </CardDescription>
              </CardHeader>
              <CardContent className="flex flex-col gap-3">
                <ExportGuidance id="wizard-export-guide" />
                <div className="flex flex-col gap-1">
                  <Button
                    variant="outline"
                    className="self-start"
                    disabled={liveAccounts.length === 0}
                    onClick={() => setImporting(true)}
                  >
                    <Upload aria-hidden />
                    Import a file
                  </Button>
                  {liveAccounts.length === 0 ? (
                    <p className="text-xs text-muted-foreground">
                      Add the account the file belongs to first.
                    </p>
                  ) : null}
                </div>
              </CardContent>
            </Card>
          </StepShell>
        )}

        {step === 3 && (
          <StepShell
            title="Add your income"
            description="Your paychecks and other recurring income. Enter your NET pay — what actually lands in your account each period. Deposits already in your history show up below as suggestions. You can skip and add later."
            items={liveIncome.map(
              (source) => `${source.name} · ${formatMoney(source.net_amount)}`,
            )}
          >
            <SuggestedIncome />
            <Card>
              <CardContent className="pt-6">
                <IncomeForm
                  key={`income-${incomeKey}`}
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
                  onCancel={() => setIncomeKey((value) => value + 1)}
                  onSubmit={async (draft) => {
                    const failure = await addIncomeSource({
                      ...draft,
                      idempotency_key: mintIdempotencyKey(),
                    });
                    if (!failure) setIncomeKey((value) => value + 1);
                    return failure ? describeIpcError(failure) : null;
                  }}
                />
              </CardContent>
            </Card>
          </StepShell>
        )}

        {step === 4 && (
          <StepShell
            title="Add your recurring bills"
            description="Rent, utilities, subscriptions — what goes out on a schedule. Charges already repeating in your history show up below as suggestions. You can skip and add later."
            items={liveBills.map(
              (bill) => `${bill.name} · ${formatMoney(bill.amount)}`,
            )}
          >
            <SuggestedRecurring />
            <Card>
              <CardContent className="pt-6">
                <BillForm
                  key={`bill-${billKey}`}
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
                  onCancel={() => setBillKey((value) => value + 1)}
                  onSubmit={async (draft) => {
                    const { error: failure } = await addBill({
                      ...draft,
                      source_merchant_key: null,
                      idempotency_key: mintIdempotencyKey(),
                    });
                    if (!failure) setBillKey((value) => value + 1);
                    return failure ? describeIpcError(failure) : null;
                  }}
                />
              </CardContent>
            </Card>
          </StepShell>
        )}

        {step === 5 && (
          <ForecastStep
            missing={missingInputs(
              liveAccounts.length,
              liveIncome.length,
              liveBills.length,
            )}
          />
        )}

        <footer className="flex items-center justify-between">
          <Button
            variant="ghost"
            onClick={back}
            disabled={step === 0}
            className={cn(step === 0 && "invisible")}
          >
            <ArrowLeft aria-hidden />
            Back
          </Button>
          {step === STEPS.length - 1 ? (
            <Button onClick={onClose}>
              <Check aria-hidden />
              Go to dashboard
            </Button>
          ) : (
            <Button onClick={next} disabled={!canAdvance}>
              {isOptional && lengthForStep(step, liveIncome, liveBills) === 0
                ? "Skip"
                : "Next"}
              <ArrowRight aria-hidden />
            </Button>
          )}
        </footer>
      </div>
      {importing && (
        <ImportFileDialog accounts={liveAccounts} onClose={() => setImporting(false)} />
      )}
      {addingAccount && (
        <AccountEditorDrawer
          account={null}
          defaultCurrency={baseCurrency === "EUR" ? "EUR" : "USD"}
          onClose={() => setAddingAccount(false)}
        />
      )}
    </div>
  );
}

function lengthForStep(
  step: number,
  income: unknown[],
  bills: unknown[],
): number {
  if (step === 3) return income.length;
  if (step === 4) return bills.length;
  return 1;
}

/// The R1 readiness heuristic (the full 0-100 score is personal-cfo-6vj9): the
/// inputs still missing for a trustworthy forecast, most-impactful first.
function missingInputs(
  accounts: number,
  income: number,
  bills: number,
): string[] {
  const gaps: string[] = [];
  if (accounts === 0)
    gaps.push("Add an account so the forecast has a starting balance.");
  if (income === 0)
    gaps.push("Add your income to project the money coming in.");
  if (bills === 0)
    gaps.push("Add recurring bills to project the money going out.");
  return gaps;
}

function StepIndicator({ step }: { step: number }) {
  return (
    <ol className="flex items-center gap-2" aria-label="Setup progress">
      {STEPS.map((label, index) => {
        const done = index < step;
        const current = index === step;
        return (
          <li key={label} className="flex items-center gap-2">
            <span
              className={cn(
                "flex size-6 items-center justify-center rounded-full text-xs font-medium",
                done && "bg-primary text-primary-foreground",
                current && "border-2 border-primary text-foreground",
                !done && !current && "border border-input text-muted-foreground",
              )}
            >
              {done ? <Check className="size-3.5" aria-hidden /> : index + 1}
            </span>
            <span
              className={cn(
                "hidden text-xs sm:inline",
                current ? "font-medium text-foreground" : "text-muted-foreground",
              )}
            >
              {label}
            </span>
          </li>
        );
      })}
    </ol>
  );
}

function PathStep({
  path,
  onChoose,
}: {
  path: OnboardingPath | null;
  onChoose: (path: OnboardingPath) => void;
}) {
  const choices: { value: OnboardingPath; title: string; body: string }[] = [
    {
      value: "connected",
      title: "Connect my banks",
      body: "Link accounts through the SimpleFIN Bridge — an optional, paid, independent service — so balances and transactions arrive when you open the app.",
    },
    {
      value: "manual",
      title: "I'll enter and import myself",
      body: "Add accounts by hand and bring in history from bank exports (CSV, OFX) whenever you like. Nothing leaves this device.",
    },
  ];
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="text-lg font-semibold tracking-tight">
          How will your money get in?
        </h2>
        <p className="text-sm text-muted-foreground">
          Pick one to start — you can use both, and change this any time from
          Settings.
        </p>
      </div>
      <fieldset className="grid grid-cols-1 gap-3 border-0 p-0 sm:grid-cols-2">
        <legend className="sr-only">How your money gets in</legend>
        {choices.map((choice) => {
          const selected = path === choice.value;
          return (
            // Native radios: roving focus + arrow keys for free, and the
            // label is the whole card (kdw6 review).
            <label
              key={choice.value}
              className={cn(
                "flex cursor-pointer flex-col gap-1 rounded-lg border p-4 transition-colors",
                "has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring",
                selected ? "border-primary bg-primary/5" : "hover:bg-muted/40",
              )}
            >
              <input
                type="radio"
                name="onboarding-path"
                value={choice.value}
                checked={selected}
                onChange={() => onChoose(choice.value)}
                className="sr-only"
              />
              <span className="font-medium">{choice.title}</span>
              <span className="text-sm text-muted-foreground">{choice.body}</span>
            </label>
          );
        })}
      </fieldset>
    </div>
  );
}

function WelcomeStep({
  baseCurrency,
  onCurrency,
}: {
  baseCurrency: string;
  onCurrency: (code: string) => void;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Let&apos;s build your first cash forecast</CardTitle>
        <CardDescription>
          A few minutes to add your accounts, income, and bills — then you&apos;ll
          see your projected cash for the next 30 days. Everything stays on this
          device.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="flex max-w-xs flex-col gap-1.5">
          <Label htmlFor="wizard-currency">Currency</Label>
          <select
            id="wizard-currency"
            className={SELECT_CLASS}
            value={baseCurrency === "EUR" ? "EUR" : "USD"}
            onChange={(event) => onCurrency(event.target.value)}
          >
            <option value="USD">US Dollar (USD)</option>
            <option value="EUR">Euro (EUR)</option>
          </select>
          <p className="text-xs text-muted-foreground">
            Accounts, income, and bills default to this. You can override each one.
          </p>
        </div>
      </CardContent>
    </Card>
  );
}

function StepShell({
  title,
  description,
  items,
  children,
}: {
  title: string;
  description: string;
  items: string[];
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="text-lg font-semibold tracking-tight">{title}</h2>
        <p className="text-sm text-muted-foreground">{description}</p>
      </div>
      {items.length > 0 && (
        <Card>
          <CardContent className="flex flex-col gap-2 py-4">
            {items.map((item, index) => (
              <div
                key={index}
                className="flex items-center gap-2 text-sm tabular-nums"
              >
                <CircleCheck className="size-4 shrink-0 text-gain" aria-hidden />
                <span>{item}</span>
              </div>
            ))}
          </CardContent>
        </Card>
      )}
      {children}
    </div>
  );
}

function ForecastStep({ missing }: { missing: string[] }) {
  const { forecast, error } = useFutureCash(WIZARD_HORIZON_DAYS);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="text-lg font-semibold tracking-tight">
          Your first forecast
        </h2>
        <p className="text-sm text-muted-foreground">
          Projected liquid cash over the next 30 days, folding in the income and
          bills you added.
        </p>
      </div>

      <Card>
        <CardContent className="pt-6">
          {error ? (
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          ) : forecast === null ? (
            <p className="text-sm text-muted-foreground">Building your forecast…</p>
          ) : (
            <FutureCashChart days={forecast.days} currency={forecast.currency} />
          )}
        </CardContent>
      </Card>

      <ComfortBandCard />

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="flex items-center gap-2 text-sm">
            <ListChecks className="size-4" aria-hidden />
            {missing.length > 0 ? "To sharpen your forecast" : "You're all set"}
          </CardTitle>
          {missing.length === 0 && (
            <CardDescription>
              Your forecast updates automatically as you record activity.
            </CardDescription>
          )}
        </CardHeader>
        {missing.length > 0 && (
          <CardContent className="flex flex-col gap-2">
            {missing.map((gap) => (
              <div key={gap} className="flex items-start gap-2 text-sm">
                <ArrowRight
                  className="mt-0.5 size-4 shrink-0 text-muted-foreground"
                  aria-hidden
                />
                <span>{gap}</span>
              </div>
            ))}
          </CardContent>
        )}
      </Card>
    </div>
  );
}
