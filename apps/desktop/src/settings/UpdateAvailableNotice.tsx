import { useState } from "react";
import { Download, X } from "lucide-react";

import { Button } from "@/components/ui/button";

import {
  hasUpdate,
  isDevUpdate,
  isReleaseUpdate,
  useSoftwareUpdate,
} from "./useSoftwareUpdate";

/// A dismissable bottom-right notice shown on launch when a newer build is available
/// (personal-cfo-1ik.3, personal-cfo-867.1.2). The shared `useSoftwareUpdate` check runs once
/// on mount; if an update is available this appears in the corner, for either channel. "Update"
/// routes to Settings (where the update lives); the X dismisses it for the session. Renders
/// nothing when up to date / unavailable.
export function UpdateAvailableNotice({
  onGoToSettings,
}: {
  onGoToSettings: () => void;
}) {
  const { status } = useSoftwareUpdate();
  const [dismissed, setDismissed] = useState(false);

  if (dismissed || !hasUpdate(status)) return null;

  const behind = isDevUpdate(status)
    ? typeof status.commits_behind === "number"
      ? `You're ${status.commits_behind} ${status.commits_behind === 1 ? "commit" : "commits"} behind.`
      : "A newer build is available in your source checkout."
    : isReleaseUpdate(status) && status.latestVersion
      ? `Version ${status.latestVersion} is available.`
      : "A newer version is available.";

  return (
    <div
      role="status"
      aria-label="Update available"
      className="fixed bottom-4 right-4 z-40 flex max-w-sm items-start gap-3 rounded-lg border border-primary/30 bg-background px-4 py-3 shadow-lg"
    >
      <Download className="mt-0.5 size-5 shrink-0 text-primary" aria-hidden />
      <div className="flex-1">
        <p className="text-sm font-medium">A new version is available</p>
        <p className="mt-0.5 text-sm text-muted-foreground">{behind}</p>
        <Button
          size="sm"
          className="mt-2"
          onClick={() => {
            setDismissed(true);
            onGoToSettings();
          }}
        >
          Update
        </Button>
      </div>
      <button
        type="button"
        onClick={() => setDismissed(true)}
        aria-label="Dismiss"
        className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
      >
        <X className="size-4" aria-hidden />
      </button>
    </div>
  );
}
