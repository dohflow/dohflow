import "@fontsource-variable/ibm-plex-sans/index.css";
import "../styles/globals.css";

import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { NoticeProvider, NotificationsRegion } from "@/components/ui/notice";
import { ScenariosView } from "@/scenarios/ScenariosView";
import { ThemeProvider } from "@/theme/ThemeProvider";
import { writeStoredThemePreference } from "@/theme/themePreference";

/// Dev-only harness for the Apply dialog's conflict notice (personal-cfo-5ie.15). Fake data only.
/// Not part of the production bundle: the build input is `index.html` alone.
///
/// It renders the REAL `ScenariosView` and fakes only the Tauri bridge. `?state=` picks what the
/// scenario's changes meet in the existing forecast, `&theme=dark` the theme, and `&open=1` clicks
/// Apply on load:
///
/// - `overlap`: a bill amount window inside an existing open-ended amount.
/// - `windows`: a partial overlap, a from-only amount and one with no existing amount left outside.
/// - `open-ends`: a to-only amount and an every-date amount (both open-ended).
/// - `mixed`: an amount overlap plus a replaced bill date.
/// - `replace`: a replaced bill date alone (the wording that did not change).

const params = new URLSearchParams(window.location.search);
const state = params.get("state") ?? "overlap";
writeStoredThemePreference(params.get("theme") === "dark" ? "dark" : "light");

const RENT = "bill-rent";
const PHONE = "bill-phone";
const ID = "scenario-3";
const scenario = {
  id: ID,
  name: "Rent change",
  description: null,
  status: "active",
  created_at: "2026-09-01T00:00:00Z",
  updated_at: "2026-10-01T00:00:00Z",
  expires_on: null,
  event_count: 3,
  applied_at: null,
};
const event = (id: string, kind: string, target: string, p: object, owner: string | null) => ({
  id,
  kind,
  target_entity_id: target,
  scenario_id: owner,
  params_json: JSON.stringify(p),
  created_at: "2026-10-01T00:00:00Z",
  promoted_from_scenario_id: null,
});
const amount = (minor: number, from?: string, to?: string) => ({
  new_amount_minor: minor,
  ...(from ? { effective_date: from } : {}),
  ...(to ? { end_date: to } : {}),
});

function assumptions(owner: string | null): unknown[] {
  const mine = owner !== null;
  switch (state) {
    case "replace":
      return mine
        ? [event("s1", "bill_date", RENT, { new_anchor_date: "2027-01-12" }, owner)]
        : [event("b1", "bill_date", RENT, { new_anchor_date: "2027-01-05" }, null)];
    case "windows":
      return mine
        ? [
            event("s1", "bill_amount", RENT, amount(120000, "2027-04-01", "2027-09-30"), owner),
            event("s2", "bill_amount", PHONE, amount(9000, "2027-03-01"), owner),
            event("s3", "income_amount", "income-pay", amount(450000, "2027-01-01", "2027-12-31"), owner),
          ]
        : [
            event("b1", "bill_amount", RENT, amount(100000, "2027-01-01", "2027-06-30"), null),
            event("b2", "bill_amount", PHONE, amount(8000), null),
            event("b3", "income_amount", "income-pay", amount(400000, "2027-03-01", "2027-06-30"), null),
          ];
    case "open-ends":
      return mine
        ? [
            event("s1", "bill_amount", RENT, amount(120000, undefined, "2027-06-30"), owner),
            event("s2", "bill_amount", PHONE, amount(9000), owner),
          ]
        : [
            event("b1", "bill_amount", RENT, amount(100000), null),
            event("b2", "bill_amount", PHONE, amount(8000), null),
          ];
    case "mixed":
      return mine
        ? [
            event("s1", "bill_amount", RENT, amount(120000, "2027-03-01", "2027-06-30"), owner),
            event("s2", "bill_date", RENT, { new_anchor_date: "2027-01-12" }, owner),
          ]
        : [
            event("b1", "bill_amount", RENT, amount(100000), null),
            event("b2", "bill_date", RENT, { new_anchor_date: "2027-01-05" }, null),
          ];
    default:
      return mine
        ? [event("s1", "bill_amount", RENT, amount(120000, "2027-03-01", "2027-06-30"), owner)]
        : [event("b1", "bill_amount", RENT, amount(100000), null)];
  }
}

function fake(cmd: string, args?: { scenarioId?: string | null }): unknown {
  switch (cmd) {
    case "forecast_assumption_list":
      return assumptions(args?.scenarioId ?? null);
    case "scenario_list":
      return [scenario];
    case "scenario_deleted_applied_list":
      return [];
    case "recurring_bill_list":
      return [
        { id: RENT, name: "Rent", active: true },
        { id: PHONE, name: "Phone", active: true },
      ];
    case "income_source_list":
      return [{ id: "income-pay", name: "Paycheck" }];
    case "category_list":
      return [];
    case "base_currency":
      return "USD";
    case "household_timezone":
      return "America/Chicago";
    default:
      return null;
  }
}

(window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
  invoke: async (cmd: string, args?: { scenarioId?: string | null }) => fake(cmd, args),
  transformCallback: () => 0,
};

const client = new QueryClient({
  defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ThemeProvider>
      <QueryClientProvider client={client}>
        <NoticeProvider>
          <NotificationsRegion />
          <div className="mx-auto max-w-3xl p-6">
            <ScenariosView onOpenInCashFlow={() => undefined} />
          </div>
        </NoticeProvider>
      </QueryClientProvider>
    </ThemeProvider>
  </React.StrictMode>,
);

if (params.get("open") === "1") {
  const started = Date.now();
  const timer = window.setInterval(() => {
    const button = Array.from(document.querySelectorAll("button")).find((b) =>
      /apply .* to the forecast/i.test(b.getAttribute("aria-label") ?? ""),
    );
    if (button) {
      window.clearInterval(timer);
      button.click();
    } else if (Date.now() - started > 4000) {
      window.clearInterval(timer);
    }
  }, 100);
}
