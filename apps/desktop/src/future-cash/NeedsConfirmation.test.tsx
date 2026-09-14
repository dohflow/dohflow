import { fireEvent, screen } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { NeedsConfirmation } from "./NeedsConfirmation";

const mocks = vi.hoisted(() => ({
  unconfirmedPastDue: vi.fn(),
  accountList: vi.fn(),
  recurringBillList: vi.fn(),
  confirmObligationEarly: vi.fn(),
  unconfirmObligation: vi.fn(),
}));

vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

const occurrence = (over = {}) => ({
  recurring_event_id: "bill-1",
  name: "Rent",
  scheduled_date: "2026-07-20",
  expected_amount_minor: 200_000,
  currency: "USD",
  days_overdue: 12,
  ...over,
});

beforeEach(() => {
  vi.clearAllMocks();
  mocks.unconfirmedPastDue.mockResolvedValue(ok([]));
  mocks.recurringBillList.mockResolvedValue(ok([]));
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: "chk-1",
        name: "Checking",
        cashflow_role: "liquid_cash",
        subtype: "checking",
        active: true,
        balance: { minor_units: 500_000, currency: "USD" },
        notes: null,
        linked_account_id: null,
        linked_account_name: null,
      },
    ]),
  );
});

describe("NeedsConfirmation (personal-cfo-4d8.27.7.6, ADR 0058)", () => {
  it("lists a past-due obligation with its amount and how long it has waited", async () => {
    mocks.unconfirmedPastDue.mockResolvedValue(ok([occurrence()]));
    renderWithClient(<NeedsConfirmation />);

    expect(
      await screen.findByText("1 bill still needs confirming"),
    ).toBeInTheDocument();
    expect(screen.getByText("Rent")).toBeInTheDocument();
    expect(screen.getByText(/12 days ago/)).toBeInTheDocument();
    expect(screen.getByText("$2,000.00")).toBeInTheDocument();
  });

  it("says what the forecast currently assumes, without telling the user to pay", async () => {
    // ADR 0018 / ADR 0058 §4: descriptive. It states the consequence for the projection —
    // which is the whole reason the section sits on Cash Flow — and does not call the
    // household late or tell them to pay.
    mocks.unconfirmedPastDue.mockResolvedValue(ok([occurrence()]));
    renderWithClient(<NeedsConfirmation />);
    const body = await screen.findByText(/money that has not left yet/i);
    expect(body).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/overdue|late|pay this now/i);
  });

  it("renders nothing at all when the queue is clear", async () => {
    // Not an empty state: a permanent "all clear" card would be furniture on the surface a
    // healthy household sees most.
    const { container } = renderWithClient(<NeedsConfirmation />);
    await vi.waitFor(() => expect(mocks.unconfirmedPastDue).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it("does not flash an all-clear before the answer arrives", async () => {
    // `occurrences === null` means "still loading", and conflating that with "nothing
    // pending" would show a clean Cash Flow tab over an unverified projection.
    let resolve: ((v: unknown) => void) | undefined;
    mocks.unconfirmedPastDue.mockReturnValue(
      new Promise((r) => {
        resolve = r;
      }),
    );
    const { container } = renderWithClient(<NeedsConfirmation />);
    expect(container).toBeEmptyDOMElement();
    resolve?.(ok([occurrence()]));
    expect(
      await screen.findByText("1 bill still needs confirming"),
    ).toBeInTheDocument();
  });

  it("pluralizes the count", async () => {
    mocks.unconfirmedPastDue.mockResolvedValue(
      ok([
        occurrence(),
        occurrence({ recurring_event_id: "bill-2", name: "Gym", days_overdue: 1 }),
      ]),
    );
    renderWithClient(<NeedsConfirmation />);
    expect(
      await screen.findByText("2 bills still need confirming"),
    ).toBeInTheDocument();
    expect(screen.getByText(/1 day ago/)).toBeInTheDocument();
  });

  it("offers the same confirm control Projected Activity uses", async () => {
    // One confirm path, one behaviour — so a confirm from here is the same audited write
    // and the undo bar keeps working (ADR 0058, consequences).
    mocks.unconfirmedPastDue.mockResolvedValue(ok([occurrence()]));
    renderWithClient(<NeedsConfirmation />);
    expect(
      await screen.findByRole("button", { name: /mark paid/i }),
    ).toBeInTheDocument();
  });
});

test("collapses and expands, hiding the queue and persisting the choice", async () => {
  localStorage.removeItem("pcfo.needsConfirmCollapsed");
  mocks.unconfirmedPastDue.mockResolvedValue(ok([occurrence()]));
  const { unmount } = renderWithClient(<NeedsConfirmation />);
  const toggle = await screen.findByRole("button", {
    name: /1 bill still needs confirming/i,
  });
  expect(toggle).toHaveAttribute("aria-expanded", "true");
  expect(screen.getByText("Rent")).toBeInTheDocument();

  fireEvent.click(toggle);
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  expect(screen.queryByText("Rent")).not.toBeInTheDocument();
  // The count stays visible while collapsed.
  expect(toggle).toHaveTextContent(/1 bill still needs confirming/i);

  // The choice sticks across a remount.
  unmount();
  renderWithClient(<NeedsConfirmation />);
  const again = await screen.findByRole("button", {
    name: /1 bill still needs confirming/i,
  });
  expect(again).toHaveAttribute("aria-expanded", "false");
  localStorage.removeItem("pcfo.needsConfirmCollapsed");
});
