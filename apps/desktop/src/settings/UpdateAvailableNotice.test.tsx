import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { BuildInfoDto, UpdateStatusDto } from "@/bindings";
import { UpdateAvailableNotice } from "./UpdateAvailableNotice";

const mocks = vi.hoisted(() => ({
  checkForUpdate: vi.fn(),
  buildInfo: vi.fn(),
  pluginCheck: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: { checkForUpdate: mocks.checkForUpdate, buildInfo: mocks.buildInfo },
}));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.pluginCheck }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));

function buildInfo(over: Partial<BuildInfoDto> = {}): BuildInfoDto {
  return {
    version: "0.1.0",
    commit: "abc1234",
    channel: "dev",
    built_at: "2026-07-20T14:32:05Z",
    dirty: false,
    ...over,
  };
}

function status(over: Partial<UpdateStatusDto> = {}): UpdateStatusDto {
  return {
    current_version: "0.1.0",
    current_commit: "abc1234",
    build_channel: "release",
    build_time: "2026-07-20T14:32:05Z",
    build_dirty: false,
    latest_commit: "def5678",
    commits_behind: 3,
    latest_date: "2026-07-01",
    up_to_date: false,
    checked: true,
    error: null,
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.buildInfo.mockResolvedValue(buildInfo());
});

describe("dev channel", () => {
  test("appears when behind and routes to settings on Update", async () => {
    mocks.checkForUpdate.mockResolvedValue(status());
    const onGoToSettings = vi.fn();
    renderWithClient(<UpdateAvailableNotice onGoToSettings={onGoToSettings} />);

    const notice = await screen.findByRole("status", { name: /update available/i });
    expect(notice).toHaveTextContent(/new version is available/i);
    expect(notice).toHaveTextContent(/3 commits behind/i);

    fireEvent.click(screen.getByRole("button", { name: /^update$/i }));
    expect(onGoToSettings).toHaveBeenCalled();
    await waitFor(() =>
      expect(screen.queryByRole("status", { name: /update available/i })).not.toBeInTheDocument(),
    );
  });

  test("can be dismissed", async () => {
    mocks.checkForUpdate.mockResolvedValue(status());
    renderWithClient(<UpdateAvailableNotice onGoToSettings={() => {}} />);
    await screen.findByRole("status", { name: /update available/i });
    fireEvent.click(screen.getByRole("button", { name: /dismiss/i }));
    await waitFor(() =>
      expect(screen.queryByRole("status", { name: /update available/i })).not.toBeInTheDocument(),
    );
  });

  test("renders nothing when up to date", async () => {
    mocks.checkForUpdate.mockResolvedValue(
      status({ up_to_date: true, commits_behind: 0, latest_commit: "abc1234" }),
    );
    const { container } = renderWithClient(
      <UpdateAvailableNotice onGoToSettings={() => {}} />,
    );
    // Give the query a tick to resolve, then assert nothing rendered.
    await waitFor(() => expect(mocks.checkForUpdate).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });
});

// personal-cfo-867.1.2, ADR 0068.
describe("release channel", () => {
  beforeEach(() => {
    mocks.buildInfo.mockResolvedValue(buildInfo({ channel: "release" }));
  });

  test("appears with the target version when the plugin finds an update", async () => {
    mocks.pluginCheck.mockResolvedValue({
      version: "0.2.0",
      currentVersion: "0.1.0",
      date: "2026-08-01",
      body: "Bug fixes.",
      downloadAndInstall: vi.fn(),
    });
    renderWithClient(<UpdateAvailableNotice onGoToSettings={() => {}} />);
    const notice = await screen.findByRole("status", { name: /update available/i });
    expect(notice).toHaveTextContent(/version 0\.2\.0 is available/i);
  });

  test("renders nothing when no update is found", async () => {
    mocks.pluginCheck.mockResolvedValue(null);
    const { container } = renderWithClient(
      <UpdateAvailableNotice onGoToSettings={() => {}} />,
    );
    await waitFor(() => expect(mocks.pluginCheck).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });
});
