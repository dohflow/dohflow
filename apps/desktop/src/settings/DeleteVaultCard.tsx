import { useState } from "react";
import { AlertTriangle, Loader2 } from "lucide-react";

import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { describeIpcError, useVault } from "@/vault/useVault";

/// The phrase the user must type to arm the permanent delete.
const CONFIRM_PHRASE = "delete my data";

/// The danger-zone card (personal-cfo-j0cg.5): permanently delete this vault — the whole instance,
/// all accounts/transactions/settings — recoverable only by restoring an encrypted backup. Gated
/// behind a typed confirmation. On success the vault status flips to NoVault and the app returns to
/// the create-vault screen (handled by the vault gate), so nothing else to route here.
export function DeleteVaultCard() {
  const { deleteVault } = useVault();
  const [open, setOpen] = useState(false);
  const [phrase, setPhrase] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const armed = phrase.trim().toLowerCase() === CONFIRM_PHRASE;

  function reset() {
    setOpen(false);
    setPhrase("");
    setError(null);
  }

  async function onDelete() {
    if (!armed) return;
    setBusy(true);
    setError(null);
    const failure = await deleteVault();
    setBusy(false);
    if (failure) setError(describeIpcError(failure));
    // On success the vault gate re-renders the create screen — this card unmounts.
  }

  return (
    <Card className="border-loss/40">
      <CardContent className="flex flex-col gap-3 p-4">
        <div className="flex items-center gap-2">
          <AlertTriangle className="size-4 text-loss" aria-hidden />
          <h3 className="font-semibold text-loss">Delete this vault</h3>
        </div>
        <p className="text-sm text-muted-foreground">
          Permanently delete this vault and everything in it — every account, transaction, and
          setting. This cannot be undone except by restoring an encrypted backup you made earlier.
        </p>

        {!open ? (
          <Button
            variant="outline"
            size="sm"
            className="self-start border-loss/50 text-loss hover:bg-loss/10 hover:text-loss"
            onClick={() => setOpen(true)}
          >
            Delete vault…
          </Button>
        ) : (
          <div className="flex flex-col gap-2 rounded-md border border-loss/40 bg-loss/5 p-3">
            <Label htmlFor="delete-vault-confirm" className="text-sm">
              Type <span className="font-mono font-medium">{CONFIRM_PHRASE}</span> to confirm
            </Label>
            <Input
              id="delete-vault-confirm"
              value={phrase}
              onChange={(e) => setPhrase(e.target.value)}
              placeholder={CONFIRM_PHRASE}
              autoFocus
            />
            {error && (
              <p role="alert" className="text-sm text-loss">
                {error}
              </p>
            )}
            <div className="flex gap-2">
              <Button
                variant="destructive"
                size="sm"
                disabled={!armed || busy}
                onClick={() => void onDelete()}
              >
                {busy && <Loader2 className="size-4 animate-spin" aria-hidden />}
                Delete permanently
              </Button>
              <Button variant="ghost" size="sm" disabled={busy} onClick={reset}>
                Cancel
              </Button>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
