import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import type { VaultSummaryDto } from "@/bindings";
import { VaultsCard } from "./VaultsCard";

const ops = vi.hoisted(() => ({
  createNamedVault: vi.fn(),
  switchVault: vi.fn(),
  renameVault: vi.fn(),
}));
let vaults: VaultSummaryDto[] = [];
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({ vaults, ...ops }),
  describeIpcError: (e: unknown) => (typeof e === "string" ? e : "error"),
}));

beforeEach(() => {
  vi.clearAllMocks();
  ops.createNamedVault.mockResolvedValue(null);
  ops.switchVault.mockResolvedValue(null);
  ops.renameVault.mockResolvedValue(null);
  vaults = [
    { id: "real", name: "Real", is_active: true, created_at: "2026-07-01T00:00:00Z" },
    { id: "test", name: "Test", is_active: false, created_at: "2026-07-01T00:00:00Z" },
  ];
});

test("renders nothing in single-vault mode", () => {
  vaults = [];
  const { container } = render(<VaultsCard />);
  expect(container).toBeEmptyDOMElement();
});

test("lists vaults, flags the active one, and only the inactive one can be switched to", () => {
  render(<VaultsCard />);
  expect(screen.getByText("Real")).toBeInTheDocument();
  expect(screen.getByText("Test")).toBeInTheDocument();
  expect(screen.getByText(/active/i)).toBeInTheDocument();
  // Exactly one "Switch" — for the inactive Test.
  expect(screen.getAllByRole("button", { name: /^switch$/i })).toHaveLength(1);
});

test("switches to another vault", async () => {
  render(<VaultsCard />);
  fireEvent.click(screen.getByRole("button", { name: /^switch$/i }));
  await waitFor(() => expect(ops.switchVault).toHaveBeenCalledWith("test"));
});

test("renames a vault", async () => {
  render(<VaultsCard />);
  fireEvent.click(screen.getAllByRole("button", { name: /rename/i })[0]!);
  fireEvent.change(screen.getByLabelText(/new name for Real/i), {
    target: { value: "Primary" },
  });
  fireEvent.click(screen.getByRole("button", { name: /^save$/i }));
  await waitFor(() => expect(ops.renameVault).toHaveBeenCalledWith("real", "Primary"));
});

test("creates a new vault with a name and password", async () => {
  render(<VaultsCard />);
  fireEvent.click(screen.getByRole("button", { name: /new vault/i }));
  fireEvent.change(screen.getByLabelText(/^name$/i), { target: { value: "Scratch" } });
  fireEvent.change(screen.getByLabelText(/^password$/i), { target: { value: "hunter2hunter2" } });
  fireEvent.click(screen.getByRole("button", { name: /create vault/i }));
  await waitFor(() =>
    expect(ops.createNamedVault).toHaveBeenCalledWith("Scratch", "hunter2hunter2"),
  );
});

test("blocks create without a name", async () => {
  render(<VaultsCard />);
  fireEvent.click(screen.getByRole("button", { name: /new vault/i }));
  fireEvent.change(screen.getByLabelText(/^password$/i), { target: { value: "hunter2hunter2" } });
  fireEvent.click(screen.getByRole("button", { name: /create vault/i }));
  expect(await screen.findByRole("alert")).toHaveTextContent(/name the vault/i);
  expect(ops.createNamedVault).not.toHaveBeenCalled();
});
