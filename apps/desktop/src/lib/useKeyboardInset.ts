import { useEffect } from "react";

import { COARSE_POINTER_QUERY, matches } from "@/lib/compact";

const EDITABLE = "input, textarea, select, [contenteditable=''], [contenteditable='true']";

/// Publish the on-screen keyboard's height as `--keyboard-inset` on the root element, and keep the
/// focused field in view while it is up (personal-cfo-i93n3.25).
///
/// The keyboard covers part of the layout viewport without resizing it, so CSS alone cannot clear it.
/// The visual viewport is what shrinks: the inset is how much shorter it is than the layout viewport.
/// Its `offsetTop` is deliberately NOT subtracted: while the keyboard is up the page scrolls by
/// panning the visual viewport inside the layout viewport, so `offsetTop` moves with every scroll and
/// subtracting it made the inset shrink as the person scrolled, leaving the bottom of the page under
/// the keyboard and its form accessory bar (measured on an iPhone simulator). The visual viewport
/// already excludes that accessory bar. While the page is pinch-zoomed the visual viewport is shorter
/// for another reason, so the last value is kept until the zoom is back to 1. The shell pads its
/// bottom edge by the larger of this and the safe area, so a scroll pane ends above the keyboard and a
/// primary action pinned to the bottom stays visible.
///
/// It does nothing without a coarse pointer: on a desktop window no listener is added and the property
/// stays at its CSS default of 0px. Mount it once, near the root.
export function useKeyboardInset(): void {
  useEffect(() => {
    const viewport = window.visualViewport;
    if (!viewport || !matches(COARSE_POINTER_QUERY)) return;
    const root = document.documentElement;
    let raised = false;

    const update = () => {
      if (Math.abs(viewport.scale - 1) > 0.01) return;
      const inset = Math.max(0, Math.round(window.innerHeight - viewport.height));
      root.style.setProperty("--keyboard-inset", `${inset}px`);
      if (inset > 0) {
        raised = true;
        keepFocusedFieldInView();
      } else if (raised) {
        raised = false;
        // The shell never scrolls the document itself, so any document scroll left once the
        // keyboard is gone is the focus scroll the browser made on its own: undo it, or the shell
        // stays shifted over a blank strip.
        if (window.scrollY !== 0 || window.scrollX !== 0) window.scrollTo(0, 0);
      }
    };
    const onFocusIn = () => {
      if (raised) window.requestAnimationFrame(keepFocusedFieldInView);
    };

    viewport.addEventListener("resize", update);
    viewport.addEventListener("scroll", update);
    document.addEventListener("focusin", onFocusIn);
    update();
    return () => {
      viewport.removeEventListener("resize", update);
      viewport.removeEventListener("scroll", update);
      document.removeEventListener("focusin", onFocusIn);
      root.style.removeProperty("--keyboard-inset");
    };
  }, []);
}

function keepFocusedFieldInView() {
  const active = document.activeElement;
  if (active instanceof HTMLElement && active.matches(EDITABLE)) {
    active.scrollIntoView({ block: "nearest", inline: "nearest" });
  }
}
