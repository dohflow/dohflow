import { ExternalLink, Heart } from "lucide-react";

import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { BrandLockup } from "@/brand/Brand";
import { DOHFLOW_LINKS, openExternal } from "@/lib/openExternal";
import { cn } from "@/lib/utils";

import type { BuildInfoDto } from "@/bindings";
import { channelLabel, useBuildInfo } from "./BuildBadge";

/// The bug-report link carries the running build's identity — version, channel,
/// commit — and NOTHING else. No vault data, no OS details (the app has no offline
/// source for the macOS version yet), so the URL is safe to paste anywhere.
function bugReportUrl(info: BuildInfoDto): string {
  const params = new URLSearchParams({
    version: info.version,
    channel: info.channel,
  });
  // Same rule as the identity line and the BuildBadge: a build without git
  // metadata has no commit, so the URL carries none — not the literal "unknown".
  if (info.commit && info.commit !== "unknown") params.set("commit", info.commit);
  return `${DOHFLOW_LINKS.contribute}?${params.toString()}`;
}

/// A link that leaves the app. Rendered as a `<button>` rather than an `<a href>` on
/// purpose: an anchor gives the WebView a navigation to perform if anything ever
/// fails to prevent the default (or on a modifier-click), whereas a button has no
/// path out except `openExternal`, which enforces the dohflow.app allow-list. The
/// destination shows in the tooltip so it is still discoverable.
function SiteLink({
  label,
  url,
  quiet = false,
}: {
  label: string;
  url: string;
  /// Muted instead of link-colored — for the Support row, which must never shout.
  quiet?: boolean;
}) {
  return (
    <button
      type="button"
      title={url}
      onClick={() =>
        // Nothing in the app can act on a browser-launch failure, but it must not
        // vanish as an unhandled rejection either — keep it diagnosable.
        void openExternal(url).catch((error: unknown) => {
          console.error(`Could not open ${url} in the system browser`, error);
        })
      }
      className={cn(
        "inline-flex items-center gap-1 rounded-sm underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background [&_svg]:size-3 [&_svg]:shrink-0",
        quiet ? "text-muted-foreground hover:text-foreground" : "text-primary",
      )}
    >
      {label}
      <ExternalLink aria-hidden />
    </button>
  );
}

/// The Settings "About" card (personal-cfo-n76x.18): the app's name, the running
/// build's identity (same source as the sidebar's BuildBadge), and the handful of
/// pages worth reaching from inside the app. Every link is a page on the DohFlow
/// site that the project controls — never a repo host or a payment processor —
/// and opens in the system browser through `openExternal`. The Support row is one
/// quiet line: no nag, no prompt, no check of whether anyone ever gave anything.
export function AboutCard() {
  const info = useBuildInfo();

  const commit = info && info.commit !== "unknown" ? info.commit : null;
  const identity = info
    ? [
        `Version ${info.version}`,
        `${channelLabel(info.channel)} build`,
        commit ? `commit ${commit}${info.dirty ? " (modified)" : ""}` : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : null;

  const links: { label: string; url: string }[] = [
    { label: "Website", url: DOHFLOW_LINKS.website },
    { label: "Help", url: DOHFLOW_LINKS.help },
    { label: "Release notes", url: DOHFLOW_LINKS.changelog },
    // Without build info the report still lands on the right page, just unstamped.
    { label: "Report a bug", url: info ? bugReportUrl(info) : DOHFLOW_LINKS.contribute },
    { label: "Security policy", url: DOHFLOW_LINKS.security },
    { label: "License (AGPL-3.0-only)", url: DOHFLOW_LINKS.license },
  ];

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base">About</CardTitle>
        <CardDescription>
          Local-first household finance. Your data stays in your vault.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <div className="flex flex-col gap-1">
          <BrandLockup variant="horizontal" height={22} />
          {identity && (
            <p className="text-xs text-muted-foreground">{identity}</p>
          )}
        </div>
        <ul aria-label="DohFlow links" className="flex flex-wrap gap-x-4 gap-y-1.5 text-sm">
          {links.map((link) => (
            <li key={link.label}>
              <SiteLink label={link.label} url={link.url} />
            </li>
          ))}
        </ul>
        <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Heart className="size-3.5 shrink-0" aria-hidden />
          <SiteLink label="Support DohFlow" url={DOHFLOW_LINKS.sponsor} quiet />
        </p>
      </CardContent>
    </Card>
  );
}
