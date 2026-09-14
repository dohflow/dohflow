import { useQuery } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { cn } from "@/lib/utils";

/// The running binary's build identity (personal-cfo-4d8.27.3.2).
///
/// Deliberately NOT part of the update check: that one runs a real `git fetch` and can
/// take seconds (or block on a captive portal), which would leave the badge blank at
/// exactly the moment the question is asked. `build_info` is compile-time constants, so
/// it resolves instantly and works offline. `staleTime: Infinity` because a running
/// binary's identity cannot change.
export function useBuildInfo() {
  const query = useQuery({
    queryKey: ["build-info"],
    queryFn: () => commands.buildInfo(),
    staleTime: Number.POSITIVE_INFINITY,
    refetchOnWindowFocus: false,
    retry: false,
  });
  return query.data ?? null;
}

/// The channel's display name. `channel` may be an explicit label (e.g. `beta`), so
/// don't call everything non-release "Development" — show the channel's own name.
/// Shared by the sidebar badge and the Settings About card (personal-cfo-n76x.18).
export function channelLabel(channel: string): string {
  if (channel === "release") return "Release";
  if (channel === "dev") return "Development";
  return channel;
}

/// A readable build stamp ("Jul 20, 14:32"), or null when unparseable.
export function buildStamp(rfc3339: string): string | null {
  const at = new Date(rfc3339);
  if (Number.isNaN(at.getTime())) return null;
  return at.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/// The always-visible build identity: which build of the app you are looking at.
///
/// The owner's complaint was concrete — after a rebuild there was no way to tell whether
/// the running app was the new one. Version + commit existed, but only inside Settings.
/// So this sits in the chrome: a **non-release** build gets a solid, high-contrast pill
/// (it must never be mistaken for the installed app — a tinted-text version failed WCAG
/// AA at 2.15:1, i.e. precisely the wrong outcome for the one element whose job is to be
/// unmistakable), a release build is a muted version string, and a build made from a
/// modified worktree is marked `*`.
export function BuildBadge({
  collapsed = false,
  floating = false,
}: {
  /// Icon-only sidebar: drop the text, keep the marker.
  collapsed?: boolean;
  /// Pin to a screen corner — used on the vault screens, which have no sidebar.
  floating?: boolean;
}) {
  const info = useBuildInfo();
  if (!info) return null;

  const isRelease = info.channel === "release";
  const commit = info.commit && info.commit !== "unknown" ? info.commit : null;
  const built = buildStamp(info.built_at);
  const label = channelLabel(info.channel);
  const detail = [
    `${label} build`,
    `v${info.version}`,
    commit ? `commit ${commit}${info.dirty ? " (modified)" : ""}` : null,
    built ? `built ${built}` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <div
      // One announcement carrying the whole identity, rather than a row of fragments.
      role="note"
      aria-label={detail}
      title={detail}
      className={cn(
        "flex items-center justify-center gap-1.5 px-1 pt-1",
        floating && "fixed bottom-3 right-3 z-30 pt-0",
      )}
    >
      {isRelease ? (
        <span className="truncate text-[10px] text-muted-foreground">
          v{info.version}
        </span>
      ) : (
        // Solid fill + near-black text: readable in both themes, and impossible to
        // mistake for the installed build. The ink is `--on-brand-fill`, which does not
        // flip with the theme — the fill does not either.
        <span
          className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide text-[color:var(--on-brand-fill)]"
          style={{ backgroundColor: "var(--terracotta)" }}
        >
          {collapsed ? "D" : label === "Development" ? "DEV" : info.channel}
        </span>
      )}
      {!collapsed && commit && (
        <span aria-hidden className="truncate font-mono text-[10px] text-muted-foreground">
          {commit}
          {info.dirty && "*"}
        </span>
      )}
    </div>
  );
}
