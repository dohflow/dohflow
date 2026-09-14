import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type CreateScenarioInput,
  type IpcError,
  type ScenarioDto,
  type UpdateScenarioInput,
} from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// The discriminated result of creating a scenario: the stored scenario on success
/// (so the caller can auto-select it) or the typed error.
export type CreateScenarioResult =
  | { status: "ok"; data: ScenarioDto }
  | { status: "error"; error: IpcError };

/// Loads the user's scenarios (TanStack Query, ADR 0020) and exposes create /
/// delete. Deleting also invalidates the forecast + scenario-event caches (a
/// deleted scenario stops applying), so the chart and any selection refresh. IPC
/// flows through the generated `commands` only; Rust stays authoritative (ADR 0003).
export function useScenarios() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.scenarios,
    queryFn: () =>
      ipcQuery(commands.scenarioList(), "Could not load your scenarios."),
  });

  const addMutation = useMutation({
    mutationFn: (input: CreateScenarioInput) => commands.createScenario(input),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.scenarios });
      }
    },
  });
  const deleteMutation = useMutation({
    mutationFn: (id: string) => commands.deleteScenario(id),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.scenarios });
        // A deleted scenario and its overlay are gone (ADR 0051 §1).
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
        void queryClient.invalidateQueries({ queryKey: ["scenario-events"] });
      }
    },
  });
  const archiveMutation = useMutation({
    mutationFn: (id: string) => commands.archiveScenario(id),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.scenarios });
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
      }
    },
  });
  // Apply and revert both change the BASE forecast (ADR 0055), so they invalidate the
  // forecast cache as well as the scenario list — otherwise the chart would keep showing
  // pre-apply numbers until something else happened to refetch it.
  const applyMutation = useMutation({
    mutationFn: (id: string) => commands.applyScenario(id),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.scenarios });
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
      }
    },
  });
  const revertMutation = useMutation({
    mutationFn: (id: string) => commands.revertScenarioApply(id),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.scenarios });
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
      }
    },
  });
  const cloneMutation = useMutation({
    mutationFn: (input: { id: string; name: string }) =>
      commands.cloneScenario(input),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.scenarios });
      }
    },
  });
  const expiryMutation = useMutation({
    mutationFn: (input: { id: string; expires_on: string | null }) =>
      commands.setScenarioExpiry(input),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.scenarios });
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
      }
    },
  });
  const updateMutation = useMutation({
    mutationFn: (input: UpdateScenarioInput) => commands.updateScenario(input),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.scenarios });
        // Status gates whether a run applies the scenario (ADR 0051 §1, enforced in
        // `effective_scenario`), so the forecast refreshes.
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
      }
    },
  });

  // Returns the discriminated result so the caller gets the new scenario's id (to
  // select it) or the typed error to surface.
  const addScenario = useCallback(
    (input: CreateScenarioInput): Promise<CreateScenarioResult> =>
      addMutation.mutateAsync(input),
    [addMutation],
  );
  const deleteScenario = useCallback(
    async (id: string): Promise<IpcError | null> => {
      const result = await deleteMutation.mutateAsync(id);
      return result.status === "ok" ? null : result.error;
    },
    [deleteMutation],
  );
  const updateScenario = useCallback(
    async (input: UpdateScenarioInput): Promise<IpcError | null> => {
      const result = await updateMutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [updateMutation],
  );

  /// Archive: keeps every event, reversible (ADR 0051 §1).
  const archiveScenario = useCallback(
    async (id: string): Promise<IpcError | null> => {
      const result = await archiveMutation.mutateAsync(id);
      return result.status === "ok" ? null : result.error;
    },
    [archiveMutation],
  );
  /// Clone into a new draft; resolves to the new scenario's id.
  const cloneScenario = useCallback(
    async (id: string, name: string): Promise<{ id: string } | IpcError> => {
      const result = await cloneMutation.mutateAsync({ id, name });
      return result.status === "ok" ? { id: result.data } : result.error;
    },
    [cloneMutation],
  );
  /// Promote this scenario's changes into the real forecast (ADR 0055). Reversible.
  const applyScenario = useCallback(
    async (id: string): Promise<IpcError | null> => {
      const result = await applyMutation.mutateAsync(id);
      return result.status === "ok" ? null : result.error;
    },
    [applyMutation],
  );
  /// Undo an apply, restoring base exactly (ADR 0055 §5).
  const revertScenarioApply = useCallback(
    async (id: string): Promise<IpcError | null> => {
      const result = await revertMutation.mutateAsync(id);
      return result.status === "ok" ? null : result.error;
    },
    [revertMutation],
  );
  const setScenarioExpiry = useCallback(
    async (id: string, expiresOn: string | null): Promise<IpcError | null> => {
      const result = await expiryMutation.mutateAsync({ id, expires_on: expiresOn });
      return result.status === "ok" ? null : result.error;
    },
    [expiryMutation],
  );

  return {
    scenarios: query.data ?? null,
    error: query.error?.message ?? null,
    addScenario,
    deleteScenario,
    archiveScenario,
    applyScenario,
    revertScenarioApply,
    cloneScenario,
    setScenarioExpiry,
    updateScenario,
  };
}
