import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { CategoryDto } from "@/bindings";
import { CategoriesView } from "./CategoriesView";

const mocks = vi.hoisted(() => ({
  categoryList: vi.fn(),
  createCategory: vi.fn(),
  updateCategory: vi.fn(),
  moveCategory: vi.fn(),
  archiveCategory: vi.fn(),
  reinstateCategory: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    categoryList: mocks.categoryList,
    createCategory: mocks.createCategory,
    updateCategory: mocks.updateCategory,
    moveCategory: mocks.moveCategory,
    archiveCategory: mocks.archiveCategory,
    reinstateCategory: mocks.reinstateCategory,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const mutationOk = () => ok({ op_seq: 1, replayed: false });

function category(over: Partial<CategoryDto> = {}): CategoryDto {
  return {
    id: "id-1",
    parent_id: null,
    name: "Category",
    category_type: "expense",
    icon: null,
    color: null,
    is_system: false,
    forecast_behavior: "variable_regular",
    archived: false,
    ...over,
  };
}

// A small two-level taxonomy: a system group with a system leaf, plus one user
// category at the top level.
const TAXONOMY: CategoryDto[] = [
  category({ id: "grp", name: "Food and Drink", is_system: true }),
  category({ id: "leaf", name: "Groceries", parent_id: "grp", is_system: true }),
  category({ id: "user", name: "Coffee", is_system: false }),
];

beforeEach(() => {
  vi.clearAllMocks();
  mocks.categoryList.mockResolvedValue(ok(TAXONOMY));
  mocks.createCategory.mockResolvedValue(
    ok({ category_id: "new", mutation: { op_seq: 1, replayed: false } }),
  );
  mocks.updateCategory.mockResolvedValue(mutationOk());
  mocks.moveCategory.mockResolvedValue(mutationOk());
  mocks.archiveCategory.mockResolvedValue(mutationOk());
  mocks.reinstateCategory.mockResolvedValue(mutationOk());
});

describe("CategoriesView", () => {
  it("renders the tree; system categories carry a Default badge and are editable (kogu)", async () => {
    renderWithClient(<CategoriesView />);

    await screen.findByText("Food and Drink");
    expect(screen.getByText("Groceries")).toBeInTheDocument();
    expect(screen.getByText("Coffee")).toBeInTheDocument();
    // Every category — system or user — now offers Edit (system exposes appearance only).
    expect(
      screen.getByRole("button", { name: "Edit Food and Drink" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Edit Coffee" }),
    ).toBeInTheDocument();
    // Default badge shows on the seeded categories.
    expect(screen.getAllByText("Default").length).toBeGreaterThan(0);
  });

  it("creates a user category with an emoji + color at creation (kogu)", async () => {
    renderWithClient(<CategoriesView />);
    await screen.findByText("Coffee");

    fireEvent.click(screen.getByRole("button", { name: /add category/i }));
    fireEvent.change(screen.getByLabelText("Category name"), {
      target: { value: "Hobbies" },
    });
    // The add form offers a PICK-ONLY emoji + a native color picker (4d8.25.20).
    fireEvent.click(screen.getByRole("button", { name: "Category icon" }));
    fireEvent.click(screen.getByRole("button", { name: new RegExp("Emoji 🎨") }));
    fireEvent.change(screen.getByLabelText("Category color"), {
      target: { value: "#006341" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add category" }));

    await waitFor(() =>
      expect(mocks.createCategory).toHaveBeenCalledWith(
        expect.objectContaining({
          name: "Hobbies",
          category_type: "expense",
          icon: "🎨",
          color: "#006341",
        }),
      ),
    );
  });

  it("edits a default category's appearance but preserves its name (kogu)", async () => {
    renderWithClient(<CategoriesView />);
    await screen.findByText("Groceries");

    // "Groceries" is a system leaf — Edit is now offered.
    fireEvent.click(screen.getByRole("button", { name: "Edit Groceries" }));
    // Its name is read-only (identity immutable); appearance is editable.
    expect(screen.getByLabelText("Category name")).toHaveAttribute("readonly");
    fireEvent.click(screen.getByRole("button", { name: "Category icon" }));
    fireEvent.click(screen.getByRole("button", { name: new RegExp("Emoji 🥦") }));
    fireEvent.change(screen.getByLabelText("Category color"), {
      target: { value: "#006341" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocks.updateCategory).toHaveBeenCalledWith(
        expect.objectContaining({
          id: "leaf",
          name: "Groceries",
          icon: "🥦",
          color: "#006341",
        }),
      ),
    );
    // A system category is never re-parented.
    expect(mocks.moveCategory).not.toHaveBeenCalled();
  });

  it("renames a user category", async () => {
    renderWithClient(<CategoriesView />);
    await screen.findByText("Coffee");

    fireEvent.click(screen.getByRole("button", { name: "Edit Coffee" }));
    fireEvent.change(screen.getByLabelText("Category name"), {
      target: { value: "Coffee Shops" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocks.updateCategory).toHaveBeenCalledWith(
        expect.objectContaining({ id: "user", name: "Coffee Shops" }),
      ),
    );
  });

  it("sets an emoji icon on a user category (4d8.24.10)", async () => {
    renderWithClient(<CategoriesView />);
    await screen.findByText("Coffee");

    fireEvent.click(screen.getByRole("button", { name: "Edit Coffee" }));
    fireEvent.click(screen.getByRole("button", { name: "Category icon" }));
    fireEvent.click(screen.getByRole("button", { name: new RegExp("Emoji ☕") }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocks.updateCategory).toHaveBeenCalledWith(
        expect.objectContaining({ id: "user", icon: "☕" }),
      ),
    );
  });

  it("picks a color via the color picker (4d8.24.10)", async () => {
    renderWithClient(<CategoriesView />);
    await screen.findByText("Coffee");

    fireEvent.click(screen.getByRole("button", { name: "Edit Coffee" }));
    // Native <input type=color> emits a lowercase #rrggbb value.
    fireEvent.change(screen.getByLabelText("Category color"), {
      target: { value: "#006341" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocks.updateCategory).toHaveBeenCalledWith(
        expect.objectContaining({ id: "user", color: "#006341" }),
      ),
    );
  });

  it("clears a previously-set color back to null (4d8.24.10)", async () => {
    // A colored user category — the native picker can't be emptied, so a Clear button
    // is the only path back to null.
    mocks.categoryList.mockResolvedValue(
      ok([
        ...TAXONOMY.filter((c) => c.id !== "user"),
        category({ id: "user", name: "Coffee", color: "#006341" }),
      ]),
    );
    renderWithClient(<CategoriesView />);
    await screen.findByText("Coffee");

    fireEvent.click(screen.getByRole("button", { name: "Edit Coffee" }));
    fireEvent.click(screen.getByRole("button", { name: "Clear category color" }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocks.updateCategory).toHaveBeenCalledWith(
        expect.objectContaining({ id: "user", color: null }),
      ),
    );
  });

  it("archives a category", async () => {
    renderWithClient(<CategoriesView />);
    await screen.findByText("Coffee");

    fireEvent.click(screen.getByRole("button", { name: "Archive Coffee" }));

    await waitFor(() =>
      // The key is minted per action (personal-cfo-3fdd.5) — non-empty, not "".
      expect(mocks.archiveCategory).toHaveBeenCalledWith(
        "user",
        expect.stringMatching(/.+/),
      ),
    );
  });
});
