import { useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { Download, Loader2 } from "lucide-react";

import { commands } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { describeIpcError } from "@/vault/useVault";
import { queryKeys } from "@/lib/query";
import { useQueryClient } from "@tanstack/react-query";

/// Export an encrypted backup of the whole vault to a user-chosen file (ADR
/// 0024-A). The unlocked kernel uses its in-memory DEK; a native Save dialog
/// picks the destination, so plaintext never leaves the worker.
export function BackupView() {
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedTo, setSavedTo] = useState<string | null>(null);

  async function onExport() {
    setError(null);
    setSavedTo(null);
    const path = await save({
      title: "Save encrypted backup",
      defaultPath: "personal-cfo-backup.pcfobk",
      filters: [{ name: "DohFlow backup", extensions: ["pcfobk"] }],
    });
    if (!path) return; // the user cancelled the dialog
    setBusy(true);
    const result = await commands.exportBackup(path);
    setBusy(false);
    if (result.status === "ok") {
      setSavedTo(path);
      void queryClient.invalidateQueries({ queryKey: queryKeys.backupHistory });
    } else {
      setError(describeIpcError(result.error));
    }
  }

  return (
    <div className="mx-auto flex w-full max-w-2xl flex-col gap-4">
      <h2 className="text-lg font-semibold tracking-tight">Backup</h2>
      <Card>
        <CardContent className="flex flex-col gap-4 pt-6">
          <p className="text-sm text-muted-foreground">
            Export an encrypted copy of your entire vault — accounts,
            transactions, income, bills, and attachments — to a single file. Keep
            it somewhere safe. Your backup opens with your vault password.
          </p>
          {error && (
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          )}
          {savedTo && (
            <p className="text-sm text-gain">Backup saved to {savedTo}</p>
          )}
          <Button
            className="self-start"
            disabled={busy}
            onClick={onExport}
          >
            {busy ? (
              <Loader2 className="animate-spin" aria-hidden />
            ) : (
              <Download aria-hidden />
            )}
            {busy ? "Exporting…" : "Export backup…"}
          </Button>
        </CardContent>
      </Card>
    </div>
  );
}
