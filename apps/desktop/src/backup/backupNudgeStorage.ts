/// The backup nudge's dismissal is a viewer preference in localStorage.
/// Whether this vault has ever been backed up comes from encrypted local
/// `backup_history` via IPC, never from a browser-storage marker.

import type { VaultSummaryDto } from "@/bindings";

const DISMISSED_PREFIX = "backup-nudge-dismissed:";

/// The storage id for the active vault — its registry id, or "default" when the
/// registry is empty (single-vault mode) or hasn't loaded yet.
export function activeVaultStorageId(vaults: VaultSummaryDto[]): string {
  return vaults.find((vault) => vault.is_active)?.id ?? "default";
}

export function isBackupNudgeDismissed(vaultId: string): boolean {
  return localStorage.getItem(DISMISSED_PREFIX + vaultId) !== null;
}

export function dismissBackupNudge(vaultId: string): void {
  localStorage.setItem(DISMISSED_PREFIX + vaultId, new Date().toISOString());
}
