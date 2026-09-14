import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type IpcError } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Reads + sets the household's IANA timezone (ADR 0021 §1, personal-cfo-q329) — the
/// calendar-boundary authority "today" resolves against everywhere in the kernel. The
/// backend defaults to `"UTC"` for a vault created before this existed (no automatic
/// migration — the picker below IS the migration, owner decision A), so consumers always
/// get a usable value. Setting it invalidates the forecast (which already covers Cash Flow
/// and the past-due queue, nested under the same query-key prefix), forecast readiness and
/// band drift (both resolve "today" via `read_household_tz` too — `readiness.rs`), and
/// Money Inbox, so the day boundary updates everywhere without a relaunch (a review
/// finding on the personal-cfo-q329 pull request: the first two were missing). IPC flows
/// through the generated `commands` only (ADR 0003).
///
/// This is the frontend's first — and still only — accessor for the household timezone;
/// `ScenariosView.tsx`'s `today()` and `MarkObligationPaid.tsx`'s `todayIso()` document its
/// prior absence and should switch to it (personal-cfo-q329).
export function useHouseholdTimezone() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.householdTimezone,
    queryFn: () =>
      ipcQuery(
        commands.householdTimezone(),
        "Could not load the household timezone.",
      ),
  });

  const mutation = useMutation({
    mutationFn: (tz: string) => commands.setHouseholdTimezone(tz),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({
          queryKey: queryKeys.householdTimezone,
        });
        // Covers Cash Flow and the unconfirmedPastDue queue, both nested under this prefix.
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
        void queryClient.invalidateQueries({
          queryKey: queryKeys.forecastReadiness,
        });
        void queryClient.invalidateQueries({ queryKey: queryKeys.bandDrift });
        void queryClient.invalidateQueries({ queryKey: queryKeys.moneyInbox });
      }
    },
  });

  const setTimezone = useCallback(
    async (tz: string): Promise<IpcError | null> => {
      const result = await mutation.mutateAsync(tz);
      return result.status === "ok" ? null : result.error;
    },
    [mutation],
  );

  return {
    // Fall back to UTC while the query is in flight, matching the backend default.
    timezone: query.data ?? "UTC",
    isLoading: query.isLoading,
    error: query.error?.message ?? null,
    isSaving: mutation.isPending,
    setTimezone,
  };
}
