import { fireEvent, render, screen } from "@testing-library/react";

import type { AccountViewDto } from "@/bindings";

import { DebtScopeBar } from "./DebtScopeBar";
import { scopeSentence } from "./debtAccounts";

const debt = (over: Partial<AccountViewDto> = {}): AccountViewDto =>
  ({
    id: "card-1",
    name: "Visa",
    cashflow_role: "credit_facility",
    subtype: "credit_card",
    active: true,
    balance: { minor_units: -250_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  }) as AccountViewDto;

describe("DebtScopeBar (personal-cfo-39xn)", () => {
  it("shows each debt as a chip carrying its amount owed", () => {
    // The point of the redesign: the scope is readable without opening anything, and a
    // liability reads as a POSITIVE amount owed (ADR 0044 / balanceSign).
    render(
      <DebtScopeBar accounts={[debt()]} selected={[]} onChange={vi.fn()} />,
    );
    const chip = screen.getByRole("button", { name: /Visa/ });
    expect(chip).toHaveTextContent("$2,500.00");
    expect(chip).toHaveAttribute("aria-pressed", "true");
  });

  it("starts from everything on the first untick", () => {
    // An empty selection means ALL, so unticking one must leave the OTHERS — not select
    // only the one that was clicked. That inversion is what this representation invites.
    const onChange = vi.fn();
    render(
      <DebtScopeBar
        accounts={[debt(), debt({ id: "loan-1", name: "Car loan" })]}
        selected={[]}
        onChange={onChange}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Car loan/ }));
    expect(onChange).toHaveBeenCalledWith(["card-1"]);
  });

  it("collapses back to 'all' when everything is re-ticked", () => {
    const onChange = vi.fn();
    render(
      <DebtScopeBar
        accounts={[debt(), debt({ id: "loan-1", name: "Car loan" })]}
        selected={["card-1"]}
        onChange={onChange}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Car loan/ }));
    // Not ["card-1","loan-1"] — the "all" representation keeps the default from going
    // stale when a new debt is added later.
    expect(onChange).toHaveBeenCalledWith([]);
  });

  it("disables the reset when nothing is narrowed", () => {
    render(<DebtScopeBar accounts={[debt()]} selected={[]} onChange={vi.fn()} />);
    expect(screen.getByRole("button", { name: "All debts" })).toBeDisabled();
  });

  it("keeps the page shape identical between one and many selected debts", () => {
    // ADR 0057 §1: selection is a filter, not a mode. The bar renders the same structure
    // either way — one chip row, one sentence, one reset.
    const many = render(
      <DebtScopeBar
        accounts={[debt(), debt({ id: "loan-1", name: "Car loan" })]}
        selected={[]}
        onChange={vi.fn()}
      />,
    );
    const manyShape = many.container.querySelectorAll("p").length;
    many.unmount();
    const one = render(
      <DebtScopeBar
        accounts={[debt(), debt({ id: "loan-1", name: "Car loan" })]}
        selected={["card-1"]}
        onChange={vi.fn()}
      />,
    );
    expect(one.container.querySelectorAll("p").length).toBe(manyShape);
  });

  describe("scopeSentence", () => {
    const two = [debt(), debt({ id: "loan-1", name: "Car loan" })];
    it("names the whole set when nothing is narrowed", () => {
      expect(scopeSentence(two, [])).toBe(
        "Everything below covers all 2 of your debts.",
      );
    });
    it("names the debt when exactly one is selected", () => {
      expect(scopeSentence(two, ["card-1"])).toBe(
        "Everything below covers Visa only.",
      );
    });
    it("counts when some are selected", () => {
      const three = [...two, debt({ id: "c3", name: "Amex" })];
      expect(scopeSentence(three, ["card-1", "c3"])).toBe(
        "Everything below covers 2 of your 3 debts.",
      );
    });
    it("does not say 'all 1 of your debts'", () => {
      expect(scopeSentence([debt()], [])).toBe(
        "Everything below covers your one debt.",
      );
    });
    it("says so when there is nothing to scope", () => {
      expect(scopeSentence([], [])).toBe("No debts to scope yet.");
    });
  });
});
