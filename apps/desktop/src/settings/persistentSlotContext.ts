import { createContext } from "react";

/// The element a Settings section reserves for what must outlive a collapse (personal-cfo-4d8.39.2).
/// `undefined`: no section around (a card shown on its own); `null`: the section has not mounted
/// its slot yet; an element: where a card's persistent notice goes.
export const PersistentSlotContext = createContext<HTMLElement | null | undefined>(undefined);
