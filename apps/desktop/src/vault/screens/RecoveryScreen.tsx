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
  const { refresh } = useVault();
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
            Your vault files look incomplete — this can happen if setup was
            interrupted. Your data has not been lost.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <Button variant="outline" onClick={onRecheck} disabled={checking}>
            {checking ? "Checking…" : "Re-check vault"}
          </Button>
          <RestoreFromBackup caption="If your vault files are damaged, restore from an encrypted backup to recover your data." />
        </CardContent>
      </Card>
    </Screen>
  );
}
