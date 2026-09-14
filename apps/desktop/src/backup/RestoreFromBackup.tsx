import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Loader2, Upload } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { describeIpcError, useVault } from "@/vault/useVault";

/// Restore a vault from an encrypted backup file (au3, ADR 0024). Pick the package
/// via a native Open dialog, enter its password, and the vault opens unlocked. Used
/// on the create-vault screen (fresh clone / new machine) and on the recovery
/// screen (personal-cfo-5ivp); `caption` tailors the intro copy to the context.
export function RestoreFromBackup({
  caption = "Setting up on a new machine? Restore from an encrypted backup instead.",
}: {
  caption?: string;
} = {}) {
  const { restoreVault } = useVault();
  const [packagePath, setPackagePath] = useState<string | null>(null);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onPick() {
    setError(null);
    const selected = await open({
      title: "Choose a backup to restore",
      multiple: false,
      filters: [{ name: "DohFlow backup", extensions: ["pcfobk"] }],
    });
    // `open` returns string | string[] | null; we requested a single file.
    if (typeof selected === "string") setPackagePath(selected);
  }

  async function onRestore() {
    if (!packagePath) return;
    setBusy(true);
    setError(null);
    const failure = await restoreVault(packagePath, password);
    if (failure) {
      setError(describeIpcError(failure));
      setBusy(false);
    }
    // On success the provider flips to Unlocked and the router swaps the screen.
  }

  return (
    <div className="flex flex-col gap-3 border-t pt-4">
      <p className="text-sm text-muted-foreground">{caption}</p>
      {packagePath === null ? (
        <Button variant="outline" className="self-start" onClick={onPick}>
          <Upload aria-hidden />
          Restore from backup…
        </Button>
      ) : (
        <div className="flex flex-col gap-2">
          <p className="truncate text-xs text-muted-foreground">{packagePath}</p>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="restore-password">Backup password</Label>
            <Input
              id="restore-password"
              type="password"
              autoFocus
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder="The password this backup was created with"
            />
          </div>
          {error && (
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          )}
          <div className="flex gap-2">
            <Button
              variant="ghost"
              onClick={() => {
                setPackagePath(null);
                setPassword("");
                setError(null);
              }}
            >
              Cancel
            </Button>
            <Button disabled={busy || password.length === 0} onClick={onRestore}>
              {busy && <Loader2 className="animate-spin" aria-hidden />}
              {busy ? "Restoring…" : "Restore"}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
