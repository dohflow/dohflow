import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { DeleteVaultCard } from "./DeleteVaultCard";

const mockDelete = vi.fn();
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({ deleteVault: mockDelete }),
  describeIpcError: (e: unknown) => (typeof e === "string" ? e : "Something went wrong."),
}));

beforeEach(() => {
  vi.clearAllMocks();
  mockDelete.mockResolvedValue(null);
});

function openConfirm() {
  fireEvent.click(screen.getByRole("button", { name: /delete vault…/i }));
}

test("deletes only after the exact confirmation phrase is typed", async () => {
  render(<DeleteVaultCard />);
  openConfirm();

  const confirmButton = screen.getByRole("button", { name: /delete permanently/i });
  expect(confirmButton).toBeDisabled(); // nothing typed

  fireEvent.change(screen.getByLabelText(/type/i), { target: { value: "delete" } });
  expect(confirmButton).toBeDisabled(); // wrong phrase

  fireEvent.change(screen.getByLabelText(/type/i), { target: { value: "delete my data" } });
  expect(confirmButton).toBeEnabled();

  fireEvent.click(confirmButton);
  await waitFor(() => expect(mockDelete).toHaveBeenCalledTimes(1));
});

test("is case-insensitive on the phrase and trims whitespace", async () => {
  render(<DeleteVaultCard />);
  openConfirm();
  fireEvent.change(screen.getByLabelText(/type/i), {
    target: { value: "  DELETE MY DATA  " },
  });
  expect(screen.getByRole("button", { name: /delete permanently/i })).toBeEnabled();
});

test("surfaces a failure and does not leave the screen", async () => {
  mockDelete.mockResolvedValue("WriterPanicked");
  render(<DeleteVaultCard />);
  openConfirm();
  fireEvent.change(screen.getByLabelText(/type/i), { target: { value: "delete my data" } });
  fireEvent.click(screen.getByRole("button", { name: /delete permanently/i }));

  expect(await screen.findByRole("alert")).toHaveTextContent(/WriterPanicked/);
});

test("does not delete when cancelled", () => {
  render(<DeleteVaultCard />);
  openConfirm();
  fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
  expect(screen.queryByRole("button", { name: /delete permanently/i })).toBeNull();
  expect(mockDelete).not.toHaveBeenCalled();
});
