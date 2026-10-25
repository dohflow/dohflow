import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto, ForecastEventDto } from "@/bindings";
import { MarkObligationPaid } from "./MarkObligationPaid";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  recurringBillList: vi.fn(),
  confirmObligationEarly: vi.fn(),
  unconfirmObligation: vi.fn(),
  householdTimezone: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const CHECKING = "0190a000-0000-7000-8000-000000000001";

function event(): ForecastEventDto {
  return {
    source_event_id: "evt-rent",
    name: "Rent",
    kind: "recurring_bill",
    amount: { minor_units: -180_000, currency: "USD" },
    assumption_basis: { kind: "recurring_schedule", frequency: "monthly" },
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: CHECKING,
        name: "Checking",
        cashflow_role: "liquid_cash",
        subtype: null,
        active: true,
        balance: { minor_units: 500_000, currency: "USD" },
      } as AccountViewDto,
    ]),
  );
  mocks.recurringBillList.mockResolvedValue(ok([]));
  mocks.confirmObligationEarly.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
  // The backend default (personal-cfo-q329) — explicit here so date-boundary tests below
  // aren't at the mercy of whatever this hook's "query still loading" fallback happens to
  // be, and so every OTHER test's default/max date is pinned to a known zone rather than
  // whichever zone the machine running the suite happens to be in.
  mocks.householdTimezone.mockResolvedValue(ok("UTC"));
});

test("marks a bill occurrence paid with the event amount + a liquid account", async () => {
  renderWithClient(<MarkObligationPaid event={event()} scheduledDate="2026-07-20" />);

  fireEvent.click(await screen.findByRole("button", { name: /mark paid/i }));
  // Amount is prefilled from the occurrence magnitude.
  expect(screen.getByLabelText<HTMLInputElement>(/amount paid/i).value).toBe("1800");

  fireEvent.click(screen.getByRole("button", { name: /confirm payment/i }));
  await waitFor(() => expect(mocks.confirmObligationEarly).toHaveBeenCalledTimes(1));
  const input = mocks.confirmObligationEarly.mock.calls[0]?.[0];
  expect(input).toMatchObject({
    recurring_event_id: "evt-rent",
    scheduled_date: "2026-07-20",
    actual_amount: { minor_units: 180_000, currency: "USD" },
    paying_account_id: CHECKING,
  });
  // The default pay date is a real YYYY-MM-DD string (not a stringified function).
  expect(input.actual_date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
});

// The Undo of a mark-paid lives in a shell notice (docs/agent/FRONTEND.md section 6.2,
// personal-cfo-4d8.32), because the row leaves the forecast on success.
async function markPaid() {
  fireEvent.click(await screen.findByRole("button", { name: /mark paid/i }));
  fireEvent.click(screen.getByRole("button", { name: /confirm payment/i }));
}
const notifications = () => screen.findByRole("region", { name: "Notifications" });

test("marking paid shows a notice, and Undo reverses exactly that occurrence", async () => {
  mocks.unconfirmObligation.mockResolvedValue(ok({ op_seq: 2, replayed: false }));
  renderWithClient(<MarkObligationPaid event={event()} scheduledDate="2026-07-20" />);
  await markPaid();

  const region = await notifications();
  expect(await within(region).findByText("Marked Rent paid.")).toBeInTheDocument();
  fireEvent.click(within(region).getByRole("button", { name: "Undo" }));
  await waitFor(() => expect(mocks.unconfirmObligation).toHaveBeenCalledTimes(1));
  expect(mocks.unconfirmObligation.mock.calls[0]?.[0]).toMatchObject({
    recurring_event_id: "evt-rent",
    scheduled_date: "2026-07-20",
    idempotency_key: expect.stringMatching(/.+/),
  });
  // A successful Undo closes the notice.
  await waitFor(() =>
    expect(screen.queryByRole("region", { name: "Notifications" })).toBeNull(),
  );
});

test("a failed Undo keeps the notice and its Undo, with its own alert, not inside the status", async () => {
  mocks.unconfirmObligation.mockResolvedValue({
    status: "error",
    error: { Validation: "nothing to undo" },
  });
  renderWithClient(<MarkObligationPaid event={event()} scheduledDate="2026-07-20" />);
  await markPaid();

  const region = await notifications();
  await within(region).findByText("Marked Rent paid.");
  fireEvent.click(within(region).getByRole("button", { name: "Undo" }));
  const alert = await within(region).findByRole("alert");
  expect(alert).toHaveTextContent("Couldn't undo — try again.");
  // The error is not inside the live status region (the a11y issue this migration removes).
  expect(within(region).getByRole("status")).not.toContainElement(alert);
  expect(within(region).getByRole("button", { name: "Undo" })).toBeEnabled();
});

test("the Undo still works after the row that marked it has gone", async () => {
  mocks.unconfirmObligation.mockResolvedValue(ok({ op_seq: 2, replayed: false }));
  function Host() {
    const [shown, setShown] = useState(true);
    return (
      <>
        {shown && <MarkObligationPaid event={event()} scheduledDate="2026-07-20" />}
        <button onClick={() => setShown(false)}>forecast refreshed</button>
      </>
    );
  }
  renderWithClient(<Host />);
  await markPaid();
  const region = await notifications();
  await within(region).findByText("Marked Rent paid.");

  // The occurrence leaves the forecast, so its row unmounts; the notice belongs to the shell.
  fireEvent.click(screen.getByRole("button", { name: "forecast refreshed" }));
  expect(screen.queryByRole("button", { name: /mark paid/i })).toBeNull();
  fireEvent.click(within(region).getByRole("button", { name: "Undo" }));
  await waitFor(() => expect(mocks.unconfirmObligation).toHaveBeenCalledTimes(1));
});

test("a successful Undo refreshes the forecast, so the occurrence comes back", async () => {
  mocks.unconfirmObligation.mockResolvedValue(ok({ op_seq: 2, replayed: false }));
  const fetches = vi.fn(async () => "forecast");
  function Forecast() {
    useQuery({ queryKey: ["forecast", "test"], queryFn: fetches });
    return <MarkObligationPaid event={event()} scheduledDate="2026-07-20" />;
  }
  renderWithClient(<Forecast />);
  await markPaid();
  const region = await notifications();
  await within(region).findByText("Marked Rent paid.");
  // Marking paid refreshed it; wait for that to settle, then count.
  await waitFor(() => expect(fetches.mock.calls.length).toBeGreaterThanOrEqual(2));
  const before = fetches.mock.calls.length;
  fireEvent.click(within(region).getByRole("button", { name: "Undo" }));
  await waitFor(() => expect(fetches.mock.calls.length).toBeGreaterThan(before));
});

test("no notice (and no Undo) when the confirm fails", async () => {
  mocks.confirmObligationEarly.mockResolvedValue({
    status: "error",
    error: { kind: "validation", message: "this bill is paid via its card's payment" },
  });
  renderWithClient(<MarkObligationPaid event={event()} scheduledDate="2026-07-20" />);
  await markPaid();
  await waitFor(() => expect(mocks.confirmObligationEarly).toHaveBeenCalledTimes(1));
  expect(screen.queryByRole("region", { name: "Notifications" })).toBeNull();
});

test("a bill with no name reads 'Marked bill paid.'", async () => {
  renderWithClient(
    <MarkObligationPaid event={{ ...event(), name: "" }} scheduledDate="2026-07-20" />,
  );
  await markPaid();
  expect(await screen.findByText("Marked bill paid.")).toBeInTheDocument();
});

test("allows a $0 amount — nothing due this cycle", async () => {
  renderWithClient(<MarkObligationPaid event={event()} scheduledDate="2026-07-20" />);

  fireEvent.click(await screen.findByRole("button", { name: /mark paid/i }));
  fireEvent.change(screen.getByLabelText<HTMLInputElement>(/amount paid/i), {
    target: { value: "0" },
  });
  fireEvent.click(screen.getByRole("button", { name: /confirm payment/i }));

  await waitFor(() => expect(mocks.confirmObligationEarly).toHaveBeenCalledTimes(1));
  expect(mocks.confirmObligationEarly.mock.calls[0]?.[0]).toMatchObject({
    actual_amount: { minor_units: 0, currency: "USD" },
  });
});

test("reads 'Confirm it cleared' for an autopay bill", async () => {
  mocks.recurringBillList.mockResolvedValue(
    ok([{ id: "evt-rent", autopay_enabled: true, autopay_account_id: null }]),
  );
  renderWithClient(<MarkObligationPaid event={event()} scheduledDate="2026-07-20" />);
  expect(
    await screen.findByRole("button", { name: /confirm it cleared/i }),
  ).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /^mark paid$/i })).toBeNull();
});

test("self-hides when there is no liquid account to pay from", async () => {
  mocks.accountList.mockResolvedValue(ok([]));
  const { container } = renderWithClient(
    <MarkObligationPaid event={event()} scheduledDate="2026-07-20" />,
  );
  await waitFor(() => expect(mocks.accountList).toHaveBeenCalled());
  expect(container).toBeEmptyDOMElement();
});

test("the paid-date defaults to the DUE date for a past-due occurrence", async () => {
  renderWithClient(<MarkObligationPaid event={event()} scheduledDate="2026-07-20" />);
  fireEvent.click(await screen.findByRole("button", { name: /mark paid/i }));
  expect(screen.getByLabelText<HTMLInputElement>(/date paid/i).value).toBe("2026-07-20");
});

// Pinned system time, not `new Date()` at test-run time: this suite runs on whatever
// machine/CI executes it (America/Los_Angeles locally, unknown in CI), and the OLD version
// of this test computed its own expectation from the REAL local clock — which silently
// disagreed with a UTC-mocked household timezone for part of every day (exactly the
// personal-cfo-5ie.11 bug shape, just relocated into the test itself). A fixed instant plus
// a hardcoded expected date is deterministic regardless of when or where the suite runs.
test("the paid-date clamps to today (household timezone) for a future occurrence (early confirm)", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    vi.setSystemTime(new Date("2026-03-15T12:00:00Z"));
    renderWithClient(<MarkObligationPaid event={event()} scheduledDate="2099-01-01" />);
    fireEvent.click(await screen.findByRole("button", { name: /mark paid/i }));
    expect(screen.getByLabelText<HTMLInputElement>(/date paid/i).value).toBe("2026-03-15");
  } finally {
    vi.useRealTimers();
  }
});

// personal-cfo-ku2hn / personal-cfo-q329: todayIso() used to compute the BROWSER's local
// date; the correct boundary is the HOUSEHOLD timezone (ADR 0021 §1), which the frontend
// now has an accessor for. Pins the exact failure mode the household-timezone fix targets:
// a moment that is already tomorrow in UTC (and in this machine's real local zone) but
// still today in the mocked household's zone must clamp to the HOUSEHOLD date.
test("the paid-date clamp uses the HOUSEHOLD timezone, not the browser's local zone", async () => {
  mocks.householdTimezone.mockResolvedValue(ok("Pacific/Kiritimati")); // UTC+14
  // `shouldAdvanceTime` keeps real timers running (scaled) under the hood, so
  // testing-library's `findByRole` polling (setTimeout-based) still resolves —
  // plain `useFakeTimers()` freezes it and every subsequent test hangs too.
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    // 23:30 UTC on 2026-01-01 is already 2026-01-02 in Kiritimati.
    vi.setSystemTime(new Date("2026-01-01T23:30:00Z"));
    renderWithClient(<MarkObligationPaid event={event()} scheduledDate="2099-01-01" />);
    fireEvent.click(await screen.findByRole("button", { name: /mark paid/i }));
    expect(screen.getByLabelText<HTMLInputElement>(/date paid/i).value).toBe("2026-01-02");
  } finally {
    vi.useRealTimers();
  }
});

test("the confirm action sits far right, where the trigger button was", async () => {
  renderWithClient(<MarkObligationPaid event={event()} scheduledDate="2026-07-20" />);
  fireEvent.click(await screen.findByRole("button", { name: /mark paid/i }));
  const confirm = screen.getByRole("button", { name: /confirm payment/i });
  const row = confirm.parentElement as HTMLElement;
  expect(row.className).toContain("justify-end");
  // Confirm is the LAST button in the row so it lands outermost-right.
  const buttons = Array.from(row.querySelectorAll("button"));
  expect(buttons[buttons.length - 1]).toBe(confirm);
});
