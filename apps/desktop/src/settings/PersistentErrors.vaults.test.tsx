/**
 * personal-cfo-4d8.39.2 (slice b, part 3): Vaults puts each error, its full message and its one
 * recovery action in the section's persistent slot. A failed list read recovers with Retry, a failed
 * switch with Switch (beside the vault it is for), a failed rename with Save, a failed create with
 * Create vault. Each is rendered once, in the slot while its error shows; collapsing runs nothing.
 */

import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import type { VaultSummaryDto } from "@/bindings";
import { SettingsSection } from "./SettingsSection";
import { VaultsCard } from "./VaultsCard";

const ops = vi.hoisted(() => ({
  createNamedVault: vi.fn(),
  switchVault: vi.fn(),
  renameVault: vi.fn(),
  reloadVaults: vi.fn(),
}));
let vaults: VaultSummaryDto[] = [];
let vaultsError: string | null = null;
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({ vaults, vaultsError, ...ops }),
  describeIpcError: (error: unknown) => (typeof error === "string" ? error : "Something went wrong."),
}));

beforeEach(() => {
  vi.clearAllMocks();
  ops.createNamedVault.mockResolvedValue(null);
  ops.switchVault.mockResolvedValue(null);
  ops.renameVault.mockResolvedValue(null);
  ops.reloadVaults.mockResolvedValue(undefined);
  vaultsError = null;
  vaults = [
    { id: "real", name: "Real", is_active: true, created_at: "2026-07-01T00:00:00Z" },
    { id: "test", name: "Test", is_active: false, created_at: "2026-07-01T00:00:00Z" },
  ];
});

const renderCard = () =>
  render(
    <SettingsSection title="Vaults" span="full">
      <VaultsCard />
    </SettingsSection>,
  );
const section = () => document.querySelector(`[data-settings-section="Vaults"]`) as HTMLElement;
const slot = () => section().querySelector("[data-settings-persistent]") as HTMLElement;
const everyButton = (name: string | RegExp) =>
  within(section()).queryAllByRole("button", { name, hidden: true });
function expectOnlyInSlot(name: string | RegExp) {
  const found = everyButton(name);
  expect(found).toHaveLength(1);
  expect(slot().contains(found[0]!)).toBe(true);
}
const collapse = () => fireEvent.click(screen.getByRole("button", { name: "Vaults" }));

describe("Vaults", () => {
  it("a failed list read keeps its message and its one Retry while collapsed", async () => {
    vaultsError = "The vault service is unavailable.";
    renderCard();
    const alert = await screen.findByText("The vault service is unavailable.");
    expect(slot().contains(alert)).toBe(true);
    expectOnlyInSlot("Retry");

    collapse();
    expect(screen.getByText("The vault service is unavailable.")).toBeVisible();
    expect(ops.reloadVaults).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(ops.reloadVaults).toHaveBeenCalledTimes(1);
  });

  it("a failed list read with no list at all still shows its error and Retry", () => {
    vaults = [];
    vaultsError = "The vault service is unavailable.";
    renderCard();
    expect(slot()).toHaveTextContent("The vault service is unavailable.");
    expectOnlyInSlot("Retry");
  });

  it("a failed list read over an earlier list keeps the list and shows the error", () => {
    vaultsError = "The vault service is unavailable.";
    renderCard();
    expect(within(section()).getByText("Test")).toBeInTheDocument();
    expectOnlyInSlot("Retry");
  });

  it("a failed Switch keeps its message and its one Switch, beside its vault, while collapsed", async () => {
    ops.switchVault.mockResolvedValueOnce("Persistence failure");
    renderCard();
    fireEvent.click(screen.getByRole("button", { name: "Switch" }));
    const alert = await screen.findByText("Persistence failure");
    expect(slot().contains(alert)).toBe(true);
    expect(slot()).toHaveTextContent("Test");
    expectOnlyInSlot("Switch");

    collapse();
    expect(screen.getByText("Persistence failure")).toBeVisible();
    expect(ops.switchVault).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "Switch" }));
    await waitFor(() => expect(ops.switchVault).toHaveBeenCalledTimes(2));
    expect(ops.switchVault).toHaveBeenLastCalledWith("test");
  });

  it("a failed rename keeps the draft in the body and its one Save in the slot", async () => {
    ops.renameVault.mockResolvedValueOnce("Persistence failure");
    renderCard();
    fireEvent.click(screen.getAllByRole("button", { name: "Rename" })[1]!);
    const input = screen.getByLabelText("New name for Test");
    fireEvent.change(input, { target: { value: "Scratch" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    const alert = await screen.findByText("Persistence failure");
    expect(slot().contains(alert)).toBe(true);
    expectOnlyInSlot("Save");
    expect(screen.getByLabelText("New name for Test")).toHaveValue("Scratch");

    collapse();
    expect(screen.getByText("Persistence failure")).toBeVisible();
    expect(ops.renameVault).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(ops.renameVault).toHaveBeenCalledTimes(2));
    expect(ops.renameVault).toHaveBeenLastCalledWith("test", "Scratch");
  });

  it("cancelling a rename clears its error and the Save in the slot", async () => {
    ops.renameVault.mockResolvedValueOnce("Persistence failure");
    renderCard();
    fireEvent.click(screen.getAllByRole("button", { name: "Rename" })[1]!);
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByText("Persistence failure");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByText("Persistence failure")).not.toBeInTheDocument();
    expect(everyButton("Save")).toHaveLength(0);
  });

  it.each([
    ["no name", "", "", "Name the vault."],
    ["no password", "Scratch", "", "Choose a password."],
  ])("create with %s keeps its message and its one Create vault in the slot", async (_l, name, password, message) => {
    renderCard();
    fireEvent.click(screen.getByRole("button", { name: "New vault…" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: name } });
    fireEvent.change(screen.getByLabelText("Password"), { target: { value: password } });
    fireEvent.click(screen.getByRole("button", { name: "Create vault" }));
    const alert = await screen.findByText(message);
    expect(slot().contains(alert)).toBe(true);
    expectOnlyInSlot("Create vault");
    expect(ops.createNamedVault).not.toHaveBeenCalled();
  });

  it("a refused create keeps its message and its one Create vault, and retrying creates", async () => {
    ops.createNamedVault.mockResolvedValueOnce("VaultExists");
    renderCard();
    fireEvent.click(screen.getByRole("button", { name: "New vault…" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Scratch" } });
    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "pw" } });
    fireEvent.click(screen.getByRole("button", { name: "Create vault" }));
    const alert = await screen.findByText("VaultExists");
    expect(slot().contains(alert)).toBe(true);
    expectOnlyInSlot("Create vault");
    collapse();
    fireEvent.click(screen.getByRole("button", { name: "Create vault" }));
    await waitFor(() => expect(ops.createNamedVault).toHaveBeenCalledTimes(2));
  });

  it("focus stays on Switch when a failed switch moves it into the slot", async () => {
    ops.switchVault.mockResolvedValueOnce("Persistence failure");
    renderCard();
    const button = screen.getByRole("button", { name: "Switch" });
    button.focus();
    fireEvent.click(button);
    await screen.findByText("Persistence failure");
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Switch" })),
    );
    expect(slot().contains(document.activeElement)).toBe(true);
  });

  it("with no error the slot is empty and every action is in the body", () => {
    renderCard();
    expect(slot()).toBeEmptyDOMElement();
    expect(screen.getByRole("button", { name: "Switch" })).toBeInTheDocument();
  });
});
