import { useRef, useState, type FormEvent } from "react";
import { Fingerprint, LockKeyhole } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
} from "@/components/ui/card";
import { RestoreFromBackup } from "@/backup/RestoreFromBackup";
import { describeIpcError, useVault } from "@/vault/useVault";
import { touchIdReasonCopy } from "@/vault/touchIdCopy";
import { useTouchIdStatus } from "@/vault/useTouchIdStatus";
import { Screen } from "./Screen";

export function UnlockScreen() {
  const { unlockVault, unlockWithTouchId } = useVault();
  const touchId = useTouchIdStatus();
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  // The host asked for an integrity check and the vault failed it: the only way on is a backup.
  const [damaged, setDamaged] = useState(false);
  const [touchBusy, setTouchBusy] = useState(false);
  // What the last Touch ID attempt said, when it did not unlock (a cancelled prompt, ...).
  const [touchNote, setTouchNote] = useState<string | null>(null);
  const passwordInput = useRef<HTMLInputElement>(null);

  const touchOn = touchId.status?.state === "on";
  const touchOffered = touchOn && touchId.status?.offered === true;
  // A rule that requires the password (a restart, three misses, 30 days, ...) is explained, not hidden.
  const touchRule =
    touchOn && !touchOffered && touchId.status?.reason
      ? touchIdReasonCopy(touchId.status.reason)
      : null;
  const touchMessage = touchRule ?? touchNote;

  async function onSubmit(event: FormEvent) {
    event.preventDefault();
    if (password.length === 0 || submitting) return;
    setSubmitting(true);
    setError(null);
    setDamaged(false);
    const failure = await unlockVault(password);
    if (failure) {
      setError(describeIpcError(failure));
      setDamaged(failure === "VaultIntegrityCheckFailed");
      setPassword("");
      setSubmitting(false);
      passwordInput.current?.focus();
    }
  }

  async function onTouchId() {
    if (touchBusy || submitting) return;
    setTouchBusy(true);
    setError(null);
    setDamaged(false);
    setTouchNote(null);
    const result = await unlockWithTouchId();
    if (result.kind === "unlocked") return; // the vault is open; this screen goes away
    if (result.kind === "error") {
      // The password path would report the same thing (a damaged vault, a failed read).
      setError(describeIpcError(result.error));
      setDamaged(result.error === "VaultIntegrityCheckFailed");
    } else {
      setTouchNote(touchIdReasonCopy(result.reason));
    }
    setTouchBusy(false);
    void touchId.refresh();
    passwordInput.current?.focus();
  }

  return (
    <Screen>
      <Card>
        <CardHeader className="items-center text-center">
          <div className="mb-2 flex size-12 items-center justify-center rounded-full bg-primary">
            <LockKeyhole className="size-6 text-primary-foreground" aria-hidden />
          </div>
          <h1 className="text-xl font-semibold tracking-tight">Welcome back</h1>
          <CardDescription>
            Enter your master password to unlock your vault.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={onSubmit} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="password">Master password</Label>
              <Input
                id="password"
                ref={passwordInput}
                type="password"
                autoComplete="current-password"
                autoFocus
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                aria-invalid={error !== null}
              />
            </div>

            {error && (
              <p role="alert" className="text-sm text-loss">
                {error}
              </p>
            )}

            <Button
              type="submit"
              disabled={password.length === 0 || submitting || touchBusy}
            >
              {submitting ? "Unlocking…" : "Unlock"}
            </Button>
          </form>

          {touchOffered && (
            <Button
              type="button"
              variant="outline"
              className="mt-3 w-full"
              onClick={onTouchId}
              disabled={touchBusy || submitting}
            >
              <Fingerprint className="size-4" aria-hidden />
              {touchBusy ? "Waiting for Touch ID…" : "Unlock with Touch ID"}
            </Button>
          )}
          {touchMessage && (
            <p role="status" className="mt-3 text-sm text-muted-foreground">
              {touchMessage}
            </p>
          )}

          {damaged && (
            <div className="mt-4">
              <RestoreFromBackup
                mode="newNamed"
                caption="Restore from a backup. This makes a separate vault from the backup file and leaves the damaged one untouched."
              />
            </div>
          )}
        </CardContent>
      </Card>
    </Screen>
  );
}
