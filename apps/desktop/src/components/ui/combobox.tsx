import * as React from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown } from "lucide-react";

import { cn } from "@/lib/utils";

/// One selectable entry. `label` is the primary text (a leaf name); `hint` is
/// secondary context rendered muted after it (a parent path) — always present for
/// category items because leaf names are NOT unique (ADR 0045 §3).
export interface ComboboxItem {
  id: string;
  label: string;
  hint?: string;
  /// Extra text the filter may match beyond label/hint (e.g. a full path).
  searchText?: string;
}

interface ComboboxProps {
  items: ComboboxItem[];
  /// The selected id, or null for the empty/placeholder state.
  value: string | null;
  onSelect: (id: string | null) => void;
  placeholder: string;
  /// Renders a persistent "empty" choice at the top (e.g. "Uncategorized").
  clearLabel?: string;
  /// Persistent footer action pinned under the results (e.g. "Create new
  /// category…"); receives the current query so a dialog can prefill it.
  footerAction?: { label: string; onAction: (query: string) => void };
  buttonClassName?: string;
  disabled?: boolean;
  "aria-label": string;
  id?: string;
}

/// Case/diacritic-insensitive haystack normalization shared by filter + rank.
function normalize(text: string): string {
  return text
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase();
}

/// Rank a match: label-prefix beats label-word beats anywhere-in-search-text.
/// `null` = no match. Exported for direct unit testing.
export function matchRank(item: ComboboxItem, query: string): number | null {
  if (query === "") return 3;
  const label = normalize(item.label);
  const haystack = normalize(
    `${item.label} ${item.hint ?? ""} ${item.searchText ?? ""}`,
  );
  if (label.startsWith(query)) return 0;
  if (label.split(/\s+/).some((word) => word.startsWith(query))) return 1;
  if (haystack.includes(query)) return 2;
  return null;
}

/// A searchable single-select combobox (WAI-ARIA combobox pattern: filter input +
/// listbox with aria-activedescendant). This is the house's first departure from
/// the NativeSelect convention (ADR 0031 note in native-select.tsx): search over a
/// large taxonomy needs a filterable listbox a platform <select> cannot provide.
/// Tests drive it with click + keyDown instead of fireEvent.change.
export const Combobox = React.forwardRef<HTMLButtonElement, ComboboxProps>(
  function Combobox(
    {
      items,
      value,
      onSelect,
      placeholder,
      clearLabel,
      footerAction,
      buttonClassName,
      disabled,
      id,
      "aria-label": ariaLabel,
    },
    ref,
  ) {
    const [open, setOpen] = React.useState(false);
    const [query, setQuery] = React.useState("");
    const [activeIndex, setActiveIndex] = React.useState(0);
    const rootRef = React.useRef<HTMLDivElement>(null);
    const panelRef = React.useRef<HTMLDivElement>(null);
    const inputRef = React.useRef<HTMLInputElement>(null);
    const listboxId = React.useId();
    // The panel is portaled to <body> so an ancestor's `overflow` (e.g. the
    // transactions table's `overflow-x-auto`) can't clip it (adversarial review of
    // 4d8.25.18). Its fixed position tracks the trigger while open.
    const [anchor, setAnchor] = React.useState<{
      top: number;
      left: number;
      width: number;
    } | null>(null);

    const selected = items.find((item) => item.id === value) ?? null;

    const filtered = React.useMemo(() => {
      const q = normalize(query.trim());
      return items
        .map((item, index) => ({ item, index, rank: matchRank(item, q) }))
        .filter((entry): entry is { item: ComboboxItem; index: number; rank: number } =>
          entry.rank !== null,
        )
        .sort((a, b) => a.rank - b.rank || a.index - b.index)
        .map((entry) => entry.item);
    }, [items, query]);

    /// Row model: the clear row is itself MATCH-GATED — present when the query is
    /// empty or matches its label. An unconditional clear row made a no-match query
    /// + Enter silently clear the selection (a real recategorize-to-null IPC) and
    /// left the "No matches" state unreachable (adversarial review of 4d8.25.18).
    /// The footer action is a button below the listbox, reachable via ArrowDown.
    const clearVisible =
      clearLabel !== undefined &&
      matchRank({ id: "", label: clearLabel }, normalize(query.trim())) !== null;
    const rows: Array<{ kind: "clear" } | { kind: "item"; item: ComboboxItem }> =
      React.useMemo(
        () => [
          ...(clearVisible ? [{ kind: "clear" as const }] : []),
          ...filtered.map((item) => ({ kind: "item" as const, item })),
        ],
        [clearVisible, filtered],
      );
    const footerIndex = rows.length;
    const lastIndex = footerAction ? footerIndex : rows.length - 1;

    React.useEffect(() => {
      setActiveIndex(0);
    }, [query, open]);
    // A query with no matches defaults the active row to the create-new footer, so
    // typing a fresh name + Enter opens the dialog instead of doing nothing.
    React.useEffect(() => {
      if (footerAction && rows.length === 0) setActiveIndex(footerIndex);
    }, [footerAction, footerIndex, rows.length]);

    // autoFocus moves focus INTO the portal, so a close must hand it back or
    // it falls to <body> and tab order restarts at the document top (review
    // of ulg9; WAI-ARIA combobox pattern). Outside-click dismiss deliberately
    // does NOT refocus — focus follows the pointer.
    const close = React.useCallback((refocusTrigger = false) => {
      setOpen(false);
      setQuery("");
      if (refocusTrigger) {
        rootRef.current?.querySelector("button")?.focus();
      }
    }, []);

    // Light-dismiss: a click outside BOTH the trigger and the (portaled) panel closes.
    React.useEffect(() => {
      if (!open) return;
      function onPointerDown(event: PointerEvent) {
        const target = event.target as Node;
        const inTrigger = rootRef.current?.contains(target) ?? false;
        const inPanel = panelRef.current?.contains(target) ?? false;
        if (!inTrigger && !inPanel) close();
      }
      window.addEventListener("pointerdown", onPointerDown);
      return () => window.removeEventListener("pointerdown", onPointerDown);
    }, [close, open]);

    // Position the portaled panel under the trigger; follow scroll/resize while open.
    React.useLayoutEffect(() => {
      if (!open) {
        setAnchor(null);
        return;
      }
      function measure() {
        const rect = rootRef.current?.getBoundingClientRect();
        if (rect) setAnchor({ top: rect.bottom + 4, left: rect.left, width: rect.width });
      }
      measure();
      window.addEventListener("scroll", measure, true);
      window.addEventListener("resize", measure);
      return () => {
        window.removeEventListener("scroll", measure, true);
        window.removeEventListener("resize", measure);
      };
    }, [open]);


    function commitRow(index: number) {
      const row = rows[index];
      if (row === undefined) {
        if (footerAction && index === footerIndex) {
          const q = query.trim();
          close(true);
          footerAction.onAction(q);
        }
        return;
      }
      onSelect(row.kind === "clear" ? null : row.item.id);
      close(true);
    }

    function onInputKeyDown(event: React.KeyboardEvent) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setActiveIndex((i) => Math.min(i + 1, lastIndex));
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        setActiveIndex((i) => Math.max(i - 1, 0));
      } else if (event.key === "Enter") {
        event.preventDefault();
        commitRow(activeIndex);
      } else if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        close(true);
      } else if (event.key === "Tab") {
        // The input is about to unmount mid-Tab; without a handoff the browser
        // drops focus at <body>. Recover at the trigger so the next Tab lands
        // on the following field.
        close(true);
      }
    }

    return (
      <div ref={rootRef} className="relative">
        <button
          ref={ref}
          type="button"
          id={id}
          disabled={disabled}
          aria-label={ariaLabel}
          aria-haspopup="listbox"
          aria-expanded={open}
          onClick={() => (open ? close() : setOpen(true))}
          className={cn(
            "flex h-9 w-full items-center justify-between gap-1 rounded-md border border-input bg-background px-2 text-left text-sm",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
            "disabled:cursor-not-allowed disabled:opacity-50",
            buttonClassName,
          )}
        >
          <span className={cn("truncate", selected === null && "text-muted-foreground")}>
            {selected === null ? placeholder : selected.label}
          </span>
          <ChevronDown className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        </button>
        {open &&
          anchor !== null &&
          createPortal(
            <div
              ref={panelRef}
              // `data-escape-layer` lets an ancestor dialog's Escape handler yield
              // to this open popover instead of closing itself (adversarial review
              // of 4d8.25.19).
              data-escape-layer="popover"
              style={{
                position: "fixed",
                top: anchor.top,
                left: anchor.left,
                width: anchor.width,
              }}
              className="z-50 min-w-56 rounded-md border bg-background shadow-lg"
            >
              <input
                ref={inputRef}
                role="combobox"
              aria-expanded="true"
              aria-controls={listboxId}
              aria-activedescendant={
                activeIndex <= lastIndex ? `${listboxId}-row-${activeIndex}` : undefined
              }
              aria-label={`Search ${ariaLabel.toLowerCase()}`}
              // autoFocus, not an [open] effect: the panel (input included) only
              // mounts AFTER the anchor is measured, one render later than the
              // open flip — an effect keyed on `open` fires while inputRef is
              // still null and silently no-ops (personal-cfo-ulg9).
              autoFocus
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={onInputKeyDown}
              placeholder="Type to search…"
              className="w-full border-b bg-transparent px-3 py-2 text-sm focus:outline-none"
            />
            <ul
              id={listboxId}
              role="listbox"
              aria-label={ariaLabel}
              className="max-h-64 overflow-y-auto p-1"
            >
              {rows.length === 0 && (
                <li className="px-2 py-2 text-sm text-muted-foreground" role="presentation">
                  No matches
                </li>
              )}
              {rows.map((row, index) => {
                const active = index === activeIndex;
                const isSelected =
                  row.kind === "item" ? row.item.id === value : value === null;
                return (
                  <li
                    key={row.kind === "clear" ? " clear" : row.item.id}
                    id={`${listboxId}-row-${index}`}
                    role="option"
                    aria-selected={isSelected}
                    onPointerDown={(event) => {
                      // Commit before the outside-click dismiss can fire.
                      event.preventDefault();
                      commitRow(index);
                    }}
                    onMouseEnter={() => setActiveIndex(index)}
                    className={cn(
                      "flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm",
                      active && "bg-accent",
                    )}
                  >
                    <Check
                      className={cn("size-3.5 shrink-0", !isSelected && "invisible")}
                      aria-hidden
                    />
                    {row.kind === "clear" ? (
                      <span className="text-muted-foreground">{clearLabel}</span>
                    ) : (
                      <span className="min-w-0 truncate">
                        {row.item.label}
                        {row.item.hint !== undefined && (
                          <span className="text-muted-foreground"> · {row.item.hint}</span>
                        )}
                      </span>
                    )}
                  </li>
                );
              })}
            </ul>
            {footerAction && (
              <button
                type="button"
                id={`${listboxId}-row-${footerIndex}`}
                onPointerDown={(event) => {
                  event.preventDefault();
                  commitRow(footerIndex);
                }}
                onMouseEnter={() => setActiveIndex(footerIndex)}
                className={cn(
                  "block w-full border-t px-3 py-2 text-left text-sm text-primary",
                  activeIndex === footerIndex && "bg-accent",
                )}
              >
                {footerAction.label}
              </button>
            )}
            </div>,
            document.body,
          )}
      </div>
    );
  },
);
