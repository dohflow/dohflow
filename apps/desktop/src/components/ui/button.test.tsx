import { fireEvent, render, screen } from "@testing-library/react";

import { Button } from "./button";

const VARIANTS = ["default", "secondary", "outline", "ghost", "destructive", "link"] as const;

describe("Button disabled state (personal-cfo-4d8.52)", () => {
  it.each(VARIANTS)("%s: disabled keeps native semantics: no activation, not focusable, not aria-disabled", (variant) => {
    const onClick = vi.fn();
    render(
      <Button variant={variant} disabled onClick={onClick}>
        Save
      </Button>,
    );
    const button = screen.getByRole("button", { name: "Save" });
    expect(button).toBeDisabled();
    expect(button).not.toHaveAttribute("aria-disabled");
    fireEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
    button.focus();
    expect(button).not.toHaveFocus();
  });

  it.each(VARIANTS)("%s: enabled still activates and takes focus", (variant) => {
    const onClick = vi.fn();
    render(
      <Button variant={variant} onClick={onClick}>
        Save
      </Button>,
    );
    const button = screen.getByRole("button", { name: "Save" });
    button.focus();
    expect(button).toHaveFocus();
    fireEvent.click(button);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("a disabled button's progress indicator stays readable beside its name", () => {
    render(
      <Button disabled>
        <span role="status">Saving…</span>
        Save
      </Button>,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Saving…");
  });
});
