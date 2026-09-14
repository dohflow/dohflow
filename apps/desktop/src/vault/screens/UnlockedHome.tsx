import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";

import { DashboardView } from "@/dashboard/DashboardView";
import { FutureCashView } from "@/future-cash/FutureCashView";
import { ScenariosView } from "@/scenarios/ScenariosView";
import { DebtView } from "@/debt/DebtView";
import { AccountsView } from "@/accounts/AccountsView";
import { TransactionsView } from "@/transactions/TransactionsView";
import { RecurringTransfersView } from "@/transactions/RecurringTransfersView";
import { MoneyInboxView } from "@/money-inbox/MoneyInboxView";
import { useMoneyInbox } from "@/money-inbox/useMoneyInbox";
import { BillsView } from "@/bills/BillsView";
import { SuggestedRecurring } from "@/bills/SuggestedRecurring";
import { useTransactionSelection } from "@/transactions/useTransactionSelection";
import { TransactionBulkBar } from "@/transactions/TransactionBulkBar";
import { IncomeView } from "@/income/IncomeView";
import { CategoriesView } from "@/categories/CategoriesView";
import { BackupView } from "@/backup/BackupView";
import { BackupNudge } from "@/backup/BackupNudge";
import { SettingsView } from "@/settings/SettingsView";
import { UpdateAvailableNotice } from "@/settings/UpdateAvailableNotice";
import { FirstForecastWizard } from "@/onboarding/FirstForecastWizard";
import { CapabilityUnlockNotice } from "@/forecast-activation/CapabilityUnlockNotice";
import { useAccounts } from "@/accounts/useAccounts";
import { useVault } from "@/vault/useVault";
import { GlobalSearch } from "@/search/GlobalSearch";
import { Sidebar, type Tab } from "./Sidebar";

/// The unlocked app shell: a collapsible left sidebar (personal-cfo-sbm3) over the
/// active section. Shared read-model data is cached by TanStack Query and
/// invalidated by the mutations (ADR 0020), so e.g. recording a transaction
/// refreshes account balances and the Future Cash forecast across every section.
export function UnlockedHome() {
  const { lockVault } = useVault();
  const { accounts } = useAccounts();
  const [locking, setLocking] = useState(false);
  const [tab, setTab] = useState<Tab>("dashboard");
  // The scenario the Cash Flow screen should open with, set when the user opens one
  // from the Scenarios tab (ADR 0051 §5).
  const [cashFlowScenario, setCashFlowScenario] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(false);
  // The inbox count feeds the sidebar badge (ADR 0049 §1). Cache-shared with the inbox
  // surface's own query (same key), so this adds no extra round-trip.
  const { items: inboxItems } = useMoneyInbox();
  // App-wide transaction search (personal-cfo-z5lj). The open state lives here —
  // on the shell, not the palette — so Cmd+K works from every tab.
  const [searching, setSearching] = useState(false);
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setSearching(true);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
  // First-run: show the First Forecast Wizard (personal-cfo-uipt) when the vault
  // has no accounts yet. The decision is *latched* on the first load that finds an
  // empty vault, so adding the first account inside the wizard doesn't immediately
  // yank the user out of it; it clears (for the session) once they finish or skip.
  const [wizard, setWizard] = useState<boolean | null>(null);
  useEffect(() => {
    if (wizard === null && accounts !== null) {
      setWizard(accounts.length === 0);
    }
  }, [wizard, accounts]);

  async function onLock() {
    setLocking(true);
    await lockVault();
    // On success the provider flips to Locked and the router swaps in the
    // unlock screen; if it failed we simply re-enable the button.
    setLocking(false);
  }

  // Hold the shell until the first-run decision resolves, so an empty vault never
  // flashes the dashboard before the wizard takes over.
  if (wizard === null) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-background">
        <Loader2 className="size-6 animate-spin text-muted-foreground" aria-hidden />
      </div>
    );
  }
  if (wizard) {
    return (
      <FirstForecastWizard
        onClose={() => {
          setWizard(false);
          // "Go to dashboard" must mean it, including when the guide was
          // rerun from Settings (kdw6 review).
          setTab("dashboard");
        }}
      />
    );
  }

  return (
    // The shell is exactly viewport height with the main pane scrolling internally
    // (h-screen + overflow-hidden), so the sidebar stays full-height and its
    // bottom-pinned items never get pushed below a tall content pane (personal-cfo-4d8.9).
    <div className="flex h-screen overflow-hidden bg-background">
      <Sidebar
        tab={tab}
        onSelect={setTab}
        collapsed={collapsed}
        onToggleCollapse={() => setCollapsed((value) => !value)}
        onOpenSearch={() => setSearching(true)}
        onLock={onLock}
        locking={locking}
        inboxCount={inboxItems?.length ?? 0}
      />
      <main className="flex-1 overflow-y-auto px-6 py-8">
        <CapabilityUnlockNotice />
        {tab === "dashboard" ? (
          <>
            {/* First-run backup nudge (personal-cfo-vdmb): data + no export yet. */}
            <BackupNudge onOpenBackup={() => setTab("backup")} />
            <DashboardView onOpenCashFlow={() => setTab("cash-flow")} />
          </>
        ) : tab === "cash-flow" ? (
          // The shell owns the selection, so opening a scenario from the Scenarios tab
          // and switching back to base in the bar are the same state — navigating away
          // and back cannot resurrect a scenario the user already dismissed.
          <FutureCashView
            scenarioId={cashFlowScenario}
            onScenarioChange={setCashFlowScenario}
          />
        ) : tab === "scenarios" ? (
          <ScenariosView
            onOpenInCashFlow={(id) => {
              setCashFlowScenario(id);
              setTab("cash-flow");
            }}
          />
        ) : tab === "accounts" ? (
          <AccountsView />
        ) : tab === "debt" ? (
          // Debt analysis lives here, not in Accounts (ADR 0049 §5 / ADR 0057).
          <DebtView />
        ) : tab === "transactions" ? (
          // Transactions is the Activity list only (ADR 0049 §2) — the Money Inbox,
          // Bills, and Recurring Transfers are their own destinations below.
          <ActivitySurface />
        ) : tab === "money-inbox" ? (
          <MoneyInboxSurface />
        ) : tab === "bills" ? (
          <div className="mx-auto flex w-full max-w-2xl flex-col gap-6">
            <SuggestedRecurring />
            <BillsView onOpenScenario={() => setTab("scenarios")} />
          </div>
        ) : tab === "recurring" ? (
          <RecurringTransfersView />
        ) : tab === "income" ? (
          <IncomeView onOpenScenario={() => setTab("scenarios")} />
        ) : tab === "categories" ? (
          <CategoriesView />
        ) : tab === "backup" ? (
          <BackupView />
        ) : (
          <SettingsView onRerunSetup={() => setWizard(true)} />
        )}
      </main>
      <UpdateAvailableNotice onGoToSettings={() => setTab("settings")} />
      {searching && <GlobalSearch onClose={() => setSearching(false)} />}
    </div>
  );
}

/// The Activity surface (ADR 0049 §2): the transaction list plus its own bulk bar.
///
/// The selection hook lives HERE, not in the shell, so it is created and destroyed with
/// the tab — exactly the lifecycle the dissolved hub had. Holding it in the always-mounted
/// shell would leave a stale selection (and its floating bulk bar) alive across tab
/// switches, acting on rows the user can no longer see (adversarial review of ADR 0049).
function ActivitySurface() {
  const selection = useTransactionSelection();
  return (
    <>
      <TransactionsView selection={selection} />
      <TransactionBulkBar selection={selection} />
    </>
  );
}

/// The Money Inbox surface (ADR 0049 §2) — its own selection + bulk bar, same lifecycle
/// reasoning as [`ActivitySurface`]. The controlled selection is what keeps the inbox's
/// "Select all N in inbox" control (personal-cfo-4d8.25.16) available.
function MoneyInboxSurface() {
  const selection = useTransactionSelection();
  return (
    <>
      <MoneyInboxView selection={selection} />
      <TransactionBulkBar selection={selection} />
    </>
  );
}
