import { useEffect, useRef } from "react";

/// A button that moves between the card body and the persistent slot (it is rendered in one place at
/// a time) must not strand focus when it moves. Put `ref` on whichever one is rendered, call
/// `remember()` as the action starts, and pass whether it is in the slot: if the person had focus on
/// it, focus is handed to the new one after the move.
export function useKeepFocusAcrossMove(inSlot: boolean) {
  const ref = useRef<HTMLButtonElement>(null);
  const pending = useRef(false);
  useEffect(() => {
    if (pending.current) {
      pending.current = false;
      ref.current?.focus();
    }
  }, [inSlot]);
  return {
    ref,
    remember: () => {
      pending.current = document.activeElement === ref.current;
    },
  };
}
