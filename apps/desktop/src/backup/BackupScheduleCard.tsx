import { useEffect, useState } from "react";
import { homeDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Loader2 } from "lucide-react";

import type { BackupCadenceDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { backupFolderHint } from "./backupFolderHint";
import {
  useBackupHistory,
  useBackupScheduleSettings,
  useConfigureBackup,
  useRunBackupNow,
} from "./useBackup";

type BackupDraft = {
  cadence: BackupCadenceDto;
  destination: string | null;
};

const DEFAULT_DRAFT: BackupDraft = {
  cadence: "weekly",
  destination: null,
};

function formatTimestamp(timestamp: string): string {
  const parsed = new Date(timestamp);
  return Number.isNaN(parsed.getTime())
    ? timestamp
    : new Intl.DateTimeFormat(undefined, {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(parsed);
}

export function BackupScheduleCard() {
  const settingsQuery = useBackupScheduleSettings();
  const historyQuery = useBackupHistory();
  const configureBackup = useConfigureBackup();
  const runBackupNow = useRunBackupNow();
  const [draft, setDraft] = useState<BackupDraft>(DEFAULT_DRAFT);
  const [dirty, setDirty] = useState(false);
  const [homeDirectory, setHomeDirectory] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    void homeDir()
      .then(setHomeDirectory)
      .catch(() => setHomeDirectory(null));
  }, []);

  useEffect(() => {
    const settings = settingsQuery.data;
    if (!settings || dirty) return;
    setDraft({
      cadence: settings.cadence,
      destination: settings.destination,
    });
  }, [settingsQuery.data, dirty]);

  const provider =
    draft.destination && homeDirectory
      ? backupFolderHint(draft.destination, homeDirectory)
      : null;
  const lastVerifiedBackup = historyQuery.data?.find((backup) => backup.verified);
  const draftMatchesSavedSettings =
    settingsQuery.data?.cadence === draft.cadence &&
    settingsQuery.data?.destination === draft.destination;
  const draftNeedsSave = settingsQuery.data ? !draftMatchesSavedSettings : dirty;

  function changeDraft(update: Partial<BackupDraft>) {
    setDraft((current) => ({ ...current, ...update }));
    setDirty(true);
    setError(null);
    setNotice(null);
  }

  async function chooseFolder() {
    setError(null);
    try {
      const selected = await open({
        title: "Choose a backup folder",
        directory: true,
        multiple: false,
      });
      if (typeof selected === "string") {
        changeDraft({ destination: selected });
      }
    } catch {
      setError("Could not open the folder picker.");
    }
  }

  async function saveSettings() {
    setError(null);
    setNotice(null);
    try {
      const saved = await configureBackup.mutateAsync({
        cadence: draft.cadence,
        destination: draft.destination,
      });
      setDraft({
        cadence: saved.cadence,
        destination: saved.destination,
      });
      setDirty(false);
      setNotice("Backup settings saved.");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not save backup settings.");
    }
  }

  async function backUpNow() {
    setError(null);
    setNotice(null);
    try {
      const backup = await runBackupNow.mutateAsync();
      setNotice(`Backup created and verified at ${backup.destination}.`);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not create a backup.");
    }
  }

  const loading = settingsQuery.isPending || historyQuery.isPending;
  const busy = configureBackup.isPending || runBackupNow.isPending;

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base">Backups</CardTitle>
        <CardDescription>
          Scheduled backups use the unlocked vault key without asking for your password. If a
          backup comes due while DohFlow is closed or locked, it runs the next time you unlock.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-5">
        {loading ? (
          <p className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
            <Loader2 className="size-4 animate-spin" aria-hidden />
            Loading backup settings…
          </p>
        ) : (
          <>
            <div className="flex flex-col gap-2">
              <Label htmlFor="backup-cadence">Schedule</Label>
              <select
                id="backup-cadence"
                className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
                value={draft.cadence}
                disabled={busy}
                onChange={(event) =>
                  changeDraft({ cadence: event.target.value as BackupCadenceDto })
                }
              >
                <option value="off">Off</option>
                <option value="daily">Daily</option>
                <option value="weekly">Weekly</option>
                <option value="monthly">Monthly</option>
              </select>
              <p className="text-xs text-muted-foreground">
                The default schedule is weekly. Automatic backups run only while this vault is
                unlocked.
              </p>
            </div>

            <div className="flex flex-col gap-2">
              <Label>Destination folder</Label>
              <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
                <Button
                  type="button"
                  variant="outline"
                  className="self-start"
                  disabled={busy}
                  onClick={() => void chooseFolder()}
                >
                  <FolderOpen aria-hidden />
                  Choose folder…
                </Button>
                <span className="break-all text-sm text-muted-foreground">
                  {draft.destination ?? "No folder selected"}
                </span>
              </div>
              <p className="text-xs text-muted-foreground">
                Choose a local folder or a folder managed by iCloud Drive, Dropbox, or another
                cloud-sync client.
              </p>
              {provider && (
                <p className="text-xs text-muted-foreground">
                  This folder is synced by {provider}; your encrypted backups will follow it.
                </p>
              )}
            </div>

            <p className="text-xs text-muted-foreground">
              DohFlow keeps all backups and does not delete older copies automatically. To remove
              old backups, use Finder or your file manager.
            </p>
            {draftNeedsSave && (
              <p id="backup-now-save-first" className="text-xs text-muted-foreground">
                Save backup settings before using Back up now.
              </p>
            )}

            <div className="flex flex-wrap gap-2">
              <Button disabled={busy || !dirty} onClick={() => void saveSettings()}>
                {configureBackup.isPending ? (
                  <Loader2 className="animate-spin" aria-hidden />
                ) : null}
                Save backup settings
              </Button>
              <Button
                variant="outline"
                disabled={busy || !settingsQuery.data?.destination || draftNeedsSave}
                aria-describedby={draftNeedsSave ? "backup-now-save-first" : undefined}
                onClick={() => void backUpNow()}
              >
                {runBackupNow.isPending ? (
                  <Loader2 className="animate-spin" aria-hidden />
                ) : null}
                Back up now
              </Button>
            </div>

            {lastVerifiedBackup ? (
              <p className="break-all text-sm text-muted-foreground">
                Last backup: {formatTimestamp(lastVerifiedBackup.created_at)} to{" "}
                {lastVerifiedBackup.destination} (verified)
              </p>
            ) : (
              <p className="text-sm text-muted-foreground">No verified backups yet.</p>
            )}
            {settingsQuery.data?.last_error && (
              <p role="alert" className="text-sm text-loss">
                Scheduled backup failed: {settingsQuery.data.last_error}
              </p>
            )}
            {settingsQuery.error && (
              <p role="alert" className="text-sm text-loss">
                {settingsQuery.error.message}
              </p>
            )}
            {historyQuery.error && (
              <p role="alert" className="text-sm text-loss">
                {historyQuery.error.message}
              </p>
            )}
          </>
        )}
        {error && (
          <p role="alert" className="text-sm text-loss">
            {error}
          </p>
        )}
        {notice && (
          <p role="status" className="break-all text-sm text-gain">
            {notice}
          </p>
        )}
      </CardContent>
    </Card>
  );
}
