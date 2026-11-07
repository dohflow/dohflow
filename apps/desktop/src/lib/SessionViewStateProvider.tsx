import { useState, type ReactNode } from "react";

import {
  SessionViewStateContext,
  createSessionViewStateStore,
} from "./sessionViewState";

/// Provides one session-scoped store to everything under it. It is a Map held in a ref: no module-level or
/// global state and no browser storage of any kind (not localStorage, sessionStorage or IndexedDB), so it
/// lives exactly as long as this provider does. The unlocked shell provides it, and the router swaps the
/// shell out on every lock path and on a vault switch (a switch comes up locked), so the store is destroyed
/// at those moments by construction: nothing here crosses a lock or a vault.
export function SessionViewStateProvider({ children }: { children: ReactNode }) {
  const [store] = useState(createSessionViewStateStore);
  return <SessionViewStateContext.Provider value={store}>{children}</SessionViewStateContext.Provider>;
}

