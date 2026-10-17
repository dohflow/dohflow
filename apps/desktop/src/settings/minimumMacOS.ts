/// The updater's minimum-macOS guard (personal-cfo-md8h2.4, ADR 0065 addendum 2026-10-17).
///
/// `latest.json` carries the release's `minimumSystemVersion` (written by
/// `scripts/write-latest-json.py` from the bundle's own floor). Before the release updater keeps
/// an update it could install, it compares that floor with the running macOS. An app macOS
/// refuses to open must never be downloaded over a working one.

/// Why an update is held back, or `null` when it may be offered.
export type MinimumMacOSBlock = {
  /// The macOS version the update needs, as the manifest states it.
  required: string;
  /// This Mac's macOS version, when known.
  current: string | null;
};

/// `"14"`, `"14.8"` or `"14.8.9"` as numbers; `null` for anything else.
export function parseMacOSVersion(value: unknown): number[] | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  if (!/^\d+(\.\d+){0,2}$/.test(trimmed)) return null;
  return trimmed.split(".").map(Number);
}

/// Negative when `a` is older than `b`, zero when equal, positive when newer. Missing parts
/// count as zero, so `"15"` equals `"15.0"`.
export function compareMacOSVersions(a: number[], b: number[]): number {
  for (let i = 0; i < Math.max(a.length, b.length); i += 1) {
    const diff = (a[i] ?? 0) - (b[i] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

/// Decide whether an update's manifest rules out this Mac.
///
/// - No `minimumSystemVersion` in the manifest: offered as before (older manifests).
/// - A floor that can't be read: held back. The manifest is our own release artifact, so an
///   unreadable floor means a broken release, and holding back is the safe failure.
/// - This Mac's version unknown: offered. The guard can't evaluate it, and the app is already
///   running on a supported macOS.
/// - Otherwise: held back exactly when the floor is newer than this Mac.
export function minimumMacOSBlock(
  rawJson: Record<string, unknown> | null | undefined,
  osVersion: string | null | undefined,
): MinimumMacOSBlock | null {
  const declared = rawJson?.minimumSystemVersion;
  if (declared === undefined || declared === null) return null;
  const required = parseMacOSVersion(declared);
  const requiredText = typeof declared === "string" ? declared.trim() : String(declared);
  if (!required) return { required: requiredText, current: osVersion ?? null };
  const current = parseMacOSVersion(osVersion);
  if (!current) return null;
  return compareMacOSVersions(required, current) > 0
    ? { required: requiredText, current: osVersion?.trim() ?? null }
    : null;
}
