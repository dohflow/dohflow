import { useState, type FormEvent } from "react";
import { Lock, ShieldCheck } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
} from "@/components/ui/card";
import { describeIpcError, useVault } from "@/vault/useVault";
import { RestoreFromBackup } from "@/backup/RestoreFromBackup";
import { PasswordStrengthMeter } from "@/vault/PasswordStrengthMeter";
import { NoResetWarning } from "./NoResetWarning";
import { Screen } from "./Screen";

const MIN_LENGTH = 8;

export function CreateVaultScreen() {
  const { createVault } = useVault();
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [baseCurrency, setBaseCurrency] = useState("USD");
  const [acknowledged, setAcknowledged] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const tooShort = password.length > 0 && password.length < MIN_LENGTH;
  const mismatch = confirm.length > 0 && confirm !== password;
  const canSubmit =
    password.length >= MIN_LENGTH &&
    password === confirm &&
    acknowledged &&
    !submitting;

  async function onSubmit(event: FormEvent) {
    event.preventDefault();
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    const failure = await createVault(password, baseCurrency);
    if (failure) {
      setError(describeIpcError(failure));
      setSubmitting(false);
    }
    // On success the provider status flips to Unlocked and the router replaces
    // this screen, so there is nothing to reset here.
  }

  return (
    <Screen>
      <Card>
        <CardHeader className="items-center text-center">
          <div className="mb-2 flex size-12 items-center justify-center rounded-full bg-primary">
            <ShieldCheck className="size-6 text-primary-foreground" aria-hidden />
          </div>
          <h1 className="text-xl font-semibold tracking-tight">
            Create your vault
          </h1>
          <CardDescription>
            Set a master password to encrypt your vault.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={onSubmit} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="password">Master password</Label>
              <Input
                id="password"
                type="password"
                autoComplete="new-password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                aria-invalid={tooShort}
              />
              {tooShort && (
                <p className="text-xs text-muted-foreground">
                  Use at least {MIN_LENGTH} characters.
                </p>
              )}
              <div className="mt-1">
                <PasswordStrengthMeter password={password} />
              </div>
            </div>

            <div className="flex flex-col gap-1.5">
              <Label htmlFor="confirm">Confirm password</Label>
              <Input
                id="confirm"
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

            <div className="flex flex-col gap-1.5">
              <Label htmlFor="base-currency">Base currency</Label>
              <select
                id="base-currency"
                className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
                value={baseCurrency}
                onChange={(event) => setBaseCurrency(event.target.value)}
              >
                <option value="USD">US Dollar (USD)</option>
                <option value="EUR">Euro (EUR)</option>
              </select>
              <p className="text-xs text-muted-foreground">
                New accounts default to this. You can change it later in Settings.
              </p>
            </div>

            <NoResetWarning
              acknowledged={acknowledged}
              onAcknowledgedChange={setAcknowledged}
            />

            {error && (
              <p role="alert" className="text-sm text-loss">
                {error}
              </p>
            )}

            <Button type="submit" disabled={!canSubmit}>
              {submitting ? "Creating…" : "Create vault"}
            </Button>
          </form>

          <p className="mt-4 flex items-center justify-center gap-1.5 text-xs text-muted-foreground">
            <Lock className="size-3.5" aria-hidden />
            Encrypted on this device. Never uploaded.
          </p>

          <div className="mt-4">
            <RestoreFromBackup />
          </div>
        </CardContent>
      </Card>
    </Screen>
  );
}
