import { describe, expect, it } from "vitest";

import sourceTree from "virtual:source-tree";

/// jsdom has no layout, so these pin the CLASSES that the rendered status-fit audit
/// (`status-fit.html`, evidence in docs/design-system/evidence/personal-cfo-4d8.35) proved keep the
/// status words and their action whole at 368 and 672px. A rendered re-run is what confirms the fit;
/// this stops the geometry from being undone by an unrelated edit in between.
const text = (path: string) => sourceTree.find((f) => f.path === path)?.text ?? "";

describe("status fit geometry (owner condition C3, personal-cfo-4d8.35)", () => {
  const connections = text("settings/ConnectionsCard.tsx");

  it("composes the reviewed lane: identity above one status lane, side by side only when the row is wide", () => {
    // Paper SF-1r2 SF03/SF04 (personal-cfo-4d8.35, Studio F3). The switch is a CONTAINER query on the row
    // itself, so it follows the component's width, not the window's.
    expect(connections).toContain('className="@container rounded-md border p-3"');
    expect(connections).toContain(
      "flex flex-col gap-2 @lg:flex-row @lg:items-start @lg:justify-between @lg:gap-3",
    );
    expect(connections).toContain('data-part="identity" className="min-w-0 @lg:flex-1 @lg:basis-40"');
    expect(connections).toContain(
      'className="flex min-w-0 items-center justify-between gap-3 @lg:justify-end"',
    );
  });

  // The header of one connection: from the row's declaration to its "Refresh automatically" line.
  const row = connections.slice(connections.indexOf("export function ConnectionRow("));
  const header = row.slice(0, row.indexOf("Refresh automatically"));

  it("keeps the badge and the action in one lane that never drops the action below the status", () => {
    // The first version let the lane wrap, which put Refresh now under the badge, left-aligned, at 368px.
    expect(header).not.toContain("flex-wrap");
    expect(header).not.toContain("shrink-0 items-center");
  });

  it("never truncates the connection name: a clipped name loses its meaning", () => {
    expect(header).not.toContain("truncate");
    expect(header).toContain("break-words");
  });

  it("lets the status badge wrap its complete label and keeps Refresh now at its size", () => {
    expect(connections).toContain('"h-auto min-w-0 max-w-full whitespace-normal text-left"');
    expect(connections).toMatch(/className="shrink-0"\s+disabled=\{syncPending\}/);
  });

  it("lets the card's payment-due date and badge wrap", () => {
    const detail = text("accounts/AccountDetailView.tsx");
    expect(detail).toContain("flex flex-wrap items-center gap-x-3 gap-y-1 text-sm font-semibold");
  });
});
