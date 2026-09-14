import { type ReactNode } from "react";
import {
  ArrowLeftRight,
  Banknote,
  CreditCard,
  Download,
  FlaskConical,
  Heart,
  Inbox,
  LayoutDashboard,
  LineChart,
  LockKeyhole,
  PanelLeftClose,
  PanelLeftOpen,
  Receipt,
  Repeat,
  Search,
  Settings,
  Tags,
  Wallet,
} from "lucide-react";

import { BrandLockup } from "@/brand/Brand";
import { BuildBadge } from "@/settings/BuildBadge";
import { DOHFLOW_LINKS, openExternal } from "@/lib/openExternal";
import { cn } from "@/lib/utils";

/// The app's sections (ADR 0049). Grouped into Overview / Planning / Review, with
/// Money Inbox, Bills, and Recurring Transfers promoted out of the former unified
/// Transactions hub (which this reverses) — "Transactions" is now the Activity list.
/// `debt` landed with its surface (personal-cfo-4d8.27.9.2), as ADR 0049 §6 required;
/// `scenarios` shipped
/// with the Scenarios tab (personal-cfo-4d8.27.6.1).
export type Tab =
  | "dashboard"
  | "accounts"
  | "debt"
  | "transactions"
  | "cash-flow"
  | "scenarios"
  | "bills"
  | "recurring"
  | "income"
  | "money-inbox"
  | "categories"
  | "backup"
  | "settings";

/// The labelled nav groups, in render order (ADR 0049 §1). `utility` is pinned to the
/// bottom and renders without a heading.
const GROUPS = [
  { key: "overview", label: "Overview" },
  { key: "planning", label: "Planning" },
  { key: "review", label: "Review" },
] as const;

type NavGroup = (typeof GROUPS)[number]["key"] | "utility";

type NavItem = {
  tab: Tab;
  label: string;
  icon: ReactNode;
  group: NavGroup;
};

const NAV: NavItem[] = [
  { tab: "dashboard", label: "Dashboard", icon: <LayoutDashboard aria-hidden />, group: "overview" },
  { tab: "accounts", label: "Accounts", icon: <Wallet aria-hidden />, group: "overview" },
  // Debt sits between Accounts and Transactions in Overview (ADR 0049 §1). Accounts is
  // the home for account identity and balances; Debt is the home for debt ANALYSIS
  // (ADR 0049 §5) — which is why the insights moved off Accounts rather than being
  // duplicated here.
  { tab: "debt", label: "Debt", icon: <CreditCard aria-hidden />, group: "overview" },
  { tab: "transactions", label: "Transactions", icon: <ArrowLeftRight aria-hidden />, group: "overview" },
  { tab: "cash-flow", label: "Cash Flow", icon: <LineChart aria-hidden />, group: "planning" },
  { tab: "scenarios", label: "Scenarios", icon: <FlaskConical aria-hidden />, group: "planning" },
  { tab: "bills", label: "Bills", icon: <Receipt aria-hidden />, group: "planning" },
  { tab: "recurring", label: "Recurring Transfers", icon: <Repeat aria-hidden />, group: "planning" },
  { tab: "income", label: "Income", icon: <Banknote aria-hidden />, group: "planning" },
  { tab: "money-inbox", label: "Money Inbox", icon: <Inbox aria-hidden />, group: "review" },
  { tab: "categories", label: "Categories", icon: <Tags aria-hidden />, group: "review" },
  { tab: "backup", label: "Backup", icon: <Download aria-hidden />, group: "utility" },
  { tab: "settings", label: "Settings", icon: <Settings aria-hidden />, group: "utility" },
];

/// The collapsible left navigation (personal-cfo-sbm3). Expanded shows icon +
/// label; collapsed shows icons only (every item keeps its `aria-label`, so it
/// stays keyboard- and screen-reader-reachable either way). Replaces the former
/// top-tab bar, which didn't scale to the §18.1 sections still to come.
export function Sidebar({
  tab,
  onSelect,
  collapsed,
  onToggleCollapse,
  onOpenSearch,
  onLock,
  locking,
  inboxCount = 0,
}: {
  tab: Tab;
  onSelect: (tab: Tab) => void;
  collapsed: boolean;
  onToggleCollapse: () => void;
  /// Open the app-wide search palette (personal-cfo-z5lj); Cmd+K does the same.
  onOpenSearch: () => void;
  onLock: () => void;
  locking: boolean;
  /// Items waiting in the Money Inbox — shown as a count badge (ADR 0049 §1), so the
  /// review queue is visible from anywhere instead of buried in another screen.
  inboxCount?: number;
}) {
  const utility = NAV.filter((item) => item.group === "utility");

  return (
    <aside
      className={cn(
        "flex shrink-0 flex-col border-r bg-background transition-[width]",
        collapsed ? "w-14" : "w-56",
      )}
    >
      <div
        className={cn(
          "flex h-14 items-center border-b px-3",
          // Collapsed: just the toggle, centered — the cramped shield+toggle pair in a
          // 56px column read as broken (personal-cfo-4d8.9). Expanded: brand + toggle.
          collapsed ? "justify-center" : "gap-2",
        )}
      >
        {!collapsed && (
          // The approved lockup (mark + wordmark) in place of an icon and a text
          // name (personal-cfo-4d8.28.3). The wordmark is currentColor, so it takes
          // the foreground in both themes; the mark keeps its brand fills.
          <div className="flex min-w-0 flex-1 items-center">
            <BrandLockup variant="horizontal" height={24} className="shrink-0" />
          </div>
        )}
        <button
          type="button"
          onClick={onToggleCollapse}
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground [&_svg]:size-4"
        >
          {collapsed ? <PanelLeftOpen aria-hidden /> : <PanelLeftClose aria-hidden />}
        </button>
      </div>

      <nav
        aria-label="Sections"
        className="flex flex-1 flex-col gap-1 overflow-y-auto p-2"
      >
        {GROUPS.map((group, index) => {
          const items = NAV.filter((item) => item.group === group.key);
          if (items.length === 0) return null;
          const headingId = `nav-group-${group.key}`;
          return (
            <div key={group.key} className="flex flex-col gap-1">
              {/* Expanded: the visible heading NAMES the group (aria-labelledby), so a
                  screen reader announces it once rather than hearing the text and then
                  an identically-named group. Collapsed: a rule separates the groups and
                  the aria-label carries the name. */}
              {!collapsed && (
                <p
                  id={headingId}
                  className="px-2.5 pb-0.5 pt-3 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground/70"
                >
                  {group.label}
                </p>
              )}
              {collapsed && index > 0 && <div className="my-1 border-t" aria-hidden />}
              <div
                role="group"
                aria-labelledby={collapsed ? undefined : headingId}
                aria-label={collapsed ? group.label : undefined}
                className="flex flex-col gap-1"
              >
                {items.map((item) => (
                  <NavButton
                    key={item.tab}
                    item={item}
                    active={tab === item.tab}
                    collapsed={collapsed}
                    onClick={() => onSelect(item.tab)}
                    badge={item.tab === "money-inbox" ? inboxCount : 0}
                  />
                ))}
              </div>
            </div>
          );
        })}
        <div className="flex-1" aria-hidden />
        {/* Search is an action, not a section — no active state, opens the Cmd+K
            palette (personal-cfo-z5lj). Sits above the utility group. */}
        <button
          type="button"
          onClick={onOpenSearch}
          aria-label="Search (⌘K)"
          aria-keyshortcuts="Meta+K Control+K"
          title={collapsed ? "Search (⌘K)" : undefined}
          className={cn(
            "flex items-center gap-2.5 rounded-md px-2.5 py-2 text-sm font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground [&_svg]:size-4 [&_svg]:shrink-0",
            collapsed && "justify-center",
          )}
        >
          <Search aria-hidden />
          {!collapsed && (
            <>
              <span className="flex-1 truncate text-left">Search</span>
              <kbd className="rounded border bg-muted px-1.5 py-0.5 font-sans text-[10px] font-medium text-muted-foreground">
                ⌘K
              </kbd>
            </>
          )}
        </button>
        {utility.map((item) => (
          <NavButton
            key={item.tab}
            item={item}
            active={tab === item.tab}
            collapsed={collapsed}
            onClick={() => onSelect(item.tab)}
          />
        ))}
      </nav>

      <div className="border-t p-2">
        <div className={cn("flex items-center gap-1", collapsed && "flex-col")}>
          <button
            type="button"
            onClick={onLock}
            disabled={locking}
            aria-label="Lock vault"
            title="Lock vault"
            className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-sm font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-60 [&_svg]:size-4 [&_svg]:shrink-0"
          >
            <LockKeyhole aria-hidden />
            {!collapsed && <span>{locking ? "Locking…" : "Lock"}</span>}
          </button>
          {/* The quiet way to support the project (personal-cfo-n76x.18): a muted heart,
              no badge, no motion, no nag — it opens the sponsor page on the DohFlow site
              in the system browser and never checks whether anyone gave anything. */}
          <button
            type="button"
            onClick={() =>
              // A browser-launch failure stays diagnosable in the WebView console
              // instead of vanishing as an unhandled rejection.
              void openExternal(DOHFLOW_LINKS.sponsor).catch((error: unknown) => {
                console.error(
                  `Could not open ${DOHFLOW_LINKS.sponsor} in the system browser`,
                  error,
                );
              })
            }
            aria-label="Support DohFlow"
            title="Support DohFlow"
            className="shrink-0 rounded-md p-2 text-muted-foreground/70 hover:bg-muted hover:text-foreground [&_svg]:size-4"
          >
            <Heart aria-hidden />
          </button>
        </div>
        {/* Which build am I running? (personal-cfo-4d8.27.3.2) */}
        <BuildBadge collapsed={collapsed} />
      </div>
    </aside>
  );
}

function NavButton({
  item,
  active,
  collapsed,
  onClick,
  badge = 0,
}: {
  item: NavItem;
  active: boolean;
  collapsed: boolean;
  onClick: () => void;
  /// A pending count (Money Inbox, ADR 0049 §1); 0 renders nothing.
  badge?: number;
}) {
  // The count is part of the accessible name so a screen reader hears "Money Inbox,
  // 12 waiting" rather than an unannounced visual dot.
  const label = badge > 0 ? `${item.label}, ${badge} waiting` : item.label;
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      aria-label={label}
      title={collapsed ? label : undefined}
      className={cn(
        "relative flex items-center gap-2.5 rounded-md px-2.5 py-2 text-sm font-medium transition-colors [&_svg]:size-4 [&_svg]:shrink-0",
        collapsed && "justify-center",
        // Active: a brand-tinted fill + a slim left indicator bar, so "where am I"
        // reads at a glance without shouting (2pcx).
        active
          ? "bg-primary/10 text-primary before:absolute before:inset-y-1.5 before:left-0 before:w-0.5 before:rounded-full before:bg-primary"
          : "text-muted-foreground hover:bg-muted hover:text-foreground",
      )}
    >
      {item.icon}
      {!collapsed && <span className="flex-1 truncate text-left">{item.label}</span>}
      {badge > 0 && (
        <span
          aria-hidden
          className={cn(
            "rounded-full bg-primary/15 text-[10px] font-semibold tabular-nums text-primary",
            // Collapsed: a compact dot-badge pinned to the icon's corner.
            collapsed
              ? "absolute right-1 top-1 min-w-4 px-1 py-px text-center leading-4"
              : "min-w-5 px-1.5 py-0.5 text-center",
          )}
        >
          {badge > 99 ? "99+" : badge}
        </span>
      )}
    </button>
  );
}
