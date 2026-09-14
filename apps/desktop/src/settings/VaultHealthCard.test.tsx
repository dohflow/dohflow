import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { VaultHealthDto } from "@/bindings";
import { VaultHealthCard } from "./VaultHealthCard";

const mocks = vi.hoisted(() => ({
  vaultHealth: vi.fn(),
  rebuildReadModels: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    vaultHealth: mocks.vaultHealth,
    rebuildReadModels: mocks.rebuildReadModels,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function health(over: Partial<VaultHealthDto> = {}): VaultHealthDto {
  return {
    writer_healthy: true,
    wal_configured: true,
    schema_coherent: true,
    integrity_ok: true,
    attachments_consistent: true,
    read_models_current: true,
    is_healthy: true,
    ...over,
  };
}

beforeEach(() => {
  mocks.vaultHealth.mockReset();
  mocks.rebuildReadModels.mockReset();
  mocks.rebuildReadModels.mockResolvedValue(ok(3));
});

describe("VaultHealthCard", () => {
  it("shows a healthy verdict and the checks", async () => {
    mocks.vaultHealth.mockResolvedValue(ok(health()));
    renderWithClient(<VaultHealthCard />);

    expect(await screen.findByText(/your vault is healthy/i)).toBeInTheDocument();
    expect(screen.getByText("Database integrity")).toBeInTheDocument();
    // No drift → no rebuild action.
    expect(
      screen.queryByRole("button", { name: /rebuild data views/i }),
    ).not.toBeInTheDocument();
  });

  it("offers the read-model rebuild repair on drift", async () => {
    // Drift is informational (is_healthy stays true) but still repairable.
    mocks.vaultHealth.mockResolvedValue(ok(health({ read_models_current: false })));
    renderWithClient(<VaultHealthCard />);

    fireEvent.click(
      await screen.findByRole("button", { name: /rebuild data views/i }),
    );
    await waitFor(() => expect(mocks.rebuildReadModels).toHaveBeenCalledTimes(1));
    expect(await screen.findByText(/rebuilt/i)).toBeInTheDocument();
  });

  it("steers integrity corruption to a backup restore", async () => {
    mocks.vaultHealth.mockResolvedValue(
      ok(health({ integrity_ok: false, is_healthy: false })),
    );
    renderWithClient(<VaultHealthCard />);

    expect(
      await screen.findByText(/your vault needs attention/i),
    ).toBeInTheDocument();
    expect(screen.getByText(/may be damaged.*restore/i)).toBeInTheDocument();
    // Corruption is not rebuildable, so no rebuild action is offered.
    expect(
      screen.queryByRole("button", { name: /rebuild data views/i }),
    ).not.toBeInTheDocument();
  });

  it("explains what each check means in plain language", async () => {
    mocks.vaultHealth.mockResolvedValue(ok(health()));
    renderWithClient(<VaultHealthCard />);
    expect(
      await screen.findByText(/layout matches this version of the app/i),
    ).toBeInTheDocument();
  });

  it("gives how-to-fix guidance for a failing check (schema version)", async () => {
    mocks.vaultHealth.mockResolvedValue(
      ok(health({ schema_coherent: false, is_healthy: false })),
    );
    renderWithClient(<VaultHealthCard />);
    expect(await screen.findByText(/reopen the vault/i)).toBeInTheDocument();
    expect(
      screen.getByText(/Schema version:.*back up your vault/i),
    ).toBeInTheDocument();
  });

  it("surfaces a check error", async () => {
    mocks.vaultHealth.mockRejectedValue(new Error("locked"));
    renderWithClient(<VaultHealthCard />);
    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
  });
});
