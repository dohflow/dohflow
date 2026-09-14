import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { ExportCsvButton } from "./ExportCsvButton";

const mocks = vi.hoisted(() => ({
  save: vi.fn(),
  exportTransactionsCsv: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: mocks.save }));
vi.mock("@/bindings", () => ({
  commands: { exportTransactionsCsv: mocks.exportTransactionsCsv },
}));
vi.mock("@/vault/useVault", () => ({
  describeIpcError: (e: unknown) => (typeof e === "string" ? e : "error"),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

test("exports to the chosen path and reports the unencrypted row count", async () => {
  mocks.save.mockResolvedValue("/tmp/transactions.csv");
  mocks.exportTransactionsCsv.mockResolvedValue({ status: "ok", data: 42 });
  render(<ExportCsvButton />);
  fireEvent.click(screen.getByRole("button", { name: /export csv/i }));
  await waitFor(() =>
    expect(mocks.exportTransactionsCsv).toHaveBeenCalledWith("/tmp/transactions.csv"),
  );
  expect(await screen.findByRole("status")).toHaveTextContent(/42 transactions.*unencrypted/i);
  // The dialog itself warns about plaintext.
  expect(mocks.save.mock.calls[0]![0].title).toMatch(/unencrypted/i);
});

test("cancelling the dialog exports nothing", async () => {
  mocks.save.mockResolvedValue(null);
  render(<ExportCsvButton />);
  fireEvent.click(screen.getByRole("button", { name: /export csv/i }));
  await waitFor(() => expect(mocks.save).toHaveBeenCalled());
  expect(mocks.exportTransactionsCsv).not.toHaveBeenCalled();
});

test("surfaces an export failure inline", async () => {
  mocks.save.mockResolvedValue("/tmp/x.csv");
  mocks.exportTransactionsCsv.mockResolvedValue({ status: "error", error: "VaultLocked" });
  render(<ExportCsvButton />);
  fireEvent.click(screen.getByRole("button", { name: /export csv/i }));
  expect(await screen.findByRole("status")).toBeInTheDocument();
});
