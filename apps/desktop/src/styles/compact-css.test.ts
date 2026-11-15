/**
 * personal-cfo-i93n3.25: what the compact-layout seams in globals.css compile to, and that none of
 * them can change a desktop window. Compiles the stylesheet with the real Tailwind compiler (as
 * warning-ink-compiled.test.ts does) and reads what the browser is given.
 */

import { compile } from "tailwindcss";

import indexHtml from "../../index.html?raw";
import globalsCss from "virtual:globals-css-raw";
import sourceTree from "virtual:source-tree";
import STYLESHEETS from "virtual:tailwind-sheets";

import { COMPACT_QUERY } from "@/lib/compact";
import { SAFE_AREA_PADDING } from "@/lib/safeArea";

async function compiled(candidates: string[]): Promise<string> {
  const compiler = await compile(globalsCss, {
    base: "/",
    loadStylesheet: async (id, base) => {
      const name = id === "tailwindcss" ? "index.css" : (id.split("/").pop() ?? id);
      const content = STYLESHEETS[name];
      if (content === undefined) throw new Error(`no stylesheet for ${id}`);
      return { path: `/${name}`, base, content };
    },
  });
  return compiler.build(candidates);
}

/// The text of every `@supports (-webkit-touch-callout: none) { … }` block, and the CSS with them removed.
function splitIosBlocks(css: string): { blocks: string[]; rest: string } {
  const opener = "@supports (-webkit-touch-callout: none) {";
  const blocks: string[] = [];
  let rest = css;
  for (let start = rest.indexOf(opener); start !== -1; start = rest.indexOf(opener)) {
    let depth = 0;
    let end = start + opener.length - 1;
    for (; end < rest.length; end += 1) {
      if (rest[end] === "{") depth += 1;
      if (rest[end] === "}" && --depth === 0) break;
    }
    blocks.push(rest.slice(start, end + 1));
    rest = rest.slice(0, start) + rest.slice(end + 1);
  }
  return { blocks, rest };
}

describe("compact layout seams as compiled (personal-cfo-i93n3.25)", () => {
  it("`compact:` compiles to the one compact query", async () => {
    const css = await compiled(["compact:hidden"]);
    expect(css).toContain(`@media ${COMPACT_QUERY}`);
  });

  it("the system root font is set only inside the iOS WebKit block, so a desktop root stays 16px", async () => {
    const css = await compiled(["p-4", "compact:hidden", "text-sm"]);
    const { blocks, rest } = splitIosBlocks(css);

    expect(blocks).toHaveLength(1);
    expect(blocks[0]).toMatch(/html\s*\{[^}]*font:\s*-apple-system-body/);
    expect(rest).not.toContain("-apple-system-body");
    // No other rule sets the root's size: only the block above and Tailwind's own text-size-adjust.
    expect(rest).not.toMatch(/(?:^|[\s,}])html\s*\{[^}]*font-size/);
    // The block restates what `font:` resets, so type and spacing are unchanged on iOS apart from size.
    expect(blocks[0]).toContain("font-family: var(--font-ui)");
    expect(blocks[0]).toContain("line-height: 1.5");
  });

  it("spacing grows slowly with the system text size, inside the iOS block only", async () => {
    const css = await compiled(["p-4"]);
    const { blocks, rest } = splitIosBlocks(css);

    expect(blocks[0]).toContain("--spacing: calc(4px + (1rem - 17px) / 8)");
    // Desktop keeps Tailwind's own rem spacing.
    expect(rest).toMatch(/--spacing:\s*0\.25rem/);
    expect(rest).not.toContain("--spacing: calc(");
  });

  it("the field text floor that stops iOS zooming into a focused field is inside the iOS block only", async () => {
    const css = await compiled(["p-4"]);
    const { blocks, rest } = splitIosBlocks(css);

    expect(blocks[0]).toMatch(/input,\s*select,\s*textarea\s*\{[^}]*font-size:\s*max\(1rem,\s*16px\)\s*!important/);
    expect(rest).not.toContain("max(1rem, 16px)");
  });

  it("the safe-area insets fall back to 0px and the keyboard inset defaults to 0px", async () => {
    const css = await compiled(["pt-(--safe-area-top)"]);
    const envs = [...css.matchAll(/env\(([^)]*)\)/g)].map((m) => m[1]);
    expect(envs.sort()).toEqual([
      "safe-area-inset-bottom, 0px",
      "safe-area-inset-left, 0px",
      "safe-area-inset-right, 0px",
      "safe-area-inset-top, 0px",
    ]);
    expect(css).toMatch(/--keyboard-inset:\s*0px/);
    // Nothing but the four variables reads the environment.
    for (const name of ["top", "right", "bottom", "left"]) {
      expect(css).toContain(`--safe-area-${name}: env(safe-area-inset-${name}, 0px)`);
    }
  });

  it("the shell's safe-area padding is plain padding that resolves to 0 without insets", async () => {
    const css = await compiled([
      "pt-(--safe-area-top)",
      "pb-[max(var(--safe-area-bottom),var(--keyboard-inset))]",
    ]);
    expect(css).toMatch(/padding-top:\s*var\(--safe-area-top\)/);
    expect(css).toMatch(/padding-bottom:\s*max\(var\(--safe-area-bottom\),\s*var\(--keyboard-inset\)\)/);
  });

  it("the exported safe-area padding is the four insets, with the larger of the home indicator and the keyboard at the bottom", async () => {
    expect(SAFE_AREA_PADDING.split(" ").sort()).toEqual([
      "pb-[max(var(--safe-area-bottom),var(--keyboard-inset))]",
      "pl-(--safe-area-left)",
      "pr-(--safe-area-right)",
      "pt-(--safe-area-top)",
    ]);
    const css = await compiled(SAFE_AREA_PADDING.split(" "));
    expect(css).toMatch(/padding-bottom:\s*max\(var\(--safe-area-bottom\),\s*var\(--keyboard-inset\)\)/);
    expect(css).not.toMatch(/padding-bottom:[^;]*\+/);
  });

  it("index.html asks for the full screen, so the insets are reported, and still allows zoom", () => {
    const viewport = /<meta name="viewport" content="([^"]*)"/.exec(indexHtml)?.[1] ?? "";
    expect(viewport).toContain("viewport-fit=cover");
    expect(viewport).not.toMatch(/maximum-scale|user-scalable/);
  });

  it("no component sets a fixed-pixel font size, so Dynamic Type reaches every line", () => {
    const offenders = sourceTree
      .filter((file) => /\.tsx$/.test(file.path) && !/\.test\.tsx$/.test(file.path))
      .filter((file) => /text-\[[0-9.]+px\]/.test(file.text))
      .map((file) => file.path);
    expect(offenders).toEqual([]);
  });
});
