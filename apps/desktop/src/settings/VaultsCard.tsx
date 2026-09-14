import { useState } from "react";
import { Check, Database, Loader2 } from "lucide-react";

import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { describeIpcError, useVault } from "@/vault/useVault";

/// The multi-vault management card (personal-cfo-j0cg.6, ADR 0042): list the known vaults, switch
/// between them (which locks the current one and routes to the unlock screen), rename them, and
/// create a new one. Only shown in multi-vault mode (an unlocked real vault always has ≥1 entry).
export function VaultsCard() {
  const { vaults, createNamedVault, switchVault, renameVault } = useVault();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [newPassword, setNewPassword] = useState("");

  if (vaults.length === 0) return null;

  async function onSwitch(id: string) {
    setBusy(true);
    setError(null);
    const failure = await switchVault(id);
    setBusy(false);
    if (failure) setError(describeIpcError(failure));
    // On success the status flips to Locked and the vault gate shows the unlock screen.
  }

  async function saveRename(id: string) {
    if (!renameValue.trim()) return;
    setBusy(true);
    setError(null);
    const failure = await renameVault(id, renameValue.trim());
    setBusy(false);
    if (failure) setError(describeIpcError(failure));
    else setRenamingId(null);
  }

  async function onCreate() {
    if (!newName.trim()) {
      setError("Name the vault.");
      return;
    }
    if (!newPassword) {
      setError("Choose a password.");
      return;
    }
    setBusy(true);
    setError(null);
    const failure = await createNamedVault(newName.trim(), newPassword);
    setBusy(false);
    if (failure) {
      setError(describeIpcError(failure));
    } else {
      setCreating(false);
      setNewName("");
      setNewPassword("");
    }
    // On success the new (empty) vault is unlocked and active — the app stays open in it.
  }

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 p-4">
        <div className="flex items-center gap-2">
          <Database className="size-4 text-muted-foreground" aria-hidden />
          <h3 className="font-semibold">Vaults</h3>
        </div>
        <p className="text-sm text-muted-foreground">
          Keep separate vaults — for example your real data and a throwaway test one. Switching locks
          the current vault; you unlock the other with its own password.
        </p>

        <ul className="flex flex-col divide-y rounded-md border">
          {vaults.map((v) => (
            <li key={v.id} className="flex items-center gap-2 px-3 py-2 text-sm">
              {renamingId === v.id ? (
                <>
                  <Input
                    aria-label={`New name for ${v.name}`}
                    className="h-8 flex-1"
                    value={renameValue}
                    onChange={(e) => setRenameValue(e.target.value)}
                  />
                  <Button size="sm" disabled={busy} onClick={() => void saveRename(v.id)}>
                    Save
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy}
                    onClick={() => setRenamingId(null)}
                  >
                    Cancel
                  </Button>
                </>
              ) : (
                <>
                  <span className="flex-1 truncate font-medium">{v.name}</span>
                  {v.is_active && (
                    <span className="inline-flex items-center gap-1 rounded-full bg-gain/10 px-2 py-0.5 text-xs text-gain">
                      <Check className="size-3" aria-hidden /> Active
                    </span>
                  )}
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy}
                    onClick={() => {
                      setRenamingId(v.id);
                      setRenameValue(v.name);
                    }}
                  >
                    Rename
                  </Button>
                  {!v.is_active && (
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={busy}
                      onClick={() => void onSwitch(v.id)}
                    >
                      Switch
                    </Button>
                  )}
                </>
              )}
            </li>
          ))}
        </ul>

        {creating ? (
          <div className="flex flex-col gap-2 rounded-md border bg-muted/30 p-3">
            <div className="flex flex-col gap-1">
              <Label htmlFor="new-vault-name" className="text-xs">
                Name
              </Label>
              <Input
                id="new-vault-name"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                placeholder="e.g. Test"
              />
            </div>
            <div className="flex flex-col gap-1">
              <Label htmlFor="new-vault-password" className="text-xs">
                Password
              </Label>
              <Input
                id="new-vault-password"
                type="password"
                value={newPassword}
                onChange={(e) => setNewPassword(e.target.value)}
              />
            </div>
            <div className="flex gap-2">
              <Button size="sm" disabled={busy} onClick={() => void onCreate()}>
                {busy && <Loader2 className="size-4 animate-spin" aria-hidden />}
                Create vault
              </Button>
              <Button
                size="sm"
                variant="ghost"
                disabled={busy}
                onClick={() => {
                  setCreating(false);
                  setError(null);
                }}
              >
                Cancel
              </Button>
            </div>
          </div>
        ) : (
          <Button
            variant="outline"
            size="sm"
            className="self-start"
            disabled={busy}
            onClick={() => setCreating(true)}
          >
            New vault…
          </Button>
        )}

        {error && (
          <p role="alert" className="text-sm text-loss">
            {error}
          </p>
        )}
      </CardContent>
    </Card>
  );
}
