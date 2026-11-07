import { useEffect, useRef, useState } from "react";
import {
  CircleCheck,
  CircleX,
  Loader2,
  RotateCw,
  ShieldAlert,
  ShieldCheck,
  Wrench,
} from "lucide-react";

import type { VaultHealthDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { describeIpcError } from "@/vault/useVault";
import { cn } from "@/lib/utils";
import { PersistentNotice } from "./PersistentSlot";
import { useAttemptCount, useReannounced } from "./attemptKey";
import { useKeepFocusAcrossMove } from "./useKeepFocusAcrossMove";
import { useVaultHealth } from "./useVaultHealth";

/// The displayed checks, in order. Each carries a plain-language `meaning` (shown
/// always, so the labels aren't bare jargon) and a `remedy` (shown when it fails,
/// so there are always instructions). `rebuildable` ones (read-model drift) don't
/// fail the overall verdict — a non-destructive rebuild fixes them, not a restore.
const CHECKS: {
  key: keyof VaultHealthDto;
  label: string;
  meaning: string;
  remedy: string;
  rebuildable?: boolean;
}[] = [
  {
    key: "integrity_ok",
    label: "Database integrity",
    meaning: "Your vault's database passed its built-in consistency check.",
    remedy:
      "The database file may be damaged. Lock the vault, then restore your most recent backup as a separate vault from the vault picker.",
  },
  {
    key: "writer_healthy",
    label: "Database writer",
    meaning: "The part of the app that saves your changes is working normally.",
    remedy:
      "Lock the vault and reopen it. If this keeps happening, restore from a backup.",
  },
  {
    key: "wal_configured",
    label: "Crash-safe journal",
    meaning: "The journal that protects in-progress writes is set up as expected.",
    remedy:
      "Reopen the vault to reapply the setting. If it persists, restore from a backup.",
  },
  {
    key: "schema_coherent",
    label: "Schema version",
    meaning: "The database layout matches this version of the app.",
    remedy:
      "This normally resolves when you reopen the vault — schema updates run automatically on open. If it persists after reopening, back up your vault and check for a DohFlow update.",
  },
  {
    key: "attachments_consistent",
    label: "Attachment files",
    meaning: "Every attachment's encrypted file is present on disk.",
    remedy: "An attachment file is missing. Restore from a backup to recover it.",
  },
  {
    key: "read_models_current",
    label: "Data views",
    meaning:
      "The app's cached views of your data. These are always rebuildable from your source records.",
    remedy:
      "Rebuild the views below — it restores them without changing any of your data.",
    rebuildable: true,
  },
];

/// The vault health-check + repair surface (personal-cfo-5ivp), shown in Settings.
/// Runs `vault_health` (n9w), lists each check, and offers the recoverable repair:
/// a non-destructive read-model rebuild for drift. Genuine corruption (integrity /
/// missing attachments) is steered to backup restore — a rebuild can't fix it.
export function VaultHealthCard() {
  const { health, error, refresh, checking, failures, rebuild, rebuilding } =
    useVaultHealth();
  const [repairError, setRepairError] = useState<string | null>(null);
  const [repaired, setRepaired] = useState(false);
  // Each settled attempt that fails clears and restores its alert text, so an identical repeat is
  // announced again. A Re-check pressed and settled with the read still failing counts as one (the
  // first load settling does not).
  const [repairAttempt, nextRepairAttempt] = useAttemptCount();
  const [readAttempt, nextReadAttempt] = useAttemptCount();
  const repairAlert = useReannounced(repairError, repairAttempt);
  const readAlert = useReannounced(error, readAttempt);
  const recheckPressed = useRef(false);
  const seenFailures = useRef(failures);
  useEffect(() => {
    if (failures === seenFailures.current) return;
    seenFailures.current = failures;
    if (recheckPressed.current) {
      recheckPressed.current = false;
      nextReadAttempt();
    }
  }, [failures, nextReadAttempt]);

  // A failed read puts Re-check in the section's persistent slot with the error; the data-views drift
  // puts Rebuild there with its message. Each button is rendered in one place only.
  const recheckInSlot = Boolean(error);
  const drifted = !error && health !== null && !health.read_models_current;
  const recheck = useKeepFocusAcrossMove({ settled: !checking });
  const rebuildFocus = useKeepFocusAcrossMove({
    // Settled once the rebuild has finished AND the health re-read it triggers has landed: until then
    // the old verdict is still on screen and Rebuild looks usable, but it is about to go.
    settled: !rebuilding && !checking,
    // A rebuild that worked removes its own button; Re-check is the next thing to press (when the
    // section is collapsed Re-check is in the hidden body, so the hook uses the section's heading).
    fallback: () => recheck.ref.current,
  });

  async function onRebuild() {
    rebuildFocus.remember();
    setRepaired(false);
    // An earlier repair error stays, with its Rebuild, until this try settles.
    const failure = await rebuild();
    if (failure) {
      setRepairError(describeIpcError(failure));
      nextRepairAttempt();
    } else {
      setRepairError(null);
      setRepaired(true);
    }
  }

  const recheckButton = (
    <Button
      ref={recheck.ref}
      variant="outline"
      size="sm"
      className="self-start"
      onClick={() => {
        recheck.remember();
        recheckPressed.current = true;
        refresh();
      }}
      disabled={checking}
    >
      {checking ? <Loader2 className="animate-spin" aria-hidden /> : <RotateCw aria-hidden />}
      Re-check
    </Button>
  );

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm">Vault health</CardTitle>
        <CardDescription>
          Check your vault for problems and repair the recoverable ones.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {error ? null : health === null ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" aria-hidden />
            Checking…
          </div>
        ) : (
          <>
            <div className="flex items-center gap-2">
              {health.is_healthy ? (
                <ShieldCheck className="size-5 shrink-0 text-gain" aria-hidden />
              ) : (
                <ShieldAlert className="size-5 shrink-0 text-warning" aria-hidden />
              )}
              <span
                className={cn(
                  "text-sm font-medium",
                  health.is_healthy ? "text-gain" : "text-warning",
                )}
              >
                {health.is_healthy
                  ? "Your vault is healthy"
                  : "Your vault needs attention"}
              </span>
            </div>

            <ul className="flex flex-col gap-1.5">
              {CHECKS.map((check) => {
                const ok = health[check.key];
                return (
                  <li
                    key={check.key}
                    className="flex items-start gap-2 text-sm"
                  >
                    {ok ? (
                      <CircleCheck
                        className="mt-0.5 size-4 shrink-0 text-gain"
                        aria-hidden
                      />
                    ) : (
                      <CircleX
                        className="mt-0.5 size-4 shrink-0 text-loss"
                        aria-hidden
                      />
                    )}
                    <div className="min-w-0">
                      <div className={cn(!ok && "font-medium")}>{check.label}</div>
                      <div className="text-xs text-muted-foreground">
                        {check.meaning}
                      </div>
                    </div>
                  </li>
                );
              })}
            </ul>

          </>
        )}

        {/* What is wrong and how to fix it stays in the section's persistent slot, open or closed:
            a failed read with its one Re-check; the data-views drift with its one Rebuild (and a
            failed repair); and every failing check's complete remedy. The six checks above stay in
            the body. */}
        <PersistentNotice>
          {error ? (
            <div className="flex flex-col gap-2">
              <p role="alert" className="text-sm text-loss">
                {readAlert}
              </p>
              {recheckButton}
            </div>
          ) : null}
          {drifted && (
            <div className="flex flex-col gap-2 rounded-md bg-muted/40 p-3">
              <p className="text-sm">
                Your data views drifted from the source records. Rebuilding
                restores them without changing your data.
              </p>
              <Button
                ref={rebuildFocus.ref}
                size="sm"
                className="self-start"
                onClick={onRebuild}
                disabled={rebuilding}
              >
                {rebuilding ? <Loader2 className="animate-spin" aria-hidden /> : <Wrench aria-hidden />}
                {rebuilding ? "Rebuilding…" : "Rebuild data views"}
              </Button>
              {repaired && <p className="text-xs text-gain">Rebuilt.</p>}
              {repairError && (
                <p role="alert" className="text-xs text-loss">
                  {repairAlert}
                </p>
              )}
            </div>
          )}
          {/* Every failing check (other than the rebuildable data views) gets plain-language
              instructions on how to fix it. */}
          {health !== null &&
            !error &&
            CHECKS.filter((c) => !c.rebuildable && !health[c.key]).map((c) => (
              <p key={c.key} className="rounded-md bg-warning/10 p-3 text-sm text-warning">
                {`${c.label}: ${c.remedy}`}
              </p>
            ))}
        </PersistentNotice>

        {!recheckInSlot && recheckButton}
      </CardContent>
    </Card>
  );
}
