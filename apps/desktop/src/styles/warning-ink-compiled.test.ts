/**
 * personal-cfo-4d8.43: the warning-text override as Tailwind actually COMPILES it. The raw-stylesheet
 * checks in theme-contrast.test.ts pin the tokens and the rule's text; this compiles globals.css with the
 * real Tailwind and asserts what the browser will be given: `text-warning` resolves to the ink, and the
 * fill, outline and mark utilities still resolve to the plain warning colour.
 */

import { compile } from "tailwindcss";

import globalsCss from "virtual:globals-css-raw";
import STYLESHEETS from "virtual:tailwind-sheets";

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

/// The value of `property` in the LAST rule for exactly `selector`: equal-specificity rules in one layer
/// resolve by source order, so the last one is the one that applies.
function winning(css: string, selector: string, property: string): string | undefined {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");
  const rules = [...css.matchAll(new RegExp(`(?:^|[\\s{};])${escaped}\\s*\\{([^{}]*)\\}`, "g"))];
  const body = rules.at(-1)?.[1];
  return body === undefined ? undefined : new RegExp(`${property}:\\s*([^;]+);`).exec(body)?.[1]?.trim();
}

describe("warning text as compiled (personal-cfo-4d8.43)", () => {
  it("`text-warning` is the ink, and the fill, outline and tint utilities are still the warning colour", async () => {
    const css = await compiled(["text-warning", "bg-warning", "border-warning", "bg-warning/10"]);
    expect(winning(css, ".text-warning", "color")).toBe("var(--warning-ink)");
    expect(winning(css, ".bg-warning", "background-color")).toBe("var(--warning)");
    expect(winning(css, ".border-warning", "border-color")).toBe("var(--warning)");
    // The 10% tint is built from the plain warning colour, never the ink.
    expect(css).not.toMatch(/bg-warning\\\/10[^}]*warning-ink/);
  });

  it("would fail if the override were dropped: Tailwind's own `text-warning` is the plain colour", async () => {
    const withoutOverride = globalsCss.replace(/@layer utilities\s*\{\s*\.text-warning\s*\{[^}]*\}\s*\}/, "");
    expect(withoutOverride).not.toBe(globalsCss); // guards the guard
    const compiler = await compile(withoutOverride, {
      base: "/",
      loadStylesheet: async (id, base) => {
        const name = id === "tailwindcss" ? "index.css" : (id.split("/").pop() ?? id);
        return { path: `/${name}`, base, content: STYLESHEETS[name] ?? "" };
      },
    });
    expect(winning(compiler.build(["text-warning"]), ".text-warning", "color")).toBe("var(--warning)");
  });
});
