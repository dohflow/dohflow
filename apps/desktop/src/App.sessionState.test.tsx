import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { useSessionState } from "@/lib/sessionViewState";
import App from "./App";

// personal-cfo-i93n3.27 slice D: the unlocked session's view settings are cleared on every lock path. The
// real App routes by vault state, so this drives the real router: a view that holds state through the
// session store, a host-driven (auto) lock, and an unlock again.
const mocks = vi.hoisted(() => ({
  vaultStatus: vi.fn(),
  unlockVault: vi.fn(),
  lockVault: vi.fn(),
  listVaults: vi.fn(),
  accountList: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    vaultStatus: mocks.vaultStatus,
    unlockVault: mocks.unlockVault,
    lockVault: mocks.lockVault,
    listVaults: mocks.listVaults,
    accountList: mocks.accountList,
    moneyInboxList: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
    noResetWarning: vi.fn().mockResolvedValue("No password reset."),
    acknowledgeNoResetWarning: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    baseCurrency: vi.fn().mockResolvedValue({ status: "ok", data: "USD" }),
    checkForUpdate: vi.fn().mockResolvedValue({ up_to_date: true, checked: true, error: null }),
  },
}));
vi.mock("@/dashboard/DashboardView", () => ({ DashboardView: () => <h2>Dashboard</h2> }));
vi.mock("@/future-cash/FutureCashView", () => ({
  FutureCashView: () => {
    const [horizon, setHorizon] = useSessionState<number>("futureCash.horizon", 90);
    return (
      <div>
        <h2>Cash Flow</h2>
        <button type="button" onClick={() => setHorizon(365)}>
          horizon:{horizon}
        </button>
      </div>
    );
  },
}));
vi.mock("@/settings/BuildBadge", () => ({ BuildBadge: () => null }));
vi.mock("@/settings/UpdateAvailableNotice", () => ({ UpdateAvailableNotice: () => null }));
vi.mock("@/forecast-activation/CapabilityUnlockNotice", () => ({ CapabilityUnlockNotice: () => null }));
vi.mock("@/settings/connections/ConnectionAttentionNotice", () => ({ ConnectionAttentionNotice: () => null }));
vi.mock("@/backup/BackupNudge", () => ({ BackupNudge: () => null }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const unlocked = ok({ state: "Unlocked", account_count: 1 });
const locked = ok({ state: "Locked", account_count: null });

function setVisibility(state: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", { configurable: true, value: state });
  act(() => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.listVaults.mockResolvedValue(ok({ vaults: [] }));
  mocks.accountList.mockResolvedValue(
    ok([{ id: "acc-1", name: "Checking", cashflow_role: "liquid_cash", subtype: null, active: true, balance: { minor_units: 1, currency: "USD" } }]),
  );
  mocks.unlockVault.mockResolvedValue(ok({ state: "Unlocked", account_count: 1 }));
  setVisibility("visible");
});

it("clears the view settings when the host locks the vault on its own (auto-lock), and starts empty on the next unlock", async () => {
  mocks.vaultStatus.mockResolvedValue(unlocked);
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /^Cash Flow$/ }));
  const horizon = await screen.findByRole("button", { name: /^horizon:/ });
  expect(horizon).toHaveTextContent("horizon:90");
  fireEvent.click(horizon);
  expect(screen.getByRole("button", { name: /^horizon:/ })).toHaveTextContent("horizon:365");

  // A visit to Dashboard and back keeps it (the shell is still mounted).
  fireEvent.click(screen.getByRole("button", { name: /^Dashboard$/ }));
  fireEvent.click(await screen.findByRole("button", { name: /^Cash Flow$/ }));
  expect(await screen.findByRole("button", { name: /^horizon:/ })).toHaveTextContent("horizon:365");

  // The host locks the vault (backgrounding or a device lock): the page re-reads the status when it returns.
  mocks.vaultStatus.mockResolvedValue(locked);
  setVisibility("hidden");
  setVisibility("visible");
  expect(await screen.findByRole("heading", { name: /welcome back/i })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /^horizon:/ })).toBeNull();

  // Unlock again: a fresh shell, and the view starts from its default.
  mocks.vaultStatus.mockResolvedValue(unlocked);
  fireEvent.change(screen.getByLabelText(/master password/i), { target: { value: "synthetic-password" } });
  fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
  fireEvent.click(await screen.findByRole("button", { name: /^Cash Flow$/ }));
  await waitFor(() => expect(screen.getByRole("button", { name: /^horizon:/ })).toHaveTextContent("horizon:90"));
});

it("clears the view settings on an explicit lock", async () => {
  mocks.vaultStatus.mockResolvedValue(unlocked);
  mocks.lockVault.mockResolvedValue(locked);
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /^Cash Flow$/ }));
  fireEvent.click(await screen.findByRole("button", { name: /^horizon:/ }));
  mocks.vaultStatus.mockResolvedValue(locked);
  fireEvent.click(screen.getByRole("button", { name: /^Lock/ }));
  expect(await screen.findByRole("heading", { name: /welcome back/i })).toBeInTheDocument();
  mocks.vaultStatus.mockResolvedValue(unlocked);
  fireEvent.change(screen.getByLabelText(/master password/i), { target: { value: "synthetic-password" } });
  fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
  fireEvent.click(await screen.findByRole("button", { name: /^Cash Flow$/ }));
  await waitFor(() => expect(screen.getByRole("button", { name: /^horizon:/ })).toHaveTextContent("horizon:90"));
});

it("clears the view settings through a transient state too (a password change or a key rotation re-locks via one, and a vault switch comes up locked)", async () => {
  mocks.vaultStatus.mockResolvedValue(unlocked);
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /^Cash Flow$/ }));
  fireEvent.click(await screen.findByRole("button", { name: /^horizon:/ }));
  expect(screen.getByRole("button", { name: /^horizon:/ })).toHaveTextContent("horizon:365");

  // Any state other than Unlocked unmounts the shell, whatever the cause: the router renders a busy screen.
  mocks.vaultStatus.mockResolvedValue(ok({ state: "Locking", account_count: null }));
  setVisibility("hidden");
  setVisibility("visible");
  expect(await screen.findByText("Working…")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /^horizon:/ })).toBeNull();
  // The shell, and the store inside it, are gone; a later unlock mounts a new one (see the tests above).
  expect(screen.queryByRole("button", { name: /^Cash Flow$/ })).toBeNull();
});
