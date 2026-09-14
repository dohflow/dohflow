import { useCallback, useMemo, useRef, useState } from "react";

import type { TransactionRowDto } from "@/bindings";

/// Which list a row is currently visible in. The Money Inbox and the Activity list can
/// both show the same transaction, so selection dedupes across sources by id.
export type SelectionSource = "inbox" | "activity";

/// A unified, cross-list transaction selection (personal-cfo-4d8.24.8). Held once by the
/// TransactionsHub and shared with both the Money Inbox and the Activity list, so a
/// transaction selected in either — or in both — is a single entry, and one bulk-action
/// bar acts on the de-duplicated union. Keyed by `transaction_id`; the value is the row
/// DTO the bar needs (its `tag_ids` drive the bulk-tag merge).
export interface TransactionSelection {
  isSelected: (id: string) => boolean;
  /// Toggle a row in/out of the selection (needs the DTO, not just the id).
  toggle: (row: TransactionRowDto) => void;
  /// Register the rows currently visible + selectable in a list. Refreshes the DTO of any
  /// already-selected row that is on screen (so a bulk op reads fresh tag_ids/category);
  /// never prunes the selection (a selected row that scrolls off stays selected — AC-6).
  setVisibleRows: (source: SelectionSource, rows: TransactionRowDto[]) => void;
  /// Replace the selection by id (the bulk bar's clear / failed-retry path). `[]` clears.
  setSelectionIds: (ids: string[]) => void;
  /// Select every currently-visible selectable row (both lists, deduped).
  selectAllVisible: () => void;
  /// Add explicit rows to the selection — the whole-inbox select-all path
  /// (personal-cfo-4d8.25.16): rows fetched by id can be selected without ever
  /// having been registered as visible.
  selectRows: (rows: TransactionRowDto[]) => void;
  selectedRows: TransactionRowDto[];
  selectedCount: number;
  /// How many distinct rows are currently visible+selectable across both lists — the
  /// "Select all N shown" scope.
  visibleCount: number;
}

/// The lifted selection state (see [`TransactionSelection`]). Instantiate once in the hub.
export function useTransactionSelection(): TransactionSelection {
  const [selected, setSelected] = useState<Map<string, TransactionRowDto>>(
    () => new Map(),
  );
  // What each list currently shows (selectable rows only). A ref because it is read
  // lazily by selectAll/setSelectionIds; a version counter recomputes `visibleCount`.
  const visible = useRef<Record<SelectionSource, Map<string, TransactionRowDto>>>({
    inbox: new Map(),
    activity: new Map(),
  });
  const [visibleVersion, setVisibleVersion] = useState(0);

  const setVisibleRows = useCallback(
    (source: SelectionSource, rows: TransactionRowDto[]) => {
      visible.current[source] = new Map(rows.map((row) => [row.transaction_id, row]));
      // Refresh the DTO of any selected row now on screen — but never prune (AC-6).
      setSelected((prev) => {
        let changed = false;
        const next = new Map(prev);
        for (const row of rows) {
          if (next.has(row.transaction_id)) {
            next.set(row.transaction_id, row);
            changed = true;
          }
        }
        return changed ? next : prev;
      });
      setVisibleVersion((version) => version + 1);
    },
    [],
  );

  const isSelected = useCallback((id: string) => selected.has(id), [selected]);

  const toggle = useCallback((row: TransactionRowDto) => {
    setSelected((prev) => {
      const next = new Map(prev);
      if (next.has(row.transaction_id)) next.delete(row.transaction_id);
      else next.set(row.transaction_id, row);
      return next;
    });
  }, []);

  const setSelectionIds = useCallback((ids: string[]) => {
    setSelected((prev) => {
      const next = new Map<string, TransactionRowDto>();
      for (const id of ids) {
        // Resolve from the prior selection first (a failed-retry row is already here),
        // then from either visible list.
        const row =
          prev.get(id) ??
          visible.current.inbox.get(id) ??
          visible.current.activity.get(id);
        if (row) next.set(id, row);
      }
      return next;
    });
  }, []);

  const allVisible = useMemo(() => {
    const merged = new Map<string, TransactionRowDto>();
    for (const row of visible.current.activity.values())
      merged.set(row.transaction_id, row);
    for (const row of visible.current.inbox.values())
      merged.set(row.transaction_id, row); // dedupe by id across sources
    return merged;
    // eslint-disable-next-line react-hooks/exhaustive-deps -- visibleVersion tracks the ref
  }, [visibleVersion]);

  const selectAllVisible = useCallback(() => {
    setSelected((prev) => {
      const next = new Map(prev);
      for (const [id, row] of allVisible) next.set(id, row);
      return next;
    });
  }, [allVisible]);

  const selectRows = useCallback((rows: TransactionRowDto[]) => {
    setSelected((prev) => {
      const next = new Map(prev);
      for (const row of rows) next.set(row.transaction_id, row);
      return next;
    });
  }, []);

  const selectedRows = useMemo(() => Array.from(selected.values()), [selected]);

  return {
    isSelected,
    toggle,
    setVisibleRows,
    setSelectionIds,
    selectAllVisible,
    selectRows,
    selectedRows,
    selectedCount: selected.size,
    visibleCount: allVisible.size,
  };
}
