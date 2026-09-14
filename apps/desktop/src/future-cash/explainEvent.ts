import type { ForecastEventDto } from "@/bindings";
import { formatIsoDate, formatSignedMoney } from "@/lib/format";

/// One labelled fact in a forecast row's explanation. Plain strings only — the
/// explanation is rendered as typed React elements, never raw HTML or Markdown
/// (ADR 0003 trust boundary).
export type ExplanationFact = { label: string; value: string };

/// A typed, structured explanation of a single projected forecast row.
export type ForecastExplanation = {
  /// A one-line descriptive summary (ADR 0018: describes, never advises).
  summary: string;
  facts: ExplanationFact[];
};

const KIND_LABELS: Record<string, string> = {
  income: "Income",
  recurring_bill: "Recurring bill",
  loan_payment: "Loan payment",
  transfer: "Transfer",
  manual_entry: "Manual entry",
  starting_balance: "Opening balance",
};

const FREQUENCY_LABELS: Record<string, string> = {
  weekly: "Weekly",
  biweekly: "Every two weeks",
  semimonthly: "Twice a month",
  monthly: "Monthly",
  quarterly: "Quarterly",
  semiannual: "Twice a year",
  annual: "Annually",
};

function titleCase(token: string): string {
  const words = token.replace(/_/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function humanizeKind(kind: string): string {
  return KIND_LABELS[kind] ?? titleCase(kind);
}

function humanizeFrequency(frequency: string): string {
  return FREQUENCY_LABELS[frequency] ?? titleCase(frequency);
}

/// Describe the basis a row is assumed on (its `assumption_basis`).
function basisLabel(basis: ForecastEventDto["assumption_basis"]): string {
  if (basis.kind === "recurring_schedule") {
    return basis.frequency
      ? `Recurring schedule · ${humanizeFrequency(basis.frequency)}`
      : "Recurring schedule";
  }
  if (basis.kind === "manual_one_off") return "Manual one-off entry";
  return titleCase(basis.kind);
}

/// Compose a typed explanation for a projected forecast row from the event's
/// provenance — its source, type, amount, and assumption basis. On Layer-1 data
/// the `assumption_basis` is the explanation (ADR 0026 §6); contributing
/// assumption events and dependency edges surface here once they exist (manual
/// entries / scenarios / persisted runs).
export function explainEvent(
  event: ForecastEventDto,
  date: string,
): ForecastExplanation {
  let summary: string;
  if (event.assumption_basis.kind === "manual_one_off") {
    summary =
      event.amount.minor_units >= 0
        ? "A one-time entry you added."
        : "A one-time outflow you added.";
  } else {
    summary = `Projected from ${event.name || "an upcoming item"}'s recurring schedule.`;
  }

  return {
    summary,
    facts: [
      { label: "Type", value: humanizeKind(event.kind) },
      { label: "Basis", value: basisLabel(event.assumption_basis) },
      { label: "Amount", value: formatSignedMoney(event.amount) },
      { label: "Date", value: formatIsoDate(date) },
      { label: "Source", value: event.name || "—" },
    ],
  };
}
