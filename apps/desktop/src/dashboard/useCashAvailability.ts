import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type IpcError, type MoneyDto } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Reads the household cash-availability snapshot (ADR 0029, personal-cfo-fqbm):
/// per-account ledger/available/committed/headroom, the net rollup, and the
/// minimum-cash-floor status. Exposes a setter for the household floor; saving it
/// invalidates the snapshot so the safe-to-spend figure and the below-floor alert
/// refresh. IPC flows through the generated `commands` only (ADR 0003).
export function useCashAvailability() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.cashAvailability,
    queryFn: () =>
      ipcQuery(
        commands.cashAvailability(),
        "Could not load your cash availability.",
      ),
  });

  const mutation = useMutation({
    mutationFn: (floor: MoneyDto) => commands.setMinimumCashFloor(floor),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({
          queryKey: queryKeys.cashAvailability,
        });
      }
    },
  });

  const setMinimumCashFloor = useCallback(
    async (floor: MoneyDto): Promise<IpcError | null> => {
      const result = await mutation.mutateAsync(floor);
      return result.status === "ok" ? null : result.error;
    },
    [mutation],
  );

  return {
    /// `null` while loading or on error (see `error`).
    availability: query.data ?? null,
    error: query.error?.message ?? null,
    setMinimumCashFloor,
  };
}
