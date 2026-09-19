import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type {
  AccountViewDto,
  MoneyInboxItemDto,
  TransactionRowDto,
} from "@/bindings";
import { MoneyInboxView } from "./MoneyInboxView";

const mocks = vi.hoisted(() => ({
  moneyInboxList: vi.fn(),
  importStagedAnyway: vi.fn(),
  skipStagedTransaction: vi.fn(),
  snoozeInboxItem: vi.fn(),
  dismissInboxItem: vi.fn(),
  accountList: vi.fn(),
  importBatch: vi.fn(),
  importPreviewColumns: vi.fn(),
  listSourcePresets: vi.fn(),
  archiveAccount: vi.fn(),
  assertBalance: vi.fn(),
  markReviewed: vi.fn(),
  markInboxReviewedBulk: vi.fn(),
  transactionRowsByIds: vi.fn(),
  transactionAttachments: vi.fn(),
  duplicateCandidates: vi.fn(),
  categoryList: vi.fn(),
  transactionList: vi.fn(),
  recategorizeTransaction: vi.fn(),
  acceptLowConfidenceCategories: vi.fn(),
  voidTransaction: vi.fn(),
  connectorSync: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    moneyInboxList: mocks.moneyInboxList,
    importStagedAnyway: mocks.importStagedAnyway,
    skipStagedTransaction: mocks.skipStagedTransaction,
    snoozeInboxItem: mocks.snoozeInboxItem,
    dismissInboxItem: mocks.dismissInboxItem,
    accountList: mocks.accountList,
    importBatch: mocks.importBatch,
    importPreviewColumns: mocks.importPreviewColumns,
    listSourcePresets: mocks.listSourcePresets,
    archiveAccount: mocks.archiveAccount,
    assertBalance: mocks.assertBalance,
    markReviewed: mocks.markReviewed,
    markInboxReviewedBulk: mocks.markInboxReviewedBulk,
    transactionRowsByIds: mocks.transactionRowsByIds,
    transactionAttachments: mocks.transactionAttachments,
    duplicateCandidates: mocks.duplicateCandidates,
    categoryList: mocks.categoryList,
    transactionList: mocks.transactionList,
    recategorizeTransaction: mocks.recategorizeTransaction,
    acceptLowConfidenceCategories: mocks.acceptLowConfidenceCategories,
    voidTransaction: mocks.voidTransaction,
    connectorSync: mocks.connectorSync,
  },
}));

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

function item(over: Partial<MoneyInboxItemDto> = {}): MoneyInboxItemDto {
  return {
    item_id: "0190a000-0000-7000-8000-0000000000aa",
    item_kind: "imported_waiting_commit",
    target_table: "staged_transactions",
    target_id: "0190a000-0000-7000-8000-0000000000aa",
    priority: 20,
    surfaced_at: "2026-06-20T00:00:00Z",
    snoozed_until: null,
    dismissed_at: null,
    resolved_at: null,
    payload_json: JSON.stringify({
      merchant: "Coffee",
      description: null,
      amount_minor: -1299,
      currency: "USD",
      posted_at: "2026-06-20",
      account_id: "0190a000-0000-7000-8000-000000000001",
      account_name: "Checking",
      source_name: "statement.csv",
      dedupe_reason: "duplicate of an already-committed transaction",
      suspected_committed_txn_id: null,
    }),
    ...over,
  };
}

function duplicateItem(): MoneyInboxItemDto {
  return item({
    payload_json: JSON.stringify({
      merchant: "Whole Foods Market",
      description: "CARD PURCHASE 1284",
      amount_minor: -4200,
      currency: "USD",
      posted_at: "2026-06-24",
      account_id: "0190a000-0000-7000-8000-000000000001",
      account_name: "Chase Checking",
      source_name: "chase_checking_2026-06.csv",
      dedupe_reason:
        "Same date, amount, and merchant as a transaction already in your ledger.",
      suspected_committed_txn_id: "0190e000-0000-7000-8000-0000000000f1",
    }),
  });
}

function counterpart(): TransactionRowDto {
  return {
    transaction_id: "0190e000-0000-7000-8000-0000000000f1",
    account_id: "0190a000-0000-7000-8000-000000000001",
    account_name: "Chase Checking",
    counter_account_id: null,
    counter_account_name: null,
    occurred_at: "2026-06-24T00:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
    amount: { minor_units: -4200, currency: "USD" },
    memo: "Whole Foods Market",
    counterparty: null,
    category_id: null,
    reviewed: true,
    note: null,
    tag_ids: [],
    split_count: 0,
    category_source: null,
    category_confidence_bps: null,
  };
}

function connectorErrorItem(): MoneyInboxItemDto {
  return item({
    item_id: "0190b000-0000-7000-8000-00000000000c",
    item_kind: "connector_error",
    target_table: "connector_connections",
    target_id: "0190b000-0000-7000-8000-00000000000c",
    priority: 30,
    surfaced_at: "2026-08-22T10:00:00Z",
    payload_json: JSON.stringify({
      connection_id: "0190b000-0000-7000-8000-00000000000c",
      adapter_id: "simplefin",
      display_hint: "SimpleFIN Bridge connection",
      last_synced_at: "2026-08-21T09:00:00Z",
      last_error: "access revoked or credentials no longer valid",
    }),
  });
}

function staleItem(): MoneyInboxItemDto {
  return item({
    item_id: "0190a000-0000-7000-8000-000000000001",
    item_kind: "stale_balance",
    target_table: "accounts",
    target_id: "0190a000-0000-7000-8000-000000000001",
    priority: 40,
    surfaced_at: "2026-05-01",
    payload_json: JSON.stringify({
      account_id: "0190a000-0000-7000-8000-000000000001",
      account_name: "Checking",
      role: "liquid_cash",
      last_observed: "2026-05-01",
      days_stale: 57,
    }),
  });
}

function unreviewedItem(): MoneyInboxItemDto {
  return item({
    item_id: "0190b000-0000-7000-8000-0000000000c1",
    item_kind: "unreviewed_transaction",
    target_table: "ledger_transactions",
    target_id: "0190b000-0000-7000-8000-0000000000c1",
    priority: 60,
    surfaced_at: "2026-06-22T00:00:00Z",
    payload_json: JSON.stringify({
      memo: "Trader Joe's",
      counterparty: null,
      amount_minor: -3200,
      currency: "USD",
      occurred_at: "2026-06-22",
      transaction_date: null,
      balance_after_minor: null,
      account_id: "0190a000-0000-7000-8000-000000000001",
      account_name: "Checking",
    }),
  });
}

const GROCERIES = {
  id: "0190c000-0000-7000-8000-000000000001",
  parent_id: null,
  name: "Groceries",
  category_type: "expense",
  icon: null,
  color: null,
  is_system: true,
  forecast_behavior: "variable_regular",
  archived: false,
} as const;

const DINING = { ...GROCERIES, id: "0190c000-0000-7000-8000-000000000002", name: "Dining out" } as const;

function lowConfidenceItem(over: Partial<MoneyInboxItemDto> = {}): MoneyInboxItemDto {
  return item({
    item_id: "0190b000-0000-7000-8000-0000000000d1",
    item_kind: "low_confidence_category",
    target_table: "ledger_transactions",
    target_id: "0190b000-0000-7000-8000-0000000000d1",
    priority: 50,
    surfaced_at: "2026-06-23T00:00:00Z",
    payload_json: JSON.stringify({
      memo: "Shopmart",
      counterparty: "shopmart",
      amount_minor: -2200,
      currency: "USD",
      occurred_at: "2026-06-23",
      transaction_date: null,
      balance_after_minor: null,
      account_name: "Checking",
      category_id: GROCERIES.id,
      confidence_bps: 6666,
    }),
    ...over,
  });
}

/// Open a row's inline detail expander. After the H4b compact-row refactor the
/// kind-specific secondary actions (skip / snooze / dismiss / archive / recategorize)
/// and the kind banner live behind the row's "Show details" chevron. Single-item tests
/// have exactly one chevron, so an unscoped query is unambiguous.
async function openRowDetails() {
  fireEvent.click(await screen.findByRole("button", { name: /show details/i }));
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.importStagedAnyway.mockResolvedValue(mutationOk());
  mocks.skipStagedTransaction.mockResolvedValue(mutationOk());
  mocks.snoozeInboxItem.mockResolvedValue(mutationOk());
  mocks.dismissInboxItem.mockResolvedValue(mutationOk());
  mocks.archiveAccount.mockResolvedValue(mutationOk());
  mocks.markReviewed.mockResolvedValue(mutationOk());
  mocks.duplicateCandidates.mockResolvedValue(ok([]));
  mocks.categoryList.mockResolvedValue(ok([]));
  mocks.transactionList.mockResolvedValue(ok([]));
  mocks.recategorizeTransaction.mockResolvedValue(mutationOk());
  mocks.acceptLowConfidenceCategories.mockResolvedValue(ok(0));
  mocks.voidTransaction.mockResolvedValue(mutationOk());
  // No accounts by default → the inbox-item tests don't show the Import button.
  mocks.accountList.mockResolvedValue(ok([]));
  mocks.importPreviewColumns.mockResolvedValue(ok([]));
  mocks.listSourcePresets.mockResolvedValue([]);
});

it("renders a flagged item with its detail, reason, and actions", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([item()]));
  renderWithClient(<MoneyInboxView />);

  expect(await screen.findByText("Coffee")).toBeInTheDocument();
  expect(screen.getByText("-$12.99")).toBeInTheDocument();
  // The primary action stays on the always-visible row.
  expect(
    screen.getByRole("button", { name: /import anyway/i }),
  ).toBeInTheDocument();
  // The dedupe reason, source, and secondary Skip live in the inline expander.
  await openRowDetails();
  expect(
    screen.getByText(/duplicate of an already-committed transaction/i),
  ).toBeInTheDocument();
  expect(screen.getByText(/from statement\.csv/i)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /skip/i })).toBeInTheDocument();
});

it("shows an explicit empty state when nothing needs attention", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([]));
  renderWithClient(<MoneyInboxView />);

  expect(await screen.findByText(/caught up/i)).toBeInTheDocument();
});

it("import anyway resolves the item via the staged transaction id", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([item()]));
  renderWithClient(<MoneyInboxView />);

  fireEvent.click(await screen.findByRole("button", { name: /import anyway/i }));

  await waitFor(() =>
    expect(mocks.importStagedAnyway).toHaveBeenCalledWith(
      "0190a000-0000-7000-8000-0000000000aa",
      expect.stringMatching(/.+/),
    ),
  );
  expect(mocks.skipStagedTransaction).not.toHaveBeenCalled();
});

it("surfaces a failed Import anyway even with the row expander collapsed (4d8.24.12 review)", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([item()]));
  // The primary action fails; the error must show without opening the expander.
  mocks.importStagedAnyway.mockResolvedValue({
    status: "error",
    error: { kind: "Validation", message: "Import failed" },
  } as unknown as ReturnType<typeof mutationOk>);
  renderWithClient(<MoneyInboxView />);

  fireEvent.click(await screen.findByRole("button", { name: /import anyway/i }));

  // The expander is untouched (collapsed by default) — the alert still appears.
  expect(await screen.findByRole("alert")).toBeInTheDocument();
});

it("skip resolves the item via the staged transaction id", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([item()]));
  renderWithClient(<MoneyInboxView />);

  // Skip is now a secondary action in the row's expander.
  await openRowDetails();
  fireEvent.click(screen.getByRole("button", { name: /skip/i }));

  await waitFor(() =>
    expect(mocks.skipStagedTransaction).toHaveBeenCalledWith(
      "0190a000-0000-7000-8000-0000000000aa",
      expect.stringMatching(/.+/),
    ),
  );
  expect(mocks.importStagedAnyway).not.toHaveBeenCalled();
});

it("offers an Import action that opens the import dialog when accounts exist", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([]));
  mocks.accountList.mockResolvedValue(ok([account()]));
  renderWithClient(<MoneyInboxView />);

  fireEvent.click(await screen.findByRole("button", { name: /import file/i }));

  expect(
    await screen.findByRole("dialog", { name: /import file/i }),
  ).toBeInTheDocument();
});

it("renders a stale-balance nudge (r52x)", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([staleItem()]));
  renderWithClient(<MoneyInboxView />);

  expect(await screen.findByText("Checking")).toBeInTheDocument();
  await openRowDetails();
  expect(screen.getByText(/balance may be out of date/i)).toBeInTheDocument();
  expect(screen.getByText(/57 days ago/)).toBeInTheDocument();
  // Not the import card.
  expect(
    screen.queryByRole("button", { name: /import anyway/i }),
  ).not.toBeInTheDocument();
});

it("archives the account from a stale-balance item (r52x)", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([staleItem()]));
  renderWithClient(<MoneyInboxView />);

  await openRowDetails();
  fireEvent.click(screen.getByRole("button", { name: /archive account/i }));

  await waitFor(() =>
    expect(mocks.archiveAccount).toHaveBeenCalledWith(
      "0190a000-0000-7000-8000-000000000001",
      expect.stringMatching(/.+/),
    ),
  );
});

it("snoozes an imported item for a week (ci71)", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([item()]));
  renderWithClient(<MoneyInboxView />);

  await openRowDetails();
  fireEvent.click(screen.getByRole("button", { name: /snooze 1 week/i }));

  await waitFor(() => expect(mocks.snoozeInboxItem).toHaveBeenCalledTimes(1));
  const [itemId, until] = mocks.snoozeInboxItem.mock.calls[0] ?? [];
  expect(itemId).toBe("0190a000-0000-7000-8000-0000000000aa");
  expect(until).toMatch(/^\d{4}-\d{2}-\d{2}$/); // a YYYY-MM-DD snooze date
});

it("dismisses an imported item with a chosen reason (ci71)", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([item()]));
  renderWithClient(<MoneyInboxView />);

  await openRowDetails();
  fireEvent.click(screen.getByRole("button", { name: /^dismiss$/i }));
  fireEvent.change(screen.getByLabelText(/dismiss reason/i), {
    target: { value: "incorrect" },
  });
  fireEvent.click(screen.getByRole("button", { name: /confirm/i }));

  await waitFor(() =>
    expect(mocks.dismissInboxItem).toHaveBeenCalledWith(
      "0190a000-0000-7000-8000-0000000000aa",
      "incorrect",
      expect.stringMatching(/.+/),
    ),
  );
});

it("surfaces an unreviewed transaction and marks it reviewed (4d8.7)", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([unreviewedItem()]));
  renderWithClient(<MoneyInboxView />);

  expect(await screen.findByText("Trader Joe's")).toBeInTheDocument();
  expect(screen.getByText("-$32.00")).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: /mark reviewed/i }));
  await waitFor(() =>
    expect(mocks.markReviewed).toHaveBeenCalledWith(
      "0190b000-0000-7000-8000-0000000000c1",
      true,
      expect.stringMatching(/.+/),
    ),
  );
});

it("surfaces a low-confidence categorization with its suggested category and accepts it (j5ij)", async () => {
  mocks.categoryList.mockResolvedValue(ok([GROCERIES, DINING]));
  mocks.moneyInboxList.mockResolvedValue(ok([lowConfidenceItem()]));
  renderWithClient(<MoneyInboxView />);

  expect(await screen.findByText("Shopmart")).toBeInTheDocument();
  // The suggested category + rounded confidence are shown in the info line; the picker
  // also defaults to the suggestion. Both live in the row's expander now.
  await openRowDetails();
  expect(screen.getByText(/Auto-categorized as/)).toBeInTheDocument();
  expect(screen.getByText(/67% confidence/)).toBeInTheDocument();
  // The combobox trigger shows the suggested category's leaf name.
  expect(
    screen.getByRole("button", { name: "Change category" }),
  ).toHaveTextContent("Groceries");

  // Accept = mark the transaction reviewed (keeps source=rule). Primary, on the row.
  fireEvent.click(screen.getByRole("button", { name: /^accept$/i }));
  await waitFor(() =>
    expect(mocks.markReviewed).toHaveBeenCalledWith(
      "0190b000-0000-7000-8000-0000000000d1",
      true,
      expect.stringMatching(/.+/),
    ),
  );
});

it("changes a low-confidence category from the picker (j5ij)", async () => {
  mocks.categoryList.mockResolvedValue(ok([GROCERIES, DINING]));
  mocks.moneyInboxList.mockResolvedValue(ok([lowConfidenceItem()]));
  renderWithClient(<MoneyInboxView />);

  await screen.findByText("Shopmart");
  await openRowDetails();
  fireEvent.click(screen.getByRole("button", { name: "Change category" }));
  fireEvent.pointerDown(screen.getByRole("option", { name: /Dining/ }));
  await waitFor(() =>
    expect(mocks.recategorizeTransaction).toHaveBeenCalledWith(
      "0190b000-0000-7000-8000-0000000000d1",
      DINING.id,
      expect.stringMatching(/.+/),
    ),
  );
});

it("bulk-accepts the low-confidence queue after confirmation (j5ij)", async () => {
  mocks.categoryList.mockResolvedValue(ok([GROCERIES]));
  mocks.acceptLowConfidenceCategories.mockResolvedValue(ok(2));
  mocks.moneyInboxList.mockResolvedValue(
    ok([
      lowConfidenceItem(),
      lowConfidenceItem({
        item_id: "0190b000-0000-7000-8000-0000000000d2",
        target_id: "0190b000-0000-7000-8000-0000000000d2",
      }),
    ]),
  );
  renderWithClient(<MoneyInboxView />);

  // The bulk banner appears for two+ low-confidence items.
  expect(
    await screen.findByText(/2 transactions were auto-categorized/i),
  ).toBeInTheDocument();

  // Two-step confirm: the first click reveals Confirm, which fires the bulk command.
  fireEvent.click(screen.getByRole("button", { name: /^accept all$/i }));
  fireEvent.click(screen.getByRole("button", { name: /accept all 2/i }));
  await waitFor(() =>
    // The key is minted per action (personal-cfo-3fdd.5) — non-empty, not "".
    expect(mocks.acceptLowConfidenceCategories).toHaveBeenCalledWith(
      expect.stringMatching(/.+/),
    ),
  );
});

it("reviews a duplicate side-by-side and skips it from the panel (4d8.8)", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([duplicateItem()]));
  mocks.duplicateCandidates.mockResolvedValue(ok([counterpart()]));
  renderWithClient(<MoneyInboxView />);

  // The card leads with Review (a committed counterpart exists).
  fireEvent.click(await screen.findByRole("button", { name: /^review$/i }));

  // The panel opens with the incoming vs the ledger counterpart side by side.
  const dialog = await screen.findByRole("dialog", {
    name: /duplicate review/i,
  });
  // Wait for the counterpart to load, then both columns show the merchant.
  await waitFor(() =>
    expect(within(dialog).getAllByText("Whole Foods Market")).toHaveLength(2),
  );

  // Skip routes the incoming through the existing skip command.
  fireEvent.click(
    within(dialog).getByRole("button", { name: /it's a duplicate/i }),
  );
  await waitFor(() =>
    expect(mocks.skipStagedTransaction).toHaveBeenCalledWith(
      "0190a000-0000-7000-8000-0000000000aa",
      expect.stringMatching(/.+/),
    ),
  );
});

it("voids the committed counterpart from the panel (4d8.21)", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([duplicateItem()]));
  mocks.duplicateCandidates.mockResolvedValue(ok([counterpart()]));
  renderWithClient(<MoneyInboxView />);

  fireEvent.click(await screen.findByRole("button", { name: /^review$/i }));
  const dialog = await screen.findByRole("dialog", {
    name: /duplicate review/i,
  });

  // Void the existing ledger entry — a two-step confirm, never the incoming.
  fireEvent.click(
    await within(dialog).findByRole("button", { name: /void this entry/i }),
  );
  fireEvent.click(within(dialog).getByRole("button", { name: /confirm void/i }));
  await waitFor(() =>
    expect(mocks.voidTransaction).toHaveBeenCalledWith(
      "0190e000-0000-7000-8000-0000000000f1",
      expect.stringMatching(/.+/),
    ),
  );
});

it("collapses and expands the section from the header toggle (4d8.24.12)", async () => {
  mocks.moneyInboxList.mockResolvedValue(ok([item()]));
  renderWithClient(<MoneyInboxView />);

  // The list row is visible and the header reports the open state.
  expect(await screen.findByText("Coffee")).toBeInTheDocument();
  const toggle = screen.getByRole("button", { name: /money inbox/i });
  expect(toggle).toHaveAttribute("aria-expanded", "true");

  // Collapsing hides the list but keeps the header + count badge as the at-a-glance signal.
  fireEvent.click(toggle);
  expect(screen.queryByText("Coffee")).toBeNull();
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  expect(screen.getByText("1")).toBeInTheDocument();

  // Expanding brings the list back.
  fireEvent.click(toggle);
  expect(await screen.findByText("Coffee")).toBeInTheDocument();
  expect(toggle).toHaveAttribute("aria-expanded", "true");
});

// ----- personal-cfo-4d8.25.15 / .17: beyond-window rows + Card Review -----

function txnRow(id: string, memo: string): TransactionRowDto {
  return {
    transaction_id: id,
    account_id: "0190a000-0000-7000-8000-000000000001",
    account_name: "Checking",
    counter_account_id: null,
    counter_account_name: null,
    occurred_at: "2026-06-22T00:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
    amount: { minor_units: -3_200, currency: "USD" },
    memo,
    counterparty: null,
    category_id: null,
    reviewed: false,
    note: null,
    tag_ids: [],
    split_count: 0,
    category_source: null,
    category_confidence_bps: null,
  };
}

it("resolves a row beyond the recent-transaction window by id, so its details open (4d8.25.15)", async () => {
  const beyond = unreviewedItem();
  // The recent-window list does NOT contain the item's transaction…
  mocks.transactionList.mockResolvedValue(ok([]));
  mocks.moneyInboxList.mockResolvedValue(ok([beyond]));
  // …but the by-ids read resolves it.
  mocks.transactionRowsByIds.mockResolvedValue(
    ok([txnRow(beyond.target_id, "Trader Joe's")]),
  );
  renderWithClient(<MoneyInboxView />);

  expect(await screen.findByText("Trader Joe's")).toBeInTheDocument();
  await waitFor(() =>
    expect(mocks.transactionRowsByIds).toHaveBeenCalledWith([beyond.target_id]),
  );
});

it("opens Card Review over the whole inbox and reviews the first card (4d8.25.17)", async () => {
  const first = unreviewedItem();
  mocks.transactionList.mockResolvedValue(ok([]));
  mocks.moneyInboxList.mockResolvedValue(ok([first]));
  mocks.transactionRowsByIds.mockResolvedValue(
    ok([txnRow(first.target_id, "Trader Joe's")]),
  );
  mocks.transactionAttachments.mockResolvedValue(ok([]));
  mocks.markReviewed.mockResolvedValue(mutationOk());
  renderWithClient(<MoneyInboxView />);

  fireEvent.click(await screen.findByRole("button", { name: /card review/i }));
  expect(await screen.findByText("1 of 1 to review")).toBeInTheDocument();

  fireEvent.keyDown(window, { key: "ArrowRight" });
  await waitFor(() =>
    expect(mocks.markReviewed).toHaveBeenCalledWith(
      first.target_id,
      true,
      expect.stringMatching(/.+/),
    ),
  );
  expect(await screen.findByText("Inbox reviewed")).toBeInTheDocument();
});

describe("connector_error items", () => {
  it("renders the connection problem, not the import-duplicate card", async () => {
    mocks.moneyInboxList.mockResolvedValue(ok([connectorErrorItem()]));
    const view = renderWithClient(<MoneyInboxView />);
    expect(
      await view.findByText("SimpleFIN Bridge connection could not refresh"),
    ).toBeInTheDocument();
    expect(view.getByText("Retry refresh")).toBeInTheDocument();
    // The fallthrough guard: no import actions on a connection problem.
    expect(view.queryByText("Import anyway")).not.toBeInTheDocument();
    expect(view.queryByText("Skip")).not.toBeInTheDocument();
  });

  it("retries the refresh against the connection id", async () => {
    mocks.moneyInboxList.mockResolvedValue(ok([connectorErrorItem()]));
    mocks.connectorSync.mockResolvedValue(
      ok({
        connection_id: "0190b000-0000-7000-8000-00000000000c",
        status: "synced",
        staged: 0,
        committed: 0,
        flagged: 0,
        skipped_unmapped: 0,
        warnings: [],
        message: null,
      }),
    );
    const view = renderWithClient(<MoneyInboxView />);
    fireEvent.click(await view.findByText("Retry refresh"));
    await waitFor(() =>
      expect(mocks.connectorSync).toHaveBeenCalledWith(
        expect.objectContaining({
          connection_id: "0190b000-0000-7000-8000-00000000000c",
        }),
      ),
    );
  });
});
