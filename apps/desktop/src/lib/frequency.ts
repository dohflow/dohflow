/// Frequency tokens + labels shared by bills, recurring transfers, and candidates
/// (ADR 0048): the six classic cadences plus parameterized interval tokens
/// (`every_<n>_days|weeks|months`), mirrored from pay_schedule::Frequency.

export const CLASSIC_FREQUENCIES: { value: string; label: string }[] = [
  { value: "weekly", label: "Weekly" },
  { value: "biweekly", label: "Biweekly" },
  { value: "semi_monthly", label: "Semimonthly" },
  { value: "monthly", label: "Monthly" },
  { value: "quarterly", label: "Quarterly" },
  { value: "annual", label: "Annual" },
];

export type IntervalUnit = "days" | "weeks" | "months";

/// Bounds mirror `pay_schedule::Frequency::from_token` (ADR 0048 §3) so the form
/// rejects exactly what the backend would.
export const INTERVAL_UNITS: { value: IntervalUnit; label: string; max: number }[] = [
  { value: "days", label: "days", max: 366 },
  { value: "weeks", label: "weeks", max: 52 },
  { value: "months", label: "months", max: 36 },
];

const CLASSIC_LABELS = new Map(CLASSIC_FREQUENCIES.map((f) => [f.value, f.label]));

const INTERVAL_TOKEN = /^every_(\d+)_(days|weeks|months)$/;

/// Parse an interval token into its parts, or null for classic/malformed tokens.
export function parseIntervalToken(
  token: string,
): { n: number; unit: IntervalUnit } | null {
  const match = INTERVAL_TOKEN.exec(token);
  if (!match) return null;
  const n = Number(match[1]);
  const unit = match[2] as IntervalUnit;
  const bounds = INTERVAL_UNITS.find((u) => u.value === unit);
  if (!bounds || !Number.isInteger(n) || n < 1 || n > bounds.max) return null;
  return { n, unit };
}

/// Build the wire token for a custom interval (`every_6_weeks`).
export function buildIntervalToken(n: number, unit: IntervalUnit): string {
  return `every_${n}_${unit}`;
}

/// Whether a count is valid for a unit (1..=max, integer).
export function isValidIntervalCount(n: number, unit: IntervalUnit): boolean {
  const bounds = INTERVAL_UNITS.find((u) => u.value === unit);
  return bounds !== undefined && Number.isInteger(n) && n >= 1 && n <= bounds.max;
}

/// A display label for ANY frequency token — classic ("Monthly") or interval
/// ("Every 6 weeks", "Every day"). Unknown tokens fall through unchanged so a
/// newer vault never renders blank.
export function frequencyLabel(token: string): string {
  const classic = CLASSIC_LABELS.get(token);
  if (classic) return classic;
  const interval = parseIntervalToken(token);
  if (!interval) return token;
  const singular = interval.unit.slice(0, -1);
  return interval.n === 1
    ? `Every ${singular}`
    : `Every ${interval.n} ${interval.unit}`;
}
