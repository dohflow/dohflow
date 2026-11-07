/**
 * personal-cfo-4d8.39.2 AC4: the vault list read has a failure state. A refused read and a call that
 * never returns are both reported, the last list that did load stays held (but is not claimed to be
 * current), and a later successful read clears the failure.
 */

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";

import { createQueryClient } from "@/lib/queryClient";

import { VaultProvider, useVault } from "./useVault";

const mocks = vi.hoisted(() => ({
  vaultStatus: vi.fn(),
  listVaults: vi.fn(),
  renameVault: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const listing = (names: string[]) =>
  ok({
    vaults: names.map((name, i) => ({
      id: `v${i}`,
      name,
      is_active: i === 0,
      created_at: "2026-07-01T00:00:00Z",
    })),
    restore_recovery_status: "clear",
  });

function Probe() {
  const { vaults, vaultsError, reloadVaults, renameVault } = useVault();
  return (
    <>
      <p data-testid="names">{vaults.map((v) => v.name).join(",")}</p>
      <p data-testid="error">{vaultsError ?? "none"}</p>
      <button type="button" onClick={() => void reloadVaults()}>
        reload
      </button>
      <button type="button" onClick={() => void renameVault("v0", "Renamed")}>
        rename
      </button>
    </>
  );
}
const mount = () =>
  render(
    <QueryClientProvider client={createQueryClient()}>
      <VaultProvider>
        <Probe />
      </VaultProvider>
    </QueryClientProvider>,
  );
const error = () => screen.getByTestId("error").textContent;
const names = () => screen.getByTestId("names").textContent;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.vaultStatus.mockResolvedValue(ok({ state: "Unlocked", account_count: 1 }));
});

test("a clean read has no failure", async () => {
  mocks.listVaults.mockResolvedValue(listing(["Home"]));
  mount();
  await waitFor(() => expect(names()).toBe("Home"));
  expect(error()).toBe("none");
});

test("a refused first read is a failure, with no list", async () => {
  mocks.listVaults.mockResolvedValue({ status: "error", error: "VaultLocked" });
  mount();
  await waitFor(() => expect(error()).not.toBe("none"));
  expect(names()).toBe("");
});

test("a first read that throws is a failure, not an unhandled rejection", async () => {
  mocks.listVaults.mockRejectedValue(new Error("ipc down"));
  mount();
  await waitFor(() => expect(error()).toBe("The vault service is unavailable."));
  expect(names()).toBe("");
});

test.each([
  ["refused", () => mocks.listVaults.mockResolvedValueOnce({ status: "error", error: "Persistence" })],
  ["thrown", () => mocks.listVaults.mockRejectedValueOnce(new Error("ipc down"))],
])("a %s refresh over an earlier list keeps that list and reports the failure; a good read clears it", async (_kind, fail) => {
  mocks.listVaults.mockResolvedValueOnce(listing(["Home", "Test"]));
  mount();
  await waitFor(() => expect(names()).toBe("Home,Test"));
  expect(error()).toBe("none");

  fail();
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "reload" })));
  await waitFor(() => expect(error()).not.toBe("none"));
  expect(names()).toBe("Home,Test");

  mocks.listVaults.mockResolvedValueOnce(listing(["Home"]));
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "reload" })));
  await waitFor(() => expect(error()).toBe("none"));
  expect(names()).toBe("Home");
});
