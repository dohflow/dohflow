/// Client-side markers behind the first-run backup nudge (personal-cfo-vdmb,
/// Launch AC#3).
///
/// Honesty note: there is no backup-history table in the vault, so "has this
/// vault ever been exported" cannot be answered by the backend. Instead
/// `BackupView` records a localStorage marker on a successful export, and the
/// nudge's dismissal is stored the same way. Clearing the WebView's storage
/// revives the nudge — an acceptable failure mode for a reminder (it shows once
/// too often, never silently never).
///
/// Keys are scoped per vault id ("default" in single-vault mode, where the
/// registry is empty) so dismissing the nudge on one vault doesn't silence it
/// for another.

import type { VaultSummaryDto } from "@/bindings";

const DISMISSED_PREFIX = "backup-nudge-dismissed:";
const EXPORTED_PREFIX = "backup-exported:";

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

export function hasExportedBackup(vaultId: string): boolean {
  return localStorage.getItem(EXPORTED_PREFIX + vaultId) !== null;
}

export function recordBackupExported(vaultId: string): void {
  localStorage.setItem(EXPORTED_PREFIX + vaultId, new Date().toISOString());
}
