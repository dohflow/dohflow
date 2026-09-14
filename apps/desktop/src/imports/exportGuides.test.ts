// The export-guidance registry (personal-cfo-rfsc / lu4tm): static, complete,
// and honest — every entry has landmark steps, a known format, and no URLs
// that can rot.

import { describe, expect, it } from "vitest";

import { EXPORT_GUIDES, guideFor } from "./exportGuides";

describe("export guidance registry", () => {
  it("has unique ids and a generic fallback last", () => {
    const ids = EXPORT_GUIDES.map((guide) => guide.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(EXPORT_GUIDES[EXPORT_GUIDES.length - 1]?.id).toBe("other");
  });

  it("gives every institution at least three landmark steps and a format", () => {
    for (const guide of EXPORT_GUIDES) {
      expect(guide.steps.length, guide.name).toBeGreaterThanOrEqual(3);
      expect(guide.formats.length, guide.name).toBeGreaterThan(0);
      for (const step of guide.steps) {
        expect(step, guide.name).not.toMatch(/https?:\/\//);
      }
    }
  });

  it("falls back to the generic guide for an unknown institution", () => {
    expect(guideFor("no-such-bank").id).toBe("other");
    expect(guideFor("chase").name).toBe("Chase");
  });
});
