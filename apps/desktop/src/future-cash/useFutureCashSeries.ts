import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type IpcError } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// The default plotted series when the user has never chosen: the three aggregate
/// tiers (Net Cash / Spendable / Reserve), per the owner's spec (personal-cfo-4d8.25.26).
export const DEFAULT_SERIES_SELECTION = ["net", "spendable", "reserve"];

/// Parse the stored opaque JSON string into a list of series keys, falling back to
/// the default on absent/invalid data so the chart always has a usable selection.
function parseSelection(raw: string | null): string[] {
  if (raw === null) return DEFAULT_SERIES_SELECTION;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (Array.isArray(parsed) && parsed.every((k) => typeof k === "string")) {
      return parsed as string[];
    }
  } catch {
    // fall through to the default
  }
  return DEFAULT_SERIES_SELECTION;
}

/// Reads + persists the Future Cash chart's series selection through the vault
/// settings KV (personal-cfo-4d8.25.26), mirroring `useAutoCategorizeOnImport`. IPC
/// flows through the generated `commands` only (ADR 0003).
export function useFutureCashSeries() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.futureCashSeries,
    queryFn: () =>
      ipcQuery(
        commands.futureCashSeriesSelection(),
        "Could not load the chart's series selection.",
      ),
  });

  const mutation = useMutation({
    mutationFn: (selection: string[]) =>
      commands.setFutureCashSeriesSelection(JSON.stringify(selection)),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.futureCashSeries });
      }
    },
  });

  const setSelection = useCallback(
    async (selection: string[]): Promise<IpcError | null> => {
      const result = await mutation.mutateAsync(selection);
      return result.status === "ok" ? null : result.error;
    },
    [mutation],
  );

  return {
    selection: parseSelection(query.data ?? null),
    setSelection,
  };
}
