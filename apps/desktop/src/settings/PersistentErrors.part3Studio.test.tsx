/**
 * personal-cfo-4d8.39.2, Studio's review of f47efc8 (part 3): (R1) a delayed Diagnostics preview does
 * not take focus from a control the person moved to; (R2) a long vault name wraps in the Switch slot
 * next to a Switch that does not shrink; (R3) a Refresh now that worked keeps focus ownership
 * through the delayed list re-read, then the heading (collapsed) or Refresh now (open); (R4) Refresh
 * now and Rename have accessible names that say which connection or vault.
 */

import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";

import type { VaultSummaryDto } from "@/bindings";
import { renderWithClient } from "@/test/renderWithClient";

import { ConnectionsCard } from "./ConnectionsCard";
import { DiagnosticsCard } from "./DiagnosticsCard";
import { exampleflow, simplefin } from "./connections/fixtures/adapters";
import { SettingsSection } from "./SettingsSection";
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
  connectorForget: vi.fn(),
  reloadVaults: vi.fn(),
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
let vaultsError: string | null = null;
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({
    vaults,
    vaultsError,
    reloadVaults: mocks.reloadVaults,
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
beforeEach(() => {
  vi.clearAllMocks();
  vaults = [];
  vaultsError = null;
  mocks.reloadVaults.mockResolvedValue(undefined);
  mocks.diagnosticsDiscard.mockResolvedValue(ok(null));
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.setBaseCurrency.mockResolvedValue(ok(null));
  mocks.connectorAdapters.mockResolvedValue([simplefin, exampleflow]);
  mocks.connectorConnections.mockResolvedValue(ok([]));
  mocks.accountList.mockResolvedValue(ok([]));
});




describe("R1: Diagnostics", () => {
  it("a delayed preview does not take focus from a control the person moved to", async () => {
    mocks.diagnosticsPreview.mockResolvedValueOnce({ status: "error", error: "VaultLocked" });
    renderWithClient(
      <>
        <SettingsSection title="Diagnostics" span="full">
          <DiagnosticsCard />
        </SettingsSection>
        <button type="button">Next section</button>
      </>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Preview diagnostics" }));
    await screen.findByText("VaultLocked");
    const retry = held<ReturnType<typeof ok>>();
    mocks.diagnosticsPreview.mockReturnValueOnce(retry.promise);
    const preview = within(slotOf("Diagnostics")).getByRole("button", { name: "Preview diagnostics" });
    preview.focus();
    fireEvent.click(preview);
    await waitFor(() => expect(mocks.diagnosticsPreview).toHaveBeenCalledTimes(2));
    // The person chooses the next section while the preview is still being built.
    const next = screen.getByRole("button", { name: "Next section" });
    act(() => next.focus());
    await act(async () =>
      retry.release(
        ok({ snapshot_id: 8, text: "{}", records: 1, dropped: 0, rejected: 0, suggested_file_name: "d.json" }),
      ),
    );
    await screen.findByLabelText(/exactly what will be saved/i);
    expect(document.activeElement).toBe(next);
  });

  it("an open section still moves focus to the preview when nothing else has focus", async () => {
    mocks.diagnosticsPreview.mockResolvedValue(
      ok({ snapshot_id: 8, text: "{}", records: 1, dropped: 0, rejected: 0, suggested_file_name: "d.json" }),
    );
    renderWithClient(
      <SettingsSection title="Diagnostics" span="full">
        <DiagnosticsCard />
      </SettingsSection>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Preview diagnostics" }));
    const region = await screen.findByLabelText(/exactly what will be saved/i);
    await waitFor(() => expect(document.activeElement).toBe(region));
  });
});

describe("R2: Vaults slot name", () => {
  it("wraps the full name beside a Switch that does not shrink, in the slot only", async () => {
    const long = "Household finances for the whole extended family, archive copy, 2019 through 2024 (suffix-XYZ)";
    vaults = [
      { id: "real", name: "Real", is_active: true, created_at: "2026-07-01T00:00:00Z" },
      { id: "test", name: long, is_active: false, created_at: "2026-07-01T00:00:00Z" },
    ];
    mocks.switchVault.mockResolvedValueOnce("Persistence failure");
    renderWithClient(
      <SettingsSection title="Vaults" span="full">
        <VaultsCard />
      </SettingsSection>,
    );
    const bodyName = within(section("Vaults")).getAllByText(long)[0]!;
    expect(bodyName).toHaveClass("truncate"); // the list row is unchanged
    fireEvent.click(screen.getByRole("button", { name: "Switch" }));
    await screen.findByText("Persistence failure");
    const slotName = within(slotOf("Vaults")).getByText(long);
    expect(slotName).toHaveClass("min-w-0", "break-words");
    expect(slotName).not.toHaveClass("truncate");
    expect(within(slotOf("Vaults")).getByRole("button", { name: "Switch" })).toHaveClass("shrink-0");
    // Still one Switch in the section.
    expect(within(section("Vaults")).getAllByRole("button", { name: "Switch", hidden: true })).toHaveLength(1);
  });
});

describe("R3: Connections", () => {
  const connection = (over: Record<string, unknown> = {}) => ({
    id: "a", adapter_id: "simplefin", display_hint: "Bank a", last_synced_at: "2026-08-22T10:00:00Z",
    last_error: "access revoked", refresh_cadence: "every_open", attention: [], links: [], ...over,
  });
  const synced = ok({
    connection_id: "a", status: "synced", staged: 0, committed: 0, flagged: 0, skipped_unmapped: 0, warnings: [], message: null,
  });
  async function succeedWithHeldReread(collapsed: boolean) {
    mocks.connectorConnections.mockResolvedValueOnce(ok([connection()]));
    renderWithClient(
      <SettingsSection title="Connections" span="full">
        <ConnectionsCard />
      </SettingsSection>,
    );
    await screen.findByText("Last refresh failed: access revoked");
    if (collapsed) fireEvent.click(trigger("Connections"));
    mocks.connectorSync.mockResolvedValueOnce(synced);
    const reread = held<ReturnType<typeof ok>>();
    mocks.connectorConnections.mockReturnValue(reread.promise);
    const refresh = within(slotOf("Connections")).getByRole("button", { name: /^Refresh now/ });
    refresh.focus();
    fireEvent.click(refresh);
    await waitFor(() => expect(mocks.connectorSync).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mocks.connectorConnections.mock.calls.length).toBeGreaterThanOrEqual(2));
    // The sync worked, but the list that still carries the error has not been re-read: the entry and
    // its Refresh now are still in the slot.
    expect(within(slotOf("Connections")).getByText("Last refresh failed: access revoked")).toBeInTheDocument();
    await act(async () => reread.release(ok([connection({ last_error: null })])));
    await waitFor(() => expect(screen.queryByText("Last refresh failed: access revoked")).toBeNull());
  }

  it("collapsed: ownership lasts through the delayed list re-read, then the heading", async () => {
    await succeedWithHeldReread(true);
    await waitFor(() => expect(document.activeElement).toBe(trigger("Connections")));
    expect(trigger("Connections")).toHaveAttribute("aria-expanded", "false");
  });

  it("open: ownership lasts through the delayed list re-read, then Refresh now in the body", async () => {
    await succeedWithHeldReread(false);
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: /^Refresh now/ })),
    );
    expect(slotOf("Connections").contains(document.activeElement)).toBe(false);
  });
});

describe("R4: accessible names", () => {
  it("Refresh now says which connection, in the body and in the slot, and every name is unique", async () => {
    const make = (id: string, name: string, over: Record<string, unknown> = {}) => ({
      id, adapter_id: "simplefin", display_hint: name, last_synced_at: "2026-08-22T10:00:00Z",
      last_error: null, refresh_cadence: "every_open", attention: [], links: [], ...over,
    });
    mocks.connectorConnections.mockResolvedValue(
      ok([make("a", "Bank a", { last_error: "boom" }), make("b", "Bank a2"), make("c", "Bank a")]),
    );
    renderWithClient(
      <SettingsSection title="Connections" span="full">
        <ConnectionsCard />
      </SettingsSection>,
    );
    await screen.findByText("Last refresh failed: boom");
    const buttons = within(section("Connections")).getAllByRole("button", { name: /^Refresh now/, hidden: true });
    const names = buttons.map((b) => b.getAttribute("aria-label"));
    expect(names).toEqual(expect.arrayContaining(["Refresh now, Bank a", "Refresh now, Bank a2"]));
    // The slot's instance carries the same template; two same-named connections are told apart by
    // the app, not by this name (the contract has no further qualifier).
    expect(within(slotOf("Connections")).getByRole("button", { name: "Refresh now, Bank a" })).toBeInTheDocument();
    expect(within(section("Connections")).getByRole("button", { name: "Refresh now, Bank a2" })).toBeInTheDocument();
    // The visible label is unchanged.
    for (const button of buttons) expect(button).toHaveTextContent("Refresh now");
  });

  it("Rename says which vault, with the visible label unchanged, and the names are unique", () => {
    vaults = [
      { id: "real", name: "Real", is_active: true, created_at: "2026-07-01T00:00:00Z" },
      { id: "t1", name: "Test", is_active: false, created_at: "2026-07-01T00:00:00Z" },
      { id: "t2", name: "Test 2", is_active: false, created_at: "2026-07-01T00:00:00Z" },
    ];
    renderWithClient(
      <SettingsSection title="Vaults" span="full">
        <VaultsCard />
      </SettingsSection>,
    );
    const renames = screen.getAllByRole("button", { name: /^Rename / });
    const names = renames.map((b) => b.getAttribute("aria-label"));
    expect(names).toEqual(["Rename Real", "Rename Test", "Rename Test 2"]);
    expect(new Set(names).size).toBe(names.length);
    for (const button of renames) expect(button).toHaveTextContent(/^Rename$/);
  });
});
