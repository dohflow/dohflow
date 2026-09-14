import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type {
  AccountViewDto,
  RecurringBillDto,
  TransactionRowDto,
} from "@/bindings";
import { BillsView } from "./BillsView";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  recurringBillList: vi.fn(),
  createRecurringBill: vi.fn(),
  recurringBillHistory: vi.fn(),
  updateRecurringBill: vi.fn(),
  deleteRecurringBill: vi.fn(),
  archiveRecurringBill: vi.fn(),
  restoreRecurringBill: vi.fn(),
  baseCurrency: vi.fn(),
  setBaseCurrency: vi.fn(),
  transactionPage: vi.fn(),
  categoryList: vi.fn(),
  tagList: vi.fn(),
  forecastAssumptionList: vi.fn(),
  scenarioList: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    accountList: mocks.accountList,
    recurringBillList: mocks.recurringBillList,
    forecastAssumptionList: mocks.forecastAssumptionList,
    scenarioList: mocks.scenarioList,
    createRecurringBill: mocks.createRecurringBill,
    recurringBillHistory: mocks.recurringBillHistory,
    updateRecurringBill: mocks.updateRecurringBill,
    deleteRecurringBill: mocks.deleteRecurringBill,
    archiveRecurringBill: mocks.archiveRecurringBill,
    restoreRecurringBill: mocks.restoreRecurringBill,
    baseCurrency: mocks.baseCurrency,
    setBaseCurrency: mocks.setBaseCurrency,
    transactionPage: mocks.transactionPage,
    categoryList: mocks.categoryList,
    tagList: mocks.tagList,
  },
}));

function txn(over: Partial<TransactionRowDto> = {}): TransactionRowDto {
  return {
    transaction_id: "0190b000-0000-7000-8000-0000000000f1",
    account_id: "0190a000-0000-7000-8000-000000000001",
    account_name: "Checking",
    counter_account_id: null,
    counter_account_name: null,
    occurred_at: "2026-07-03T00:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
    amount: { minor_units: -1599, currency: "USD" },
    memo: null,
    counterparty: "Netflix",
    category_id: null,
    reviewed: false,
    note: null,
    tag_ids: [],
    split_count: 0,
    category_source: null,
    category_confidence_bps: null,
    ...over,
  };
}

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const mutationOk = () => ok({ op_seq: 1, replayed: false });

function account(over: Partial<AccountViewDto> = {}): AccountViewDto {
  return {
    id: "0190a000-0000-7000-8000-000000000001",
    name: "Checking",
    cashflow_role: "liquid_cash",
    subtype: null,
    active: true,
    balance: { minor_units: 125_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  };
}

function bill(over: Partial<RecurringBillDto> = {}): RecurringBillDto {
  return {
    id: "0190d000-0000-7000-8000-000000000001",
    name: "Netflix",
    bill_type: "subscription",
    amount: { minor_units: 1599, currency: "USD" },
    frequency: "monthly",
    anchor_date: "2026-07-01",
    autopay_account_id: null,
    autopay_account_name: null,
    autopay_enabled: false,
    next_due_date: "2026-07-01",
    description: null,
    active: true,
    created_at: "2026-06-01T00:00:00Z",
    archived_at: null,
    category_id: null,
    tag_ids: [],
    ...over,
  };
}

beforeEach(() => {
  mocks.accountList.mockReset();
  mocks.recurringBillList.mockReset();
  mocks.createRecurringBill.mockReset();
  mocks.updateRecurringBill.mockReset();
  mocks.deleteRecurringBill.mockReset();
  mocks.archiveRecurringBill.mockReset();
  mocks.restoreRecurringBill.mockReset();
  mocks.baseCurrency.mockReset();
  mocks.setBaseCurrency.mockReset();
  mocks.accountList.mockResolvedValue(ok([account()]));
  mocks.recurringBillList.mockResolvedValue(ok([]));
  mocks.archiveRecurringBill.mockResolvedValue(mutationOk());
  mocks.recurringBillHistory.mockResolvedValue(ok([]));
  mocks.restoreRecurringBill.mockResolvedValue(mutationOk());
  mocks.forecastAssumptionList.mockReset();
  mocks.forecastAssumptionList.mockResolvedValue(ok([]));
  mocks.scenarioList.mockReset();
  mocks.scenarioList.mockResolvedValue(ok([]));
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.setBaseCurrency.mockResolvedValue(ok(null));
  // Drawer dependencies (personal-cfo-4d8.24.7.2). A bare [] would break the paged
  // read — it expects a { rows, total } page.
  mocks.transactionPage.mockReset().mockResolvedValue(ok({ rows: [], total: 0 }));
  mocks.categoryList.mockReset().mockResolvedValue(ok([]));
  mocks.tagList.mockReset().mockResolvedValue(ok([]));
});

describe("BillsView", () => {
  it("shows the empty state when there are no bills", async () => {
    renderWithClient(<BillsView />);
    expect(await screen.findByText(/no bills yet/i)).toBeInTheDocument();
  });

  it("lists a bill with its type label, next due date, amount, and description", async () => {
    mocks.recurringBillList.mockResolvedValue(
      ok([bill({ description: "Family plan" })]),
    );
    renderWithClient(<BillsView />);
    expect(await screen.findByText("Netflix")).toBeInTheDocument();
    // The subline concatenates the type + frequency labels and the next due date;
    // the plain `YYYY-MM-DD` renders as a local date (no off-by-one shift).
    expect(screen.getByText(/Subscription/)).toBeInTheDocument();
    expect(screen.getByText(/Jul 1, 2026/)).toBeInTheDocument();
    expect(screen.getByText(/\$15\.99/)).toBeInTheDocument();
    expect(screen.getByText("Family plan")).toBeInTheDocument();
  });

  it("creates a bill on a custom interval, resolving the every_N token (ADR 0048)", async () => {
    mocks.accountList.mockResolvedValue(ok([]));
    mocks.recurringBillList.mockResolvedValue(ok([]));
    mocks.createRecurringBill.mockResolvedValue(
      ok({ op_seq: 1, replayed: false, event_id: "0190e000-0000-7000-8000-0000000000e2" }),
    );
    renderWithClient(<BillsView />);

    fireEvent.click(await screen.findByRole("button", { name: /add bill/i }));
    fireEvent.change(screen.getByLabelText(/bill name/i), {
      target: { value: "Lawn service" },
    });
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "90" },
    });
    fireEvent.change(screen.getByLabelText(/frequency/i), {
      target: { value: "custom" },
    });
    fireEvent.change(screen.getByLabelText(/interval count/i), {
      target: { value: "6" },
    });
    fireEvent.change(screen.getByLabelText(/interval unit/i), {
      target: { value: "weeks" },
    });
    fireEvent.change(screen.getByLabelText(/anchor date/i), {
      target: { value: "2026-07-14" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add bill/i }));

    await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
    expect(mocks.createRecurringBill.mock.calls[0]?.[0].frequency).toBe("every_6_weeks");
  });

  it("rejects an out-of-bounds custom interval before any IPC call (ADR 0048 s3)", async () => {
    mocks.accountList.mockResolvedValue(ok([]));
    mocks.recurringBillList.mockResolvedValue(ok([]));
    renderWithClient(<BillsView />);

    fireEvent.click(await screen.findByRole("button", { name: /add bill/i }));
    fireEvent.change(screen.getByLabelText(/bill name/i), {
      target: { value: "Lawn service" },
    });
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "90" },
    });
    fireEvent.change(screen.getByLabelText(/frequency/i), {
      target: { value: "custom" },
    });
    fireEvent.change(screen.getByLabelText(/interval count/i), {
      target: { value: "53" },
    });
    fireEvent.change(screen.getByLabelText(/interval unit/i), {
      target: { value: "weeks" },
    });
    fireEvent.change(screen.getByLabelText(/anchor date/i), {
      target: { value: "2026-07-14" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add bill/i }));

    expect(
      await screen.findByText(/between 1 and 52/i),
    ).toBeInTheDocument();
    expect(mocks.createRecurringBill).not.toHaveBeenCalled();
  });

  it("creates a bill with the entered fields and refreshes", async () => {
    mocks.accountList.mockResolvedValue(ok([])); // no autopay account available
    mocks.recurringBillList
      .mockResolvedValueOnce(ok([]))
      .mockResolvedValue(ok([bill({ name: "Rent", bill_type: "rent_mortgage" })]));
    mocks.createRecurringBill.mockResolvedValue(
      ok({ op_seq: 1, replayed: false, event_id: "0190e000-0000-7000-8000-0000000000e1" }),
    );
    renderWithClient(<BillsView />);

    fireEvent.click(await screen.findByRole("button", { name: /add bill/i }));
    fireEvent.change(screen.getByLabelText(/bill name/i), {
      target: { value: "Rent" },
    });
    fireEvent.change(screen.getByLabelText(/type/i), {
      target: { value: "rent_mortgage" },
    });
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "1800" },
    });
    fireEvent.change(screen.getByLabelText(/frequency/i), {
      target: { value: "monthly" },
    });
    fireEvent.change(screen.getByLabelText(/anchor date/i), {
      target: { value: "2026-07-01" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add bill/i }));

    await screen.findByText("Rent");
    expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1);
    const input = mocks.createRecurringBill.mock.calls[0]?.[0];
    expect(input.name).toBe("Rent");
    expect(input.bill_type).toBe("rent_mortgage");
    expect(input.frequency).toBe("monthly");
    expect(input.anchor_date).toBe("2026-07-01");
    expect(input.amount.minor_units).toBe(180_000); // 1800.00 → minor units
    expect(input.amount.currency).toBe("USD"); // default without an account
    expect(input.autopay_account_id).toBeNull();
    expect(input.autopay).toBe(false); // autopay checkbox unchecked by default
    expect(input.description).toBeNull(); // blank description normalizes to null
  });

  it("marks a new bill autopay via the checkbox", async () => {
    mocks.accountList.mockResolvedValue(ok([]));
    mocks.recurringBillList
      .mockResolvedValueOnce(ok([]))
      .mockResolvedValue(ok([bill({ name: "Mortgage", autopay_enabled: true })]));
    mocks.createRecurringBill.mockResolvedValue(
      ok({ op_seq: 1, replayed: false, event_id: "0190e000-0000-7000-8000-0000000000e1" }),
    );
    renderWithClient(<BillsView />);

    fireEvent.click(await screen.findByRole("button", { name: /add bill/i }));
    fireEvent.change(screen.getByLabelText(/bill name/i), {
      target: { value: "Mortgage" },
    });
    fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "2000" } });
    fireEvent.change(screen.getByLabelText(/anchor date/i), {
      target: { value: "2026-07-01" },
    });
    fireEvent.click(screen.getByLabelText(/pays itself/i));
    fireEvent.click(screen.getByRole("button", { name: /add bill/i }));

    await screen.findByText("Mortgage");
    expect(mocks.createRecurringBill.mock.calls[0]?.[0].autopay).toBe(true);
  });

  it("badges an autopay bill in the list", async () => {
    mocks.recurringBillList.mockResolvedValue(
      ok([bill({ name: "Mortgage", autopay_enabled: true })]),
    );
    renderWithClient(<BillsView />);
    await screen.findByText("Mortgage");
    expect(screen.getByText(/autopay/i)).toBeInTheDocument();
  });

  it("links an autopay account and derives its currency", async () => {
    const euro = account({
      id: "0190a000-0000-7000-8000-0000000000ee",
      name: "Euro Checking",
      balance: { minor_units: 0, currency: "EUR" },
    });
    mocks.accountList.mockResolvedValue(ok([euro]));
    mocks.createRecurringBill.mockResolvedValue(
      ok({ op_seq: 1, replayed: false, event_id: "0190e000-0000-7000-8000-0000000000e1" }),
    );
    renderWithClient(<BillsView />);

    fireEvent.click(await screen.findByRole("button", { name: /add bill/i }));
    // Wait for the account list to populate the autopay dropdown.
    await screen.findByRole("option", { name: "Euro Checking" });
    fireEvent.change(screen.getByLabelText(/bill name/i), {
      target: { value: "Euro Bill" },
    });
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "50" },
    });
    fireEvent.change(screen.getByLabelText(/autopay account/i), {
      target: { value: euro.id },
    });
    fireEvent.click(screen.getByRole("button", { name: /add bill/i }));

    await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalled());
    const input = mocks.createRecurringBill.mock.calls[0]?.[0];
    expect(input.autopay_account_id).toBe(euro.id);
    expect(input.amount.currency).toBe("EUR");
  });

  it("edits a bill from the detail drawer with the new fields including description", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    mocks.updateRecurringBill.mockResolvedValue(mutationOk());
    renderWithClient(<BillsView />);

    // Editing now lives in the detail drawer (personal-cfo-4d8.24.7.2), not inline.
    fireEvent.click(await screen.findByRole("button", { name: /open netflix detail/i }));
    const drawer = await screen.findByRole("dialog", { name: /bill detail/i });
    // The form prefills from the bill; change the name, amount, and description.
    fireEvent.change(within(drawer).getByLabelText(/bill name/i), {
      target: { value: "Netflix Premium" },
    });
    fireEvent.change(within(drawer).getByLabelText(/amount/i), {
      target: { value: "22.99" },
    });
    fireEvent.change(within(drawer).getByLabelText(/description/i), {
      target: { value: "4K family plan" },
    });
    fireEvent.click(within(drawer).getByRole("button", { name: /^save$/i }));

    await waitFor(() =>
      expect(mocks.updateRecurringBill).toHaveBeenCalledTimes(1),
    );
    const input = mocks.updateRecurringBill.mock.calls[0]?.[0];
    expect(input.bill_id).toBe(bill().id);
    expect(input.name).toBe("Netflix Premium");
    expect(input.amount.minor_units).toBe(2299);
    expect(input.description).toBe("4K family plan");
  });

  it("deletes a bill only after the inline confirm", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    mocks.deleteRecurringBill.mockResolvedValue(mutationOk());
    renderWithClient(<BillsView />);

    // The first Delete click only reveals the confirm — nothing is deleted yet.
    fireEvent.click(
      await screen.findByRole("button", { name: /^delete netflix$/i }),
    );
    expect(mocks.deleteRecurringBill).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("button", { name: /confirm delete netflix/i }),
    );
    await waitFor(() =>
      expect(mocks.deleteRecurringBill).toHaveBeenCalledTimes(1),
    );
    expect(mocks.deleteRecurringBill).toHaveBeenCalledWith(
      bill().id,
      expect.stringMatching(/.+/),
    );
  });

  it("archives an active bill", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    renderWithClient(<BillsView />);

    fireEvent.click(
      await screen.findByRole("button", { name: /archive netflix/i }),
    );
    await waitFor(() =>
      expect(mocks.archiveRecurringBill).toHaveBeenCalledWith(
        bill().id,
        expect.stringMatching(/.+/),
      ),
    );
  });

  it("searches bills by name or description and shows the no-match state (yequ)", async () => {
    mocks.recurringBillList.mockResolvedValue(
      ok([
        bill(),
        bill({
          id: "0190d000-0000-7000-8000-000000000002",
          name: "Spectrum",
          description: "Internet",
        }),
      ]),
    );
    renderWithClient(<BillsView />);
    await screen.findByText("Netflix");

    const search = screen.getByRole("searchbox", { name: /search bills/i });
    fireEvent.change(search, { target: { value: "spec" } });
    expect(screen.getByText("Spectrum")).toBeInTheDocument();
    expect(screen.queryByText("Netflix")).toBeNull();

    // The description is searchable too, not just the name.
    fireEvent.change(search, { target: { value: "internet" } });
    expect(screen.getByText("Spectrum")).toBeInTheDocument();

    fireEvent.change(search, { target: { value: "zzz" } });
    expect(screen.getByText(/no bills match/i)).toBeInTheDocument();
  });

  it("sorts by next due by default and re-sorts by name / amount (yequ)", async () => {
    mocks.recurringBillList.mockResolvedValue(
      ok([
        // Backend order is deliberately scrambled relative to every sort key.
        bill({
          id: "0190d000-0000-7000-8000-000000000011",
          name: "Aqua",
          amount: { minor_units: 5_000, currency: "USD" },
          next_due_date: "2026-07-20",
        }),
        bill({ next_due_date: "2026-07-15" }), // Netflix, $15.99
        bill({
          id: "0190d000-0000-7000-8000-000000000012",
          name: "Rent",
          amount: { minor_units: 180_000, currency: "USD" },
          next_due_date: "2026-07-01",
        }),
      ]),
    );
    renderWithClient(<BillsView />);
    await screen.findByText("Rent");

    const names = () =>
      screen.getAllByRole("listitem").map((item) => {
        const text = item.textContent ?? "";
        if (text.includes("Netflix")) return "Netflix";
        return text.includes("Aqua") ? "Aqua" : "Rent";
      });
    // Default: next due, soonest first.
    expect(names()).toEqual(["Rent", "Netflix", "Aqua"]);

    const sortSelect = screen.getByRole("combobox", { name: /sort bills/i });
    fireEvent.change(sortSelect, { target: { value: "name" } });
    expect(names()).toEqual(["Aqua", "Netflix", "Rent"]);

    fireEvent.change(sortSelect, { target: { value: "amount_desc" } });
    expect(names()).toEqual(["Rent", "Aqua", "Netflix"]);
  });

  it("lists an archived bill under Archived and restores it", async () => {
    mocks.recurringBillList.mockResolvedValue(
      ok([bill({ active: false, archived_at: "2026-06-20T00:00:00Z" })]),
    );
    renderWithClient(<BillsView />);

    // The bill lives in the Archived section (an active bill never offers Restore).
    expect(
      await screen.findByRole("heading", { name: /archived/i }),
    ).toBeInTheDocument();
    fireEvent.click(
      await screen.findByRole("button", { name: /restore netflix/i }),
    );
    await waitFor(() =>
      expect(mocks.restoreRecurringBill).toHaveBeenCalledWith(
        bill().id,
        expect.stringMatching(/.+/),
      ),
    );
  });

  // Bills side-panel (personal-cfo-4d8.24.7.2).
  it("opens a detail drawer with the bill's edit form when a bill is clicked", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    renderWithClient(<BillsView />);

    fireEvent.click(await screen.findByRole("button", { name: /open netflix detail/i }));
    const drawer = await screen.findByRole("dialog", { name: /bill detail/i });
    // The edit form is pre-filled with the bill's fields.
    expect(
      within(drawer).getByLabelText<HTMLInputElement>(/bill name/i).value,
    ).toBe("Netflix");
    expect(
      within(drawer).getByLabelText<HTMLInputElement>(/amount/i).value,
    ).toBe("15.99");
  });

  it("shows the bill's confirmed payments in the drawer, scoped by recurring_event_id", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    mocks.transactionPage.mockResolvedValue(
      ok({ rows: [txn({ counterparty: "Netflix" })], total: 1 }),
    );
    renderWithClient(<BillsView />);

    fireEvent.click(await screen.findByRole("button", { name: /open netflix detail/i }));
    const drawer = await screen.findByRole("dialog", { name: /bill detail/i });
    // The payment renders inside the drawer…
    expect(await within(drawer).findByText("-$15.99")).toBeInTheDocument();
    // …and the query is scoped to THIS bill (server-side confirmed_obligations filter).
    await waitFor(() =>
      expect(mocks.transactionPage).toHaveBeenCalledWith(
        expect.objectContaining({ recurring_event_id: bill().id }),
      ),
    );
  });

  it("shows the bill's matched history in the drawer (ADR 0047 s1)", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    mocks.recurringBillHistory.mockResolvedValue(
      ok([
        {
          scheduled_date: "2026-05-12",
          expected_amount_minor: 1_599,
          currency: "USD",
          status: "paid",
          linked_transaction_id: "0190f000-0000-7000-8000-0000000000f1",
        },
        {
          scheduled_date: "2026-06-12",
          expected_amount_minor: 1_599,
          currency: "USD",
          status: "paid",
          linked_transaction_id: "0190f000-0000-7000-8000-0000000000f2",
        },
        {
          scheduled_date: "2026-08-12",
          expected_amount_minor: 1_599,
          currency: "USD",
          status: "scheduled",
          linked_transaction_id: null,
        },
      ]),
    );
    renderWithClient(<BillsView />);

    fireEvent.click(await screen.findByRole("button", { name: /open netflix detail/i }));
    const drawer = await screen.findByRole("dialog", { name: /bill detail/i });
    // Only the LINKED occurrences count toward matched history.
    expect(
      await within(drawer).findByText(/2 past transactions matched/i),
    ).toBeInTheDocument();
    expect(mocks.recurringBillHistory).toHaveBeenCalledWith(bill().id);
  });

  it("anchor field carries the shared ADR 0047 s3 help copy", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([]));
    renderWithClient(<BillsView />);
    fireEvent.click(await screen.findByRole("button", { name: /add bill/i }));
    expect(
      await screen.findByText(/the schedule counts from this date/i),
    ).toBeInTheDocument();
  });

  it("re-queries the payments when the drawer search box changes", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    renderWithClient(<BillsView />);

    fireEvent.click(await screen.findByRole("button", { name: /open netflix detail/i }));
    const drawer = await screen.findByRole("dialog", { name: /bill detail/i });
    fireEvent.change(
      within(drawer).getByRole("searchbox", { name: /search payments/i }),
      { target: { value: "refund" } },
    );
    await waitFor(() =>
      expect(mocks.transactionPage).toHaveBeenCalledWith(
        expect.objectContaining({
          recurring_event_id: bill().id,
          query: "refund",
        }),
      ),
    );
  });

});

describe("applied-scenario overrides (personal-cfo-abhr, ADR 0055)", () => {
  const BILL_ID = "0190d000-0000-7000-8000-000000000001";

  const promotedEvent = (over = {}) => ({
    id: "evt-promoted",
    kind: "bill_amount",
    target_entity_id: BILL_ID,
    scenario_id: null,
    promoted_from_scenario_id: "scn-1",
    params_json: JSON.stringify({ new_amount_minor: 250_000 }),
    created_at: "2026-08-02T00:00:00Z",
    ...over,
  });

  it("says which figure the forecast uses and which scenario put it there", async () => {
    // ADR 0055 applies a scenario by promoting events into base rather than editing the
    // bill, so the stored amount and the forecast's amount legitimately differ. Without
    // this note the app just looks self-contradictory.
    mocks.recurringBillList.mockResolvedValue(ok([bill({ id: BILL_ID })]));
    mocks.forecastAssumptionList.mockResolvedValue(ok([promotedEvent()]));
    mocks.scenarioList.mockResolvedValue(
      ok([
        {
          id: "scn-1",
          name: "Rent hike",
          description: null,
          status: "draft",
          created_at: "2026-08-01",
          updated_at: "2026-08-01",
          expires_on: null,
          event_count: 1,
          applied_at: "2026-08-02T00:00:00Z",
        },
      ]),
    );
    const onOpenScenario = vi.fn();
    renderWithClient(<BillsView onOpenScenario={onOpenScenario} />);

    expect(await screen.findByText(/your forecast uses/i)).toBeInTheDocument();
    expect(screen.getByText("$2,500.00")).toBeInTheDocument();
    // Naming the scenario is the point — and it is a route, not a dead end.
    fireEvent.click(screen.getByRole("button", { name: "Rent hike" }));
    expect(onOpenScenario).toHaveBeenCalledWith("scn-1");
  });

  it("says nothing for a bill no applied scenario touched", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill({ id: BILL_ID })]));
    mocks.forecastAssumptionList.mockResolvedValue(ok([]));
    renderWithClient(<BillsView onOpenScenario={vi.fn()} />);
    await screen.findByText("Netflix");
    expect(screen.queryByText(/your forecast uses/i)).not.toBeInTheDocument();
  });

  it("does not attribute an override the user typed to a scenario", async () => {
    // A base event with no promoted_from_scenario_id is the user's own edit. Claiming a
    // scenario did it would be a plain lie, and would send them to an unrelated screen.
    mocks.recurringBillList.mockResolvedValue(ok([bill({ id: BILL_ID })]));
    mocks.forecastAssumptionList.mockResolvedValue(
      ok([promotedEvent({ promoted_from_scenario_id: null })]),
    );
    renderWithClient(<BillsView onOpenScenario={vi.fn()} />);
    await screen.findByText("Netflix");
    expect(screen.queryByText(/your forecast uses/i)).not.toBeInTheDocument();
  });
});
