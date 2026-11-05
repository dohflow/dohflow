import { useContext, type ReactNode } from "react";
import { createPortal } from "react-dom";

import { PersistentSlotContext } from "./persistentSlotContext";

/// An error, its complete remedy and the one action that recovers from it, placed in the section's
/// persistent slot, outside the hidden body (docs/agent/FRONTEND.md section 7.5). The card keeps all
/// of its state and its form where they were, so a draft survives a collapse; only what is shown
/// moves. A card shown outside a section renders this in place, so it works standalone.
export function PersistentNotice({ children }: { children: ReactNode }) {
  const slot = useContext(PersistentSlotContext);
  if (slot === undefined) return <>{children}</>;
  if (slot === null) return null;
  return createPortal(children, slot);
}
