import { createContext } from "react";

/// What a card needs from its Settings section to hand focus on safely (personal-cfo-4d8.39.2):
/// whether the section is open, and its own heading trigger, which is the one control that is
/// always visible. `undefined`: no section around (a card shown on its own).
export type SectionFocus = {
  open: boolean;
  trigger: () => HTMLElement | null;
};

export const SectionFocusContext = createContext<SectionFocus | undefined>(undefined);
