import { useCallback, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch as relaunchProcess } from "@tauri-apps/plugin-process";

import { commands, type UpdateStatusDto } from "@/bindings";

/// A second, independent query cache entry for the live `Update` resource handle
/// (personal-cfo-sk4xr) — see the long comment on `useSoftwareUpdate` for why a
/// per-component `useRef` cannot hold this. Kept OUT of `SoftwareUpdateStatus`
/// itself so that stays a plain, JSON-shaped status DTO; this key holds the one
/// non-serializable piece, addressed through the SAME `QueryClient` every
/// observer shares. It rides along with `["software-update"]`'s own lifecycle
/// (cleared on vault lock, ADR 0003, since it is the same client) without any
/// separate bookkeeping. Exported so `useSoftwareUpdate.test.tsx` can drive the
/// "handle missing" fallback path directly (e.g. simulating eviction) without
/// hardcoding the key in two places.
export const UPDATE_HANDLE_QUERY_KEY = ["software-update", "handle"] as const;

/// The from-source dev updater's status, unchanged (personal-cfo-1ik.3) — commit-count-based,
/// only ever produced when `PCFO_BUILD_CHANNEL == dev`.
export type DevUpdateStatus = { kind: "dev" } & UpdateStatusDto;

/// The real signed-artifact updater's status (personal-cfo-867.1.2, ADR 0068) — semver-based,
/// against the GitHub Releases `latest.json`. `checked: false` covers both "the network is
/// unreachable" and "a bad/tampered manifest", per `error`.
export type ReleaseUpdateStatus = {
  kind: "release";
  currentVersion: string;
  checked: boolean;
  available: boolean;
  latestVersion: string | null;
  notes: string | null;
  date: string | null;
  error: string | null;
};

export type SoftwareUpdateStatus = DevUpdateStatus | ReleaseUpdateStatus;

/// Narrows `status.kind`, for callers that prefer a named check to an inline one.
///
/// These exist because of a narrowing quirk worth knowing about, described precisely so
/// nobody has to rediscover it: with `status` returned from `useSoftwareUpdate` as the bare
/// INFERRED type of `query.data ?? null`, an inline `status.kind === "dev"` check did not
/// narrow — `tsc` rejected the subsequent `status.current_version` as "not on
/// SoftwareUpdateStatus", even though the reference displays as `SoftwareUpdateStatus | null`
/// and `status.kind` displays as `"dev" | "release"`. The fix is the explicit
/// `SoftwareUpdateStatus | null` annotation on `status` where the hook returns it (below):
/// with it, inline checks narrow normally at every call site.
///
/// It is NOT a general "TypeScript can't narrow through a generic" defect — a minimal
/// reproduction with a stand-in `useQuery` narrows fine (checked independently in review).
/// It is specific to the type TanStack's `UseQueryResult["data"]` infers here, which is
/// assignable to the alias but not discriminable until re-annotated. Both the annotation and
/// these predicates are kept: the annotation is the actual fix, the predicates read better at
/// the call sites and are immune to the quirk regardless.
export function isDevUpdate(
  status: SoftwareUpdateStatus | null,
): status is DevUpdateStatus {
  return status?.kind === "dev";
}

export function isReleaseUpdate(
  status: SoftwareUpdateStatus | null,
): status is ReleaseUpdateStatus {
  return status?.kind === "release";
}

/// The install step's progress, release channel only — the dev updater's `apply()` is a single
/// blocking call with no intermediate state.
export type ApplyProgress =
  | { phase: "downloading"; percent: number | null }
  | { phase: "installing" };

export type ApplyOutcome = { ok: true } | { ok: false; message: string };

/// The in-app update check + install, covering BOTH channels (personal-cfo-1ik.3/1ik.4,
/// personal-cfo-867.1.2). Shared by the Settings card and the launch corner notice: one check
/// runs on first mount and is cached, and `check()` re-runs it (the "Check for updates" button).
///
/// Which path runs is decided by `commands.buildInfo()`'s `channel` — no new IPC command for
/// this; `dev` keeps today's `checkForUpdate`/`applyUpdate`/`relaunchApp` IPC path exactly as
/// it was, everything else calls the real Tauri updater plugin's JS API
/// (`@tauri-apps/plugin-updater`) directly. This matches how the app already calls first-party
/// Tauri plugin bindings directly elsewhere (`@tauri-apps/plugin-dialog` in the backup views,
/// `@tauri-apps/plugin-opener` in `openExternal.ts`) — ADR 0003's "IPC flows through generated
/// commands" governs our OWN custom commands, not first-party plugin bindings.
export function useSoftwareUpdate() {
  // The live `Update` resource from the most recent release-channel check (holds a backend
  // resource handle — `downloadAndInstall` must be called on this SAME object, not a fresh
  // `check()`, so install always applies exactly what was just verified/shown to the user).
  //
  // NOT a per-component `useRef`: `["software-update"]`'s queryFn is SHARED across every
  // caller of this hook (`SoftwareUpdateCard` in Settings, `UpdateAvailableNotice` as the
  // always-mounted Dashboard-level toast). TanStack Query dedupes concurrent/cached fetches
  // — only the ONE observer whose queryFn TanStack actually executes gets to run this
  // closure at all. A `useRef` scoped to that observer's own component instance would leave
  // every OTHER observer's `apply()` finding nothing, even though `query.data` (served from
  // the shared cache) correctly shows `available: true` — exactly `personal-cfo-sk4xr`'s
  // "Update & relaunch" reports 'No update is available' bug. Writing through `queryClient`
  // instead means whichever observer's closure runs, every observer reads the same value.
  const queryClient = useQueryClient();
  const [progress, setProgress] = useState<ApplyProgress | null>(null);

  const query = useQuery<SoftwareUpdateStatus>({
    queryKey: ["software-update"],
    queryFn: async () => {
      const info = await commands.buildInfo();
      if (info.channel === "dev") {
        const dto: UpdateStatusDto = await commands.checkForUpdate();
        return { kind: "dev", ...dto };
      }
      try {
        const update = await check();
        queryClient.setQueryData(UPDATE_HANDLE_QUERY_KEY, update);
        return {
          kind: "release",
          currentVersion: info.version,
          checked: true,
          available: update !== null,
          latestVersion: update?.version ?? null,
          notes: update?.body ?? null,
          date: update?.date ?? null,
          error: null,
        } satisfies ReleaseUpdateStatus;
      } catch (e) {
        queryClient.setQueryData(UPDATE_HANDLE_QUERY_KEY, null);
        return {
          kind: "release",
          currentVersion: info.version,
          checked: false,
          available: false,
          latestVersion: null,
          notes: null,
          date: null,
          error: e instanceof Error ? e.message : "Could not check for updates.",
        } satisfies ReleaseUpdateStatus;
      }
    },
    // A launch-time check is fine, but don't hammer the network/git on every refocus/remount.
    staleTime: 5 * 60 * 1000,
    refetchOnWindowFocus: false,
  });

  /// Rebuild + reinstall (dev channel, personal-cfo-1ik.4) or download + verify + install the
  /// signed artifact (release channel). Never throws — a failure comes back as `{ ok: false }`
  /// so the caller can show it inline rather than an unhandled rejection.
  const apply = useCallback(async (): Promise<ApplyOutcome> => {
    const status = query.data;
    if (status?.kind === "dev") {
      const result = await commands.applyUpdate();
      return result.ok ? { ok: true } : { ok: false, message: result.output_tail };
    }

    let update =
      queryClient.getQueryData<Update | null>(UPDATE_HANDLE_QUERY_KEY) ?? null;
    if (!update) {
      // The handle can be missing even though `status.available` is true — e.g. this
      // observer's own queryFn never ran (the dedup above) and the shared cache entry has
      // since been garbage-collected (no active observer of the SECOND key itself; the
      // default `gcTime` applies), or a vault lock cleared the whole client (ADR 0003). Fix
      // direction per the owner (personal-cfo-sk4xr, 2026-09-09): re-check rather than
      // refuse — the explicit click the user already made is what ADR 0068 §4 requires
      // before ANY download/install, and re-running `check()` here is still gated behind
      // that same click; it does not add a second, separate authorization to install.
      await query.refetch();
      update =
        queryClient.getQueryData<Update | null>(UPDATE_HANDLE_QUERY_KEY) ?? null;
    }
    if (!update) {
      return { ok: false, message: "No update is available to install." };
    }
    let total: number | null = null;
    let downloaded = 0;
    setProgress({ phase: "downloading", percent: null });
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? null;
          downloaded = 0;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          setProgress({
            phase: "downloading",
            percent:
              total !== null
                ? Math.min(100, Math.round((downloaded / total) * 100))
                : null,
          });
        } else {
          setProgress({ phase: "installing" });
        }
      });
      return { ok: true };
    } catch (e) {
      // A tampered artifact or a mismatched signature surfaces here as a rejected promise
      // (personal-cfo-miei) — the plugin verifies against tauri.conf.json's pubkey before
      // any bytes are trusted.
      return {
        ok: false,
        message:
          e instanceof Error
            ? e.message
            : "The update couldn't be installed — it may have failed signature verification.",
      };
    } finally {
      setProgress(null);
    }
  }, [query, queryClient]);

  /// Relaunch into the freshly-installed build. Quits this instance, so the promise never
  /// resolves — call it last, after a successful `apply()`.
  const relaunch = useCallback(async () => {
    if (query.data?.kind === "dev") {
      await commands.relaunchApp();
    } else {
      await relaunchProcess();
    }
  }, [query.data]);

  // The annotation is load-bearing, not decoration: returning the bare inferred type of
  // `query.data ?? null` leaves callers unable to narrow on `.kind` at all (see the note
  // on `isDevUpdate` above). Re-annotating to the alias restores discriminant narrowing.
  const status: SoftwareUpdateStatus | null = query.data ?? null;
  return {
    status,
    checking: query.isFetching,
    check: () => query.refetch(),
    apply,
    relaunch,
    progress,
  };
}

/// Whether a status represents a real, actionable update, for either channel.
///
/// Dev: the check ran and the built commit is a determinable, positive number of commits
/// behind. A `null` `commits_behind` means we *couldn't determine* the delta (e.g. a build made
/// outside the git tree) — that is not the same as "an update is available", so it is not
/// surfaced as one.
///
/// Release: the check ran and a newer version's manifest was returned.
export function hasUpdate(status: SoftwareUpdateStatus | null): boolean {
  if (!status) return false;
  if (status.kind === "dev") {
    return (
      status.checked &&
      !status.up_to_date &&
      typeof status.commits_behind === "number" &&
      status.commits_behind > 0
    );
  }
  return status.checked && status.available;
}
