import { describe, expect, it } from "vitest";

import type {
  AssumptionEventDto,
  CreateForecastAssumptionInput,
} from "@/bindings";

import { describeAssumption, previewChange, type Target } from "./describeChange";

const TARGETS: Target[] = [{ id: "rent", name: "Rent", type: "bill" }];
const CATEGORIES = new Map([["dining", "Dining"]]);

const stored = (kind: string, params: Record<string, unknown>, target: string) =>
  ({
    id: "e1",
    kind,
    target_entity_id: target,
    scenario_id: "s1",
    params_json: JSON.stringify(params),
    created_at: "2026-08-01",
    promoted_from_scenario_id: null,
  }) as unknown as AssumptionEventDto;

const input = (over: Partial<CreateForecastAssumptionInput>) =>
  ({
    kind: "bill_amount",
    scenario_id: "s1",
    target_entity_id: "rent",
    amount: null,
    date: null,
    label: null,
    new_amount_minor: null,
    new_anchor_date: null,
    effective_date: null,
    end_date: null,
    ...over,
  }) as CreateForecastAssumptionInput;

describe("the preview is the same sentence as the saved row (personal-cfo-c5en)", () => {
  it("agrees with the stored description for an amount change", () => {
    // The property that makes the preview trustworthy: what you are promised before saving
    // is literally what the list says afterwards. A second wording would be free to drift.
    const pending = previewChange(
      input({ new_amount_minor: 290_000, effective_date: "2027-01-01" }),
      TARGETS,
      "USD",
      CATEGORIES,
    );
    const after = describeAssumption(
      stored(
        "bill_amount",
        { new_amount_minor: 290_000, effective_date: "2027-01-01" },
        "rent",
      ),
      TARGETS,
      "USD",
      CATEGORIES,
    );
    expect(pending).toBe(after);
    expect(pending).toContain("Rent");
    expect(pending).toContain("$2,900.00");
  });

  it("agrees for a removal", () => {
    expect(
      previewChange(
        input({ kind: "exclusion", effective_date: "2027-02-01" }),
        TARGETS,
        "USD",
        CATEGORIES,
      ),
    ).toBe(
      describeAssumption(
        stored("exclusion", { effective_date: "2027-02-01" }, "rent"),
        TARGETS,
        "USD",
        CATEGORIES,
      ),
    );
  });

  it("renames new_amount_minor to the stored delta for a spend override", () => {
    // The one key that is NOT a straight rename: sent as `new_amount_minor`, stored as
    // `delta_minor_per_month`. Mapping it through unchanged previewed "$0.00/mo" for a
    // correctly-filled form — a confidently wrong sentence.
    const pending = previewChange(
      input({
        kind: "variable_spend_override",
        target_entity_id: "dining",
        new_amount_minor: -20_000,
        effective_date: "2027-03-01",
      }),
      TARGETS,
      "USD",
      CATEGORIES,
    );
    expect(pending).toContain("$200.00");
    expect(pending).toContain("less");
    expect(pending).toContain("Dining");
    expect(pending).not.toContain("$0.00");

    expect(pending).toBe(
      describeAssumption(
        stored(
          "variable_spend_override",
          { delta_minor_per_month: -20_000, effective_date: "2027-03-01" },
          "dining",
        ),
        TARGETS,
        "USD",
        CATEGORIES,
      ),
    );
  });
});
