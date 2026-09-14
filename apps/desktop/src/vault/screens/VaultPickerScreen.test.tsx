import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import type { VaultSummaryDto } from "@/bindings";
import { sparklinePoints, VaultPickerScreen } from "./VaultPickerScreen";

const ops = vi.hoisted(() => ({
  unlockVault: vi.fn(),
  switchVault: vi.fn(),
  createNamedVault: vi.fn(),
}));
let vaults: VaultSummaryDto[] = [];
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({ vaults, ...ops }),
  describeIpcError: (e: unknown) => (typeof e === "string" ? e : "error"),
}));
// The no-reset warning + restore flow have their own tests; stub them so the picker
// test needs no react-query client or Tauri dialog plugin.
vi.mock("./NoResetWarning", () => ({
  NoResetWarning: ({
    acknowledged,
    onAcknowledgedChange,
  }: {
    acknowledged: boolean;
    onAcknowledgedChange: (v: boolean) => void;
  }) => (
    <label>
      <input
        type="checkbox"
        checked={acknowledged}
        onChange={(e) => onAcknowledgedChange(e.target.checked)}
      />
      I understand
    </label>
  ),
}));
vi.mock("@/backup/RestoreFromBackup", () => ({
  RestoreFromBackup: () => <div data-testid="restore" />,
}));

beforeEach(() => {
  vi.clearAllMocks();
  ops.unlockVault.mockResolvedValue(null);
  ops.switchVault.mockResolvedValue(null);
  ops.createNamedVault.mockResolvedValue(null);
  vaults = [
    { id: "real", name: "Real", is_active: true, created_at: "2026-05-01T00:00:00Z" },
    { id: "demo", name: "Polish Demo", is_active: false, created_at: "2026-07-03T00:00:00Z" },
  ];
});

test("lists every vault with the active one marked as last opened", () => {
  render(<VaultPickerScreen />);
  expect(screen.getByText("Real")).toBeInTheDocument();
  expect(screen.getByText("Polish Demo")).toBeInTheDocument();
  expect(screen.getByText(/last opened/i)).toBeInTheDocument();
  expect(screen.getByText(/2 vaults on this device/i)).toBeInTheDocument();
});

test("clicking the active vault opens its unlock modal without switching", async () => {
  render(<VaultPickerScreen />);
  fireEvent.click(screen.getByRole("button", { name: /real/i }));
  expect(
    await screen.findByRole("dialog", { name: /unlock real/i }),
  ).toBeInTheDocument();
  expect(ops.switchVault).not.toHaveBeenCalled();
});

test("clicking another vault switches to it first, then asks for ITS password", async () => {
  render(<VaultPickerScreen />);
  fireEvent.click(screen.getByRole("button", { name: /polish demo/i }));
  await waitFor(() => expect(ops.switchVault).toHaveBeenCalledWith("demo"));
  expect(
    await screen.findByRole("dialog", { name: /unlock polish demo/i }),
  ).toBeInTheDocument();
});

test("unlocking submits the typed password", async () => {
  render(<VaultPickerScreen />);
  fireEvent.click(screen.getByRole("button", { name: /real/i }));
  fireEvent.change(await screen.findByLabelText(/master password/i), {
    target: { value: "hunter2hunter2" },
  });
  fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
  await waitFor(() =>
    expect(ops.unlockVault).toHaveBeenCalledWith("hunter2hunter2"),
  );
});

test("a wrong password surfaces the error and stays on the modal", async () => {
  ops.unlockVault.mockResolvedValue("VaultUnlockFailed");
  render(<VaultPickerScreen />);
  fireEvent.click(screen.getByRole("button", { name: /real/i }));
  fireEvent.change(await screen.findByLabelText(/master password/i), {
    target: { value: "wrong" },
  });
  fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
  expect(await screen.findByRole("alert")).toBeInTheDocument();
  expect(screen.getByRole("dialog", { name: /unlock real/i })).toBeInTheDocument();
});

test("forgot-password explains there is no reset (backup restore instead)", async () => {
  render(<VaultPickerScreen />);
  fireEvent.click(screen.getByRole("button", { name: /real/i }));
  fireEvent.click(await screen.findByRole("button", { name: /forgot master password/i }));
  expect(screen.getByText(/no password reset/i)).toBeInTheDocument();
});

test("search narrows the list (shown once there are enough vaults)", () => {
  vaults = [
    ...vaults,
    { id: "a", name: "Archive", is_active: false, created_at: "2026-01-01T00:00:00Z" },
    { id: "b", name: "Business", is_active: false, created_at: "2026-01-01T00:00:00Z" },
  ];
  render(<VaultPickerScreen />);
  fireEvent.change(screen.getByLabelText(/search vaults/i), {
    target: { value: "demo" },
  });
  expect(screen.getByText("Polish Demo")).toBeInTheDocument();
  expect(screen.queryByText("Business")).not.toBeInTheDocument();
});

test("creating a vault requires name, matching password, and the acknowledgement", async () => {
  render(<VaultPickerScreen />);
  fireEvent.click(screen.getByRole("button", { name: /new vault/i }));
  const dialog = await screen.findByRole("dialog", { name: /create a new vault/i });
  expect(dialog).toBeInTheDocument();

  fireEvent.change(screen.getByLabelText(/^name$/i), { target: { value: "Scratch" } });
  fireEvent.change(screen.getByLabelText(/^master password$/i), {
    target: { value: "hunter2hunter2" },
  });
  fireEvent.change(screen.getByLabelText(/confirm password/i), {
    target: { value: "hunter2hunter2" },
  });
  const submit = screen.getByRole("button", { name: /create vault/i });
  expect(submit).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox"));
  expect(submit).toBeEnabled();
  fireEvent.click(submit);
  await waitFor(() =>
    expect(ops.createNamedVault).toHaveBeenCalledWith("Scratch", "hunter2hunter2"),
  );
});

test("sparklines are deterministic per vault id and differ across vaults", () => {
  expect(sparklinePoints("vault-a")).toEqual(sparklinePoints("vault-a"));
  expect(sparklinePoints("vault-a").points).not.toEqual(
    sparklinePoints("vault-b").points,
  );
});
