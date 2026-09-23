import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { SettingsView } from "./SettingsView";

const mocks = vi.hoisted(() => ({
  connectorConnections: vi.fn(),
  accountList: vi.fn(),
  baseCurrency: vi.fn(),
  setBaseCurrency: vi.fn(),
  householdTimezone: vi.fn(),
  setHouseholdTimezone: vi.fn(),
  cashAvailability: vi.fn(),
  setMinimumCashFloor: vi.fn(),
  comfortBand: vi.fn(),
  setComfortBandUpper: vi.fn(),
  autoCategorizeOnImport: vi.fn(),
  setAutoCategorizeOnImport: vi.fn(),
  vaultHealth: vi.fn(),
  rebuildReadModels: vi.fn(),
  checkForUpdate: vi.fn(),
  changePassword: vi.fn(),
  buildInfo: vi.fn(),
  openUrl: vi.fn(),
  backupScheduleSettings: vi.fn(),
  backupHistory: vi.fn(),
  configureBackup: vi.fn(),
  runBackupNow: vi.fn(),
}));

vi.mock("@tauri-apps/api/path", () => ({
  homeDir: vi.fn().mockResolvedValue("/Users/test"),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue(null),
}));

// The About card's links leave the app through the opener plugin (n76x.18).
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));

vi.mock("@/vault/useVault", () => ({
  useVault: () => ({
    deleteVault: vi.fn(),
    vaults: [],
    createNamedVault: vi.fn(),
    switchVault: vi.fn(),
    renameVault: vi.fn(),
  }),
  describeIpcError: (e: unknown) => (typeof e === "string" ? e : "Something went wrong."),
}));

vi.mock("@/bindings", () => ({
  commands: {
    baseCurrency: mocks.baseCurrency,
    setBaseCurrency: mocks.setBaseCurrency,
    householdTimezone: mocks.householdTimezone,
    setHouseholdTimezone: mocks.setHouseholdTimezone,
    cashAvailability: mocks.cashAvailability,
    setMinimumCashFloor: mocks.setMinimumCashFloor,
    comfortBand: mocks.comfortBand,
    setComfortBandUpper: mocks.setComfortBandUpper,
    autoCategorizeOnImport: mocks.autoCategorizeOnImport,
    setAutoCategorizeOnImport: mocks.setAutoCategorizeOnImport,
    vaultHealth: mocks.vaultHealth,
    rebuildReadModels: mocks.rebuildReadModels,
    checkForUpdate: mocks.checkForUpdate,
    changePassword: mocks.changePassword,
    connectorConnections: mocks.connectorConnections,
    accountList: mocks.accountList,
    buildInfo: mocks.buildInfo,
    backupScheduleSettings: mocks.backupScheduleSettings,
    backupHistory: mocks.backupHistory,
    configureBackup: mocks.configureBackup,
    runBackupNow: mocks.runBackupNow,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

beforeEach(() => {
  // SettingsView now also renders ConnectionsCard (personal-cfo-ul5d).
  mocks.connectorConnections.mockReset();
  mocks.connectorConnections.mockResolvedValue(ok([]));
  mocks.accountList.mockReset();
  mocks.accountList.mockResolvedValue(ok([]));
  mocks.baseCurrency.mockReset();
  mocks.setBaseCurrency.mockReset();
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.setBaseCurrency.mockResolvedValue(ok(null));
  // SettingsView now also renders HouseholdCard (personal-cfo-q329).
  mocks.householdTimezone.mockReset();
  mocks.setHouseholdTimezone.mockReset();
  mocks.householdTimezone.mockResolvedValue(ok("UTC"));
  mocks.setHouseholdTimezone.mockResolvedValue(ok(null));
  // SettingsView now also renders SoftwareUpdateCard (personal-cfo-1ik.3).
  mocks.checkForUpdate.mockReset();
  mocks.checkForUpdate.mockResolvedValue({
    current_version: "0.1.0",
    current_commit: "abc1234",
    latest_commit: "abc1234",
    commits_behind: 0,
    latest_date: null,
    up_to_date: true,
    checked: true,
    error: null,
  });
  // SettingsView now also renders the self-contained ComfortBandCard.
  mocks.cashAvailability.mockReset();
  mocks.setMinimumCashFloor.mockReset();
  mocks.comfortBand.mockReset();
  mocks.setComfortBandUpper.mockReset();
  mocks.cashAvailability.mockResolvedValue(
    ok({
      currency: "USD",
      accounts: [],
      net_available: { minor_units: 0, currency: "USD" },
      net_committed: { minor_units: 0, currency: "USD" },
      net_headroom: { minor_units: 0, currency: "USD" },
      floor: { minor_units: 0, currency: "USD" },
      below_floor: false,
    }),
  );
  mocks.setMinimumCashFloor.mockResolvedValue(ok(null));
  mocks.comfortBand.mockResolvedValue(
    ok({
      currency: "USD",
      lower: { minor_units: 0, currency: "USD" },
      upper: null,
    }),
  );
  mocks.setComfortBandUpper.mockResolvedValue(ok(null));
  // SettingsView also renders the self-contained AutoCategorizeOnImportCard.
  mocks.autoCategorizeOnImport.mockReset();
  mocks.setAutoCategorizeOnImport.mockReset();
  mocks.autoCategorizeOnImport.mockResolvedValue(ok(true));
  mocks.setAutoCategorizeOnImport.mockResolvedValue(ok(null));
  // SettingsView also renders the self-contained VaultHealthCard.
  mocks.vaultHealth.mockReset();
  mocks.rebuildReadModels.mockReset();
  mocks.vaultHealth.mockResolvedValue(
    ok({
      writer_healthy: true,
      wal_configured: true,
      schema_coherent: true,
      integrity_ok: true,
      attachments_consistent: true,
      read_models_current: true,
      is_healthy: true,
    }),
  );
  mocks.rebuildReadModels.mockResolvedValue(ok(0));
  // SettingsView also renders the AboutCard (n76x.18), which reads build_info.
  mocks.buildInfo.mockReset();
  mocks.buildInfo.mockResolvedValue({
    version: "0.1.0",
    commit: "abc1234",
    channel: "release",
    built_at: "2026-07-20T14:32:05Z",
    dirty: false,
  });
  mocks.openUrl.mockReset();
  mocks.openUrl.mockResolvedValue(undefined);
  mocks.backupScheduleSettings.mockReset();
  mocks.backupHistory.mockReset();
  mocks.configureBackup.mockReset();
  mocks.runBackupNow.mockReset();
  mocks.backupScheduleSettings.mockResolvedValue(
    ok({
      cadence: "weekly",
      destination: null,
      next_due_at: null,
      last_run_at: null,
      last_error: null,
    }),
  );
  mocks.backupHistory.mockResolvedValue(ok([]));
  mocks.configureBackup.mockResolvedValue(ok({ cadence: "weekly" }));
  mocks.runBackupNow.mockResolvedValue(ok({ destination: "/backup.pcfobk" }));
});

describe("SettingsView", () => {
  it("shows the current base currency", async () => {
    mocks.baseCurrency.mockResolvedValue(ok("EUR"));
    renderWithClient(<SettingsView />);
    await waitFor(() =>
      expect(screen.getByLabelText(/base currency/i)).toHaveValue("EUR"),
    );
  });

  it("saves a new base-currency selection", async () => {
    renderWithClient(<SettingsView />);
    await waitFor(() =>
      expect(screen.getByLabelText(/base currency/i)).toHaveValue("USD"),
    );

    fireEvent.change(screen.getByLabelText(/base currency/i), {
      target: { value: "EUR" },
    });

    await waitFor(() =>
      expect(mocks.setBaseCurrency).toHaveBeenCalledWith("EUR"),
    );
    expect(await screen.findByText(/saved/i)).toBeInTheDocument();
  });

  it("renders the Appearance card (personal-cfo-17u1)", async () => {
    renderWithClient(<SettingsView />);
    expect(await screen.findByText("Appearance")).toBeInTheDocument();
    expect(
      screen.getByRole("radiogroup", { name: "Appearance" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "System" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  it("renders the Connections card", async () => {
    renderWithClient(<SettingsView />);
    expect(await screen.findByText("Connections")).toBeInTheDocument();
    expect(await screen.findByText("No connections yet")).toBeInTheDocument();
  });

  it("renders one Settings backup card with schedule and on-demand backup", async () => {
    renderWithClient(<SettingsView />);
    expect(await screen.findByText("Backups")).toBeInTheDocument();
    expect(screen.getByLabelText("Schedule")).toHaveValue("weekly");
    expect(
      screen.getByRole("button", { name: /back up now/i }),
    ).toBeDisabled();
    expect(screen.getByText(/no verified backups yet/i)).toBeInTheDocument();
  });

  it("renders the About card with the build identity and the Support row (n76x.18)", async () => {
    renderWithClient(<SettingsView />);
    expect(await screen.findByText("About")).toBeInTheDocument();
    expect(
      await screen.findByText("Version 0.1.0 · Release build · commit abc1234"),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Support DohFlow" }));
    await waitFor(() =>
      expect(mocks.openUrl).toHaveBeenCalledWith("https://dohflow.app/sponsor"),
    );
  });

  it("offers to run the setup guide again when the host provides it (kdw6)", async () => {
    const rerun = vi.fn();
    renderWithClient(<SettingsView onRerunSetup={rerun} />);
    fireEvent.click(
      await screen.findByRole("button", { name: /run the setup guide again/i }),
    );
    expect(rerun).toHaveBeenCalledTimes(1);
  });
});
