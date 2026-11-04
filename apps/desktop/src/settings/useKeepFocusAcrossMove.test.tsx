/**
 * The shared focus hand-over (personal-cfo-4d8.39.2): a button that moves between the card body and
 * the persistent slot keeps focus if it had it, waits while its action is settling, and never takes
 * focus from a control the person has moved to.
 */

import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";

import { useKeepFocusAcrossMove } from "./useKeepFocusAcrossMove";

function Harness({ settled = true, fallback }: { settled?: boolean; fallback?: () => void }) {
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

  it("runs the fallback when the action settles and the button is gone", () => {
    const fallback = vi.fn();
    function Gone({ shown }: { shown: boolean }) {
      const keep = useKeepFocusAcrossMove({ fallback });
      return shown ? (
        <button ref={keep.ref} type="button" onClick={() => keep.remember()}>
          action
        </button>
      ) : null;
    }
    const { rerender } = render(<Gone shown />);
    const action = screen.getByRole("button", { name: "action" });
    action.focus();
    fireEvent.click(action);
    expect(fallback).not.toHaveBeenCalled();
    rerender(<Gone shown={false} />);
    expect(fallback).toHaveBeenCalledTimes(1);
  });
});
