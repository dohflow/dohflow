import { describe, expect, it } from "vitest";

import sourceTree from "virtual:source-tree";

import { BADGE_CASES, badgeVerdict, type BadgeMeasure } from "./badge-verdict";

const GOOD: BadgeMeasure = { label: 12, markOnFill: 5, overflows: false };
/// The measurement the Studio injected to prove the old classification: unreadable label (1:1),
/// invisible mark (1:1), and a label that overflows its box.
const BROKEN: BadgeMeasure = { label: 1, markOnFill: 1, overflows: true };

describe("badge verdicts (personal-cfo-4d8.35, Studio F1)", () => {
  it("declares every badge, long ones included, as required", () => {
    const ids = BADGE_CASES.map((c) => c.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toEqual(expect.arrayContaining(["badge-long-narrow", "badge-long-narrow-gain", "badge-autopay", "badge-resurfaced"]));
    for (const c of BADGE_CASES) expect(c.required, c.id).toBe(true);
  });

  it("passes a good measurement for every declared badge", () => {
    for (const c of BADGE_CASES) expect(badgeVerdict(c, GOOD), c.id).toBe("pass");
  });

  // The failing mutation: with the old id-parsing the two long rows (both themes, both surfaces:
  // eight rows) read `info` here and the audit reported them as certified.
  it("FAILS every declared badge, long ones included, on the measurement that must fail", () => {
    for (const c of BADGE_CASES) expect(badgeVerdict(c, BROKEN), c.id).toBe("FAIL");
    for (const id of ["badge-long-narrow", "badge-long-narrow-gain"]) {
      const spec = BADGE_CASES.find((c) => c.id === id);
      expect(badgeVerdict(spec, BROKEN), id).toBe("FAIL");
    }
  });

  it("fails each floor on its own", () => {
    const warning = BADGE_CASES.find((c) => c.id === "badge-long-narrow")!;
    expect(badgeVerdict(warning, { ...GOOD, label: 4.5 })).toBe("FAIL"); // the floor plus headroom is 4.6
    expect(badgeVerdict(warning, { ...GOOD, label: 4.6 })).toBe("pass");
    expect(badgeVerdict(warning, { ...GOOD, markOnFill: 3.0 })).toBe("FAIL");
    expect(badgeVerdict(warning, { ...GOOD, markOnFill: 3.1 })).toBe("pass");
    expect(badgeVerdict(warning, { ...GOOD, overflows: true })).toBe("FAIL");
    // A semantic badge with no mark at all has lost its status cue.
    expect(badgeVerdict(warning, { ...GOOD, markOnFill: null })).toBe("FAIL");
    // A neutral badge needs none.
    const plain = BADGE_CASES.find((c) => c.id === "badge-default")!;
    expect(badgeVerdict(plain, { ...GOOD, markOnFill: null })).toBe("pass");
  });

  it("fails a badge that was never declared instead of letting it read as info", () => {
    expect(badgeVerdict(undefined, GOOD)).toBe("FAIL");
  });

  it("reports a badge that is declared not required as info, whatever it measures", () => {
    const exempt = { ...BADGE_CASES[0]!, required: false, exemption: "test" };
    expect(badgeVerdict(exempt, BROKEN)).toBe("info");
  });

  it("is what the audit page uses: it renders from the declarations and never derives a variant from an id", () => {
    const page = sourceTree.find((f) => f.path === "dev/contrast-audit.tsx")?.text ?? "";
    expect(page).toContain("BADGE_CASES");
    expect(page).toContain("badgeVerdict(");
    expect(page).not.toContain('.split("-")[0]');
    expect(page).not.toContain("SEMANTIC.has(");
  });
});
