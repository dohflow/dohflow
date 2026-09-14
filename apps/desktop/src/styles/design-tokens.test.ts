// The source tree, served by a Vite virtual module (see vite.config.ts). This package's
// tsconfig excludes node types on purpose, so the walk happens in config — the same way the
// ADR 0054 palette guard reads globals.css. A wrong root still cannot pass silently: the
// "scans a real source tree" assertion below fails loudly on an empty list.
import sourceTree from "virtual:source-tree";

type SourceFile = { path: string; text: string };
const FILES = sourceTree as SourceFile[];

/// Files allowed to contain a literal colour.
///
/// - `globals.css` and `design-tokens.json` are where the tokens are DEFINED.
/// - Tests assert on concrete values on purpose — a palette guard that could not name a
///   hex would not be a guard.
const ALLOWED = (path: string) =>
  path.endsWith("styles/globals.css") ||
  path.endsWith("design-tokens.json") ||
  path.includes(".test.");

/// A literal colour used as DATA rather than as styling.
///
/// The category colour picker is an `<input type="color">`, which cannot take a CSS
/// variable — it needs a literal to seed an unset value. That is a default datum the user
/// immediately overwrites, not a theme decision, so it is named here rather than pretended
/// away. Any OTHER raw hex is a styling decision that belongs in a token.
const DATA_DEFAULT = /value=\{[^}]*"#[0-9a-fA-F]{3,8}"/;

const HEX = /#[0-9a-fA-F]{3,8}\b/g;

describe("design tokens (personal-cfo-4d8.27.4.7, FRONTEND.md §7)", () => {
  const files = FILES;

  it("scans a real source tree", () => {
    // Guards the guard. A path or filter mistake would make every assertion below pass
    // over an empty list — which is exactly how the ADR 0054 palette test shipped green
    // while reading nothing.
    expect(files.length).toBeGreaterThan(50);
    expect(files.some((f) => f.path === "styles/globals.css")).toBe(true);
  });

  it("has no raw hex outside the token definitions", () => {
    const offenders: string[] = [];
    for (const file of files) {
      if (ALLOWED(file.path)) continue;
      for (const line of file.text.split("\n")) {
        if (!HEX.test(line)) {
          HEX.lastIndex = 0;
          continue;
        }
        HEX.lastIndex = 0;
        if (DATA_DEFAULT.test(line)) continue;
        offenders.push(`${file.path}: ${line.trim()}`);
      }
    }
    expect(offenders).toEqual([]);
  });

  it("defines the tokens the app actually references", () => {
    // A `var(--x)` pointing at a token that does not exist renders as nothing, which on a
    // colour is invisible rather than broken — so it has to be checked, not eyeballed.
    const globals =
      files.find((f) => f.path === "styles/globals.css")?.text ?? "";
    const defined = new Set(
      [...globals.matchAll(/^\s*(--[a-z0-9-]+):/gm)].map((m) => m[1]),
    );
    const missing = new Set<string>();
    for (const file of files) {
      if (file.path === "styles/globals.css") continue;
      // This file's own prose names an example var; scanning it would flag the
      // documentation for the rule it documents.
      if (file.path.endsWith("design-tokens.test.ts")) continue;
      for (const [, name] of file.text.matchAll(/var\((--[a-z0-9-]+)\)/g)) {
        if (name === undefined) continue;
        // Tailwind's own theme vars live in its layer, not ours.
        if (name.startsWith("--tw-")) continue;
        // `--color-<key>` is generated at RUNTIME by the chart primitive from each chart's
        // config (components/ui/chart.tsx sets it on the container's style), so it is
        // correctly absent from globals.css. Verified, not assumed.
        if (name.startsWith("--color-")) continue;
        if (!defined.has(name)) missing.add(`${file.path}: ${name}`);
      }
    }
    expect([...missing]).toEqual([]);
  });

  /// Surfaces where money sits in a COLUMN and therefore must align digit-for-digit.
  ///
  /// An explicit list, not "every file that formats money". Most money in the app is prose
  /// — "Your forecast uses $2,500.00 from Rent hike" — where tabular figures are the wrong
  /// choice, so a blanket rule would force noise into sentences to satisfy a lint. The
  /// value here is catching a REMOVAL from a table, which this does.
  it("keeps money columns on tabular figures", () => {
    const COLUMNAR = [
      "components/ui/ranked-bars.tsx",
      "transactions/TransactionsView.tsx",
      "future-cash/ProjectedActivityTable.tsx",
      "accounts/AccountsView.tsx",
      "accounts/DebtPayoffCompare.tsx",
    ];
    const missing = COLUMNAR.filter(
      (rel) => !(files.find((f) => f.path === rel)?.text ?? "").includes("tabular-nums"),
    );
    expect(missing).toEqual([]);
  });

  /// A reference REGION must not wear a categorical series colour
  /// (personal-cfo-o7bi, ADR 0054).
  ///
  /// The comfort band used to be shaded `--chart-2` — the slot Reserve plots in — so
  /// turning Reserve on drew its line in the reference region's own hue. Asserted on the
  /// SOURCE rather than the DOM because Recharts renders no geometry under jsdom, so a
  /// rendered-markup check here would pass without looking at anything.
  it("keeps the comfort band off the categorical slots", () => {
    const chart = files.find(
      (f) => f.path === "future-cash/MultiSeriesChart.tsx",
    );
    expect(chart, "the chart file is in the scan").toBeDefined();

    // The comfort-band block: from its marker comment to the series that follow it.
    const text = chart?.text ?? "";
    const start = text.indexOf("Comfort band (915.1)");
    expect(start).toBeGreaterThan(-1);
    const end = text.indexOf("<Legend", start);
    const block = text.slice(start, end > start ? end : undefined);

    // The block names --chart-2 once, in the comment explaining why it no longer uses it.
    const uses = [...block.matchAll(/(fill|stroke)="var\((--chart-[0-9]+)\)"/g)];
    expect(uses.map((m) => m[2])).toEqual([]);
  });

  /// The projected half is one continuous stroke (personal-cfo-7c7a).
  ///
  /// Dashes are the chart vocabulary for MISSING data, so on a projection they claim the
  /// median is imprecise — but the median is exactly computed, and the uncertainty is
  /// already drawn as the band. Asserted on source for the same reason as the comfort-band
  /// check: Recharts renders no geometry under jsdom.
  it("does not dash the projected series", () => {
    const chart = files.find(
      (f) => f.path === "future-cash/MultiSeriesChart.tsx",
    );
    const text = chart?.text ?? "";
    expect(text).toContain("const forwardDash = undefined;");
  });
});
