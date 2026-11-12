import { useState } from "react";
import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { CreateCategoryDialog } from "./CreateCategoryDialog";

const mocks = vi.hoisted(() => ({
  categoryList: vi.fn(),
  createCategory: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    categoryList: mocks.categoryList,
    createCategory: mocks.createCategory,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

beforeEach(() => {
  mocks.categoryList.mockResolvedValue(ok([]));
  mocks.createCategory.mockResolvedValue(
    ok({ category_id: "new", mutation: { op_seq: 1, replayed: false } }),
  );
});

describe("CreateCategoryDialog", () => {
  it("Escape on the open emoji panel dismisses the panel only, not the whole form (4d8.25.19 review)", async () => {
    const onClose = vi.fn();
    renderWithClient(
      <CreateCategoryDialog
        initialName="Coffee shops"
        onCreated={vi.fn()}
        onClose={onClose}
      />,
    );
    // The form is prefilled with the search text.
    expect(await screen.findByLabelText("Category name")).toHaveValue("Coffee shops");

    // Open the emoji picker, then press Escape to dismiss just the grid.
    fireEvent.click(screen.getByRole("button", { name: "Category icon" }));
    const emojiPanel = await screen.findByRole("dialog", { name: "Pick an emoji" });
    fireEvent.keyDown(screen.getByLabelText("Filter emoji"), { key: "Escape" });

    // The emoji panel closes; the dialog and its typed name survive.
    await waitFor(() =>
      expect(
        screen.queryByRole("dialog", { name: "Pick an emoji" }),
      ).not.toBeInTheDocument(),
    );
    expect(emojiPanel).toBeDefined();
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Category name")).toHaveValue("Coffee shops");

    // A second Escape (no inner popover open) closes the dialog.
    fireEvent.keyDown(screen.getByLabelText("Category name"), { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("starts in the name field, keeps Tab inside, and hides the page behind it (shared Dialog, 4d8.6.2)", async () => {
    renderWithClient(
      <div>
        <button>Opener</button>
        <CreateCategoryDialog onCreated={vi.fn()} onClose={vi.fn()} />
      </div>,
    );
    const dialog = await screen.findByRole("dialog", { name: "New category" });
    await waitFor(() => expect(screen.getByLabelText("Category name")).toHaveFocus());
    const create = screen.getByRole("button", { name: "Create category" });
    create.focus();
    fireEvent.keyDown(create, { key: "Tab" });
    expect(dialog).toContainElement(document.activeElement as HTMLElement);
    expect(screen.queryByRole("button", { name: "Opener" })).not.toBeInTheDocument();
  });

  it("Escape closes only this dialog: a listener on the window behind never sees the key", async () => {
    const onClose = vi.fn();
    renderWithClient(<CreateCategoryDialog onCreated={vi.fn()} onClose={onClose} />);
    const behind = vi.fn();
    window.addEventListener("keydown", behind);
    const dialog = await screen.findByRole("dialog", { name: "New category" });
    fireEvent.keyDown(dialog, { key: "Escape" });
    window.removeEventListener("keydown", behind);
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(behind).not.toHaveBeenCalled();
  });

  it("the Close button and Cancel close it without creating a category", async () => {
    const onClose = vi.fn();
    renderWithClient(<CreateCategoryDialog onCreated={vi.fn()} onClose={onClose} />);
    await screen.findByRole("dialog", { name: "New category" });
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(mocks.createCategory).not.toHaveBeenCalled();
  });

  it("lets the emoji panel and parent list overflow: both the dialog and its body are overflow-visible, not scrolling", async () => {
    renderWithClient(<CreateCategoryDialog onCreated={vi.fn()} onClose={vi.fn()} />);
    const dialog = await screen.findByRole("dialog", { name: "New category" });
    expect(dialog.className).toMatch(/\boverflow-visible\b/);
    // On a phone the sheet itself scrolls (the sheet is the scroll container there): a class that only compiles under `compact:`.
    expect(dialog.className).toMatch(/\bcompact:overflow-y-auto\b/);
    expect(dialog.className).toMatch(/\bcompact:overflow-x-hidden\b/);
    // The wrapper the form sits in (DialogBody), by itself: it defaults to scrolling.
    const body = screen.getByLabelText("Category name").closest("form")!.parentElement!.parentElement!.parentElement!;
    expect(body.className).toMatch(/\boverflow-visible\b/);
    expect(body.className).not.toMatch(/overflow-y-auto/);
  });

  describe("focus returns to the control that opened it (4d8.6.2 R1)", () => {
    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <div>
          <button onClick={() => setOpen(true)}>Create new category…</button>
          {open && <CreateCategoryDialog onCreated={() => undefined} onClose={() => setOpen(false)} />}
        </div>
      );
    }
    async function openFromFocusedOpener() {
      renderWithClient(<Harness />);
      const opener = screen.getByRole("button", { name: "Create new category…" });
      opener.focus();
      fireEvent.click(opener);
      const dialog = await screen.findByRole("dialog", { name: "New category" });
      // The name field starts focused, as it always has.
      await waitFor(() => expect(screen.getByLabelText("Category name")).toHaveFocus());
      return { opener, dialog };
    }
    const closed = () => waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

    it("on Escape", async () => {
      const { opener, dialog } = await openFromFocusedOpener();
      fireEvent.keyDown(dialog, { key: "Escape" });
      await closed();
      await waitFor(() => expect(opener).toHaveFocus());
    });

    it("on Cancel", async () => {
      const { opener } = await openFromFocusedOpener();
      fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
      await closed();
      await waitFor(() => expect(opener).toHaveFocus());
    });

    it("on Close", async () => {
      const { opener } = await openFromFocusedOpener();
      fireEvent.click(screen.getByRole("button", { name: "Close" }));
      await closed();
      await waitFor(() => expect(opener).toHaveFocus());
    });

    it("on success", async () => {
      const { opener } = await openFromFocusedOpener();
      fireEvent.change(screen.getByLabelText("Category name"), { target: { value: "Coffee" } });
      fireEvent.click(screen.getByRole("button", { name: "Create category" }));
      await waitFor(() => expect(mocks.createCategory).toHaveBeenCalledTimes(1));
      await closed();
      await waitFor(() => expect(opener).toHaveFocus());
    });
  });
});

