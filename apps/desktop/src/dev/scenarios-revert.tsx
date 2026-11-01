import "@fontsource-variable/ibm-plex-sans/index.css";
import "../styles/globals.css";

import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { NoticeProvider, NotificationsRegion } from "@/components/ui/notice";
import { ScenariosView } from "@/scenarios/ScenariosView";
import { ThemeProvider } from "@/theme/ThemeProvider";
import { writeStoredThemePreference } from "@/theme/themePreference";

/// Dev-only harness for the Scenarios manager's undo states (personal-cfo-5ie.12). Fake data only.
/// Not part of the production bundle: the build input is `index.html` alone.
///
/// It renders the REAL `ScenariosView` and fakes only the Tauri bridge, so every string on the page
/// is the product's. `?state=` picks the state and `?theme=dark` the theme:
///
/// - `list`: an applied scenario, the one-click undo (nothing changed since applying).
/// - `lines`: the confirmation with an edit, a removal and a kept newer value.
/// - `blocker`: a scenario applied later has to be undone first.
/// - `changed`: the confirmation shown again because the changes moved while the person looked.
/// - `deleted`: the "Deleted, but still applied" section.
///
/// `&open=1` clicks Undo on load (the confirmation or the blocker); `changed` also clicks its
/// confirm button.

const params = new URLSearchParams(window.location.search);
const state = params.get("state") ?? "list";
// The provider resolves the theme from the stored preference, so set that.
writeStoredThemePreference(params.get("theme") === "dark" ? "dark" : "light");

const scenario = (over: Record<string, unknown>) => ({
  id: "scenario-1",
  name: "Maternity leave",
  description: null,
  status: "active",
  created_at: "2026-09-01T00:00:00Z",
  updated_at: "2026-10-01T00:00:00Z",
  expires_on: null,
  event_count: 3,
  applied_at: null,
  ...over,
});
const applied = scenario({ applied_at: "2026-10-12T15:00:00Z" });

const RENT = "bill-rent";
const lines = (second: boolean) => [
  {
    change: "edited",
    kind: "bill_amount",
    target_entity_id: RENT,
    before_params_json: '{"new_amount_minor":250000,"effective_date":"2027-01-01"}',
    after_params_json: second
      ? '{"new_amount_minor":235000,"effective_date":"2027-01-01"}'
      : '{"new_amount_minor":240000,"effective_date":"2027-01-01"}',
  },
  {
    change: "cleared",
    kind: "one_time_event",
    target_entity_id: null,
    before_params_json:
      '{"label":"Moving costs","amount_minor":-120000,"currency":"USD","date":"2027-02-10"}',
    after_params_json: null,
  },
  {
    change: "kept_newer",
    kind: "bill_date",
    target_entity_id: RENT,
    before_params_json: '{"new_anchor_date":"2027-03-05"}',
    after_params_json: '{"new_anchor_date":"2027-03-12"}',
  },
];
const preview = (over: Record<string, unknown>) => ({
  lines: [],
  blocker_name: null,
  confirmed_descendants: [],
  ...over,
});

let previews = 0;
function fake(cmd: string): unknown {
  switch (cmd) {
    case "scenario_list":
      return state === "deleted" ? [scenario({ id: "scenario-2", name: "New job" })] : [applied];
    case "scenario_deleted_applied_list":
      return state === "deleted"
        ? [scenario({ id: "scenario-9", name: "Old budget plan", applied_at: "2026-08-20T09:00:00Z" })]
        : [];
    case "scenario_revert_preview": {
      previews += 1;
      if (state === "lines") return preview({ lines: lines(false), confirmed_descendants: ["a"] });
      if (state === "blocker") return preview({ blocker_name: "Second plan" });
      if (state === "changed") {
        return previews === 1
          ? preview({ lines: lines(false), confirmed_descendants: ["a"] })
          : preview({ lines: lines(true), confirmed_descendants: ["b"] });
      }
      return preview({});
    }
    case "revert_scenario_apply":
      throw { Validation: "the changes made since this scenario was applied are not the ones confirmed" };
    case "recurring_bill_list":
      return [{ id: RENT, name: "Rent", active: true }];
    case "income_source_list":
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
  invoke: async (cmd: string) => fake(cmd),
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

/// Click through to the state a screenshot needs, once the view has rendered.
function click(name: RegExp, next?: () => void) {
  const started = Date.now();
  const timer = window.setInterval(() => {
    const button = Array.from(document.querySelectorAll("button")).find((b) =>
      name.test(b.getAttribute("aria-label") ?? b.textContent ?? ""),
    );
    if (button) {
      window.clearInterval(timer);
      button.click();
      next?.();
    } else if (Date.now() - started > 4000) {
      window.clearInterval(timer);
    }
  }, 100);
}
if (params.get("open") === "1") {
  click(/undo applying/i, () => {
    if (state === "changed") window.setTimeout(() => click(/^undo scenario$/i), 400);
  });
}
