import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { useLayoutEffect, useRef, useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { Disclosure, usePinDisclosures } from "@/components/ui/disclosure";
import sourceTree from "virtual:source-tree";

afterEach(cleanup);

function trigger(name: string) {
  return screen.getByRole("button", { name });
}

describe("Disclosure trigger", () => {
  it("is one native button inside the heading, with a stable name, aria-expanded and aria-controls", () => {
    render(
      <Disclosure title="Backups" summary="Daily · last verified Tuesday" defaultOpen={false}>
        <p>Body</p>
      </Disclosure>,
    );
    const heading = screen.getByRole("heading", { level: 2 });
    const button = within(heading).getByRole("button");
    // A native <button type="button"> is what gives Enter and Space activation and a place in
    // the Tab order for free (jsdom does not synthesize the click from a key press, so the
    // keyboard path itself is covered by the runtime accessibility check on the built page).
    expect(button.tagName).toBe("BUTTON");
    expect(button).toHaveAttribute("type", "button");
    expect(button).toHaveAccessibleName("Backups");
    expect(button).toHaveAttribute("aria-expanded", "false");
    const body = document.getElementById(button.getAttribute("aria-controls") ?? "");
    expect(body).not.toBeNull();
    expect(body).toHaveTextContent("Body");

    fireEvent.click(button);
    expect(button).toHaveAttribute("aria-expanded", "true");
    // The name does not change with the state or the summary.
    expect(button).toHaveAccessibleName("Backups");
  });

  it("describes the trigger with the summary only while collapsed", () => {
    render(
      <Disclosure title="Backups" summary="Daily">
        <p>Body</p>
      </Disclosure>,
    );
    expect(trigger("Backups")).toHaveAccessibleDescription("Daily");
    fireEvent.click(trigger("Backups"));
    expect(trigger("Backups")).not.toHaveAccessibleDescription();
    expect(screen.queryByText("Daily")).toBeNull();
  });

  it("uses the heading level it is given", () => {
    render(
      <Disclosure title="Backups" headingLevel={3}>
        <p>Body</p>
      </Disclosure>,
    );
    expect(screen.getByRole("heading", { level: 3 })).toBeInTheDocument();
  });

  it("names the body as a region only when asked", () => {
    const { rerender } = render(
      <Disclosure title="Backups" defaultOpen>
        <p>Body</p>
      </Disclosure>,
    );
    expect(screen.queryByRole("region")).toBeNull();
    rerender(
      <Disclosure title="Backups" defaultOpen region>
        <p>Body</p>
      </Disclosure>,
    );
    expect(screen.getByRole("region", { name: "Backups" })).toBeInTheDocument();
  });
});

describe("Disclosure hidden body", () => {
  it("leaves the accessibility tree and the Tab order when collapsed, and keeps drafts", () => {
    render(
      <Disclosure title="Comfort band" defaultOpen>
        <input aria-label="Lower" defaultValue="" />
      </Disclosure>,
    );
    const input = screen.getByRole("textbox", { name: "Lower" });
    fireEvent.change(input, { target: { value: "1200" } });

    fireEvent.click(trigger("Comfort band"));
    // Not in the accessibility tree...
    expect(screen.queryByRole("textbox", { name: "Lower" })).toBeNull();
    // ...but still mounted, hidden and inert, so the draft is kept and Tab skips it.
    const hiddenInput = screen.getByRole("textbox", { name: "Lower", hidden: true });
    const body = hiddenInput.closest("[hidden]");
    expect(body).not.toBeNull();
    expect(body).toHaveAttribute("inert");
    expect(hiddenInput).toHaveValue("1200");

    fireEvent.click(trigger("Comfort band"));
    expect(screen.getByRole("textbox", { name: "Lower" })).toHaveValue("1200");
    expect(document.querySelector("[inert]")).toBeNull();
  });

  it("returns focus to the trigger before hiding focused content", () => {
    function Controlled() {
      const [open, setOpen] = useState(true);
      return (
        <>
          <button type="button" onClick={() => setOpen(false)}>
            Collapse from outside
          </button>
          <Disclosure title="Comfort band" open={open} onOpenChange={setOpen}>
            <input aria-label="Lower" />
          </Disclosure>
        </>
      );
    }
    render(<Controlled />);
    screen.getByRole("textbox", { name: "Lower" }).focus();
    expect(screen.getByRole("textbox", { name: "Lower" })).toHaveFocus();

    fireEvent.click(screen.getByRole("button", { name: "Collapse from outside" }));
    expect(trigger("Comfort band")).toHaveAttribute("aria-expanded", "false");
    expect(trigger("Comfort band")).toHaveFocus();
  });

  it("returns focus to the trigger when focused content is collapsed from the trigger itself", () => {
    render(
      <Disclosure title="Comfort band" defaultOpen>
        <input aria-label="Lower" />
      </Disclosure>,
    );
    // jsdom does not move focus on a click the way a browser does, so focus is still inside the
    // body when the trigger fires; the section must hand it to the trigger, not strand it.
    screen.getByRole("textbox", { name: "Lower" }).focus();
    fireEvent.click(trigger("Comfort band"));
    expect(trigger("Comfort band")).toHaveFocus();
  });

  it("never submits, resets or cancels anything by collapsing", () => {
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
    const onReset = vi.fn();
    render(
      <Disclosure title="Comfort band" defaultOpen>
        <form onSubmit={onSubmit} onReset={onReset}>
          <input aria-label="Lower" defaultValue="" />
          <button type="submit">Save</button>
        </form>
      </Disclosure>,
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Lower" }), { target: { value: "9" } });
    fireEvent.click(trigger("Comfort band"));
    fireEvent.click(trigger("Comfort band"));
    expect(onSubmit).not.toHaveBeenCalled();
    expect(onReset).not.toHaveBeenCalled();
    expect(screen.getByRole("textbox", { name: "Lower" })).toHaveValue("9");
  });
});

describe("Disclosure independence", () => {
  it("expands and collapses sections independently, in a stable DOM order", () => {
    render(
      <>
        <Disclosure title="One" defaultOpen>
          <p>one body</p>
        </Disclosure>
        <Disclosure title="Two">
          <p>two body</p>
        </Disclosure>
        <Disclosure title="Three">
          <p>three body</p>
        </Disclosure>
      </>,
    );
    const order = () => screen.getAllByRole("button").map((b) => b.textContent);
    expect(order()).toEqual(["One", "Two", "Three"]);

    fireEvent.click(trigger("Two"));
    expect(trigger("One")).toHaveAttribute("aria-expanded", "true");
    expect(trigger("Two")).toHaveAttribute("aria-expanded", "true");
    expect(trigger("Three")).toHaveAttribute("aria-expanded", "false");
    expect(order()).toEqual(["One", "Two", "Three"]);

    fireEvent.click(trigger("One"));
    expect(trigger("One")).toHaveAttribute("aria-expanded", "false");
    expect(trigger("Two")).toHaveAttribute("aria-expanded", "true");
  });

  it("starts from defaultOpen and writes no persistence", () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem");
    render(
      <Disclosure title="Backups">
        <p>Body</p>
      </Disclosure>,
    );
    fireEvent.click(trigger("Backups"));
    expect(setItem).not.toHaveBeenCalled();
    setItem.mockRestore();
  });
});

describe("Disclosure persistent slot", () => {
  it("keeps an error and exactly one retry outside the body, open or collapsed", () => {
    const onRetry = vi.fn();
    render(
      <Disclosure
        title="Connections"
        defaultOpen
        persistent={
          <div>
            <p role="alert">Could not refresh. Your last balances are from Monday.</p>
            <button type="button" onClick={onRetry}>
              Refresh now
            </button>
          </div>
        }
      >
        <p>Body</p>
      </Disclosure>,
    );
    expect(screen.getAllByRole("button", { name: "Refresh now" })).toHaveLength(1);

    fireEvent.click(trigger("Connections"));
    expect(trigger("Connections")).toHaveAttribute("aria-expanded", "false");
    // Still exposed once, and reachable: it is not inside the hidden body.
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    const retry = screen.getByRole("button", { name: "Refresh now" });
    expect(retry.closest("[hidden]")).toBeNull();
    expect(retry.closest("[inert]")).toBeNull();

    fireEvent.click(retry);
    expect(onRetry).toHaveBeenCalledTimes(1);
  });
});

/// A destructive confirmation in the shape the Settings security group will use: open, then
/// armed by typing, and a source Cancel that clears it and puts focus back.
function DeleteSection({ busy = false }: { busy?: boolean }) {
  const [confirming, setConfirming] = useState(false);
  const [armed, setArmed] = useState("");
  const startRef = useRef<HTMLButtonElement>(null);
  usePinDisclosures(confirming || busy);
  return (
    <div>
      <button ref={startRef} type="button" onClick={() => setConfirming(true)}>
        Delete vault
      </button>
      {confirming && (
        <div role="group" aria-label="Confirm delete">
          <input
            aria-label="Type DELETE"
            value={armed}
            onChange={(e) => setArmed(e.target.value)}
          />
          <button
            type="button"
            onClick={() => {
              setConfirming(false);
              setArmed("");
              startRef.current?.focus();
            }}
          >
            Cancel
          </button>
        </div>
      )}
    </div>
  );
}

/// The other real shape: the start control is REPLACED by the confirmation, so it is not mounted
/// while confirming and focus can only go back to it after the re-render.
function ReplacingDeleteSection() {
  const [confirming, setConfirming] = useState(false);
  const [returnFocus, setReturnFocus] = useState(false);
  const startRef = useRef<HTMLButtonElement>(null);
  usePinDisclosures(confirming);
  useLayoutEffect(() => {
    if (returnFocus && !confirming) {
      startRef.current?.focus();
      setReturnFocus(false);
    }
  }, [returnFocus, confirming]);
  return confirming ? (
    <div role="group" aria-label="Confirm delete">
      <button
        type="button"
        onClick={() => {
          setConfirming(false);
          setReturnFocus(true);
        }}
      >
        Cancel
      </button>
    </div>
  ) : (
    <button ref={startRef} type="button" onClick={() => setConfirming(true)}>
      Delete vault
    </button>
  );
}

describe("Disclosure pinning", () => {
  it("returns focus to a start control that was unmounted while confirming", () => {
    render(
      <Disclosure title="Section" defaultOpen>
        <ReplacingDeleteSection />
      </Disclosure>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Delete vault" }));
    expect(screen.queryByRole("button", { name: "Delete vault" })).toBeNull();
    screen.getByRole("button", { name: "Cancel" }).focus();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(screen.getByRole("button", { name: "Delete vault" })).toHaveFocus();
    expect(trigger("Section")).toHaveAttribute("aria-expanded", "true");
    expect(trigger("Section")).not.toHaveAttribute("aria-disabled");
  });

  it("does not request a collapse while pinned", () => {
    const onOpenChange = vi.fn();
    render(
      <Disclosure title="Section" defaultOpen onOpenChange={onOpenChange}>
        <DeleteSection busy />
      </Disclosure>,
    );
    fireEvent.click(trigger("Section"));
    expect(onOpenChange).not.toHaveBeenCalled();
  });

  function Nested({ busy = false }: { busy?: boolean }) {
    return (
      <Disclosure title="Password, encryption and deletion" defaultOpen>
        <Disclosure title="Delete this vault" defaultOpen headingLevel={3}>
          <DeleteSection busy={busy} />
        </Disclosure>
      </Disclosure>
    );
  }

  it("pins every ancestor open while a confirmation is open, and keeps the triggers focusable", () => {
    render(<Nested />);
    fireEvent.click(screen.getByRole("button", { name: "Delete vault" }));

    for (const name of ["Password, encryption and deletion", "Delete this vault"]) {
      const t = trigger(name);
      expect(t).toHaveAttribute("aria-disabled", "true");
      expect(t).not.toBeDisabled();
      t.focus();
      expect(t).toHaveFocus();
      fireEvent.click(t);
      expect(t).toHaveAttribute("aria-expanded", "true");
    }
    expect(screen.getByRole("textbox", { name: "Type DELETE" })).toBeInTheDocument();
  });

  it("Cancel clears the confirmation, returns focus and leaves the section open", () => {
    render(<Nested />);
    fireEvent.click(screen.getByRole("button", { name: "Delete vault" }));
    // Collapse attempts while pinned are ignored, not merely hidden by the pin: once the pin is
    // released the sections must still be open.
    fireEvent.click(trigger("Delete this vault"));
    fireEvent.click(trigger("Password, encryption and deletion"));
    fireEvent.change(screen.getByRole("textbox", { name: "Type DELETE" }), {
      target: { value: "DELETE" },
    });
    screen.getByRole("button", { name: "Cancel" }).focus();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(screen.queryByRole("group", { name: "Confirm delete" })).toBeNull();
    expect(screen.getByRole("button", { name: "Delete vault" })).toHaveFocus();
    // Both sections are still open, and no longer pinned.
    for (const name of ["Password, encryption and deletion", "Delete this vault"]) {
      expect(trigger(name)).toHaveAttribute("aria-expanded", "true");
      expect(trigger(name)).not.toHaveAttribute("aria-disabled");
    }
    // A later collapse works again.
    fireEvent.click(trigger("Delete this vault"));
    expect(trigger("Delete this vault")).toHaveAttribute("aria-expanded", "false");
  });

  it("stays pinned while an irreversible operation is busy, even with no confirmation open", () => {
    render(<Nested busy />);
    for (const name of ["Password, encryption and deletion", "Delete this vault"]) {
      expect(trigger(name)).toHaveAttribute("aria-disabled", "true");
      fireEvent.click(trigger(name));
      expect(trigger(name)).toHaveAttribute("aria-expanded", "true");
    }
  });

  it("only pins the sections above the confirmation, not their siblings", () => {
    render(
      <>
        <Disclosure title="Pinned parent" defaultOpen>
          <DeleteSection busy />
        </Disclosure>
        <Disclosure title="Sibling" defaultOpen>
          <p>Body</p>
        </Disclosure>
      </>,
    );
    expect(trigger("Pinned parent")).toHaveAttribute("aria-disabled", "true");
    expect(trigger("Sibling")).not.toHaveAttribute("aria-disabled");
    fireEvent.click(trigger("Sibling"));
    expect(trigger("Sibling")).toHaveAttribute("aria-expanded", "false");
  });

  it("opens a collapsed section that gains a pin", () => {
    function Late() {
      const [busy, setBusy] = useState(false);
      return (
        <>
          <button type="button" onClick={() => setBusy(true)}>
            Start
          </button>
          <Disclosure title="Section">
            <DeleteSection busy={busy} />
          </Disclosure>
        </>
      );
    }
    render(<Late />);
    expect(trigger("Section")).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(screen.getByRole("button", { name: "Start" }));
    expect(trigger("Section")).toHaveAttribute("aria-expanded", "true");
  });
});

describe("Disclosure motion", () => {
  it("turns the chevron transition off for reduced motion", () => {
    const source = sourceTree.find((f) => f.path === "components/ui/disclosure.tsx")?.text ?? "";
    expect(source).toContain("transition-transform motion-reduce:transition-none");
    // The body does not animate at all: no height or opacity transition to respect.
    expect(source).not.toMatch(/transition-(all|\[height|height|opacity)/);
  });
});
