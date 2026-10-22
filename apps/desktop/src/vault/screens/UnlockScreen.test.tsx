import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { UnlockScreen } from "./UnlockScreen";

const unlockVault = vi.hoisted(() => vi.fn());
vi.mock("@/vault/useVault", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/vault/useVault")>()),
  useVault: () => ({ unlockVault }),
}));

test("newer schema refusal stays on unlock with actionable copy, cleared password and focus", async () => {
  unlockVault.mockResolvedValue("NewerVaultSchema");
  render(<UnlockScreen />);
  const password = screen.getByLabelText(/master password/i);
  fireEvent.change(password, { target: { value: "synthetic-password" } });
  fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "This vault requires a newer version of DohFlow. Open it with a compatible newer version.",
  );
  expect(password).toHaveValue("");
  // Focus returns in an effect after the refusal renders: poll for it (personal-cfo-v9xls).
  await waitFor(() => expect(password).toHaveFocus());
  expect(screen.getByRole("button", { name: /^unlock$/i })).toBeDisabled();
});

// personal-cfo-i93n3.13: a host that asks for an integrity check, and a vault that fails it.
vi.mock("@/backup/RestoreFromBackup", () => ({
  RestoreFromBackup: ({ mode }: { mode?: string }) => (
    <button type="button" data-mode={mode}>
      Restore as a new vault…
    </button>
  ),
}));

test("a failed integrity check says so plainly, writes nothing and offers a restore from a backup", async () => {
  unlockVault.mockResolvedValue("VaultIntegrityCheckFailed");
  render(<UnlockScreen />);
  fireEvent.change(screen.getByLabelText(/master password/i), {
    target: { value: "synthetic-password" },
  });
  fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("found damage");
  expect(alert).toHaveTextContent("Nothing was opened or changed");
  expect(alert).toHaveTextContent("Restore it from a backup you made earlier");
  // The existing restore-as-new flow is the action: it makes a SEPARATE vault.
  const restore = screen.getByRole("button", { name: /restore as a new vault/i });
  expect(restore).toHaveAttribute("data-mode", "newNamed");
  // Generic wording: no platform or device is named.
  expect(document.body.textContent ?? "").not.toMatch(/\b(iphone|ios|android)\b/i);
});

test("an ordinary wrong password offers no restore action", async () => {
  unlockVault.mockResolvedValue("VaultUnlockFailed");
  render(<UnlockScreen />);
  fireEvent.change(screen.getByLabelText(/master password/i), { target: { value: "nope" } });
  fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Incorrect password");
  expect(screen.queryByRole("button", { name: /restore as a new vault/i })).not.toBeInTheDocument();
});
