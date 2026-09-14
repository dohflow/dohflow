import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type {
  AttachmentDto,
  CategoryDto,
  TagViewDto,
  TransactionRowDto,
} from "@/bindings";
import { TransactionDetailDrawer } from "./TransactionDetailDrawer";

const mocks = vi.hoisted(() => ({
  transactionAttachments: vi.fn(),
  importedTransactionFields: vi.fn(),
  attachDocument: vi.fn(),
  removeAttachment: vi.fn(),
  transactionSplits: vi.fn(),
  setSplits: vi.fn(),
  // Create-from-picker dialog (4d8.25.19) fetches its own taxonomy.
  categoryList: vi.fn(),
  createCategory: vi.fn(),
}));

const GROCERIES: CategoryDto = {
  id: "0190c000-0000-7000-8000-0000000000c1",
  parent_id: null,
  name: "Groceries",
  category_type: "expense",
  icon: null,
  color: null,
  is_system: true,
  forecast_behavior: "variable_regular",
  archived: false,
};
const CATEGORIES: CategoryDto[] = [GROCERIES];

const VACATION: TagViewDto = {
  id: "0190d000-0000-7000-8000-0000000000d1",
  name: "Vacation",
  color: null,
  archived: false,
};
const TAGS: TagViewDto[] = [VACATION];

const recategorize = vi.fn();
const onDelete = vi.fn();
const onSetReviewed = vi.fn();
const onCreateTag = vi.fn();
const onSetTags = vi.fn();
const onSetNote = vi.fn();

function renderDrawer(
  transaction: TransactionRowDto = txn(),
  onClose: () => void = () => {},
) {
  return renderWithClient(
    <TransactionDetailDrawer
      transaction={transaction}
      categories={CATEGORIES}
      onRecategorize={recategorize}
      onDelete={onDelete}
      onSetReviewed={onSetReviewed}
      tags={TAGS}
      onCreateTag={onCreateTag}
      onSetTags={onSetTags}
      onSetNote={onSetNote}
      onClose={onClose}
    />,
  );
}

vi.mock("@/bindings", () => ({
  commands: {
    transactionAttachments: mocks.transactionAttachments,
    importedTransactionFields: mocks.importedTransactionFields,
    attachDocument: mocks.attachDocument,
    removeAttachment: mocks.removeAttachment,
    transactionSplits: mocks.transactionSplits,
    setSplits: mocks.setSplits,
    categoryList: mocks.categoryList,
    createCategory: mocks.createCategory,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function txn(over: Partial<TransactionRowDto> = {}): TransactionRowDto {
  return {
    transaction_id: "0190b000-0000-7000-8000-000000000001",
    account_id: "0190a000-0000-7000-8000-000000000001",
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

function doc(over: Partial<AttachmentDto> = {}): AttachmentDto {
  return {
    id: "0190c000-0000-7000-8000-000000000001",
    mime_type: "application/pdf",
    original_filename: "receipt.pdf",
    plaintext_size: 2048,
    created_at: "2026-06-05T00:00:00Z",
    ...over,
  };
}

beforeEach(() => {
  mocks.transactionAttachments.mockReset();
  mocks.attachDocument.mockReset();
  mocks.removeAttachment.mockReset();
  mocks.transactionAttachments.mockResolvedValue(ok([]));
  mocks.importedTransactionFields.mockResolvedValue(ok(null));
  mocks.transactionSplits.mockReset();
  mocks.transactionSplits.mockResolvedValue(ok([]));
  mocks.setSplits.mockReset();
  mocks.setSplits.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
  recategorize.mockReset();
  recategorize.mockResolvedValue(null);
  onDelete.mockReset();
  onDelete.mockResolvedValue(null);
  onSetReviewed.mockReset();
  onSetReviewed.mockResolvedValue(null);
  onCreateTag.mockReset();
  onCreateTag.mockResolvedValue("0190d000-0000-7000-8000-0000000000d9");
  onSetTags.mockReset();
  onSetTags.mockResolvedValue(null);
  onSetNote.mockReset();
  onSetNote.mockResolvedValue(null);
});

describe("TransactionDetailDrawer", () => {
  it("shows the empty state when nothing is attached", async () => {
    renderDrawer();
    expect(
      await screen.findByText(/no documents attached/i),
    ).toBeInTheDocument();
  });

  it("lists attachments with their name and size", async () => {
    mocks.transactionAttachments.mockResolvedValue(ok([doc()]));
    renderDrawer();
    expect(await screen.findByText("receipt.pdf")).toBeInTheDocument();
    expect(screen.getByText("2.0 KB")).toBeInTheDocument();
  });

  it("attaches a picked file (as bytes) and refreshes the list", async () => {
    mocks.transactionAttachments
      .mockResolvedValueOnce(ok([]))
      .mockResolvedValue(ok([doc({ original_filename: "statement.pdf" })]));
    mocks.attachDocument.mockResolvedValue(
      ok(doc({ original_filename: "statement.pdf" })),
    );
    renderDrawer();
    await screen.findByText(/no documents attached/i);

    const file = new File([new Uint8Array([1, 2, 3])], "statement.pdf", {
      type: "application/pdf",
    });
    // jsdom's File has no arrayBuffer(); the real Tauri webview does. Stub it.
    file.arrayBuffer = () => Promise.resolve(new Uint8Array([1, 2, 3]).buffer);
    fireEvent.change(screen.getByLabelText(/choose a document/i), {
      target: { files: [file] },
    });

    expect(await screen.findByText("statement.pdf")).toBeInTheDocument();
    expect(mocks.attachDocument).toHaveBeenCalledTimes(1);
    const [transactionId, filename, mime, data] =
      mocks.attachDocument.mock.calls[0] ?? [];
    expect(transactionId).toBe(txn().transaction_id);
    expect(filename).toBe("statement.pdf");
    expect(mime).toBe("application/pdf");
    expect(data).toEqual([1, 2, 3]); // raw bytes, not base64 or a blob URL
  });

  it("removes an attachment", async () => {
    mocks.transactionAttachments
      .mockResolvedValueOnce(ok([doc()]))
      .mockResolvedValue(ok([]));
    mocks.removeAttachment.mockResolvedValue(ok(null));
    renderDrawer();

    fireEvent.click(
      await screen.findByRole("button", { name: /remove receipt\.pdf/i }),
    );
    await waitFor(() =>
      expect(mocks.removeAttachment).toHaveBeenCalledWith(
        doc().id,
        txn().transaction_id,
      ),
    );
  });

  it("assigns a category by typing its LEAF name in the picker (bac, 4d8.25.18)", async () => {
    renderDrawer();
    fireEvent.click(await screen.findByRole("button", { name: "Category" }));
    // Leaf-name search: no parent-path prefix needed (the owner's complaint).
    fireEvent.change(screen.getByRole("combobox", { name: "Search category" }), {
      target: { value: "groc" },
    });
    fireEvent.pointerDown(screen.getByRole("option", { name: /Groceries/ }));
    await waitFor(() =>
      expect(recategorize).toHaveBeenCalledWith(
        txn().transaction_id,
        GROCERIES.id,
      ),
    );
  });

  it("clears the category when Uncategorized is chosen (bac)", async () => {
    renderDrawer(txn());
    fireEvent.click(await screen.findByRole("button", { name: "Category" }));
    fireEvent.pointerDown(screen.getByRole("option", { name: /Groceries/ }));
    await waitFor(() => expect(recategorize).toHaveBeenCalled());
    fireEvent.click(screen.getByRole("button", { name: "Category" }));
    fireEvent.pointerDown(screen.getByRole("option", { name: "Uncategorized" }));
    await waitFor(() =>
      expect(recategorize).toHaveBeenLastCalledWith(txn().transaction_id, null),
    );
  });

  it("creates a category from the picker and selects it without leaving the drawer (4d8.25.19)", async () => {
    mocks.categoryList.mockResolvedValue(ok(CATEGORIES));
    mocks.createCategory.mockResolvedValue(
      ok({ category_id: "new-cat", mutation: { op_seq: 9, replayed: false } }),
    );
    renderDrawer();
    fireEvent.click(await screen.findByRole("button", { name: "Category" }));
    fireEvent.change(screen.getByRole("combobox", { name: "Search category" }), {
      target: { value: "Coffee shops" },
    });
    fireEvent.pointerDown(screen.getByRole("button", { name: "Create new category…" }));

    // The dialog opens prefilled with the search text; complete + submit it.
    const dialog = await screen.findByRole("dialog", { name: "New category" });
    expect(within(dialog).getByLabelText("Category name")).toHaveValue("Coffee shops");
    fireEvent.click(within(dialog).getByRole("button", { name: "Create category" }));

    await waitFor(() =>
      expect(mocks.createCategory).toHaveBeenCalledWith(
        expect.objectContaining({ name: "Coffee shops" }),
      ),
    );
    // The new id autofills the picker's selection — still inside the drawer.
    await waitFor(() =>
      expect(recategorize).toHaveBeenLastCalledWith(txn().transaction_id, "new-cat"),
    );
    expect(screen.queryByRole("dialog", { name: "New category" })).not.toBeInTheDocument();
  });

  it("deletes the transaction after a confirm step, then closes (4d8.11)", async () => {
    const onClose = vi.fn();
    renderDrawer(txn(), onClose);

    // The first click reveals the confirm — it does not delete yet.
    fireEvent.click(
      screen.getByRole("button", { name: /delete transaction/i }),
    );
    expect(onDelete).not.toHaveBeenCalled();
    expect(screen.getByText(/delete this transaction\?/i)).toBeInTheDocument();

    // Confirming voids the transaction and closes the drawer.
    fireEvent.click(screen.getByRole("button", { name: /^delete$/i }));
    await waitFor(() =>
      expect(onDelete).toHaveBeenCalledWith(txn().transaction_id),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("marks the transaction reviewed from the drawer (4d8.7)", async () => {
    renderDrawer(); // txn() defaults reviewed: false → shows "Mark reviewed"
    fireEvent.click(
      await screen.findByRole("button", { name: /mark reviewed/i }),
    );
    await waitFor(() =>
      expect(onSetReviewed).toHaveBeenCalledWith(txn().transaction_id, true),
    );
    // Optimistic flip → the toggle now reads "Reviewed".
    expect(
      await screen.findByRole("button", { name: /^reviewed$/i }),
    ).toBeInTheDocument();
  });

  it("assigns an existing tag from the input (4d8.15)", async () => {
    renderDrawer();
    const input = await screen.findByLabelText(/add a tag/i);
    fireEvent.change(input, { target: { value: "Vacation" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() =>
      expect(onSetTags).toHaveBeenCalledWith(txn().transaction_id, [
        VACATION.id,
      ]),
    );
    expect(onCreateTag).not.toHaveBeenCalled();
  });

  it("creates a new tag on the fly and assigns it (4d8.15)", async () => {
    renderDrawer();
    const input = await screen.findByLabelText(/add a tag/i);
    fireEvent.change(input, { target: { value: "Travel" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(onCreateTag).toHaveBeenCalledWith("Travel"));
    await waitFor(() =>
      expect(onSetTags).toHaveBeenCalledWith(txn().transaction_id, [
        "0190d000-0000-7000-8000-0000000000d9",
      ]),
    );
  });

  it("saves a note on blur (4d8.15)", async () => {
    renderDrawer();
    const note = await screen.findByLabelText(/^note$/i);
    fireEvent.change(note, { target: { value: "Hotel in Cancún" } });
    fireEvent.blur(note);
    await waitFor(() =>
      expect(onSetNote).toHaveBeenCalledWith(
        txn().transaction_id,
        "Hotel in Cancún",
      ),
    );
  });

  it("splits a transaction into balanced lines (4d8.18)", async () => {
    renderDrawer(); // txn amount -$40.00, no existing splits
    fireEvent.click(
      await screen.findByRole("button", { name: /split transaction/i }),
    );
    // Two lines summing to $40 — same (negative) sign as the transaction on save.
    fireEvent.change(screen.getByLabelText(/split 1 amount/i), {
      target: { value: "25.00" },
    });
    fireEvent.change(screen.getByLabelText(/split 2 amount/i), {
      target: { value: "15.00" },
    });
    fireEvent.click(screen.getByRole("button", { name: /save split/i }));
    await waitFor(() =>
      expect(mocks.setSplits).toHaveBeenCalledWith(
        txn().transaction_id,
        [
          {
            amount: { minor_units: -2500, currency: "USD" },
            category_id: null,
            note: null,
            tag_ids: [],
          },
          {
            amount: { minor_units: -1500, currency: "USD" },
            category_id: null,
            note: null,
            tag_ids: [],
          },
        ],
        expect.stringMatching(/.+/),
      ),
    );
  });

  it("carries a per-line note and tag into the split (4d8.19)", async () => {
    renderDrawer(); // txn amount -$40.00
    fireEvent.click(
      await screen.findByRole("button", { name: /split transaction/i }),
    );
    fireEvent.change(screen.getByLabelText(/split 1 amount/i), {
      target: { value: "25.00" },
    });
    fireEvent.change(screen.getByLabelText(/split 2 amount/i), {
      target: { value: "15.00" },
    });
    // A note + an existing tag on line 1 only.
    fireEvent.change(screen.getByLabelText(/split 1 note/i), {
      target: { value: "kids supplies" },
    });
    const tagInput = screen.getByLabelText(/split 1 tags/i);
    fireEvent.change(tagInput, { target: { value: "Vacation" } });
    fireEvent.keyDown(tagInput, { key: "Enter" });
    await screen.findByText("Vacation"); // the chip appears on the line

    fireEvent.click(screen.getByRole("button", { name: /save split/i }));
    await waitFor(() =>
      expect(mocks.setSplits).toHaveBeenCalledWith(
        txn().transaction_id,
        [
          {
            amount: { minor_units: -2500, currency: "USD" },
            category_id: null,
            note: "kids supplies",
            tag_ids: [VACATION.id],
          },
          {
            amount: { minor_units: -1500, currency: "USD" },
            category_id: null,
            note: null,
            tag_ids: [],
          },
        ],
        expect.stringMatching(/.+/),
      ),
    );
  });

  it("offers a Make recurring action (personal-cfo-5n4.6)", async () => {
    renderDrawer();
    // The affordance is present; the pre-filled form itself is covered by
    // MakeRecurringBillForm.test.tsx (opening it needs the accounts/bills queries).
    expect(
      await screen.findByRole("button", { name: /make recurring/i }),
    ).toBeInTheDocument();
  });

  it("shows a Dates section with posted + transaction dates when an import carried both (4d8.24.1.3)", () => {
    renderDrawer(txn({ transaction_date: "2026-06-03" }));
    const dates = screen.getByRole("region", { name: "Dates" });
    expect(within(dates).getByText("Posted")).toBeInTheDocument();
    expect(within(dates).getByText("Transaction")).toBeInTheDocument();
    // The transaction date is the bare calendar date, not shifted by timezone.
    expect(within(dates).getByText("Jun 3, 2026")).toBeInTheDocument();
  });

  it("omits the Dates section for a single-date transaction", () => {
    renderDrawer(txn({ transaction_date: null }));
    expect(
      screen.queryByRole("region", { name: "Dates" }),
    ).not.toBeInTheDocument();
  });

  it("shows the raw imported fields in a collapsible Imported details section (4d8.24.1.4)", async () => {
    mocks.importedTransactionFields.mockResolvedValue(
      ok({
        source_type: "csv",
        imported_at: "2026-07-01T00:00:00Z",
        fields: [
          { key: "Card No.", value: "1234" },
          { key: "Category", value: "Dining" },
        ],
      }),
    );
    renderDrawer();
    const section = await screen.findByRole("region", {
      name: "Imported details",
    });
    const toggle = within(section).getByRole("button", {
      name: /imported details/i,
    });
    // Collapsed by default; the captured columns appear once expanded.
    expect(within(section).queryByText("Card No.")).not.toBeInTheDocument();
    fireEvent.click(toggle);
    expect(within(section).getByText("Card No.")).toBeInTheDocument();
    expect(within(section).getByText("1234")).toBeInTheDocument();
    expect(within(section).getByText("Category")).toBeInTheDocument();
    expect(within(section).getByText("Dining")).toBeInTheDocument();
  });

  it("omits Imported details for a manually-entered transaction", () => {
    renderDrawer(); // default mock: importedTransactionFields → null
    expect(
      screen.queryByRole("region", { name: "Imported details" }),
    ).not.toBeInTheDocument();
  });
});
