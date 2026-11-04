import { useEffect, useRef } from "react";

/// A button (or select) that moves between the card body and the persistent slot (it is rendered in one place at
/// a time) must not strand focus when it moves. Put `ref` on whichever one is rendered and call
/// `remember()` as the action starts. If the person had focus on the button, focus is handed to it
/// wherever it is rendered next, as soon as it can take focus: a disabled button cannot, so the
/// hand-over waits while the action is still settling. If focus has moved to another control in the
/// meantime nothing is taken from them. When the action has settled and the button stays disabled
/// (there is nothing left to do), `fallback` says where focus goes instead.
export function useKeepFocusAcrossMove<T extends HTMLButtonElement | HTMLSelectElement = HTMLButtonElement>(
  options?: { settled?: boolean; fallback?: () => void },
) {
  const ref = useRef<T>(null);
  const pending = useRef(false);
  const settled = options?.settled ?? true;
  const fallback = options?.fallback;
  useEffect(() => {
    if (!pending.current) return;
    const button = ref.current;
    const active = document.activeElement;
    if (active && active !== document.body && active !== button) {
      pending.current = false;
    } else if (button && !button.disabled) {
      pending.current = false;
      button.focus();
    } else if (settled && fallback) {
      pending.current = false;
      fallback();
    }
  });
  return {
    ref,
    remember: () => {
      pending.current = document.activeElement === ref.current;
    },
  };
}
