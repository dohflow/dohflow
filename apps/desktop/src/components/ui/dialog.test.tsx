import { useRef, useState } from "react";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogError,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

/// A confirm opened from a button, the way a screen uses it. `removeOpener` simulates an action
/// whose own row disappears; `restoreFocus` passes an explicit return target.
function Harness({
  onConfirm = () => undefined,
  error = null,
  longBody = false,
  removeOpener = false,
  restoreFocus = false,
  focusCancel = false,
}: {
  onConfirm?: () => void;
  error?: string | null;
  longBody?: boolean;
  removeOpener?: boolean;
  restoreFocus?: boolean;
  focusCancel?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [openerGone, setOpenerGone] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const explicitRef = useRef<HTMLButtonElement>(null);
  const fallbackRef = useRef<HTMLButtonElement>(null);
  return (
    <div>
      <main>
        <button type="button" onClick={() => undefined}>
          Background action
        </button>
        {!openerGone && (
          <button type="button" onClick={() => setOpen(true)}>
            Open
          </button>
        )}
        <button type="button" ref={explicitRef}>
          Explicit target
        </button>
        <button type="button" ref={fallbackRef}>
          Fallback target
        </button>
      </main>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent
          initialFocusRef={focusCancel ? cancelRef : undefined}
          restoreFocusTo={restoreFocus ? explicitRef : undefined}
          fallbackFocusTo={fallbackRef}
        >
          <DialogHeader>
            <DialogTitle>Undo “Maternity leave”?</DialogTitle>
            <DialogDescription>This removes what applying added.</DialogDescription>
          </DialogHeader>
          <DialogBody data-testid="body">
            {longBody ? "x".repeat(120) : "Short body."}
          </DialogBody>
          <DialogError error={error} />
          <DialogFooter>
            <Button variant="ghost" ref={cancelRef} onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button
              onClick={() => {
                onConfirm();
                if (removeOpener) setOpenerGone(true);
                setOpen(false);
              }}
            >
              Undo scenario
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function openDialog() {
  const opener = screen.getByRole("button", { name: "Open" });
  opener.focus();
  fireEvent.click(opener);
  return opener;
}

describe("the shared Dialog (personal-cfo-4d8.6.1)", () => {
  it("names itself from its title and describes itself from its description", () => {
    render(<Harness />);
    openDialog();
    const dialog = screen.getByRole("dialog", { name: "Undo “Maternity leave”?" });
    expect(dialog).toHaveAccessibleDescription("This removes what applying added.");
  });

  it("keeps Tab and Shift+Tab inside the dialog", () => {
    render(<Harness />);
    openDialog();
    const dialog = screen.getByRole("dialog");
    const cancel = within(dialog).getByRole("button", { name: "Cancel" });
    const confirm = within(dialog).getByRole("button", { name: "Undo scenario" });

    confirm.focus();
    fireEvent.keyDown(confirm, { key: "Tab" });
    expect(document.activeElement).toBe(cancel);

    cancel.focus();
    fireEvent.keyDown(cancel, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(confirm);
    expect(dialog).toContainElement(document.activeElement as HTMLElement);
  });

  it("makes the background inert to assistive technology and unreachable by role", () => {
    const { container } = render(<Harness />);
    openDialog();
    // Everything outside the dialog's portal is hidden from assistive technology.
    expect(container).toHaveAttribute("aria-hidden", "true");
    expect(screen.queryByRole("button", { name: "Background action" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it("closes on Escape without running the confirm action", () => {
    const onConfirm = vi.fn();
    render(<Harness onConfirm={onConfirm} />);
    openDialog();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("focuses the chosen element on open, so a confirm can start on its safe action", () => {
    render(<Harness focusCancel />);
    openDialog();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel" }));
  });

  it("returns focus to the element that opened it", async () => {
    render(<Harness />);
    const opener = openDialog();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    // Radix hands focus back on the next task.
    await waitFor(() => expect(document.activeElement).toBe(opener));
  });

  it("returns focus to an explicit target when one is given (an opener that takes no focus)", async () => {
    render(<Harness restoreFocus />);
    openDialog();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Explicit target" })),
    );
  });

  it("uses the fallback when the caller's explicit target is empty, ignoring the opener", async () => {
    function Explicit() {
      const [open, setOpen] = useState(false);
      const none = useRef<HTMLElement | null>(null);
      const fallback = useRef<HTMLButtonElement>(null);
      return (
        <div>
          <button type="button" onClick={() => setOpen(true)}>
            Open
          </button>
          <button type="button" ref={fallback}>
            Fallback target
          </button>
          <Dialog open={open} onOpenChange={setOpen}>
            <DialogContent restoreFocusTo={none} fallbackFocusTo={fallback}>
              <DialogHeader>
                <DialogTitle>Title</DialogTitle>
                <DialogDescription>Description</DialogDescription>
              </DialogHeader>
              <DialogFooter>
                <Button onClick={() => setOpen(false)}>Close</Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </div>
      );
    }
    render(<Explicit />);
    openDialog();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Fallback target" })),
    );
  });

  it("returns focus to the fallback when the opener is gone", async () => {
    render(<Harness removeOpener />);
    openDialog();
    fireEvent.click(screen.getByRole("button", { name: "Undo scenario" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Open" })).not.toBeInTheDocument();
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Fallback target" })),
    );
  });

  it("keeps an error in a persistent alert and does not close by itself", () => {
    const { rerender } = render(<Harness />);
    openDialog();
    // The slot is mounted and empty first, so a later error is announced.
    expect(screen.getByRole("alert")).toBeEmptyDOMElement();

    rerender(<Harness error="The changes moved. Check the list and confirm again." />);
    expect(screen.getByRole("alert")).toHaveTextContent(
      "The changes moved. Check the list and confirm again.",
    );
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    // It stays until the caller clears it.
    rerender(<Harness error="The changes moved. Check the list and confirm again." />);
    expect(screen.getByRole("alert")).toHaveTextContent("The changes moved.");
    rerender(<Harness error={null} />);
    expect(screen.getByRole("alert")).toBeEmptyDOMElement();
  });

  it("bounds the content to the window, scrolls only the body, and keeps title and actions outside it", () => {
    render(<Harness longBody />);
    openDialog();
    const dialog = screen.getByRole("dialog");
    const body = screen.getByTestId("body");
    expect(dialog.className).toContain("max-h-[calc(100dvh-3rem)]");
    expect(body.className).toContain("overflow-y-auto");
    expect(body.className).toContain("[overflow-wrap:anywhere]");
    expect(body).not.toContainElement(screen.getByRole("heading", { name: /Undo/ }));
    expect(body).not.toContainElement(screen.getByRole("button", { name: "Undo scenario" }));
  });

  it("makes the scrolling body keyboard-reachable only while it overflows", () => {
    const scroll = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollHeight");
    const client = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
    Object.defineProperty(HTMLElement.prototype, "scrollHeight", {
      configurable: true,
      get: () => 600,
    });
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get: () => 200,
    });
    try {
      render(<Harness longBody />);
      act(() => {
        openDialog();
      });
      expect(screen.getByTestId("body")).toHaveAttribute("tabindex", "0");
    } finally {
      // The real properties live on Element.prototype; drop the overrides on HTMLElement.prototype.
      if (scroll) Object.defineProperty(HTMLElement.prototype, "scrollHeight", scroll);
      else Reflect.deleteProperty(HTMLElement.prototype, "scrollHeight");
      if (client) Object.defineProperty(HTMLElement.prototype, "clientHeight", client);
      else Reflect.deleteProperty(HTMLElement.prototype, "clientHeight");
    }
  });

  it("leaves a body that fits out of the tab order", () => {
    render(<Harness />);
    openDialog();
    expect(screen.getByTestId("body")).not.toHaveAttribute("tabindex");
  });
});
