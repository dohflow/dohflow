import { screen } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { BillLoanLinkDto, RecurringBillDto } from "@/bindings";
import { LinkedBillNotices } from "./LinkedBillNotices";

const mocks = vi.hoisted(() => ({ recurringBillList: vi.fn() }));

vi.mock("@/bindings", () => ({ commands: { recurringBillList: mocks.recurringBillList } }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const usd = (minor_units: number) => ({ minor_units, currency: "USD" });

function bill(id: string, name: string, loan_link: BillLoanLinkDto | null): RecurringBillDto {
  return {
    id,
    name,
    bill_type: "subscription",
    amount: usd(150_000),
    frequency: "monthly",
    anchor_date: "2026-07-15",
    autopay_account_id: null,
    autopay_account_name: null,
    autopay_enabled: false,
    next_due_date: "2026-07-15",
    description: null,
    active: true,
    created_at: "2026-06-01T00:00:00Z",
    archived_at: null,
    category_id: null,
    tag_ids: [],
    loan_link,
    not_loan_payment: null,
  };
}

const healthy: BillLoanLinkDto = {
  loan_account_id: "loan-1",
  loan_name: "Test Mortgage",
  in_force: true,
  cadence_changed: false,
  below_scheduled: false,
  scheduled_payment: usd(150_000),
  other: null,
};

test("only links that need attention are listed, with the approved sentences", async () => {
  mocks.recurringBillList.mockResolvedValue(
    ok([
      bill("a", "Healthy bill", healthy),
      bill("b", "Weekly now", { ...healthy, in_force: false, cadence_changed: true }),
      bill("c", "Too small", { ...healthy, below_scheduled: true }),
      bill("d", "Not linked", null),
    ]),
  );
  renderWithClient(<LinkedBillNotices />);

  const region = await screen.findByRole("region", { name: "Linked bills" });
  expect(region).toHaveTextContent("Weekly now");
  expect(region).toHaveTextContent(
    "This bill no longer repeats like Test Mortgage's payment, so both now count.",
  );
  expect(region).toHaveTextContent("Too small");
  expect(region).toHaveTextContent("This linked bill is less than Test Mortgage's scheduled payment.");
  expect(region).not.toHaveTextContent("Healthy bill");
  expect(region).not.toHaveTextContent("Not linked");
});

test("nothing is shown when every link is doing its job", async () => {
  mocks.recurringBillList.mockResolvedValue(ok([bill("a", "Healthy bill", healthy)]));
  const { container } = renderWithClient(<LinkedBillNotices />);
  await vi.waitFor(() => expect(mocks.recurringBillList).toHaveBeenCalled());
  expect(container).toBeEmptyDOMElement();
});

test("a bill with both problems lists both messages", async () => {
  mocks.recurringBillList.mockResolvedValue(
    ok([bill("a", "Both", { ...healthy, cadence_changed: true, below_scheduled: true })]),
  );
  renderWithClient(<LinkedBillNotices />);
  const region = await screen.findByRole("region", { name: "Linked bills" });
  expect(region).toHaveTextContent("so both now count.");
  expect(region).toHaveTextContent("is less than Test Mortgage's scheduled payment.");
});
