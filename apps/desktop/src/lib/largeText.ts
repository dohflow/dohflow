import { useEffect, useState } from "react";

/// The root font size, in CSS px, from which the largest-text layout is used (personal-cfo-i93n3.25
/// slice C). On iOS the root follows the system text size (slice A): 14 to 23 px are the ordinary sizes
/// and 28 px and up are the accessibility sizes, where three word buttons no longer fit a phone's
/// width. Desktop and any ordinary size stay below it.
export const LARGE_TEXT_ROOT_PX = 28;

function rootFontPx(): number {
  if (typeof document === "undefined") return 16;
  return Number.parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
}

/// Whether the system text size is one of the accessibility sizes. It cannot be a CSS media query:
/// `rem` in a media query is the browser's default, not the root size set here. It is read from the
/// computed root size and re-read when the window resizes, regains focus or becomes visible again (a
/// change in Settings happens while the app is in the background).
export function useLargeText(): boolean {
  const [large, setLarge] = useState(() => rootFontPx() >= LARGE_TEXT_ROOT_PX);
  useEffect(() => {
    const update = () => setLarge(rootFontPx() >= LARGE_TEXT_ROOT_PX);
    update();
    window.addEventListener("resize", update);
    window.addEventListener("focus", update);
    document.addEventListener("visibilitychange", update);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("focus", update);
      document.removeEventListener("visibilitychange", update);
    };
  }, []);
  return large;
}
