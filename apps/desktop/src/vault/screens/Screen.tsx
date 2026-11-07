import { useRef, type ReactNode } from "react";

import { useFocusHeadingOnArrival } from "@/lib/arrivalFocus";
import { SAFE_AREA_PADDING } from "@/lib/safeArea";

/// Centered single-card layout shared by the pre-vault screens.
///
/// On a compact screen (personal-cfo-i93n3.26, the vault access design) it also moves focus to the screen's
/// heading when the screen appears (`useFocusHeadingOnArrival`), and marks its controls for **48-point
/// targets** (`data-comfortable-targets`, see `globals.css`): the design asks for inputs and buttons of at
/// least 48 points here, above the 44-point floor every compact screen has. Neither applies to a desktop
/// window. The vault picker is not a `Screen` and does the same itself.
export function Screen({ children }: { children: ReactNode }) {
  const main = useRef<HTMLElement>(null);
  useFocusHeadingOnArrival(main);
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
