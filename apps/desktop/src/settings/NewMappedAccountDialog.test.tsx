// NewMappedAccountDialog — the create-from-mapping path (personal-cfo-049p6):
// with a currency refusal it shows the guard's copy and creates nothing.

import { fireEvent } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderWithClient } from "@/test/renderWithClient";

const mocks = vi.hoisted(() => ({ createAccount: vi.fn(), openUrl: vi.fn() }));
vi.mock("@/bindings", () => ({ commands: { ...mocks } }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));

import { NewMappedAccountDialog } from "./NewMappedAccountDialog";

const REFUSAL =
  "This account is in EUR, but your base currency is USD. Accounts in a currency other than your base currency aren't supported yet.";

beforeEach(() => vi.clearAllMocks());

describe("NewMappedAccountDialog", () => {
  it("shows the currency refusal and disables creating", () => {
    const onCreated = vi.fn();
    const { getByRole, getByText } = renderWithClient(
      <NewMappedAccountDialog
        externalName="Euro Savings"
        currency="USD"
        currencyRefusal={REFUSAL}
        onCreated={onCreated}
        onClose={vi.fn()}
      />,
    );
    expect(getByRole("alert")).toHaveTextContent(REFUSAL);
    const create = getByText("Create and map").closest("button") as HTMLButtonElement;
    expect(create).toBeDisabled();
    fireEvent.click(create);
    expect(mocks.createAccount).not.toHaveBeenCalled();
    expect(onCreated).not.toHaveBeenCalled();
  });

  it("creates as before when there is no refusal", () => {
    const { getByText, queryByRole } = renderWithClient(
      <NewMappedAccountDialog
        externalName="Checking"
        currency="USD"
        onCreated={vi.fn()}
        onClose={vi.fn()}
      />,
    );
    expect(queryByRole("alert")).toBeNull();
    expect(getByText("Create and map").closest("button")).not.toBeDisabled();
  });
});
