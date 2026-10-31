import { describe, expect, it } from "vitest";

import sourceTree from "virtual:source-tree";

/// jsdom has no layout, so these pin the CLASSES that the rendered status-fit audit
/// (`status-fit.html`, evidence in docs/design-system/evidence/personal-cfo-4d8.35) proved keep the
/// status words and their action whole at 368 and 672px. A rendered re-run is what confirms the fit;
/// this stops the geometry from being undone by an unrelated edit in between.
const text = (path: string) => sourceTree.find((f) => f.path === path)?.text ?? "";

describe("status fit geometry (owner condition C3, personal-cfo-4d8.35)", () => {
  const connections = text("settings/ConnectionsCard.tsx");

  it("lets the connection header wrap, with a shrinkable identity and a wrapping lane", () => {
    expect(connections).toContain(
      "flex flex-wrap items-center justify-between gap-x-3 gap-y-2",
    );
    expect(connections).toContain("min-w-0 flex-1 basis-40");
    expect(connections).toContain("flex min-w-0 max-w-full flex-wrap items-center gap-2");
  });

  // The header of one connection: from the row's declaration to its "Refresh automatically" line.
  const row = connections.slice(connections.indexOf("export function ConnectionRow("));
  const header = row.slice(0, row.indexOf("Refresh automatically"));

  it("does not put the status and its action in a lane that cannot shrink or wrap", () => {
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
