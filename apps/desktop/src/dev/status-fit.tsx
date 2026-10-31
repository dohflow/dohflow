/* eslint-disable react-refresh/only-export-components -- an entry module: it renders itself and nothing imports it, so fast refresh has no component boundary to preserve. */
// Status fit (personal-cfo-4d8.35, the owner's C3 condition). DEV ONLY and FAKE DATA ONLY: not part of
// the app bundle (`index.html` is the only build entry). It renders the REAL `ConnectionRow` and
// `CardStatsStrip` with the real status words, at a chosen component width, and reports for each
// whether anything is clipped, overlaps, scrolls sideways, or pushes its action out of the row.
//
// Open: http://127.0.0.1:5199/status-fit.html?w=368&theme=dark   (`w` is the component width in px)
// `window.__statusFit` carries the results.
import React, { useEffect, useState } from "react";
import ReactDOM from "react-dom/client";

import "@fontsource-variable/ibm-plex-sans/index.css";
import "@/styles/globals.css";

import { CardStatsStrip } from "@/accounts/AccountDetailView";
import type {
  AccountViewDto,
  CardCycleDto,
  CardStatementForecastDto,
  ConnectorConnectionDto,
} from "@/bindings";
import { ConnectionRow } from "@/settings/ConnectionsCard";

import { captureMeta } from "./measure";

const params = new URLSearchParams(window.location.search);
const width = Number(params.get("w") ?? 672);
document.documentElement.classList.toggle("dark", params.get("theme") === "dark");

/// `?set=stale` shows the rows that stress the fit (an old stamp, a long name, a pending refresh, the
/// card strip's due-soon and plain badges) so one screenshot holds them; the default shows all.
const onlyStale = params.get("set") === "stale";
const STRESS = /prior-year stamp|pending|due in 1 day|due in 34/;
const keep = (label: string) => !onlyStale || STRESS.test(label);

const DAY = 86_400_000;
const ago = (days: number) => new Date(Date.now() - days * DAY).toISOString();
/// A prior-year stamp: formatDateTime adds the year, which is the longest the text gets in en-US.
const priorYear = () => new Date(new Date().getFullYear() - 1, 0, 5, 16, 35).toISOString();

function connection(over: Partial<ConnectorConnectionDto>): ConnectorConnectionDto {
  return {
    id: "11111111-1111-7111-8111-111111111111",
    adapter_id: "simplefin",
    display_hint: "Example Bank connection",
    last_synced_at: ago(0.2),
    last_error: null,
    needs_token: false,
    refresh_cadence: "daily",
    attention: [],
    links: [],
    ...over,
  };
}

const CONNECTIONS: [string, Partial<ConnectorConnectionDto>, boolean][] = [
  ["fresh, short stamp", { last_synced_at: ago(0.2) }, false],
  ["2 days, still fresh", { last_synced_at: ago(2) }, false],
  ["3 days: Out of date", { last_synced_at: ago(3) }, false],
  ["Out of date, prior-year stamp", { last_synced_at: priorYear() }, false],
  [
    "Out of date, long name, prior-year stamp",
    {
      last_synced_at: priorYear(),
      display_hint: "A deliberately long connection name for the household's joint savings feed",
    },
    false,
  ],
  ["Out of date, refresh pending (spinner)", { last_synced_at: priorYear() }, true],
  ["Needs attention", { last_error: "The provider refused the token." }, false],
  ["Never refreshed", { last_synced_at: null }, false],
];

/// A typed card forecast: the statement balance is the large historical figure the design record uses on
/// purpose, and does not change current utilization (owed balance over limit, below).
function card(dueDays: number): CardStatementForecastDto {
  const due = new Date(Date.now() + dueDays * DAY).toISOString().slice(0, 10);
  const cycle: CardCycleDto = {
    close_date: due,
    due_date: due,
    carried_opening_balance_minor: 0,
    known_charges_minor: 0,
    projected_variable_minor: 0,
    accrued_interest_minor: 0,
    statement_balance_minor: 123_456,
    minimum_due_minor: 2_500,
    full_pay_minor: 123_456,
    forecast_payment_minor: 123_456,
    statement_is_actual: true,
    is_closed: true,
  };
  return {
    account_id: "22222222-2222-7222-8222-222222222222",
    account_name: "Example card",
    currency: "USD",
    credit_limit_minor: 1_000_000,
    repayment_philosophy: "full_pay",
    cycles: [cycle],
    stored_statements: [],
    estimate_basis: "none",
    estimate_mape_bps: 0,
    estimate_sample_cycles: 0,
  };
}

/// A credit card is a `credit_facility` (a liability: stored as a negative balance, shown as the amount
/// owed), exactly as the account detail page identifies it. The fixture owes $2,500 of a $10,000 limit, so
/// the strip shows 25% used and $7,500 available.
const ACCOUNT: AccountViewDto = {
  id: "22222222-2222-7222-8222-222222222222",
  name: "Example card",
  cashflow_role: "credit_facility",
  subtype: "credit_card",
  active: true,
  balance: { minor_units: -250_000, currency: "USD" },
  notes: null,
  linked_account_id: null,
  linked_account_name: null,
};
const DUE: [string, number][] = [
  ["due today", 0],
  ["due in 1 day (singular)", 1],
  ["due in 5 days", 5],
  ["due in 6 days (secondary)", 6],
  ["due in 34 days", 34],
];

/// Everything that would mean the status or its action has been lost: a child extending past the
/// container, two controls overlapping, text clipped by its box, or the page scrolling sideways.
function audit(box: HTMLElement): { problems: string[]; metrics: Record<string, number> } {
  const problems: string[] = [];
  const outer = box.getBoundingClientRect();
  const leaves = [...box.querySelectorAll<HTMLElement>("button, [data-slot], span, p, div")].filter(
    (e) => e.children.length === 0 || e.matches("button, span"),
  );
  for (const e of leaves) {
    const r = e.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    if (r.right > outer.right + 0.5 || r.left < outer.left - 0.5)
      problems.push(`extends past the component: ${e.textContent?.trim().slice(0, 40)}`);
    if (e.scrollWidth > e.clientWidth + 1 && getComputedStyle(e).overflow !== "visible")
      problems.push(`clipped: ${e.textContent?.trim().slice(0, 40)}`);
  }
  const ctrls = [...box.querySelectorAll<HTMLElement>("button")].map((b) => b.getBoundingClientRect());
  const badges = [...box.querySelectorAll<HTMLElement>("[class*=rounded-full]")]
    .filter((b) => b.tagName !== "DIV" || b.textContent !== "")
    .map((b) => b.getBoundingClientRect());
  const overlap = (a: DOMRect, b: DOMRect) =>
    a.left < b.right - 0.5 && b.left < a.right - 0.5 && a.top < b.bottom - 0.5 && b.top < a.bottom - 0.5;
  ctrls.forEach((a, i) =>
    ctrls.slice(i + 1).forEach((b) => {
      if (overlap(a, b)) problems.push("two buttons overlap");
    }),
  );
  for (const b of badges)
    for (const c of ctrls)
      if (b !== c && overlap(b, c) && !(c.left <= b.left && c.right >= b.right && c.top <= b.top && c.bottom >= b.bottom))
        problems.push("a badge overlaps a button");
  // Sideways scrolling is a problem in its own right, not just a number recorded beside the others.
  if (box.scrollWidth > box.clientWidth + 1) problems.push("the component scrolls sideways");
  const metrics: Record<string, number> = {
    width: outer.width,
    height: outer.height,
    scrollW: box.scrollWidth,
    clientW: box.clientWidth,
  };
  problems.push(...auditConnectionLane(box, metrics));
  return { problems, metrics };
}

/// The reviewed composition for a connection (Paper SF-1r2 SF03/SF04, merged 2026-10-30): the status
/// words and the action sit in ONE lane. At a narrow component width the identity is above the lane; at
/// a wide width it is beside it. In the lane the complete badge is beside Refresh now (not above or below
/// it), the action keeps its 36px height, and its right edge is the row's right content edge, with the
/// pending spinner too.
function auditConnectionLane(box: HTMLElement, metrics: Record<string, number>): string[] {
  const problems: string[] = [];
  const row = box.querySelector<HTMLElement>("[data-part='identity']")?.parentElement?.parentElement;
  const identity = box.querySelector<HTMLElement>("[data-part='identity']");
  const lane = box.querySelector<HTMLElement>("[data-part='status-lane']");
  if (!row || !identity || !lane) return problems; // a card strip row
  const button = [...lane.querySelectorAll<HTMLElement>("button")].find((b) => /Refresh now/.test(b.textContent ?? ""));
  const badge = lane.querySelector<HTMLElement>("[class*=rounded-full]");
  if (!button || !badge) return ["the lane lost its badge or its Refresh now"];
  const rowStyle = getComputedStyle(row);
  const rowBox = row.getBoundingClientRect();
  const contentRight = rowBox.right - parseFloat(rowStyle.paddingRight) - parseFloat(rowStyle.borderRightWidth);
  const contentWidth =
    rowBox.width - parseFloat(rowStyle.paddingLeft) - parseFloat(rowStyle.paddingRight) - parseFloat(rowStyle.borderLeftWidth) - parseFloat(rowStyle.borderRightWidth);
  const b = button.getBoundingClientRect();
  const g = badge.getBoundingClientRect();
  const i = identity.getBoundingClientRect();
  const l = lane.getBoundingClientRect();
  const narrow = contentWidth < 512; // the container query's @lg breakpoint
  Object.assign(metrics, {
    contentWidth,
    actionHeight: b.height,
    actionRightGap: contentRight - b.right,
    badgeToActionGap: b.left - g.right,
    badgeLines: Math.round(g.height / 20),
    narrow: narrow ? 1 : 0,
  });
  if (Math.abs(b.height - 36) > 0.5) problems.push(`Refresh now is ${b.height}px high, not 36`);
  if (Math.abs(contentRight - b.right) > 1) problems.push("Refresh now is not at the row's right edge");
  if (!(g.top < b.bottom - 1 && b.top < g.bottom - 1)) problems.push("the badge is not beside Refresh now");
  if (g.right > b.left + 0.5) problems.push("the badge runs into Refresh now");
  if (narrow && i.bottom > l.top + 1) problems.push("at a narrow width the identity is not above the lane");
  if (!narrow && i.right > l.left + 0.5) problems.push("at a wide width the identity is not beside the lane");
  return problems;
}

function Probe({ label, children }: { label: string; children: React.ReactNode }) {
  const ref = React.useRef<HTMLDivElement>(null);
  const [res, setRes] = useState<ReturnType<typeof audit> | null>(null);
  useEffect(() => {
    // Measure only after the web font has loaded: before it, a fallback font has different widths and
    // a row could read as fitting (or not) for the wrong reason.
    let timer: ReturnType<typeof setTimeout> | undefined;
    let cancelled = false;
    void document.fonts.ready.then(() => {
      if (cancelled) return;
      timer = setTimeout(() => {
        if (!ref.current) return;
        const r = audit(ref.current);
        setRes(r);
        const w = window as unknown as { __statusFit?: { meta: Record<string, unknown>; rows: Record<string, unknown> } };
        w.__statusFit = {
          meta: { ...captureMeta(), componentWidth: width, theme: params.get("theme") === "dark" ? "dark" : "light" },
          rows: { ...(w.__statusFit?.rows ?? {}), [label]: { ...r, verdict: r.problems.length === 0 ? "fits" : "PROBLEMS" } },
        };
      }, 50);
    });
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [label]);
  return (
    <section className="flex flex-col gap-1">
      <p className="text-[11px] text-muted-foreground">
        {label}:{" "}
        {res === null
          ? "measuring"
          : res.problems.length === 0
            ? "fits"
            : `PROBLEMS: ${[...new Set(res.problems)].join("; ")}`}
      </p>
      <div ref={ref}>{children}</div>
    </section>
  );
}

const noop = () => undefined;

function Page() {
  return (
    <main className="p-4">
      <div style={{ width }} className="flex flex-col gap-4">
        {CONNECTIONS.filter(([label]) => keep(label)).map(([label, over, pending]) => (
          <Probe key={label} label={`connection · ${label}`}>
            <ConnectionRow
              connection={connection(over)}
              providerName="SimpleFIN Bridge"
              credentialNoun="setup token"
              accounts={[]}
              accountsPending={false}
              onMap={noop}
              onCreateNew={noop}
              onSetCadence={noop}
              onSync={noop}
              syncPending={pending}
              onForget={noop}
            />
          </Probe>
        ))}
        {DUE.filter(([label]) => keep(label)).map(([label, days]) => (
          <Probe key={label} label={`card strip · ${label}`}>
            <CardStatsStrip
              account={ACCOUNT}
              card={card(days)}
              todayIso={new Date().toISOString().slice(0, 10)}
            />
          </Probe>
        ))}
      </div>
    </main>
  );
}

const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("Root element #root not found");
ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    <Page />
  </React.StrictMode>,
);
