import { foldDebts, type PerDebt } from "./debtBands";

const debt = (label: string, owed: number[]): PerDebt => ({
  label,
  monthly_owed_minor: owed,
});

describe("foldDebts (personal-cfo-hnba, ADR 0054)", () => {
  it("leaves the list alone when it fits the slots", () => {
    const debts = [debt("Visa", [100, 50, 0]), debt("Auto", [500, 400, 200])];
    expect(foldDebts(debts, 4).map((b) => b.label)).toEqual(["Visa", "Auto"]);
  });

  it("folds the tail once the debts outnumber the slots", () => {
    const debts = [
      debt("A", [600, 300]),
      debt("B", [500, 250]),
      debt("C", [400, 200]),
      debt("D", [300, 150]),
      debt("E", [200, 100]),
    ];
    const bands = foldDebts(debts, 4);
    expect(bands).toHaveLength(4);
    expect(bands.map((b) => b.label)).toEqual(["A", "B", "C", "2 smaller debts"]);
  });

  it("keeps the bands summing to the total owed, every month", () => {
    // THE invariant. The fold must never drop the tail: on a debt chart, understating
    // what is owed is the one direction the error must not go. Cycling colours (the old
    // behaviour) kept the arithmetic right but lied visually; a naive `slice(0, 4)` fix
    // would do the reverse. This asserts both stay honest.
    const debts = [
      debt("A", [600, 300, 100]),
      debt("B", [500, 250, 0]),
      debt("C", [400, 200, 50]),
      debt("D", [300, 150, 0]),
      debt("E", [200, 100, 25]),
      debt("F", [100, 50, 0]),
    ];
    const bands = foldDebts(debts, 4);
    for (let month = 0; month < 3; month++) {
      const folded = bands.reduce((sum, b) => sum + (b.owed[month] ?? 0), 0);
      const actual = debts.reduce((sum, d) => sum + (d.monthly_owed_minor[month] ?? 0), 0);
      expect(folded, `month ${month}`).toBe(actual);
    }
  });

  it("keeps the largest debts as their own bands", () => {
    // Deliberately unsorted input — the fold ranks by starting balance, so the two big
    // debts survive individually no matter what order the backend returns them in.
    const debts = [
      debt("small-1", [10, 5]),
      debt("BIG", [900, 400]),
      debt("small-2", [20, 10]),
      debt("BIGGER", [1000, 500]),
      debt("small-3", [30, 15]),
    ];
    const bands = foldDebts(debts, 3);
    expect(bands.map((b) => b.label)).toEqual(["BIGGER", "BIG", "3 smaller debts"]);
    expect(bands[2]!.owed).toEqual([60, 30]);
  });

  it("pads a short series rather than dropping its later months", () => {
    // A debt that cleared early has a shorter array than one still amortising; the folded
    // band has to span the LONGEST series or the stack would end early.
    const bands = foldDebts(
      [debt("A", [100, 90, 80]), debt("B", [50]), debt("C", [40, 30])],
      2,
    );
    expect(bands[1]!.owed).toEqual([90, 30, 0]);
  });
});
