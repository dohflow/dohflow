import { describe, expect, it } from "vitest";

import { SUBTYPE_LABELS, subtypesForRoleToken } from "./subtypes";

describe("account subtypes", () => {
  it("offers HSA + crypto under the investment role (personal-cfo-4d8.25.23)", () => {
    const investment = subtypesForRoleToken("investment_asset");
    const values = investment.map((s) => s.value);
    expect(values).toEqual(["brokerage", "retirement", "hsa", "crypto"]);
    expect(investment.find((s) => s.value === "hsa")?.label).toBe("HSA");
    expect(investment.find((s) => s.value === "crypto")?.label).toBe("Crypto");
  });

  it("does not leak the investment subtypes into other roles", () => {
    expect(subtypesForRoleToken("liquid_cash").map((s) => s.value)).not.toContain("hsa");
    expect(subtypesForRoleToken("credit_facility").map((s) => s.value)).not.toContain(
      "crypto",
    );
  });

  it("labels every subtype token it advertises", () => {
    for (const role of [
      "liquid_cash",
      "credit_facility",
      "loan_liability",
      "investment_asset",
      "real_asset",
    ]) {
      for (const { value } of subtypesForRoleToken(role)) {
        expect(SUBTYPE_LABELS[value]).toBeTruthy();
      }
    }
  });
});
