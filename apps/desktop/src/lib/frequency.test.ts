import { describe, expect, it } from "vitest";

import {
  buildIntervalToken,
  frequencyLabel,
  isValidIntervalCount,
  parseIntervalToken,
} from "./frequency";

describe("frequency tokens (ADR 0048)", () => {
  it("labels classic tokens with their curated names", () => {
    expect(frequencyLabel("monthly")).toBe("Monthly");
    expect(frequencyLabel("semi_monthly")).toBe("Semimonthly");
  });

  it("labels interval tokens in plain language", () => {
    expect(frequencyLabel("every_6_weeks")).toBe("Every 6 weeks");
    expect(frequencyLabel("every_45_days")).toBe("Every 45 days");
    expect(frequencyLabel("every_1_months")).toBe("Every month");
  });

  it("falls through unknown tokens unchanged", () => {
    expect(frequencyLabel("fortnightly")).toBe("fortnightly");
  });

  it("round-trips build/parse", () => {
    expect(parseIntervalToken(buildIntervalToken(6, "weeks"))).toEqual({
      n: 6,
      unit: "weeks",
    });
  });

  it("rejects out-of-bounds and malformed tokens, mirroring the backend", () => {
    expect(parseIntervalToken("every_0_days")).toBeNull();
    expect(parseIntervalToken("every_367_days")).toBeNull();
    expect(parseIntervalToken("every_53_weeks")).toBeNull();
    expect(parseIntervalToken("every_37_months")).toBeNull();
    expect(parseIntervalToken("every__weeks")).toBeNull();
    expect(parseIntervalToken("every_6_fortnights")).toBeNull();
    expect(isValidIntervalCount(0, "days")).toBe(false);
    expect(isValidIntervalCount(366, "days")).toBe(true);
    expect(isValidIntervalCount(1.5, "months")).toBe(false);
  });
});
