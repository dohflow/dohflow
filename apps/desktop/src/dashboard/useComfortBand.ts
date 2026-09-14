import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type IpcError, type MoneyDto } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Reads the household liquid-cash comfort band (ADR 0018 addendum 915.1, personal-cfo-3v6d): the
/// lower edge (the minimum-cash floor) and an optional upper edge. Exposes setters for each; the
/// lower edge is the shipped floor command, so setting it also refreshes the cash-availability
/// snapshot (the below-floor alert). IPC flows through the generated `commands` only (ADR 0003).
export function useComfortBand() {
  const queryClient = useQueryClient();

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.comfortBand });
    void queryClient.invalidateQueries({ queryKey: queryKeys.cashAvailability });
  };

  const query = useQuery({
    queryKey: queryKeys.comfortBand,
    queryFn: () =>
      ipcQuery(commands.comfortBand(), "Could not load your comfort band."),
  });

  const lowerMutation = useMutation({
    mutationFn: (floor: MoneyDto) => commands.setMinimumCashFloor(floor),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const upperMutation = useMutation({
    mutationFn: (upper: MoneyDto | null) => commands.setComfortBandUpper(upper),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });

  const setLower = useCallback(
    async (floor: MoneyDto): Promise<IpcError | null> => {
      const result = await lowerMutation.mutateAsync(floor);
      return result.status === "ok" ? null : result.error;
    },
    [lowerMutation],
  );
  /// `null` clears the upper edge.
  const setUpper = useCallback(
    async (upper: MoneyDto | null): Promise<IpcError | null> => {
      const result = await upperMutation.mutateAsync(upper);
      return result.status === "ok" ? null : result.error;
    },
    [upperMutation],
  );

  return {
    /// `null` while loading or on error (see `error`).
    band: query.data ?? null,
    error: query.error?.message ?? null,
    setLower,
    setUpper,
  };
}
