import { fireEvent, render, screen } from "@testing-library/react";

import App from "./App";

// Drive the UI through mocked vault commands so we can assert that the lifecycle
// state routes to the right screen and that the create/unlock flows behave.
const mocks = vi.hoisted(() => ({
  vaultStatus: vi.fn(),
  createVault: vi.fn(),
  unlockVault: vi.fn(),
  lockVault: vi.fn(),
  accountList: vi.fn(),
  incomeSourceList: vi.fn(),
  recurringBillList: vi.fn(),
  futureCashForecast: vi.fn(),
  listVaults: vi.fn(),
  noResetWarning: vi.fn(),
  acknowledgeNoResetWarning: vi.fn(),
  baseCurrency: vi.fn(),
  setBaseCurrency: vi.fn(),
  checkForUpdate: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    vaultStatus: mocks.vaultStatus,
    createVault: mocks.createVault,
    unlockVault: mocks.unlockVault,
    lockVault: mocks.lockVault,
    moneyInboxList: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
    accountList: mocks.accountList,
    incomeSourceList: mocks.incomeSourceList,
    recurringBillList: mocks.recurringBillList,
    futureCashForecast: mocks.futureCashForecast,
    // VaultProvider loads the vault registry on mount (j0cg.6); an empty list
    // keeps these tests in single-vault mode without unhandled rejections.
    listVaults: mocks.listVaults,
    noResetWarning: mocks.noResetWarning,
    acknowledgeNoResetWarning: mocks.acknowledgeNoResetWarning,
    baseCurrency: mocks.baseCurrency,
    setBaseCurrency: mocks.setBaseCurrency,
    checkForUpdate: mocks.checkForUpdate,
  },
}));

// A minimal liquid account so the unlocked shell renders without the first-run
// wizard taking over (the wizard auto-shows only on an empty vault).
const oneAccount = {
  id: "acc-1",
  name: "Checking",
  cashflow_role: "liquid_cash",
  subtype: null,
  active: true,
  balance: { minor_units: 50_000, currency: "USD" },
} as const;

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const fail = (error: unknown) => ({ status: "error", error }) as const;

beforeEach(() => {
  mocks.vaultStatus.mockReset();
  mocks.createVault.mockReset();
  mocks.unlockVault.mockReset();
  mocks.lockVault.mockReset();
  mocks.accountList.mockReset();
  mocks.futureCashForecast.mockReset();
  mocks.noResetWarning.mockReset();
  mocks.acknowledgeNoResetWarning.mockReset();
  mocks.baseCurrency.mockReset();
  mocks.setBaseCurrency.mockReset();
  mocks.accountList.mockResolvedValue(ok([]));
  mocks.incomeSourceList.mockReset();
  mocks.recurringBillList.mockReset();
  mocks.incomeSourceList.mockResolvedValue(ok([]));
  mocks.recurringBillList.mockResolvedValue(ok([]));
  mocks.listVaults.mockReset();
  mocks.listVaults.mockResolvedValue(ok({ vaults: [] }));
  mocks.noResetWarning.mockResolvedValue("No password reset.");
  mocks.acknowledgeNoResetWarning.mockResolvedValue(ok(null));
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.setBaseCurrency.mockResolvedValue(ok(null));
  // The unlocked shell mounts the launch update-notice, which checks for updates.
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
  // The unlocked shell defaults to the Dashboard tab, which loads the forecast.
  mocks.futureCashForecast.mockResolvedValue(
    ok({
      currency: "USD",
      starting_balance: { minor_units: 0, currency: "USD" },
      start_date: "2026-06-20",
      horizon_days: 30,
      days: [],
    }),
  );
});

describe("vault routing", () => {
  it("shows the create screen when there is no vault", async () => {
    mocks.vaultStatus.mockResolvedValue(ok({ state: "NoVault", account_count: null }));
    render(<App />);
    expect(
      await screen.findByRole("heading", { name: /create your vault/i }),
    ).toBeInTheDocument();
  });

  it("shows the unlock screen when the vault is locked", async () => {
    mocks.vaultStatus.mockResolvedValue(ok({ state: "Locked", account_count: null }));
    render(<App />);
    expect(
      await screen.findByRole("heading", { name: /welcome back/i }),
    ).toBeInTheDocument();
  });

  it("shows the unlocked app shell on the dashboard", async () => {
    mocks.vaultStatus.mockResolvedValue(ok({ state: "Unlocked", account_count: 1 }));
    // A non-empty vault skips the first-run wizard and lands on the shell.
    mocks.accountList.mockResolvedValue(ok([oneAccount]));
    render(<App />);
    // The shell exposes the Lock control and defaults to the Dashboard section.
    // The Dashboard heading renders only after the forecast query resolves, so
    // wait for it (findBy) rather than asserting synchronously.
    expect(
      await screen.findByRole("button", { name: /lock/i }),
    ).toBeInTheDocument();
    expect(
      await screen.findByRole("heading", { name: /dashboard/i }),
    ).toBeInTheDocument();
  });

  it("shows the recovery screen for a half-open vault", async () => {
    mocks.vaultStatus.mockResolvedValue(
      ok({ state: "CorruptNeedsRecovery", account_count: null }),
    );
    render(<App />);
    expect(
      await screen.findByRole("heading", { name: /needs attention/i }),
    ).toBeInTheDocument();
  });
});

describe("theme toggle reachability (personal-cfo-17u1 review, Finding 1)", () => {
  // The control that changes appearance must be reachable regardless of vault
  // state — it lived only inside Settings before, which does not exist on the
  // lock screen at all. Proven here against the REAL <App/> tree (no theme
  // internals mocked), on both a locked screen and the unlocked shell.
  it("is present on the lock screen, before any vault is unlocked", async () => {
    mocks.vaultStatus.mockResolvedValue(ok({ state: "Locked", account_count: null }));
    render(<App />);
    await screen.findByRole("heading", { name: /welcome back/i });
    expect(
      screen.getByRole("button", { name: /^Appearance:/ }),
    ).toBeInTheDocument();
  });

  it("is present on the unlocked shell too — the SAME control, not a second one", async () => {
    mocks.vaultStatus.mockResolvedValue(ok({ state: "Unlocked", account_count: 1 }));
    mocks.accountList.mockResolvedValue(ok([oneAccount]));
    render(<App />);
    await screen.findByRole("heading", { name: /dashboard/i });
    expect(
      screen.getAllByRole("button", { name: /^Appearance:/ }),
    ).toHaveLength(1);
  });
});

describe("vault flows", () => {
  it("creates a vault and lands on the first-run wizard", async () => {
    mocks.vaultStatus.mockResolvedValue(ok({ state: "NoVault", account_count: null }));
    mocks.createVault.mockResolvedValue(ok({ state: "Unlocked", account_count: 0 }));
    render(<App />);

    await screen.findByRole("heading", { name: /create your vault/i });
    fireEvent.change(screen.getByLabelText(/master password/i), {
      target: { value: "supersecret" },
    });
    fireEvent.change(screen.getByLabelText(/confirm password/i), {
      target: { value: "supersecret" },
    });
    // The no-reset warning must be acknowledged before creation is allowed.
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: /create vault/i }));

    // A brand-new (empty) vault lands on the First Forecast Wizard, not the shell.
    expect(
      await screen.findByText(/build your first cash forecast/i),
    ).toBeInTheDocument();
    expect(mocks.createVault).toHaveBeenCalledWith("supersecret");
    expect(mocks.acknowledgeNoResetWarning).toHaveBeenCalled();
    // Onboarding seeds the base currency (defaults to USD) on the new vault.
    expect(mocks.setBaseCurrency).toHaveBeenCalledWith("USD");
  });

  it("surfaces a wrong-password unlock as an inline error", async () => {
    mocks.vaultStatus.mockResolvedValue(ok({ state: "Locked", account_count: null }));
    mocks.unlockVault.mockResolvedValue(fail("VaultUnlockFailed"));
    render(<App />);

    await screen.findByRole("heading", { name: /welcome back/i });
    fireEvent.change(screen.getByLabelText(/master password/i), {
      target: { value: "wrong" },
    });
    fireEvent.click(screen.getByRole("button", { name: /unlock/i }));

    expect(await screen.findByText(/incorrect password/i)).toBeInTheDocument();
  });
});
