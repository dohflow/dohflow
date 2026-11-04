import { fireEvent, render, screen } from "@testing-library/react";

import { SettingsSection } from "./SettingsSection";

describe("SettingsSection persistent slot (personal-cfo-4d8.39.1)", () => {
  function sectionWith(persistent?: React.ReactNode) {
    return render(
      <SettingsSection title="Backups" span="full" persistent={persistent}>
        <p>the card</p>
      </SettingsSection>,
    );
  }

  it("shows the persistent content once, outside the body, and keeps it when the section is collapsed", () => {
    sectionWith(<p role="alert">Could not save.</p>);
    const alerts = screen.getAllByRole("alert");
    expect(alerts).toHaveLength(1);
    const trigger = screen.getByRole("button", { name: "Backups" });
    const body = document.getElementById(trigger.getAttribute("aria-controls")!)!;
    expect(body.contains(alerts[0]!)).toBe(false);

    fireEvent.click(trigger);
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(body).toHaveAttribute("hidden");
    // Still there, once, and not inside anything hidden or inert.
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(screen.getByRole("alert")).toBeVisible();
    expect(screen.getByRole("alert").closest("[hidden], [inert]")).toBeNull();
    expect(screen.getByText("Could not save.")).toBeInTheDocument();
  });

  it("adds nothing when no persistent content is given", () => {
    const { container } = sectionWith();
    expect(screen.queryByRole("alert")).toBeNull();
    const withSlot = render(
      <SettingsSection title="Backups" span="full" persistent={<span>x</span>}>
        <p>the card</p>
      </SettingsSection>,
    );
    expect(container.querySelectorAll("div").length).toBeLessThan(
      withSlot.container.querySelectorAll("div").length,
    );
  });
});
