/**
 * personal-cfo-4d8.39.2 (slice b, part 3): Software update puts each error, its full message and its
 * one recovery action in the section's persistent slot. A failed check recovers with Check for
 * updates, a failed install with Update & relaunch; each is rendered once, in the slot while its
 * error shows, and nothing runs because the section was collapsed.
 */

import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { BuildInfoDto, UpdateStatusDto } from "@/bindings";
import { SettingsSection } from "./SettingsSection";
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
vi.mock("@/bindings", () => ({ commands: mocks }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.pluginCheck }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: mocks.pluginRelaunch }));

const buildInfo = (channel: string): BuildInfoDto => ({
  version: "0.1.0",
  commit: "abc1234",
  channel,
  built_at: "2026-07-20T14:32:05Z",
  dirty: false,
  os_version: "15.7.9",
  self_update_available: true,
  system_backup_device: null,
});
const status = (over: Partial<UpdateStatusDto> = {}): UpdateStatusDto => ({
  current_version: "0.1.0",
  current_commit: "abc1234",
  build_channel: "dev",
  build_time: "2026-07-20T14:32:05Z",
  build_dirty: false,
  latest_commit: "abc1234",
  commits_behind: 0,
  latest_date: null,
  up_to_date: true,
  checked: true,
  error: null,
  ...over,
});
const behind = () => status({ up_to_date: false, commits_behind: 2, latest_commit: "def5678" });

beforeEach(() => {
  vi.clearAllMocks();
  mocks.buildInfo.mockResolvedValue(buildInfo("dev"));
});

const render = () =>
  renderWithClient(
    <SettingsSection title="Software update" span="half">
      <SoftwareUpdateCard />
    </SettingsSection>,
  );
const section = () =>
  document.querySelector(`[data-settings-section="Software update"]`) as HTMLElement;
const slot = () => section().querySelector("[data-settings-persistent]") as HTMLElement;
const everyButton = (name: string | RegExp) =>
  within(section()).queryAllByRole("button", { name, hidden: true });
/// Exactly one such button in the section, and it is in the slot.
function expectOnlyInSlot(name: string | RegExp) {
  const found = everyButton(name);
  expect(found).toHaveLength(1);
  expect(slot().contains(found[0]!)).toBe(true);
}
const collapse = () => fireEvent.click(screen.getByRole("button", { name: "Software update" }));

describe("Software update", () => {
  it("a failed check keeps its message and its one Check for updates while collapsed", async () => {
    mocks.checkForUpdate.mockResolvedValueOnce(
      status({ checked: false, up_to_date: false, error: "git fetch failed: no route to host" }),
    );
    render();
    const message = await screen.findByText("git fetch failed: no route to host");
    expect(slot().contains(message)).toBe(true);
    expectOnlyInSlot("Check for updates");

    collapse();
    expect(screen.getByText("git fetch failed: no route to host")).toBeVisible();
    expect(screen.getByRole("button", { name: "Check for updates" })).toBeVisible();
    expect(mocks.checkForUpdate).toHaveBeenCalledTimes(1);

    mocks.checkForUpdate.mockResolvedValueOnce(status());
    fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    await waitFor(() => expect(mocks.checkForUpdate).toHaveBeenCalledTimes(2));
    // Recovered: the message goes, and the one button is back in the body.
    await screen.findByText(/on the latest version/i);
    expect(screen.queryByText("git fetch failed: no route to host")).not.toBeInTheDocument();
    expect(everyButton("Check for updates")).toHaveLength(1);
    expect(slot().contains(everyButton("Check for updates")[0]!)).toBe(false);
  });

  it("the offline state keeps its full copy and its one Check for updates in the slot", async () => {
    mocks.buildInfo.mockResolvedValue(buildInfo("release"));
    mocks.pluginCheck.mockRejectedValue(new Error("network error: could not connect"));
    render();
    const message = await screen.findByText(/couldn't reach the update server/i);
    expect(slot().contains(message)).toBe(true);
    expect(message.textContent).toContain("Check your connection and try again.");
    expectOnlyInSlot("Check for updates");
  });

  it("a failed install keeps its full output and its one Update & relaunch while collapsed", async () => {
    mocks.checkForUpdate.mockResolvedValue(behind());
    mocks.applyUpdate.mockResolvedValueOnce({ ok: false, output_tail: "error: build failed at step X" });
    render();
    fireEvent.click(await screen.findByRole("button", { name: "Update & relaunch" }));
    const output = await screen.findByText("error: build failed at step X");
    expect(slot().contains(output)).toBe(true);
    expect(slot()).toHaveTextContent("The update didn't complete.");
    expect(slot()).toHaveTextContent("./scripts/update-app.sh");
    expectOnlyInSlot("Update & relaunch");

    collapse();
    expect(screen.getByText("error: build failed at step X")).toBeVisible();
    expect(screen.getByRole("button", { name: "Update & relaunch" })).toBeVisible();
    expect(mocks.applyUpdate).toHaveBeenCalledTimes(1);
    expect(mocks.relaunchApp).not.toHaveBeenCalled();

    mocks.applyUpdate.mockResolvedValueOnce({ ok: true, output_tail: "" });
    mocks.relaunchApp.mockResolvedValue(undefined);
    fireEvent.click(screen.getByRole("button", { name: "Update & relaunch" }));
    await waitFor(() => expect(mocks.applyUpdate).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(mocks.relaunchApp).toHaveBeenCalledTimes(1));
  });

  it("focus stays on Update & relaunch when a failed install moves it into the slot", async () => {
    mocks.checkForUpdate.mockResolvedValue(behind());
    mocks.applyUpdate.mockResolvedValueOnce({ ok: false, output_tail: "error: build failed" });
    render();
    const update = await screen.findByRole("button", { name: "Update & relaunch" });
    update.focus();
    fireEvent.click(update);
    await screen.findByText("error: build failed");
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Update & relaunch" })),
    );
    expect(slot().contains(document.activeElement)).toBe(true);
  });

  it("focus stays on Check for updates when a failed check moves it into the slot", async () => {
    mocks.checkForUpdate.mockResolvedValueOnce(status());
    render();
    const check = await screen.findByRole("button", { name: "Check for updates" });
    check.focus();
    mocks.checkForUpdate.mockResolvedValueOnce(
      status({ checked: false, up_to_date: false, error: "git fetch failed" }),
    );
    fireEvent.click(check);
    await screen.findByText("git fetch failed");
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Check for updates" })),
    );
    expect(slot().contains(document.activeElement)).toBe(true);
  });

  it("with no error, both buttons are in the body and the slot is empty", async () => {
    mocks.checkForUpdate.mockResolvedValue(behind());
    render();
    await screen.findByRole("button", { name: "Update & relaunch" });
    expect(slot()).toBeEmptyDOMElement();
  });
});
