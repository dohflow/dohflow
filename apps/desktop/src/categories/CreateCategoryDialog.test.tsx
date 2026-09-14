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
});
