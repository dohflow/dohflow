import { describe, expect, it } from "vitest";

import type { AssumptionEventDto } from "@/bindings";

import { detectConflicts } from "./scenarioConflicts";
import { topFirst } from "./scenarioPile";

const event = (over: Record<string, unknown> = {}) =>
  ({
    id: "e1",
    kind: "bill_amount",
    target_entity_id: "rent",
    scenario_id: "a",
    params_json: JSON.stringify({ new_amount_minor: 250_000 }),
    created_at: "2026-08-01",
    promoted_from_scenario_id: null,
    ...over,
  }) as unknown as AssumptionEventDto;

/// The shipped weakest-to-strongest order: base, then each stacked scenario, last wins.
const RANKED = [
  { scenarioId: null, events: [event({ id: "base1", scenario_id: null })] },
  { scenarioId: "a", events: [event({ id: "a1", scenario_id: "a" })] },
  {
    scenarioId: "b",
    events: [
      event({
        id: "b1",
        scenario_id: "b",
        params_json: JSON.stringify({ new_amount_minor: 300_000 }),
      }),
    ],
  },
];

describe("the pile is the precedence list reversed (personal-cfo-88o4)", () => {
  it("puts the WINNING scenario on top, agreeing with detectConflicts", () => {
    // The defect this exists to prevent: rendering the shipped order un-reversed would
    // confidently name the wrong scenario as winning. So the assertion is not "b is
    // first" — it is "the top sheet is the one the CONFLICT ENGINE calls the winner",
    // which stays true even if the precedence convention itself is ever changed.
    const layers = topFirst(RANKED);
    // Three sheets all change rent, so the pairs are (base,a), (base,b), (a,b).
    const conflicts = detectConflicts(RANKED);
    expect(conflicts).toHaveLength(3);
    // Nobody overrules the strongest sheet, so it never appears as a loser.
    const losers = new Set(conflicts.map((c) => c.loser.scenarioId));
    const winner = conflicts.find((c) => !losers.has(c.winner.scenarioId))?.winner;

    const top = layers[0];
    expect(top?.isTop).toBe(true);
    expect(top?.scenarioId).toBe(winner?.scenarioId);
  });

  it("puts the base forecast at the BOTTOM, where it loses to everything", () => {
    const layers = topFirst(RANKED);
    expect(layers[layers.length - 1]?.scenarioId).toBeNull();
    expect(layers[layers.length - 1]?.isTop).toBe(false);
  });

  it("marks exactly one sheet as the top", () => {
    expect(topFirst(RANKED).filter((l) => l.isTop)).toHaveLength(1);
  });

  it("strikes a losing change through inside the sheet that LOST it", () => {
    // Not just "who won" — the reader needs to see where their own scenario was overruled,
    // which means the mark lands on the loser's own row.
    const layers = topFirst(RANKED);
    const a = layers.find((l) => l.scenarioId === "a");
    expect(a?.changes[0]?.overruled).toBe(true);
    expect(a?.changes[0]?.overruledBy).toBe("b");

    const b = layers.find((l) => l.scenarioId === "b");
    expect(b?.changes[0]?.overruled).toBe(false);
    expect(b?.changes[0]?.overruledBy).toBeNull();
  });

  it("reordering flips which sheet is on top AND which change is struck", () => {
    // The mock's claim is that reordering makes the outcome legible — so swapping the two
    // scenarios must flip both the pile and the strike-through, not just the visual order.
    const swapped = [RANKED[0]!, RANKED[2]!, RANKED[1]!];
    const layers = topFirst(swapped);

    expect(layers[0]?.scenarioId).toBe("a");
    expect(layers.find((l) => l.scenarioId === "a")?.changes[0]?.overruled).toBe(false);
    expect(layers.find((l) => l.scenarioId === "b")?.changes[0]?.overruled).toBe(true);
    expect(layers.find((l) => l.scenarioId === "b")?.changes[0]?.overruledBy).toBe("a");
  });

  it("names the STRONGEST overruler when a change loses more than once", () => {
    // The base sheet's rent change loses to BOTH scenarios. Naming whichever pair came
    // last would credit a scenario that is itself overruled — the reader would be told
    // their change lost to something that is not in the forecast either.
    const layers = topFirst(RANKED);
    const base = layers.find((l) => l.scenarioId === null);
    expect(base?.changes[0]?.overruled).toBe(true);
    expect(base?.changes[0]?.overruledBy).toBe("b");
  });

  it("leaves non-conflicting changes unmarked", () => {
    const layers = topFirst([
      { scenarioId: "a", events: [event({ id: "a1", target_entity_id: "rent" })] },
      { scenarioId: "b", events: [event({ id: "b1", target_entity_id: "phone" })] },
    ]);
    expect(layers.every((l) => l.changes.every((c) => !c.overruled))).toBe(true);
  });
});
