import { explainEvent, type ForecastExplanation } from "./explainEvent";

import type { ForecastEventDto } from "@/bindings";

function event(over: Partial<ForecastEventDto> = {}): ForecastEventDto {
  return {
    source_event_id: "evt-1",
    name: "Rent",
    kind: "recurring_bill",
    amount: { minor_units: -180_000, currency: "USD" },
    assumption_basis: { kind: "recurring_schedule", frequency: "monthly" },
    ...over,
  };
}

function facts(explanation: ForecastExplanation) {
  return Object.fromEntries(explanation.facts.map((f) => [f.label, f.value]));
}

describe("explainEvent", () => {
  it("explains a recurring bill from its schedule", () => {
    const explanation = explainEvent(event(), "2026-07-01");
    expect(explanation.summary).toBe(
      "Projected from Rent's recurring schedule.",
    );
    const f = facts(explanation);
    expect(f.Type).toBe("Recurring bill");
    expect(f.Basis).toBe("Recurring schedule · Monthly");
    expect(f.Amount).toBe("-$1,800.00");
    expect(f.Date).toContain("2026");
    expect(f.Source).toBe("Rent");
  });

  it("explains recurring income with a signed inflow", () => {
    const explanation = explainEvent(
      event({
        name: "Acme Corp",
        kind: "income",
        amount: { minor_units: 500_000, currency: "USD" },
      }),
      "2026-06-15",
    );
    const f = facts(explanation);
    expect(f.Type).toBe("Income");
    expect(f.Amount).toBe("+$5,000.00");
    expect(f.Source).toBe("Acme Corp");
  });

  it("explains a manual one-off entry", () => {
    const explanation = explainEvent(
      event({
        name: "Bonus",
        kind: "manual_entry",
        amount: { minor_units: 500_000, currency: "USD" },
        assumption_basis: { kind: "manual_one_off", frequency: null },
      }),
      "2026-08-01",
    );
    expect(explanation.summary).toBe("A one-time entry you added.");
    expect(facts(explanation).Basis).toBe("Manual one-off entry");
  });
});
