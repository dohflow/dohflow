/**
 * personal-cfo-4d8.39.2, 04b's review of the R1 repair, part 3 cards: (1) an error kept through a
 * retry that fails the SAME way is announced again (its text clears and comes back in the same
 * element) and does not change while the retry is pending; (2) a Vaults rename and a Diagnostics
 * Preview keep their error and slot action through a collapsed retry.
 */

import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";

import type { VaultSummaryDto } from "@/bindings";
import { renderWithClient } from "@/test/renderWithClient";

import { ConnectionsCard } from "./ConnectionsCard";
import { DiagnosticsCard } from "./DiagnosticsCard";
import { GeneralCard } from "./GeneralCard";
import { exampleflow, simplefin } from "./connections/fixtures/adapters";
import { SettingsSection } from "./SettingsSection";
import { SoftwareUpdateCard } from "./SoftwareUpdateCard";
import { VaultsCard } from "./VaultsCard";

const mocks = vi.hoisted(() => ({
  save: vi.fn(),
  diagnosticsPreview: vi.fn(),
  diagnosticsSave: vi.fn(),
  diagnosticsDiscard: vi.fn(),
  checkForUpdate: vi.fn(),
  applyUpdate: vi.fn(),
  relaunchApp: vi.fn(),
  buildInfo: vi.fn(),
  pluginCheck: vi.fn(),
  pluginRelaunch: vi.fn(),
  recordReleaseUpdateFailure: vi.fn(),
  baseCurrency: vi.fn(),
  setBaseCurrency: vi.fn(),
  connectorAdapters: vi.fn(),
  connectorConnections: vi.fn(),
  connectorSync: vi.fn(),
  accountList: vi.fn(),
  openUrl: vi.fn(),
  switchVault: vi.fn(),
  renameVault: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: mocks.save }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.pluginCheck }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: mocks.pluginRelaunch }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));
vi.mock("@/bindings", () => ({ commands: mocks }));
let vaults: VaultSummaryDto[] = [];
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({
    vaults,
    vaultsError: null,
    reloadVaults: vi.fn(),
    createNamedVault: vi.fn(),
    switchVault: mocks.switchVault,
    renameVault: mocks.renameVault,
  }),
  describeIpcError: (error: unknown) => (typeof error === "string" ? error : "VaultLocked"),
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const trigger = (title: string) => screen.getByRole("button", { name: title });
const section = (title: string) =>
  document.querySelector(`[data-settings-section="${title}"]`) as HTMLElement;
const slotOf = (title: string) =>
  section(title).querySelector("[data-settings-persistent]") as HTMLElement;
const held = <T,>() => {
  let release!: (value: T) => void;
  const promise = new Promise<T>((resolve) => (release = resolve));
  return { promise, release };
};
/// Collapse, run `action` from the slot with `finish` still pending, check what stays, then settle.
async function expectEndsOnHeading(title: string, opts: {
  action: () => HTMLElement;
  pending: () => void;
  settle: () => Promise<void>;
  gone: () => void;
}) {
  fireEvent.click(trigger(title));
  expect(trigger(title)).toHaveAttribute("aria-expanded", "false");
  const action = opts.action();
  action.focus();
  opts.pending();
  await opts.settle();
  await waitFor(opts.gone);
  await waitFor(() => expect(document.activeElement).toBe(trigger(title)));
  expect(trigger(title)).toHaveAttribute("aria-expanded", "false");
}

beforeEach(() => {
  vi.clearAllMocks();
  vaults = [];
  mocks.diagnosticsDiscard.mockResolvedValue(ok(null));
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.setBaseCurrency.mockResolvedValue(ok(null));
  mocks.connectorAdapters.mockResolvedValue([simplefin, exampleflow]);
  mocks.connectorConnections.mockResolvedValue(ok([]));
  mocks.accountList.mockResolvedValue(ok([]));
});


import { watchAlert } from "@/test/alertChanges";

describe("an identical repeat failure is announced again, and a pending retry changes nothing", () => {
  it("Diagnostics: a refused Save", async () => {
    mocks.diagnosticsPreview.mockResolvedValue(
      ok({ snapshot_id: 7, text: "{}", records: 2, dropped: 0, rejected: 0, suggested_file_name: "d.json" }),
    );
    mocks.save.mockResolvedValue("/tmp/d.json");
    mocks.diagnosticsSave.mockResolvedValueOnce(ok("permission_denied"));
    renderWithClient(
      <SettingsSection title="Diagnostics" span="full">
        <DiagnosticsCard />
      </SettingsSection>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Preview diagnostics" }));
    await screen.findByLabelText(/exactly what will be saved/i);
    fireEvent.click(screen.getByRole("button", { name: "Save…" }));
    const alert = await screen.findByText(/permission to save there/i);
    const retry = held<ReturnType<typeof ok>>();
    mocks.diagnosticsSave.mockReturnValueOnce(retry.promise);
    const pending = watchAlert(alert);
    fireEvent.click(within(slotOf("Diagnostics")).getByRole("button", { name: "Save…" }));
    await waitFor(() => expect(mocks.diagnosticsSave).toHaveBeenCalledTimes(2));
    expect(screen.getByText(/permission to save there/i)).toBe(alert);
    expect(pending.stop()).toBe(0);
    const again = watchAlert(alert);
    await act(async () => retry.release(ok("permission_denied")));
    await waitFor(() => expect(again.count()).toBeGreaterThanOrEqual(2));
    expect(screen.getByText(/permission to save there/i)).toBe(alert);
  });

  it("Software update: a failed install", async () => {
    mocks.buildInfo.mockResolvedValue({
      version: "0.1.0", commit: "abc1234", channel: "dev", built_at: "2026-07-20T14:32:05Z", dirty: false,
      os_version: "15.7.9", self_update_available: true, system_backup_device: null,
    });
    mocks.checkForUpdate.mockResolvedValue({
      current_version: "0.1.0", current_commit: "abc1234", build_channel: "dev", build_time: "2026-07-20T14:32:05Z",
      build_dirty: false, latest_commit: "def5678", commits_behind: 2, latest_date: null, up_to_date: false,
      checked: true, error: null,
    });
    mocks.applyUpdate.mockResolvedValueOnce({ ok: false, output_tail: "error: build failed" });
    renderWithClient(
      <SettingsSection title="Software update" span="half">
        <SoftwareUpdateCard />
      </SettingsSection>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Update & relaunch" }));
    const output = await screen.findByText("error: build failed");
    const alert = output.closest('[role="alert"]') as HTMLElement;
    const retry = held<{ ok: boolean; output_tail: string }>();
    mocks.applyUpdate.mockReturnValueOnce(retry.promise);
    const pending = watchAlert(alert);
    fireEvent.click(within(slotOf("Software update")).getByRole("button", { name: "Update & relaunch" }));
    await waitFor(() => expect(mocks.applyUpdate).toHaveBeenCalledTimes(2));
    expect(pending.stop()).toBe(0);
    const again = watchAlert(alert);
    await act(async () => retry.release({ ok: false, output_tail: "error: build failed" }));
    await waitFor(() => expect(again.count()).toBeGreaterThanOrEqual(2));
    expect(screen.getByText("error: build failed")).toBeInTheDocument();
  });

  it("General: a refused currency save", async () => {
    mocks.setBaseCurrency.mockResolvedValueOnce({ status: "error", error: "Persistence failure" });
    renderWithClient(
      <SettingsSection title="General" span="full">
        <GeneralCard />
      </SettingsSection>,
    );
    fireEvent.change(await screen.findByLabelText("Base currency"), { target: { value: "EUR" } });
    const alert = await screen.findByText("Persistence failure");
    const retry = held<{ status: "error"; error: string }>();
    mocks.setBaseCurrency.mockReturnValueOnce(retry.promise);
    const pending = watchAlert(alert);
    fireEvent.change(within(slotOf("General")).getByLabelText("Base currency"), { target: { value: "EUR" } });
    await waitFor(() => expect(mocks.setBaseCurrency).toHaveBeenCalledTimes(2));
    expect(pending.stop()).toBe(0);
    const again = watchAlert(alert);
    await act(async () => retry.release({ status: "error", error: "Persistence failure" }));
    await waitFor(() => expect(again.count()).toBeGreaterThanOrEqual(2));
    expect(screen.getByText("Persistence failure")).toBe(alert);
  });

  it("Vaults: a refused Switch", async () => {
    vaults = [
      { id: "real", name: "Real", is_active: true, created_at: "2026-07-01T00:00:00Z" },
      { id: "test", name: "Test", is_active: false, created_at: "2026-07-01T00:00:00Z" },
    ];
    mocks.switchVault.mockResolvedValueOnce("Persistence failure");
    renderWithClient(
      <SettingsSection title="Vaults" span="full">
        <VaultsCard />
      </SettingsSection>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Switch" }));
    const alert = await screen.findByText("Persistence failure");
    const retry = held<string>();
    mocks.switchVault.mockReturnValueOnce(retry.promise);
    const pending = watchAlert(alert);
    fireEvent.click(within(slotOf("Vaults")).getByRole("button", { name: "Switch" }));
    await waitFor(() => expect(mocks.switchVault).toHaveBeenCalledTimes(2));
    expect(pending.stop()).toBe(0);
    const again = watchAlert(alert);
    await act(async () => retry.release("Persistence failure"));
    await waitFor(() => expect(again.count()).toBeGreaterThanOrEqual(2));
    expect(screen.getByText("Persistence failure")).toBe(alert);
  });

  it("Connections: a failed Refresh now", async () => {
    const connection = {
      id: "a", adapter_id: "simplefin", display_hint: "Bank a", last_synced_at: "2026-08-22T10:00:00Z",
      last_error: null, refresh_cadence: "every_open", attention: [], links: [],
    };
    const result = (message: string) =>
      ok({ connection_id: "a", status: "failed", staged: 0, committed: 0, flagged: 0, skipped_unmapped: 0, warnings: [], message });
    mocks.connectorConnections.mockResolvedValue(ok([connection]));
    mocks.connectorSync.mockResolvedValueOnce(result("provider said no"));
    renderWithClient(
      <SettingsSection title="Connections" span="full">
        <ConnectionsCard />
      </SettingsSection>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Refresh now" }));
    const alert = await screen.findByText("provider said no");
    const retry = held<ReturnType<typeof result>>();
    mocks.connectorSync.mockReturnValueOnce(retry.promise);
    const pending = watchAlert(alert);
    fireEvent.click(within(slotOf("Connections")).getByRole("button", { name: "Refresh now" }));
    await waitFor(() => expect(mocks.connectorSync).toHaveBeenCalledTimes(2));
    expect(pending.stop()).toBe(0);
    const again = watchAlert(alert);
    await act(async () => retry.release(result("provider said no")));
    await waitFor(() => expect(again.count()).toBeGreaterThanOrEqual(2));
    expect(screen.getByText("provider said no")).toBe(alert);
  });
});

describe("collapsed retries keep their error and action until they settle", () => {
  it("Vaults: a rename retry", async () => {
    vaults = [
      { id: "real", name: "Real", is_active: true, created_at: "2026-07-01T00:00:00Z" },
      { id: "test", name: "Test", is_active: false, created_at: "2026-07-01T00:00:00Z" },
    ];
    mocks.renameVault.mockResolvedValueOnce("Persistence failure");
    renderWithClient(
      <SettingsSection title="Vaults" span="full">
        <VaultsCard />
      </SettingsSection>,
    );
    fireEvent.click(screen.getAllByRole("button", { name: "Rename" })[1]!);
    fireEvent.change(screen.getByLabelText("New name for Test"), { target: { value: "Scratch" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByText("Persistence failure");
    const retry = held<string | null>();
    mocks.renameVault.mockReturnValueOnce(retry.promise);
    await expectEndsOnHeading("Vaults", {
      action: () => within(slotOf("Vaults")).getByRole("button", { name: "Save" }),
      pending: () => {
        fireEvent.click(within(slotOf("Vaults")).getByRole("button", { name: "Save" }));
      },
      settle: async () => {
        await waitFor(() => expect(mocks.renameVault).toHaveBeenCalledTimes(2));
        // Pending: the error and the slot's Save are still on screen.
        expect(within(slotOf("Vaults")).getByText("Persistence failure")).toBeInTheDocument();
        expect(within(slotOf("Vaults")).getByRole("button", { name: "Save" })).toBeInTheDocument();
        await act(async () => retry.release(null));
      },
      gone: () => expect(screen.queryByText("Persistence failure")).toBeNull(),
    });
  });

  it("Diagnostics: a Preview retry", async () => {
    mocks.diagnosticsPreview.mockResolvedValueOnce({ status: "error", error: "VaultLocked" });
    renderWithClient(
      <SettingsSection title="Diagnostics" span="full">
        <DiagnosticsCard />
      </SettingsSection>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Preview diagnostics" }));
    await screen.findByText("VaultLocked");
    const retry = held<ReturnType<typeof ok>>();
    mocks.diagnosticsPreview.mockReturnValueOnce(retry.promise);
    await expectEndsOnHeading("Diagnostics", {
      action: () => within(slotOf("Diagnostics")).getByRole("button", { name: "Preview diagnostics" }),
      pending: () => {
        fireEvent.click(within(slotOf("Diagnostics")).getByRole("button", { name: /Preview diagnostics/ }));
      },
      settle: async () => {
        await waitFor(() => expect(mocks.diagnosticsPreview).toHaveBeenCalledTimes(2));
        expect(within(slotOf("Diagnostics")).getByText("VaultLocked")).toBeInTheDocument();
        expect(within(slotOf("Diagnostics")).getByRole("button", { name: /Preview diagnostics/ })).toBeInTheDocument();
        await act(async () =>
          retry.release(
            ok({ snapshot_id: 8, text: "{}", records: 1, dropped: 0, rejected: 0, suggested_file_name: "d.json" }),
          ),
        );
      },
      gone: () => expect(screen.queryByText("VaultLocked")).toBeNull(),
    });
  });
});
