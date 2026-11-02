import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

import type {
  CategoryDto,
  IpcError,
  TagViewDto,
  TransactionRowDto,
} from "@/bindings";
import { BulkTransactionActions } from "./BulkTransactionActions";

const mocks = vi.hoisted(() => ({ transactionRowsByIds: vi.fn() }));
vi.mock("@/bindings", () => ({ commands: mocks }));
const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

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

/// The fresh read returns the rows as selected unless a test says the tags moved.
beforeEach(() => {
  mocks.transactionRowsByIds.mockReset();
  mocks.transactionRowsByIds.mockImplementation(async () => ok(currentRows));
});
let currentRows: TransactionRowDto[] = [];

function setup(over: Partial<Parameters<typeof BulkTransactionActions>[0]> = {}) {
  const ops = {
    recategorize: vi.fn(okOp),
    setReviewed: vi.fn(okOp),
    deleteTransaction: vi.fn(okOp),
    setTags: vi.fn<(id: string, tagIds: string[]) => Promise<IpcError | null>>(okOp),
    createTag: vi.fn<(name: string) => Promise<string | IpcError>>(async () => "tag-new"),
    onSelectAll: vi.fn(),
    onSelectionChange: vi.fn(),
  };
  const selected = over.selected ?? [txn("t1"), txn("t2")];
  currentRows = selected;
  render(
    <BulkTransactionActions
      selected={selected}
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

describe("bulk tag is honest (5n4.10)", () => {
  const tagInput = () => screen.getByLabelText(/add or create tag/i);
  const enter = (name = "Vacation") => {
    fireEvent.change(tagInput(), { target: { value: name } });
    fireEvent.keyDown(tagInput(), { key: "Enter" });
  };

  test("merges into the tags as they are now, not as the list last saw them", async () => {
    const ops = setup({ selected: [txn("t1", { tag_ids: ["old"] })] });
    currentRows = [txn("t1", { tag_ids: ["old", "added-elsewhere"] })];
    enter();
    await waitFor(() =>
      expect(ops.setTags).toHaveBeenCalledWith("t1", ["old", "added-elsewhere", "tag-1"]),
    );
  });

  test("a second Enter or click while the fresh read is pending does nothing", async () => {
    const ops = setup();
    let release: (value: unknown) => void = () => {};
    mocks.transactionRowsByIds.mockReturnValue(new Promise((resolve) => (release = resolve)));
    enter();
    // Busy from the first keystroke, not only once the writes start.
    await waitFor(() => expect(screen.getByRole("button", { name: "Add" })).toBeDisabled());
    fireEvent.keyDown(tagInput(), { key: "Enter" });
    fireEvent.keyDown(tagInput(), { key: "Enter" });
    expect(mocks.transactionRowsByIds).toHaveBeenCalledTimes(1);
    release(ok([txn("t1"), txn("t2")]));
    await waitFor(() => expect(ops.setTags).toHaveBeenCalledTimes(2));
    expect(mocks.transactionRowsByIds).toHaveBeenCalledTimes(1);
  });

  test.each([
    ["is refused", () => mocks.transactionRowsByIds.mockResolvedValue({ status: "error", error: { Validation: "no" } })],
    ["throws", () => mocks.transactionRowsByIds.mockRejectedValue(new Error("connection lost"))],
  ])("a fresh read that %s stops with an error and writes nothing", async (_name, fail) => {
    const ops = setup({ selected: [txn("t1", { tag_ids: ["old"] }), txn("t2")] });
    fail();
    enter();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn’t read the current tags, so nothing was changed. Try again.",
    );
    expect(ops.setTags).not.toHaveBeenCalled();
    // The selection is untouched and the bar is usable again.
    expect(ops.onSelectionChange).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Mark reviewed" })).toBeEnabled();
  });

  test("a new tag is only created after the fresh read succeeds, so a failed read leaves nothing behind and keeps the typed name", async () => {
    const ops = setup({ tags: [] });
    mocks.transactionRowsByIds.mockRejectedValue(new Error("connection lost"));
    enter("Travel");
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn’t read the current tags, so nothing was changed. Try again.",
    );
    expect(ops.createTag).not.toHaveBeenCalled();
    expect(ops.setTags).not.toHaveBeenCalled();
    // Try again does not mean retyping.
    expect(tagInput()).toHaveValue("Travel");

    mocks.transactionRowsByIds.mockImplementation(async () => ok(currentRows));
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() => expect(ops.setTags).toHaveBeenCalledTimes(2));
    expect(ops.createTag).toHaveBeenCalledTimes(1);
    expect(mocks.transactionRowsByIds.mock.invocationCallOrder[1]!).toBeLessThan(
      ops.createTag.mock.invocationCallOrder[0]!,
    );
    expect(tagInput()).toHaveValue("");
  });

  test("two starts in the same tick run once: the guard does not wait for a re-render", async () => {
    const ops = setup();
    fireEvent.change(tagInput(), { target: { value: "Vacation" } });
    const add = screen.getByRole("button", { name: "Add" });
    // One act batches both clicks, so the button is still enabled for the second: only the
    // synchronous guard can stop it.
    await act(async () => {
      add.click();
      add.click();
    });
    await waitFor(() => expect(ops.setTags).toHaveBeenCalledTimes(2));
    expect(mocks.transactionRowsByIds).toHaveBeenCalledTimes(1);
  });

  test("a row the fresh read no longer returns is not written, and is counted as not changed", async () => {
    const ops = setup();
    currentRows = [txn("t1")];
    enter();
    expect(await screen.findByRole("alert")).toHaveTextContent("Tagged 1 of 2 — 1 failed.");
    expect(ops.setTags).toHaveBeenCalledTimes(1);
    expect(ops.setTags).toHaveBeenCalledWith("t1", ["tag-1"]);
    expect(ops.onSelectionChange).toHaveBeenCalledWith(["t2"]);
  });

  test("a partial result says how many changed and how many did not, whether a row was refused or threw", async () => {
    const ops = setup({ selected: [txn("t1"), txn("t2"), txn("t3")] });
    ops.setTags.mockImplementation(async (id: string) => {
      if (id === "t2") return { Validation: "nope" } as IpcError;
      if (id === "t3") throw new Error("connection lost");
      return null;
    });
    enter();
    // A refusal is a failure; a call that threw is unconfirmed, because it may have saved.
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Tagged 1 of 3 — 1 failed, 1 couldn’t be confirmed. Check the unconfirmed rows before trying again.",
    );
    // Both stay selected; the saved one drops out.
    expect(ops.onSelectionChange).toHaveBeenCalledWith(["t2", "t3"]);
    // Not stuck busy after a row threw.
    expect(screen.getByRole("button", { name: "Mark reviewed" })).toBeEnabled();
  });

  describe("a call that throws is unconfirmed for every bulk verb (5n4.10)", () => {
    const CHECK = "Check the unconfirmed rows before trying again.";

    test("only a thrown write: no failed count, and the row stays selected", async () => {
      const ops = setup({ selected: [txn("t1"), txn("t2"), txn("t3")] });
      ops.setTags.mockImplementation(async (id: string) => {
        if (id === "t3") throw new Error("connection lost");
        return null;
      });
      enter();
      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent(`Tagged 2 of 3 — 1 couldn’t be confirmed. ${CHECK}`);
      expect(alert).not.toHaveTextContent(/failed/);
      expect(ops.onSelectionChange).toHaveBeenCalledWith(["t3"]);
    });

    test("only a typed refusal: counted as failed, no unconfirmed sentence", async () => {
      const ops = setup({ selected: [txn("t1"), txn("t2")] });
      ops.setTags.mockImplementation(async (id: string) =>
        id === "t2" ? ({ Validation: "nope" } as IpcError) : null,
      );
      enter();
      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent("Tagged 1 of 2 — 1 failed.");
      expect(alert).not.toHaveTextContent(/confirm/);
    });

    test("Categorize: a rejected write is unconfirmed and stays selected", async () => {
      const ops = setup();
      ops.recategorize.mockImplementation(async (id: string) => {
        if (id === "t1") throw new Error("connection lost");
        return null;
      });
      fireEvent.click(screen.getByRole("button", { name: "Categorize selected" }));
      fireEvent.pointerDown(screen.getByRole("option", { name: /Groceries/ }));
      expect(await screen.findByRole("alert")).toHaveTextContent(
        `Recategorized 1 of 2 — 1 couldn’t be confirmed. ${CHECK}`,
      );
      expect(ops.onSelectionChange).toHaveBeenCalledWith(["t1"]);
    });

    test("Delete: a rejected write is unconfirmed and stays selected", async () => {
      const ops = setup();
      ops.deleteTransaction.mockImplementation(async (id: string) => {
        if (id === "t2") throw new Error("connection lost");
        return null;
      });
      fireEvent.click(screen.getByRole("button", { name: /^delete$/i }));
      fireEvent.click(screen.getByRole("button", { name: /confirm delete/i }));
      expect(await screen.findByRole("alert")).toHaveTextContent(
        `Deleted 1 of 2 — 1 couldn’t be confirmed. ${CHECK}`,
      );
      expect(ops.onSelectionChange).toHaveBeenCalledWith(["t2"]);
    });

    test("Mark reviewed (per row): a rejected write is unconfirmed and stays selected", async () => {
      const ops = setup();
      ops.setReviewed.mockImplementation(async (id: string) => {
        if (id === "t1") throw new Error("connection lost");
        return null;
      });
      fireEvent.click(screen.getByRole("button", { name: "Mark reviewed" }));
      expect(await screen.findByRole("alert")).toHaveTextContent(
        `Marked reviewed 1 of 2 — 1 couldn’t be confirmed. ${CHECK}`,
      );
      expect(ops.onSelectionChange).toHaveBeenCalledWith(["t1"]);
    });

    test("Mark reviewed (one call) that throws leaves every row selected and unconfirmed", async () => {
      const bulk = vi.fn().mockRejectedValue(new Error("connection lost"));
      const ops = setup({ bulkMarkReviewed: bulk });
      fireEvent.click(screen.getByRole("button", { name: "Mark reviewed" }));
      expect(await screen.findByRole("alert")).toHaveTextContent(
        `Marked reviewed 0 of 2 — 2 couldn’t be confirmed. ${CHECK}`,
      );
      expect(ops.onSelectionChange).not.toHaveBeenCalled();
      expect(screen.getByRole("button", { name: "Mark reviewed" })).toBeEnabled();
    });
  });

  test("other bulk actions are also exclusive while one is running", async () => {
    let release: (value: IpcError | null) => void = () => {};
    const ops = setup();
    ops.recategorize.mockReturnValue(new Promise((resolve) => (release = resolve)));
    fireEvent.click(screen.getByRole("button", { name: "Categorize selected" }));
    fireEvent.pointerDown(screen.getByRole("option", { name: /Groceries/ }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Mark reviewed" })).toBeDisabled());
    fireEvent.click(screen.getByRole("button", { name: "Mark reviewed" }));
    expect(ops.setReviewed).not.toHaveBeenCalled();
    release(null);
  });
});
