import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type CreateScenarioInput,
  type IpcError,
  type RevertPreviewDto,
  type ScenarioDto,
  type UpdateScenarioInput,
} from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// The longest scenario name the backend accepts (personal-cfo-5ymg8.4: 1 to 100 characters).
export const SCENARIO_NAME_MAX = 100;

/// The discriminated result of creating a scenario: the stored scenario on success
/// (so the caller can auto-select it) or the typed error.
export type CreateScenarioResult =
  | { status: "ok"; data: ScenarioDto }
  | { status: "error"; error: IpcError };

/// The discriminated result of reading what an undo would do.
export type RevertPreviewResult =
  | { status: "ok"; data: RevertPreviewDto }
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

  // Applied scenarios that were deleted afterwards: deleting hides a scenario but leaves its
  // apply in the forecast, so they stay reachable to be undone (personal-cfo-5ie.12).
  const deletedAppliedQuery = useQuery({
    queryKey: [...queryKeys.scenarios, "deleted-applied"],
    queryFn: () =>
      ipcQuery(
        commands.scenarioDeletedAppliedList(),
        "Could not load your deleted scenarios.",
      ),
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
    mutationFn: (input: { id: string; confirmed: string[] }) =>
      commands.revertScenarioApply(input.id, input.confirmed),
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
  /// What undoing an apply would do: the changes made to its items since, a later apply that
  /// has to be undone first, and the list to send back with the undo (personal-cfo-5ie.12).
  const previewRevert = useCallback(
    (id: string): Promise<RevertPreviewResult> =>
      commands.scenarioRevertPreview(id),
    [],
  );
  /// Undo an apply and the changes made to its items since (ADR 0055 §5). `confirmed` is the
  /// preview's `confirmed_descendants`, sent back unchanged: the backend refuses, writing
  /// nothing, if they are no longer the changes on the base.
  const revertScenarioApply = useCallback(
    async (id: string, confirmed: string[]): Promise<IpcError | null> => {
      const result = await revertMutation.mutateAsync({ id, confirmed });
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
    deletedApplied: deletedAppliedQuery.data ?? [],
    applyScenario,
    previewRevert,
    revertScenarioApply,
    cloneScenario,
    setScenarioExpiry,
    updateScenario,
  };
}
