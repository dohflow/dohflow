import { fireEvent, screen } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { MoneyInboxItemDto, TransactionRowDto } from "@/bindings";
import { MoneyInboxView } from "@/money-inbox/MoneyInboxView";
import { TransactionBulkBar } from "./TransactionBulkBar";
import { useTransactionSelection } from "./useTransactionSelection";

const mocks = vi.hoisted(() => ({
  moneyInboxList: vi.fn(),
  transactionRowsByIds: vi.fn(),
  accountList: vi.fn(),
  categoryList: vi.fn(),
  tagList: vi.fn(),
  transactionAttachments: vi.fn(),
  duplicateCandidates: vi.fn(),
  markReviewed: vi.fn(),
  markInboxReviewedBulk: vi.fn(),
  recategorizeTransaction: vi.fn(),
  acceptLowConfidenceCategories: vi.fn(),
  voidTransaction: vi.fn(),
  importStagedAnyway: vi.fn(),
  skipStagedTransaction: vi.fn(),
  snoozeInboxItem: vi.fn(),
  dismissInboxItem: vi.fn(),
  importBatch: vi.fn(),
  archiveAccount: vi.fn(),
  assertBalance: vi.fn(),
  transactionList: vi.fn(),
  setTags: vi.fn(),
  createTag: vi.fn(),
}));

vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const TXN_ID = "0190b000-0000-7000-8000-0000000000c1";

function unreviewedItem(): MoneyInboxItemDto {
  return {
    item_id: TXN_ID,
    item_kind: "unreviewed_transaction",
    target_table: "ledger_transactions",
    target_id: TXN_ID,
    priority: 60,
    surfaced_at: "2026-06-22T00:00:00Z",
    snoozed_until: null,
    dismissed_at: null,
    resolved_at: null,
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
      counter_account_id: null,
      counter_account_name: null,
    }),
  };
}

function row(): TransactionRowDto {
  return {
    transaction_id: TXN_ID,
    account_id: "0190a000-0000-7000-8000-000000000001",
    account_name: "Checking",
    counter_account_id: null,
    counter_account_name: null,
    occurred_at: "2026-06-22T00:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
    amount: { minor_units: -3200, currency: "USD" },
    memo: "Trader Joe's",
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

/// The shell's Money Inbox surface: the view with a CONTROLLED selection plus the
/// extracted bulk bar — the composition ADR 0049 §3 replaced the dissolved hub with.
function MoneyInboxSurface() {
  const selection = useTransactionSelection();
  return (
    <>
      <MoneyInboxView selection={selection} />
      <TransactionBulkBar selection={selection} />
    </>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.moneyInboxList.mockResolvedValue(ok([unreviewedItem()]));
  mocks.transactionRowsByIds.mockResolvedValue(ok([row()]));
  mocks.accountList.mockResolvedValue(ok([]));
  mocks.categoryList.mockResolvedValue(ok([]));
  mocks.tagList.mockResolvedValue(ok([]));
  mocks.transactionAttachments.mockResolvedValue(ok([]));
  mocks.duplicateCandidates.mockResolvedValue(ok([]));
  mocks.markInboxReviewedBulk.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
});

describe("Money Inbox surface (ADR 0049 §3: controlled selection + extracted bulk bar)", () => {
  it("keeps the inbox's Select-all control, which only renders on the controlled path", async () => {
    renderWithClient(<MoneyInboxSurface />);
    // Gated on a selection being PASSED (personal-cfo-4d8.25.16). It disappears if the
    // shell ever stops threading one through — exactly the regression that dissolving
    // the hub could have caused, and which no test covered once the hub test was deleted.
    expect(
      await screen.findByRole("button", { name: /select all/i }),
    ).toBeInTheDocument();
  });

  it("renders the bulk bar over the surface's own selection", async () => {
    renderWithClient(<MoneyInboxSurface />);
    fireEvent.click(await screen.findByRole("button", { name: /select all/i }));
    // The bar is the extracted TransactionBulkBar — the hub used to host it.
    expect(await screen.findByText(/1 selected/i)).toBeInTheDocument();
  });
});
