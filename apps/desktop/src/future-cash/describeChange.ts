import type {
  AssumptionEventDto,
  CreateForecastAssumptionInput,
} from "@/bindings";
import { formatIsoDate, formatMoney, formatSignedMoney } from "@/lib/format";

/// A base income/bill a change can target.
export type Target = { id: string; name: string; type: "bill" | "income" };

/// A one-line, human description of a stored scenario event (parsing its
/// `params_json`), for the list.
export function describeAssumption(
  event: AssumptionEventDto,
  targets: Target[],
  currency: string,
  /// Category names by id — a spend override targets a CATEGORY, which is never in
  /// `targets` (bills + income), so without this every row read "on an item".
  categoryNames: Map<string, string>,
): string {
  const targetName = event.target_entity_id
    ? (targets.find((t) => t.id === event.target_entity_id)?.name ??
       categoryNames.get(event.target_entity_id) ??
       "an item")
    : null;
  let params: Record<string, unknown> = {};
  try {
    params = JSON.parse(event.params_json) as Record<string, unknown>;
  } catch {
    // A malformed params blob falls back to just the kind below.
  }
  const minor = (key: string): number =>
    typeof params[key] === "number" ? (params[key] as number) : 0;
  const date = (key: string): string | null =>
    typeof params[key] === "string" ? (params[key] as string) : null;
  const from = (key: string): string => {
    const value = date(key);
    return value ? ` from ${formatIsoDate(value)}` : "";
  };

  switch (event.kind) {
    case "one_time_event": {
      const label = (params.label as string) || "One-off";
      const cur = (params.currency as string) || currency;
      const on = date("date");
      return `${label}: ${formatSignedMoney({ minor_units: minor("amount_minor"), currency: cur })}${on ? ` on ${formatIsoDate(on)}` : ""}`;
    }
    case "income_amount":
    case "bill_amount": {
      const start = date("effective_date");
      const stop = date("end_date");
      const window =
        start && stop
          ? ` from ${formatIsoDate(start)} to ${formatIsoDate(stop)}`
          : start
            ? ` from ${formatIsoDate(start)}`
            : stop
              ? ` until ${formatIsoDate(stop)}`
              : "";
      return `Change ${targetName ?? "an item"} to ${formatMoney({ minor_units: minor("new_amount_minor"), currency })}${window}`;
    }
    case "income_date":
    case "bill_date": {
      const anchor = date("new_anchor_date");
      return `Move ${targetName ?? "an item"} to start ${anchor ? formatIsoDate(anchor) : "—"}`;
    }
    case "exclusion":
      return `Remove ${targetName ?? "an item"}${from("effective_date")}`;
    case "variable_spend_override": {
      // A planned change to a category's discretionary spend (4d8.27.6.2). The stored
      // delta is signed: negative = spend less.
      const delta = minor("delta_minor_per_month");
      const verb = delta < 0 ? "Spend" : "Spend an extra";
      const rest = delta < 0 ? " less" : "";
      const start = date("effective_date");
      const stop = date("end_date");
      const window =
        start && stop
          ? ` from ${formatIsoDate(start)} to ${formatIsoDate(stop)}`
          : start
            ? ` from ${formatIsoDate(start)}`
            : stop
              ? ` until ${formatIsoDate(stop)}`
              : "";
      return `${verb} ${formatMoney({ minor_units: Math.abs(delta), currency })}/mo${rest} on ${targetName ?? "a category"}${window}`;
    }
    case "recurring_debt_payment": {
      // The payoff-scenario overlay (useCreatePayoffScenario): an extra monthly debt payment.
      const label = (params.label as string) || "Extra debt payment";
      const cur = (params.currency as string) || currency;
      const stop = date("end_date");
      const until = stop ? ` until ${formatIsoDate(stop)}` : "";
      return `${label}: ${formatMoney({ minor_units: minor("amount_minor"), currency: cur })}/mo${from("anchor_date")}${until}`;
    }
    default:
      return event.kind;
  }
}


/// The same sentence, for a change the user has NOT saved yet (personal-cfo-c5en).
///
/// A form full of individually correct fields can still add up to something the user did
/// not intend — an assumption event is abstract enough that "bill_amount, Rent, 290000,
/// 2027-01-01" does not read as "Change Rent to $2,900.00 from 1 Jan 2027" until you say
/// so. Hence the restatement before save.
///
/// It goes through `describeAssumption` rather than wording it separately, so the preview
/// and the row the user sees afterwards are the SAME SENTENCE. A second implementation
/// would be free to drift, and a preview that drifts is worse than none: it promises
/// something the save does not deliver. `describeChange.test.ts` asserts the two agree.
export function previewChange(
  input: CreateForecastAssumptionInput,
  targets: Target[],
  currency: string,
  categoryNames: Map<string, string>,
): string {
  // The backend packs these flat fields into `params_json`. Mostly under the SAME key —
  // but not always, and the exception is load-bearing: a variable_spend_override is SENT
  // as `new_amount_minor` (signed) and STORED as `delta_minor_per_month`. Mapping it
  // straight through would preview "Spend an extra $0.00/mo" for a real, correctly-filled
  // form, which is worse than showing nothing. `describeChange.test.ts` pins it.
  //
  // `recurring_debt_payment` has no case here: it is authored by the payoff flow, not this
  // form, and the input type carries no fields for it. It still DESCRIBES correctly once
  // stored — only its pending preview is out of scope.
  const params: Record<string, unknown> = {
    amount_minor: input.amount?.minor_units,
    currency: input.amount?.currency,
    date: input.date,
    label: input.label,
    new_amount_minor: input.new_amount_minor,
    ...(input.kind === "variable_spend_override"
      ? { delta_minor_per_month: input.new_amount_minor }
      : {}),
    new_anchor_date: input.new_anchor_date,
    effective_date: input.effective_date,
    end_date: input.end_date,
  };
  for (const key of Object.keys(params)) {
    if (params[key] === null || params[key] === undefined) delete params[key];
  }
  return describeAssumption(
    {
      id: "pending",
      kind: input.kind,
      target_entity_id: input.target_entity_id,
      scenario_id: input.scenario_id,
      params_json: JSON.stringify(params),
      created_at: "",
      promoted_from_scenario_id: null,
    } as unknown as AssumptionEventDto,
    targets,
    currency,
    categoryNames,
  );
}
