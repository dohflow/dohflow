const FOCUSABLE =
  "button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex='-1'])";

/// Where focus goes when a removed row disappears (docs/agent/FRONTEND.md section 6.2): the nearest
/// surviving row, or the section heading when there is none, never the page body. Call it BEFORE
/// the removal with the row's element, and call the function it returns once the removal has
/// succeeded. The heading is the element marked `data-removal-heading` (with `tabIndex={-1}`) inside
/// the nearest ancestor marked `data-removal-scope` (also `tabIndex={-1}`, the fallback itself).
export function rememberFocusTarget(row: HTMLElement | null): () => void {
  if (!row) return () => {};
  const neighbor = (row.nextElementSibling ?? row.previousElementSibling) as HTMLElement | null;
  // The section heading, or the section itself when it has none (an embedded list).
  const scope = row.closest<HTMLElement>("[data-removal-scope]");
  const heading = scope?.querySelector<HTMLElement>("[data-removal-heading]") ?? scope;
  return () => {
    const target = neighbor?.isConnected
      ? (neighbor.querySelector<HTMLElement>(FOCUSABLE) ?? neighbor)
      : heading;
    target?.focus();
  };
}
