import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { TransactionRowDto } from "@/bindings";
import { CardReviewModal } from "./CardReviewModal";

const mocks = vi.hoisted(() => ({
  transactionAttachments: vi.fn(),
  categoryList: vi.fn(),
  createCategory: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    transactionAttachments: mocks.transactionAttachments,
    categoryList: mocks.categoryList,
    createCategory: mocks.createCategory,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function row(id: string, over: Partial<TransactionRowDto> = {}): TransactionRowDto {
  return {
    transaction_id: id,
    account_id: "0190a000-0000-7000-8000-00000000000a",
    account_name: "Venture X",
    counter_account_id: null,
    counter_account_name: null,
    occurred_at: "2026-07-08T12:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
    amount: { minor_units: -4_387, currency: "USD" },
    memo: "TRADER JOES #482 QPS",
    counterparty: "Trader Joe's",
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

function setup(over: {
  queueIds?: string[];
  onMarkReviewed?: ReturnType<typeof vi.fn>;
  onSetNote?: ReturnType<typeof vi.fn>;
  onClose?: ReturnType<typeof vi.fn>;
} = {}) {
  mocks.transactionAttachments.mockResolvedValue(ok([]));
  const queueIds = over.queueIds ?? ["t1", "t2", "t3"];
  const rowsById = new Map(
    queueIds.map((id) => [id, row(id, { counterparty: `Merchant ${id}` })]),
  );
  const props = {
    queueIds,
    rowsById,
    categories: [],
    tags: [],
    onMarkReviewed: over.onMarkReviewed ?? vi.fn().mockResolvedValue(null),
    onRecategorize: vi.fn().mockResolvedValue(null),
    onCreateTag: vi.fn().mockResolvedValue("tag-1"),
    onSetTags: vi.fn().mockResolvedValue(null),
    onSetNote: over.onSetNote ?? vi.fn().mockResolvedValue(null),
    onClose: over.onClose ?? vi.fn(),
  };
  renderWithClient(<CardReviewModal {...props} />);
  return props;
}

describe("CardReviewModal", () => {
  it("reviews with the right arrow and advances the queue", async () => {
    const props = setup();
    expect(screen.getByText("1 of 3 to review")).toBeInTheDocument();
    expect(screen.getByText("Merchant t1")).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "ArrowRight" });
    await waitFor(() =>
      expect(props.onMarkReviewed).toHaveBeenCalledWith("t1"),
    );
    expect(await screen.findByText("1 of 2 to review")).toBeInTheDocument();
    expect(screen.getByText("Merchant t2")).toBeInTheDocument();
    expect(screen.getByText(/1 of 3 reviewed/)).toBeInTheDocument();
  });

  it("defers with the left arrow, counts it set aside, and badges the resurfaced card", async () => {
    const props = setup();
    fireEvent.keyDown(window, { key: "ArrowLeft" }); // defer t1
    expect(await screen.findByText("Merchant t2")).toBeInTheDocument();
    expect(screen.getByText(/1 set aside/)).toBeInTheDocument();
    expect(props.onMarkReviewed).not.toHaveBeenCalled();

    // Review t2 and t3; t1 resurfaces with the badge.
    fireEvent.keyDown(window, { key: "ArrowRight" });
    await screen.findByText("Merchant t3");
    fireEvent.keyDown(window, { key: "ArrowRight" });
    expect(await screen.findByText("Merchant t1")).toBeInTheDocument();
    expect(screen.getByText("Resurfaced")).toBeInTheDocument();
    expect(screen.getByText("1 of 1 to review")).toBeInTheDocument();
  });

  it("arrow keys are inert while typing in a text field", async () => {
    const props = setup();
    const notes = screen.getByLabelText("Notes");
    fireEvent.keyDown(notes, { key: "ArrowRight" });
    await waitFor(() => expect(props.onMarkReviewed).not.toHaveBeenCalled());
    expect(screen.getByText("1 of 3 to review")).toBeInTheDocument();
  });

  it("flushes a dirty note before reviewing", async () => {
    const onSetNote = vi.fn().mockResolvedValue(null);
    const props = setup({ onSetNote });
    fireEvent.change(screen.getByLabelText("Notes"), {
      target: { value: "split with roommate" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Mark reviewed" }));
    await waitFor(() =>
      expect(onSetNote).toHaveBeenCalledWith("t1", "split with roommate"),
    );
    expect(props.onMarkReviewed).toHaveBeenCalledWith("t1");
  });

  it("shows the finished panel with stats and closes with Back to inbox", async () => {
    const onClose = vi.fn();
    setup({ queueIds: ["t1", "t2"], onClose });
    fireEvent.keyDown(window, { key: "ArrowLeft" }); // set t1 aside
    await screen.findByText("Merchant t2");
    fireEvent.keyDown(window, { key: "ArrowRight" }); // review t2
    await screen.findByText("Merchant t1");
    fireEvent.keyDown(window, { key: "ArrowRight" }); // review resurfaced t1

    expect(await screen.findByText("Inbox reviewed")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument(); // reviewed stat
    expect(screen.getByText("Set aside, then cleared")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Back to inbox" }));
    expect(onClose).toHaveBeenCalled();
  });

  it("keeps persisted edits when a deferred card resurfaces (no snapshot re-seed tag loss)", async () => {
    const onSetTags = vi.fn().mockResolvedValue(null);
    mocks.transactionAttachments.mockResolvedValue(ok([]));
    const queueIds = ["t1", "t2"];
    const rowsById = new Map(
      queueIds.map((id) => [id, row(id, { counterparty: `Merchant ${id}` })]),
    );
    const props = {
      queueIds,
      rowsById,
      categories: [],
      tags: [
        { id: "tag-vac", name: "vacation", color: null, archived: false, usage_count: 0 },
        { id: "tag-food", name: "food", color: null, archived: false, usage_count: 0 },
      ],
      onMarkReviewed: vi.fn().mockResolvedValue(null),
      onRecategorize: vi.fn().mockResolvedValue(null),
      onCreateTag: vi.fn().mockResolvedValue("tag-new"),
      onSetTags,
      onSetNote: vi.fn().mockResolvedValue(null),
      onClose: vi.fn(),
    };
    renderWithClient(<CardReviewModal {...props} />);

    // Visit 1 on t1: add the "vacation" tag (persisted via exact-replace SetTags).
    const tagInput = screen.getByLabelText("Tags");
    fireEvent.change(tagInput, { target: { value: "vacation" } });
    fireEvent.keyDown(tagInput, { key: "Enter" });
    await waitFor(() => expect(onSetTags).toHaveBeenCalledWith("t1", ["tag-vac"]));

    // Defer t1, review t2 — t1 resurfaces.
    fireEvent.keyDown(window, { key: "ArrowLeft" });
    await screen.findByText("Merchant t2");
    fireEvent.keyDown(window, { key: "ArrowRight" });
    await screen.findByText("Merchant t1");

    // The persisted tag still shows (NOT the stale snapshot's empty set) — awaited,
    // since the resurfaced card re-seeds its chips from the edits overlay in an
    // effect that hasn't necessarily flushed synchronously (personal-cfo-56w.4).
    await screen.findByText("vacation");

    // …and adding a second tag merges with it instead of replacing it.
    fireEvent.change(screen.getByLabelText("Tags"), { target: { value: "food" } });
    fireEvent.keyDown(screen.getByLabelText("Tags"), { key: "Enter" });
    await waitFor(() =>
      expect(onSetTags).toHaveBeenLastCalledWith("t1", ["tag-vac", "tag-food"]),
    );
  });

  it("suspends review shortcuts while the create-category dialog is open (4d8.25.19 review)", async () => {
    mocks.categoryList.mockResolvedValue(ok([]));
    const props = setup();
    // Open the category picker and choose "Create new category…".
    fireEvent.click(screen.getByRole("button", { name: "Category" }));
    fireEvent.pointerDown(
      await screen.findByRole("button", { name: "Create new category…" }),
    );
    await screen.findByRole("dialog", { name: "New category" });
    // An arrow key now belongs to the dialog, not the review queue underneath.
    fireEvent.keyDown(window, { key: "ArrowRight" });
    expect(props.onMarkReviewed).not.toHaveBeenCalled();
    expect(screen.getByText("1 of 3 to review")).toBeInTheDocument();
  });

  it("closes on Escape", () => {
    const onClose = vi.fn();
    setup({ onClose });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("keeps the card and shows the error when review fails", async () => {
    const onMarkReviewed = vi
      .fn()
      .mockResolvedValue({ kind: "persistence", message: "nope" });
    setup({ onMarkReviewed });
    fireEvent.keyDown(window, { key: "ArrowRight" });
    await waitFor(() => expect(onMarkReviewed).toHaveBeenCalled());
    expect(screen.getByText("1 of 3 to review")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });
});
