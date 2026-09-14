import { useEffect, useRef, useState } from "react";
import { Check, SlidersHorizontal } from "lucide-react";

import type { AccountSeriesDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import { accountLabel, accountSeriesKey, seriesAsLabelled } from "./seriesKeys";

/// The aggregate tiers the picker offers, in the owner's hierarchy order. Net cash is
/// a standalone total; Spendable + Reserve each expand to their individual accounts.
const TIERS: { key: string; label: string; childTier: string | null }[] = [
  { key: "net", label: "Net cash", childTier: null },
  { key: "spendable", label: "Spendable", childTier: "spendable" },
  { key: "reserve", label: "Reserve", childTier: "reserve" },
];

/// The hierarchical series-selection control for the Future Cash chart
/// (personal-cfo-4d8.25.26): plot the three aggregate tiers (default) OR drill into
/// individual accounts. Coherence rule (ADR 0026 §12 — children sum to their tier):
/// a tier and its own accounts are mutually exclusive, so the same money is never
/// drawn twice. Selecting a tier drops its accounts from the plot and vice-versa.
export function SeriesPicker({
  accounts,
  selection,
  onChange,
}: {
  accounts: AccountSeriesDto[];
  selection: string[];
  onChange: (selection: string[]) => void;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: PointerEvent) {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const accountsFor = (childTier: string) =>
    accounts.filter((a) => a.account_id !== null && a.tier === childTier);
  // Disambiguation set: every account the picker offers, so duplicate names get a
  // stable suffix (adversarial review of 4d8.25.26).
  const offered = accounts.filter((a) => a.account_id !== null);

  const has = (key: string) => selection.includes(key);

  /// Toggle an aggregate tier: turning it on drops its accounts (coherence rule).
  function toggleTier(tierKey: string, childKeys: string[]) {
    const on = has(tierKey);
    let next = selection.filter((k) => k !== tierKey && !childKeys.includes(k));
    if (!on) next = [...next, tierKey];
    onChange(next);
  }

  /// Toggle an individual account: turning it on drops its parent tier.
  function toggleAccount(tierKey: string, acctKey: string) {
    const on = has(acctKey);
    let next = selection.filter((k) => k !== tierKey);
    next = on ? next.filter((k) => k !== acctKey) : [...next, acctKey];
    onChange(next);
  }

  return (
    <div ref={rootRef} className="relative">
      <Button
        type="button"
        size="sm"
        variant="outline"
        aria-haspopup="true"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <SlidersHorizontal aria-hidden />
        Series
      </Button>
      {open && (
        <div
          role="dialog"
          aria-label="Choose which series to plot"
          className="absolute right-0 top-full z-50 mt-1 w-64 rounded-md border bg-background p-2 shadow-lg"
        >
          <p className="px-1 pb-1.5 text-xs text-muted-foreground">
            Plot a tier or its individual accounts.
          </p>
          <ul className="flex max-h-72 flex-col overflow-y-auto">
            {TIERS.map((tier) => {
              const kids = tier.childTier ? accountsFor(tier.childTier) : [];
              const childKeys = kids.map((a) => accountSeriesKey(a.account_id ?? ""));
              return (
                <li key={tier.key} className="flex flex-col">
                  <Row
                    label={tier.label}
                    checked={has(tier.key)}
                    onToggle={() => toggleTier(tier.key, childKeys)}
                    bold
                  />
                  {kids.map((a) => {
                    const key = accountSeriesKey(a.account_id ?? "");
                    return (
                      <Row
                        key={key}
                        label={accountLabel(seriesAsLabelled(a), offered.map(seriesAsLabelled))}
                        checked={has(key)}
                        onToggle={() => toggleAccount(tier.key, key)}
                        indent
                      />
                    );
                  })}
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}

function Row({
  label,
  checked,
  onToggle,
  bold,
  indent,
}: {
  label: string;
  checked: boolean;
  onToggle: () => void;
  bold?: boolean;
  indent?: boolean;
}) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      onClick={onToggle}
      className={cn(
        "flex items-center gap-2 rounded px-1.5 py-1 text-left text-sm hover:bg-muted",
        indent && "pl-6",
        bold && "font-medium",
      )}
    >
      <span
        aria-hidden
        className={cn(
          "flex size-4 shrink-0 items-center justify-center rounded border",
          checked ? "border-primary bg-primary text-primary-foreground" : "border-input",
        )}
      >
        {checked && <Check className="size-3" />}
      </span>
      <span className="min-w-0 truncate">{label}</span>
    </button>
  );
}
