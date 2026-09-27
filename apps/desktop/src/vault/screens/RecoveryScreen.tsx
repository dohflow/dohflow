import { useState } from "react";
import { AlertTriangle } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
} from "@/components/ui/card";
import { RestoreFromBackup } from "@/backup/RestoreFromBackup";
import { useVault } from "@/vault/useVault";
import { Screen } from "./Screen";

/// Shown when the vault on disk is half-open (DB without its envelope, or vice
/// versa) — the pre-unlock half of recovery (`5ivp`). The vault can't be opened to
/// repair in place, so we explain the state, offer a re-check, and offer restore
/// from an encrypted backup.
export function RecoveryScreen() {
  const { refresh, restoreRecoveryStatus } = useVault();
  const [checking, setChecking] = useState(false);

  async function onRecheck() {
    setChecking(true);
    await refresh();
    setChecking(false);
  }

  return (
    <Screen>
      <Card>
        <CardHeader className="items-center text-center">
          <div className="mb-2 flex size-12 items-center justify-center rounded-full bg-warning">
            <AlertTriangle className="size-6 text-background" aria-hidden />
          </div>
          <h1 className="text-xl font-semibold tracking-tight">
            Vault needs attention
          </h1>
          <CardDescription>
            Your vault files look incomplete or damaged, so DohFlow cannot open
            them safely. Keep the original files for diagnosis or recovery.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <Button variant="outline" onClick={onRecheck} disabled={checking}>
            {checking ? "Checking…" : "Re-check vault"}
          </Button>
          {restoreRecoveryStatus === "interrupted" && (
            <p role="alert" className="text-sm text-warning">
              An unregistered vault folder remains from an interrupted operation.
              Keep it for diagnosis; DohFlow has not added it to the vault list.
            </p>
          )}
          {restoreRecoveryStatus === "unavailable" && (
            <p role="alert" className="text-sm text-warning">
              DohFlow could not check for interrupted restores. Check file access before retrying.
            </p>
          )}
          <RestoreFromBackup
            mode="newNamed"
            caption="Restore an encrypted backup as a new named vault to recover access."
          />
        </CardContent>
      </Card>
    </Screen>
  );
}
