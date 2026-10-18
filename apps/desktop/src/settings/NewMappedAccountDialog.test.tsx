// NewMappedAccountDialog — the create-from-mapping path (personal-cfo-049p6):
// with a currency refusal it shows the guard's copy and creates nothing.

import { fireEvent } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderWithClient } from "@/test/renderWithClient";

const mocks = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@/bindings", () => ({ commands: { ...mocks } }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));

import { NewMappedAccountDialog } from "./NewMappedAccountDialog";

const REFUSAL =
  "This account is in EUR, but your base currency is USD. Accounts in a currency other than your base currency aren't supported yet.";

beforeEach(() => vi.clearAllMocks());

describe("NewMappedAccountDialog", () => {
  it("shows the currency refusal and disables creating", () => {
    const onCreated = vi.fn();
    const onCreate = vi.fn();
    const { getByRole, getByText } = renderWithClient(
      <NewMappedAccountDialog
        externalName="Euro Savings"
        currency="USD"
        currencyRefusal={REFUSAL}
        onCreate={onCreate}
        onCreated={onCreated}
        onClose={vi.fn()}
      />,
    );
    expect(getByRole("alert")).toHaveTextContent(REFUSAL);
    const create = getByText("Create and map").closest("button") as HTMLButtonElement;
    expect(create).toBeDisabled();
    fireEvent.click(create);
    expect(onCreate).not.toHaveBeenCalled();
    expect(onCreated).not.toHaveBeenCalled();
  });

  it("creates as before when there is no refusal", () => {
    const { getByText, queryByRole } = renderWithClient(
      <NewMappedAccountDialog
        externalName="Checking"
        currency="USD"
        onCreate={vi.fn()}
        onCreated={vi.fn()}
        onClose={vi.fn()}
      />,
    );
    expect(queryByRole("alert")).toBeNull();
    expect(getByText("Create and map").closest("button")).not.toBeDisabled();
  });

  it("shows the guard's refusal when the dialog's own state was stale (pxi.8)", async () => {
    // The dialog thought this account was mappable; the one-call create-and-map
    // ran the guard against the current base currency and refused.
    const onCreated = vi.fn();
    const onClose = vi.fn();
    const onCreate = vi.fn().mockResolvedValue({ id: null, error: { Validation: REFUSAL } });
    const { getByText, findByRole } = renderWithClient(
      <NewMappedAccountDialog
        externalName="Checking"
        currency="USD"
        onCreate={onCreate}
        onCreated={onCreated}
        onClose={onClose}
      />,
    );
    fireEvent.click(getByText("Create and map"));
    expect(await findByRole("alert")).toHaveTextContent(REFUSAL);
    expect(onCreate).toHaveBeenCalledTimes(1);
    expect(onCreate.mock.calls[0]![0]).toMatchObject({ name: "Checking", currency: "USD" });
    expect(onCreated).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("closes after a successful create-and-map", async () => {
    const onCreated = vi.fn();
    const onClose = vi.fn();
    const onCreate = vi.fn().mockResolvedValue({ id: "acct-1", error: null });
    const { getByText } = renderWithClient(
      <NewMappedAccountDialog
        externalName="Checking"
        currency="USD"
        onCreate={onCreate}
        onCreated={onCreated}
        onClose={onClose}
      />,
    );
    fireEvent.click(getByText("Create and map"));
    await vi.waitFor(() => expect(onCreated).toHaveBeenCalledWith("acct-1"));
    expect(onClose).toHaveBeenCalled();
  });
});
