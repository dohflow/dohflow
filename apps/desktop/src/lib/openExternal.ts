import { openUrl } from "@tauri-apps/plugin-opener";

/// The one origin the app may hand to the system browser (ADR 0010 addendum
/// 2026-09-06, personal-cfo-n76x.18). Every outbound link points at a page the
/// project controls: shipped binaries outlive URLs, so github.com, sponsor
/// platforms, and payment processors never enter the app — the site redirects
/// instead. The trailing slash is deliberate: it terminates the host, so
/// `https://dohflow.app.example.com/` does not match.
export const DOHFLOW_ORIGIN = "https://dohflow.app/";

/// The pages the app links to. Add here — never inline a URL at a call site.
export const DOHFLOW_LINKS = {
  website: DOHFLOW_ORIGIN,
  help: `${DOHFLOW_ORIGIN}help`,
  changelog: `${DOHFLOW_ORIGIN}changelog`,
  contribute: `${DOHFLOW_ORIGIN}contribute`,
  security: `${DOHFLOW_ORIGIN}security`,
  license: `${DOHFLOW_ORIGIN}license`,
  sponsor: `${DOHFLOW_ORIGIN}sponsor`,
} as const;

/// Whether `url` may leave the app. A plain prefix check on purpose: no URL
/// parsing whose normalization could disagree with the Rust side.
export function isAllowedExternalUrl(url: string): boolean {
  return url.startsWith(DOHFLOW_ORIGIN);
}

/// A migrate guide's URL, from a `SourcePresetDto.help_slug` (personal-cfo-gvidg).
/// The ONE place `help/migrate/<slug>` gets built — every other DOHFLOW_LINKS
/// entry is a fixed string; this is the sole dynamic one, kept here rather
/// than inlined at the "Import from <app>" picker's call site so a future
/// slug source still goes through `isAllowedExternalUrl`'s same-origin check.
export function migrateGuideUrl(helpSlug: string): string {
  return `${DOHFLOW_ORIGIN}help/migrate/${helpSlug}`;
}

/// Open `url` in the system browser. Refuses anything outside the DohFlow site
/// BEFORE the IPC call — defense in depth beside the Tauri capability scope
/// (`opener:allow-open-url` → `https://dohflow.app/*`), which rejects it again in
/// Rust. The refusal throws rather than silently no-oping so a wrong constant is
/// loud in tests and in development.
export async function openExternal(url: string): Promise<void> {
  if (!isAllowedExternalUrl(url)) {
    throw new Error(
      `Refused to open ${url}: only ${DOHFLOW_ORIGIN} links may leave the app.`,
    );
  }
  await openUrl(url);
}
