import { useEffect, type RefObject } from "react";

import { useCompactLayout } from "@/lib/compact";

/// On a compact screen, move focus to the container's heading when the screen appears, so a screen reader
/// announces where it landed after a change of screen (personal-cfo-i93n3.26, the vault access design). A
/// screen with its own focus rule (an incorrect password, a damaged vault, an opened dialog) sets that after.
/// Arrival only: a later change of `compact` (a rotation) must not steal focus. A desktop window is unchanged.
export function useFocusHeadingOnArrival(container: RefObject<HTMLElement | null>): void {
  const compact = useCompactLayout();
  useEffect(() => {
    if (!compact) return;
    const heading = container.current?.querySelector<HTMLElement>("h1, h2");
    if (heading) {
      heading.tabIndex = -1;
      heading.focus({ preventScroll: true });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
