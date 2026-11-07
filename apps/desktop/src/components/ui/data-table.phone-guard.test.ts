// A guard (personal-cfo-i93n3.28): no phone table stays horizontal by omission. It reads every source file that renders a
// `<DataTable` and fails when a call site does not opt into the stacked phone layout, or pins raw table cells with
// `leadingRow` without supplying `leadingCards`. A new table is therefore either stacked on a phone or an explicit,
// reviewed exception below.
const sources = import.meta.glob<string>("/src/**/*.tsx", { query: "?raw", import: "default", eager: true });

/// A call site that deliberately stays the horizontal table on a phone, with the reason. Empty today: adding to it needs
/// planning's sign-off, so a table cannot quietly skip the phone layout.
const HORIZONTAL_ON_PHONE: Record<string, string> = {};

const callSites = Object.entries(sources).filter(
  ([path, text]) => !/\.test\.tsx$/.test(path) && !path.endsWith("/components/ui/data-table.tsx") && /<DataTable\b/.test(text),
);

describe("every DataTable call site has a phone layout", () => {
  it("finds the call sites (the guard is not vacuous)", () => {
    expect(callSites.length).toBeGreaterThanOrEqual(6);
  });

  it.each(callSites.map(([path]) => path))("%s opts into the stacked layout", (path) => {
    const text = sources[path]!;
    if (path in HORIZONTAL_ON_PHONE) return;
    const tables = text.match(/<DataTable\b/g)!.length;
    const optedIn = (text.match(/phoneLayout="stacked"/g) ?? []).length;
    expect(optedIn, `${path} renders ${tables} DataTable(s) but ${optedIn} opt into phoneLayout="stacked"`).toBe(tables);
  });

  it.each(callSites.map(([path]) => path))("%s supplies leadingCards wherever it pins leadingRow", (path) => {
    const text = sources[path]!;
    if (path in HORIZONTAL_ON_PHONE) return;
    const pinned = (text.match(/leadingRow=/g) ?? []).length;
    const cards = (text.match(/leadingCards=/g) ?? []).length;
    expect(cards, `${path} pins ${pinned} leadingRow block(s) but supplies ${cards} leadingCards`).toBeGreaterThanOrEqual(pinned);
  });
});
