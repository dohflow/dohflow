import { fireEvent, render, screen } from "@testing-library/react";

import type { CategoryDto } from "@/bindings";
import { matchRank } from "@/components/ui/combobox";
import { EmojiPicker } from "@/components/ui/emoji-picker";
import { CategoryCombobox } from "./CategoryCombobox";

function category(over: Partial<CategoryDto> & { id: string; name: string }): CategoryDto {
  return {
    parent_id: null,
    category_type: "expense",
    icon: null,
    color: null,
    is_system: false,
    forecast_behavior: "variable_regular",
    archived: false,
    ...over,
  };
}

/// The ADR 0045 §3 fixture: "Maintenance" exists under BOTH Housing and
/// Transportation — leaf names are not unique.
const TAXONOMY: CategoryDto[] = [
  category({ id: "food", name: "Food and Drink" }),
  category({ id: "groceries", name: "Groceries", parent_id: "food" }),
  category({ id: "housing", name: "Housing" }),
  category({ id: "transport", name: "Transportation" }),
  category({ id: "maint-home", name: "Maintenance", parent_id: "housing" }),
  category({ id: "maint-car", name: "Maintenance", parent_id: "transport" }),
  category({ id: "archived", name: "Old Category", archived: true }),
];

describe("matchRank", () => {
  it("ranks leaf-prefix over word over path matches", () => {
    const item = { id: "x", label: "Groceries", hint: "Food and Drink" };
    expect(matchRank(item, "groc")).toBe(0);
    expect(matchRank({ id: "y", label: "Whole Groceries" }, "groc")).toBe(1);
    expect(matchRank(item, "drink")).toBe(2);
    expect(matchRank(item, "zzz")).toBeNull();
  });
});

describe("CategoryCombobox", () => {
  it("focuses the search input the moment the panel opens (ulg9)", () => {
    render(
      <CategoryCombobox
        categories={TAXONOMY}
        value={null}
        onSelect={vi.fn()}
        aria-label="Category"
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Category" }));
    // Click-then-type with no second click: the portaled input must hold focus
    // as soon as it exists (it mounts a render AFTER open flips, so an
    // [open]-keyed effect misses it — pinned here).
    expect(document.activeElement).toBe(screen.getByRole("combobox"));
  });

  it("hands focus back to the trigger when the panel closes via keyboard", () => {
    render(
      <CategoryCombobox
        categories={TAXONOMY}
        value={null}
        onSelect={vi.fn()}
        aria-label="Category"
      />,
    );
    const trigger = screen.getByRole("button", { name: "Category" });
    fireEvent.click(trigger);
    const input = screen.getByRole("combobox");
    expect(document.activeElement).toBe(input);
    fireEvent.keyDown(input, { key: "Escape" });
    // Without the handoff, focus falls to <body> and tab order restarts.
    expect(document.activeElement).toBe(trigger);
  });

  it("matches the LEAF name as typed — the owner's Groceries case (4d8.25.18)", () => {
    const onSelect = vi.fn();
    render(
      <CategoryCombobox
        categories={TAXONOMY}
        value={null}
        onSelect={onSelect}
        aria-label="Category"
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Category" }));
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "Groceries" } });
    fireEvent.pointerDown(screen.getByRole("option", { name: /Groceries/ }));
    expect(onSelect).toHaveBeenCalledWith("groceries");
  });

  it("disambiguates duplicate leaf names with parent context (ADR 0045 §3)", () => {
    render(
      <CategoryCombobox
        categories={TAXONOMY}
        value={null}
        onSelect={vi.fn()}
        aria-label="Category"
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Category" }));
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "mainten" } });
    expect(screen.getByRole("option", { name: /Maintenance · Housing/ })).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: /Maintenance · Transportation/ }),
    ).toBeInTheDocument();
  });

  it("excludes archived categories and supports keyboard selection", () => {
    const onSelect = vi.fn();
    render(
      <CategoryCombobox
        categories={TAXONOMY}
        value={null}
        onSelect={onSelect}
        aria-label="Category"
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Category" }));
    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "old cat" } });
    expect(screen.queryByRole("option", { name: /Old Category/ })).not.toBeInTheDocument();
    fireEvent.change(input, { target: { value: "groc" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onSelect).toHaveBeenCalledWith("groceries");
  });

  it("shows the clear row only when the query matches it — a no-match Enter is inert (4d8.25.18 review)", () => {
    const onSelect = vi.fn();
    render(
      <CategoryCombobox
        categories={TAXONOMY}
        value="groceries"
        onSelect={onSelect}
        clearLabel="Uncategorized"
        aria-label="Category"
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Category" }));
    // Empty query: the clear row is offered.
    expect(screen.getByRole("option", { name: "Uncategorized" })).toBeInTheDocument();
    // A no-match typo hides the clear row and shows "No matches" — Enter must NOT
    // silently clear the category (was a real recategorize-to-null IPC).
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "zzznope" } });
    expect(screen.queryByRole("option", { name: "Uncategorized" })).not.toBeInTheDocument();
    expect(screen.getByText("No matches")).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole("combobox"), { key: "Enter" });
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("Enter on a fresh no-match name opens create-new (4d8.25.18/.19 review)", () => {
    const onCreateNew = vi.fn();
    render(
      <CategoryCombobox
        categories={TAXONOMY}
        value={null}
        onSelect={vi.fn()}
        clearLabel="Uncategorized"
        onCreateNew={onCreateNew}
        aria-label="Category"
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Category" }));
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "Coffee shops" } });
    // No matching category → the create footer is the active row, so Enter creates.
    fireEvent.keyDown(screen.getByRole("combobox"), { key: "Enter" });
    expect(onCreateNew).toHaveBeenCalledWith("Coffee shops");
  });

  it("offers the create-new footer with the current query (4d8.25.19)", () => {
    const onCreateNew = vi.fn();
    render(
      <CategoryCombobox
        categories={TAXONOMY}
        value={null}
        onSelect={vi.fn()}
        onCreateNew={onCreateNew}
        aria-label="Category"
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Category" }));
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "Coffee shops" } });
    fireEvent.pointerDown(screen.getByRole("button", { name: "Create new category…" }));
    expect(onCreateNew).toHaveBeenCalledWith("Coffee shops");
  });
});

describe("EmojiPicker", () => {
  it("commits only by picking — the filter text can never become the value (4d8.25.20)", () => {
    const onSelect = vi.fn();
    render(<EmojiPicker value="" onSelect={onSelect} aria-label="Category icon" />);
    fireEvent.click(screen.getByRole("button", { name: "Category icon" }));
    const filter = screen.getByLabelText("Filter emoji");
    // Raw text + Enter commits nothing.
    fireEvent.change(filter, { target: { value: "abc" } });
    fireEvent.keyDown(filter, { key: "Enter" });
    expect(onSelect).not.toHaveBeenCalled();
    // Keyword filter + click commits the emoji.
    fireEvent.change(filter, { target: { value: "coffee" } });
    fireEvent.click(screen.getByRole("button", { name: /Emoji ☕/ }));
    expect(onSelect).toHaveBeenCalledWith("☕");
  });
});
