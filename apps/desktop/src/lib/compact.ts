import { useSyncExternalStore } from "react";

/// The compact layout signal (personal-cfo-i93n3.25): a narrow screen driven by a touch pointer.
///
/// Width alone is not enough. The desktop window has no minimum width, so a narrowed desktop window is
/// narrow without being a phone; the pointer decides. This is the one definition on the TypeScript side:
/// `globals.css` declares the Tailwind `compact:` variant with the same query, and a test keeps the two
/// equal. 639px is the width just under Tailwind's `sm` breakpoint (640px).
export const COMPACT_QUERY = "(max-width: 639px) and (pointer: coarse)";

/// A touch pointer is the primary pointer. The keyboard handling reads this, not [`COMPACT_QUERY`]: a
/// phone turned to landscape is wider than the breakpoint and still has an on-screen keyboard.
export const COARSE_POINTER_QUERY = "(pointer: coarse)";

function mediaQuery(query: string): MediaQueryList | null {
  return typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia(query)
    : null;
}

/// Whether `query` matches now. False where `matchMedia` is unavailable.
export function matches(query: string): boolean {
  return mediaQuery(query)?.matches ?? false;
}

function subscribe(query: string, onChange: () => void): () => void {
  const list = mediaQuery(query);
  list?.addEventListener("change", onChange);
  return () => list?.removeEventListener("change", onChange);
}

/// Whether the layout is compact right now, and re-rendered when it changes (a rotation, a window
/// resize on a touch device). Layout that can be expressed in CSS should use the `compact:` variant
/// instead; this hook is for the structure that differs (which component renders), not the styling.
export function useCompactLayout(): boolean {
  return useSyncExternalStore(
    (onChange) => subscribe(COMPACT_QUERY, onChange),
    () => matches(COMPACT_QUERY),
    () => false,
  );
}

/// Whether a touch pointer is the primary pointer right now, and re-rendered when it changes. Unlike
/// [`useCompactLayout`] it does not depend on the width, so a phone turned to landscape still counts.
export function useCoarsePointer(): boolean {
  return useSyncExternalStore(
    (onChange) => subscribe(COARSE_POINTER_QUERY, onChange),
    () => matches(COARSE_POINTER_QUERY),
    () => false,
  );
}
