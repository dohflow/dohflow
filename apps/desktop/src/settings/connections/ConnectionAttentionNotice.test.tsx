// ConnectionAttentionNotice (personal-cfo-pxi.22): show, partial (one bank), clear, the approved
// copy only, accessibility, the refresh and open-site actions, and the error states, against
// mocked bindings (FRONTEND.md §8).

import { act, fireEvent, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { renderWithClient } from "@/test/renderWithClient";

const mocks = vi.hoisted(() => ({
  connectorConnections: vi.fn(),
  connectorSync: vi.fn(),
  openProviderSite: vi.fn(),
}));

vi.mock("@/bindings", () => ({ commands: { ...mocks } }));

vi.mock("@/vault/useVault", () => ({
  describeIpcError: (error: unknown) =>
    typeof error === "string" ? error : "Something went wrong.",
}));

import { ConnectionAttentionNotice } from "./ConnectionAttentionNotice";

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

const SIMPLEFIN_ID = "11111111-1111-7111-8111-111111111111";
const LUNCHFLOW_ID = "33333333-3333-7333-8333-333333333333";

function connection(over: Record<string, unknown> = {}) {
  return {
    id: SIMPLEFIN_ID,
    adapter_id: "simplefin",
    display_hint: "SimpleFIN Bridge connection",
    last_synced_at: "2026-08-22T10:00:00Z",
    last_error: null,
    refresh_cadence: "every_open",
    attention: [],
    links: [],
    ...over,
  };
}

const reauth = (institution: string | null) => ({
  kind: "reauth_institution",
  institution,
});
const relink = { kind: "relink_connection", institution: null };

beforeEach(() => {
  vi.clearAllMocks();
  mocks.connectorConnections.mockResolvedValue(ok([]));
  mocks.connectorSync.mockResolvedValue(
    ok({
      connection_id: SIMPLEFIN_ID,
      status: "synced",
      staged: 0,
      committed: 0,
      flagged: 0,
      skipped_unmapped: 0,
      warnings: [],
      message: null,
    }),
  );
  mocks.openProviderSite.mockResolvedValue(ok(null));
});

afterEach(() => {
  vi.useRealTimers();
});

describe("ConnectionAttentionNotice", () => {
  it("renders nothing while loading, when nothing needs attention, and when the load fails", async () => {
    mocks.connectorConnections.mockReturnValue(new Promise(() => {}));
    const loading = renderWithClient(<ConnectionAttentionNotice />);
    expect(loading.container).toBeEmptyDOMElement();
    loading.unmount();

    mocks.connectorConnections.mockResolvedValue(ok([connection()]));
    const healthy = renderWithClient(<ConnectionAttentionNotice />);
    await waitFor(() => expect(mocks.connectorConnections).toHaveBeenCalledTimes(2));
    expect(healthy.container).toBeEmptyDOMElement();
    healthy.unmount();

    mocks.connectorConnections.mockRejectedValue(new Error("locked"));
    const failed = renderWithClient(<ConnectionAttentionNotice />);
    await waitFor(() => expect(mocks.connectorConnections).toHaveBeenCalledTimes(3));
    expect(failed.container).toBeEmptyDOMElement();
  });

  it("names the bank that needs a sign-in, in the approved words", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([connection({ attention: [reauth("First Bank")] })]),
    );
    const { findByRole } = renderWithClient(<ConnectionAttentionNotice />);
    const alert = await findByRole("alert");
    expect(alert).toHaveTextContent(
      "Sign in to First Bank again in SimpleFIN Bridge so its accounts keep refreshing.",
    );
  });

  it("uses the no-name wording when the provider did not name the bank", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([connection({ attention: [reauth(null)] })]),
    );
    const { findByRole } = renderWithClient(<ConnectionAttentionNotice />);
    expect(await findByRole("alert")).toHaveTextContent(
      "One of your banks needs you to sign in again in SimpleFIN Bridge.",
    );
  });

  it("says only the affected banks are out of date (partial)", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([connection({ attention: [reauth("First Bank"), reauth("Third Bank")] })]),
    );
    const { findByRole, queryByText } = renderWithClient(<ConnectionAttentionNotice />);
    const alert = await findByRole("alert");
    expect(alert).toHaveTextContent("Sign in to First Bank again");
    expect(alert).toHaveTextContent("Sign in to Third Bank again");
    expect(queryByText(/Second Bank/)).toBeNull();
  });

  it("asks for the whole connection to be set up again, for SimpleFIN and LunchFlow", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([
        connection({ attention: [relink] }),
        connection({ id: LUNCHFLOW_ID, adapter_id: "lunchflow", attention: [relink] }),
      ]),
    );
    const { findAllByRole } = renderWithClient(<ConnectionAttentionNotice />);
    const alerts = await findAllByRole("alert");
    expect(alerts).toHaveLength(2);
    expect(alerts[0]).toHaveTextContent("Your SimpleFIN connection needs to be set up again.");
    expect(alerts[1]).toHaveTextContent("Your LunchFlow connection needs a new API key.");
  });

  it("shows one card per affected connection and skips healthy ones", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([
        connection({ attention: [reauth("First Bank")] }),
        connection({ id: "44444444-4444-7444-8444-444444444444" }),
        connection({
          id: LUNCHFLOW_ID,
          adapter_id: "lunchflow",
          attention: [relink],
        }),
      ]),
    );
    const { findAllByRole } = renderWithClient(<ConnectionAttentionNotice />);
    expect(await findAllByRole("alert")).toHaveLength(2);
  });

  it("shows nothing for an adapter or kind with no approved wording", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([
        connection({ adapter_id: "someprovider", attention: [relink] }),
        // LunchFlow never reports a per-bank sign-in; there is no approved line for it.
        connection({ id: LUNCHFLOW_ID, adapter_id: "lunchflow", attention: [reauth("X")] }),
      ]),
    );
    const { container } = renderWithClient(<ConnectionAttentionNotice />);
    await waitFor(() => expect(mocks.connectorConnections).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it("renders a bank name as plain text, never as markup", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([connection({ attention: [reauth("<img src=x onerror=alert(1)>")] })]),
    );
    const { findByRole, container } = renderWithClient(<ConnectionAttentionNotice />);
    expect(await findByRole("alert")).toHaveTextContent("<img src=x onerror=alert(1)>");
    expect(container.querySelector("img")).toBeNull();
  });

  it("is a labelled region whose cards are announced and keyboard reachable", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([connection({ attention: [reauth("First Bank")] })]),
    );
    const { findByRole, getByRole } = renderWithClient(<ConnectionAttentionNotice />);
    const alert = await findByRole("alert");
    expect(getByRole("region", { name: "Bank connections that need you" })).toBeInTheDocument();
    expect(alert).toHaveAccessibleDescription(
      "Sign in to First Bank again in SimpleFIN Bridge so its accounts keep refreshing.",
    );
    // Real buttons: focusable in order, no dismiss control.
    const buttons = alert.querySelectorAll("button");
    expect([...buttons].map((b) => b.textContent)).toEqual([
      "Open SimpleFIN Bridge",
      "Refresh now",
    ]);
    buttons.forEach((b) => expect(b.tabIndex).toBe(0));
  });

  it("refreshes the connection it belongs to, and the notice clears when the next read is healthy", async () => {
    mocks.connectorConnections
      .mockResolvedValueOnce(ok([connection({ attention: [reauth("First Bank")] })]))
      .mockResolvedValue(ok([connection()]));
    const { findByRole, getByRole, queryByRole } = renderWithClient(<ConnectionAttentionNotice />);
    await findByRole("alert");

    fireEvent.click(getByRole("button", { name: "Refresh now" }));
    await waitFor(() =>
      expect(mocks.connectorSync).toHaveBeenCalledWith(
        expect.objectContaining({ connection_id: SIMPLEFIN_ID }),
      ),
    );
    await waitFor(() => expect(queryByRole("alert")).toBeNull());
  });

  it("shows the refresh as pending and keeps the notice when the bank still needs a sign-in", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([connection({ attention: [reauth("First Bank")] })]),
    );
    let finish: (value: unknown) => void = () => {};
    mocks.connectorSync.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    const { findByRole, getByRole, queryByRole } = renderWithClient(<ConnectionAttentionNotice />);
    await findByRole("alert");

    fireEvent.click(getByRole("button", { name: "Refresh now" }));
    await waitFor(() => expect(getByRole("button", { name: "Refresh now" })).toBeDisabled());

    await act(async () => {
      finish(
        ok({
          connection_id: SIMPLEFIN_ID,
          status: "synced",
          staged: 0,
          committed: 0,
          flagged: 0,
          skipped_unmapped: 0,
          warnings: [],
          message: null,
        }),
      );
    });
    await waitFor(() => expect(getByRole("button", { name: "Refresh now" })).toBeEnabled());
    // The notice stays: only the backend clearing it removes it.
    expect(getByRole("alert")).toHaveTextContent("Sign in to First Bank again");
    // The outcome is announced as part of the card's one live region, not a nested one.
    expect(getByRole("alert")).toHaveTextContent("Refreshed.");
    expect(queryByRole("status")).toBeNull();
  });

  it("reports a failed refresh (for example offline) instead of going quiet", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([connection({ attention: [reauth("First Bank")] })]),
    );
    mocks.connectorSync.mockResolvedValue({
      status: "error",
      error: "offline",
    });
    const { findByRole, getByRole, findByText } = renderWithClient(<ConnectionAttentionNotice />);
    const alert = await findByRole("alert");
    fireEvent.click(getByRole("button", { name: "Refresh now" }));
    // The failure text appears inside the card's live region.
    const message = await findByText("offline");
    expect(alert).toContainElement(message);
  });

  it("opens the provider's site by naming only the connection", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([connection({ attention: [reauth("First Bank")] })]),
    );
    const { findByRole, getByRole } = renderWithClient(<ConnectionAttentionNotice />);
    await findByRole("alert");
    fireEvent.click(getByRole("button", { name: "Open SimpleFIN Bridge" }));
    await waitFor(() => expect(mocks.openProviderSite).toHaveBeenCalledTimes(1));
    expect(mocks.openProviderSite).toHaveBeenCalledWith({ connection_id: SIMPLEFIN_ID });
  });

  it("announces a failure to open the provider's site", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([connection({ attention: [reauth("First Bank")] })]),
    );
    mocks.openProviderSite.mockResolvedValue({
      status: "error",
      error: "The provider's site could not be opened",
    });
    const { findByRole, getByRole, findByText } = renderWithClient(
      <ConnectionAttentionNotice />,
    );
    const alert = await findByRole("alert");
    fireEvent.click(getByRole("button", { name: "Open SimpleFIN Bridge" }));
    // Inside the card's own live region, not an alert of its own.
    const message = await findByText("The provider's site could not be opened");
    expect(alert).toContainElement(message);
    expect(message).not.toHaveAttribute("role");
  });

  it("clears one action's message when the other runs", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([connection({ attention: [reauth("First Bank")] })]),
    );
    mocks.openProviderSite.mockResolvedValue({ status: "error", error: "could not open" });
    const { findByRole, getByRole, findByText, queryByText } = renderWithClient(
      <ConnectionAttentionNotice />,
    );
    await findByRole("alert");
    fireEvent.click(getByRole("button", { name: "Open SimpleFIN Bridge" }));
    await findByText("could not open");
    fireEvent.click(getByRole("button", { name: "Refresh now" }));
    await findByText("Refreshed.");
    expect(queryByText("could not open")).toBeNull();
  });

  it("re-reads the connections on a timer, so a background refresh's notice appears and clears", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mocks.connectorConnections
      .mockResolvedValueOnce(ok([connection()]))
      .mockResolvedValueOnce(ok([connection({ attention: [reauth("First Bank")] })]))
      .mockResolvedValue(ok([connection()]));
    const { queryByRole, findByRole } = renderWithClient(<ConnectionAttentionNotice />);
    await waitFor(() => expect(mocks.connectorConnections).toHaveBeenCalledTimes(1));
    expect(queryByRole("alert")).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    await findByRole("alert");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    await waitFor(() => expect(queryByRole("alert")).toBeNull());
  });
});
