import { useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { Download, Loader2 } from "lucide-react";

import { commands } from "@/bindings";
import { Button } from "@/components/ui/button";
import { describeIpcError } from "@/vault/useVault";
import { queryKeys } from "@/lib/query";
import { useQueryClient } from "@tanstack/react-query";

/// The manual file export inside Settings → Backups. It retains the native Save
/// dialog and typed export command; scheduled backups remain a separate action.
export function ManualBackupExport({
  disabled = false,
  onBusyChange,
}: {
  disabled?: boolean;
  onBusyChange?: (busy: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const exporting = useRef(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedTo, setSavedTo] = useState<string | null>(null);

  async function onExport() {
    if (disabled || exporting.current) return;
    exporting.current = true;
    setBusy(true);
    onBusyChange?.(true);
    setError(null);
    setSavedTo(null);
    try {
      const path = await save({
        title: "Save encrypted backup",
        defaultPath: "personal-cfo-backup.pcfobk",
        filters: [{ name: "DohFlow backup", extensions: ["pcfobk"] }],
      });
      if (!path) return; // the user cancelled the dialog
      const result = await commands.exportBackup(path);
      if (result.status === "ok") {
        await queryClient.invalidateQueries({ queryKey: queryKeys.backupHistory });
        setSavedTo(path);
      } else {
        setError(describeIpcError(result.error));
      }
    } catch {
      setError("Could not export the backup. Please try again.");
    } finally {
      exporting.current = false;
      setBusy(false);
      onBusyChange?.(false);
    }
  }

  return (
    <div className="flex flex-col gap-3 border-t pt-5">
      <p className="text-sm font-medium">Manual file export</p>
      <p className="text-sm text-muted-foreground">
        Save an encrypted copy of this vault to a file you choose. This is separate
        from the scheduled destination above; your vault password opens the backup.
      </p>
      {error && <p role="alert" className="text-sm text-loss">{error}</p>}
      {savedTo && <p role="status" className="break-all text-sm text-gain">Backup saved to {savedTo}</p>}
      <Button
        className="self-start"
        disabled={busy || disabled}
        onClick={() => void onExport()}
      >
        {busy ? <Loader2 className="animate-spin" aria-hidden /> : <Download aria-hidden />}
        {busy ? "Exporting…" : "Export backup…"}
      </Button>
    </div>
  );
}
