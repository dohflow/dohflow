import { describe, expect, test } from "vitest";

import {
  enteredToStoredMinor,
  figureLabelForRole,
  isLiabilityRole,
  storedToShownMinor,
} from "./balanceSign";

describe("balanceSign", () => {
  test("liability roles are credit_facility + loan_liability", () => {
    expect(isLiabilityRole("credit_facility")).toBe(true);
    expect(isLiabilityRole("loan_liability")).toBe(true);
    expect(isLiabilityRole("liquid_cash")).toBe(false);
    expect(isLiabilityRole("investment_asset")).toBe(false);
    expect(isLiabilityRole("real_asset")).toBe(false);
  });

  test("a liability's positive amount owed stores as a negative balance and back", () => {
    // User sees $10,000 owed, enters 10000 (1_000_000 minor) → stored -1_000_000.
    expect(enteredToStoredMinor("credit_facility", 1_000_000)).toBe(-1_000_000);
    expect(storedToShownMinor("credit_facility", -1_000_000)).toBe(1_000_000);
    // Round-trip is the identity.
    expect(
      storedToShownMinor("loan_liability", enteredToStoredMinor("loan_liability", 500_000)),
    ).toBe(500_000);
  });

  test("assets pass through unchanged", () => {
    expect(enteredToStoredMinor("liquid_cash", 200_000)).toBe(200_000);
    expect(storedToShownMinor("liquid_cash", 200_000)).toBe(200_000);
    expect(storedToShownMinor("real_asset", 5_000_000)).toBe(5_000_000);
  });

  test("figure label is role-appropriate", () => {
    expect(figureLabelForRole("real_asset")).toBe("Value");
    expect(figureLabelForRole("credit_facility")).toBe("Amount owed");
    expect(figureLabelForRole("loan_liability")).toBe("Amount owed");
    expect(figureLabelForRole("liquid_cash")).toBe("Balance");
    expect(figureLabelForRole("investment_asset")).toBe("Balance");
  });
});
