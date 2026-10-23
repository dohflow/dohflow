import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import type { VaultSummaryDto } from "@/bindings";
import { sparklinePoints, VaultPickerScreen } from "./VaultPickerScreen";

const ops = vi.hoisted(() => ({
  unlockVault: vi.fn(),
  switchVault: vi.fn(),
  createNamedVault: vi.fn(),
}));
let vaults: VaultSummaryDto[] = [];
let status: { state: "Locked" | "NoVault" } = { state: "Locked" };
vi.mock("@/vault/useVault", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/vault/useVault")>()),
  useVault: () => ({ status, vaults, ...ops }),
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
  RestoreFromBackup: ({ mode }: { mode?: string }) => <div data-testid="restore" data-mode={mode} />,
}));

beforeEach(() => {
  vi.clearAllMocks();
  ops.unlockVault.mockResolvedValue(null);
  ops.switchVault.mockResolvedValue(null);
  ops.createNamedVault.mockResolvedValue(null);
  status = { state: "Locked" };
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
  expect(screen.getByTestId("restore")).toHaveAttribute("data-mode", "newNamed");
});

test("does not offer occupied-vault restore when no vault is selected", () => {
  status = { state: "NoVault" };
  render(<VaultPickerScreen />);
  expect(screen.queryByTestId("restore")).not.toBeInTheDocument();
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

test("newer schema refusal resets busy state, focuses retry, and permits back without force-open", async () => {
  let refuse!: (value: "NewerVaultSchema") => void;
  ops.unlockVault.mockResolvedValue("NewerVaultSchema");
  ops.unlockVault.mockReturnValueOnce(new Promise((resolve) => { refuse = resolve; }));
  render(<VaultPickerScreen />);
  fireEvent.click(screen.getByRole("button", { name: /real/i }));
  const password = await screen.findByLabelText(/master password/i);
  fireEvent.change(password, { target: { value: "synthetic-password" } });
  fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
  expect(screen.getByRole("button", { name: /unlocking/i })).toBeDisabled();
  expect(screen.getByRole("button", { name: /back to vaults/i })).toBeDisabled();
  fireEvent.submit(password.closest("form")!);
  expect(ops.unlockVault).toHaveBeenCalledTimes(1);
  refuse("NewerVaultSchema");
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "This vault requires a newer version of DohFlow. Open it with a compatible newer version.",
  );
  expect(password).toHaveValue("");
  // Focus returns in an effect after the refusal renders: poll for it (personal-cfo-v9xls).
  await waitFor(() => expect(password).toHaveFocus());
  expect(screen.getByRole("dialog", { name: /unlock real/i })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /force|downgrade|repair/i })).not.toBeInTheDocument();
  fireEvent.change(password, { target: { value: "retry-password" } });
  fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
  await waitFor(() => expect(ops.unlockVault).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(screen.getByRole("button", { name: /back to vaults/i })).toBeEnabled());
  fireEvent.click(screen.getByRole("button", { name: /back to vaults/i }));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(status.state).toBe("Locked");
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

// personal-cfo-i93n3.16: a host that asks for an integrity check, and a vault that fails it.
async function submitWith(failure: string, password = "synthetic-password") {
  ops.unlockVault.mockResolvedValue(failure);
  fireEvent.click(screen.getByRole("button", { name: /real/i }));
  const field = await screen.findByLabelText(/master password/i);
  fireEvent.change(field, { target: { value: password } });
  fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
  return field;
}

test("a failed integrity check shows the message, opens the restore flow in the sheet and moves focus to the message", async () => {
  render(<VaultPickerScreen />);
  const field = await submitWith("VaultIntegrityCheckFailed");
  const dialog = await screen.findByRole("dialog", { name: /unlock real/i });
  const message = await within(dialog).findByRole("alert");
  expect(message).toHaveTextContent("found damage");
  expect(message).toHaveTextContent("Nothing was opened or changed");
  expect(message).toHaveTextContent("Restore it from a backup you made earlier");
  // The existing restore-as-new flow, inside the sheet (not only on the screen behind it).
  const restore = within(dialog).getByTestId("restore");
  expect(restore).toHaveAttribute("data-mode", "newNamed");
  // Focus moves to the message (announced, and the restore action is the next stop), not back to
  // the password field the user cannot use to fix this.
  await waitFor(() => expect(message).toHaveFocus());
  expect(field).not.toHaveFocus();
  // Generic wording: no platform named.
  expect(dialog.textContent ?? "").not.toMatch(/\b(iphone|ios|android)\b/i);
});

test("a wrong password offers no restore action and returns focus to the password field", async () => {
  render(<VaultPickerScreen />);
  const field = await submitWith("VaultUnlockFailed", "wrong");
  const dialog = await screen.findByRole("dialog", { name: /unlock real/i });
  expect(await within(dialog).findByRole("alert")).toHaveTextContent("Incorrect password");
  expect(within(dialog).queryByTestId("restore")).not.toBeInTheDocument();
  await waitFor(() => expect(field).toHaveFocus());
});

test("the newer-schema refusal is unchanged: no restore action in the sheet", async () => {
  render(<VaultPickerScreen />);
  await submitWith("NewerVaultSchema");
  const dialog = await screen.findByRole("dialog", { name: /unlock real/i });
  await within(dialog).findByRole("alert");
  expect(within(dialog).queryByTestId("restore")).not.toBeInTheDocument();
});

test("the restore action goes away on the next attempt when it fails for another reason", async () => {
  render(<VaultPickerScreen />);
  const field = await submitWith("VaultIntegrityCheckFailed");
  const dialog = await screen.findByRole("dialog", { name: /unlock real/i });
  await waitFor(() => expect(within(dialog).getByTestId("restore")).toBeInTheDocument());
  ops.unlockVault.mockResolvedValue("VaultUnlockFailed");
  fireEvent.change(field, { target: { value: "another" } });
  fireEvent.click(within(dialog).getByRole("button", { name: /^unlock$/i }));
  expect(await within(dialog).findByText(/incorrect password/i)).toBeInTheDocument();
  expect(within(dialog).queryByTestId("restore")).not.toBeInTheDocument();
});
