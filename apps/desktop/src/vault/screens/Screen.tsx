import { useEffect, useRef, type ReactNode } from "react";

import { useCompactLayout } from "@/lib/compact";
import { SAFE_AREA_PADDING } from "@/lib/safeArea";

/// Centered single-card layout shared by the pre-vault screens.
///
/// On a compact screen (personal-cfo-i93n3.26, the vault access design) it also:
///
/// - **moves focus to the screen's heading when the screen appears**, so a screen reader announces where
///   it landed after a change of screen (a screen with its own focus rule, such as an incorrect password
///   or a damaged vault, sets that focus itself afterwards);
/// - marks its controls for **48-point targets** (`data-comfortable-targets`, see `globals.css`): the
///   vault access design asks for inputs and buttons of at least 48 points here, above the 44-point
///   floor every compact screen has.
///
/// Neither applies to a desktop window.
export function Screen({ children }: { children: ReactNode }) {
  const compact = useCompactLayout();
  const main = useRef<HTMLElement>(null);
  useEffect(() => {
    if (!compact) return;
    const heading = main.current?.querySelector<HTMLElement>("h1, h2");
    if (heading) {
      heading.tabIndex = -1;
      heading.focus({ preventScroll: true });
    }
    // On arrival only: a later change of `compact` (a rotation) must not steal focus.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div
      data-comfortable-targets=""
      className={`flex min-h-dvh flex-col bg-background ${SAFE_AREA_PADDING}`}
    >
      <main ref={main} className="flex flex-1 items-center justify-center p-6">
        <div className="w-full max-w-md">{children}</div>
      </main>
    </div>
  );
}
