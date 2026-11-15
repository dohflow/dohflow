import { screen } from "@testing-library/react";
import { compile } from "tailwindcss";

import { COMPACT_QUERY } from "@/lib/compact";
import { renderWithClient } from "@/test/renderWithClient";

import { CreateVaultScreen } from "./CreateVaultScreen";
import { Screen } from "./Screen";

// Studio F3 (personal-cfo-i93n3.25): at the largest system text size the screen, card and callout
// padding nest, and a narrow phone is left a sliver of a column, so critical words broke mid-word
// ("pass-/word", "un-/der-/stand"). On a narrow card each layer trims its horizontal gutter; type,
// copy and the checkbox's label are untouched, and a normal-size card and the desktop keep theirs.

const mocks = vi.hoisted(() => ({
  noResetWarning: vi.fn(),
  buildInfo: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: { noResetWarning: mocks.noResetWarning, buildInfo: mocks.buildInfo },
}));
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({ createVault: vi.fn() }),
  describeIpcError: (error: unknown) => String(error),
}));
vi.mock("@/backup/RestoreFromBackup", () => ({ RestoreFromBackup: () => null }));

beforeEach(() => {
  mocks.noResetWarning.mockResolvedValue("DohFlow has no cloud password reset.");
  mocks.buildInfo.mockResolvedValue(null);
});

const NARROW = "compact:@max-[15rem]";

describe("narrow-card gutters", () => {
  it("the screen's padding sits inside a size container, so a query can measure it", () => {
    renderWithClient(<Screen>content</Screen>);

    const main = screen.getByRole("main");
    // The container is `main`; the padding moved to the element inside it (a container's own padding
    // is outside what its query measures).
    expect(main).toHaveClass("@container");
    expect(main.className).not.toMatch(/(^|\s)p-\d/);
    const gutter = main.firstElementChild as HTMLElement;
    expect(gutter).toHaveClass("w-full", "p-6", `${NARROW}:p-1`);
    // The width cap is on its own child with no padding: Tailwind's preflight makes every box
    // border-box, so padding on the `max-w-md` element would shrink the 28rem column on a desktop
    // window (04b's finding on 4ff1e593).
    expect(gutter).not.toHaveClass("max-w-md");
    const column = gutter.firstElementChild as HTMLElement;
    expect(column).toHaveClass("mx-auto", "w-full", "max-w-md");
    expect(column.className).not.toMatch(/(^|\s)(compact:\S+:)?p[xytblr]?-\d/);
    expect(column.textContent).toBe("content");
  });

  it("the card header, the card content and the warning callout each trim their gutter", async () => {
    renderWithClient(<CreateVaultScreen />);

    const callout = (await screen.findByTestId("no-reset-warning")).closest(
      ".\\@container",
    ) as HTMLElement;
    const heading = screen.getByRole("heading", { name: "Create your vault" });
    const header = heading.closest("div.items-center") as HTMLElement;
    const content = screen.getByLabelText("Master password").closest("form")!
      .parentElement as HTMLElement;
    for (const layer of [header, content, callout]) {
      expect(layer).toHaveClass(`${NARROW}:px-2`);
    }
    // A normal-size card keeps the gutters it had.
    expect(header).toHaveClass("p-6");
    expect(content).toHaveClass("p-6", "pt-0");
    expect(callout).toHaveClass("p-3");
    // The type is not reduced to make room, and the checkbox stays inside its label.
    for (const layer of [header, content, callout]) {
      expect(layer.className).not.toMatch(/text-\[|text-(xs|sm)\b.*compact/);
    }
    const checkbox = screen.getByRole("checkbox");
    expect(checkbox.closest("label")).toHaveTextContent(
      "I understand and have saved my password somewhere safe.",
    );
  });

  // A class that Tailwind does not know emits nothing and no DOM assertion notices, so the classes
  // that carry the fix are compiled and the rules they must produce are checked.
  it("compiles to padding changes that only apply on a narrow card on a touch screen", async () => {
    const css = (
      await compile(
        `@custom-variant compact (@media ${COMPACT_QUERY});\n@theme { --spacing: 0.25rem; }\n@tailwind utilities;`,
      )
    ).build([`${NARROW}:p-1`, `${NARROW}:px-2`, "p-6", "p-3"]);

    expect(css).toContain(`@media ${COMPACT_QUERY}`);
    expect(css).toMatch(/@container \(width < 15rem\) \{[^}]*padding: var\(--spacing\)\s*;/);
    expect(css).toMatch(
      /@container \(width < 15rem\) \{[^}]*padding-inline: calc\(var\(--spacing\) \* 2\)/,
    );
    // The base rules are unconditional and unchanged, so a normal-size card and the desktop keep them.
    expect(css).toMatch(/\.p-6 \{\s*padding: calc\(var\(--spacing\) \* 6\)/);
    expect(css).toMatch(/\.p-3 \{\s*padding: calc\(var\(--spacing\) \* 3\)/);
  });
});
