import { describeIpcError } from "./useVault";

vi.mock("@/bindings", () => ({ commands: {} }));

// personal-cfo-6wk.22: a second bill cannot be linked to a loan that already has one; the typed
// error carries the existing bill so the message can name it.
test("a loan that already has a linked bill is described by naming that bill", () => {
  expect(
    describeIpcError({
      LoanAlreadyLinked: { bill_id: "0190b000-0000-7000-8000-000000000001", bill_name: "ACME HOME LOANS" },
    }),
  ).toBe("This loan already has a linked bill: ACME HOME LOANS.");
});

test("the other error shapes still describe themselves", () => {
  expect(describeIpcError({ Validation: "Enter a name." })).toBe("Enter a name.");
  expect(describeIpcError("VaultLocked")).toBe("The vault is locked.");
});
