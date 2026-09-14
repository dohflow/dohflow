import { screen } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { BuildInfoDto } from "@/bindings";
import { BuildBadge } from "./BuildBadge";

const mocks = vi.hoisted(() => ({ buildInfo: vi.fn() }));
vi.mock("@/bindings", () => ({ commands: { buildInfo: mocks.buildInfo } }));

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

describe("BuildBadge (personal-cfo-4d8.27.3.2)", () => {
  it("shouts DEV for a development build, so it is never mistaken for the installed app", async () => {
    mocks.buildInfo.mockResolvedValue(info({ channel: "dev" }));
    renderWithClient(<BuildBadge />);
    expect(await screen.findByText("DEV")).toBeInTheDocument();
    // The whole identity is one announcement, not a row of fragments.
    expect(
      screen.getByRole("note", { name: /development build · v0\.1\.0 · commit abc1234/i }),
    ).toBeInTheDocument();
  });

  it("stays muted for a release build, showing the version + commit", async () => {
    mocks.buildInfo.mockResolvedValue(info());
    renderWithClient(<BuildBadge />);
    expect(await screen.findByText("v0.1.0")).toBeInTheDocument();
    expect(screen.queryByText("DEV")).not.toBeInTheDocument();
    expect(screen.getByRole("note", { name: /release build/i })).toBeInTheDocument();
  });

  it("marks a build made from a modified worktree", async () => {
    mocks.buildInfo.mockResolvedValue(info({ dirty: true }));
    renderWithClient(<BuildBadge />);
    expect(await screen.findByText(/abc1234\*/)).toBeInTheDocument();
    expect(
      screen.getByRole("note", { name: /commit abc1234 \(modified\)/i }),
    ).toBeInTheDocument();
  });

  it("labels an explicit channel by its own name, not 'Development'", async () => {
    // build.rs invites a beta pipeline via PCFO_BUILD_CHANNEL; calling it "Development"
    // would be wrong.
    mocks.buildInfo.mockResolvedValue(info({ channel: "beta" }));
    renderWithClient(<BuildBadge />);
    expect(await screen.findByText("beta")).toBeInTheDocument();
    expect(screen.getByRole("note", { name: /beta build/i })).toBeInTheDocument();
  });

  it("does not depend on the network-bound update check", async () => {
    // The identity is compile-time constant: it must render from build_info alone, with
    // no git fetch in the path (the update check can take seconds, or stall offline).
    mocks.buildInfo.mockResolvedValue(info({ channel: "dev" }));
    renderWithClient(<BuildBadge />);
    expect(await screen.findByText("DEV")).toBeInTheDocument();
    expect(mocks.buildInfo).toHaveBeenCalled();
  });
});
