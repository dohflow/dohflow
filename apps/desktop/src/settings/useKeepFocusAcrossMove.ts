import { useContext, useEffect, useRef } from "react";

import { SectionFocusContext } from "./sectionFocusContext";

/// Whether an element can take focus right now: it is in the page, not disabled, and not inside
/// hidden or inert content (a collapsed section's body stays mounted but is both).
function canTakeFocus(element: HTMLElement | null | undefined): boolean {
  if (!element || !element.isConnected) return false;
  if ((element as HTMLButtonElement).disabled) return false;
  return element.closest("[hidden],[inert]") === null;
}

/// A button (or select) that moves between the card body and the persistent slot (it is rendered in
/// one place at a time) must not strand focus when it moves. Put `ref` on whichever one is rendered
/// and call `remember()` as the action starts. If the person had focus on the button, focus is
/// handed on once the action has settled (`settled`):
///
/// 1. to the button, wherever it is rendered now, if it can take focus: a disabled button cannot, and
///    neither can one in a collapsed section's hidden body;
/// 2. else to `fallback()`'s element, if that can take focus (the control the person was working in,
///    when the section is open);
/// 3. else, when the section is collapsed, to the section's own heading trigger: the one visible
///    control left, reached without expanding the section.
///
/// Nothing is taken from a control the person has moved to meanwhile. Outside a section only 1 and 2
/// apply.
export function useKeepFocusAcrossMove<T extends HTMLButtonElement | HTMLSelectElement = HTMLButtonElement>(
  options?: { settled?: boolean; fallback?: () => HTMLElement | null | undefined },
) {
  const ref = useRef<T>(null);
  const pending = useRef(false);
  const section = useContext(SectionFocusContext);
  const settled = options?.settled ?? true;
  const fallback = options?.fallback;
  useEffect(() => {
    if (!pending.current) return;
    const button = ref.current;
    const active = document.activeElement;
    if (active && active !== document.body && active !== button) {
      pending.current = false;
      return;
    }
    // Still settling (the write, or the read that follows it): where the button ends up is not
    // known yet, so focus is not handed anywhere.
    if (!settled) return;
    // Focus is still on the button and it can keep it: nothing has moved yet, so there is nothing to
    // hand over. Stay armed until focus is actually lost (an unrelated render must not use it up).
    if (button !== null && active === button && canTakeFocus(button)) return;
    if (button !== null && canTakeFocus(button)) {
      pending.current = false;
      button.focus();
      return;
    }
    const target = fallback?.();
    if (target && canTakeFocus(target)) {
      pending.current = false;
      target.focus();
      return;
    }
    if (section !== undefined && !section.open) {
      const trigger = section.trigger();
      if (trigger && canTakeFocus(trigger)) {
        pending.current = false;
        trigger.focus();
        return;
      }
    }
    // A visible but still-disabled button with nowhere else to go: keep waiting for it.
    if (button !== null && button.isConnected && button.closest("[hidden],[inert]") === null && !fallback) {
      return;
    }
    pending.current = false;
  });
  return {
    ref,
    /// Call as the action starts. When several buttons share one hook (a row each), pass the one
    /// that was pressed: `ref` is on whichever is rendered for the pressed row, which is not yet
    /// known at this moment.
    remember: (pressed?: Element | null) => {
      pending.current = document.activeElement === (pressed ?? ref.current);
    },
  };
}
