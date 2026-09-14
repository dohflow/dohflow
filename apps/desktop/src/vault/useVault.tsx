import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type IpcError,
  type VaultStatusDto,
  type VaultSummaryDto,
} from "@/bindings";

type VaultResult =
  | { status: "ok"; data: VaultStatusDto }
  | { status: "error"; error: IpcError };

type VaultContextValue = {
  /// The current vault status, or `null` while the first status load is pending.
  status: VaultStatusDto | null;
  /// A non-recoverable load/IPC failure (distinct from an action error).
  loadError: string | null;
  /// Lifecycle actions. Each resolves to `null` on success (status updates) or
  /// the `IpcError` so the calling screen can show it inline.
  createVault: (
    password: string,
    baseCurrency?: string,
  ) => Promise<IpcError | null>;
  unlockVault: (password: string) => Promise<IpcError | null>;
  lockVault: () => Promise<IpcError | null>;
  /// Permanently delete the vault and all its data (j0cg.5); on success the status
  /// flips to NoVault and the app returns to the create-vault screen. Recoverable
  /// only by restoring an encrypted backup.
  deleteVault: () => Promise<IpcError | null>;
  /// Restore an encrypted backup into a fresh vault (au3); on success the status
  /// flips to Unlocked.
  restoreVault: (packagePath: string, password: string) => Promise<IpcError | null>;
  /// Re-read the on-disk status (used by the recovery screen).
  refresh: () => Promise<void>;
  /// The known vaults + which is active (multi-vault, j0cg.6). Empty in single-vault mode.
  vaults: VaultSummaryDto[];
  /// Create a new named vault and switch to it, unlocked; the status flips to Unlocked.
  createNamedVault: (name: string, password: string) => Promise<IpcError | null>;
  /// Switch to another vault; it comes up locked, so the status flips to Locked.
  switchVault: (id: string) => Promise<IpcError | null>;
  /// Rename a vault (registry only; the status is unaffected).
  renameVault: (id: string, name: string) => Promise<IpcError | null>;
};

const VaultContext = createContext<VaultContextValue | null>(null);

/// Owns the vault status and the typed lifecycle calls. All IPC flows through
/// the generated `commands` (never a raw `invoke`).
export function VaultProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<VaultStatusDto | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [vaults, setVaults] = useState<VaultSummaryDto[]>([]);
  const queryClient = useQueryClient();

  const refresh = useCallback(async () => {
    try {
      const result = await commands.vaultStatus();
      if (result.status === "ok") {
        setStatus(result.data);
        setLoadError(null);
      } else {
        setLoadError(describeIpcError(result.error));
      }
    } catch {
      setLoadError("The vault service is unavailable.");
    }
  }, []);

  const refreshVaults = useCallback(async () => {
    const result = await commands.listVaults();
    if (result.status === "ok") setVaults(result.data.vaults);
  }, []);

  useEffect(() => {
    void refresh();
    void refreshVaults();
  }, [refresh, refreshVaults]);

  const run = useCallback(
    async (call: Promise<VaultResult>): Promise<IpcError | null> => {
      const result = await call;
      if (result.status === "ok") {
        setStatus(result.data);
        return null;
      }
      return result.error;
    },
    [],
  );

  const createVault = useCallback(
    async (
      password: string,
      baseCurrency?: string,
    ): Promise<IpcError | null> => {
      const result = await commands.createVault(password);
      if (result.status !== "ok") return result.error;
      // Record the no-reset-warning acknowledgement on the freshly created vault
      // (personal-cfo-n7bo) before surfacing it as unlocked. The gating checkbox
      // lives in CreateVaultScreen; a failure here is logged but must not strand
      // the user from their new vault.
      const ack = await commands.acknowledgeNoResetWarning();
      if (ack.status !== "ok") {
        console.error(
          "failed to record no-reset-warning acknowledgement",
          ack.error,
        );
      }
      // Seed the chosen base currency (personal-cfo-4d8.1) before the unlocked
      // app reads it; likewise non-fatal — the default is USD either way.
      if (baseCurrency) {
        const cur = await commands.setBaseCurrency(baseCurrency);
        if (cur.status !== "ok") {
          console.error("failed to set the base currency", cur.error);
        }
      }
      setStatus(result.data);
      return null;
    },
    [],
  );
  const unlockVault = useCallback(
    (password: string) => run(commands.unlockVault(password)),
    [run],
  );
  const lockVault = useCallback(async () => {
    const error = await run(commands.lockVault());
    // No financial read-model data may linger in the cache after lock (ADR 0003).
    if (error === null) queryClient.clear();
    return error;
  }, [run, queryClient]);
  const deleteVault = useCallback(async () => {
    const error = await run(commands.deleteVault());
    if (error === null) {
      // Every read-model in the cache belongs to a vault that no longer exists.
      queryClient.clear();
      await refreshVaults();
    } else {
      // A failed wipe may have partly removed the vault; re-read the real on-disk status so the
      // gate reflects it (e.g. a recovery screen) rather than a stale "unlocked".
      await refresh();
    }
    return error;
  }, [run, queryClient, refresh, refreshVaults]);
  const restoreVault = useCallback(
    (packagePath: string, password: string) =>
      run(commands.restoreBackup(packagePath, password)),
    [run],
  );
  const createNamedVault = useCallback(
    async (name: string, password: string) => {
      // Switching to a fresh vault means none of the previous vault's cache applies.
      queryClient.clear();
      const error = await run(commands.createVaultNamed(name, password));
      await refreshVaults();
      return error;
    },
    [run, queryClient, refreshVaults],
  );
  const switchVault = useCallback(
    async (id: string) => {
      // The target comes up locked with different data; drop the current vault's cache.
      queryClient.clear();
      const error = await run(commands.switchVault(id));
      await refreshVaults();
      return error;
    },
    [run, queryClient, refreshVaults],
  );
  const renameVault = useCallback(
    async (id: string, name: string) => {
      const result = await commands.renameVault(id, name);
      if (result.status === "ok") {
        setVaults(result.data.vaults);
        return null;
      }
      return result.error;
    },
    [],
  );

  return (
    <VaultContext.Provider
      value={{
        status,
        loadError,
        createVault,
        unlockVault,
        lockVault,
        deleteVault,
        restoreVault,
        refresh,
        vaults,
        createNamedVault,
        switchVault,
        renameVault,
      }}
    >
      {children}
    </VaultContext.Provider>
  );
}

export function useVault(): VaultContextValue {
  const ctx = useContext(VaultContext);
  if (!ctx) {
    throw new Error("useVault must be used within a VaultProvider");
  }
  return ctx;
}

/// Turn an `IpcError` into a user-facing message. String variants like
/// `"VaultUnlockFailed"` get friendly copy; object variants carry their message.
export function describeIpcError(error: IpcError): string {
  if (typeof error === "string") {
    switch (error) {
      case "VaultUnlockFailed":
        return "Incorrect password. Please try again.";
      case "VaultLocked":
        return "The vault is locked.";
      case "WriterPanicked":
        return "A write failed and the vault needs recovery.";
      default:
        return error;
    }
  }
  const [, message] = Object.entries(error)[0] ?? ["", "Something went wrong."];
  return typeof message === "string" ? message : "Something went wrong.";
}
