import { useState } from "react";
import { Check, Database, Loader2 } from "lucide-react";

import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { describeIpcError, useVault } from "@/vault/useVault";

import { useAttemptCount, useReannounced } from "./attemptKey";
import { PersistentNotice } from "./PersistentSlot";
import { useKeepFocusAcrossMove } from "./useKeepFocusAcrossMove";

/// What failed, so the section's persistent slot can show the error with the one action that
/// recovers from it: Switch (for that vault), Save (the rename in progress) or Create vault.
type Failure =
  | { action: "switch"; id: string; message: string }
  | { action: "rename"; id: string; message: string }
  | { action: "create"; message: string };

/// The multi-vault management card (personal-cfo-j0cg.6, ADR 0042): list the known vaults, switch
/// between them (which locks the current one and routes to the unlock screen), rename them, and
/// create a new one. Only shown in multi-vault mode (an unlocked real vault always has ≥1 entry).
export function VaultsCard() {
  const { vaults, vaultsError, reloadVaults, createNamedVault, switchVault, renameVault } =
    useVault();
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [newPassword, setNewPassword] = useState("");
  // Each recovery action is rendered once: in the persistent slot while its error shows, else in
  // the body. Moving it must not strand focus.
  const retry = useKeepFocusAcrossMove({ settled: !busy });
  const switchFocus = useKeepFocusAcrossMove({ settled: !busy });
  // Every non-active vault has its own Switch, but one hand-over serves them all: its ref goes on
  // the Switch of the vault last pressed, wherever that button is rendered.
  const [switchPressed, setSwitchPressed] = useState<string | null>(null);
  const saveFocus = useKeepFocusAcrossMove({ settled: !busy });
  const createFocus = useKeepFocusAcrossMove({ settled: !busy });

  // Each settled attempt that fails clears and restores its alert text, so an identical repeat is
  // announced again (the error is kept on screen through the retry).
  const [failureAttempt, nextFailureAttempt] = useAttemptCount();
  const [loadAttempt, nextLoadAttempt] = useAttemptCount();
  const failureAlert = useReannounced(failure?.message ?? null, failureAttempt);
  const loadAlert = useReannounced(vaultsError, loadAttempt);
  function fail(next: Failure) {
    setFailure(next);
    nextFailureAttempt();
  }

  if (vaults.length === 0 && !vaultsError) return null;

  async function onSwitch(id: string) {
    setBusy(true);
    // An earlier error stays, with its Switch, until this try settles.
    const result = await switchVault(id);
    setBusy(false);
    if (result) fail({ action: "switch", id, message: describeIpcError(result) });
    else setFailure(null);
    // On success the status flips to Locked and the vault gate shows the unlock screen.
  }

  async function saveRename(id: string) {
    if (!renameValue.trim()) return;
    saveFocus.remember();
    setBusy(true);
    const result = await renameVault(id, renameValue.trim());
    setBusy(false);
    if (result) {
      fail({ action: "rename", id, message: describeIpcError(result) });
    } else {
      setFailure(null);
      setRenamingId(null);
    }
  }

  async function onCreate() {
    createFocus.remember();
    if (!newName.trim()) {
      fail({ action: "create", message: "Name the vault." });
      return;
    }
    if (!newPassword) {
      fail({ action: "create", message: "Choose a password." });
      return;
    }
    setBusy(true);
    const result = await createNamedVault(newName.trim(), newPassword);
    setBusy(false);
    if (result) {
      fail({ action: "create", message: describeIpcError(result) });
    } else {
      setFailure(null);
      setCreating(false);
      setNewName("");
      setNewPassword("");
    }
    // On success the new (empty) vault is unlocked and active — the app stays open in it.
  }

  // An error, its full message and its one recovery action stay in the section's persistent slot,
  // open or closed. A failed list read recovers with Retry; a failed switch with Switch (shown
  // beside the vault it is for), a failed rename with Save, a failed create with Create vault.
  const switchInSlot = failure?.action === "switch" ? failure.id : null;
  const renameInSlot = failure?.action === "rename" ? failure.id : null;
  const createInSlot = failure?.action === "create";
  const retryButton = (
    <Button
      ref={retry.ref}
      size="sm"
      variant="outline"
      disabled={busy}
      onClick={() => {
        retry.remember();
        void reloadVaults().then(nextLoadAttempt);
      }}
    >
      Retry
    </Button>
  );
  const loadNotice = vaultsError ? (
    <div className="flex flex-col gap-2">
      <p role="alert" className="text-sm text-loss">
        {loadAlert}
      </p>
      <div>{retryButton}</div>
    </div>
  ) : null;

  if (vaults.length === 0) return <PersistentNotice>{loadNotice}</PersistentNotice>;

  const switchTarget = vaults.find((v) => v.id === switchInSlot);
  const switchButton = (id: string) => (
    <Button
      ref={switchPressed === id ? switchFocus.ref : undefined}
      size="sm"
      variant="outline"
      disabled={busy}
      onClick={(event) => {
        switchFocus.remember(event.currentTarget);
        setSwitchPressed(id);
        void onSwitch(id);
      }}
    >
      Switch
    </Button>
  );
  const saveButton = (id: string) => (
    <Button ref={saveFocus.ref} size="sm" disabled={busy} onClick={() => void saveRename(id)}>
      Save
    </Button>
  );
  const createButton = (
    <Button ref={createFocus.ref} size="sm" disabled={busy} onClick={() => void onCreate()}>
      {busy && <Loader2 className="size-4 animate-spin" aria-hidden />}
      Create vault
    </Button>
  );

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
                  {renameInSlot !== v.id && saveButton(v.id)}
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy}
                    onClick={() => {
                      setRenamingId(null);
                      setFailure(null);
                    }}
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
                      setFailure(null);
                    }}
                  >
                    Rename
                  </Button>
                  {!v.is_active && switchInSlot !== v.id && switchButton(v.id)}
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
              {!createInSlot && createButton}
              <Button
                size="sm"
                variant="ghost"
                disabled={busy}
                onClick={() => {
                  setCreating(false);
                  setFailure(null);
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

        <PersistentNotice>
          {loadNotice}
          {failure && (
            <div className="flex flex-col gap-2">
              <p role="alert" className="text-sm text-loss">
                {failureAlert}
              </p>
              <div className="flex items-center gap-2">
                {failure.action === "switch" && switchTarget && (
                  <span className="truncate text-sm font-medium">{switchTarget.name}</span>
                )}
                {failure.action === "switch" && switchButton(failure.id)}
                {failure.action === "rename" && saveButton(failure.id)}
                {failure.action === "create" && createButton}
              </div>
            </div>
          )}
        </PersistentNotice>
      </CardContent>
    </Card>
  );
}
