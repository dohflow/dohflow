import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import type {
  CategoryDto,
  IpcError,
  TagViewDto,
  TransactionRowDto,
} from "@/bindings";
import { BulkTransactionActions } from "./BulkTransactionActions";

function txn(id: string, over: Partial<TransactionRowDto> = {}): TransactionRowDto {
  return {
    transaction_id: id,
    account_id: "acct-1",
    account_name: "Checking",
    counter_account_id: null,
    counter_account_name: null,
    occurred_at: "2026-06-05T00:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
    amount: { minor_units: -4000, currency: "USD" },
    memo: null,
    counterparty: null,
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

const CATEGORY: CategoryDto = {
  id: "cat-1",
  parent_id: null,
  name: "Groceries",
  category_type: "expense",
  icon: null,
  color: null,
  is_system: true,
  forecast_behavior: "variable_regular",
  archived: false,
};
const TAG: TagViewDto = { id: "tag-1", name: "Vacation", color: null, archived: false };

const okOp = () => Promise.resolve<IpcError | null>(null);

function setup(over: Partial<Parameters<typeof BulkTransactionActions>[0]> = {}) {
  const ops = {
    recategorize: vi.fn(okOp),
    setReviewed: vi.fn(okOp),
    deleteTransaction: vi.fn(okOp),
    setTags: vi.fn(okOp),
    createTag: vi.fn<(name: string) => Promise<string | IpcError>>(async () => "tag-new"),
    onSelectAll: vi.fn(),
    onSelectionChange: vi.fn(),
  };
  render(
    <BulkTransactionActions
      selected={[txn("t1"), txn("t2")]}
      total={5}
      categories={[CATEGORY]}
      tags={[TAG]}
      {...ops}
      {...over}
    />,
  );
  return ops;
}

test("recategorizes every selected transaction, then clears", async () => {
  const ops = setup();
  fireEvent.click(screen.getByRole("button", { name: "Categorize selected" }));
  fireEvent.pointerDown(screen.getByRole("option", { name: /Groceries/ }));
  await waitFor(() => expect(ops.recategorize).toHaveBeenCalledTimes(2));
  expect(ops.recategorize).toHaveBeenCalledWith("t1", "cat-1");
  expect(ops.recategorize).toHaveBeenCalledWith("t2", "cat-1");
  // A clean run leaves nothing selected.
  await waitFor(() => expect(ops.onSelectionChange).toHaveBeenCalledWith([]));
});

test("applies an existing tag (by name) to all, merging with each transaction's tags", async () => {
  const ops = setup({ selected: [txn("t1", { tag_ids: ["existing"] })] });
  const input = screen.getByLabelText(/add or create tag/i);
  fireEvent.change(input, { target: { value: "Vacation" } }); // matches TAG (id tag-1)
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() =>
    expect(ops.setTags).toHaveBeenCalledWith("t1", ["existing", "tag-1"]),
  );
  // An existing match does not mint a new tag.
  expect(ops.createTag).not.toHaveBeenCalled();
});

test("creates a new tag and applies it to every selected transaction (even with zero tags)", async () => {
  const ops = setup({
    tags: [],
    selected: [txn("t1"), txn("t2", { tag_ids: ["keep"] })],
  });
  const input = screen.getByLabelText(/add or create tag/i);
  fireEvent.change(input, { target: { value: "Travel" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(ops.createTag).toHaveBeenCalledTimes(1));
  expect(ops.createTag).toHaveBeenCalledWith("Travel");
  await waitFor(() => expect(ops.setTags).toHaveBeenCalledTimes(2));
  expect(ops.setTags).toHaveBeenCalledWith("t1", ["tag-new"]);
  expect(ops.setTags).toHaveBeenCalledWith("t2", ["keep", "tag-new"]);
});

test("does not create a tag for an empty name, and surfaces a create failure", async () => {
  const failing = vi.fn(async () => ({ kind: "Validation", message: "no" }) as unknown as IpcError);
  const ops = setup({ tags: [], createTag: failing });
  const input = screen.getByLabelText(/add or create tag/i);
  // Empty/whitespace -> no-op.
  fireEvent.change(input, { target: { value: "   " } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(failing).not.toHaveBeenCalled();
  // A create failure surfaces a note and applies no tags.
  fireEvent.change(input, { target: { value: "Travel" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(await screen.findByRole("alert")).toHaveTextContent(/could not create tag/i);
  expect(ops.setTags).not.toHaveBeenCalled();
});

test("marks every selected transaction reviewed", async () => {
  const ops = setup();
  fireEvent.click(screen.getByRole("button", { name: /mark reviewed/i }));
  await waitFor(() => expect(ops.setReviewed).toHaveBeenCalledTimes(2));
  expect(ops.setReviewed).toHaveBeenCalledWith("t1", true);
});

test("deletes all only after confirming", async () => {
  const ops = setup();
  fireEvent.click(screen.getByRole("button", { name: /^delete$/i }));
  expect(ops.deleteTransaction).not.toHaveBeenCalled(); // not until confirmed
  fireEvent.click(screen.getByRole("button", { name: /confirm delete/i }));
  await waitFor(() => expect(ops.deleteTransaction).toHaveBeenCalledTimes(2));
});

test("keeps the selection and reports the count on partial failure", async () => {
  const failing = vi
    .fn<(id: string) => Promise<IpcError | null>>()
    .mockResolvedValueOnce(null)
    .mockResolvedValueOnce({ kind: "Validation", message: "nope" } as unknown as IpcError);
  const ops = setup({ deleteTransaction: failing });
  fireEvent.click(screen.getByRole("button", { name: /^delete$/i }));
  fireEvent.click(screen.getByRole("button", { name: /confirm delete/i }));
  expect(await screen.findByRole("alert")).toHaveTextContent(/1 failed/i);
  // Only the failed row stays selected, ready to retry.
  expect(ops.onSelectionChange).toHaveBeenCalledWith(["t2"]);
});

test("offers Select all when not everything is selected", () => {
  const ops = setup(); // 2 of 5 selected
  fireEvent.click(screen.getByRole("button", { name: /select all 5 shown/i }));
  expect(ops.onSelectAll).toHaveBeenCalled();
});
