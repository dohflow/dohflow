// ConnectionsCard (personal-cfo-ul5d): four states, the link flow, account
// mapping, sync outcomes (incl. rate-limited-is-healthy copy), and the forget
// confirm — all against mocked bindings, per FRONTEND.md §8.

import { fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderWithClient } from "@/test/renderWithClient";

const mocks = vi.hoisted(() => ({
  connectorConnections: vi.fn(),
  connectorLink: vi.fn(),
  connectorSetAccountLink: vi.fn(),
  connectorSync: vi.fn(),
  connectorForget: vi.fn(),
  accountList: vi.fn(),
  createAccount: vi.fn(),
  baseCurrency: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: { ...mocks },
}));

vi.mock("@/vault/useVault", () => ({
  describeIpcError: (error: unknown) =>
    typeof error === "string" ? error : "Something went wrong.",
}));

import { ConnectionsCard } from "./ConnectionsCard";

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function connection(over: Record<string, unknown> = {}) {
  return {
    id: "11111111-1111-7111-8111-111111111111",
    adapter_id: "simplefin",
    display_hint: "SimpleFIN Bridge connection",
    last_synced_at: "2026-08-22T10:00:00Z",
    last_error: null,
    links: [
      {
        external_id: "ACT-1",
        external_name: "Demo Checking",
        account_id: null,
        last_synced_on: null,
      },
    ],
    ...over,
  };
}

const account = {
  id: "22222222-2222-7222-8222-222222222222",
  name: "Checking",
  active: true,
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.connectorConnections.mockResolvedValue(ok([]));
  mocks.accountList.mockResolvedValue(ok([account]));
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
});

describe("ConnectionsCard", () => {
  it("shows a skeleton while connections load", () => {
    mocks.connectorConnections.mockReturnValue(new Promise(() => {}));
    const { container } = renderWithClient(<ConnectionsCard />);
    expect(
      container.querySelector('[data-slot="skeleton"], .animate-pulse'),
    ).toBeTruthy();
  });

  it("shows the empty state when nothing is linked", async () => {
    const { findByText } = renderWithClient(<ConnectionsCard />);
    expect(await findByText("No connections yet")).toBeInTheDocument();
  });

  it("shows a load error as an alert", async () => {
    mocks.connectorConnections.mockRejectedValue(new Error("locked"));
    const { findByRole } = renderWithClient(<ConnectionsCard />);
    expect(await findByRole("alert")).toHaveTextContent(
      "Could not load connections.",
    );
  });

  it("renders a healthy connection with its sync stamp and links", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection()]));
    const { findByText, getByText } = renderWithClient(<ConnectionsCard />);
    expect(await findByText("SimpleFIN Bridge connection")).toBeInTheDocument();
    expect(getByText(/^Synced /)).toBeInTheDocument();
    expect(getByText("Demo Checking")).toBeInTheDocument();
    expect(
      getByText(
        "Not mapped — transactions from this account are not imported",
      ),
    ).toBeInTheDocument();
  });

  it("surfaces a failed connection with the error and a warning badge", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([connection({ last_error: "access revoked — re-link required" })]),
    );
    const { findByText, getByRole } = renderWithClient(<ConnectionsCard />);
    expect(await findByText("Needs attention")).toBeInTheDocument();
    expect(getByRole("alert")).toHaveTextContent(
      "Last sync failed: access revoked — re-link required",
    );
  });

  it("links a connection from a pasted setup token", async () => {
    mocks.connectorLink.mockResolvedValue(
      ok({
        connection_id: "33333333-3333-7333-8333-333333333333",
        display_hint: "SimpleFIN Bridge connection",
        accounts: [
          { external_id: "ACT-1", external_name: "Demo Checking" },
          { external_id: "ACT-2", external_name: "Demo Card" },
        ],
        fetch_error: null,
      }),
    );
    const { findByText, getByLabelText, getByText } = renderWithClient(
      <ConnectionsCard />,
    );
    fireEvent.click(await findByText("Link a connection…"));
    fireEvent.change(getByLabelText("Setup token"), {
      target: { value: "  base64token  " },
    });
    fireEvent.click(getByText("Connect"));
    await waitFor(() =>
      expect(mocks.connectorLink).toHaveBeenCalledWith({
        adapter_id: "simplefin",
        setup_token: "base64token",
      }),
    );
    expect(
      await findByText("Connected — 2 account(s) discovered. Map them below."),
    ).toBeInTheDocument();
  });

  it("maps an external account onto a real account", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection()]));
    mocks.connectorSetAccountLink.mockResolvedValue(ok(null));
    const { findByLabelText } = renderWithClient(<ConnectionsCard />);
    const select = await findByLabelText("Account for Demo Checking");
    fireEvent.change(select, { target: { value: account.id } });
    await waitFor(() =>
      expect(mocks.connectorSetAccountLink).toHaveBeenCalledWith({
        connection_id: connection().id,
        external_id: "ACT-1",
        account_id: account.id,
      }),
    );
  });

  it("reads a rate-limited sync as healthy pacing, not an error", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection()]));
    mocks.connectorSync.mockResolvedValue(
      ok({
        connection_id: connection().id,
        status: "rate_limited",
        staged: 0,
        committed: 0,
        flagged: 0,
        skipped_unmapped: 0,
        warnings: [],
        message: "provider throttled the request — retry later",
      }),
    );
    const { findByText, findByRole } = renderWithClient(<ConnectionsCard />);
    fireEvent.click(await findByText("Sync now"));
    const status = await findByRole("status");
    expect(status).toHaveTextContent(
      "The provider is pacing requests — this connection will sync later.",
    );
  });

  it("reads an unmapped-discovery outcome as progress, not an error", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection()]));
    mocks.connectorSync.mockResolvedValue(
      ok({
        connection_id: connection().id,
        status: "discovered_accounts",
        staged: 0,
        committed: 0,
        flagged: 0,
        skipped_unmapped: 0,
        warnings: [],
        message: "Found 2 account(s) — map them in Settings, then sync.",
      }),
    );
    const { findByText, findByRole } = renderWithClient(<ConnectionsCard />);
    fireEvent.click(await findByText("Sync now"));
    const status = await findByRole("status");
    expect(status).toHaveTextContent(
      "Found 2 account(s) — map them in Settings, then sync.",
    );
  });

  it("reads an empty-provider discovery as healthy waiting, not an error", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection()]));
    mocks.connectorSync.mockResolvedValue(
      ok({
        connection_id: connection().id,
        status: "no_mapped_accounts",
        staged: 0,
        committed: 0,
        flagged: 0,
        skipped_unmapped: 0,
        warnings: [],
        message:
          "The provider reported no accounts yet — new connections can take a while to appear at the Bridge. Try again soon.",
      }),
    );
    const { findByText, findByRole } = renderWithClient(<ConnectionsCard />);
    fireEvent.click(await findByText("Sync now"));
    const status = await findByRole("status");
    expect(status).toHaveTextContent(/reported no accounts yet/);
  });

  it("routes a failed sync outcome to the error channel with fallback copy", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection()]));
    mocks.connectorSync.mockResolvedValue(
      ok({
        connection_id: connection().id,
        status: "failed",
        staged: 0,
        committed: 0,
        flagged: 0,
        skipped_unmapped: 0,
        warnings: [],
        message: null,
      }),
    );
    const { findByText, findByRole } = renderWithClient(<ConnectionsCard />);
    fireEvent.click(await findByText("Sync now"));
    const alert = await findByRole("alert");
    expect(alert).toHaveTextContent("The sync did not complete.");
  });

  it("forgets a connection only after the inline confirm", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection()]));
    mocks.connectorForget.mockResolvedValue(ok(null));
    const { findByText, getByText, queryByText } = renderWithClient(
      <ConnectionsCard />,
    );
    fireEvent.click(await findByText("Forget connection…"));
    expect(
      getByText(/Transactions already synced stay in the ledger/),
    ).toBeInTheDocument();
    expect(mocks.connectorForget).not.toHaveBeenCalled();
    fireEvent.click(getByText("Forget"));
    await waitFor(() =>
      expect(mocks.connectorForget).toHaveBeenCalledWith({
        connection_id: connection().id,
      }),
    );
    expect(queryByText("delete my data")).not.toBeInTheDocument();
  });

  it("creates a new account from the mapping step and maps it (07bn)", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection()]));
    mocks.createAccount.mockResolvedValue(
      ok({ account_id: "44444444-4444-7444-8444-444444444444" }),
    );
    mocks.connectorSetAccountLink.mockResolvedValue(ok(null));
    const { findByLabelText, findByRole, getByRole, getByLabelText } =
      renderWithClient(<ConnectionsCard />);
    const select = await findByLabelText("Account for Demo Checking");
    fireEvent.change(select, { target: { value: "__create_new__" } });

    // The dialog opens prefilled with the provider's account name.
    const dialog = await findByRole("dialog", {
      name: /new account for this connection/i,
    });
    expect(dialog).toBeInTheDocument();
    // The sentinel never reaches the backend, and the controlled select
    // snaps back to the stored (unmapped) state.
    expect(mocks.connectorSetAccountLink).not.toHaveBeenCalled();
    expect((select as HTMLSelectElement).value).toBe("");
    expect((getByLabelText("Account name") as HTMLInputElement).value).toBe(
      "Demo Checking",
    );
    fireEvent.click(getByRole("button", { name: /create and map/i }));

    await waitFor(() =>
      expect(mocks.createAccount).toHaveBeenCalledWith(
        expect.objectContaining({
          name: "Demo Checking",
          cashflow_role: "LiquidCash",
          subtype: null,
          flags: null,
          currency: "USD",
          opening_balance: null,
        }),
      ),
    );
    // …and the new account is mapped onto the external one in the same flow.
    await waitFor(() =>
      expect(mocks.connectorSetAccountLink).toHaveBeenCalledWith({
        connection_id: connection().id,
        external_id: "ACT-1",
        account_id: "44444444-4444-7444-8444-444444444444",
      }),
    );
  });

  it("cannot be dismissed while a create is in flight (07bn review)", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection()]));
    mocks.createAccount.mockReturnValue(new Promise(() => {}));
    const { findByLabelText, findByRole, getByRole, queryByRole } =
      renderWithClient(<ConnectionsCard />);
    const select = await findByLabelText("Account for Demo Checking");
    fireEvent.change(select, { target: { value: "__create_new__" } });
    await findByRole("dialog", { name: /new account for this connection/i });
    fireEvent.click(getByRole("button", { name: /create and map/i }));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(
      queryByRole("dialog", { name: /new account for this connection/i }),
    ).toBeInTheDocument();
    expect(getByRole("button", { name: /close/i })).toBeDisabled();
  });
});
