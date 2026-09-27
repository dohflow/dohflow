import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { RecoveryScreen } from "./RecoveryScreen";

const ops = vi.hoisted(() => ({ refresh: vi.fn() }));
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({ refresh: ops.refresh, restoreRecoveryStatus: "interrupted" }),
}));
vi.mock("@/backup/RestoreFromBackup", () => ({
  RestoreFromBackup: ({ mode }: { mode?: string }) => <div data-testid="restore" data-mode={mode} />,
}));

beforeEach(() => {
  ops.refresh.mockReset();
  ops.refresh.mockResolvedValue(undefined);
});

test("damaged-vault recovery offers a separate named restore and retains the original", async () => {
  render(<RecoveryScreen />);
  expect(screen.getByText(/keep the original files for diagnosis/i)).toBeInTheDocument();
  expect(screen.getByTestId("restore")).toHaveAttribute("data-mode", "newNamed");
  expect(screen.getByRole("alert")).toHaveTextContent(/unregistered vault folder/i);

  fireEvent.click(screen.getByRole("button", { name: /re-check vault/i }));
  await waitFor(() => expect(ops.refresh).toHaveBeenCalledTimes(1));
});
