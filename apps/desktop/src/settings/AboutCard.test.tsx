import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { BuildInfoDto } from "@/bindings";
import { AboutCard } from "./AboutCard";

const mocks = vi.hoisted(() => ({ buildInfo: vi.fn(), openUrl: vi.fn() }));
vi.mock("@/bindings", () => ({ commands: { buildInfo: mocks.buildInfo } }));
// The opener plugin is mocked at the boundary: every click must reach it through
// the allow-listing helper with the exact URL, and nothing else may.
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));

function info(over: Partial<BuildInfoDto> = {}): BuildInfoDto {
  return {
    version: "0.1.0",
    commit: "abc1234",
    channel: "release",
    built_at: "2026-07-20T14:32:05Z",
    dirty: false,
    ...over,
  };
}

async function renderCard(over: Partial<BuildInfoDto> = {}) {
  mocks.buildInfo.mockResolvedValue(info(over));
  const view = renderWithClient(<AboutCard />);
  // The identity line is query-backed; wait for it so the bug-report link is stamped.
  await screen.findByText(/version 0\.1\.0/i);
  return view;
}

beforeEach(() => {
  mocks.buildInfo.mockReset();
  mocks.openUrl.mockReset();
  mocks.openUrl.mockResolvedValue(undefined);
});

describe("AboutCard (personal-cfo-n76x.18)", () => {
  it("names the app and shows the running build's identity", async () => {
    await renderCard();
    expect(screen.getByText("About")).toBeInTheDocument();
    // The name is the brand lockup (SVG mark + wordmark), not text (4d8.28.3).
    expect(screen.getByRole("img", { name: "DohFlow" }).tagName).toBe("svg");
    expect(
      screen.getByText("Version 0.1.0 · Release build · commit abc1234"),
    ).toBeInTheDocument();
  });

  it("labels a dev build and a modified worktree, like the BuildBadge", async () => {
    await renderCard({ channel: "dev", dirty: true });
    expect(
      screen.getByText("Version 0.1.0 · Development build · commit abc1234 (modified)"),
    ).toBeInTheDocument();
  });

  it.each([
    ["Website", "https://dohflow.app/"],
    ["Help", "https://dohflow.app/help"],
    ["Release notes", "https://dohflow.app/changelog"],
    ["Security policy", "https://dohflow.app/security"],
    ["License (AGPL-3.0-only)", "https://dohflow.app/license"],
    ["Support DohFlow", "https://dohflow.app/sponsor"],
  ])("%s opens exactly %s in the system browser", async (label, url) => {
    await renderCard();
    const link = screen.getByRole("button", { name: label });
    // The destination is discoverable without clicking.
    expect(link).toHaveAttribute("title", url);
    fireEvent.click(link);
    await waitFor(() => expect(mocks.openUrl).toHaveBeenCalledWith(url));
    expect(mocks.openUrl).toHaveBeenCalledTimes(1);
  });

  it("stamps the bug report with version, channel, and commit — and nothing else", async () => {
    await renderCard({ channel: "beta" });
    fireEvent.click(screen.getByRole("button", { name: "Report a bug" }));
    await waitFor(() =>
      expect(mocks.openUrl).toHaveBeenCalledWith(
        "https://dohflow.app/contribute?version=0.1.0&channel=beta&commit=abc1234",
      ),
    );
  });

  it("omits the commit from the bug report when the build has none, like the identity line", async () => {
    await renderCard({ commit: "unknown" });
    expect(screen.getByText("Version 0.1.0 · Release build")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Report a bug" }));
    await waitFor(() =>
      expect(mocks.openUrl).toHaveBeenCalledWith(
        "https://dohflow.app/contribute?version=0.1.0&channel=release",
      ),
    );
  });

  it("lands the bug report on the contribute page, unstamped, before build info resolves", async () => {
    // Never resolves: the card renders without an identity line and the link must
    // not wait on it.
    mocks.buildInfo.mockReturnValue(new Promise<never>(() => undefined));
    renderWithClient(<AboutCard />);
    expect(screen.queryByText(/version/i)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Report a bug" }));
    await waitFor(() =>
      expect(mocks.openUrl).toHaveBeenCalledWith("https://dohflow.app/contribute"),
    );
    expect(mocks.openUrl).toHaveBeenCalledTimes(1);
  });

  it("reports a failed open in the console instead of dropping the rejection", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    try {
      mocks.openUrl.mockRejectedValue(new Error("no default browser"));
      await renderCard();
      fireEvent.click(screen.getByRole("button", { name: "Website" }));
      await waitFor(() =>
        expect(error).toHaveBeenCalledWith(
          "Could not open https://dohflow.app/ in the system browser",
          expect.any(Error),
        ),
      );
    } finally {
      error.mockRestore();
    }
  });

  it("points every link at a page the project controls", async () => {
    const { container } = await renderCard();
    const links = screen.getAllByRole("button");
    expect(links).toHaveLength(7);
    for (const link of links) {
      expect(link.getAttribute("title")).toMatch(/^https:\/\/dohflow\.app\//);
    }
    // Shipped binaries outlive URLs: no repo host, no sponsor platform, no processor.
    expect(container.innerHTML).not.toMatch(/github\.com|ko-fi|patreon|stripe/i);
  });

  it("keeps the Support row to one quiet line — no badge, no prompt", async () => {
    await renderCard();
    const support = screen.getByRole("button", { name: "Support DohFlow" });
    expect(support.closest("p")).toHaveClass("text-muted-foreground");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
