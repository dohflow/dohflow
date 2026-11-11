import { render, screen } from "@testing-library/react";

import { Input } from "./input";

// Many desktop callers override the height (`h-8`, `h-7`, `h-6`). `cn()` merges `h-*` against `h-*`
// but keeps a `min-h-*` next to it, and a minimum height beats a smaller height, so the primitive
// must not carry one: a screen that needs a field that grows at the largest system text size opts in
// with `h-auto min-h-10` where it renders the field (personal-cfo-i93n3.25).
describe("Input", () => {
  // personal-cfo-i93n3.28 slice 8: a native date field has an intrinsic minimum width on iOS WebKit that min-width cannot lower, so every
  // date field drops its native appearance on a phone (compact-only); other field types are untouched.
  test("every date field drops its native appearance and fits its column on a phone, and no other type does", () => {
    render(
      <>
        <Input aria-label="when" type="date" />
        <Input aria-label="amount" inputMode="decimal" />
      </>,
    );
    expect(screen.getByLabelText("when").className).toContain("compact:[&[type=date]]:appearance-none");
    expect(screen.getByLabelText("when").className).toContain("compact:[&[type=date]]:max-w-full");
    // The rule is conditional on the type in the selector itself, so it is a no-op for other fields, and nothing is unconditional.
    expect(screen.getByLabelText("when").className).not.toMatch(/(^|\s)appearance-none/);
    expect(screen.getByLabelText("amount")).toHaveAttribute("inputmode", "decimal");
  });

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
