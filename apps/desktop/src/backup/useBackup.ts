import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";
import { describeIpcError } from "@/vault/useVault";

export function useBackupScheduleSettings() {
  return useQuery({
    queryKey: queryKeys.backupSchedule,
    queryFn: () =>
      ipcQuery(
        commands.backupScheduleSettings(),
        "Could not load backup settings.",
      ),
    refetchInterval: 10_000,
  });
}

export function useBackupHistory() {
  return useQuery({
    queryKey: queryKeys.backupHistory,
    queryFn: () =>
      ipcQuery(commands.backupHistory(), "Could not load backup history."),
    refetchInterval: 10_000,
  });
}

export function useConfigureBackup() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (input: {
      cadence: "off" | "daily" | "weekly" | "monthly";
      destination: string | null;
      keepLast: number | null;
    }) => {
      const result = await commands.configureBackup(
        input.cadence,
        input.destination,
        input.keepLast,
      );
      if (result.status === "error") {
        throw new Error(describeIpcError(result.error));
      }
      return result.data;
    },
    onSuccess: (settings) => {
      queryClient.setQueryData(queryKeys.backupSchedule, settings);
      void queryClient.invalidateQueries({ queryKey: queryKeys.backupSchedule });
    },
  });
}

export function useRunBackupNow() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const result = await commands.runBackupNow();
      if (result.status === "error") {
        throw new Error(describeIpcError(result.error));
      }
      return result.data;
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.backupHistory });
      void queryClient.invalidateQueries({ queryKey: queryKeys.backupSchedule });
    },
  });
}
