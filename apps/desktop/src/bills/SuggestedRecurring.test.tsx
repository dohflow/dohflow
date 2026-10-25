import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto, RecurringCandidateDto } from "@/bindings";
import { SuggestedRecurring } from "./SuggestedRecurring";

const mocks = vi.hoisted(() => ({
  recurringCandidates: vi.fn(),
  accountList: vi.fn(),
  recurringBillList: vi.fn(),
  createRecurringBill: vi.fn(),
  checkLoanOverlap: vi.fn(),
  setBillNotLoanPayment: vi.fn(),
  recurringBillHistory: vi.fn(),
  markReviewed: vi.fn(),
  dismissRecurringSuggestion: vi.fn(),
  categoryList: vi.fn(),
  tagList: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    recurringCandidates: mocks.recurringCandidates,
    accountList: mocks.accountList,
    recurringBillList: mocks.recurringBillList,
    createRecurringBill: mocks.createRecurringBill,
    checkLoanOverlap: mocks.checkLoanOverlap,
    setBillNotLoanPayment: mocks.setBillNotLoanPayment,
    recurringBillHistory: mocks.recurringBillHistory,
    markReviewed: mocks.markReviewed,
    dismissRecurringSuggestion: mocks.dismissRecurringSuggestion,
    categoryList: mocks.categoryList,
    tagList: mocks.tagList,
  },
}));

const GROCERIES_ID = "0190c000-0000-7000-8000-0000000000c1";
const VACATION_ID = "0190d000-0000-7000-8000-0000000000d1";

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const mutationOk = () => ok({ op_seq: 1, replayed: false });
const CHECKING = "0190a000-0000-7000-8000-000000000001";

function candidate(over: Partial<RecurringCandidateDto> = {}): RecurringCandidateDto {
  return {
    merchant_key: "SPOTIFY",
    display: "SPOTIFY",
    amount_minor: 1_099,
    amount_min_minor: 1_099,
    amount_max_minor: 1_099,
    currency: "USD",
    frequency: "monthly",
    last_seen: "2026-07-05",
    next_date: "2026-08-05",
    occurrence_count: 6,
    confidence_bps: 10_000,
    dominant_category_id: null,
    source_account_names: ["Venture X"],
    source_account_id: CHECKING,
    typical_day_of_month: 5,
    loan_match: null,
    observations: [
      { date: "2026-07-05", amount_minor: 1_099, account_name: "Venture X" },
      { date: "2026-06-05", amount_minor: 1_099, account_name: "Venture X" },
    ],
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.checkLoanOverlap.mockResolvedValue(ok(null));
  mocks.setBillNotLoanPayment.mockResolvedValue(mutationOk());
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: CHECKING,
        name: "Checking",
        cashflow_role: "liquid_cash",
        subtype: null,
        active: true,
        balance: { minor_units: 100_000, currency: "USD" },
      } as AccountViewDto,
    ]),
  );
  mocks.recurringBillList.mockResolvedValue(ok([]));
  mocks.createRecurringBill.mockResolvedValue(
    ok({ op_seq: 1, replayed: false, event_id: "0190e000-0000-7000-8000-0000000000e1" }),
  );
  mocks.recurringBillHistory.mockResolvedValue(ok([]));
  mocks.markReviewed.mockResolvedValue(mutationOk());
  mocks.dismissRecurringSuggestion.mockResolvedValue(mutationOk());
  mocks.categoryList.mockResolvedValue(
    ok([
      {
        id: GROCERIES_ID,
        parent_id: null,
        name: "Groceries",
        category_type: "expense",
        icon: null,
        color: null,
        is_system: true,
        forecast_behavior: "variable_regular",
        archived: false,
      },
    ]),
  );
  mocks.tagList.mockResolvedValue(
    ok([{ id: VACATION_ID, name: "Vacation", color: null, archived: false }]),
  );
});

test("lists a detected candidate with its cadence + amount", async () => {
  mocks.recurringCandidates.mockResolvedValue(ok([candidate()]));
  renderWithClient(<SuggestedRecurring />);

  expect(await screen.findByText("SPOTIFY")).toBeInTheDocument();
  expect(screen.getByText(/monthly/i)).toBeInTheDocument();
  expect(screen.getByText(/seen 6 times/i)).toBeInTheDocument();
});

test("surfaces the amount range, last-seen, and dominant category (4d8.24.5)", async () => {
  mocks.recurringCandidates.mockResolvedValue(
    ok([
      candidate({
        amount_min_minor: 999,
        amount_max_minor: 1_199,
        last_seen: "2026-06-05",
        dominant_category_id: GROCERIES_ID,
      }),
    ]),
  );
  renderWithClient(<SuggestedRecurring />);

  // A variable bill shows a min–max range, not just the median.
  expect(await screen.findByText(/\$9\.99.*\$11\.99/)).toBeInTheDocument();
  // The last-seen date + the dominant category name (Groceries, from the categoryList mock).
  expect(screen.getByText(/last seen/i)).toBeInTheDocument();
  expect(screen.getByText(/Groceries/)).toBeInTheDocument();
});

test("a fixed-amount candidate shows a single amount, not a range", async () => {
  mocks.recurringCandidates.mockResolvedValue(
    ok([candidate({ amount_min_minor: 1_099, amount_max_minor: 1_099 })]),
  );
  renderWithClient(<SuggestedRecurring />);
  const meta = await screen.findByText(/seen 6 times/i);
  expect(meta.textContent).toContain("$10.99");
  expect(meta.textContent).not.toContain("–");
});

test("pre-fills the promote form's category from the dominant category (4d8.24.5)", async () => {
  mocks.recurringCandidates.mockResolvedValue(
    ok([candidate({ dominant_category_id: GROCERIES_ID })]),
  );
  renderWithClient(<SuggestedRecurring />);

  fireEvent.click(await screen.findByRole("button", { name: /add as recurring/i }));
  // The category is pre-selected before the user touches it.
  const category = await screen.findByLabelText<HTMLSelectElement>("Category");
  expect(category.value).toBe(GROCERIES_ID);

  // Creating without changing the category submits the pre-filled dominant one.
  fireEvent.click(screen.getByRole("button", { name: /create recurring bill/i }));
  await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
  expect(mocks.createRecurringBill.mock.calls[0]?.[0]).toMatchObject({
    category_id: GROCERIES_ID,
  });
});

test("Add as recurring opens a pre-filled form and creates the bill", async () => {
  mocks.recurringCandidates.mockResolvedValue(ok([candidate()]));
  renderWithClient(<SuggestedRecurring />);

  fireEvent.click(await screen.findByRole("button", { name: /add as recurring/i }));

  // Pre-filled from the candidate.
  const name = await screen.findByLabelText<HTMLInputElement>("Bill name");
  expect(name.value).toBe("SPOTIFY");
  expect(screen.getByLabelText<HTMLInputElement>("Amount").value).toBe("10.99");

  // Set the category up front (personal-cfo-4d8.24.5).
  fireEvent.change(await screen.findByLabelText("Category"), {
    target: { value: GROCERIES_ID },
  });
  // Tag it (personal-cfo-4d8.24.5.1).
  fireEvent.click(await screen.findByRole("button", { name: "Vacation" }));

  fireEvent.click(screen.getByRole("button", { name: /create recurring bill/i }));
  await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
  const input = mocks.createRecurringBill.mock.calls[0]?.[0];
  expect(input).toMatchObject({
    name: "SPOTIFY",
    frequency: "monthly",
    anchor_date: "2026-07-05",
    amount: { minor_units: 1_099, currency: "USD" },
    // The candidate's merchant key is persisted so the suggestion stays suppressed
    // even if the bill is later renamed (personal-cfo-5n4.8).
    source_merchant_key: "SPOTIFY",
    // The chosen category + tags are submitted (personal-cfo-4d8.24.5 / .5.1).
    category_id: GROCERIES_ID,
    tag_ids: [VACATION_ID],
  });
  // The creation-time loan check saw the same bill and the candidate's merchant key (6wk.20).
  expect(mocks.checkLoanOverlap.mock.calls[0]?.[0]).toMatchObject({
    name: "SPOTIFY",
    source_merchant_key: "SPOTIFY",
  });
});

// ----- a loan's own payment is not offered as a new recurring bill (personal-cfo-6wk.20) -------------

test("a STRONG loan match says it is the loan's payment and offers no way to add it as a bill", async () => {
  mocks.recurringCandidates.mockResolvedValue(
    ok([
      candidate({
        merchant_key: "ACME HOME LOANS",
        display: "ACME HOME LOANS",
        loan_match: {
          loan_account_id: "loan-1",
          loan_name: "Test Mortgage",
          strength: "strong",
          matched_by: "payee",
        },
      }),
    ]),
  );
  renderWithClient(<SuggestedRecurring />);

  expect(await screen.findByText("This is Test Mortgage's payment.")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /add as recurring/i })).toBeNull();
  // It can still be dismissed.
  expect(screen.getByRole("button", { name: /dismiss acme home loans/i })).toBeInTheDocument();
});

test("a POSSIBLE loan match is a soft note and the suggestion can still be added", async () => {
  mocks.recurringCandidates.mockResolvedValue(
    ok([
      candidate({
        loan_match: {
          loan_account_id: "loan-1",
          loan_name: "Test Mortgage",
          strength: "possible",
          matched_by: null,
        },
      }),
    ]),
  );
  renderWithClient(<SuggestedRecurring />);

  expect(
    await screen.findByText("This bill might be the same as Test Mortgage's payment."),
  ).toBeInTheDocument();
  expect(screen.queryByText(/this is test mortgage/i)).toBeNull();
  expect(screen.getByRole("button", { name: /add as recurring/i })).toBeInTheDocument();
});

test("a suggestion with no loan match shows no loan text at all", async () => {
  mocks.recurringCandidates.mockResolvedValue(ok([candidate()]));
  renderWithClient(<SuggestedRecurring />);
  await screen.findByText("SPOTIFY");
  expect(screen.queryByText(/payment\./i)).toBeNull();
  expect(screen.queryByText(/might be the same/i)).toBeNull();
});

test("Dismiss records a suppression with the row's identity + pattern (4d8.24.6)", async () => {
  mocks.recurringCandidates.mockResolvedValue(ok([candidate()]));
  renderWithClient(<SuggestedRecurring />);

  fireEvent.click(await screen.findByRole("button", { name: /dismiss spotify/i }));

  await waitFor(() =>
    expect(mocks.dismissRecurringSuggestion).toHaveBeenCalledTimes(1),
  );
  const input = mocks.dismissRecurringSuggestion.mock.calls[0]?.[0];
  expect(input).toMatchObject({
    merchant_key: "SPOTIFY",
    currency: "USD",
    amount_minor: 1_099,
    frequency: "monthly",
  });
});

test("a dismissed candidate drops out after the list refreshes", async () => {
  // First load shows the candidate; after dismissal the query refetches empty.
  mocks.recurringCandidates
    .mockResolvedValueOnce(ok([candidate()]))
    .mockResolvedValue(ok([]));
  renderWithClient(<SuggestedRecurring />);

  fireEvent.click(await screen.findByRole("button", { name: /dismiss spotify/i }));
  await waitFor(() =>
    expect(screen.queryByText("SPOTIFY")).not.toBeInTheDocument(),
  );
});

test("renders nothing when there are no suggestions", () => {
  mocks.recurringCandidates.mockResolvedValue(ok([]));
  const { container } = renderWithClient(<SuggestedRecurring />);
  expect(container).toBeEmptyDOMElement();
});

test("promotion surfaces the retro-attached count with an explicit mark-reviewed affordance (ADR 0047 s1)", async () => {
  mocks.recurringCandidates.mockResolvedValue(ok([candidate()]));
  mocks.recurringBillHistory.mockResolvedValue(
    ok([
      {
        scheduled_date: "2026-05-05",
        expected_amount_minor: 1_099,
        currency: "USD",
        status: "paid",
        linked_transaction_id: "0190f000-0000-7000-8000-0000000000f1",
      },
      {
        scheduled_date: "2026-06-05",
        expected_amount_minor: 1_099,
        currency: "USD",
        status: "paid",
        linked_transaction_id: "0190f000-0000-7000-8000-0000000000f2",
      },
      // A future occurrence with no link must not count.
      {
        scheduled_date: "2026-08-05",
        expected_amount_minor: 1_099,
        currency: "USD",
        status: "scheduled",
        linked_transaction_id: null,
      },
    ]),
  );
  renderWithClient(<SuggestedRecurring />);

  fireEvent.click(await screen.findByRole("button", { name: /add as recurring/i }));
  fireEvent.click(
    await screen.findByRole("button", { name: /create recurring bill/i }),
  );

  // The panel reports only LINKED occurrences, and review is an explicit click
  // (ADR 0032 — never silent).
  await screen.findByText(/matched 2 past transactions for spotify/i);
  expect(mocks.markReviewed).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: /mark 2 reviewed/i }));
  await screen.findByText(/marked 2 past transactions reviewed/i);
  expect(mocks.markReviewed).toHaveBeenCalledTimes(2);
  expect(mocks.markReviewed.mock.calls.map((call) => call[0])).toEqual([
    "0190f000-0000-7000-8000-0000000000f1",
    "0190f000-0000-7000-8000-0000000000f2",
  ]);
  expect(mocks.markReviewed.mock.calls.every((call) => call[1] === true)).toBe(true);
});

test("promotion with no matched history shows no attached panel", async () => {
  mocks.recurringCandidates.mockResolvedValue(ok([candidate()]));
  mocks.recurringBillHistory.mockResolvedValue(ok([]));
  renderWithClient(<SuggestedRecurring />);

  fireEvent.click(await screen.findByRole("button", { name: /add as recurring/i }));
  fireEvent.click(
    await screen.findByRole("button", { name: /create recurring bill/i }),
  );
  await waitFor(() => expect(mocks.recurringBillHistory).toHaveBeenCalledTimes(1));
  expect(screen.queryByText(/matched .* past transactions/i)).toBeNull();
});

test("shows the detection proof: typical day, pay-from account, and the observations expander (4d8.25.11)", async () => {
  mocks.recurringCandidates.mockResolvedValue(ok([candidate()]));
  renderWithClient(<SuggestedRecurring />);
  const meta = await screen.findByText(/seen 6 times/i);
  expect(meta.textContent).toContain("roughly the 5th");
  expect(meta.textContent).toContain("always on Venture X");
  // The proof expander lists the observed charges with their accounts.
  const summary = screen.getByText(/why\? 2 observed charges/i);
  fireEvent.click(summary);
  const rows = screen.getAllByText(/Venture X/);
  expect(rows.length).toBeGreaterThanOrEqual(2);
});

test("a multi-account series lists the accounts and leaves pay-from unfilled (ADR 0047 s4)", async () => {
  mocks.recurringCandidates.mockResolvedValue(
    ok([
      candidate({
        source_account_names: ["Checking", "Venture X"],
        source_account_id: null,
      }),
    ]),
  );
  renderWithClient(<SuggestedRecurring />);
  const meta = await screen.findByText(/seen 6 times/i);
  expect(meta.textContent).toContain("across Checking, Venture X");
  fireEvent.click(screen.getByRole("button", { name: /add as recurring/i }));
  const paySelect = (await screen.findByLabelText(
    /autopay account/i,
  )) as HTMLSelectElement;
  expect(paySelect.value).toBe("");
});

test("promoting a single-account candidate prefills the pay-from account (ADR 0047 s4)", async () => {
  mocks.recurringCandidates.mockResolvedValue(ok([candidate()]));
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: CHECKING,
        name: "Venture X",
        cashflow_role: "credit_facility",
        subtype: null,
        active: true,
        balance: { minor_units: -50_000, currency: "USD" },
      },
    ]),
  );
  renderWithClient(<SuggestedRecurring />);
  fireEvent.click(await screen.findByRole("button", { name: /add as recurring/i }));
  const paySelect = (await screen.findByLabelText(
    /autopay account/i,
  )) as HTMLSelectElement;
  expect(paySelect.value).toBe(CHECKING);
});

// ----- the declared-terms ASK on a suggestion (planning's ruling) ------------------------------------------

function askingCandidate() {
  return candidate({
    merchant_key: "ACME HOME LOANS",
    display: "ACME HOME LOANS",
    loan_match: {
      loan_account_id: "loan-1",
      loan_name: "Test Mortgage",
      strength: "possible",
      matched_by: "declared_terms",
    },
  });
}

test("a suggestion that fits the declared loan terms is shown, labeled as looking like the loan's payment, and asks", async () => {
  mocks.recurringCandidates.mockResolvedValue(ok([askingCandidate()]));
  renderWithClient(<SuggestedRecurring />);

  // Still shown (the owner's import-only case), never asserted to be counted twice.
  expect(await screen.findByText("ACME HOME LOANS")).toBeInTheDocument();
  expect(screen.getByText("Looks like Test Mortgage's payment")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "It's Test Mortgage's payment" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Add as a separate bill" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /add as recurring/i })).toBeNull();
  expect(screen.queryByText(/counted twice|might be the same/i)).toBeNull();
});

test("\"It's the loan's payment\" dismisses the suggestion, remembered with its reason", async () => {
  mocks.recurringCandidates.mockResolvedValue(ok([askingCandidate()]));
  renderWithClient(<SuggestedRecurring />);

  fireEvent.click(await screen.findByRole("button", { name: "It's Test Mortgage's payment" }));
  await waitFor(() => expect(mocks.dismissRecurringSuggestion).toHaveBeenCalledTimes(1));
  expect(mocks.dismissRecurringSuggestion.mock.calls[0]?.[0]).toMatchObject({
    merchant_key: "ACME HOME LOANS",
    reason: "loan_payment",
  });
  expect(mocks.createRecurringBill).not.toHaveBeenCalled();
});

test("\"Add as a separate bill\" answers the question, so creating does not ask it again", async () => {
  mocks.recurringCandidates.mockResolvedValue(ok([askingCandidate()]));
  // The creation-time check would ask (declared terms) if it were not already answered.
  mocks.checkLoanOverlap.mockResolvedValue(
    ok({
      loan_account_id: "loan-1",
      loan_name: "Test Mortgage",
      strength: "possible",
      matched_by: "declared_terms",
    }),
  );
  renderWithClient(<SuggestedRecurring />);

  fireEvent.click(await screen.findByRole("button", { name: "Add as a separate bill" }));
  fireEvent.click(await screen.findByRole("button", { name: /create recurring bill/i }));

  await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
  expect(screen.queryByText("Is this Test Mortgage's payment?")).toBeNull();
  // "Add as a separate bill" IS the answer "no": it is remembered with the new bill.
  await waitFor(() => expect(mocks.setBillNotLoanPayment).toHaveBeenCalledTimes(1));
  expect(mocks.setBillNotLoanPayment.mock.calls[0]?.slice(0, 2)).toEqual([
    "0190e000-0000-7000-8000-0000000000e1",
    "loan-1",
  ]);
});

test("an ordinary soft possible match (no declared terms) keeps the normal buttons", async () => {
  mocks.recurringCandidates.mockResolvedValue(
    ok([
      candidate({
        loan_match: {
          loan_account_id: "loan-1",
          loan_name: "Test Mortgage",
          strength: "possible",
          matched_by: null,
        },
      }),
    ]),
  );
  renderWithClient(<SuggestedRecurring />);
  expect(await screen.findByRole("button", { name: /add as recurring/i })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /dismiss spotify/i })).toBeInTheDocument();
  expect(screen.queryByText(/looks like/i)).toBeNull();
});

