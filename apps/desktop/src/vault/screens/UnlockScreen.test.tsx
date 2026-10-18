import { fireEvent, render, screen } from "@testing-library/react";

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
  expect(password).toHaveFocus();
  expect(screen.getByRole("button", { name: /^unlock$/i })).toBeDisabled();
});
