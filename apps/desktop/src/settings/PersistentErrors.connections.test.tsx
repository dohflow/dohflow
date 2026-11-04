/**
 * personal-cfo-4d8.39.2 (slice b, part 3): Connections keeps each connection's refresh failure,
 * held-account message and failed Refresh now, with the one Refresh now that recovers from them, in
 * the section's persistent slot. Read errors and the messages of other failed actions stay there too.
 * Refresh now is rendered once per connection; collapsing the section runs nothing.
 */

import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { ConnectionsCard } from "./ConnectionsCard";
import { exampleflow, simplefin } from "./connections/fixtures/adapters";
import { SettingsSection } from "./SettingsSection";

const mocks = vi.hoisted(() => ({
  openUrl: vi.fn(),
  connectorAdapters: vi.fn(),
  connectorConnections: vi.fn(),
  connectorLink: vi.fn(),
  connectorSetAccountLink: vi.fn(),
  connectorCreateMappedAccount: vi.fn(),
  connectorSetRefreshCadence: vi.fn(),
  connectorSync: vi.fn(),
  connectorForget: vi.fn(),
  accountList: vi.fn(),
  createAccount: vi.fn(),
  baseCurrency: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: { ...mocks } }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));
vi.mock("@/vault/useVault", () => ({
  describeIpcError: (error: unknown) => (typeof error === "string" ? error : "Something went wrong."),
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const connection = (id: string, over: Record<string, unknown> = {}) => ({
  id,
  adapter_id: "simplefin",
  display_hint: `Bank ${id}`,
  last_synced_at: "2026-08-22T10:00:00Z",
  last_error: null,
  refresh_cadence: "every_open",
  attention: [],
  links: [
    {
      external_id: `ACT-${id}`,
      external_name: "Demo Checking",
      account_id: null,
      last_synced_on: null,
      currency: "USD",
      currency_refusal: null,
    },
  ],
  ...over,
});
const heldLink = {
  external_id: "ACT-EUR",
  external_name: "Euro account",
  account_id: "22222222-2222-7222-8222-222222222222",
  last_synced_on: null,
  currency: "EUR",
  currency_refusal: "This account is in EUR; your base currency is USD.",
};
const syncResult = (id: string, status: string, message: string | null) =>
  ok({
    connection_id: id,
    status,
    staged: 0,
    committed: 0,
    flagged: 0,
    skipped_unmapped: 0,
    warnings: [],
    message,
  });

beforeEach(() => {
  vi.clearAllMocks();
  mocks.connectorAdapters.mockResolvedValue([simplefin, exampleflow]);
  mocks.connectorConnections.mockResolvedValue(ok([]));
  mocks.accountList.mockResolvedValue(
    ok([{ id: "22222222-2222-7222-8222-222222222222", name: "Checking", active: true }]),
  );
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
});

const renderCard = () =>
  renderWithClient(
    <SettingsSection title="Connections" span="full">
      <ConnectionsCard />
    </SettingsSection>,
  );
const section = () => document.querySelector(`[data-settings-section="Connections"]`) as HTMLElement;
const slot = () => section().querySelector("[data-settings-persistent]") as HTMLElement;
const refreshButtons = () => within(section()).queryAllByRole("button", { name: "Refresh now", hidden: true });
const collapse = () => fireEvent.click(screen.getByRole("button", { name: "Connections" }));

describe("Connections", () => {
  it("a failed last refresh keeps its message and its one Refresh now in the slot while collapsed", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([connection("a", { last_error: "access revoked — re-link required" })]),
    );
    renderCard();
    const message = await screen.findByText("Last refresh failed: access revoked — re-link required");
    expect(slot().contains(message)).toBe(true);
    expect(slot()).toHaveTextContent("Bank a");
    expect(refreshButtons()).toHaveLength(1);
    expect(slot().contains(refreshButtons()[0]!)).toBe(true);

    collapse();
    expect(screen.getByText("Last refresh failed: access revoked — re-link required")).toBeVisible();
    expect(mocks.connectorSync).not.toHaveBeenCalled();
    mocks.connectorSync.mockResolvedValue(syncResult("a", "synced", null));
    fireEvent.click(screen.getByRole("button", { name: "Refresh now" }));
    await waitFor(() => expect(mocks.connectorSync).toHaveBeenCalledTimes(1));
  });

  it("a held foreign-currency account keeps its full message and its one Refresh now in the slot", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection("a", { links: [heldLink] })]));
    renderCard();
    const message = await screen.findByText(
      /An account on this connection is in a currency other than your base currency\. Its transactions are held and not imported\./,
    );
    expect(slot().contains(message)).toBe(true);
    expect(refreshButtons()).toHaveLength(1);
    expect(slot().contains(refreshButtons()[0]!)).toBe(true);
  });

  it("a healthy connection keeps Refresh now in the body, and the slot is empty", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection("a")]));
    renderCard();
    await screen.findByRole("button", { name: "Refresh now" });
    expect(refreshButtons()).toHaveLength(1);
    expect(slot().contains(refreshButtons()[0]!)).toBe(false);
    expect(slot()).toBeEmptyDOMElement();
  });

  it("only the connection that needs it moves its Refresh now; each connection has exactly one", async () => {
    mocks.connectorConnections.mockResolvedValue(
      ok([connection("a", { last_error: "boom" }), connection("b")]),
    );
    renderCard();
    await screen.findByText("Last refresh failed: boom");
    const buttons = refreshButtons();
    expect(buttons).toHaveLength(2);
    expect(buttons.filter((b) => slot().contains(b))).toHaveLength(1);
  });

  it("a failed Refresh now keeps its message and its Refresh now in the slot, and trying again runs the refresh", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection("a")]));
    mocks.connectorSync.mockResolvedValueOnce(syncResult("a", "failed", null));
    renderCard();
    fireEvent.click(await screen.findByRole("button", { name: "Refresh now" }));
    const alert = await screen.findByText("The refresh did not complete.");
    expect(slot().contains(alert)).toBe(true);
    expect(refreshButtons()).toHaveLength(1);
    expect(slot().contains(refreshButtons()[0]!)).toBe(true);

    collapse();
    expect(screen.getByText("The refresh did not complete.")).toBeVisible();
    mocks.connectorSync.mockResolvedValueOnce(syncResult("a", "synced", null));
    fireEvent.click(screen.getByRole("button", { name: "Refresh now" }));
    await waitFor(() => expect(mocks.connectorSync).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByText("The refresh did not complete.")).not.toBeInTheDocument());
    // Recovered: the one Refresh now is back in the body.
    expect(refreshButtons()).toHaveLength(1);
    expect(slot().contains(refreshButtons()[0]!)).toBe(false);
  });

  it("a failed Refresh now on one connection leaves the other connection's button in the body", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection("a"), connection("b")]));
    mocks.connectorSync.mockResolvedValueOnce(syncResult("b", "failed", "provider said no"));
    renderCard();
    const buttons = await screen.findAllByRole("button", { name: "Refresh now" });
    fireEvent.click(buttons[1]!);
    await screen.findByText("provider said no");
    const after = refreshButtons();
    expect(after).toHaveLength(2);
    expect(after.filter((b) => slot().contains(b))).toHaveLength(1);
    expect(slot()).toHaveTextContent("Bank b");
    expect(slot()).not.toHaveTextContent("Bank a");
  });

  it("focus stays on Refresh now when a failed refresh moves it into the slot", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection("a")]));
    mocks.connectorSync.mockResolvedValueOnce(syncResult("a", "failed", "provider said no"));
    renderCard();
    const button = await screen.findByRole("button", { name: "Refresh now" });
    button.focus();
    fireEvent.click(button);
    await screen.findByText("provider said no");
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Refresh now" })),
    );
    expect(slot().contains(document.activeElement)).toBe(true);
  });

  it("a failed read of the connections keeps its message in the slot", async () => {
    mocks.connectorConnections.mockRejectedValue(new Error("locked"));
    renderCard();
    const message = await screen.findByText("Could not load connections.");
    expect(slot().contains(message)).toBe(true);
    collapse();
    expect(screen.getByText("Could not load connections.")).toBeVisible();
  });

  it("a failed forget keeps its message in the slot and is not tied to any Refresh now", async () => {
    mocks.connectorConnections.mockResolvedValue(ok([connection("a")]));
    mocks.connectorForget.mockResolvedValue({ status: "error", error: "Persistence failure" });
    renderCard();
    fireEvent.click(await screen.findByRole("button", { name: "Forget connection…" }));
    fireEvent.click(screen.getByRole("button", { name: "Forget" }));
    const message = await screen.findByText("Persistence failure");
    expect(slot().contains(message)).toBe(true);
    expect(refreshButtons()).toHaveLength(1);
    expect(slot().contains(refreshButtons()[0]!)).toBe(false);
  });
});
