import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { RotateKeyCard } from "./RotateKeyCard";

const mockRotate = vi.fn();
vi.mock("@/vault/useVault", async () => {
  const actual =
    await vi.importActual<typeof import("@/vault/useVault")>("@/vault/useVault");
  return {
    useVault: () => ({ rotateVaultKey: mockRotate }),
    describeIpcError: actual.describeIpcError,
  };
});

beforeEach(() => {
  vi.clearAllMocks();
  mockRotate.mockResolvedValue(null);
});

function openForm() {
  fireEvent.click(screen.getByRole("button", { name: /rotate key…/i }));
}

function typePassword(value: string) {
  fireEvent.change(screen.getByLabelText(/current password/i), {
    target: { value },
  });
}

test("rotates with the current password and confirms", async () => {
  render(<RotateKeyCard />);
  openForm();
  // What rotation does not protect is stated before the user commits.
  expect(screen.getByText(/backups made before rotating/i)).toBeInTheDocument();
  expect(
    screen.getByText(/copies of this vault made before rotating/i),
  ).toBeInTheDocument();

  typePassword("my password 1");
  fireEvent.click(screen.getByRole("button", { name: /^rotate key$/i }));

  await waitFor(() => expect(mockRotate).toHaveBeenCalledWith("my password 1"));
  expect(await screen.findByText(/encryption key rotated/i)).toBeInTheDocument();
  // The form closes and the password does not linger.
  expect(screen.queryByLabelText(/current password/i)).not.toBeInTheDocument();
});

test("cannot submit without a password", () => {
  render(<RotateKeyCard />);
  openForm();
  expect(screen.getByRole("button", { name: /^rotate key$/i })).toBeDisabled();
});

test("covers the whole app only while the rotation is running", async () => {
  let finish: (value: null) => void = () => {};
  mockRotate.mockReturnValue(
    new Promise<null>((resolve) => {
      finish = resolve;
    }),
  );
  render(<RotateKeyCard />);
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  openForm();
  typePassword("my password 1");
  fireEvent.click(screen.getByRole("button", { name: /^rotate key$/i }));

  const overlay = await screen.findByRole("alertdialog");
  expect(overlay).toHaveAttribute("aria-modal", "true");
  expect(overlay).toHaveAttribute("aria-busy", "true");
  expect(screen.getByText(/re-encrypting your vault/i)).toBeInTheDocument();

  finish(null);
  await waitFor(() =>
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument(),
  );
});

test("a wrong password is reported and nothing is confirmed", async () => {
  mockRotate.mockResolvedValue("VaultUnlockFailed");
  render(<RotateKeyCard />);
  openForm();
  typePassword("wrong");
  fireEvent.click(screen.getByRole("button", { name: /^rotate key$/i }));

  expect(await screen.findByRole("alert")).toHaveTextContent(
    /incorrect password/i,
  );
  expect(screen.queryByText(/encryption key rotated/i)).not.toBeInTheDocument();
  expect(screen.getByLabelText(/current password/i)).toHaveValue("");
});

test("too little disk space says nothing changed and how to fix it", async () => {
  mockRotate.mockResolvedValue("InsufficientDiskSpace");
  render(<RotateKeyCard />);
  openForm();
  typePassword("my password 1");
  fireEvent.click(screen.getByRole("button", { name: /^rotate key$/i }));

  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent(/enough free disk space/i);
  expect(alert).toHaveTextContent(/nothing was changed/i);
});

test("any other failure is shown", async () => {
  mockRotate.mockResolvedValue({ Persistence: "vault error: rotation failed" });
  render(<RotateKeyCard />);
  openForm();
  typePassword("my password 1");
  fireEvent.click(screen.getByRole("button", { name: /^rotate key$/i }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    /rotation failed/i,
  );
});

test("cancel closes the form and clears the password", () => {
  render(<RotateKeyCard />);
  openForm();
  typePassword("my password 1");
  fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
  expect(screen.queryByLabelText(/current password/i)).not.toBeInTheDocument();
  openForm();
  expect(screen.getByLabelText(/current password/i)).toHaveValue("");
  expect(mockRotate).not.toHaveBeenCalled();
});
