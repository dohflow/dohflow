import adr from "../../../../docs/adr/0055-applying-a-scenario.md?raw";

import { APPLY_DISPOSITION, replacesOnApply } from "./applyDisposition";

/// The `(token, COLLIDES|ADDS)` rows of the amendment's table in ADR 0055 section 2. That table is
/// what `crates/db-worker/tests/collision_kinds.rs` pins against the kernel's
/// `AssumptionKind::supersedes_on_promotion`, so agreeing with it is agreeing with the kernel.
function adrTable(): Map<string, string> {
  const rows = new Map<string, string>();
  for (const line of adr.split("\n")) {
    const cells = line.split("|").map((cell: string) => cell.trim());
    const kind = cells[1]?.match(/^`([a-z_]+)`$/)?.[1];
    const disposition = cells[2];
    if (kind !== undefined && (disposition === "COLLIDES" || disposition === "ADDS")) {
      rows.set(kind, disposition);
    }
  }
  return rows;
}

describe("the apply disposition list (personal-cfo-5ie.15, ADR 0055 section 2)", () => {
  it("agrees with the ADR table, kind by kind", () => {
    const table = adrTable();
    expect(table.size).toBe(12);
    const ours = new Map(
      Object.entries(APPLY_DISPOSITION).map(([kind, how]) => [
        kind,
        how === "collides" ? "COLLIDES" : "ADDS",
      ]),
    );
    expect([...ours.entries()].sort()).toEqual([...table.entries()].sort());
  });

  it("replaces only for a collision kind, and for a kind it does not know", () => {
    expect(replacesOnApply("bill_date")).toBe(true);
    expect(replacesOnApply("exclusion")).toBe(true);
    expect(replacesOnApply("bill_amount")).toBe(false);
    expect(replacesOnApply("one_time_event")).toBe(false);
    expect(replacesOnApply("some_future_kind")).toBe(true);
  });
});
