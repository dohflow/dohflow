/// Role-aware balance sign conversion (personal-cfo-4d8.23.3).
///
/// Liabilities store a NEGATIVE signed balance (credit normal balance, core-ledger), but users
/// think in a positive "amount owed" — they see $10,000 on their card statement and enter 10000,
/// not -10000. These helpers convert between the positive amount the user enters/sees and the
/// stored signed minor units, so entry is intuitive while the ledger stays canonical. Assets are
/// unchanged (their stored balance is already the positive figure the user means). Net-worth math
/// is unaffected — it reads the stored signed balance, never these display figures.

const LIABILITY_ROLES = new Set(["credit_facility", "loan_liability"]);

/// Whether a `cashflow_role` token is a liability (its balance is entered/shown as amount owed).
export function isLiabilityRole(role: string): boolean {
  return LIABILITY_ROLES.has(role);
}

/// A user-entered figure (positive "amount owed" for liabilities) → stored signed minor units.
export function enteredToStoredMinor(role: string, enteredMinor: number): number {
  return isLiabilityRole(role) ? -enteredMinor : enteredMinor;
}

/// A stored signed balance → the figure to show (positive "amount owed" for liabilities).
export function storedToShownMinor(role: string, storedMinor: number): number {
  return isLiabilityRole(role) ? -storedMinor : storedMinor;
}

/// The entry-field label for a role's headline figure.
export function figureLabelForRole(role: string): "Value" | "Amount owed" | "Balance" {
  if (role === "real_asset") return "Value";
  if (isLiabilityRole(role)) return "Amount owed";
  return "Balance";
}
