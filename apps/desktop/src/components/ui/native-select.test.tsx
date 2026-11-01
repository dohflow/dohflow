import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { NativeSelect } from "@/components/ui/native-select";

afterEach(cleanup);

// personal-cfo-4d8.35: a base `pr-8` followed by a size's `px-*` was dropped by tailwind-merge, so a long
// option ran under the chevron. The chevron sits 10px from the edge: 26px of icon area at the default
// size (16px icon) and 22px at `sm` (12px icon), so the select's own right padding must clear that.
describe("NativeSelect right padding clears the chevron", () => {
  it.each([
    ["default", "pr-8", "pl-3"],
    ["sm", "pr-7", "pl-2.5"],
  ] as const)("keeps %s's %s after the size's left padding", (size, right, left) => {
    render(
      <NativeSelect aria-label="Pick" size={size}>
        <option>A deliberately long option label that would run under the chevron</option>
      </NativeSelect>,
    );
    const classes = screen.getByRole("combobox", { name: "Pick" }).classList;
    expect(classes.contains(right)).toBe(true);
    expect(classes.contains(left)).toBe(true);
    // A horizontal `px-*` would let tailwind-merge drop the right padding again.
    expect([...classes].filter((c) => /^px-/.test(c))).toEqual([]);
  });
});
