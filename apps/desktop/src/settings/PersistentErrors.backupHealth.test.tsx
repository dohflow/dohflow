/**
 * personal-cfo-4d8.39.2: Backups and Vault health put each error, its remedy and its one recovery
 * action in the section's persistent slot (slice b, part 2). The action is rendered once, in the
 * slot while the error shows, and nothing runs because a section was collapsed.
 */

import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { BackupScheduleCard } from "@/backup/BackupScheduleCard";
import { renderWithClient } from "@/test/renderWithClient";

import type { VaultHealthDto } from "@/bindings";
import { SettingsSection } from "./SettingsSection";
import { VaultHealthCard } from "./VaultHealthCard";

const mocks = vi.hoisted(() => ({
  buildInfo: vi.fn(),
  backupScheduleSettings: vi.fn(),
  backupHistory: vi.fn(),
  configureBackup: vi.fn(),
  runBackupNow: vi.fn(),
  exportBackup: vi.fn(),
  open: vi.fn(),
  save: vi.fn(),
  vaultHealth: vi.fn(),
  rebuildReadModels: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn().mockResolvedValue("/Users/test") }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open, save: mocks.save }));
vi.mock("@/vault/useVault", () => ({ describeIpcError: (error: unknown) => String(error) }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const failed = (message: string) => ({ status: "error", error: message }) as const;
const FOLDER = "/Users/test/Backups";
const settings = (over: Record<string, unknown> = {}) => ({
  cadence: "weekly",
  destination: FOLDER,
  next_due_at: null,
  last_run_at: null,
  last_error: null,
  ...over,
});
const health = (over: Partial<VaultHealthDto> = {}): VaultHealthDto => ({
  writer_healthy: true,
  wal_configured: true,
  schema_coherent: true,
  integrity_ok: true,
  attachments_consistent: true,
  read_models_current: true,
  is_healthy: true,
  ...over,
});

beforeEach(() => {
  vi.clearAllMocks();
  mocks.buildInfo.mockResolvedValue({
    version: "0.3.0",
    commit: "abc1234",
    channel: "release",
    built_at: "2026-07-20T14:32:05Z",
    dirty: false,
    os_version: null,
    self_update_available: false,
    system_backup_device: null,
  });
  mocks.backupScheduleSettings.mockResolvedValue(ok(settings()));
  mocks.backupHistory.mockResolvedValue(ok([]));
  mocks.configureBackup.mockResolvedValue(ok(settings()));
  mocks.runBackupNow.mockResolvedValue(ok({ destination: `${FOLDER}/new.pcfobk` }));
  mocks.open.mockResolvedValue(null);
  mocks.exportBackup.mockResolvedValue(ok(null));
  mocks.vaultHealth.mockResolvedValue(ok(health()));
  mocks.rebuildReadModels.mockResolvedValue(ok(3));
});

const toggle = (title: string) => fireEvent.click(screen.getByRole("button", { name: title }));
const section = (title: string) =>
  document.querySelector(`[data-settings-section="${title}"]`) as HTMLElement;
const everyButton = (title: string, name: string | RegExp) =>
  within(section(title)).queryAllByRole("button", { name, hidden: true });
const slotOf = (title: string) =>
  section(title).querySelector("[data-settings-persistent]") as HTMLElement;
/// Exactly one such button in the section, and it is in the slot.
function expectOnlyInSlot(title: string, name: string | RegExp) {
  const found = everyButton(title, name);
  expect(found).toHaveLength(1);
  expect(slotOf(title).contains(found[0]!)).toBe(true);
}

describe("Backups", () => {
  const render = () =>
    renderWithClient(
      <SettingsSection title="Backups" span="oneOfThree">
        <BackupScheduleCard />
      </SettingsSection>,
    );
  const ready = () => screen.findByRole("button", { name: "Save backup settings" });

  it("a failed scheduled backup keeps its message and its one Back up now while collapsed; nothing runs on collapse", async () => {
    mocks.backupScheduleSettings.mockResolvedValue(ok(settings({ last_error: "disk is full" })));
    render();
    const alert = await screen.findByText("Scheduled backup failed: disk is full");
    expect(slotOf("Backups").contains(alert)).toBe(true);
    expectOnlyInSlot("Backups", "Back up now");

    toggle("Backups");
    expect(screen.getByText("Scheduled backup failed: disk is full")).toBeVisible();
    expect(screen.getByRole("button", { name: "Back up now" })).toBeVisible();
    expect(mocks.runBackupNow).not.toHaveBeenCalled();
    expect(mocks.configureBackup).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Back up now" }));
    await waitFor(() => expect(mocks.runBackupNow).toHaveBeenCalledTimes(1));
  });

  it("a failed Back up now keeps its error and its one button in the slot", async () => {
    mocks.runBackupNow.mockRejectedValue(new Error("The destination is read-only."));
    render();
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "Back up now" }));
    const alert = await screen.findByText("The destination is read-only.");
    expect(slotOf("Backups").contains(alert)).toBe(true);
    expectOnlyInSlot("Backups", "Back up now");
    toggle("Backups");
    expect(screen.getByText("The destination is read-only.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Back up now" })).toBeVisible();
  });

  it("a failed Save keeps the draft, its message and its one Save backup settings in the slot", async () => {
    mocks.configureBackup.mockResolvedValue(failed("That folder cannot be used."));
    mocks.open.mockResolvedValue("/Users/test/Elsewhere");
    render();
    await ready();
    fireEvent.click(screen.getByRole("button", { name: /choose folder/i }));
    await screen.findByText("/Users/test/Elsewhere");
    fireEvent.click(screen.getByRole("button", { name: "Save backup settings" }));
    const alert = await screen.findByText("That folder cannot be used.");
    expect(slotOf("Backups").contains(alert)).toBe(true);
    expectOnlyInSlot("Backups", "Save backup settings");

    toggle("Backups");
    expect(screen.getByRole("button", { name: "Save backup settings" })).toBeVisible();
    toggle("Backups");
    // The draft is where the person left it, and the save-first guard still holds.
    expect(screen.getByText("/Users/test/Elsewhere")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Back up now" })).toBeDisabled();
    expect(screen.getByText("Save backup settings before using Back up now.")).toBeInTheDocument();

    // The slot's button saves the same draft, and returns to the body when it works.
    mocks.configureBackup.mockResolvedValue(ok(settings({ destination: "/Users/test/Elsewhere" })));
    fireEvent.click(screen.getByRole("button", { name: "Save backup settings" }));
    await waitFor(() => expect(mocks.configureBackup).toHaveBeenLastCalledWith("weekly", "/Users/test/Elsewhere"));
    await waitFor(() => expect(screen.queryByText("That folder cannot be used.")).toBeNull());
    const saves = everyButton("Backups", "Save backup settings");
    expect(saves).toHaveLength(1);
    expect(slotOf("Backups").contains(saves[0]!)).toBe(false);
  });

  it("the folder picker error keeps its one Choose folder… in the slot", async () => {
    mocks.open.mockRejectedValue(new Error("no dialog"));
    render();
    await ready();
    fireEvent.click(screen.getByRole("button", { name: /choose folder/i }));
    const alert = await screen.findByText("Could not open the folder picker.");
    expect(slotOf("Backups").contains(alert)).toBe(true);
    expectOnlyInSlot("Backups", /choose folder/i);
  });

  it("the save-first note travels with Back up now, and is described once", async () => {
    mocks.backupScheduleSettings.mockResolvedValue(ok(settings({ last_error: "disk is full" })));
    mocks.open.mockResolvedValue("/Users/test/Elsewhere");
    render();
    await screen.findByText("Scheduled backup failed: disk is full");
    fireEvent.click(screen.getByRole("button", { name: /choose folder/i }));
    await screen.findByText("/Users/test/Elsewhere");
    const note = screen.getByText("Save backup settings before using Back up now.");
    expect(slotOf("Backups").contains(note)).toBe(true);
    expect(document.querySelectorAll("#backup-now-save-first")).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Back up now" })).toHaveAccessibleDescription(
      "Save backup settings before using Back up now.",
    );
  });

  it("a failed settings read and a failed history read stay visible while collapsed", async () => {
    mocks.backupScheduleSettings.mockResolvedValue(failed("Settings are unavailable."));
    mocks.backupHistory.mockResolvedValue(failed("History is unavailable."));
    render();
    const settingsAlert = await screen.findByText("Settings are unavailable.");
    const historyAlert = await screen.findByText("History is unavailable.");
    toggle("Backups");
    expect(settingsAlert).toBeVisible();
    expect(historyAlert).toBeVisible();
  });
});

describe("Vault health", () => {
  const render = () =>
    renderWithClient(
      <SettingsSection title="Vault health" span="full">
        <VaultHealthCard />
      </SettingsSection>,
    );

  it("a failed check keeps the message and its one Re-check while collapsed, and Re-check reads again", async () => {
    mocks.vaultHealth.mockResolvedValueOnce(failed("The health check could not run."));
    render();
    const alert = await screen.findByText("The health check could not run.");
    expect(slotOf("Vault health").contains(alert)).toBe(true);
    expectOnlyInSlot("Vault health", "Re-check");
    toggle("Vault health");
    expect(screen.getByText("The health check could not run.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Re-check" })).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Re-check" }));
    await waitFor(() => expect(mocks.vaultHealth).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByText("The health check could not run.")).toBeNull());
    // The check passed: Re-check is back in the body, once.
    const found = everyButton("Vault health", "Re-check");
    expect(found).toHaveLength(1);
    expect(slotOf("Vault health").contains(found[0]!)).toBe(false);
  });

  it("data-views drift keeps its message and its one Rebuild while collapsed, and a failed rebuild its error", async () => {
    mocks.vaultHealth.mockResolvedValue(ok(health({ read_models_current: false, is_healthy: false })));
    mocks.rebuildReadModels.mockResolvedValue(failed("The rebuild was interrupted."));
    render();
    const drift = await screen.findByText(/Your data views drifted from the source records/);
    expect(slotOf("Vault health").contains(drift)).toBe(true);
    expectOnlyInSlot("Vault health", "Rebuild data views");
    toggle("Vault health");
    expect(screen.getByRole("button", { name: "Rebuild data views" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Rebuild data views" }));
    const repairError = await screen.findByText("The rebuild was interrupted.");
    expect(slotOf("Vault health").contains(repairError)).toBe(true);
    expect(repairError).toBeVisible();
    expect(mocks.rebuildReadModels).toHaveBeenCalledTimes(1);
  });

  it("a failing check shows its complete remedy while collapsed, and the checks stay in the body", async () => {
    mocks.vaultHealth.mockResolvedValue(ok(health({ integrity_ok: false, is_healthy: false })));
    render();
    const remedy = await screen.findByText(
      "Database integrity: The database file may be damaged. Lock the vault, then restore your most recent backup as a separate vault from the vault picker.",
    );
    expect(slotOf("Vault health").contains(remedy)).toBe(true);
    toggle("Vault health");
    expect(remedy).toBeVisible();
    // No rebuild is offered for corruption, and Re-check stays in the body, once.
    expect(everyButton("Vault health", "Rebuild data views")).toHaveLength(0);
    const recheck = everyButton("Vault health", "Re-check");
    expect(recheck).toHaveLength(1);
    expect(slotOf("Vault health").contains(recheck[0]!)).toBe(false);
  });

  it("a healthy vault puts nothing in the slot", async () => {
    render();
    await screen.findByText(/your vault is healthy/i);
    expect(slotOf("Vault health")).toBeEmptyDOMElement();
  });
});
