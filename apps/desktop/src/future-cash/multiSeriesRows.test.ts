import type { AccountHistoryDto, GroupSeriesDto } from "@/bindings";

import { assembleMultiSeriesRows, historyTiersWithData } from "./multiSeriesRows";

const usd = (m: number) => ({ minor_units: m, currency: "USD" });
const bandDto = (p10: number, p50: number, p90: number) => ({
  p10: usd(p10),
  p50: usd(p50),
  p90: usd(p90),
});

const groups: GroupSeriesDto[] = [
  {
    tier: "net",
    closings: [
      { date: "2026-07-14", closing: bandDto(90_000, 90_000, 90_000) },
      { date: "2026-07-15", closing: bandDto(80_000, 90_000, 100_000) },
    ],
  },
  {
    tier: "spendable",
    closings: [
      { date: "2026-07-14", closing: bandDto(60_000, 60_000, 60_000) },
      { date: "2026-07-15", closing: bandDto(55_000, 60_000, 65_000) },
    ],
  },
  {
    tier: "reserve",
    closings: [
      { date: "2026-07-14", closing: bandDto(30_000, 30_000, 30_000) },
      { date: "2026-07-15", closing: bandDto(30_000, 30_000, 30_000) },
    ],
  },
];

function historyAccount(
  id: string,
  tier: string,
  days: Array<[string, number]>,
): AccountHistoryDto {
  return {
    account_id: id,
    name: id,
    subtype: null,
    tier,
    days: days.map(([date, minor]) => ({ date, closing: usd(minor) })),
  };
}

describe("assembleMultiSeriesRows", () => {
  it("rolls history up by tier, treats a not-yet-existing account as 0, and excludes cards", () => {
    const history = [
      historyAccount("checking", "spendable", [
        ["2026-07-12", 50_000],
        ["2026-07-13", 55_000],
        ["2026-07-14", 60_000],
      ]),
      // Savings only exists from the 13th — on the 12th it contributes nothing.
      historyAccount("savings", "reserve", [
        ["2026-07-13", 30_000],
        ["2026-07-14", 30_000],
      ]),
      // A card's owed history must NOT leak into the liquid chart.
      historyAccount("card", "card", [["2026-07-12", -40_000]]),
    ];
    const { rows, historyDayCount } = assembleMultiSeriesRows({
      groups,
      tiers: ["spendable", "reserve", "net"],
      plottedAccounts: [],
      history,
      todayIso: "2026-07-14",
    });
    const at = (d: string) => rows.find((r) => r.date === d)!;
    expect(at("2026-07-12").hist_net).toBe(50_000); // savings absent → 0, card excluded
    expect(at("2026-07-13").hist_net).toBe(85_000);
    expect(at("2026-07-13").hist_reserve).toBe(30_000);
    expect(historyDayCount).toBe(2); // the 12th + 13th (today is the boundary)
    // The today row carries BOTH halves so the lines meet.
    const today = at("2026-07-14");
    expect(today.hist_net).toBe(90_000);
    expect(today.net).toBe(90_000);
    // The wash is continuous: realized before today, projected after.
    expect(at("2026-07-13").spendableWash).toBe(55_000);
    expect(at("2026-07-15").spendableWash).toBe(60_000);
  });

  it("emits per-tier forward bands and flags only tiers with real width", () => {
    const { rows, bandedTiers } = assembleMultiSeriesRows({
      groups,
      tiers: ["spendable", "reserve", "net"],
      plottedAccounts: [],
      history: null,
      todayIso: "2026-07-14",
    });
    const tomorrow = rows.find((r) => r.date === "2026-07-15")!;
    expect(tomorrow.band_net).toEqual([80_000, 100_000]);
    expect(tomorrow.band_spendable).toEqual([55_000, 65_000]);
    // Reserve's cone is collapsed everywhere → no band Area for it.
    expect(bandedTiers.sort()).toEqual(["net", "spendable"]);
  });

  it("maps per-account history onto plotted account keys and ignores future-dated rows", () => {
    const history = [
      historyAccount("acct-1", "spendable", [
        ["2026-07-13", 10_000],
        ["2026-07-14", 12_000],
        ["2026-07-15", 99_999], // beyond today — never a realized point
      ]),
    ];
    const { rows } = assembleMultiSeriesRows({
      groups,
      tiers: ["net"],
      plottedAccounts: [
        { key: "acct_acct1", account_id: "acct-1", days: [] },
      ],
      history,
      todayIso: "2026-07-14",
    });
    expect(rows.find((r) => r.date === "2026-07-13")!.hist_acct_acct1).toBe(10_000);
    expect(
      rows.find((r) => r.date === "2026-07-15")!.hist_acct_acct1,
    ).toBeUndefined();
  });

  it("grounds the zero line when only a band's low edge dips negative", () => {
    const dipping: GroupSeriesDto[] = [
      {
        tier: "net",
        closings: [
          { date: "2026-07-14", closing: bandDto(50_000, 50_000, 50_000) },
          { date: "2026-07-15", closing: bandDto(-30_000, 50_000, 130_000) },
        ],
      },
    ];
    const { anyNegative } = assembleMultiSeriesRows({
      groups: dipping,
      tiers: ["net"],
      plottedAccounts: [],
      history: null,
      todayIso: "2026-07-14",
    });
    expect(anyNegative).toBe(true);
  });

  it("never fabricates realized history for the synthetic Unallocated bucket", () => {
    const history = [historyAccount("checking", "spendable", [["2026-07-13", 10_000]])];
    const { rows } = assembleMultiSeriesRows({
      groups,
      tiers: ["net", "unallocated"],
      plottedAccounts: [],
      history,
      todayIso: "2026-07-14",
    });
    expect(rows.find((r) => r.date === "2026-07-13")!.hist_unallocated).toBeUndefined();
  });
});

describe("historyTiersWithData", () => {
  it("keeps a tier with nonzero realized history and always excludes cards", () => {
    const tiers = historyTiersWithData([
      historyAccount("savings", "reserve", [["2026-07-13", 30_000]]),
      historyAccount("card", "card", [["2026-07-13", -40_000]]),
    ]);
    expect(tiers.has("reserve")).toBe(true);
    expect(tiers.has("net")).toBe(true);
    expect(tiers.has("spendable")).toBe(false);
  });
});

describe("the band carries the realized/projected boundary (personal-cfo-7c7a)", () => {
  // History ending on the seam day, so today has both a realized close and a projection.
  const history = [
    historyAccount("checking", "spendable", [
      ["2026-07-12", 50_000],
      ["2026-07-13", 55_000],
      ["2026-07-14", 60_000],
    ]),
    historyAccount("savings", "reserve", [
      ["2026-07-12", 30_000],
      ["2026-07-13", 30_000],
      ["2026-07-14", 30_000],
    ]),
  ];

  const assemble = () =>
    assembleMultiSeriesRows({
      groups,
      tiers: ["net", "spendable", "reserve"],
      plottedAccounts: [],
      history,
      todayIso: "2026-07-14",
    });

  it("draws NO band over realized days", () => {
    // The absence of the whole uncertainty apparatus is itself the claim that nothing on
    // the left is estimated. A band there would say the past is in doubt.
    const { rows } = assemble();
    const past = rows.filter((r) => r.date < "2026-07-14");
    expect(past.length).toBeGreaterThan(0);
    for (const row of past) {
      expect(row["band_net"]).toBeUndefined();
      expect(row["band_spendable"]).toBeUndefined();
    }
  });

  it("opens the band from ZERO WIDTH at the seam", () => {
    // Today's balance is the one number the projection is anchored to. A band already wide
    // there would say the anchor itself is in doubt.
    const { rows } = assemble();
    const seam = rows.find((r) => r.date === "2026-07-14");
    expect(seam).toBeDefined();
    const band = seam?.["band_net"] as [number, number];
    expect(band[0]).toBe(band[1]);
    // …and it opens from the REALIZED close, not from the projection's own p50.
    expect(band[0]).toBe(90_000);
  });

  it("lets the band open on the day after the seam", () => {
    // Zero-width at the anchor must not flatten the whole cone — the uncertainty is about
    // what happens AFTER the known balance.
    const { rows } = assemble();
    const next = rows.find((r) => r.date === "2026-07-15");
    const band = next?.["band_net"] as [number, number];
    expect(band[0]).toBe(80_000);
    expect(band[1]).toBe(100_000);
    expect(band[0]).toBeLessThan(band[1]);
  });

  it("leaves the band alone when there is no realized history to anchor to", () => {
    // With no past there is no known balance at the seam, so nothing is collapsed — the
    // projection stands on its own from day one.
    //
    // The day-0 band here is deliberately WIDE. The shared fixture's is already
    // zero-width, so asserting against it could not tell "left alone" from "collapsed" —
    // the assertion would pass either way and prove nothing.
    const { rows } = assembleMultiSeriesRows({
      groups: [
        {
          tier: "net",
          closings: [
            { date: "2026-07-14", closing: bandDto(70_000, 90_000, 110_000) },
          ],
        },
      ],
      tiers: ["net"],
      plottedAccounts: [],
      history: [],
      todayIso: "2026-07-14",
    });
    const seam = rows.find((r) => r.date === "2026-07-14");
    expect(seam?.["band_net"]).toEqual([70_000, 110_000]);
  });
});
