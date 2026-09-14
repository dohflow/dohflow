import { detectConflicts } from "./scenarioConflicts";

const event = (over: Record<string, unknown> = {}) => ({
  id: "e1",
  kind: "bill_amount",
  target_entity_id: "rent",
  scenario_id: "a",
  params_json: JSON.stringify({ new_amount_minor: 250_000 }),
  created_at: "2026-08-01",
  promoted_from_scenario_id: null,
  ...over,
});

describe("detectConflicts (personal-cfo-4d8.27.6.5, ADR 0059 §3)", () => {
  it("flags two scenarios changing the same bill, and names the winner", () => {
    const conflicts = detectConflicts([
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
    ]);
    expect(conflicts).toHaveLength(1);
    // The LATER-stacked one wins, matching what the forecast actually composes.
    expect(conflicts[0]?.winner.eventId).toBe("b1");
    expect(conflicts[0]?.winner.amountMinor).toBe(300_000);
    expect(conflicts[0]?.loser.eventId).toBe("a1");
  });

  it("does not flag two scenarios changing different bills", () => {
    expect(
      detectConflicts([
        { scenarioId: "a", events: [event({ id: "a1", target_entity_id: "rent" })] },
        { scenarioId: "b", events: [event({ id: "b1", target_entity_id: "phone" })] },
      ]),
    ).toEqual([]);
  });

  it("does not flag windows that never overlap", () => {
    // Same bill, but one ends before the other starts — both apply, in sequence, and
    // calling that a contradiction would be noise.
    expect(
      detectConflicts([
        {
          scenarioId: "a",
          events: [
            event({
              id: "a1",
              params_json: JSON.stringify({
                new_amount_minor: 250_000,
                effective_date: "2026-01-01",
                end_date: "2026-06-30",
              }),
            }),
          ],
        },
        {
          scenarioId: "b",
          events: [
            event({
              id: "b1",
              params_json: JSON.stringify({
                new_amount_minor: 300_000,
                effective_date: "2026-07-01",
              }),
            }),
          ],
        },
      ]),
    ).toEqual([]);
  });

  it("treats an open end as unbounded", () => {
    // No effective_date and no end_date means "always", which overlaps anything.
    const conflicts = detectConflicts([
      { scenarioId: "a", events: [event({ id: "a1" })] },
      {
        scenarioId: "b",
        events: [
          event({
            id: "b1",
            params_json: JSON.stringify({
              new_amount_minor: 300_000,
              effective_date: "2030-01-01",
            }),
          }),
        ],
      },
    ]);
    expect(conflicts).toHaveLength(1);
  });

  it("does not flag two events from the same source", () => {
    // Within one scenario, creation order already settles it — that is not a conflict the
    // user has to reconcile, it is authorship.
    expect(
      detectConflicts([
        {
          scenarioId: "a",
          events: [event({ id: "a1" }), event({ id: "a2" })],
        },
      ]),
    ).toEqual([]);
  });

  it("does not flag one-off events that land on the same day", () => {
    // Two one-offs are two separate cash movements; summing them is correct, and flagging
    // them would be noise.
    expect(
      detectConflicts([
        {
          scenarioId: "a",
          events: [
            event({ id: "a1", kind: "one_time_event", target_entity_id: "acct" }),
          ],
        },
        {
          scenarioId: "b",
          events: [
            event({ id: "b1", kind: "one_time_event", target_entity_id: "acct" }),
          ],
        },
      ]),
    ).toEqual([]);
  });

  it("flags a scenario conflicting with an APPLIED base override", () => {
    // Base is rank 0 (ADR 0059 §2), so a selected scenario beats it — this is the case
    // that matters after ADR 0055, where applying promotes events into base.
    const conflicts = detectConflicts([
      { scenarioId: null, events: [event({ id: "base1", scenario_id: null })] },
      { scenarioId: "b", events: [event({ id: "b1", scenario_id: "b" })] },
    ]);
    expect(conflicts).toHaveLength(1);
    expect(conflicts[0]?.loser.scenarioId).toBeNull();
    expect(conflicts[0]?.winner.eventId).toBe("b1");
  });

  it("stays quiet on a malformed event rather than inventing a conflict", () => {
    expect(
      detectConflicts([
        { scenarioId: "a", events: [event({ id: "a1", params_json: "{not json" })] },
        { scenarioId: "b", events: [event({ id: "b1" })] },
      ]),
    ).toEqual([]);
  });
});
