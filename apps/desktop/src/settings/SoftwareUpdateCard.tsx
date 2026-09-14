import { useState } from "react";
import { CircleCheck, CloudOff, Download, Loader2, RotateCw } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

import { hasUpdate, isDevUpdate, isReleaseUpdate, useSoftwareUpdate } from "./useSoftwareUpdate";

/// A readable build stamp ("Jul 20, 2026, 14:32"), or null when unparseable.
function buildStamp(rfc3339: string): string | null {
  const at = new Date(rfc3339);
  if (Number.isNaN(at.getTime())) return null;
  return at.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/// Whether the offline-looking error text names a network problem, so the offline state (a
/// design-brief-named state distinct from a generic error) shows its own quieter copy instead
/// of the alert-styled error box.
function looksOffline(message: string): boolean {
  return /network|offline|fetch|dns|connect|timed? ?out/i.test(message);
}

/// The Settings "Software update" surface (personal-cfo-1ik.3 / 1ik.4 / 867.1.2). Two channels,
/// one card: a `dev` build (`PCFO_BUILD_CHANNEL == dev`) rebuilds + reinstalls from the source
/// checkout, git-commit-based; a `release` build checks a signed GitHub Releases manifest and
/// downloads + verifies + installs the artifact, semver-based (ADR 0068). Both share the same
/// "Update & relaunch" action and error surface — only the wording and the availability signal
/// differ, via `useSoftwareUpdate`'s discriminated `status.kind`.
export function SoftwareUpdateCard() {
  const { status, checking, check, apply, relaunch, progress } =
    useSoftwareUpdate();
  const [applying, setApplying] = useState(false);
  const [applyError, setApplyError] = useState<string | null>(null);

  async function onUpdate() {
    setApplying(true);
    setApplyError(null);
    const result = await apply();
    if (result.ok) {
      // Relaunch quits this instance and reopens the new build — this never returns.
      await relaunch();
    } else {
      setApplying(false);
      setApplyError(result.message);
    }
  }

  const dev = isDevUpdate(status) ? status : null;
  const release = isReleaseUpdate(status) ? status : null;
  const offline = release && !release.checked && release.error && looksOffline(release.error);

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm">Software update</CardTitle>
        <CardDescription>
          {dev
            ? "Check whether a newer build is available in your source checkout."
            : "Check whether a newer version has been released."}
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="text-sm text-muted-foreground">
          Current version{" "}
          <span className="font-medium text-foreground">
            {dev ? dev.current_version : (release?.currentVersion ?? "—")}
          </span>
          {dev?.current_commit && dev.current_commit !== "unknown" && (
            <>
              {" "}
              (<span className="font-mono">{dev.current_commit}</span>
              {dev.build_dirty && (
                <span title="Built from a worktree with uncommitted changes">
                  , modified
                </span>
              )}
              )
            </>
          )}
          {/* Which build this is + when it was made (personal-cfo-4d8.27.3.3) — the
              detail behind the sidebar's badge. Release-channel status doesn't carry
              build_channel/build_time (it's not commit-shaped), so this line is dev-only;
              a release build's identity lives in BuildBadge elsewhere in Settings. */}
          {dev && (
            <div className="mt-1 text-xs">
              {dev.build_channel === "release"
                ? "Release"
                : dev.build_channel === "dev"
                  ? "Development"
                  : dev.build_channel}{" "}
              build
              {buildStamp(dev.build_time) && (
                <> · built {buildStamp(dev.build_time)}</>
              )}
            </div>
          )}
        </div>

        {offline && (
          <div className="flex items-center gap-2 rounded-md bg-muted/40 p-3 text-sm text-muted-foreground">
            <CloudOff className="size-4 shrink-0" aria-hidden />
            Couldn't reach the update server. Check your connection and try again.
          </div>
        )}

        {dev && !dev.checked && (
          <p className="rounded-md bg-muted/40 p-3 text-sm text-muted-foreground">
            {dev.error ?? "Couldn't check for updates."}
          </p>
        )}
        {release && !release.checked && !offline && (
          <p className="rounded-md bg-muted/40 p-3 text-sm text-muted-foreground">
            {release.error ?? "Couldn't check for updates."}
          </p>
        )}

        {dev?.checked && dev.up_to_date && (
          <div className="flex items-center gap-2 text-sm text-gain">
            <CircleCheck className="size-4 shrink-0" aria-hidden />
            You're on the latest version.
          </div>
        )}
        {release?.checked && !release.available && (
          <div className="flex items-center gap-2 text-sm text-gain">
            <CircleCheck className="size-4 shrink-0" aria-hidden />
            You're on the latest version.
          </div>
        )}

        {/* Checked, but the built commit isn't in the tree (e.g. built outside git) so the
            delta is unknown — say so rather than implying an update is available. */}
        {dev?.checked && !dev.up_to_date && dev.commits_behind === null && (
          <p className="rounded-md bg-muted/40 p-3 text-sm text-muted-foreground">
            Couldn't determine whether you're up to date — this build isn't tied to a tracked
            commit.
          </p>
        )}

        {hasUpdate(status) && (
          <div className="flex flex-col gap-2 rounded-md bg-primary/5 p-3">
            <div className="flex items-center gap-2 text-sm font-medium text-primary">
              <Download className="size-4 shrink-0" aria-hidden />
              A new version is available
              {dev && typeof dev.commits_behind === "number" && (
                <span className="font-normal text-muted-foreground">
                  — {dev.commits_behind}{" "}
                  {dev.commits_behind === 1 ? "commit" : "commits"} behind
                </span>
              )}
              {release?.latestVersion && (
                <span className="font-normal text-muted-foreground">
                  — v{release.latestVersion}
                </span>
              )}
            </div>
            {dev?.latest_date && (
              <p className="text-xs text-muted-foreground">
                Latest change: {dev.latest_date}
              </p>
            )}
            {release?.notes && (
              <p className="whitespace-pre-wrap text-xs text-muted-foreground">
                {release.notes}
              </p>
            )}
            <Button
              size="sm"
              className="self-start"
              onClick={() => void onUpdate()}
              disabled={applying}
            >
              {applying ? (
                <Loader2 className="animate-spin" aria-hidden />
              ) : (
                <Download aria-hidden />
              )}
              {applying
                ? progress?.phase === "installing"
                  ? "Installing…"
                  : progress?.phase === "downloading" &&
                      typeof progress.percent === "number"
                    ? `Downloading… ${progress.percent}%`
                    : progress?.phase === "downloading"
                      ? "Downloading…"
                      : "Updating…"
                : "Update & relaunch"}
            </Button>
            {applying && (
              <p className="text-xs text-muted-foreground">
                {dev
                  ? "This rebuilds the app from source — it can take a few minutes, then the app relaunches."
                  : "The update downloads, its signature is verified, then the app installs it and relaunches."}
              </p>
            )}
            {applyError && (
              <div
                role="alert"
                className="flex flex-col gap-1 rounded-md bg-warning/10 p-2 text-xs text-warning"
              >
                <p className="font-medium">The update didn't complete.</p>
                <pre className="whitespace-pre-wrap font-mono">{applyError}</pre>
                {dev && (
                  <p>
                    You can also run{" "}
                    <span className="font-mono">./scripts/update-app.sh</span> from the
                    project folder.
                  </p>
                )}
              </div>
            )}
          </div>
        )}

        <Button
          variant="outline"
          size="sm"
          className="self-start"
          onClick={() => void check()}
          disabled={checking || applying}
        >
          {checking ? (
            <Loader2 className="animate-spin" aria-hidden />
          ) : (
            <RotateCw aria-hidden />
          )}
          {checking ? "Checking…" : "Check for updates"}
        </Button>
      </CardContent>
    </Card>
  );
}
