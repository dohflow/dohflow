import { useState } from "react";
import { HardDriveDownload, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useAccounts } from "@/accounts/useAccounts";
import { useVault } from "@/vault/useVault";

import {
  activeVaultStorageId,
  dismissBackupNudge,
  hasExportedBackup,
  isBackupNudgeDismissed,
} from "./backupNudgeStorage";

/// A dismissible "export a backup" notice for the Dashboard (personal-cfo-vdmb,
/// Launch AC#3), mirroring the CapabilityUnlockNotice pattern. Shows once the
/// vault holds real data (at least one account) and the user has neither
/// exported a backup nor dismissed the notice — both tracked in localStorage
/// per vault (see `backupNudgeStorage` for why the backend can't answer this).
export function BackupNudge({ onOpenBackup }: { onOpenBackup: () => void }) {
  const { accounts } = useAccounts();
  const { vaults } = useVault();
  const vaultId = activeVaultStorageId(vaults);
  // Local mirror of the dismissal so the notice disappears immediately;
  // localStorage is the durable copy.
  const [dismissed, setDismissed] = useState(false);

  if (dismissed || isBackupNudgeDismissed(vaultId)) return null;
  if (hasExportedBackup(vaultId)) return null;
  if ((accounts?.length ?? 0) < 1) return null;

  return (
    <div
      role="status"
      className="mb-6 flex items-start gap-3 rounded-lg border border-primary/30 bg-primary/5 px-4 py-3"
    >
      <HardDriveDownload
        className="mt-0.5 size-5 shrink-0 text-primary"
        aria-hidden
      />
      <div className="flex-1">
        <p className="font-medium text-foreground">
          Your data lives only on this device — export an encrypted backup.
        </p>
        <p className="mt-0.5 text-sm text-muted-foreground">
          There is no cloud copy and no password reset; an encrypted backup is
          the only way back after a lost or broken machine.
        </p>
      </div>
      <Button variant="outline" size="sm" onClick={onOpenBackup}>
        Open Backup
      </Button>
      <Button
        variant="ghost"
        size="icon"
        className="size-8 shrink-0"
        aria-label="Dismiss backup reminder"
        onClick={() => {
          dismissBackupNudge(vaultId);
          setDismissed(true);
        }}
      >
        <X className="size-4" aria-hidden />
      </Button>
    </div>
  );
}
