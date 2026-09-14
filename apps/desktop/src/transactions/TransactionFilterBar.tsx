import { useId, useState } from "react";
import { Search, SlidersHorizontal, X } from "lucide-react";

import type { AccountViewDto, TagViewDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { cn } from "@/lib/utils";
import {
  activeFilterCount,
  EMPTY_FILTERS,
  SORT_OPTIONS,
  type TransactionFilters,
  type TransactionSort,
} from "./filters";

/// The search / sort / filter strip over the transactions list (feedback 2026-07-03).
/// Search is always visible (it's the primary ask); the facet filters live behind a
/// toggle with an active-count badge so narrowing is never invisible. All state lives
/// in the parent — this renders controls only.
export function TransactionFilterBar({
  filters,
  onFiltersChange,
  sort,
  onSortChange,
  accounts,
  categoryOptions,
  tags,
  accountScopeLocked = false,
}: {
  filters: TransactionFilters;
  onFiltersChange: (filters: TransactionFilters) => void;
  sort: TransactionSort;
  onSortChange: (sort: TransactionSort) => void;
  accounts: AccountViewDto[];
  categoryOptions: { id: string; label: string }[];
  tags: TagViewDto[];
  /// The account scope belongs to the page, not to this bar (ADR 0057 §3 — the Debt
  /// page's selector owns it). When locked, the account facet is not offered here and is
  /// not counted as user-applied narrowing: a second control for the same thing is how
  /// two controls come to disagree about what is shown.
  accountScopeLocked?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const ids = {
    account: useId(),
    category: useId(),
    tag: useId(),
    from: useId(),
    to: useId(),
  };
  // A locked scope is the page's, not the user's, so it must not inflate the badge —
  // otherwise the embedded list always looks filtered even when the user has done
  // nothing.
  const facets =
    activeFilterCount(filters) - (accountScopeLocked && filters.accountIds.length > 0 ? 1 : 0);
  const set = (patch: Partial<TransactionFilters>) =>
    onFiltersChange({ ...filters, ...patch });
  const activeTags = tags.filter((t) => !t.archived);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <div className="relative flex-1">
          <Search
            className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
            aria-hidden
          />
          <Input
            type="search"
            role="searchbox"
            aria-label="Search transactions"
            placeholder="Search transactions, merchants, notes…"
            className="pl-9"
            value={filters.query}
            onChange={(e) => set({ query: e.target.value })}
          />
        </div>
        <NativeSelect
          aria-label="Sort transactions"
          value={sort}
          onChange={(e) => onSortChange(e.target.value as TransactionSort)}
          className="w-40 shrink-0"
        >
          {SORT_OPTIONS.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </NativeSelect>
        <Button
          type="button"
          variant={open || facets > 0 ? "secondary" : "outline"}
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
          className="shrink-0"
        >
          <SlidersHorizontal aria-hidden />
          Filters
          {facets > 0 && (
            <span className="inline-flex size-5 items-center justify-center rounded-full bg-primary text-[11px] font-semibold text-primary-foreground">
              {facets}
            </span>
          )}
        </Button>
      </div>

      {open && (
        <div className="flex flex-wrap items-end gap-3 rounded-lg border bg-muted/30 p-3">
          {!accountScopeLocked && (
          <div className="flex min-w-36 flex-col gap-1">
            <Label htmlFor={ids.account} className="text-xs">
              Account
            </Label>
            <NativeSelect
              id={ids.account}
              size="sm"
              value={filters.accountIds[0] ?? ""}
              // The bar offers a SINGLE account; a multi-account scope comes from a
              // page that owns it (the Debt page, ADR 0057 §3), through the same field.
              onChange={(e) =>
                set({ accountIds: e.target.value ? [e.target.value] : [] })
              }
            >
              <option value="">All accounts</option>
              {accounts.map((account) => (
                <option key={account.id} value={account.id}>
                  {account.name}
                </option>
              ))}
            </NativeSelect>
          </div>
          )}

          <div className="flex min-w-36 flex-col gap-1">
            <Label htmlFor={ids.category} className="text-xs">
              Category
            </Label>
            <NativeSelect
              id={ids.category}
              size="sm"
              value={filters.categoryId}
              onChange={(e) => set({ categoryId: e.target.value })}
            >
              <option value="">All categories</option>
              <option value="uncategorized">Uncategorized</option>
              {categoryOptions.map((option) => (
                <option key={option.id} value={option.id}>
                  {option.label}
                </option>
              ))}
            </NativeSelect>
          </div>

          {activeTags.length > 0 && (
            <div className="flex min-w-32 flex-col gap-1">
              <Label htmlFor={ids.tag} className="text-xs">
                Tag
              </Label>
              <NativeSelect
                id={ids.tag}
                size="sm"
                value={filters.tagId}
                onChange={(e) => set({ tagId: e.target.value })}
              >
                <option value="">All tags</option>
                {activeTags.map((tag) => (
                  <option key={tag.id} value={tag.id}>
                    {tag.name}
                  </option>
                ))}
              </NativeSelect>
            </div>
          )}

          <div className="flex flex-col gap-1">
            <Label htmlFor={ids.from} className="text-xs">
              From
            </Label>
            <Input
              id={ids.from}
              type="date"
              className="h-8 w-36 text-xs"
              value={filters.from}
              onChange={(e) => set({ from: e.target.value })}
            />
          </div>
          <div className="flex flex-col gap-1">
            <Label htmlFor={ids.to} className="text-xs">
              To
            </Label>
            <Input
              id={ids.to}
              type="date"
              className="h-8 w-36 text-xs"
              value={filters.to}
              onChange={(e) => set({ to: e.target.value })}
            />
          </div>

          <label
            className={cn(
              "flex h-8 cursor-pointer items-center gap-2 rounded-md border px-2.5 text-xs font-medium",
              filters.unreviewedOnly
                ? "border-info/40 bg-info/10 text-info"
                : "text-muted-foreground",
            )}
          >
            <input
              type="checkbox"
              className="size-3.5 accent-primary"
              checked={filters.unreviewedOnly}
              onChange={(e) => set({ unreviewedOnly: e.target.checked })}
            />
            Unreviewed only
          </label>

          {facets > 0 && (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={() =>
                // Preserve a LOCKED account scope across "clear": resetting to
                // EMPTY_FILTERS would set accountIds to [] — which means ALL accounts —
                // and silently widen the embedded list past the page's selection. That
                // is the one way this surface could break its own contract.
                onFiltersChange({
                  ...EMPTY_FILTERS,
                  query: filters.query,
                  accountIds: accountScopeLocked ? filters.accountIds : [],
                })
              }
            >
              <X aria-hidden />
              Clear filters
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
