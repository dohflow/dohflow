import { useState, type FormEvent } from "react";
import { Fingerprint, Loader2 } from "lucide-react";

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
import { touchIdReasonCopy } from "@/vault/touchIdCopy";
import { describeIpcError } from "@/vault/useVault";
import { useTouchIdStatus } from "@/vault/useTouchIdStatus";

/// The Touch ID card (personal-cfo-xuu), shown in Settings while the vault is unlocked. Turning it on asks
/// for the master password again: Touch ID becomes a standing way in, so it needs the same proof as a
/// password change. Turning it off needs nothing. Whether it can be used, and why not when it cannot,
/// comes from the host as a state and a stable reason code; the wording is in `touchIdReasonCopy`.
export function TouchIdCard() {
  const { status, loaded, setStatus } = useTouchIdStatus();
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onEnable(event: FormEvent) {
    event.preventDefault();
    if (password.length === 0 || busy) return;
    setBusy(true);
    setError(null);
    const result = await commands.touchIdEnable({ password });
    if (result.status !== "ok") {
      setError(describeIpcError(result.error));
    } else {
      setPassword("");
      if (!result.data.enabled) {
        setError(touchIdReasonCopy(result.data.reason));
      }
      setStatus(await commands.touchIdStatus());
    }
    setBusy(false);
  }

  async function onDisable() {
    if (busy) return;
    setBusy(true);
    setError(null);
    const result = await commands.touchIdDisable();
    if (result.status === "ok") {
      setStatus(result.data);
    } else {
      setError(describeIpcError(result.error));
    }
    setBusy(false);
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm">Touch ID</CardTitle>
        <CardDescription>
          Unlock this vault with your fingerprint instead of typing your master
          password. Your master password stays the way in: it is asked for
          after you restart your Mac, after three Touch ID tries that fail or
          are cancelled, and every 30 days.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {!loaded ? (
          <p className="flex items-center gap-1.5 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" aria-hidden />
            Checking Touch ID…
          </p>
        ) : status === null ? (
          <p className="text-sm text-muted-foreground">
            Touch ID status could not be read. Your master password still works.
          </p>
        ) : status.state === "unavailable" ? (
          <p className="text-sm text-muted-foreground">
            {touchIdReasonCopy(status.reason)}
          </p>
        ) : status.state === "on" ? (
          <div className="flex flex-col gap-3">
            <p className="flex items-center gap-1.5 text-sm">
              <Fingerprint className="size-4 text-gain" aria-hidden />
              Touch ID is on for this vault.
            </p>
            {!status.offered && status.reason && (
              <p role="status" className="text-sm text-muted-foreground">
                {touchIdReasonCopy(status.reason)}
              </p>
            )}
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="self-start"
              onClick={onDisable}
              disabled={busy}
            >
              {busy && <Loader2 className="size-4 animate-spin" aria-hidden />}
              Turn off Touch ID
            </Button>
          </div>
        ) : (
          <form onSubmit={onEnable} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="touch-id-password">Master password</Label>
              <Input
                id="touch-id-password"
                type="password"
                autoComplete="current-password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                Enter it again to turn Touch ID on.
              </p>
            </div>
            <Button
              type="submit"
              size="sm"
              className="self-start"
              disabled={password.length === 0 || busy}
            >
              {busy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden />
              ) : (
                <Fingerprint className="size-4" aria-hidden />
              )}
              {busy ? "Turning on…" : "Turn on Touch ID"}
            </Button>
          </form>
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
