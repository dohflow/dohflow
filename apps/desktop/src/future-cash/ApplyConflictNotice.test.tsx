import { render, screen } from "@testing-library/react";

import type { ScenarioConflict } from "./scenarioConflicts";
import { ApplyConflictNotice } from "./ApplyConflictNotice";

const mocks = vi.hoisted(() => ({ conflicts: [] as ScenarioConflict[] | null }));
vi.mock("./useApplyConflicts", () => ({
  useApplyConflicts: () => ({ conflicts: mocks.conflicts }),
}));
vi.mock("@/settings/useBaseCurrency", () => ({
  useBaseCurrency: () => ({ baseCurrency: "USD" }),
}));

const side = (
  eventId: string,
  amountMinor: number | null,
  from: string | null,
  to: string | null,
) => ({ eventId, scenarioId: null, from, to, amountMinor });

const overlap = (
  over: Partial<ScenarioConflict> & Pick<ScenarioConflict, "overlap">,
): ScenarioConflict => ({
  kind: "bill_amount",
  targetEntityId: "rent",
  loser: side("base1", 100_000, null, null),
  winner: side("b1", 120_000, "2027-03-01", "2027-06-30"),
  change: "overlaps",
  ...over,
});

describe("ApplyConflictNotice (personal-cfo-5ie.15)", () => {
  it("states the overlap for an amount change and says the existing amount stays outside it", () => {
    mocks.conflicts = [
      overlap({
        overlap: { from: "2027-03-01", to: "2027-06-30", existingContinuesOutside: true },
      }),
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent(
      "Bill amount: from Mar 1, 2027 to Jun 30, 2027 this uses $1,200.00 instead of $1,000.00; outside those dates your forecast keeps $1,000.00.",
    );
    expect(notice).toHaveTextContent("This overrides an amount you already have, for some dates");
    expect(notice).not.toHaveTextContent(/replace/i);
  });

  it("omits the outside clause when the existing amount does not reach past the overlap", () => {
    mocks.conflicts = [
      overlap({
        overlap: { from: "2027-03-01", to: "2027-06-30", existingContinuesOutside: false },
      }),
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    expect(screen.getByRole("status")).not.toHaveTextContent(/outside those dates your forecast/);
  });

  it("words the open ends", () => {
    mocks.conflicts = [
      overlap({ overlap: { from: "2027-03-01", to: null, existingContinuesOutside: true } }),
      overlap({ overlap: { from: null, to: "2027-06-30", existingContinuesOutside: true } }),
      overlap({ overlap: { from: null, to: null, existingContinuesOutside: false } }),
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent("from Mar 1, 2027 this uses");
    expect(notice).toHaveTextContent("through Jun 30, 2027 this uses");
    expect(notice).toHaveTextContent("for every date this uses");
    expect(notice).toHaveTextContent("This overrides 3 amounts you already have, for some dates");
  });

  it("keeps the replace wording for a date kind", () => {
    mocks.conflicts = [
      {
        kind: "bill_date",
        targetEntityId: "rent",
        loser: side("base1", null, "2027-01-05", null),
        winner: side("b1", null, "2027-01-12", null),
        change: "replaces",
        overlap: null,
      },
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent("This replaces one assumption you already have");
    expect(notice).toHaveTextContent("Applying this replaces it from Jan 12, 2027.");
    expect(notice).toHaveTextContent("Reverting this scenario restores it.");
  });

  it("counts a mix as a change, not a replacement", () => {
    mocks.conflicts = [
      overlap({ overlap: { from: null, to: null, existingContinuesOutside: false } }),
      {
        kind: "exclusion",
        targetEntityId: "rent",
        loser: side("base2", null, null, null),
        winner: side("b2", null, null, null),
        change: "replaces",
        overlap: null,
      },
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    expect(screen.getByRole("status")).toHaveTextContent(
      "This changes 2 assumptions you already have",
    );
  });

  it("renders nothing while comparing, and nothing when there is no conflict", () => {
    mocks.conflicts = null;
    const { container, rerender } = render(<ApplyConflictNotice scenarioId="s" />);
    expect(container).toBeEmptyDOMElement();
    mocks.conflicts = [];
    rerender(<ApplyConflictNotice scenarioId="s" />);
    expect(container).toBeEmptyDOMElement();
  });
});
