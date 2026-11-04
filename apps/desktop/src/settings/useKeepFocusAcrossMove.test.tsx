/**
 * The shared focus hand-over (personal-cfo-4d8.39.2): a button that moves between the card body and
 * the persistent slot keeps focus if it had it, waits while its action is settling, and never takes
 * focus from a control the person has moved to.
 */

import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";

import { SectionFocusContext } from "./sectionFocusContext";
import { useKeepFocusAcrossMove } from "./useKeepFocusAcrossMove";

function Harness({
  settled = true,
  fallback,
}: {
  settled?: boolean;
  fallback?: () => HTMLElement | null;
}) {
  const [inSlot, setInSlot] = useState(false);
  const keep = useKeepFocusAcrossMove({ settled, fallback });
  const action = (
    <button
      ref={keep.ref}
      type="button"
      onClick={() => {
        keep.remember();
        setInSlot(true);
      }}
    >
      action
    </button>
  );
  return (
    <>
      <div data-testid="body">{!inSlot && action}</div>
      <div data-testid="slot">{inSlot && action}</div>
      <button type="button">elsewhere</button>
    </>
  );
}

function SectionCard({ open, inSlot }: { open: boolean; inSlot: boolean }) {
  const keep = useKeepFocusAcrossMove({ fallback: () => document.getElementById("inside") });
  const action = (
    <button ref={keep.ref} type="button" onClick={() => keep.remember()}>
      action
    </button>
  );
  return (
    <>
      <div>{inSlot && action}</div>
      <div hidden={!open} {...(open ? {} : { inert: true })}>
        <button id="inside" type="button">
          inside
        </button>
        {!inSlot && action}
      </div>
    </>
  );
}

describe("useKeepFocusAcrossMove", () => {
  it("hands focus to the button where it lands, when it had focus", () => {
    render(<Harness />);
    const action = screen.getByRole("button", { name: "action" });
    action.focus();
    fireEvent.click(action);
    expect(screen.getByTestId("slot")).toContainElement(document.activeElement as HTMLElement);
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "action" }));
  });

  it("takes nothing from a control the person moved to while the action was pending", () => {
    const { rerender } = render(<Harness settled={false} />);
    const action = screen.getByRole("button", { name: "action" });
    action.focus();
    fireEvent.click(action);
    // The action is still settling; the person moves on.
    act(() => screen.getByRole("button", { name: "elsewhere" }).focus());
    rerender(<Harness settled />);
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "elsewhere" }));
  });

  it("does not hand focus anywhere while the action is still settling", () => {
    const { rerender } = render(<Harness settled={false} />);
    const action = screen.getByRole("button", { name: "action" });
    action.focus();
    fireEvent.click(action);
    // Moved, enabled, unsettled: the button is not focused yet (jsdom drops focus with the node).
    expect(document.activeElement).toBe(document.body);
    rerender(<Harness settled />);
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "action" }));
  });

  it("hands focus to the fallback's element when the button is gone", () => {
    function Gone({ shown }: { shown: boolean }) {
      const keep = useKeepFocusAcrossMove({ fallback: () => document.getElementById("elsewhere") });
      return (
        <>
          {shown ? (
            <button ref={keep.ref} type="button" onClick={() => keep.remember()}>
              action
            </button>
          ) : null}
          <button id="elsewhere" type="button">
            elsewhere
          </button>
        </>
      );
    }
    const { rerender } = render(<Gone shown />);
    const action = screen.getByRole("button", { name: "action" });
    action.focus();
    fireEvent.click(action);
    rerender(<Gone shown={false} />);
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "elsewhere" }));
  });

  /// The hook sits in a child of the section, as a card does; the provider is above it.
  function InSection({ open, inSlot }: { open: boolean; inSlot: boolean }) {
    return (
      <SectionFocusContext.Provider value={{ open, trigger: () => document.getElementById("heading") }}>
        <button id="heading" type="button">
          heading
        </button>
        <SectionCard open={open} inSlot={inSlot} />
      </SectionFocusContext.Provider>
    );
  }

  it("never focuses a target inside hidden or inert content, and uses the section's heading when it is collapsed", () => {
    // Collapsed: the action moves from the visible slot into the hidden body, and so does the
    // fallback; the heading is chosen, not "inside".
    const { rerender } = render(<InSection open={false} inSlot />);
    const action = screen.getByRole("button", { name: "action" });
    action.focus();
    fireEvent.click(action);
    rerender(<InSection open={false} inSlot={false} />);
    expect(document.activeElement).toBe(document.getElementById("heading"));
  });

  it("open: the button itself, in the body, is where focus goes", () => {
    const { rerender } = render(<InSection open inSlot />);
    const action = screen.getByRole("button", { name: "action" });
    action.focus();
    fireEvent.click(action);
    rerender(<InSection open inSlot={false} />);
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "action" }));
  });

  it("open, with nothing usable to go to, does not invent a destination: the heading is for a collapsed section only", () => {
    function Card({ shown }: { shown: boolean }) {
      const keep = useKeepFocusAcrossMove();
      return shown ? (
        <button ref={keep.ref} type="button" onClick={() => keep.remember()}>
          action
        </button>
      ) : null;
    }
    function Open({ shown }: { shown: boolean }) {
      return (
        <SectionFocusContext.Provider value={{ open: true, trigger: () => document.getElementById("heading") }}>
          <button id="heading" type="button">
            heading
          </button>
          <Card shown={shown} />
        </SectionFocusContext.Provider>
      );
    }
    const { rerender } = render(<Open shown />);
    const action = screen.getByRole("button", { name: "action" });
    action.focus();
    fireEvent.click(action);
    rerender(<Open shown={false} />);
    expect(document.activeElement).toBe(document.body);
  });
});
