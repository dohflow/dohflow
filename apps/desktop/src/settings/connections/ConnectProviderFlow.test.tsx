// ConnectProviderFlow (personal-cfo-dto2j): picker → disclosure → credential
// form, fed by a mocked connector registry. Covers one vs many enabled
// providers, disabled providers hidden, registry order kept, no credential
// field before the disclosure, back navigation and focus, and the loading /
// error / empty states (FRONTEND.md §6).

import { fireEvent, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderWithClient } from "@/test/renderWithClient";

const mocks = vi.hoisted(() => ({ connectorAdapters: vi.fn() }));
vi.mock("@/bindings", () => ({ commands: { ...mocks } }));

import { ConnectProviderFlow } from "./ConnectProviderFlow";
import { dormant, exampleflow, simplefin } from "./fixtures/adapters";

function renderFlow(onLink = vi.fn().mockResolvedValue(undefined)) {
  const view = renderWithClient(
    <ConnectProviderFlow onLink={onLink} linkPending={false} onCancel={vi.fn()} />,
  );
  return { ...view, onLink };
}

const passwordInputs = (container: HTMLElement) =>
  container.querySelectorAll('input[type="password"]');

beforeEach(() => {
  vi.clearAllMocks();
});

describe("ConnectProviderFlow", () => {
  it("lists exactly the enabled providers, in registry order, with their details", async () => {
    // Registry order (ADR 0076 §4) is the backend's; the picker keeps it.
    mocks.connectorAdapters.mockResolvedValue([simplefin, dormant, exampleflow]);
    const { findByRole, container } = renderFlow();
    const list = await findByRole("region", { name: "Choose a provider" });
    const names = within(list)
      .getAllByRole("button", { name: /^Choose / })
      .map((button) => button.textContent);
    expect(names).toEqual(["Choose SimpleFIN", "Choose ExampleFlow"]);
    expect(within(list).queryByText("Dormant Provider")).toBeNull();

    const entry = within(list).getByText("ExampleFlow").closest("li") as HTMLElement;
    for (const chip of ["Accounts", "Transactions", "Balances", "Holdings"]) {
      expect(within(entry).getByText(chip)).toBeInTheDocument();
    }
    expect(entry).toHaveTextContent(
      "You pay ExampleFlow directly: $34.99 a year, up to 2 connections included, then $10.00 a year for each extra connection.",
    );
    expect(entry).toHaveTextContent(/Terms reviewed/);
    expect(entry).toHaveTextContent("https://exampleflow.example/terms");
    expect(entry).toHaveTextContent(/Available in: .*United Kingdom.*United States/);
    // Choosing asks for nothing yet.
    expect(passwordInputs(container)).toHaveLength(0);
  });

  it("shows the disclosure before any credential field, then the provider's form", async () => {
    mocks.connectorAdapters.mockResolvedValue([simplefin, exampleflow]);
    const { findByText, getByText, getByLabelText, container, onLink } = renderFlow();
    fireEvent.click(await findByText("Choose ExampleFlow"));

    expect(getByText("About ExampleFlow")).toBeInTheDocument();
    expect(passwordInputs(container)).toHaveLength(0);
    expect(document.activeElement).toHaveTextContent("About ExampleFlow");

    fireEvent.click(getByText("Continue"));
    const input = getByLabelText("API key");
    expect(input).toHaveAttribute("type", "password");
    expect(input).toHaveAttribute("placeholder", "Paste the API key");
    fireEvent.change(input, { target: { value: "  key-123  " } });
    fireEvent.click(getByText("Connect"));
    await waitFor(() => expect(onLink).toHaveBeenCalledWith("exampleflow", "key-123"));
    await waitFor(() => expect(input).toHaveValue(""));
  });

  it("goes back to the list and returns focus to its heading", async () => {
    mocks.connectorAdapters.mockResolvedValue([simplefin, exampleflow]);
    const { findByText, getByText, findByRole } = renderFlow();
    fireEvent.click(await findByText("Choose SimpleFIN"));
    fireEvent.click(getByText("Back to providers"));
    const heading = await findByRole("heading", { name: "Choose a provider" });
    await waitFor(() => expect(document.activeElement).toBe(heading));
  });

  it("skips the picker when exactly one provider is enabled", async () => {
    mocks.connectorAdapters.mockResolvedValue([simplefin, dormant]);
    const { findByText, queryByText, container } = renderFlow();
    expect(await findByText("About the SimpleFIN Bridge")).toBeInTheDocument();
    expect(queryByText("Choose a provider")).toBeNull();
    expect(queryByText("Back to providers")).toBeNull();
    expect(passwordInputs(container)).toHaveLength(0);
  });

  it("never offers a relay-tier provider, even if one is registered", async () => {
    // Invariant 6 (ADR 0076 §1): relay providers are never enabled in the free
    // app, so the enabled-only filter is what keeps them out.
    const relay = { ...exampleflow, adapter_id: "relayed", tier: "Relay" as const, enabled: false };
    mocks.connectorAdapters.mockResolvedValue([simplefin, relay]);
    const { findByText, queryByText } = renderFlow();
    expect(await findByText("About the SimpleFIN Bridge")).toBeInTheDocument();
    expect(queryByText(/relayed/i)).toBeNull();
  });

  it("shows loading, error and no-provider states", async () => {
    mocks.connectorAdapters.mockReturnValue(new Promise(() => {}));
    const loading = renderFlow();
    expect(loading.getByRole("status", { name: "Loading connection providers" })).toBeInTheDocument();
    loading.unmount();

    mocks.connectorAdapters.mockRejectedValue(new Error("ipc down"));
    const failed = renderFlow();
    expect(await failed.findByRole("alert")).toHaveTextContent(
      "Could not load connection providers.",
    );
    failed.unmount();

    mocks.connectorAdapters.mockResolvedValue([dormant]);
    const none = renderFlow();
    expect(await none.findByText("No connection providers available")).toBeInTheDocument();
  });
});
