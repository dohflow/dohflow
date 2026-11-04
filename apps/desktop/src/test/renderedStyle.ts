import { compile } from "tailwindcss";

/**
 * Applies the real Tailwind utilities for an element's own classes to the jsdom document, so
 * `getComputedStyle` answers what the browser would render rather than what the DOM contains.
 * jsdom reads no stylesheet on its own, so a class like `empty:hidden` is otherwise invisible to a
 * test. Tailwind 4 emits variants as nested rules (`.x { &:empty { ... } }`); they are flattened
 * here because jsdom's CSS parser does not understand nesting.
 *
 * Returns a function that removes the stylesheet again.
 */
export async function applyUtilityStyles(element: Element): Promise<() => void> {
  const classes = Array.from(element.classList);
  const css = (await compile("@tailwind utilities;")).build(classes);
  const flattened = css.replace(
    /(\.[^\s{]+)\s*\{\s*&(:[a-z-]+)\s*\{([^}]*)\}\s*\}/g,
    "$1$2 {$3}",
  );
  const style = document.createElement("style");
  style.textContent = flattened;
  document.head.appendChild(style);
  return () => style.remove();
}
