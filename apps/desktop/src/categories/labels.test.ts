import { describe, expect, it } from "vitest";

import type { CategoryDto } from "@/bindings";
import { categoryLabels } from "./labels";

function category(over: Partial<CategoryDto> & { id: string; name: string }): CategoryDto {
  return {
    parent_id: null,
    category_type: "expense",
    icon: null,
    color: null,
    is_system: true,
    forecast_behavior: "variable_regular",
    archived: false,
    ...over,
  };
}

// A 2-level taxonomy with a same-named leaf under two parents ("Maintenance" is under
// both Housing and Transportation, matching the real seed).
const HOUSING = "0190c000-0000-7000-8000-0000000000a0";
const FOOD = "0190c000-0000-7000-8000-0000000000b0";
const TRANSPORT = "0190c000-0000-7000-8000-0000000000c0";
const GROCERIES = "0190c000-0000-7000-8000-0000000000b1";
const MAINT_HOUSING = "0190c000-0000-7000-8000-0000000000a1";
const MAINT_TRANSPORT = "0190c000-0000-7000-8000-0000000000c1";

const CATEGORIES: CategoryDto[] = [
  category({ id: HOUSING, name: "Housing" }),
  category({ id: FOOD, name: "Food and Drink" }),
  category({ id: TRANSPORT, name: "Transportation" }),
  category({ id: GROCERIES, name: "Groceries", parent_id: FOOD }),
  category({ id: MAINT_HOUSING, name: "Maintenance", parent_id: HOUSING }),
  category({ id: MAINT_TRANSPORT, name: "Maintenance", parent_id: TRANSPORT }),
];

describe("categoryLabels", () => {
  it("leafLabel is the leaf name only; label is the full Parent / Leaf path", () => {
    const { label, leafLabel } = categoryLabels(CATEGORIES);

    // A child category: leaf-only vs full path.
    expect(leafLabel(GROCERIES)).toBe("Groceries");
    expect(label(GROCERIES)).toBe("Food and Drink / Groceries");

    // A top-level category shows its own name either way.
    expect(leafLabel(FOOD)).toBe("Food and Drink");
    expect(label(FOOD)).toBe("Food and Drink");

    // Null / unknown ids yield null.
    expect(leafLabel(null)).toBeNull();
    expect(leafLabel("missing")).toBeNull();
  });

  it("groups nest each parent's leaves for <optgroup>, keeping the parent selectable", () => {
    const { groups } = categoryLabels(CATEGORIES);

    const food = groups.find((g) => g.id === FOOD);
    expect(food?.name).toBe("Food and Drink");
    expect(food?.children.map((c) => c.name)).toEqual(["Groceries"]);

    // The same-named "Maintenance" leaves are disambiguated by their parent group, and
    // both remain distinct ids under their own parents.
    const housing = groups.find((g) => g.id === HOUSING);
    const transport = groups.find((g) => g.id === TRANSPORT);
    expect(housing?.children).toEqual([{ id: MAINT_HOUSING, name: "Maintenance" }]);
    expect(transport?.children).toEqual([
      { id: MAINT_TRANSPORT, name: "Maintenance" },
    ]);

    // Groups are the top-level categories only (leaves never appear as roots).
    expect(groups.map((g) => g.id).sort()).toEqual(
      [FOOD, HOUSING, TRANSPORT].sort(),
    );
  });

  it("keeps a deep (3-level) descendant in its top-level ancestor's group (never drops it)", () => {
    // Regression: a grandchild must not vanish from the picker (would misrender as
    // Uncategorized on the row, and be unassignable). Backend allows arbitrary depth.
    const ORGANIC = "0190c000-0000-7000-8000-0000000000b2";
    const deep = [
      ...CATEGORIES,
      category({ id: ORGANIC, name: "Organic", parent_id: GROCERIES }),
    ];
    const { groups } = categoryLabels(deep);
    const food = groups.find((g) => g.id === FOOD);
    // Both the child (Groceries) and the grandchild (Organic) sit under Food and Drink.
    expect(food?.children.map((c) => c.id).sort()).toEqual(
      [GROCERIES, ORGANIC].sort(),
    );
    // The grandchild is NOT promoted to its own top-level group.
    expect(groups.some((g) => g.id === ORGANIC)).toBe(false);
  });

  it("meta(id) carries the category's own name, icon, and color (4d8.24.10)", () => {
    const withMeta = [
      ...CATEGORIES,
      category({
        id: "0190c000-0000-7000-8000-0000000000d0",
        name: "Coffee",
        parent_id: FOOD,
        icon: "☕",
        color: "#006341",
      }),
    ];
    const { meta } = categoryLabels(withMeta);
    expect(meta("0190c000-0000-7000-8000-0000000000d0")).toEqual({
      name: "Coffee",
      icon: "☕",
      color: "#006341",
    });
    // A category with no icon/color still resolves (nulls), and null id -> null.
    expect(meta(GROCERIES)).toEqual({ name: "Groceries", icon: null, color: null });
    expect(meta(null)).toBeNull();
  });

  it("omits archived categories from options and groups", () => {
    const withArchived = [
      ...CATEGORIES,
      category({
        id: "0190c000-0000-7000-8000-0000000000ff",
        name: "Retired",
        parent_id: FOOD,
        archived: true,
      }),
    ];
    const { options, groups } = categoryLabels(withArchived);
    expect(options.some((o) => o.label.includes("Retired"))).toBe(false);
    expect(groups.find((g) => g.id === FOOD)?.children).toEqual([
      { id: GROCERIES, name: "Groceries" },
    ]);
  });
});
