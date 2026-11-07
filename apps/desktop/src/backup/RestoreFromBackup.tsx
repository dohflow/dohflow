import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Upload } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Spinner } from "@/components/ui/spinner";
import { describeIpcError, useVault } from "@/vault/useVault";
import { pickedFilePath } from "./pickedPath";

/// Restore a vault from an encrypted backup file (au3, ADR 0024). Pick the package
/// via a native Open dialog, enter its password, and the vault opens unlocked. A fresh
/// install keeps the original no-vault destination; a locked picker/recovery screen
/// creates a separate named vault while retaining the existing vault's files.
export function RestoreFromBackup({
  caption = "Setting up on a new machine? Restore from an encrypted backup instead.",
  mode = "fresh",
}: {
  caption?: string;
  mode?: "fresh" | "newNamed";
} = {}) {
  const { restoreVault, restoreAsNewVault } = useVault();
  const id = useId();
  const pickerButton = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef(false);
  // Set when the native picker closed without a file (cancelled, or failed): focus goes back to the
  // button that opened it. The button is disabled while the picker is open, so a disabled element
  // cannot hold focus and the browser drops it; it is restored once the button is enabled again.
  const returnFocusAfterPick = useRef(false);
  const submitting = useRef(false);
  const [packagePath, setPackagePath] = useState<string | null>(null);
  const [password, setPassword] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (packagePath === null && returnFocus.current) {
      pickerButton.current?.focus();
      returnFocus.current = false;
    }
  }, [packagePath]);

  useEffect(() => {
    if (!picking && packagePath === null && returnFocusAfterPick.current) {
      returnFocusAfterPick.current = false;
      pickerButton.current?.focus();
    }
  }, [picking, packagePath]);

  async function onPick() {
    if (picking || submitting.current) return;
    setPicking(true);
    setError(null);
    try {
      const selected = await open({
        title: "Choose a backup to restore",
        multiple: false,
        filters: [{ name: "DohFlow backup", extensions: ["pcfobk"] }],
      });
      // `open` returns string | string[] | null; we requested a single file.
      if (typeof selected === "string") setPackagePath(pickedFilePath(selected));
      else returnFocusAfterPick.current = true;
    } catch {
      setError("The backup file picker could not be opened. Please try again.");
      returnFocusAfterPick.current = true;
    } finally {
      setPicking(false);
    }
  }

  async function onRestore(event: FormEvent) {
    event.preventDefault();
    if (!packagePath || !password || (mode === "newNamed" && !name.trim()) || submitting.current) {
      return;
    }
    submitting.current = true;
    setBusy(true);
    setError(null);
    try {
      const failure = mode === "newNamed"
        ? await restoreAsNewVault(packagePath, password, name.trim())
        : await restoreVault(packagePath, password);
      if (failure) setError(describeIpcError(failure));
    } catch {
      setError("The restore service is unavailable. Please try again.");
    } finally {
      setPassword("");
      submitting.current = false;
      setBusy(false);
    }
    // On success the provider flips to Unlocked and the router swaps the screen.
  }

  function onCancel() {
    if (submitting.current) return;
    returnFocus.current = true;
    setPackagePath(null);
    setPassword("");
    setName("");
    setError(null);
  }

  return (
    <div className="@container flex flex-col gap-3 border-t pt-4">
      <p className="text-sm text-muted-foreground">{caption}</p>
      {mode === "newNamed" && (
        <p className="text-sm text-muted-foreground">
          The original vault and its files stay on this device. The backup opens as a separate vault.
        </p>
      )}
      {packagePath === null ? (
        <>
          <Button
            ref={pickerButton}
            variant="outline"
            className="h-auto min-h-10 max-w-full self-start whitespace-normal py-2 [overflow-wrap:anywhere] compact:@max-[15rem]:flex-col compact:@max-[15rem]:px-1"
            onClick={() => void onPick()}
            disabled={picking}
          >
            {picking ? <Spinner /> : <Upload aria-hidden />}
            {mode === "newNamed" ? "Restore as a new vault…" : "Restore from backup…"}
          </Button>
          {error && <p role="alert" className="text-sm text-loss">{error}</p>}
        </>
      ) : (
        <form onSubmit={(event) => void onRestore(event)} className="flex flex-col gap-2" aria-busy={busy}>
          {/* The chosen file wraps (a long path is never cut off); it is not truncated. */}
          <p className="text-xs text-muted-foreground [overflow-wrap:anywhere]">{packagePath}</p>
          {mode === "newNamed" && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor={`${id}-name`}>New vault name</Label>
              <Input
                id={`${id}-name`}
                autoFocus
                required
                value={name}
                onChange={(event) => setName(event.target.value)}
                disabled={busy}
                aria-invalid={name.length > 0 && name.trim().length === 0}
              />
            </div>
          )}
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={`${id}-password`}>Backup password</Label>
            <Input
              id={`${id}-password`}
              type="password"
              autoFocus={mode === "fresh"}
              autoComplete="off"
              required
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder="The password this backup was created with"
              disabled={busy}
            />
          </div>
          {error && (
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          )}
          {busy && <p role="status" className="text-sm text-muted-foreground">Restoring backup…</p>}
          <div className="flex gap-2">
            <Button
              type="button"
              variant="ghost"
              disabled={busy}
              onClick={onCancel}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={busy || password.length === 0 || (mode === "newNamed" && name.trim().length === 0)}
            >
              {busy && <Spinner />}
              {busy ? "Restoring…" : "Restore"}
            </Button>
          </div>
        </form>
      )}
    </div>
  );
}
