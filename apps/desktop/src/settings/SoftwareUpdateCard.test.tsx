import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { BuildInfoDto, UpdateStatusDto } from "@/bindings";
import { SoftwareUpdateCard } from "./SoftwareUpdateCard";

const mocks = vi.hoisted(() => ({
  checkForUpdate: vi.fn(),
  applyUpdate: vi.fn(),
  relaunchApp: vi.fn(),
  buildInfo: vi.fn(),
  pluginCheck: vi.fn(),
  pluginRelaunch: vi.fn(),
  recordReleaseUpdateFailure: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    checkForUpdate: mocks.checkForUpdate,
    applyUpdate: mocks.applyUpdate,
    relaunchApp: mocks.relaunchApp,
    buildInfo: mocks.buildInfo,
    recordReleaseUpdateFailure: mocks.recordReleaseUpdateFailure,
  },
}));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.pluginCheck }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: mocks.pluginRelaunch }));

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
    latest_commit: "abc1234",
    commits_behind: 0,
    latest_date: null,
    up_to_date: true,
    checked: true,
    error: null,
    ...over,
  };
}

/// A minimal plugin `Update` stub: only the fields/methods the card reads.
function pluginUpdate(over: Partial<Record<string, unknown>> = {}) {
  return {
    version: "0.2.0",
    currentVersion: "0.1.0",
    date: "2026-08-01",
    body: "Bug fixes.",
    downloadAndInstall: vi.fn().mockResolvedValue(undefined),
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.buildInfo.mockResolvedValue(buildInfo());
});

describe("dev channel", () => {
  test("shows up-to-date when the built commit is at the tip", async () => {
    mocks.checkForUpdate.mockResolvedValue(status());
    renderWithClient(<SoftwareUpdateCard />);
    expect(await screen.findByText(/on the latest version/i)).toBeInTheDocument();
    expect(screen.getByText(/0\.1\.0/)).toBeInTheDocument();
  });

  test("surfaces an available update with the commits-behind count", async () => {
    mocks.checkForUpdate.mockResolvedValue(
      status({ up_to_date: false, commits_behind: 2, latest_commit: "def5678", latest_date: "2026-07-01" }),
    );
    renderWithClient(<SoftwareUpdateCard />);
    expect(await screen.findByText(/a new version is available/i)).toBeInTheDocument();
    expect(screen.getByText(/2 commits behind/i)).toBeInTheDocument();
    expect(screen.getByText(/2026-07-01/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /update & relaunch/i })).toBeInTheDocument();
  });

  test("Update & relaunch applies then relaunches", async () => {
    mocks.checkForUpdate.mockResolvedValue(status({ up_to_date: false, commits_behind: 2, latest_commit: "def5678" }));
    mocks.applyUpdate.mockResolvedValue({ ok: true, output_tail: "" });
    mocks.relaunchApp.mockResolvedValue(undefined);
    renderWithClient(<SoftwareUpdateCard />);

    fireEvent.click(await screen.findByRole("button", { name: /update & relaunch/i }));
    await waitFor(() => expect(mocks.applyUpdate).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mocks.relaunchApp).toHaveBeenCalledTimes(1));
  });

  test("a failed update shows the output tail and doesn't relaunch", async () => {
    mocks.checkForUpdate.mockResolvedValue(status({ up_to_date: false, commits_behind: 2, latest_commit: "def5678" }));
    mocks.applyUpdate.mockResolvedValue({ ok: false, output_tail: "error: build failed at step X" });
    renderWithClient(<SoftwareUpdateCard />);

    fireEvent.click(await screen.findByRole("button", { name: /update & relaunch/i }));
    expect(await screen.findByText(/build failed at step X/i)).toBeInTheDocument();
    expect(screen.getByText(/didn't complete/i)).toBeInTheDocument();
    expect(mocks.relaunchApp).not.toHaveBeenCalled();
  });

  test("an unknown built commit shows 'can't determine', not a false update", async () => {
    mocks.checkForUpdate.mockResolvedValue(
      status({ current_commit: "unknown", up_to_date: false, commits_behind: null, latest_commit: "def5678" }),
    );
    renderWithClient(<SoftwareUpdateCard />);
    expect(await screen.findByText(/couldn't determine whether you're up to date/i)).toBeInTheDocument();
    expect(screen.queryByText(/a new version is available/i)).not.toBeInTheDocument();
  });

  test("degrades gracefully when the check can't run", async () => {
    mocks.checkForUpdate.mockResolvedValue(
      status({ checked: false, up_to_date: false, error: "Source checkout not found." }),
    );
    renderWithClient(<SoftwareUpdateCard />);
    expect(await screen.findByText(/source checkout not found/i)).toBeInTheDocument();
  });

  test("the Check button re-runs the check", async () => {
    mocks.checkForUpdate.mockResolvedValue(status());
    renderWithClient(<SoftwareUpdateCard />);
    await screen.findByText(/on the latest version/i);
    expect(mocks.checkForUpdate).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: /check for updates/i }));
    await waitFor(() => expect(mocks.checkForUpdate).toHaveBeenCalledTimes(2));
  });
});

// personal-cfo-867.1.2, ADR 0068: the real signed-artifact updater, release builds only.
describe("release channel", () => {
  beforeEach(() => {
    mocks.buildInfo.mockResolvedValue(buildInfo({ channel: "release" }));
    mocks.recordReleaseUpdateFailure.mockResolvedValue(undefined);
  });

  test("shows up-to-date when the plugin finds no update", async () => {
    mocks.pluginCheck.mockResolvedValue(null);
    renderWithClient(<SoftwareUpdateCard />);
    expect(await screen.findByText(/on the latest version/i)).toBeInTheDocument();
    expect(mocks.checkForUpdate).not.toHaveBeenCalled();
  });

  test("surfaces an available update with the target version and notes", async () => {
    mocks.pluginCheck.mockResolvedValue(pluginUpdate());
    renderWithClient(<SoftwareUpdateCard />);
    expect(await screen.findByText(/a new version is available/i)).toBeInTheDocument();
    expect(screen.getByText(/v0\.2\.0/)).toBeInTheDocument();
    expect(screen.getByText(/bug fixes/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /update & relaunch/i })).toBeInTheDocument();
  });

  test("Update & relaunch downloads, installs, then relaunches via the process plugin", async () => {
    const update = pluginUpdate();
    mocks.pluginCheck.mockResolvedValue(update);
    mocks.pluginRelaunch.mockResolvedValue(undefined);
    renderWithClient(<SoftwareUpdateCard />);

    fireEvent.click(await screen.findByRole("button", { name: /update & relaunch/i }));
    await waitFor(() => expect(update.downloadAndInstall).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mocks.pluginRelaunch).toHaveBeenCalledTimes(1));
    // The dev-channel IPC path is never touched on the release channel.
    expect(mocks.applyUpdate).not.toHaveBeenCalled();
    expect(mocks.relaunchApp).not.toHaveBeenCalled();
  });

  // The negative-test shape personal-cfo-miei asks for: a signature mismatch or tampered
  // artifact surfaces as a rejected `downloadAndInstall()` promise — this pins that the UI
  // shows it as a failed update, never a silent success or an uncaught rejection.
  test("a signature-verification failure shows the error and doesn't relaunch", async () => {
    const update = pluginUpdate({
      downloadAndInstall: vi
        .fn()
        .mockRejectedValue(new Error("signature verification failed")),
    });
    mocks.pluginCheck.mockResolvedValue(update);
    renderWithClient(<SoftwareUpdateCard />);

    fireEvent.click(await screen.findByRole("button", { name: /update & relaunch/i }));
    expect(await screen.findByText(/signature verification failed/i)).toBeInTheDocument();
    expect(screen.getByText(/didn't complete/i)).toBeInTheDocument();
    await waitFor(() =>
      expect(mocks.recordReleaseUpdateFailure).toHaveBeenCalledWith(
        "signature verification failed",
        "signature",
      ),
    );
    expect(mocks.pluginRelaunch).not.toHaveBeenCalled();
  });

  test("a plain-string download failure shows and logs the plugin's real text", async () => {
    const pluginError = "network error: request timed out while downloading the update";
    const update = pluginUpdate({
      downloadAndInstall: vi.fn().mockRejectedValue(pluginError),
    });
    mocks.pluginCheck.mockResolvedValue(update);
    renderWithClient(<SoftwareUpdateCard />);

    fireEvent.click(await screen.findByRole("button", { name: /update & relaunch/i }));

    expect(await screen.findByText(/couldn't download the update/i)).toBeInTheDocument();
    expect(
      screen.getByText(/network error: request timed out while downloading the update/i),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(mocks.recordReleaseUpdateFailure).toHaveBeenCalledWith(pluginError, "download"),
    );
    expect(mocks.pluginRelaunch).not.toHaveBeenCalled();
  });

  test("a non-Error object failure uses its message and classifies installation", async () => {
    const pluginError = { message: "failed to replace the app bundle" };
    const update = pluginUpdate({
      downloadAndInstall: vi.fn().mockRejectedValue(pluginError),
    });
    mocks.pluginCheck.mockResolvedValue(update);
    renderWithClient(<SoftwareUpdateCard />);

    fireEvent.click(await screen.findByRole("button", { name: /update & relaunch/i }));

    expect(await screen.findByText(/couldn't install the update/i)).toBeInTheDocument();
    expect(screen.getByText(/failed to replace the app bundle/i)).toBeInTheDocument();
    await waitFor(() =>
      expect(mocks.recordReleaseUpdateFailure).toHaveBeenCalledWith(pluginError.message, "install"),
    );
    expect(mocks.pluginRelaunch).not.toHaveBeenCalled();
  });

  test("shows the offline state when the check can't reach the network", async () => {
    mocks.pluginCheck.mockRejectedValue(new Error("network error: could not connect"));
    renderWithClient(<SoftwareUpdateCard />);
    expect(
      await screen.findByText(/couldn't reach the update server/i),
    ).toBeInTheDocument();
  });

  test("the Check button re-runs the check", async () => {
    mocks.pluginCheck.mockResolvedValue(null);
    renderWithClient(<SoftwareUpdateCard />);
    await screen.findByText(/on the latest version/i);
    expect(mocks.pluginCheck).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: /check for updates/i }));
    await waitFor(() => expect(mocks.pluginCheck).toHaveBeenCalledTimes(2));
  });
});
