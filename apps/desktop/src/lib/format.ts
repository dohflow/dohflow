import type { MoneyDto } from "@/bindings";

/// Format a wire `MoneyDto` (integer minor units + ISO currency) as a localized
/// currency string, e.g. `{ minor_units: 25000, currency: "USD" }` → "$250.00".
export function formatMoney(money: MoneyDto): string {
  return new Intl.NumberFormat(undefined, {
    style: "currency",
    currency: money.currency,
  }).format(money.minor_units / 100);
}

/// Parse a user-entered major-unit amount (e.g. "250" or "250.5") into integer
/// minor units. Returns `null` if the input has no finite number after cleanup.
///
/// Real-world money gets pasted with currency symbols, thousands separators, and
/// stray whitespace — "$5,500.00", "5 500", "USD 1,200.50". Rather than reject
/// those, strip everything that isn't a digit, decimal point, or leading sign
/// before parsing (personal-cfo-a0rv). Commas and spaces are treated as thousands
/// separators (this app's amounts are decimal-point, matching `inputMode="decimal"`).
export function dollarsToMinorUnits(input: string): number | null {
  const sanitized = input
    .replace(/[\s,]/g, "") // whitespace + thousands separators
    .replace(/[^0-9.+-]/g, ""); // currency symbols, letters, stray characters
  // `Number("")` is 0, not NaN, so reject an empty result before parsing; the
  // remaining degenerate forms (".", "+", "-") fall out as NaN below.
  if (sanitized === "") return null;
  const amount = Number(sanitized);
  if (!Number.isFinite(amount)) return null;
  return Math.round(amount * 100);
}

/// Format an RFC 3339 timestamp as a short localized date, e.g. "Jun 5, 2026".
export function formatDate(rfc3339: string): string {
  const date = new Date(rfc3339);
  if (Number.isNaN(date.getTime())) return rfc3339;
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  }).format(date);
}

/// Today's calendar date, `YYYY-MM-DD`, in an arbitrary IANA zone — the household
/// timezone (ADR 0021 §1, personal-cfo-q329), when the caller has it, rather than the
/// browser's own local date. `Intl.DateTimeFormat`'s `timeZone` option does the
/// conversion; `formatToParts` (not the formatted string) keeps this locale-independent —
/// no assumption about month/day ordering in the output.
///
/// `timeZone` is validated on write against **chrono-tz**'s bundled database
/// (`write_household_tz`, `crates/db-worker/src/forecast/events.rs`), but resolved here
/// against the **WebView's own ICU** — the two can drift, so a name valid to one and
/// unknown to the other is possible (an OS-reported zone at vault creation, or one carried
/// in from a restored backup). `Intl.DateTimeFormat` throws a `RangeError` on a zone its
/// ICU doesn't recognize; this is called unconditionally on every render of
/// `MarkObligationPaid`/`ScenariosView`, so an uncaught throw here would crash those views
/// for the affected vault (a review finding on the personal-cfo-q329 pull request). Falls
/// back to UTC — the same fallback `useHouseholdTimezone`'s loading state already uses —
/// rather than propagate the crash; the Household Settings card stays the recovery path
/// regardless.
export function todayInTimezone(timeZone: string, at: Date = new Date()): string {
  let parts: Intl.DateTimeFormatPart[];
  try {
    parts = new Intl.DateTimeFormat("en-US", {
      timeZone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    }).formatToParts(at);
  } catch {
    return todayInTimezone("UTC", at);
  }
  const part = (type: string) => parts.find((p) => p.type === type)?.value ?? "";
  return `${part("year")}-${part("month")}-${part("day")}`;
}

/// Format a plain `YYYY-MM-DD` calendar date (no time, no zone) as a short
/// localized date, e.g. "Jun 5, 2026". Unlike `formatDate`, this parses the
/// components into a *local* `Date` so a bare date is never shifted to the
/// previous day in negative-UTC-offset timezones (where `new Date("2026-06-05")`
/// would land on Jun 4). A full timestamp is accepted too — its *date part* is
/// shown as written (the kernel stamps noon UTC precisely so the calendar date is
/// stable). Falls back to the raw string on anything else — never show a user a
/// raw RFC 3339 string (2pcx).
export function formatIsoDate(isoDate: string): string {
  const match = /^(\d{4})-(\d{2})-(\d{2})(?:[T ]|$)/.exec(isoDate.trim());
  if (!match) return isoDate;
  const [, year, month, day] = match;
  const date = new Date(Number(year), Number(month) - 1, Number(day));
  if (Number.isNaN(date.getTime())) return isoDate;
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  }).format(date);
}

/// Format a plain `YYYY-MM-DD` date as a compact axis tick, e.g. "Jul 5" — the
/// year is dropped (chart captions carry it). Local-parsed like `formatIsoDate`.
export function shortIsoDate(isoDate: string): string {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(isoDate.trim());
  if (!match) return isoDate;
  const [, year, month, day] = match;
  const date = new Date(Number(year), Number(month) - 1, Number(day));
  if (Number.isNaN(date.getTime())) return isoDate;
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
  }).format(date);
}

/// Format signed money with an explicit leading "+" for non-negative amounts
/// (`Intl` already renders the "-" for negatives), e.g. "+$150.00" / "-$40.00".
export function formatSignedMoney(money: MoneyDto): string {
  const formatted = formatMoney(money);
  return money.minor_units >= 0 ? `+${formatted}` : formatted;
}

/// The Tailwind text-color token for a signed amount: gain for money in, loss
/// for money out.
export function signedAmountClass(money: MoneyDto): string {
  return money.minor_units < 0 ? "text-loss" : "text-gain";
}

/// Compact money for chart axes: minor units → "$620k" / "$1.2M" (USD gets the
/// "$"; other currencies render bare). Used by the Future Cash + debt-burndown charts.
export function compactMoney(minorUnits: number, currency: string): string {
  const dollars = minorUnits / 100;
  const abs = Math.abs(dollars);
  const sym = currency === "USD" ? "$" : "";
  const sign = dollars < 0 ? "-" : "";
  if (abs >= 1_000_000) return `${sign}${sym}${(abs / 1_000_000).toFixed(1)}M`;
  if (abs >= 1_000) return `${sign}${sym}${Math.round(abs / 1_000)}k`;
  return `${sign}${sym}${Math.round(abs)}`;
}

/// A byte count as a human-readable size for attachment rows, e.g. 2048 → "2.0 KB".
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/// A month count as a coarse duration, e.g. 31 → "2 yr 7 mo", 36 → "3 yr", 0 → "Now".
/// Deterministic (no calendar clock) — used for debt-free timelines. Floors to whole months so a
/// fractional chart-axis tick never renders as "2.25 mo".
export function monthsToDuration(months: number): string {
  const whole = Math.floor(months);
  if (whole <= 0) return "Now";
  const years = Math.floor(whole / 12);
  const rem = whole % 12;
  if (years === 0) return `${rem} mo`;
  if (rem === 0) return `${years} yr`;
  return `${years} yr ${rem} mo`;
}

/// An RFC 3339 instant as "Jul 20, 14:32" (year included when it isn't the
/// current one). `null` when absent/unparseable — callers choose the fallback
/// copy (personal-cfo-ul5d).
export function formatDateTime(rfc3339: string | null): string | null {
  if (!rfc3339) return null;
  const date = new Date(rfc3339);
  if (Number.isNaN(date.getTime())) return null;
  const sameYear = date.getFullYear() === new Date().getFullYear();
  return new Intl.DateTimeFormat(undefined, {
    ...(sameYear ? {} : { year: "numeric" }),
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}
