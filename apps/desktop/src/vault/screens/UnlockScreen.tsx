import { useRef, useState, type FormEvent } from "react";
import { LockKeyhole } from "lucide-react";

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
import { Screen } from "./Screen";

export function UnlockScreen() {
  const { unlockVault } = useVault();
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  // The host asked for an integrity check and the vault failed it: the only way on is a backup.
  const [damaged, setDamaged] = useState(false);
  const passwordInput = useRef<HTMLInputElement>(null);

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

            <Button type="submit" disabled={password.length === 0 || submitting}>
              {submitting ? "Unlocking…" : "Unlock"}
            </Button>
          </form>

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
