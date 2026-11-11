import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { NativeSelect, NativeSelectValue } from "@/components/ui/native-select";

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

// personal-cfo-i93n3.28: a native select cannot wrap, so a narrow container shows its selected label as text.
describe("NativeSelectValue", () => {
  it("is for the eyes only, hidden unless a compact container is narrow, and breaks long names", () => {
    render(<NativeSelectValue data-testid="v">A very long account name</NativeSelectValue>);
    const copy = screen.getByTestId("v");
    expect(copy).toHaveAttribute("aria-hidden", "true");
    expect(copy).toHaveClass("hidden", "compact:@max-[15rem]:block", "[overflow-wrap:anywhere]");
    expect(copy).not.toHaveClass("block");
  });

  it("uses a narrower threshold for one column of two in a form row", () => {
    render(<NativeSelectValue narrow="field" data-testid="v">Weekly</NativeSelectValue>);
    expect(screen.getByTestId("v")).toHaveClass("hidden", "compact:@max-[10rem]:block");
    expect(screen.getByTestId("v")).not.toHaveClass("compact:@max-[15rem]:block");
  });
});
