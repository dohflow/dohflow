import { useRef, useState } from "react";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import { Button } from "@/components/ui/button";
import {
  bodyIsStarved,
  COMPACT_BODY_CRAMPED,
  COMPACT_FOOTER_WRAPS,
  COMPACT_SHEET,
  COMPACT_SHEET_CRAMPED,
  CRAMPED_BODY_LINES,
} from "@/lib/sheet";
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
  focusConfirm = false,
}: {
  onConfirm?: () => void;
  error?: string | null;
  longBody?: boolean;
  removeOpener?: boolean;
  restoreFocus?: boolean;
  focusConfirm?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [openerGone, setOpenerGone] = useState(false);
  const confirmRef = useRef<HTMLButtonElement>(null);
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
          initialFocusRef={focusConfirm ? confirmRef : undefined}
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
            <Button variant="ghost" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button
              ref={confirmRef}
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

  it("focuses the chosen element on open, even when it is not the first one", () => {
    // Cancel is the first tabbable element; the option points at the confirm button instead.
    render(<Harness focusConfirm />);
    openDialog();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Undo scenario" }));
  });

  it("without the option, focus starts on the first tabbable element", () => {
    render(<Harness />);
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
  it("is a bottom sheet on a compact screen and the centered card everywhere else (slice B)", () => {
    render(<Harness />);
    openDialog();
    const content = screen.getByRole("dialog");
    // The desktop presentation is untouched ...
    for (const name of ["left-1/2", "top-1/2", "-translate-x-1/2", "-translate-y-1/2", "max-w-md", "rounded-lg"]) {
      expect(content).toHaveClass(name);
    }
    // ... and the sheet is added only behind `compact:` (every sheet class is compact-gated).
    for (const name of COMPACT_SHEET.split(" ")) expect(content).toHaveClass(name);
    expect(content.className.split(" ").filter((n) => /bottom-0|inset-x-0|rounded-t/.test(n)).every((n) => n.startsWith("compact:"))).toBe(true);
    // Still the same dialog: named, modal, and containing the focus trap's content.
    expect(content).toHaveAccessibleName("Undo “Maternity leave”?");
  });


  describe("a sheet that does not fit (largest text size, long name; slice B, Studio's review)", () => {
    // jsdom has no layout: report the heights a phone at the largest text size would give. The header and the
    // footer are tall (a nine-line title, two stacked actions); the sheet's cap is 812.
    function withHeights(heights: { header: number; footer: number; body?: number }) {
      return vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (this: HTMLElement) {
        if (this.dataset.testid === "header") return heights.header;
        if (this.dataset.testid === "footer") return heights.footer;
        if (this.dataset.testid === "body") return heights.body ?? 0;
        return 0;
      });
    }
    function Sheet() {
      return (
        <Dialog open>
          <DialogContent style={{ maxHeight: "812px" }}>
            <DialogHeader data-testid="header">
              <DialogTitle>A very long title</DialogTitle>
            </DialogHeader>
            <DialogBody data-testid="body" style={{ lineHeight: "75px" }}>
              Body text.
            </DialogBody>
            <DialogFooter data-testid="footer">
              <Button>Cancel</Button>
              <Button>Delete scenario</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      );
    }

    it("pins the threshold: fewer than three lines of body left is starved, exactly three is not", () => {
      expect(CRAMPED_BODY_LINES).toBe(3);
      expect(bodyIsStarved({ limit: 812, chrome: 34, others: 727, lineHeight: 17 })).toBe(false); // 51pt left, 17pt lines: exactly 3
      expect(bodyIsStarved({ limit: 812, chrome: 34, others: 728, lineHeight: 17 })).toBe(true);
      expect(bodyIsStarved({ limit: 812, chrome: 34, others: 1000, lineHeight: 75 })).toBe(true); // the measured case: a nine-line title
      expect(bodyIsStarved({ limit: 812, chrome: 34, others: 130, lineHeight: 75 })).toBe(false); // a short confirmation
    });

    it("scrolls as a whole when the title and footer starve the body", () => {
      const spy = withHeights({ header: 1000, footer: 170 });
      try {
        render(<Sheet />);
        const sheet = screen.getByRole("dialog");
        expect(sheet).toHaveAttribute("data-cramped");
        expect(screen.getByTestId("body")).toHaveClass(...COMPACT_BODY_CRAMPED.split(" "));
      } finally {
        spy.mockRestore();
      }
    });

    it("leaves a sheet that fits exactly as it was: no data-cramped, the body keeps its own scrolling", () => {
      // A very tall body must not count against itself: the measure is the OTHER children's heights (04d's survivor).
      const spy = withHeights({ header: 60, footer: 80, body: 5000 });
      try {
        render(<Sheet />);
        expect(screen.getByRole("dialog")).not.toHaveAttribute("data-cramped");
        const body = screen.getByTestId("body");
        for (const name of COMPACT_BODY_CRAMPED.split(" ")) expect(body).not.toHaveClass(name);
        expect(body).toHaveClass("min-h-0", "flex-1", "overflow-y-auto");
      } finally {
        spy.mockRestore();
      }
    });

    it("focuses the safe action without scrolling the sheet, so a cramped sheet opens on its title", () => {
      const focus = vi.spyOn(HTMLElement.prototype, "focus");
      try {
        render(<Harness focusConfirm />);
        openDialog();
        const confirm = screen.getByRole("button", { name: "Undo scenario" });
        const calls = focus.mock.calls.map((args, index) => ({ args, target: focus.mock.contexts[index] }));
        expect(calls).toContainEqual({ args: [{ preventScroll: true }], target: confirm });
        expect(document.activeElement).toBe(confirm);
      } finally {
        focus.mockRestore();
      }
    });

    it("every reflow class is behind `compact:`, and the sheet carries the whole-sheet scroll only on data-cramped", () => {
      render(<Sheet />);
      const sheet = screen.getByRole("dialog");
      for (const name of COMPACT_SHEET_CRAMPED.split(" ")) {
        expect(name).toMatch(/^compact:data-\[cramped\]:/);
        expect(sheet).toHaveClass(name);
      }
      for (const name of [...COMPACT_BODY_CRAMPED.split(" "), ...COMPACT_FOOTER_WRAPS.split(" ")]) {
        expect(name.startsWith("compact:")).toBe(true);
      }
      const footer = screen.getByTestId("footer");
      for (const name of COMPACT_FOOTER_WRAPS.split(" ")) expect(footer).toHaveClass(name);
    });
  });
});
