import {
  compactMoney,
  dollarsToMinorUnits,
  monthsToDuration,
  todayInTimezone,
} from "./format";

describe("compactMoney", () => {
  it("compacts thousands and millions with the USD symbol", () => {
    expect(compactMoney(62_000_000, "USD")).toBe("$620k");
    expect(compactMoney(120_000_000, "USD")).toBe("$1.2M");
    expect(compactMoney(4_500, "USD")).toBe("$45");
  });

  it("renders non-USD currencies without a symbol and keeps the sign", () => {
    expect(compactMoney(500_000, "EUR")).toBe("5k");
    expect(compactMoney(-200_000, "USD")).toBe("-$2k");
  });
});

describe("monthsToDuration", () => {
  it("formats months as a coarse year/month duration", () => {
    expect(monthsToDuration(0)).toBe("Now");
    expect(monthsToDuration(7)).toBe("7 mo");
    expect(monthsToDuration(36)).toBe("3 yr");
    expect(monthsToDuration(31)).toBe("2 yr 7 mo");
  });

  it("floors fractional months (guards against fractional chart ticks)", () => {
    expect(monthsToDuration(2.25)).toBe("2 mo");
    expect(monthsToDuration(0.5)).toBe("Now");
  });
});

describe("todayInTimezone", () => {
  // personal-cfo-q329, ADR 0021 §1: an instant near a UTC day boundary must resolve to
  // the LOCAL calendar date of the given IANA zone, not UTC's — the exact bug shape
  // 5ie.11/ku2hn fixed on the backend, now given a frontend accessor.
  it("resolves the local date, not UTC's, on either side of a day boundary", () => {
    const at = new Date("2026-01-01T23:30:00Z");
    // 23:30 UTC on Jan 1 is already Jan 2 east of UTC (Kiritimati, UTC+14)…
    expect(todayInTimezone("Pacific/Kiritimati", at)).toBe("2026-01-02");
    // …but still Jan 1 west of UTC (Los Angeles, UTC-8 in January).
    expect(todayInTimezone("America/Los_Angeles", at)).toBe("2026-01-01");
  });

  it("agrees with UTC itself away from a boundary", () => {
    expect(todayInTimezone("UTC", new Date("2026-06-15T12:00:00Z"))).toBe(
      "2026-06-15",
    );
  });

  it("defaults `at` to the current instant", () => {
    expect(todayInTimezone("UTC")).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  // Review finding on PR #403: the stored zone is validated on write against
  // chrono-tz's database (write_household_tz) but resolved here against the WebView's
  // own ICU — the two can drift, and Intl.DateTimeFormat throws a RangeError on a zone
  // its ICU doesn't recognize. Called unconditionally on every render of
  // MarkObligationPaid/ScenariosView, an uncaught throw here would crash those views.
  it("falls back to UTC instead of throwing on a zone this ICU doesn't recognize", () => {
    expect(() =>
      todayInTimezone("Not/A_Real_Zone", new Date("2026-06-15T12:00:00Z")),
    ).not.toThrow();
    expect(
      todayInTimezone("Not/A_Real_Zone", new Date("2026-06-15T12:00:00Z")),
    ).toBe("2026-06-15");
  });
});

describe("dollarsToMinorUnits", () => {
  it("parses a plain major-unit amount", () => {
    expect(dollarsToMinorUnits("250")).toBe(25_000);
    expect(dollarsToMinorUnits("250.5")).toBe(25_050);
    expect(dollarsToMinorUnits("0")).toBe(0);
  });

  it("strips currency symbols, thousands separators, and whitespace (a0rv)", () => {
    // The dogfooding bug: pasting a formatted value used to fail outright.
    expect(dollarsToMinorUnits("$5,500.00")).toBe(550_000);
    expect(dollarsToMinorUnits("5 500")).toBe(550_000);
    expect(dollarsToMinorUnits("  1,200.50  ")).toBe(120_050);
    expect(dollarsToMinorUnits("USD 1,200")).toBe(120_000);
    expect(dollarsToMinorUnits("€90")).toBe(9_000);
  });

  it("preserves a leading sign", () => {
    expect(dollarsToMinorUnits("-$40.00")).toBe(-4_000);
    expect(dollarsToMinorUnits("+250")).toBe(25_000);
  });

  it("returns null when nothing numeric remains", () => {
    expect(dollarsToMinorUnits("")).toBeNull();
    expect(dollarsToMinorUnits("   ")).toBeNull();
    expect(dollarsToMinorUnits("abc")).toBeNull();
    expect(dollarsToMinorUnits("$")).toBeNull();
    expect(dollarsToMinorUnits(".")).toBeNull();
    expect(dollarsToMinorUnits("-")).toBeNull();
  });

  it("returns null for an ambiguous multi-decimal value rather than guessing", () => {
    // After stripping, "5.5.5" is not a finite number — better to reject than
    // silently coerce.
    expect(dollarsToMinorUnits("5.5.5")).toBeNull();
  });
});
