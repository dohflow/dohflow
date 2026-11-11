import { fireEvent, screen, waitFor } from "@testing-library/react";

import { QueryClient } from "@tanstack/react-query";

import { queryKeys } from "@/lib/query";
import { renderWithClient } from "@/test/renderWithClient";

import { SettingsView } from "./SettingsView";

const mocks = vi.hoisted(() => ({
  vault: { vaults: [] as unknown[] },
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
  exportBackup: vi.fn(),
  save: vi.fn(),
  updaterCheck: vi.fn(),
}));

// The release updater plugin (personal-cfo-4d8.39.1: it must not run when the build cannot self-update).
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.updaterCheck }));

vi.mock("@tauri-apps/api/path", () => ({
  homeDir: vi.fn().mockResolvedValue("/Users/test"),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue(null),
  save: mocks.save,
}));

// The About card's links leave the app through the opener plugin (n76x.18).
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));

vi.mock("@/vault/useVault", () => ({
  useVault: () => ({
    deleteVault: vi.fn(),
    vaults: mocks.vault.vaults,
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
    exportBackup: mocks.exportBackup,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

beforeEach(() => {
  mocks.vault.vaults = [];
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
    os_version: "15.7.9",
    self_update_available: true,
    system_backup_device: null,
  });
  mocks.openUrl.mockReset();
  mocks.openUrl.mockResolvedValue(undefined);
  mocks.backupScheduleSettings.mockReset();
  mocks.backupHistory.mockReset();
  mocks.configureBackup.mockReset();
  mocks.runBackupNow.mockReset();
  mocks.exportBackup.mockReset();
  mocks.save.mockReset();
  mocks.updaterCheck.mockReset();
  mocks.updaterCheck.mockResolvedValue(null);
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
  mocks.exportBackup.mockResolvedValue(ok(null));
  mocks.save.mockResolvedValue(null);
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

  it("shows the selected base currency as text under the select on a narrow card (a native select cannot wrap)", async () => {
    renderWithClient(<SettingsView />);
    await waitFor(() => expect(screen.getByLabelText(/base currency/i)).toHaveValue("USD"));
    const copy = screen.getByTestId("base-currency-value");
    expect(copy).toHaveTextContent("US Dollar (USD)");
    expect(copy).toHaveAttribute("aria-hidden", "true");
    expect(copy).toHaveClass("hidden", "compact:@max-[15rem]:block");
    expect(copy.parentElement).toHaveClass("@container");
    fireEvent.change(screen.getByLabelText(/base currency/i), { target: { value: "EUR" } });
    await waitFor(() => expect(screen.getByTestId("base-currency-value")).toHaveTextContent("Euro (EUR)"));
  });

  // personal-cfo-i93n3.28 slice 7: reflow only at the largest text size on a phone (compact-only classes; desktop unchanged).
  it("lets every button in Settings wrap on a phone (one container rule), and the theme control fill its row", async () => {
    const { container } = renderWithClient(<SettingsView />);
    const group = await screen.findByRole("radiogroup", { name: "Appearance" });
    const page = container.querySelector(".\\@container")!;
    for (const rule of ["compact:[&_button]:h-auto", "compact:[&_button]:max-w-full", "compact:[&_button]:whitespace-normal"]) {
      expect(page.className).toContain(rule);
    }
    // Desktop: the page has no unconditional wrap rule, and the theme control is still a fitted inline segmented control.
    expect(page.className).not.toMatch(/(^|\s)\[&_button\]/);
    expect(group).toHaveClass("inline-flex", "w-fit");
    expect(group).toHaveClass("compact:flex", "compact:w-full", "compact:flex-wrap");
    expect(screen.getByRole("radio", { name: "System" })).toHaveClass("compact:flex-1");
    // The comfort band's two edge fields fit as many columns as the text size allows (their cells may shrink).
    const grid = (await screen.findByLabelText(/Upper edge/)).closest("div.grid")!;
    expect(grid.className).toContain("compact:grid-cols-[repeat(auto-fit,minmax(min(100%,9rem),1fr))]");
    expect(grid.className).toContain("grid-cols-2");
  });

  it("renders the Appearance card (personal-cfo-17u1)", async () => {
    renderWithClient(<SettingsView />);
    // The section's wrapper title and the card's own title (4d8.39.1: cards are unchanged inside it).
    expect((await screen.findAllByText("Appearance")).length).toBeGreaterThanOrEqual(2);
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
    // The section's wrapper title and the card's own title (4d8.39.1: cards are unchanged inside it).
    expect((await screen.findAllByText("Connections")).length).toBeGreaterThanOrEqual(2);
    expect(await screen.findByText("No connections yet")).toBeInTheDocument();
  });

  it("renders one Settings backup card with schedule and on-demand backup", async () => {
    renderWithClient(<SettingsView />);
    const card = await screen.findByRole("region", { name: "Backups" });
    expect(card).toBeInTheDocument();
    expect(screen.getByLabelText("Schedule")).toHaveValue("weekly");
    expect(
      screen.getByRole("button", { name: /back up now/i }),
    ).toBeDisabled();
    expect(screen.getByRole("button", { name: /export backup/i })).toBeEnabled();
    expect(screen.queryByRole("button", { name: /restore from backup/i })).not.toBeInTheDocument();
    expect(screen.getByText(/no verified backups yet/i)).toBeInTheDocument();
  });

  it("manually exports from Settings and refreshes the verified backup receipt", async () => {
    const destination = "/Users/test/Manual/copy.pcfobk";
    mocks.save.mockResolvedValue(destination);
    mocks.backupHistory
      .mockResolvedValueOnce(ok([]))
      .mockResolvedValue(ok([{
        backup_id: "manual-1",
        created_at: "2026-09-27T12:00:00Z",
        destination,
        verified: true,
      }]));
    renderWithClient(<SettingsView />);

    const card = await screen.findByRole("region", { name: "Backups" });
    fireEvent.click(screen.getByRole("button", { name: /export backup/i }));

    await waitFor(() => expect(mocks.exportBackup).toHaveBeenCalledWith(destination));
    expect(mocks.save).toHaveBeenCalledWith(expect.objectContaining({
      title: "Save encrypted backup",
      filters: [{ name: "DohFlow backup", extensions: ["pcfobk"] }],
    }));
    expect(await screen.findByText(/backup saved to/i)).toHaveAttribute("role", "status");
    await waitFor(() => expect(mocks.backupHistory).toHaveBeenCalledTimes(2));
    expect(card).toHaveTextContent(`to ${destination} (verified)`);
    expect(screen.getByRole("button", { name: /back up now/i })).toBeDisabled();
  });

  it("renders the About card with the build identity and the Support row (n76x.18)", async () => {
    renderWithClient(<SettingsView />);
    // The section's wrapper title and the card's own title (4d8.39.1: cards are unchanged inside it).
    expect((await screen.findAllByText("About")).length).toBeGreaterThanOrEqual(2);
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

// ── the Settings sections (personal-cfo-4d8.39.1) ────────────────────────────────────────────────

/// How many of the grid's six tracks a section spans at a container width `c`, from the literal
/// classes it ships: `col-span-N`, widened by `@min-[Wpx]:col-span-M` once `c >= W`.
function spanAt(classes: string, c: number): number {
  let span = 6;
  let from = -1;
  for (const token of classes.split(/\s+/)) {
    const base = /^col-span-(\d+)$/.exec(token);
    if (base) {
      span = Number(base[1]);
      from = 0;
      continue;
    }
    const wide = /^@min-\[(\d+)px\]:col-span-(\d+)$/.exec(token);
    if (wide && c >= Number(wide[1]) && Number(wide[1]) > from) {
      span = Number(wide[2]);
      from = Number(wide[1]);
    }
  }
  return span;
}

const sectionNames = () =>
  Array.from(document.querySelectorAll("[data-settings-section]")).map((el) =>
    el.getAttribute("data-settings-section"),
  );
const sectionEl = (name: string) =>
  document.querySelector(`[data-settings-section="${name}"]`) as HTMLElement | null;
const spansAt = (names: string[], c: number) =>
  names.map((name) => spanAt(sectionEl(name)?.className ?? "", c));
/// Names of the sections' triggers in DOM (Tab) order.
const triggerNames = () =>
  Array.from(document.querySelectorAll("[data-settings-section] h3 button")).map(
    (el) => el.textContent,
  );

describe("the Settings sections (personal-cfo-4d8.39.1)", () => {
  const FULL_ORDER = [
    "Household",
    "General",
    "Appearance",
    "Backups",
    "Comfort band",
    "Auto-categorize imported transactions",
    "Connections",
    "Vault health",
    "Software update",
    "Diagnostics",
    "Vaults",
    "Setup guide",
    "Touch ID",
    "Change master password",
    "Rotate encryption key",
    "Delete this vault",
    "About",
  ];

  async function render(over: { setup?: boolean; selfUpdate?: boolean | null } = {}) {
    if (over.selfUpdate === false) {
      mocks.buildInfo.mockResolvedValue({
        ...(await mocks.buildInfo()),
        self_update_available: false,
      });
    }
    if (over.selfUpdate === null) mocks.buildInfo.mockReturnValue(new Promise(() => {}));
    renderWithClient(<SettingsView onRerunSetup={over.setup ? vi.fn() : undefined} />);
    await screen.findByText("Base currency");
  }

  it("lists every section in the approved order, all open, with the wrapper titles", async () => {
    await render({ setup: true });
    await waitFor(() => expect(sectionNames()).toEqual(FULL_ORDER));
    expect(triggerNames()).toEqual(FULL_ORDER);
    for (const trigger of document.querySelectorAll("[data-settings-section] h3 button")) {
      expect(trigger).toHaveAttribute("aria-expanded", "true");
    }
  });

  describe("the Setup guide", () => {
    it("is absent without the callback: no section, no placeholder, and Vaults has the whole row", async () => {
      await render({ setup: false });
      expect(sectionEl("Setup guide")).toBeNull();
      expect(screen.queryByText(/run the setup guide again/i)).toBeNull();
      for (const c of [368, 655, 656, 991, 992, 1176]) {
        expect(spansAt(["Vaults"], c), `Vaults at C=${c}`).toEqual([6]);
      }
    });

    it("is present with the callback: Vaults and Setup are halves from 656, whole rows below", async () => {
      await render({ setup: true });
      expect(spansAt(["Vaults", "Setup guide"], 655)).toEqual([6, 6]);
      expect(spansAt(["Vaults", "Setup guide"], 656)).toEqual([3, 3]);
      expect(spansAt(["Vaults", "Setup guide"], 991)).toEqual([3, 3]);
      expect(spansAt(["Vaults", "Setup guide"], 992)).toEqual([3, 3]); // two equal tracks across the whole width
    });
  });

  describe("the updater", () => {
    it("true: Software update is present; Health is a whole row, then Update and Diagnostics are halves from 656", async () => {
      await render({ selfUpdate: true });
      await waitFor(() => expect(sectionEl("Software update")).not.toBeNull());
      const group = ["Vault health", "Software update", "Diagnostics"];
      expect(spansAt(group, 655)).toEqual([6, 6, 6]);
      expect(spansAt(group, 656)).toEqual([6, 3, 3]);
      expect(spansAt(group, 992)).toEqual([6, 3, 3]);
      expect(triggerNames().slice(triggerNames().indexOf("Vault health"), triggerNames().indexOf("Vaults"))).toEqual(group);
    });

    it("false: there is no Software update section, no updater command runs, and Health then Diagnostics are whole rows at every width", async () => {
      await render({ selfUpdate: false });
      await waitFor(() => expect(sectionEl("Software update")).toBeNull());
      expect(screen.queryByText("Software update")).toBeNull();
      for (const c of [368, 655, 656, 991, 992, 1176]) {
        expect(spansAt(["Vault health", "Diagnostics"], c), `C=${c}`).toEqual([6, 6]);
      }
      const names = triggerNames();
      expect(names[names.indexOf("Vault health") + 1]).toBe("Diagnostics");
      expect(mocks.checkForUpdate).not.toHaveBeenCalled();
      expect(mocks.updaterCheck).not.toHaveBeenCalled();
    });

    it("unknown (the capability has not been read): the section is absent and no updater command runs", async () => {
      await render({ selfUpdate: null });
      expect(sectionEl("Software update")).toBeNull();
      expect(spansAt(["Vault health", "Diagnostics"], 1176)).toEqual([6, 6]);
      expect(mocks.checkForUpdate).not.toHaveBeenCalled();
      expect(mocks.updaterCheck).not.toHaveBeenCalled();
    });
  });

  it("Backups, Comfort band and Auto-categorize: one, two, whole row from 992; every one whole below", async () => {
    await render();
    const group = ["Backups", "Comfort band", "Auto-categorize imported transactions"];
    expect(spansAt(group, 991)).toEqual([6, 6, 6]);
    expect(spansAt(group, 992)).toEqual([2, 4, 6]);
    expect(spansAt(group, 1176)).toEqual([2, 4, 6]);
    expect(spansAt(group, 656)).toEqual([6, 6, 6]);
  });

  it("Household, General, Appearance, Connections, About and the destructive cards are whole rows at every width", async () => {
    await render({ setup: true });
    const wholeRows = [
      "Household", "General", "Appearance", "Connections", "About",
      "Touch ID", "Change master password", "Rotate encryption key", "Delete this vault",
    ];
    for (const c of [368, 655, 656, 991, 992, 1176]) {
      expect(spansAt(wholeRows, c), `C=${c}`).toEqual(wholeRows.map(() => 6));
    }
  });

  it("the page is a container and the grid is six tracks, so spans answer to the pane, not the window", async () => {
    await render();
    const grid = sectionEl("Household")!.parentElement!;
    expect(grid.className).toMatch(/\bgrid-cols-6\b/);
    expect(grid.parentElement!.className).toMatch(/@container/);
  });

  it("an open section keeps its natural height; a collapsed one stretches to its row", async () => {
    await render();
    const grid = sectionEl("Household")!.parentElement!;
    expect(grid.className).toMatch(/\bitems-start\b/);
    expect(sectionEl("About")!.className).toMatch(/\bself-start\b/);
    fireEvent.click(screen.getByRole("button", { name: "About" }));
    expect(sectionEl("About")!.className).toMatch(/\bself-stretch\b/);
    expect(sectionEl("About")!.className).not.toMatch(/\bself-start\b/);
  });

  it("collapsing hides and inerts a section but keeps its draft; expanding brings it back", async () => {
    await render();
    const lower = (await screen.findByLabelText(/lower/i)) as HTMLInputElement;
    fireEvent.change(lower, { target: { value: "1234" } });
    const trigger = screen.getByRole("button", { name: "Comfort band" });
    fireEvent.click(trigger);
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    const body = document.getElementById(trigger.getAttribute("aria-controls")!)!;
    expect(body).toHaveAttribute("hidden");
    expect(body).toHaveAttribute("inert");
    // The card stays mounted, so the draft is still there.
    expect(lower.isConnected).toBe(true);
    fireEvent.click(trigger);
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(lower).toHaveValue("1234");
  });

  it("a section is not remembered: it opens again on the next visit", async () => {
    const first = renderWithClient(<SettingsView />);
    fireEvent.click(await screen.findByRole("button", { name: "About" }));
    first.unmount();
    renderWithClient(<SettingsView />);
    expect(await screen.findByRole("button", { name: "About" })).toHaveAttribute("aria-expanded", "true");
  });

  describe("summaries come from real data only", () => {
    it("shows the approved lines while collapsed, once each read has loaded", async () => {
      mocks.backupHistory.mockResolvedValue(ok([{ backup_id: "b1", vault_id: "v", created_at: "2026-11-01T00:00:00Z", kind: "scheduled", destination: "/b", format_version: 1, size_bytes: 1, verified: true, error: null }]));
      mocks.comfortBand.mockResolvedValue(ok({ currency: "USD", lower: { minor_units: 500_000, currency: "USD" }, upper: { minor_units: 1_500_000, currency: "USD" } }));
      await render();
      for (const name of ["Backups", "Comfort band", "Auto-categorize imported transactions", "About"]) {
        fireEvent.click(await screen.findByRole("button", { name }));
      }
      expect(await screen.findByText("Weekly · last backup verified")).toBeInTheDocument();
      expect(await screen.findByText("$5,000.00–$15,000.00")).toBeInTheDocument();
      expect(await screen.findByText("On")).toBeInTheDocument();
      expect(await screen.findByText("DohFlow · version 0.1.0")).toBeInTheDocument();
    });

    it("shows no line when the data does not say it: no verified backup, no upper edge, setting off, unread build", async () => {
      mocks.autoCategorizeOnImport.mockResolvedValue(ok(false));
      await render();
      for (const name of ["Backups", "Comfort band", "Auto-categorize imported transactions"]) {
        fireEvent.click(await screen.findByRole("button", { name }));
      }
      expect(screen.queryByText(/last backup verified/)).toBeNull();
      expect(screen.queryByText(/–/)).toBeNull();
      expect(screen.queryByText("On")).toBeNull();
    });
  });

  describe("a summary never outlives its read (Studio R2)", () => {
    const verified = ok([{ backup_id: "b1", vault_id: "v", created_at: "2026-11-01T00:00:00Z", kind: "scheduled", destination: "/b", format_version: 1, size_bytes: 1, verified: true, error: null }]);
    const band = ok({ currency: "USD", lower: { minor_units: 500_000, currency: "USD" }, upper: { minor_units: 1_500_000, currency: "USD" } });
    const twoConnections = ok([
      { id: "c1", adapter_id: "lunchflow", display_hint: null, last_synced_at: null, last_error: null, needs_token: false, refresh_cadence: "manual", attention: [], links: [] },
      { id: "c2", adapter_id: "lunchflow", display_hint: null, last_synced_at: null, last_error: null, needs_token: false, refresh_cadence: "manual", attention: [], links: [] },
    ]);

    async function collapsed(names: string[]) {
      const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
      renderWithClient(<SettingsView />, client);
      for (const name of names) {
        fireEvent.click(await screen.findByRole("button", { name }));
      }
      return client;
    }
    const failedRefresh = async (client: QueryClient, queryKey: readonly unknown[]) => {
      await client.refetchQueries({ queryKey });
    };

    it("backups: a failed history refresh over a cached verified backup drops the line, and the next good read brings it back", async () => {
      mocks.backupHistory.mockResolvedValue(verified);
      const client = await collapsed(["Backups"]);
      expect(await screen.findByText("Weekly · last backup verified")).toBeInTheDocument();

      mocks.backupHistory.mockRejectedValue(new Error("history unavailable"));
      await failedRefresh(client, queryKeys.backupHistory);
      await waitFor(() => expect(screen.queryByText(/last backup verified/)).toBeNull());

      mocks.backupHistory.mockResolvedValue(verified);
      await failedRefresh(client, queryKeys.backupHistory);
      expect(await screen.findByText("Weekly · last backup verified")).toBeInTheDocument();
    });

    it("backups: a failed schedule refresh drops the line too", async () => {
      mocks.backupHistory.mockResolvedValue(verified);
      const client = await collapsed(["Backups"]);
      expect(await screen.findByText("Weekly · last backup verified")).toBeInTheDocument();
      mocks.backupScheduleSettings.mockRejectedValue(new Error("settings unavailable"));
      await failedRefresh(client, queryKeys.backupSchedule);
      await waitFor(() => expect(screen.queryByText(/last backup verified/)).toBeNull());
    });

    it("backups: nothing is said while the history is still loading", async () => {
      mocks.backupHistory.mockReturnValue(new Promise(() => {}));
      await collapsed(["Backups"]);
      await screen.findByRole("button", { name: "Backups" });
      expect(screen.queryByText(/last backup verified/)).toBeNull();
    });

    it("comfort band: a failed refresh drops the range", async () => {
      mocks.comfortBand.mockResolvedValue(band);
      const client = await collapsed(["Comfort band"]);
      expect(await screen.findByText("$5,000.00–$15,000.00")).toBeInTheDocument();
      mocks.comfortBand.mockRejectedValue(new Error("band unavailable"));
      await failedRefresh(client, queryKeys.comfortBand);
      await waitFor(() => expect(screen.queryByText("$5,000.00–$15,000.00")).toBeNull());
    });

    it("auto-categorize: a failed refresh drops On", async () => {
      const client = await collapsed(["Auto-categorize imported transactions"]);
      expect(await screen.findByText("On")).toBeInTheDocument();
      mocks.autoCategorizeOnImport.mockRejectedValue(new Error("setting unavailable"));
      await failedRefresh(client, queryKeys.autoCategorizeOnImport);
      await waitFor(() => expect(screen.queryByText("On")).toBeNull());
    });

    it("connections: a failed refresh drops the count", async () => {
      mocks.connectorConnections.mockResolvedValue(twoConnections);
      const client = await collapsed(["Connections"]);
      expect(await screen.findByText("2 connections")).toBeInTheDocument();
      mocks.connectorConnections.mockRejectedValue(new Error("connections unavailable"));
      await failedRefresh(client, queryKeys.connections);
      await waitFor(() => expect(screen.queryByText("2 connections")).toBeNull());
    });

    it("vaults: no summary at all while the vault list has no failure state, whatever it holds (4d8.39.2 AC4 brings it back)", async () => {
      mocks.vault.vaults = [{ id: "v1", name: "Home", is_active: true }];
      await collapsed(["Vaults"]);
      const trigger = await screen.findByRole("button", { name: "Vaults" });
      expect(trigger).toHaveAttribute("aria-expanded", "false");
      expect(screen.queryByText(/ · active/)).toBeNull();
      expect(trigger).not.toHaveAttribute("aria-describedby");
    });
  });
});

