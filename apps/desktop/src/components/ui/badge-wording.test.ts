// A semantic Badge's words must carry its status (personal-cfo-4d8.35, the Studio's rule for the
// neutral-text-plus-mark treatment): the tint and the mark are only a redundant cue, so a screen-reader
// user and a color-blind user get the status from the label. A test can't judge wording, but it can
// forbid the three shapes that make color the ONLY carrier, by reading every call site in the source.
import sourceTree from "virtual:source-tree";

const SEMANTIC = ["gain", "warning", "info", "loss"] as const;

type Site = { path: string; line: number; attrs: string; label: string };

/// Every `<Badge ...>label</Badge>` in non-test source (the dev audit page is a fixture, not a call site).
function sites(): Site[] {
  const out: Site[] = [];
  for (const file of sourceTree as { path: string; text: string }[]) {
    if (file.path.includes(".test.") || file.path.startsWith("dev/")) continue;
    for (const m of file.text.matchAll(/<Badge\b([^>]*)>([\s\S]*?)<\/Badge>/g)) {
      out.push({
        path: file.path,
        line: file.text.slice(0, m.index).split("\n").length,
        attrs: m[1] ?? "",
        // The words as written: expressions collapse to `{}`, JSX tags and whitespace are dropped.
        label: (m[2] ?? "")
          .replace(/\{[^{}]*\}/g, "{}")
          .replace(/<[^>]*>/g, " ")
          .replace(/\s+/g, " ")
          .trim(),
      });
    }
  }
  return out;
}

const variantsIn = (attrs: string) => SEMANTIC.filter((v) => new RegExp(`["']${v}["']`).test(attrs));

describe("semantic Badge wording (personal-cfo-4d8.35)", () => {
  const all = sites();
  const semantic = all.filter((s) => variantsIn(s.attrs).length > 0);

  it("reads the real call sites", () => {
    expect(all.length).toBeGreaterThan(10); // guards the guard
    expect(semantic.length).toBeGreaterThan(6);
  });

  it("never picks a semantic variant by a condition on one element (the words could not change with it)", () => {
    const conditional = semantic.filter((s) => /variant=\{/.test(s.attrs));
    expect(
      conditional.map((s) => `${s.path}:${s.line}`),
      "split into one element per variant, each with its own words",
    ).toEqual([]);
  });

  it("never uses the same words under two different semantic variants", () => {
    const byLabel = new Map<string, Set<string>>();
    for (const s of semantic) {
      const variants = byLabel.get(s.label) ?? new Set<string>();
      for (const v of variantsIn(s.attrs)) variants.add(v);
      byLabel.set(s.label, variants);
    }
    const shared = [...byLabel.entries()].filter(([, v]) => v.size > 1).map(([label, v]) => `"${label}" under ${[...v].join(" and ")}`);
    expect(shared).toEqual([]);
  });

  it("gives every semantic Badge literal words, not just a value", () => {
    const wordless = semantic
      .filter((s) => !/[A-Za-z]{3,}/.test(s.label.replace(/\{\}/g, "")))
      .map((s) => `${s.path}:${s.line}`);
    expect(wordless).toEqual([]);
  });

  it("holds for the two sites the Studio's review found resting on color", () => {
    const labels = all.map((s) => s.label);
    expect(labels).toContain("Out of date · refreshed {}");
    expect(labels).toContain("Refreshed {}");
    expect(labels.some((l) => l.startsWith("Due soon · in {}"))).toBe(true);
  });
});
