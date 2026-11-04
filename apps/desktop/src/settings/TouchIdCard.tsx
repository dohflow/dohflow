import { useEffect, useRef, useState, type FormEvent } from "react";
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
import {
  TOUCH_ID_CHANGE_UNKNOWN,
  TOUCH_ID_CHECK_FAILED,
  touchIdReasonCopy,
} from "@/vault/touchIdCopy";
import { describeIpcError } from "@/vault/useVault";
import { useTouchIdStatus } from "@/vault/useTouchIdStatus";

/// The Touch ID card (personal-cfo-xuu), shown in Settings while the vault is unlocked. Turning it on asks
/// for the master password again: Touch ID becomes a standing way in, so it needs the same proof as a
/// password change. Turning it off needs nothing. Whether it can be used, and why not when it cannot,
/// comes from the host as a state and a stable reason code; the wording is in `touchIdReasonCopy`.
export function TouchIdCard() {
  const { status, loaded, refresh, setStatus } = useTouchIdStatus();
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // A call threw instead of answering, so whether Touch ID changed is not known. The card shows no
  // on/off claim until a read-only status check succeeds.
  const [unknown, setUnknown] = useState(false);
  // One persistent polite live region: the pending step and the confirmed result are each announced
  // once. It stays empty until the person does something, so loading the card says nothing.
  const [announcement, setAnnouncement] = useState("");
  // The control that held focus unmounts when the state changes, so an ended action asks for focus.
  const [refocus, setRefocus] = useState(false);
  const passwordRef = useRef<HTMLInputElement>(null);
  const turnOffRef = useRef<HTMLButtonElement>(null);
  const checkRef = useRef<HTMLButtonElement>(null);
  const titleRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!refocus || busy) return;
    const target = unknown
      ? checkRef.current
      : status?.state === "on"
        ? turnOffRef.current
        : passwordRef.current;
    (target ?? titleRef.current)?.focus();
    setRefocus(false);
  }, [refocus, busy, unknown, status]);

  function begin(pending: string) {
    setBusy(true);
    setError(null);
    setAnnouncement(pending);
  }

  function end(next: typeof status) {
    setUnknown(false);
    setAnnouncement(
      next?.state === "on"
        ? "Touch ID is on for this vault."
        : next?.state === "off"
          ? "Touch ID is off for this vault."
          : "",
    );
    setBusy(false);
    setRefocus(true);
  }

  function endUnknown(announce = "") {
    setUnknown(true);
    setAnnouncement(announce);
    setBusy(false);
    setRefocus(true);
  }

  async function onEnable(event: FormEvent) {
    event.preventDefault();
    if (password.length === 0 || busy) return;
    begin("Turning on…");
    try {
      const result = await commands.touchIdEnable({ password });
      if (result.status !== "ok") {
        setError(describeIpcError(result.error));
        setUnknown(false);
        setAnnouncement("");
        setBusy(false);
        setRefocus(true);
        return;
      }
      setPassword("");
      if (!result.data.enabled) {
        setError(touchIdReasonCopy(result.data.reason));
      }
      const read = await commands.touchIdStatus();
      setStatus(read);
      end(read);
    } catch {
      // The typed password stays: nothing says whether it was used.
      endUnknown();
    }
  }

  async function onDisable() {
    if (busy) return;
    begin("Turning off…");
    try {
      const result = await commands.touchIdDisable();
      if (result.status === "ok") {
        setStatus(result.data);
        end(result.data);
      } else {
        setError(describeIpcError(result.error));
        setUnknown(false);
        setAnnouncement("");
        setBusy(false);
        setRefocus(true);
      }
    } catch {
      endUnknown();
    }
  }

  // Read-only: asks the host for its status and nothing else (no prompt, no write).
  async function onCheck() {
    if (busy) return;
    begin("Checking Touch ID…");
    const read = await refresh();
    if (read) {
      end(read);
    } else {
      // Every failed check is announced. A second identical failure is still heard because each
      // check first announces "Checking Touch ID…", so the region's text changes between the two.
      endUnknown(TOUCH_ID_CHECK_FAILED);
    }
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle ref={titleRef} tabIndex={-1} className="text-sm outline-none">
          Touch ID
        </CardTitle>
        <CardDescription>
          Unlock this vault with your fingerprint instead of typing your master
          password. Your master password stays the way in: it is asked for
          after you restart your Mac, after three Touch ID tries that fail or
          are canceled, and every 30 days.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {/* Persistent so a screen reader hears each change; empty until the person acts. */}
        <div
          aria-live="polite"
          aria-atomic="true"
          data-testid="touch-id-announcement"
          className="sr-only"
        >
          {announcement}
        </div>
        {unknown ? (
          <div className="flex flex-col gap-3">
            <p role="alert" className="text-sm text-loss">
              {TOUCH_ID_CHANGE_UNKNOWN}
            </p>
            <Button
              ref={checkRef}
              type="button"
              size="sm"
              variant="outline"
              className="self-start"
              onClick={onCheck}
              disabled={busy}
            >
              {busy && <Loader2 className="size-4 animate-spin" aria-hidden />}
              Check Touch ID status
            </Button>
          </div>
        ) : !loaded ? (
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
              ref={turnOffRef}
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
                ref={passwordRef}
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

        {error && !unknown && (
          <p role="alert" className="text-sm text-loss">
            {error}
          </p>
        )}
      </CardContent>
    </Card>
  );
}
