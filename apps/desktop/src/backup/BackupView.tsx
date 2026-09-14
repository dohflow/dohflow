import { useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { Download, Loader2 } from "lucide-react";

import { commands } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent } from "@/components/ui/card";
import { describeIpcError, useVault } from "@/vault/useVault";

import {
  activeVaultStorageId,
  recordBackupExported,
} from "./backupNudgeStorage";

/// Export an encrypted backup of the whole vault to a user-chosen file (ef3,
/// ADR 0024). The user re-enters their vault password (the backup is encrypted
/// with it); a native Save dialog picks the destination, so plaintext never
/// leaves the worker.
export function BackupView() {
  const { vaults } = useVault();
  const [password, setPassword] = useState("");
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
    const result = await commands.exportBackup(password, path);
    setBusy(false);
    if (result.status === "ok") {
      setPassword("");
      setSavedTo(path);
      // Retires the Dashboard's first-run backup nudge (personal-cfo-vdmb) — a
      // localStorage marker, since the vault keeps no backup history.
      recordBackupExported(activeVaultStorageId(vaults));
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
            it somewhere safe; you&apos;ll need this password to restore it.
          </p>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="backup-password">Vault password</Label>
            <Input
              id="backup-password"
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder="Confirm your password to export"
            />
          </div>
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
            disabled={busy || password.length === 0}
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
