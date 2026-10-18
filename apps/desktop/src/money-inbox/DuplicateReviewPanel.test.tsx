import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import type { CategoryDto, TransactionRowDto } from "@/bindings";
import { renderWithClient } from "@/test/renderWithClient";
import { DuplicateReviewPanel, type IncomingDuplicate } from "./DuplicateReviewPanel";

const mocks = vi.hoisted(() => ({
  duplicateCandidates: vi.fn(),
  categoryList: vi.fn(),
  transactionList: vi.fn(),
  recategorizeTransaction: vi.fn(),
  voidTransaction: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    duplicateCandidates: mocks.duplicateCandidates,
    categoryList: mocks.categoryList,
    transactionList: mocks.transactionList,
    recategorizeTransaction: mocks.recategorizeTransaction,
    voidTransaction: mocks.voidTransaction,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const fail = (message: string) =>
  ({ status: "error", error: { Validation: message } }) as const;
const mutationOk = () => ok({ op_seq: 1, replayed: false });

const STAGED_ID = "0190a000-0000-7000-8000-0000000000aa";

const incoming: IncomingDuplicate = {
  merchant: "Whole Foods Market",
  amount: { minor_units: -4200, currency: "USD" },
  date: "2026-06-24",
  account: "Chase Checking",
  source: "chase_checking_2026-06.csv",
};

function counterpart(over: Partial<TransactionRowDto> = {}): TransactionRowDto {
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
    ...over,
  };
}

const groceries: CategoryDto = {
  id: "0190c000-0000-7000-8000-000000000001",
  parent_id: null,
  name: "Groceries",
  category_type: "expense",
  icon: null,
  color: null,
  is_system: false,
  forecast_behavior: "variable_regular",
  archived: false,
};

function renderPanel(
  handlers: Partial<{
    onSkip: () => Promise<null>;
    onImportAnyway: () => Promise<unknown>;
    onClose: () => void;
  }> = {},
) {
  const props = {
    onSkip: vi.fn().mockResolvedValue(null),
    onImportAnyway: vi.fn().mockResolvedValue(null),
    onClose: vi.fn(),
    ...handlers,
  };
  renderWithClient(
    <DuplicateReviewPanel
      stagedTxnId={STAGED_ID}
      incoming={incoming}
      reason="Same date, amount, and merchant as a transaction already in your ledger."
      onSkip={props.onSkip}
      onImportAnyway={props.onImportAnyway as never}
      onClose={props.onClose}
    />,
  );
  return props;
}

const skipButton = () => screen.getByRole("button", { name: /it's a duplicate/i });
const importButton = () => screen.getByRole("button", { name: /it's different/i });
/// The panel promotes one action with the primary (filled) button style.
const isPrimary = (button: HTMLElement) => button.className.includes("bg-primary");

beforeEach(() => {
  // Reset, not clear: a queued one-shot value must never leak into the next test.
  vi.resetAllMocks();
  mocks.categoryList.mockResolvedValue(ok([groceries]));
  mocks.transactionList.mockResolvedValue(ok([]));
  mocks.recategorizeTransaction.mockResolvedValue(mutationOk());
  mocks.voidTransaction.mockResolvedValue(mutationOk());
});

it("shows a loading state and suggests nothing until the ledger answers", async () => {
  mocks.duplicateCandidates.mockReturnValue(new Promise(() => {}));
  renderPanel();
  expect(
    await screen.findByText(/loading the matching transaction/i),
  ).toBeInTheDocument();
  expect(screen.queryByText(/suggested|safe to import|close matches/i)).toBeNull();
});

it("a failed fetch is an error with Retry, not 'safe to import' (pxi.11)", async () => {
  mocks.duplicateCandidates
    .mockResolvedValueOnce(fail("The vault is busy."))
    .mockResolvedValueOnce(ok([counterpart()]));
  renderPanel();

  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent(/couldn't check your ledger for a match/i);
  expect(alert).toHaveTextContent("The vault is busy.");
  // No suggestion, no "no longer in your ledger" claim, nothing promoted.
  expect(screen.queryByText(/safe to import|no longer in your ledger/i)).toBeNull();
  expect(screen.queryByText(/suggested/i)).toBeNull();
  expect(isPrimary(skipButton())).toBe(false);
  expect(isPrimary(importButton())).toBe(false);
  // The incoming row is still there to look at.
  const card = screen.getByLabelText("Incoming transaction");
  expect(card).toHaveTextContent("Whole Foods Market");
  expect(card).toHaveTextContent("Chase Checking");

  // Retry refetches and the normal comparison takes over.
  fireEvent.click(within(alert).getByRole("button", { name: /retry/i }));
  expect(
    await screen.findByText("Suggested: Skip — every field matches."),
  ).toBeInTheDocument();
  expect(mocks.duplicateCandidates).toHaveBeenCalledTimes(2);
  expect(screen.queryByRole("alert")).toBeNull();
  expect(isPrimary(skipButton())).toBe(true);
  expect(isPrimary(importButton())).toBe(false);
});

it("with no counterpart left, says so and leans Import anyway", async () => {
  mocks.duplicateCandidates.mockResolvedValue(ok([]));
  renderPanel();
  expect(
    await screen.findByText(/matching entry is no longer in your ledger — safe to import/i),
  ).toBeInTheDocument();
  expect(isPrimary(importButton())).toBe(true);
  expect(isPrimary(skipButton())).toBe(false);
});

it("with one exact counterpart, compares side by side and leans Skip", async () => {
  mocks.duplicateCandidates.mockResolvedValue(ok([counterpart()]));
  renderPanel();
  expect(
    await screen.findByText("Suggested: Skip — every field matches."),
  ).toBeInTheDocument();
  expect(screen.getAllByText("Whole Foods Market")).toHaveLength(2);
  expect(isPrimary(skipButton())).toBe(true);
  expect(isPrimary(importButton())).toBe(false);
});

it("with several counterparts, asks to confirm and leans Import anyway", async () => {
  mocks.duplicateCandidates.mockResolvedValue(
    ok([
      counterpart(),
      counterpart({
        transaction_id: "0190e000-0000-7000-8000-0000000000f2",
        memo: "WHOLEFDS 1284",
      }),
    ]),
  );
  renderPanel();
  expect(
    await screen.findByText("2 close matches — confirm before skipping."),
  ).toBeInTheDocument();
  expect(screen.getByText(/2 ledger entries already match/i)).toBeInTheDocument();
  expect(screen.getByText("WHOLEFDS 1284")).toBeInTheDocument();
  expect(isPrimary(importButton())).toBe(true);
});

it("imports anyway from the panel and closes on success", async () => {
  mocks.duplicateCandidates.mockResolvedValue(ok([counterpart()]));
  const props = renderPanel();
  await screen.findByText(/suggested: skip/i);
  fireEvent.click(importButton());
  await waitFor(() => expect(props.onImportAnyway).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(props.onClose).toHaveBeenCalledTimes(1));
});

it("a refused import anyway shows the error and keeps the panel open", async () => {
  mocks.duplicateCandidates.mockResolvedValue(ok([counterpart()]));
  const props = renderPanel({
    onImportAnyway: vi
      .fn()
      .mockResolvedValue({ Validation: "staged transaction amount must be non-zero" }),
  });
  await screen.findByText(/suggested: skip/i);
  fireEvent.click(importButton());
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "staged transaction amount must be non-zero",
  );
  expect(props.onClose).not.toHaveBeenCalled();
});

it("recategorizes the counterpart and refreshes the comparison", async () => {
  mocks.duplicateCandidates.mockResolvedValue(ok([counterpart()]));
  renderPanel();
  const select = await screen.findByLabelText("Counterpart category");
  await waitFor(() =>
    expect(within(select).getByRole("option", { name: /groceries/i })).toBeInTheDocument(),
  );
  fireEvent.change(select, { target: { value: groceries.id } });
  await waitFor(() =>
    expect(mocks.recategorizeTransaction).toHaveBeenCalledWith(
      "0190e000-0000-7000-8000-0000000000f1",
      groceries.id,
      expect.stringMatching(/.+/),
    ),
  );
  await waitFor(() => expect(mocks.duplicateCandidates).toHaveBeenCalledTimes(2));
});

it("a failed counterpart action shows an alert by that entry", async () => {
  mocks.duplicateCandidates.mockResolvedValue(ok([counterpart()]));
  mocks.voidTransaction.mockResolvedValue(fail("That entry is reconciled."));
  renderPanel();
  fireEvent.click(await screen.findByRole("button", { name: /void this entry/i }));
  fireEvent.click(screen.getByRole("button", { name: /confirm void/i }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "That entry is reconciled.",
  );
  // Nothing was refetched, and the incoming row was never touched.
  expect(mocks.duplicateCandidates).toHaveBeenCalledTimes(1);
});
