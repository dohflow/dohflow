import { fireEvent, render, screen } from "@testing-library/react";

import { PersistentNotice } from "./PersistentSlot";
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

  it("reserves an empty slot that takes no room when nothing is given", () => {
    sectionWith();
    expect(screen.queryByRole("alert")).toBeNull();
    const slot = document.querySelector("[data-settings-persistent]") as HTMLElement;
    expect(slot).toBeEmptyDOMElement();
    // The disclosure's wrapper hides an empty slot, so a healthy section gains no gap.
    expect(slot.parentElement).toHaveClass("has-[>:only-child:empty]:hidden");
  });
});

describe("PersistentNotice (personal-cfo-4d8.39.2)", () => {
  function Card() {
    return (
      <div>
        <input aria-label="draft" />
        <PersistentNotice>
          <p role="alert">Could not save.</p>
        </PersistentNotice>
      </div>
    );
  }

  it("lands in the section's slot, outside the body, and stays once when collapsed", () => {
    render(
      <SettingsSection title="Backups" span="full">
        <Card />
      </SettingsSection>,
    );
    const trigger = screen.getByRole("button", { name: "Backups" });
    const body = document.getElementById(trigger.getAttribute("aria-controls")!)!;
    const alert = screen.getByRole("alert");
    expect(body.contains(alert)).toBe(false);
    expect(alert.closest("[data-settings-persistent]")).not.toBeNull();

    fireEvent.change(screen.getByLabelText("draft"), { target: { value: "kept" } });
    fireEvent.click(trigger);
    expect(body).toHaveAttribute("hidden");
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(screen.getByRole("alert")).toBeVisible();
    // The card is still mounted, so its draft survives the collapse.
    fireEvent.click(trigger);
    expect(screen.getByLabelText("draft")).toHaveValue("kept");
  });

  it("renders in place when the card is shown outside a section", () => {
    render(<Card />);
    expect(screen.getByRole("alert")).toHaveTextContent("Could not save.");
  });
});
