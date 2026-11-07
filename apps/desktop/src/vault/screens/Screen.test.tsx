import { render, screen, waitFor } from "@testing-library/react";

import { COMPACT_QUERY } from "@/lib/compact";
import { Screen } from "./Screen";

function stubMatchMedia(compact: boolean) {
  vi.stubGlobal("matchMedia", ((query: string) => ({
    matches: compact && query === COMPACT_QUERY,
    media: query,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
  })) as unknown as typeof window.matchMedia);
}

afterEach(() => vi.unstubAllGlobals());

describe("the pre-vault Screen (personal-cfo-i93n3.26)", () => {
  it("moves focus to its heading when it appears on a compact screen", async () => {
    stubMatchMedia(true);
    render(
      <Screen>
        <h1>Welcome back</h1>
        <input aria-label="Master password" autoFocus />
      </Screen>,
    );
    const heading = screen.getByRole("heading", { name: "Welcome back" });
    // After the field's own autofocus: arriving on a screen lands on its heading.
    await waitFor(() => expect(heading).toHaveFocus());
    expect(heading).toHaveAttribute("tabindex", "-1");
  });

  it("leaves focus alone on a desktop window", () => {
    stubMatchMedia(false);
    render(
      <Screen>
        <h1>Welcome back</h1>
        <input aria-label="Master password" autoFocus />
      </Screen>,
    );
    expect(screen.getByLabelText("Master password")).toHaveFocus();
    expect(screen.getByRole("heading", { name: "Welcome back" })).not.toHaveAttribute("tabindex");
  });

  it("marks its controls for 48-point targets, on every window (the CSS applies it only when compact)", () => {
    stubMatchMedia(false);
    const { container } = render(
      <Screen>
        <h1>Title</h1>
      </Screen>,
    );
    expect(container.firstElementChild).toHaveAttribute("data-comfortable-targets");
  });
});
