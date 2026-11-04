/// Which scenarios the forecast actually includes, by the rule the backend applies
/// (`effective_scenario`, crates/db-worker/src/scenarios.rs; ADR 0051 §1/§3): an archived
/// scenario drops out, and so does one whose `expires_on` is before the household-local
/// date. A deleted one is not in the scenario list at all. Draft and active both apply.
///
/// `today` is the household-local calendar date (ADR 0021 §1) — `todayInTimezone` of
/// `useHouseholdTimezone().timezone` — never the device's own date, which can differ from
/// the household's by a day.
export function isExpired(
  scenario: { expires_on: string | null },
  today: string,
): boolean {
  return !!scenario.expires_on && scenario.expires_on < today;
}

export function isInForecast(
  scenario: { status: string; expires_on: string | null },
  today: string,
): boolean {
  return (
    (scenario.status === "draft" || scenario.status === "active") &&
    !isExpired(scenario, today)
  );
}
