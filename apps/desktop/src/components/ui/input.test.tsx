import { render, screen } from "@testing-library/react";

import { Input } from "./input";

// Many desktop callers override the height (`h-8`, `h-7`, `h-6`). `cn()` merges `h-*` against `h-*`
// but keeps a `min-h-*` next to it, and a minimum height beats a smaller height, so the primitive
// must not carry one: a screen that needs a field that grows at the largest system text size opts in
// with `h-auto min-h-10` where it renders the field (personal-cfo-i93n3.25).
describe("Input", () => {
  test("a caller's height replaces the default and no minimum height is left behind", () => {
    render(<Input aria-label="compact" className="h-8" />);
    const field = screen.getByLabelText("compact");
    expect(field).toHaveClass("h-8");
    expect(field).not.toHaveClass("h-10");
    expect(field.className).not.toMatch(/min-h-/);
  });

  test("by default the field is 40px tall with no minimum height", () => {
    render(<Input aria-label="plain" />);
    const field = screen.getByLabelText("plain");
    expect(field).toHaveClass("h-10");
    expect(field.className).not.toMatch(/min-h-/);
  });

  test("a call site can make the field grow with h-auto min-h-10", () => {
    render(<Input aria-label="grows" className="h-auto min-h-10" />);
    const field = screen.getByLabelText("grows");
    expect(field).toHaveClass("h-auto", "min-h-10");
    expect(field).not.toHaveClass("h-10");
  });
});
