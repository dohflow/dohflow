/**
 * personal-cfo-4d8.39.2, Studio R1, part 3 cards: a recovery run from a COLLAPSED section ends on the
 * section's own heading (the one visible control), not on a hidden fallback, and the error and the
 * slot's action stay until the retry settles. See PersistentErrors.collapsedFocus.test.tsx for why
 * these assert the chosen target in jsdom.
 */

import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";

import type { BuildInfoDto, UpdateStatusDto, VaultSummaryDto } from "@/bindings";
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
    renameVault: vi.fn(),
  }),
  describeIpcError: (error: unknown) => (typeof error === "string" ? error : "Something went wrong."),
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

describe("Diagnostics", () => {
  it("a Save that works from the slot ends on the section's heading", async () => {
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
    await screen.findByText(/permission to save there/i);
    const retry = held<ReturnType<typeof ok>>();
    mocks.diagnosticsSave.mockReturnValueOnce(retry.promise);
    await expectEndsOnHeading("Diagnostics", {
      action: () => within(slotOf("Diagnostics")).getByRole("button", { name: "Save…" }),
      pending: () => {
        fireEvent.click(within(slotOf("Diagnostics")).getByRole("button", { name: /Save…|Saving…/ }));
      },
      settle: async () => {
        await waitFor(() => expect(mocks.diagnosticsSave).toHaveBeenCalledTimes(2));
        // Pending: the error and the slot's action are still on screen.
        expect(within(slotOf("Diagnostics")).getByText(/permission to save there/i)).toBeInTheDocument();
        expect(within(slotOf("Diagnostics")).getByRole("button", { name: /Save…|Saving…/ })).toBeInTheDocument();
        await act(async () => retry.release(ok("saved")));
      },
      gone: () => expect(screen.queryByText(/permission to save there/i)).toBeNull(),
    });
  });
});

describe("Software update", () => {
  it("a Check for updates that works from the slot ends on the section's heading", async () => {
    const dev: BuildInfoDto = {
      version: "0.1.0",
      commit: "abc1234",
      channel: "dev",
      built_at: "2026-07-20T14:32:05Z",
      dirty: false,
      os_version: "15.7.9",
      self_update_available: true,
      system_backup_device: null,
    };
    const status = (over: Partial<UpdateStatusDto>): UpdateStatusDto => ({
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
    mocks.buildInfo.mockResolvedValue(dev);
    mocks.checkForUpdate.mockResolvedValueOnce(
      status({ checked: false, up_to_date: false, error: "git fetch failed" }),
    );
    renderWithClient(
      <SettingsSection title="Software update" span="half">
        <SoftwareUpdateCard />
      </SettingsSection>,
    );
    await screen.findByText("git fetch failed");
    const retry = held<UpdateStatusDto>();
    mocks.checkForUpdate.mockReturnValueOnce(retry.promise);
    await expectEndsOnHeading("Software update", {
      action: () => within(slotOf("Software update")).getByRole("button", { name: "Check for updates" }),
      pending: () => {
        fireEvent.click(within(slotOf("Software update")).getByRole("button", { name: /Check for updates|Checking…/ }));
      },
      settle: async () => {
        await waitFor(() => expect(mocks.checkForUpdate).toHaveBeenCalledTimes(2));
        // Pending: the failure and the slot's action are still on screen.
        expect(within(slotOf("Software update")).getByText("git fetch failed")).toBeInTheDocument();
        expect(
          within(slotOf("Software update")).getByRole("button", { name: /Check for updates|Checking…/ }),
        ).toBeInTheDocument();
        await act(async () => retry.release(status({})));
      },
      gone: () => expect(screen.queryByText("git fetch failed")).toBeNull(),
    });
  });

  it("an Update & relaunch that fails again keeps focus on the slot's button, and its failure until then", async () => {
    mocks.buildInfo.mockResolvedValue({
      version: "0.1.0",
      commit: "abc1234",
      channel: "dev",
      built_at: "2026-07-20T14:32:05Z",
      dirty: false,
      os_version: "15.7.9",
      self_update_available: true,
      system_backup_device: null,
    });
    mocks.checkForUpdate.mockResolvedValue({
      current_version: "0.1.0",
      current_commit: "abc1234",
      build_channel: "dev",
      build_time: "2026-07-20T14:32:05Z",
      build_dirty: false,
      latest_commit: "def5678",
      commits_behind: 2,
      latest_date: null,
      up_to_date: false,
      checked: true,
      error: null,
    });
    mocks.applyUpdate.mockResolvedValueOnce({ ok: false, output_tail: "error: build failed" });
    renderWithClient(
      <SettingsSection title="Software update" span="half">
        <SoftwareUpdateCard />
      </SettingsSection>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Update & relaunch" }));
    await screen.findByText("error: build failed");
    fireEvent.click(trigger("Software update"));
    const retry = held<{ ok: boolean; output_tail: string }>();
    mocks.applyUpdate.mockReturnValueOnce(retry.promise);
    const update = within(slotOf("Software update")).getByRole("button", { name: "Update & relaunch" });
    update.focus();
    fireEvent.click(update);
    await waitFor(() => expect(mocks.applyUpdate).toHaveBeenCalledTimes(2));
    expect(within(slotOf("Software update")).getByText("error: build failed")).toBeInTheDocument();
    await act(async () => retry.release({ ok: false, output_tail: "error: still failed" }));
    await screen.findByText("error: still failed");
    await waitFor(() =>
      expect(document.activeElement).toBe(
        within(slotOf("Software update")).getByRole("button", { name: "Update & relaunch" }),
      ),
    );
  });
});

describe("General", () => {
  it("a currency save that works from the slot ends on the section's heading", async () => {
    mocks.setBaseCurrency.mockResolvedValueOnce({ status: "error", error: "Persistence failure" });
    renderWithClient(
      <SettingsSection title="General" span="full">
        <GeneralCard />
      </SettingsSection>,
    );
    fireEvent.change(await screen.findByLabelText("Base currency"), { target: { value: "EUR" } });
    await screen.findByText("Persistence failure");
    const retry = held<ReturnType<typeof ok>>();
    mocks.setBaseCurrency.mockReturnValueOnce(retry.promise);
    await expectEndsOnHeading("General", {
      action: () => within(slotOf("General")).getByLabelText("Base currency"),
      pending: () => {
        fireEvent.change(within(slotOf("General")).getByLabelText("Base currency"), { target: { value: "EUR" } });
      },
      settle: async () => {
        await waitFor(() => expect(mocks.setBaseCurrency).toHaveBeenCalledTimes(2));
        expect(within(slotOf("General")).getByText("Persistence failure")).toBeInTheDocument();
        expect(within(slotOf("General")).getByLabelText("Base currency")).toBeInTheDocument();
        await act(async () => retry.release(ok(null)));
      },
      gone: () => expect(screen.queryByText("Persistence failure")).toBeNull(),
    });
  });
});

describe("Vaults", () => {
  it("a Switch that works from the slot ends on the section's heading", async () => {
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
    await screen.findByText("Persistence failure");
    const retry = held<null>();
    mocks.switchVault.mockReturnValueOnce(retry.promise);
    await expectEndsOnHeading("Vaults", {
      action: () => within(slotOf("Vaults")).getByRole("button", { name: "Switch" }),
      pending: () => {
        fireEvent.click(within(slotOf("Vaults")).getByRole("button", { name: "Switch" }));
      },
      settle: async () => {
        await waitFor(() => expect(mocks.switchVault).toHaveBeenCalledTimes(2));
        expect(within(slotOf("Vaults")).getByText("Persistence failure")).toBeInTheDocument();
        expect(within(slotOf("Vaults")).getByRole("button", { name: "Switch" })).toBeInTheDocument();
        await act(async () => retry.release(null));
      },
      gone: () => expect(screen.queryByText("Persistence failure")).toBeNull(),
    });
  });
});

describe("Connections", () => {
  it("a Refresh now that works from the slot ends on the section's heading", async () => {
    const connection = {
      id: "a",
      adapter_id: "simplefin",
      display_hint: "Bank a",
      last_synced_at: "2026-08-22T10:00:00Z",
      last_error: null,
      refresh_cadence: "every_open",
      attention: [],
      links: [],
    };
    const result = (status: string, message: string | null) =>
      ok({ connection_id: "a", status, staged: 0, committed: 0, flagged: 0, skipped_unmapped: 0, warnings: [], message });
    mocks.connectorConnections.mockResolvedValue(ok([connection]));
    mocks.connectorSync.mockResolvedValueOnce(result("failed", "provider said no"));
    renderWithClient(
      <SettingsSection title="Connections" span="full">
        <ConnectionsCard />
      </SettingsSection>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Refresh now" }));
    await screen.findByText("provider said no");
    const retry = held<ReturnType<typeof result>>();
    mocks.connectorSync.mockReturnValueOnce(retry.promise);
    await expectEndsOnHeading("Connections", {
      action: () => within(slotOf("Connections")).getByRole("button", { name: "Refresh now" }),
      pending: () => {
        fireEvent.click(within(slotOf("Connections")).getByRole("button", { name: "Refresh now" }));
      },
      settle: async () => {
        await waitFor(() => expect(mocks.connectorSync).toHaveBeenCalledTimes(2));
        expect(within(slotOf("Connections")).getByText("provider said no")).toBeInTheDocument();
        expect(within(slotOf("Connections")).getByRole("button", { name: "Refresh now" })).toBeInTheDocument();
        await act(async () => retry.release(result("synced", null)));
      },
      gone: () => expect(screen.queryByText("provider said no")).toBeNull(),
    });
  });
});
