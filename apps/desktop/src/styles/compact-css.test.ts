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
import { COMPACT_DRAWER_CLEARS_CHROME, COMPACT_DRAWER_PADDED_6, COMPACT_SHEET } from "@/lib/sheet";

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


/// The text of every `@media <COMPACT_QUERY> { … }` block, and the CSS with them removed.
function splitCompactBlocks(css: string): { blocks: string[]; rest: string } {
  const opener = `@media ${COMPACT_QUERY} {`;
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
  describe("touch targets (slice B)", () => {
    it("the 44px floor is declared once, inside the compact query, in px", async () => {
      const css = await compiled(["p-4"]);
      const { blocks, rest } = splitCompactBlocks(css);
      // The `compact:` variant also compiles to this query, but only for candidates; here none use it.
      const floor = blocks.filter((block) => block.includes("min-height: 44px"));
      expect(floor).toHaveLength(1);
      expect(floor[0]).toMatch(/min-width:\s*44px/);
      // A fixed floor, never spacing utilities: --spacing is below 4px at the smaller system text sizes.
      expect(floor[0]).not.toContain("--spacing");
      expect(floor[0]).not.toMatch(/min-(height|width):\s*(calc|var)\(/);
      // Nothing outside the compact query raises a control to 44px, so a desktop window is unchanged.
      expect(rest).not.toMatch(/min-(height|width):\s*44px/);
      expect(rest).not.toMatch(/input\[type="checkbox"\]/);
    });

    it("the floor covers every kind of control a finger can press", async () => {
      const css = await compiled(["p-4"]);
      const block = splitCompactBlocks(css).blocks.find((b) => b.includes("min-height: 44px")) ?? "";
      for (const selector of ["button", '[role="button"]', '[role="menuitem"]', "summary", "select", "textarea"]) {
        expect(block, selector).toContain(selector);
      }
      // The width floor covers the same set, text-like inputs included: measured on a simulator, amount
      // fields laid out 33pt wide and a color input 40pt.
      const widthAt = block.indexOf("min-width: 44px");
      const widthSelectors = block.slice(block.lastIndexOf(":where(", widthAt), widthAt);
      for (const selector of ["button", "select", "textarea", "input:not("]) {
        expect(widthSelectors, selector).toContain(selector);
      }
      // Text-like inputs get the height; checkboxes, radios, hidden and range inputs are excluded from
      // THAT rule (the first two have their own below, the others are not pressed).
      expect(block).toMatch(/input:not\(\[type="hidden"\],\s*\[type="checkbox"\],\s*\[type="radio"\],\s*\[type="range"\]\)/);
    });

    it("the vault access screens raise inputs and buttons from 44px to 48px, and checkboxes stay at 44px", async () => {
      const css = await compiled(["p-4"]);
      const block = splitCompactBlocks(css).blocks.find((b) => b.includes("min-height: 44px")) ?? "";
      const at = block.indexOf("[data-comfortable-targets]");
      expect(at).toBeGreaterThan(-1);
      const rule = block.slice(at, block.indexOf("}", at) + 1);
      expect(rule).toMatch(/min-height:\s*48px/);
      for (const selector of ["button", "select", "textarea", "input:not("]) {
        expect(rule, selector).toContain(selector);
      }
      // The checkbox and radio are excluded: the design asks 44 for those.
      expect(rule).toMatch(/\[type="checkbox"\],\s*\[type="radio"\]/);
      // It comes after the 44px floor and is more specific, so it wins (both are unlayered).
      expect(at).toBeGreaterThan(block.indexOf("min-height: 44px"));
      // A native select ignores min-height on iOS WebKit: it takes an explicit 48px height as well, and
      // that rule also follows the 44px select rule so that it wins.
      const selectAt = block.indexOf("[data-comfortable-targets] select");
      expect(selectAt).toBeGreaterThan(-1);
      expect(block.slice(selectAt, block.indexOf("}", selectAt))).toMatch(/height:\s*max\(48px,\s*calc\(1lh \+ 20px\)\)/);
      expect(selectAt).toBeGreaterThan(block.indexOf("select {"));
      // Never outside the compact query.
      expect(splitCompactBlocks(css).rest).not.toContain("min-height: 48px");
      expect(splitCompactBlocks(css).rest).not.toContain("max(48px");
    });

    it("a native select takes an explicit height, because iOS WebKit ignores its min-height", async () => {
      const css = await compiled(["p-4"]);
      const block = splitCompactBlocks(css).blocks.find((b) => b.includes("min-height: 44px")) ?? "";
      // WebKit's menulist keeps its own `min-height: 30px` (a `min-h-10` select laid out at 41pt on a
      // simulator), but honors `height`. The height is the larger of the floor and one line of text
      // plus padding, so a large system text size grows the control instead of clipping it.
      expect(block).toMatch(/select\s*\{[^}]*height:\s*max\(44px,\s*calc\(1lh \+ 20px\)\)/);
      expect(block).not.toMatch(/select\s*\{[^}]*height:\s*44px/);
    });

    it("a checkbox and a radio are drawn as a 44px hit region holding a smaller box", async () => {
      const css = await compiled(["p-4"]);
      const block = splitCompactBlocks(css).blocks.find((b) => b.includes("min-height: 44px")) ?? "";
      const checkbox = /input\[type="checkbox"\],\s*input\[type="radio"\]\s*\{([^}]*)\}/.exec(block)?.[1] ?? "";
      expect(checkbox).toMatch(/width:\s*44px/);
      expect(checkbox).toMatch(/height:\s*44px/);
      expect(checkbox).toMatch(/appearance:\s*none/);
      // The visible box is the padding box inside a transparent border: 44 - 2 * 12 = 20px.
      expect(checkbox).toMatch(/border:\s*12px solid transparent/);
      expect(checkbox).toMatch(/background:\s*var\(--card\)\s+padding-box/);
      // The mark is a pseudo-element, drawn in the theme's own tokens (no image, no raw colour).
      expect(block).toMatch(/input\[type="checkbox"\]:checked::before/);
      expect(block).toMatch(/input\[type="radio"\]:checked::before/);
      expect(block).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    });

    it("the floor is outside every cascade layer, so a call site's h-8, min-h-0 or min-h-10 cannot lower it", async () => {
      // Tailwind utilities live in `@layer utilities`; unlayered CSS beats any layer whatever the
      // specificity. So the media block that holds the floor must sit at brace depth 0.
      const css = await compiled(["h-8", "min-h-0", "min-h-10"]);
      const opener = `@media ${COMPACT_QUERY} {`;
      let depthAtFloor: number | null = null;
      for (let at = css.indexOf(opener); at !== -1; at = css.indexOf(opener, at + 1)) {
        let depth = 0;
        let end = at + opener.length - 1;
        for (; end < css.length; end += 1) {
          if (css[end] === "{") depth += 1;
          if (css[end] === "}" && --depth === 0) break;
        }
        if (css.slice(at, end).includes("min-height: 44px")) {
          let outer = 0;
          for (const ch of css.slice(0, at)) {
            if (ch === "{") outer += 1;
            if (ch === "}") outer -= 1;
          }
          depthAtFloor = outer;
        }
      }
      expect(depthAtFloor).toBe(0);
      expect(css).toMatch(/@layer utilities\s*\{/);
    });
  });

  describe("sheets (slice B)", () => {
    const sheetClasses = [
      ...COMPACT_SHEET.split(" "),
      ...COMPACT_DRAWER_PADDED_6.split(" "),
      ...COMPACT_DRAWER_CLEARS_CHROME.split(" "),
    ];

    it("every sheet and drawer class is behind `compact:`, so a desktop window renders as before", async () => {
      expect(sheetClasses.every((name) => name.startsWith("compact:"))).toBe(true);
      const css = await compiled(sheetClasses);
      const { blocks, rest } = splitCompactBlocks(css);
      expect(blocks.length).toBeGreaterThan(0);
      // Tailwind nests the query inside each class (`.compact\\:x { @media (...) { ... } }`), so once the
      // compact blocks are removed only EMPTY shells may be left: no declaration applies outside it.
      const stray = rest.replace(/\.compact\\:[^{]+\{\s*\}/g, "");
      expect(stray).not.toContain("compact\\:");
      // They do compile (a typo would silently compile to nothing).
      const inCompact = blocks.join("\n");
      for (const declaration of [
        /bottom:\s*calc\(var\(--spacing\) \* 0\)|bottom:\s*0/,
        /max-height:\s*calc\(100dvh - var\(--safe-area-top\)\)/,
        /padding-bottom:\s*max\(var\(--safe-area-bottom\),\s*var\(--keyboard-inset\)\)/,
        /padding-top:\s*calc\(1\.5rem \+ var\(--safe-area-top\)\)/,
        /border-top-left-radius/,
      ]) {
        expect(inCompact).toMatch(declaration);
      }
    });

    it("the bottom edge takes the larger of the home indicator and the keyboard, never their sum", async () => {
      for (const name of sheetClasses.filter((n) => n.includes("pb-"))) {
        expect(name).toContain("max(var(--safe-area-bottom),var(--keyboard-inset))");
        expect(name).not.toMatch(/safe-area-bottom\)\s*\+\s*var\(--keyboard/);
      }
    });
  });

});
