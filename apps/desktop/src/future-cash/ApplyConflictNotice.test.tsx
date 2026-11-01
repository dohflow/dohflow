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

const REPLACE_ONLY =
  "Undoing restores replaced assumptions unless a newer assumption is kept instead.";
const MIXED =
  "Undoing removes this scenario’s additions; replaced assumptions return unless a newer assumption is kept instead.";
const OVERLAPS_ONLY = "Undoing removes the amount changes this scenario added to your forecast.";
const CONTINUES = "Existing amounts continue outside the overlap dates where they already apply.";
const REMAINING = "The existing amount still applies on its remaining dates.";

const dateConflict = (id: string): ScenarioConflict => ({
  kind: "bill_date",
  targetEntityId: "rent",
  loser: side(`base-${id}`, null, "2027-01-05", null),
  winner: side(id, null, "2027-01-12", null),
  change: "replaces",
  overlap: null,
});

describe("ApplyConflictNotice (personal-cfo-5ie.15)", () => {
  it("partial overlap: names the overlap dates and that the existing amount keeps its remaining dates", () => {
    // Existing Jan to Jun, scenario Apr to Sep: the overlap is Apr to Jun, the existing amount still
    // applies Jan to Mar, and Jul to Sep uses the scenario's amount where nothing existed.
    mocks.conflicts = [
      overlap({
        loser: side("base1", 100_000, "2027-01-01", "2027-06-30"),
        winner: side("b1", 120_000, "2027-04-01", "2027-09-30"),
        overlap: { from: "2027-04-01", to: "2027-06-30", existingContinuesOutside: true },
      }),
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent(
      `Bill amount: from Apr 1, 2027 to Jun 30, 2027, this uses $1,200.00 instead of $1,000.00. ${REMAINING}`,
    );
    expect(notice).toHaveTextContent("This overrides an amount you already have");
    expect(notice).not.toHaveTextContent("for some dates");
    expect(notice).not.toHaveTextContent(/replace/i);
    expect(notice).not.toHaveTextContent("outside those dates");
    expect(notice).toHaveTextContent(`${CONTINUES} ${OVERLAPS_ONLY}`);
  });

  it("no outside: leaves out the remaining-dates sentence and the continuing-amounts footer line", () => {
    mocks.conflicts = [
      overlap({
        overlap: { from: "2027-03-01", to: "2027-06-30", existingContinuesOutside: false },
      }),
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent(
      "Bill amount: from Mar 1, 2027 to Jun 30, 2027, this uses $1,200.00 instead of $1,000.00.",
    );
    expect(notice).not.toHaveTextContent(REMAINING);
    expect(notice).not.toHaveTextContent(CONTINUES);
    expect(notice).toHaveTextContent(OVERLAPS_ONLY);
  });

  it("from only", () => {
    mocks.conflicts = [
      overlap({ overlap: { from: "2027-03-01", to: null, existingContinuesOutside: true } }),
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    expect(screen.getByRole("status")).toHaveTextContent(
      `Bill amount: from Mar 1, 2027, this uses $1,200.00 instead of $1,000.00. ${REMAINING}`,
    );
  });

  it("to only", () => {
    mocks.conflicts = [
      overlap({ overlap: { from: null, to: "2027-06-30", existingContinuesOutside: true } }),
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    expect(screen.getByRole("status")).toHaveTextContent(
      `Bill amount: through Jun 30, 2027, this uses $1,200.00 instead of $1,000.00. ${REMAINING}`,
    );
  });

  it("every date", () => {
    mocks.conflicts = [
      overlap({ overlap: { from: null, to: null, existingContinuesOutside: false } }),
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent(
      "Bill amount: for every date, this uses $1,200.00 instead of $1,000.00.",
    );
    expect(notice).toHaveTextContent("This overrides an amount you already have");
    expect(notice).not.toHaveTextContent("for some dates");
    expect(notice).not.toHaveTextContent(CONTINUES);
  });

  it("several overlaps: counts them and says continuing amounts once", () => {
    mocks.conflicts = [
      overlap({ overlap: { from: "2027-03-01", to: null, existingContinuesOutside: true } }),
      overlap({
        kind: "income_amount",
        overlap: { from: null, to: null, existingContinuesOutside: false },
      }),
      overlap({ overlap: { from: null, to: "2027-06-30", existingContinuesOutside: true } }),
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent("This overrides 3 amounts you already have");
    expect(notice).toHaveTextContent(
      "Income amount: for every date, this uses $1,200.00 instead of $1,000.00.",
    );
    expect(notice.textContent?.split(CONTINUES).length).toBe(2);
  });

  it("a missing amount is never invented", () => {
    mocks.conflicts = [
      overlap({
        loser: side("base1", null, null, null),
        overlap: { from: null, to: "2027-06-30", existingContinuesOutside: false },
      }),
      overlap({
        kind: "income_amount",
        winner: side("b2", null, null, null),
        overlap: { from: null, to: null, existingContinuesOutside: false },
      }),
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent(
      "Bill amount: through Jun 30, 2027, this overrides the amount already in your forecast.",
    );
    expect(notice).toHaveTextContent(
      "Income amount: for every date, this overrides the amount already in your forecast.",
    );
    expect(notice).not.toHaveTextContent("$");
  });

  it("keeps the replace wording for a date kind, with the replacement-only footer", () => {
    mocks.conflicts = [dateConflict("b1")];
    render(<ApplyConflictNotice scenarioId="s" />);
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent("This replaces one assumption you already have");
    expect(notice).toHaveTextContent("Applying this replaces it from Jan 12, 2027.");
    expect(notice).toHaveTextContent(REPLACE_ONLY);
    expect(notice).not.toHaveTextContent(CONTINUES);
  });

  it("counts replacements", () => {
    mocks.conflicts = [dateConflict("b1"), dateConflict("b2")];
    render(<ApplyConflictNotice scenarioId="s" />);
    expect(screen.getByRole("status")).toHaveTextContent(
      "This replaces 2 assumptions you already have",
    );
  });

  it("a mix: a change, with the footer that covers both behaviors", () => {
    mocks.conflicts = [
      overlap({ overlap: { from: "2027-03-01", to: "2027-06-30", existingContinuesOutside: true } }),
      dateConflict("b2"),
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent("This changes 2 assumptions you already have");
    expect(notice).toHaveTextContent(`${CONTINUES} ${MIXED}`);
  });

  it("a mix with no amount continuing outside leaves the continuing line out", () => {
    mocks.conflicts = [
      overlap({ overlap: { from: null, to: null, existingContinuesOutside: false } }),
      dateConflict("b2"),
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent(MIXED);
    expect(notice).not.toHaveTextContent(CONTINUES);
  });

  it("uses Undo, never Revert, in the notice", () => {
    mocks.conflicts = [
      overlap({ overlap: { from: "2027-03-01", to: null, existingContinuesOutside: true } }),
      dateConflict("b2"),
    ];
    render(<ApplyConflictNotice scenarioId="s" />);
    expect(screen.getByRole("status")).not.toHaveTextContent(/revert/i);
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
