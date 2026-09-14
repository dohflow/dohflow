import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type CreateForecastAssumptionInput,
  type IpcError,
} from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Loads one scenario's active assumption events (additions / amount+date
/// modifications / removals) and exposes add / delete (TanStack Query, ADR 0020).
/// Disabled until a scenario is selected (`scenarioId` non-null). Each mutation
/// invalidates this scenario's events + the forecast caches, so the chart and the
/// compare delta refresh. IPC flows through the generated `commands` only.
export function useScenarioEvents(scenarioId: string | null) {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.scenarioEvents(scenarioId),
    queryFn: () =>
      ipcQuery(
        commands.forecastAssumptionList(scenarioId),
        "Could not load this scenario's changes.",
      ),
    enabled: scenarioId !== null,
  });

  const invalidate = useCallback(() => {
    void queryClient.invalidateQueries({
      queryKey: queryKeys.scenarioEvents(scenarioId),
    });
    void queryClient.invalidateQueries({ queryKey: ["forecast"] });
    void queryClient.invalidateQueries({ queryKey: queryKeys.cashAvailability });
  }, [queryClient, scenarioId]);

  const addMutation = useMutation({
    mutationFn: (input: CreateForecastAssumptionInput) =>
      commands.createForecastAssumption(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const deleteMutation = useMutation({
    mutationFn: (id: string) => commands.deleteForecastAssumption(id),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });

  const addEvent = useCallback(
    async (input: CreateForecastAssumptionInput): Promise<IpcError | null> => {
      const result = await addMutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [addMutation],
  );
  const deleteEvent = useCallback(
    async (id: string): Promise<IpcError | null> => {
      const result = await deleteMutation.mutateAsync(id);
      return result.status === "ok" ? null : result.error;
    },
    [deleteMutation],
  );

  return {
    events: query.data ?? null,
    error: query.error?.message ?? null,
    addEvent,
    deleteEvent,
  };
}
