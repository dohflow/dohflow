import { useState, type FormEvent } from "react";
import { KeyRound, Loader2, RefreshCcw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { describeIpcError, useVault } from "@/vault/useVault";

/// The rotate-encryption-key card (personal-cfo-2y8, ADR 0083), shown in
/// Settings while the vault is unlocked. Replaces the vault's data key without
/// changing the password, for when the key may have been exposed. Asks for the
/// current password, states what rotation does not protect (ADR 0083 §6), and
/// covers the whole app while the rotation runs: every other command waits on
/// the same lock, and the app cannot observe the in-progress state (§5).
export function RotateKeyCard() {
  const { rotateVaultKey } = useVault();
  const [open, setOpen] = useState(false);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [rotated, setRotated] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function reset() {
    setOpen(false);
    setPassword("");
    setError(null);
  }

  async function onSubmit(event: FormEvent) {
    event.preventDefault();
    if (password.length === 0 || busy) return;
    setBusy(true);
    setRotated(false);
    setError(null);
    const failure = await rotateVaultKey(password);
    setBusy(false);
    setPassword("");
    if (failure) {
      setError(describeIpcError(failure));
    } else {
      setRotated(true);
      setOpen(false);
    }
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm">Rotate encryption key</CardTitle>
        <CardDescription>
          Replace the key that encrypts this vault with a new one, without
          changing your password. Use this if you think the vault's key may
          have been exposed.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {rotated && !open && (
          <p className="text-sm text-gain">Encryption key rotated.</p>
        )}
        {!open ? (
          <Button
            variant="outline"
            size="sm"
            className="self-start"
            onClick={() => {
              setRotated(false);
              setOpen(true);
            }}
          >
            <RefreshCcw className="size-4" aria-hidden />
            Rotate key…
          </Button>
        ) : (
          <form onSubmit={onSubmit} className="flex flex-col gap-3">
            <ul className="flex list-disc flex-col gap-1 pl-5 text-xs text-muted-foreground">
              <li>Your data, attachments and password stay the same.</li>
              <li>
                Backups made before rotating still open with the password you
                had when you made them. If you think the old key leaked, delete
                old backups you no longer need.
              </li>
              <li>
                Copies of this vault made before rotating can still be opened
                with the old key.
              </li>
              <li>
                It needs free disk space for one copy of your vault, and DohFlow
                is unavailable until it finishes.
              </li>
            </ul>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="rotate-key-password">Current password</Label>
              <Input
                id="rotate-key-password"
                type="password"
                autoComplete="current-password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                autoFocus
              />
            </div>
            {error && (
              <p role="alert" className="text-sm text-loss">
                {error}
              </p>
            )}
            <div className="flex gap-2">
              <Button
                type="submit"
                size="sm"
                disabled={password.length === 0 || busy}
              >
                <KeyRound className="size-4" aria-hidden />
                Rotate key
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={busy}
                onClick={reset}
              >
                Cancel
              </Button>
            </div>
          </form>
        )}
      </CardContent>
      {busy && (
        <div
          role="alertdialog"
          aria-modal="true"
          aria-busy="true"
          aria-labelledby="rotate-key-progress"
          className="fixed inset-0 z-[70] flex items-center justify-center bg-foreground/50 p-6 backdrop-blur-[3px]"
        >
          <div className="flex max-w-sm flex-col items-center gap-3 rounded-lg border bg-background p-6 text-center shadow-lg">
            <Loader2 className="size-6 animate-spin" aria-hidden />
            <p id="rotate-key-progress" className="font-medium">
              Re-encrypting your vault…
            </p>
            <p className="text-sm text-muted-foreground">
              Don't quit DohFlow. This can take a minute on a large vault.
            </p>
          </div>
        </div>
      )}
    </Card>
  );
}
