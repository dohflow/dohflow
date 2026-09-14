/// The account-subtype taxonomy (ADR 0028), mirrored from the Rust `AccountSubtype`.
/// Each subtype belongs to exactly one cashflow role; the storage tokens match the
/// backend, which validates the role↔subtype match (the frontend only filters the
/// picker — Rust stays authoritative, ADR 0003). Roles not listed have no subtypes.

/// Display label for a subtype storage token.
export const SUBTYPE_LABELS: Record<string, string> = {
  checking: "Checking",
  savings: "Savings",
  money_market: "Money market",
  cash: "Cash",
  credit_card: "Credit card",
  line_of_credit: "Line of credit",
  mortgage: "Mortgage",
  auto_loan: "Auto loan",
  student_loan: "Student loan",
  brokerage: "Brokerage",
  retirement: "Retirement",
  hsa: "HSA",
  crypto: "Crypto",
  property: "Property",
  vehicle: "Vehicle",
  other_real_asset: "Other",
};

/// The subtype tokens available for each cashflow-role **storage token** (as carried
/// by `AccountViewDto.cashflow_role`). Roles absent here have no subtypes.
export const SUBTYPES_BY_ROLE_TOKEN: Record<string, string[]> = {
  liquid_cash: ["checking", "savings", "money_market", "cash"],
  credit_facility: ["credit_card", "line_of_credit"],
  loan_liability: ["mortgage", "auto_loan", "student_loan"],
  investment_asset: ["brokerage", "retirement", "hsa", "crypto"],
  real_asset: ["property", "vehicle", "other_real_asset"],
};

/// `CashflowRoleDto` enum value → its storage token. The add-account form picks a
/// role via the DTO enum (`"LiquidCash"`); `AccountViewDto` carries the token
/// (`"liquid_cash"`).
export const ROLE_DTO_TO_TOKEN: Record<string, string> = {
  LiquidCash: "liquid_cash",
  CreditFacility: "credit_facility",
  LoanLiability: "loan_liability",
  InvestmentAsset: "investment_asset",
  RealAsset: "real_asset",
};

/// The subtype options (token + label) for a role **token**, or `[]` if it has none.
export function subtypesForRoleToken(
  roleToken: string,
): { value: string; label: string }[] {
  return (SUBTYPES_BY_ROLE_TOKEN[roleToken] ?? []).map((value) => ({
    value,
    label: SUBTYPE_LABELS[value] ?? value,
  }));
}
