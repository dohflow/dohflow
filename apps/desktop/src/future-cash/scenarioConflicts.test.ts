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

  describe("amount kinds overlap rather than replace (personal-cfo-5ie.15)", () => {
    const window = (minor: number, from?: string, to?: string) =>
      JSON.stringify({
        new_amount_minor: minor,
        ...(from ? { effective_date: from } : {}),
        ...(to ? { end_date: to } : {}),
      });
    const amounts = (existing: string, change: string, kind = "bill_amount") =>
      detectConflicts([
        {
          scenarioId: null,
          events: [event({ id: "base1", kind, scenario_id: null, params_json: existing })],
        },
        { scenarioId: "b", events: [event({ id: "b1", kind, params_json: change })] },
      ]);

    it("marks a date kind as a replacement with no overlap window", () => {
      const [conflict] = detectConflicts([
        {
          scenarioId: null,
          events: [
            event({
              id: "base1",
              kind: "bill_date",
              scenario_id: null,
              params_json: JSON.stringify({ new_anchor_date: "2027-01-05" }),
            }),
          ],
        },
        {
          scenarioId: "b",
          events: [
            event({
              id: "b1",
              kind: "bill_date",
              params_json: JSON.stringify({ new_anchor_date: "2027-01-12" }),
            }),
          ],
        },
      ]);
      expect(conflict?.change).toBe("replaces");
      expect(conflict?.overlap).toBeNull();
    });

    it("a window inside the existing one: the overlap is the scenario's, the existing continues", () => {
      const [conflict] = amounts(
        window(100_000, "2027-01-01", "2027-12-31"),
        window(120_000, "2027-03-01", "2027-06-30"),
      );
      expect(conflict?.change).toBe("overlaps");
      expect(conflict?.overlap).toEqual({
        from: "2027-03-01",
        to: "2027-06-30",
        existingContinuesOutside: true,
      });
    });

    it("a scenario window that covers the whole existing one leaves nothing outside", () => {
      const [conflict] = amounts(
        window(100_000, "2027-03-01", "2027-06-30"),
        window(120_000, "2027-01-01", "2027-12-31"),
      );
      expect(conflict?.overlap).toEqual({
        from: "2027-03-01",
        to: "2027-06-30",
        existingContinuesOutside: false,
      });
    });

    it("partial overlap at the start of the scenario window", () => {
      const [conflict] = amounts(
        window(100_000, "2027-01-01", "2027-06-30"),
        window(120_000, "2027-04-01", "2027-09-30"),
      );
      expect(conflict?.overlap).toEqual({
        from: "2027-04-01",
        to: "2027-06-30",
        existingContinuesOutside: true,
      });
    });

    it("partial overlap at the end of the scenario window", () => {
      const [conflict] = amounts(
        window(100_000, "2027-04-01", "2027-09-30"),
        window(120_000, "2027-01-01", "2027-06-30"),
      );
      expect(conflict?.overlap).toEqual({
        from: "2027-04-01",
        to: "2027-06-30",
        existingContinuesOutside: true,
      });
    });

    it("an open-ended existing amount continues outside a bounded scenario window", () => {
      const [conflict] = amounts(
        window(100_000),
        window(120_000, "2027-03-01", "2027-06-30"),
      );
      expect(conflict?.overlap).toEqual({
        from: "2027-03-01",
        to: "2027-06-30",
        existingContinuesOutside: true,
      });
    });

    it("an open-ended scenario amount takes over from its start onward", () => {
      const [conflict] = amounts(
        window(100_000, "2027-01-01", "2027-12-31"),
        window(120_000, "2027-03-01"),
      );
      expect(conflict?.overlap).toEqual({
        from: "2027-03-01",
        to: "2027-12-31",
        existingContinuesOutside: true,
      });
    });

    it("two open-ended amounts overlap on every date, with nothing outside", () => {
      const [conflict] = amounts(window(100_000), window(120_000));
      expect(conflict?.overlap).toEqual({
        from: null,
        to: null,
        existingContinuesOutside: false,
      });
    });

    it("an open-ended scenario over an open-ended existing amount from a date: the earlier dates stay", () => {
      const [conflict] = amounts(window(100_000), window(120_000, "2027-03-01"));
      expect(conflict?.overlap).toEqual({
        from: "2027-03-01",
        to: null,
        existingContinuesOutside: true,
      });
    });

    it("income amounts overlap the same way", () => {
      const [conflict] = amounts(
        window(300_000, "2027-01-01", "2027-12-31"),
        window(350_000, "2027-05-01", "2027-05-31"),
        "income_amount",
      );
      expect(conflict?.change).toBe("overlaps");
    });

    it("reports nothing when the windows do not overlap", () => {
      expect(
        amounts(
          window(100_000, "2027-01-01", "2027-02-28"),
          window(120_000, "2027-03-01", "2027-06-30"),
        ),
      ).toEqual([]);
    });
  });
});
