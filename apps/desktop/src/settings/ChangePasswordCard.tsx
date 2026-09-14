import { useState, type FormEvent } from "react";
import { KeyRound, Loader2, TriangleAlert } from "lucide-react";

import { commands } from "@/bindings";
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
import { describeIpcError } from "@/vault/useVault";
import { PasswordStrengthMeter } from "@/vault/PasswordStrengthMeter";

/// Mirrors CreateVaultScreen's MIN_LENGTH (and the backend boundary check).
const MIN_LENGTH = 8;

/// The change-password card (personal-cfo-zxq), shown in Settings while the
/// vault is unlocked. Verifies the current password and atomically rewraps the
/// vault key under the new one — the data itself is never re-encrypted. A wrong
/// current password surfaces inline; there is still no reset if the new
/// password is forgotten, so the no-reset reminder is repeated here.
export function ChangePasswordCard() {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [changed, setChanged] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const tooShort = next.length > 0 && next.length < MIN_LENGTH;
  const mismatch = confirm.length > 0 && confirm !== next;
  const canSubmit =
    current.length > 0 && next.length >= MIN_LENGTH && next === confirm && !busy;

  async function onSubmit(event: FormEvent) {
    event.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setChanged(false);
    setError(null);
    const result = await commands.changePassword({
      old_password: current,
      new_password: next,
    });
    setBusy(false);
    if (result.status === "ok") {
      setChanged(true);
      setCurrent("");
      setNext("");
      setConfirm("");
    } else {
      setError(describeIpcError(result.error));
    }
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm">Change master password</CardTitle>
        <CardDescription>
          Re-protect this vault with a new password. Your data stays as it is —
          only the key that unlocks it changes.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={onSubmit} className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="current-password">Current password</Label>
            <Input
              id="current-password"
              type="password"
              autoComplete="current-password"
              value={current}
              onChange={(event) => setCurrent(event.target.value)}
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="new-password">New password</Label>
            <Input
              id="new-password"
              type="password"
              autoComplete="new-password"
              value={next}
              onChange={(event) => setNext(event.target.value)}
              aria-invalid={tooShort}
            />
            {tooShort && (
              <p className="text-xs text-muted-foreground">
                Use at least {MIN_LENGTH} characters.
              </p>
            )}
            <div className="mt-1">
              <PasswordStrengthMeter password={next} />
            </div>
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="confirm-new-password">Confirm new password</Label>
            <Input
              id="confirm-new-password"
              type="password"
              autoComplete="new-password"
              value={confirm}
              onChange={(event) => setConfirm(event.target.value)}
              aria-invalid={mismatch}
            />
            {mismatch && (
              <p className="text-xs text-loss">Passwords do not match.</p>
            )}
          </div>

          <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
            <TriangleAlert
              className="mt-0.5 size-3.5 shrink-0 text-warning"
              aria-hidden
            />
            There is still no password reset. If you forget the new password,
            your data is unrecoverable — save it somewhere safe.
          </p>

          {error && (
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          )}
          {changed && !busy && (
            <p className="text-sm text-gain">Password changed.</p>
          )}

          <Button
            type="submit"
            size="sm"
            className="self-start"
            disabled={!canSubmit}
          >
            {busy ? (
              <Loader2 className="size-4 animate-spin" aria-hidden />
            ) : (
              <KeyRound className="size-4" aria-hidden />
            )}
            {busy ? "Changing…" : "Change password"}
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}
